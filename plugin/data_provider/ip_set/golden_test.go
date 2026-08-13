/*
 * Copyright (C) 2020-2026, IrineSistiana
 *
 * Slice 0 golden fixtures for the ip_set provider. These freeze the Go
 * plain-IP/SRS input parsing, MatcherGroup composition, and atomic snapshot
 * reload behavior that the Rust IP integration must preserve.
 */

package ip_set

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"errors"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
)

func mustAddr(t *testing.T, s string) netip.Addr {
	t.Helper()
	a, err := netip.ParseAddr(s)
	if err != nil {
		t.Fatal(err)
	}
	return a
}

// varbinBytes encodes b as sing varbin []byte: uvarint length + raw bytes.
func varbinBytes(b []byte) []byte {
	var buf bytes.Buffer
	var cnt [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(cnt[:], uint64(len(b)))
	buf.Write(cnt[:n])
	buf.Write(b)
	return buf.Bytes()
}

// buildIPSRS encodes a v3 IP rule set: magic, version, zlib stream containing
// uvarint rule count, one default-mode rule with one IPCIDR item.
func buildIPSRS(t *testing.T, ranges [][2]netip.Addr) []byte {
	t.Helper()
	var compressed bytes.Buffer
	zw := zlib.NewWriter(&compressed)
	bw := bufio.NewWriter(zw)
	var cnt [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(cnt[:], 1)
	if _, err := bw.Write(cnt[:n]); err != nil {
		t.Fatal(err)
	}
	writeByte := func(b byte) {
		t.Helper()
		if err := bw.WriteByte(b); err != nil {
			t.Fatal(err)
		}
	}
	writeByte(0x00) // mode: default
	writeByte(ruleItemIPCIDR)
	writeByte(1) // ipset version
	// streamParseIPSet reads the range count as a fixed-width big-endian
	// uint64, not a uvarint.
	if err := binary.Write(bw, binary.BigEndian, uint64(len(ranges))); err != nil {
		t.Fatal(err)
	}
	for _, r := range ranges {
		if _, err := bw.Write(varbinBytes(r[0].AsSlice())); err != nil {
			t.Fatal(err)
		}
		if _, err := bw.Write(varbinBytes(r[1].AsSlice())); err != nil {
			t.Fatal(err)
		}
	}
	if err := bw.Flush(); err != nil {
		t.Fatal(err)
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	out := make([]byte, 0, 4+compressed.Len())
	out = append(out, srsMagic[:]...)
	out = append(out, maxSupportedVersion)
	out = append(out, compressed.Bytes()...)
	return out
}

func newTestIPSet() *IPSet {
	return &IPSet{list: netlist.NewList(), mutex: sync.RWMutex{}}
}

func TestGoldenIPSetPlainLoad(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "ips.txt")
	content := `# comment
10.0.0.0/8
192.168.1.1        # host address -> /32
2001:db8::/32
`
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}

	p := newTestIPSet()
	err := LoadFromIPsAndFiles([]string{"8.8.8.8/32"}, []string{path}, p.list)
	if err != nil {
		t.Fatal(err)
	}
	p.list.Sort()
	p.rebuildSnapshot()

	tests := []struct {
		addr string
		want bool
	}{
		{"10.1.2.3", true},
		{"192.168.1.1", true},
		{"192.168.1.2", false},
		{"2001:db8::1", true},
		{"8.8.8.8", true},
		{"8.8.4.4", false},
		{"1.2.3.4", false},
	}
	for _, tt := range tests {
		if got := p.Match(mustAddr(t, tt.addr)); got != tt.want {
			t.Errorf("Match(%s) = %v, want %v", tt.addr, got, tt.want)
		}
	}
}

func TestGoldenIPSetSRSLoad(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "ips.srs")
	srs := buildIPSRS(t, [][2]netip.Addr{
		{mustAddr(t, "203.0.113.0"), mustAddr(t, "203.0.113.255")},
		{mustAddr(t, "2001:db8:ffff::"), mustAddr(t, "2001:db8:ffff:ffff::")},
	})
	if err := os.WriteFile(path, srs, 0o644); err != nil {
		t.Fatal(err)
	}

	p := newTestIPSet()
	if err := LoadFromIPsAndFiles(nil, []string{path}, p.list); err != nil {
		t.Fatal(err)
	}
	p.list.Sort()
	p.rebuildSnapshot()

	tests := []struct {
		addr string
		want bool
	}{
		{"203.0.113.1", true},
		{"203.0.113.255", true},
		{"203.0.114.1", false},
		{"2001:db8:ffff::1", true},
		{"2001:db8:ffff:ffff::", true},   // range upper bound
		{"2001:db8:ffff:ffff::1", false}, // one past the upper bound
		{"2001:db8:fffe::1", false},
	}
	for _, tt := range tests {
		if got := p.Match(mustAddr(t, tt.addr)); got != tt.want {
			t.Errorf("Match(%s) = %v, want %v", tt.addr, got, tt.want)
		}
	}
}

