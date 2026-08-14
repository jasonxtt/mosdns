package si_set

import (
	"errors"
	"net/netip"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
)

type slice2IPSnapshot struct {
	matchResult bool
	matchErr    error
	closed      atomic.Int32
	started     chan struct{}
	release     <-chan struct{}
	startOnce   sync.Once
}

func (s *slice2IPSnapshot) Match(string) (bool, error) {
	if s.started != nil {
		s.startOnce.Do(func() { close(s.started) })
		<-s.release
	}
	if s.closed.Load() != 0 {
		return false, errors.New("Rust snapshot closed during Match")
	}
	return s.matchResult, s.matchErr
}

func (s *slice2IPSnapshot) Len() (uint64, error) { return 0, nil }

func (s *slice2IPSnapshot) Close() error {
	s.closed.Add(1)
	return nil
}

func TestSlice2SiSetBuildsRustFromAcceptedPrefixStream(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustIPSnapshotBuilder
	t.Cleanup(func() { rustIPSnapshotBuilder = oldBuilder })

	var gotPrefixes []string
	rustIPSnapshotBuilder = func(prefixes []string) (matcher_adapter.IPSnapshot, error) {
		gotPrefixes = append([]string(nil), prefixes...)
		return &slice2IPSnapshot{matchResult: true}, nil
	}

	dir := t.TempDir()
	path := dir + "/rules.srs"
	writeSlice2SiFile(t, path, buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("192.0.2.0"), netip.MustParseAddr("192.0.2.255")},
	}))
	p := newSlice0SiSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if strings.Join(gotPrefixes, "\n") != "192.0.2.0/24" {
		t.Fatalf("Rust prefixes = %v, want [192.0.2.0/24]", gotPrefixes)
	}
	if !p.Match(netip.MustParseAddr("198.51.100.1")) {
		t.Fatal("healthy Rust snapshot was not selected for Match")
	}
}

func TestSlice2SiSetRustFailurePublishesNewGoGeneration(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustIPSnapshotBuilder
	t.Cleanup(func() { rustIPSnapshotBuilder = oldBuilder })

	oldRust := &slice2IPSnapshot{matchResult: false}
	builds := 0
	rustIPSnapshotBuilder = func([]string) (matcher_adapter.IPSnapshot, error) {
		builds++
		if builds == 1 {
			return oldRust, nil
		}
		return nil, errors.New("injected Rust build failure")
	}

	dir := t.TempDir()
	path := dir + "/rules.srs"
	p := newSlice0SiSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true},
	})
	writeSlice2SiFile(t, path, buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("10.0.0.0"), netip.MustParseAddr("10.0.0.255")},
	}))
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	writeSlice2SiFile(t, path, buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("192.0.2.0"), netip.MustParseAddr("192.0.2.255")},
	}))
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if !p.Match(netip.MustParseAddr("192.0.2.1")) {
		t.Fatal("Rust build failure did not publish the new Go generation")
	}
	if p.Match(netip.MustParseAddr("10.0.0.1")) {
		t.Fatal("Rust build failure retained the old Go generation")
	}
	if oldRust.closed.Load() != 1 {
		t.Fatalf("old Rust snapshot closed %d times, want once", oldRust.closed.Load())
	}
}

func TestSlice2SiSetRuntimeFailureFallsBackToSameGeneration(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustIPSnapshotBuilder
	t.Cleanup(func() { rustIPSnapshotBuilder = oldBuilder })

	rustSnapshot := &slice2IPSnapshot{matchErr: errors.New("runtime matcher failure")}
	rustIPSnapshotBuilder = func([]string) (matcher_adapter.IPSnapshot, error) {
		return rustSnapshot, nil
	}

	dir := t.TempDir()
	path := dir + "/rules.srs"
	writeSlice2SiFile(t, path, buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("198.51.100.0"), netip.MustParseAddr("198.51.100.255")},
	}))
	p := newSlice0SiSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if !p.Match(netip.MustParseAddr("198.51.100.1")) {
		t.Fatal("runtime Rust failure did not fall back to the same Go generation")
	}
	if rustSnapshot.closed.Load() != 1 {
		t.Fatalf("runtime-failed Rust snapshot closed %d times, want once", rustSnapshot.closed.Load())
	}
}

func TestSlice2SiSetReloadDoesNotCloseReplacementDuringMatch(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustIPSnapshotBuilder
	t.Cleanup(func() { rustIPSnapshotBuilder = oldBuilder })

	started := make(chan struct{})
	release := make(chan struct{})
	oldRust := &slice2IPSnapshot{
		matchErr: errors.New("runtime matcher failure"),
		started:  started,
		release:  release,
	}
	newRust := &slice2IPSnapshot{matchResult: true}
	builds := 0
	rustIPSnapshotBuilder = func([]string) (matcher_adapter.IPSnapshot, error) {
		builds++
		if builds == 1 {
			return oldRust, nil
		}
		return newRust, nil
	}

	dir := t.TempDir()
	path := dir + "/rules.srs"
	p := newSlice0SiSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true},
	})
	writeSlice2SiFile(t, path, buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("10.0.0.0"), netip.MustParseAddr("10.0.0.255")},
	}))
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}

	matchDone := make(chan bool, 1)
	go func() { matchDone <- p.Match(netip.MustParseAddr("10.0.0.1")) }()
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatal("old Rust generation did not enter Match")
	}

	writeSlice2SiFile(t, path, buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("192.0.2.0"), netip.MustParseAddr("192.0.2.255")},
	}))
	reloadDone := make(chan error, 1)
	go func() { reloadDone <- p.reloadAllRules() }()
	select {
	case err := <-reloadDone:
		t.Fatalf("reload completed while an old Match was in flight: %v", err)
	case <-time.After(50 * time.Millisecond):
	}
	close(release)

	select {
	case matched := <-matchDone:
		if !matched {
			t.Fatal("in-flight Match did not fall back to its same-generation Go matcher")
		}
	case <-time.After(time.Second):
		t.Fatal("in-flight Match did not finish")
	}
	if err := <-reloadDone; err != nil {
		t.Fatal(err)
	}
	if oldRust.closed.Load() != 1 {
		t.Fatalf("old Rust generation closed %d times, want once", oldRust.closed.Load())
	}
	if newRust.closed.Load() != 0 {
		t.Fatal("reload closed the replacement Rust generation")
	}
}

func TestSlice2SiSetEmptyRustGenerationClosesIdempotently(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustIPSnapshotBuilder
	t.Cleanup(func() { rustIPSnapshotBuilder = oldBuilder })

	emptyRust := &slice2IPSnapshot{}
	var gotPrefixes []string
	rustIPSnapshotBuilder = func(prefixes []string) (matcher_adapter.IPSnapshot, error) {
		gotPrefixes = append([]string(nil), prefixes...)
		return emptyRust, nil
	}

	dir := t.TempDir()
	path := dir + "/empty.srs"
	writeSlice2SiFile(t, path, buildSlice0IPSRS(t, nil))
	p := newSlice0SiSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if len(gotPrefixes) != 0 {
		t.Fatalf("empty Rust IP prefixes = %v, want empty", gotPrefixes)
	}
	if p.Match(netip.MustParseAddr("192.0.2.1")) {
		t.Fatal("empty Rust generation matched an IP")
	}
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
	if emptyRust.closed.Load() != 1 {
		t.Fatalf("empty Rust generation closed %d times, want once", emptyRust.closed.Load())
	}
}

func writeSlice2SiFile(t *testing.T, path string, data []byte) {
	t.Helper()
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
