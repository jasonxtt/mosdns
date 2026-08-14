package sd_set

import (
	"errors"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
)

type slice2DomainSnapshot struct {
	matchResult bool
	matchErr    error
	closed      atomic.Int32
	started     chan struct{}
	release     <-chan struct{}
	startOnce   sync.Once
}

func (s *slice2DomainSnapshot) Match(string) (bool, error) {
	if s.started != nil {
		s.startOnce.Do(func() { close(s.started) })
		<-s.release
	}
	if s.closed.Load() != 0 {
		return false, errors.New("Rust snapshot closed during Match")
	}
	return s.matchResult, s.matchErr
}

func (s *slice2DomainSnapshot) Len() (uint64, error) { return 0, nil }

func (s *slice2DomainSnapshot) Close() error {
	s.closed.Add(1)
	return nil
}

func TestSlice2SdSetBuildsRustFromAcceptedEnableRegexpRules(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustDomainSnapshotBuilder
	t.Cleanup(func() { rustDomainSnapshotBuilder = oldBuilder })

	var gotRules []string
	rustDomainSnapshotBuilder = func(rules []string) (matcher_adapter.DomainSnapshot, error) {
		gotRules = append([]string(nil), rules...)
		return &slice2DomainSnapshot{matchResult: true}, nil
	}

	dir := t.TempDir()
	path := dir + "/rules.srs"
	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t,
		[]string{"full.example"},
		[]string{"suffix.example"},
		[]string{"keyword"},
		[]string{`^regexp\.example$`},
	))
	p := newSlice0SdSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true, EnableRegexp: true},
	})

	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	want := []string{
		"full:full.example",
		"domain:suffix.example",
		"keyword:keyword",
		`regexp:^regexp\.example$`,
	}
	if strings.Join(gotRules, "\n") != strings.Join(want, "\n") {
		t.Fatalf("Rust rules = %v, want %v", gotRules, want)
	}
	if _, ok := p.Match("not-in-go.example"); !ok {
		t.Fatal("healthy Rust snapshot was not selected for Match")
	}
}

func TestSlice2SdSetAcceptedStreamHonorsDisabledRegexp(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustDomainSnapshotBuilder
	t.Cleanup(func() { rustDomainSnapshotBuilder = oldBuilder })

	var gotRules []string
	rustDomainSnapshotBuilder = func(rules []string) (matcher_adapter.DomainSnapshot, error) {
		gotRules = append([]string(nil), rules...)
		return &slice2DomainSnapshot{}, nil
	}

	dir := t.TempDir()
	path := dir + "/rules.srs"
	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t,
		[]string{"full.example"},
		[]string{"suffix.example"},
		[]string{"keyword"},
		[]string{`^regexp\.example$`},
	))
	p := newSlice0SdSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true, EnableRegexp: false},
	})

	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	want := []string{
		"full:full.example",
		"domain:suffix.example",
		"keyword:keyword",
	}
	if strings.Join(gotRules, "\n") != strings.Join(want, "\n") {
		t.Fatalf("Rust rules = %v, want %v", gotRules, want)
	}
	if _, ok := p.Match("regexp.example"); ok {
		t.Fatal("disabled regexp unexpectedly matched through Go generation")
	}
}

func TestSlice2SdSetRustFailurePublishesNewGoGeneration(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustDomainSnapshotBuilder
	t.Cleanup(func() { rustDomainSnapshotBuilder = oldBuilder })

	oldRust := &slice2DomainSnapshot{matchResult: false}
	builds := 0
	rustDomainSnapshotBuilder = func([]string) (matcher_adapter.DomainSnapshot, error) {
		builds++
		if builds == 1 {
			return oldRust, nil
		}
		return nil, errors.New("injected Rust build failure")
	}

	dir := t.TempDir()
	path := dir + "/rules.srs"
	p := newSlice0SdSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true},
	})
	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t, []string{"old.example"}, nil, nil, nil))
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t, []string{"new.example"}, nil, nil, nil))
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if _, ok := p.Match("new.example"); !ok {
		t.Fatal("Rust build failure did not publish the new Go generation")
	}
	if _, ok := p.Match("old.example"); ok {
		t.Fatal("Rust build failure retained the old Go generation")
	}
	if oldRust.closed.Load() != 1 {
		t.Fatalf("old Rust snapshot closed %d times, want once", oldRust.closed.Load())
	}
}

func TestSlice2SdSetRuntimeFailureFallsBackToSameGeneration(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustDomainSnapshotBuilder
	t.Cleanup(func() { rustDomainSnapshotBuilder = oldBuilder })

	rustSnapshot := &slice2DomainSnapshot{matchErr: errors.New("runtime matcher failure")}
	rustDomainSnapshotBuilder = func([]string) (matcher_adapter.DomainSnapshot, error) {
		return rustSnapshot, nil
	}

	dir := t.TempDir()
	path := dir + "/rules.srs"
	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t, []string{"same-generation.example"}, nil, nil, nil))
	p := newSlice0SdSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if _, ok := p.Match("same-generation.example"); !ok {
		t.Fatal("runtime Rust failure did not fall back to the same Go generation")
	}
	if rustSnapshot.closed.Load() != 1 {
		t.Fatalf("runtime-failed Rust snapshot closed %d times, want once", rustSnapshot.closed.Load())
	}
}

func TestSlice2SdSetReloadDoesNotCloseReplacementDuringMatch(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustDomainSnapshotBuilder
	t.Cleanup(func() { rustDomainSnapshotBuilder = oldBuilder })

	started := make(chan struct{})
	release := make(chan struct{})
	oldRust := &slice2DomainSnapshot{
		matchErr: errors.New("runtime matcher failure"),
		started:  started,
		release:  release,
	}
	newRust := &slice2DomainSnapshot{matchResult: true}
	builds := 0
	rustDomainSnapshotBuilder = func([]string) (matcher_adapter.DomainSnapshot, error) {
		builds++
		if builds == 1 {
			return oldRust, nil
		}
		return newRust, nil
	}

	dir := t.TempDir()
	path := dir + "/rules.srs"
	p := newSlice0SdSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true},
	})
	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t, []string{"old.example"}, nil, nil, nil))
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}

	matchDone := make(chan bool, 1)
	go func() {
		_, matched := p.Match("old.example")
		matchDone <- matched
	}()
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatal("old Rust generation did not enter Match")
	}

	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t, []string{"new.example"}, nil, nil, nil))
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

func TestSlice2SdSetEmptyRustGenerationClosesIdempotently(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustDomainSnapshotBuilder
	t.Cleanup(func() { rustDomainSnapshotBuilder = oldBuilder })

	emptyRust := &slice2DomainSnapshot{}
	var gotRules []string
	rustDomainSnapshotBuilder = func(rules []string) (matcher_adapter.DomainSnapshot, error) {
		gotRules = append([]string(nil), rules...)
		return emptyRust, nil
	}

	dir := t.TempDir()
	path := dir + "/empty.srs"
	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t, nil, nil, nil, nil))
	p := newSlice0SdSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if len(gotRules) != 0 {
		t.Fatalf("empty Rust domain rules = %v, want empty", gotRules)
	}
	if _, ok := p.Match("empty.example"); ok {
		t.Fatal("empty Rust generation matched a domain")
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

func writeSlice2SdFile(t *testing.T, path string, data []byte) {
	t.Helper()
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
