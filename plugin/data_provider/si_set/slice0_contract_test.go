package si_set

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"context"
	"encoding/binary"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"os"
	"path/filepath"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
	"github.com/sagernet/sing/common/varbin"
)

func buildSlice0IPSRS(t *testing.T, ranges [][2]netip.Addr) []byte {
	t.Helper()
	var compressed bytes.Buffer
	zw := zlib.NewWriter(&compressed)
	bw := bufio.NewWriter(zw)
	var count [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(count[:], 1)
	if _, err := bw.Write(count[:n]); err != nil {
		t.Fatal(err)
	}
	if err := bw.WriteByte(0); err != nil {
		t.Fatal(err)
	}
	if err := bw.WriteByte(ruleItemIPCIDR); err != nil {
		t.Fatal(err)
	}
	if err := bw.WriteByte(1); err != nil {
		t.Fatal(err)
	}
	if err := binary.Write(bw, binary.BigEndian, uint64(len(ranges))); err != nil {
		t.Fatal(err)
	}
	for _, r := range ranges {
		if err := varbin.Write(bw, binary.BigEndian, r[0].AsSlice()); err != nil {
			t.Fatal(err)
		}
		if err := varbin.Write(bw, binary.BigEndian, r[1].AsSlice()); err != nil {
			t.Fatal(err)
		}
	}
	if err := bw.WriteByte(ruleItemFinal); err != nil {
		t.Fatal(err)
	}
	if err := bw.Flush(); err != nil {
		t.Fatal(err)
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	return append([]byte{'S', 'R', 'S', maxSupportedVersion}, compressed.Bytes()...)
}

func newSlice0SiSet(t *testing.T, sources map[string]*RuleSource) *SiSet {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	p := &SiSet{
		sources:         sources,
		localConfigFile: filepath.Join(t.TempDir(), "sources.json"),
		httpClient:      &http.Client{},
		ctx:             ctx,
		cancel:          cancel,
	}
	p.matcher.Store(netlist.NewList())
	t.Cleanup(func() { _ = p.Close() })
	return p
}

func writeSlice0IPFile(t *testing.T, path string, data []byte) {
	t.Helper()
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestSlice0SiSetSRSCompositionReloadAndClose(t *testing.T) {
	dir := t.TempDir()
	first := buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("10.0.0.0"), netip.MustParseAddr("10.0.0.255")},
	})
	second := buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("192.0.2.0"), netip.MustParseAddr("192.0.2.255")},
	})
	firstPath := filepath.Join(dir, "first.srs")
	secondPath := filepath.Join(dir, "second.srs")
	badPath := filepath.Join(dir, "bad.srs")
	writeSlice0IPFile(t, firstPath, first)
	writeSlice0IPFile(t, secondPath, second)
	writeSlice0IPFile(t, badPath, []byte("not an srs"))

	p := newSlice0SiSet(t, map[string]*RuleSource{
		"first": {
			Name: "first", Type: "geoip", Files: firstPath, Enabled: true, RuleCount: 1,
		},
		"second": {
			Name: "second", Type: "geosite-ip", Files: secondPath, Enabled: true, RuleCount: 1,
		},
		"bad": {
			Name: "bad", Type: "broken", Files: badPath, Enabled: true,
		},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	matcher := p.GetIPMatcher()
	for _, tt := range []struct {
		addr string
		want bool
	}{
		{"10.0.0.1", true},
		{"192.0.2.1", true},
		{"10.0.1.1", false},
		{"198.51.100.1", false},
	} {
		addr := netip.MustParseAddr(tt.addr)
		if got := p.Match(addr); got != tt.want {
			t.Errorf("Match(%s) = %v, want %v", tt.addr, got, tt.want)
		}
		if got := matcher.Match(addr); got != tt.want {
			t.Errorf("GetIPMatcher().Match(%s) = %v, want %v", tt.addr, got, tt.want)
		}
	}

	updated := buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("198.51.100.0"), netip.MustParseAddr("198.51.100.255")},
	})
	writeSlice0IPFile(t, firstPath, updated)
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if !p.Match(netip.MustParseAddr("198.51.100.1")) {
		t.Fatal("reload did not publish the updated IP snapshot")
	}
	if p.Match(netip.MustParseAddr("10.0.0.1")) {
		t.Fatal("reload retained a removed IP prefix")
	}

	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestSlice0SiSetOnlineInvalidSourceDoesNotOverwriteFile(t *testing.T) {
	dir := t.TempDir()
	oldData := buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("203.0.113.0"), netip.MustParseAddr("203.0.113.255")},
	})
	newData := buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("198.18.0.0"), netip.MustParseAddr("198.18.0.255")},
	})
	body := oldData
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(body)
	}))
	defer server.Close()

	path := filepath.Join(dir, "online.srs")
	writeSlice0IPFile(t, path, oldData)
	p := newSlice0SiSet(t, map[string]*RuleSource{
		"online": {
			Name: "online", Type: "subscription", Files: path, URL: server.URL,
			Enabled: true, RuleCount: 1,
		},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}

	body = newData
	if err := p.downloadAndUpdateLocalFile(context.Background(), "online"); err != nil {
		t.Fatal(err)
	}
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if !p.Match(netip.MustParseAddr("198.18.0.1")) {
		t.Fatal("valid online IP reload was not published")
	}
	if p.Match(netip.MustParseAddr("203.0.113.1")) {
		t.Fatal("valid online IP reload retained the old prefix")
	}

	body = []byte("invalid")
	if err := p.downloadAndUpdateLocalFile(context.Background(), "online"); err == nil {
		t.Fatal("invalid online IP source must be rejected")
	}
	disk, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(disk, newData) {
		t.Fatal("invalid online IP source replaced the established file")
	}
}