type stubIPMatcher struct {
	prefix netip.Prefix
}

func (s stubIPMatcher) Match(addr netip.Addr) bool {
	return s.prefix.Contains(addr)
}

type fakeRustIPMatcher struct {
	prefix netip.Prefix
	closed atomic.Int32
}

func (m *fakeRustIPMatcher) Match(s string) (bool, error) {
	if m.closed.Load() != 0 {
		return false, errors.New("fake Rust IP matcher closed")
	}
	addr, err := netip.ParseAddr(s)
	if err != nil {
		return false, err
	}
	return m.prefix.Contains(addr), nil
}

func (m *fakeRustIPMatcher) Close() error {
	m.closed.Add(1)
	return nil
}

func TestIPSetPostBuildDoesNotBlockMatch(t *testing.T) {
	oldBuilder := rustIPMatcherBuilder
	t.Cleanup(func() { rustIPMatcherBuilder = oldBuilder })

	buildStarted := make(chan struct{})
	releaseBuild := make(chan struct{})
	rustIPMatcherBuilder = func(prefixes []string) (RustMatcher, error) {
		if len(prefixes) != 1 || prefixes[0] != "192.0.2.0/24" {
			return nil, errors.New("unexpected candidate prefixes")
		}
		close(buildStarted)
		<-releaseBuild
		return &fakeRustIPMatcher{prefix: netip.MustParsePrefix(prefixes[0])}, nil
	}

	p := newTestIPSet()
	p.list.Append(netip.MustParsePrefix("10.0.0.0/8"))
	p.list.Sort()
	p.rebuildSnapshot()
	oldRust := &fakeRustIPMatcher{prefix: netip.MustParsePrefix("10.0.0.0/8")}
	p.rustMatcher = oldRust
	p.files = []string{filepath.Join(t.TempDir(), "ips.txt")}

	postDone := make(chan *httptest.ResponseRecorder, 1)
	go func() {
		r := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodPost, "/post", bytes.NewBufferString(`{"values":["192.0.2.0/24"]}`))
		p.api().ServeHTTP(r, req)
		postDone <- r
	}()
	select {
	case <-buildStarted:
	case <-time.After(time.Second):
		t.Fatal("reload did not reach the Rust candidate build")
	}

	matchDone := make(chan bool, 1)
	go func() { matchDone <- p.Match(mustAddr(t, "10.1.2.3")) }()
	select {
	case matched := <-matchDone:
		if !matched {
			t.Fatal("the old generation must remain available during candidate build")
		}
	case <-time.After(time.Second):
		t.Fatal("Rust candidate build must not hold the IP match state lock")
	}

	close(releaseBuild)
	select {
	case r := <-postDone:
		if r.Code != http.StatusOK {
			t.Fatalf("POST status = %d, body=%s", r.Code, r.Body.String())
		}
	case <-time.After(time.Second):
		t.Fatal("reload did not publish after candidate build completed")
	}

	if oldRust.closed.Load() != 1 {
		t.Fatalf("old Rust handle closed %d times, want once", oldRust.closed.Load())
	}
	if !p.Match(mustAddr(t, "192.0.2.1")) {
		t.Fatal("new generation must match after publish")
	}
	if p.Match(mustAddr(t, "10.1.2.3")) {
		t.Fatal("new generation must replace the old Go/Rust snapshot")
	}
}

func TestGoldenIPSetComposition(t *testing.T) {
	p := newTestIPSet()
	p.list.Append(netip.MustParsePrefix("10.0.0.0/8"))
	p.list.Sort()
	p.otherSets = append(p.otherSets, stubIPMatcher{prefix: netip.MustParsePrefix("172.16.0.0/12")})
	p.rebuildSnapshot()

	tests := []struct {
		addr string
		want bool
	}{
		{"10.1.1.1", true},
		{"172.16.0.1", true}, // via referenced matcher
		{"192.168.1.1", false},
	}
	for _, tt := range tests {
		if got := p.Match(mustAddr(t, tt.addr)); got != tt.want {
			t.Errorf("Match(%s) = %v, want %v", tt.addr, got, tt.want)
		}
	}
}

func TestGoldenIPSetSnapshotReload(t *testing.T) {
	p := newTestIPSet()
	p.list.Append(netip.MustParsePrefix("10.0.0.0/8"))
	p.list.Sort()
	p.rebuildSnapshot()
	if !p.Match(mustAddr(t, "10.1.1.1")) {
		t.Fatal("snapshot 1 must match 10.x")
	}

	// Rebuild a new list and atomically replace the snapshot.
	newList := netlist.NewList()
	newList.Append(netip.MustParsePrefix("172.16.0.0/12"))
	newList.Sort()
	p.list = newList
	p.rebuildSnapshot()
	if !p.Match(mustAddr(t, "172.16.0.1")) {
		t.Fatal("snapshot 2 must match 172.16.x")
	}
	if p.Match(mustAddr(t, "10.1.1.1")) {
		t.Fatal("snapshot 2 must not still match old 10.x list")
	}
}

func TestGoldenIPSetNormalizePrefix(t *testing.T) {
	tests := []struct {
		in   string
		want string
	}{
		{"::ffff:192.168.0.0/120", "192.168.0.0/24"}, // IPv4-mapped -> plain IPv4
		{"10.0.0.0/8", "10.0.0.0/8"},
		{"2001:db8::/32", "2001:db8::/32"},
	}
	for _, tt := range tests {
		got := normalizePrefix(netip.MustParsePrefix(tt.in)).String()
		if got != tt.want {
			t.Errorf("normalizePrefix(%s) = %s, want %s", tt.in, got, tt.want)
		}
	}
}

func TestGoldenIPSetEmptyFileIsNoOp(t *testing.T) {
	p := newTestIPSet()
	// An empty / non-SRS file must load without error and add nothing.
	if err := LoadFromIPsAndFiles(nil, []string{"/dev/null"}, p.list); err != nil {
		t.Fatal(err)
	}
	p.list.Sort()
	if p.list.Len() != 0 {
		t.Fatalf("empty file added %d prefixes", p.list.Len())
	}
}

func TestGoldenIPSetInvalidLineErrorsFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "bad.txt")
	if err := os.WriteFile(path, []byte("10.0.0.0/8\nnot-an-ip\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	p := newTestIPSet()
	// Contract: unlike domain_set file load, one invalid IP row fails the
	// whole file load through netlist.LoadFromReader.
	if err := LoadFromIPsAndFiles(nil, []string{path}, p.list); err == nil {
		t.Fatal("invalid IP line must fail the file load")
	}
}
