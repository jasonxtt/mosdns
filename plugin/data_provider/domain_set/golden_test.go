/*
 * Copyright (C) 2020-2026, IrineSistiana
 *
 * Slice 0 golden fixtures for the domain_set provider. These freeze the Go
 * text/SRS input parsing, provider composition, atomic reload, and
 * RuleExporter behavior that the Rust integration must preserve. SRS bytes
 * are built from the same sing `scdomain`/`varbin` codecs the loader reads.
 */

package domain_set

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	scdomain "github.com/sagernet/sing/common/domain"
	"github.com/sagernet/sing/common/varbin"
)

// buildDomainSRS encodes a v3 SRS rule set with the given categories using the
// same layout `tryLoadSRS`/`readRuleCompat` consume: magic, version, zlib
// stream containing uvarint count + one default-mode rule.
func buildDomainSRS(t *testing.T, domains, suffixes, keywords, regexes []string) []byte {
	t.Helper()
	var compressed bytes.Buffer
	zw := zlib.NewWriter(&compressed)
	bw := bufio.NewWriter(zw) // varbin.Writer requires WriteByte; bufio provides it
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
	if len(domains) > 0 || len(suffixes) > 0 {
		writeByte(ruleItemDomain)
		dm := scdomain.NewMatcher(domains, suffixes, true)
		if err := dm.Write(bw); err != nil {
			t.Fatalf("write domain matcher: %v", err)
		}
	}
	if len(keywords) > 0 {
		writeByte(ruleItemDomainKeyword)
		if err := varbin.Write(bw, binary.BigEndian, keywords); err != nil {
			t.Fatalf("write keywords: %v", err)
		}
	}
	if len(regexes) > 0 {
		writeByte(ruleItemDomainRegex)
		if err := varbin.Write(bw, binary.BigEndian, regexes); err != nil {
			t.Fatalf("write regexes: %v", err)
		}
	}
	writeByte(ruleItemFinal)
	if err := bw.Flush(); err != nil {
		t.Fatal(err)
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	out := make([]byte, 0, 3+1+compressed.Len())
	out = append(out, magicBytes[:]...)
	out = append(out, ruleSetVersionCurrent)
	out = append(out, compressed.Bytes()...)
	return out
}

func newTestDomainSet(mix *domain.MixMatcher[struct{}]) *DomainSet {
	if mix == nil {
		mix = domain.NewDomainMixMatcher()
	}
	return &DomainSet{
		mixM:   mix,
		otherM: make([]domain.Matcher[struct{}], 0),
	}
}

func TestGoldenDomainSetTextLoad(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "rules.txt")
	content := `# whole-line comment
full:exact.example

domain:example.com
keyword:tracker
regexp:[        # invalid regexp: Add fails, row is skipped
full:tagged     # inline comment is NOT stripped by loadFileInternal
bare line with spaces   # Add succeeds for domain type, so it is collected
`
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}

	d := newTestDomainSet(nil)
	rules, err := d.loadFileInternal(path)
	if err != nil {
		t.Fatal(err)
	}
	// Contract: whole-line comments and empty lines are skipped; a row whose
	// Add fails (regexp compile) is skipped; but inline "# ..." text is NOT
	// stripped here (unlike pkg/matcher/domain.LoadFromTextReader), so the
	// full:/domain: rows with trailing text are added with that text in the
	// pattern and therefore do not match the bare domain.
	wantRules := []string{
		"full:exact.example",
		"domain:example.com",
		"keyword:tracker",
		"full:tagged     # inline comment is NOT stripped by loadFileInternal",
		"bare line with spaces   # Add succeeds for domain type, so it is collected",
	}
	if len(rules) != len(wantRules) {
		t.Fatalf("loaded %d rules, want %d: %v", len(rules), len(wantRules), rules)
	}
	for i, w := range wantRules {
		if rules[i] != w {
			t.Errorf("rule[%d] = %q, want %q", i, rules[i], w)
		}
	}
	tests := []struct {
		q    string
		want bool
	}{
		{"exact.example", true},
		{"sub.example.com", true},
		{"atracker.io", true},
		{"tagged", false}, // pattern kept the inline comment
		{"bare line with spaces", false},
		{"not-in-list.org", false},
	}
	for _, tt := range tests {
		if _, ok := d.Match(tt.q); ok != tt.want {
			t.Errorf("Match(%q) = %v, want %v", tt.q, ok, tt.want)
		}
	}
}

func TestGoldenDomainSetSRSLoad(t *testing.T) {
	srs := buildDomainSRS(t,
		[]string{"full.example"},      // -> full:full.example
		[]string{"suffix.example"},    // -> domain:suffix.example
		[]string{"kwterm"},            // -> keyword:kwterm
		[]string{"^regex\\.example$"}, // -> regexp:^regex\.example$
	)
	dir := t.TempDir()
	path := filepath.Join(dir, "rules.srs")
	if err := os.WriteFile(path, srs, 0o644); err != nil {
		t.Fatal(err)
	}

	d := newTestDomainSet(nil)
	rules, err := d.loadFileInternal(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(rules) != 0 {
		t.Fatalf("SRS load must not surface raw text rules, got %v", rules)
	}
	var rustRules []string
	d = newTestDomainSet(nil)
	if rules, err := d.loadFileInternalWithRules(path, &rustRules); err != nil {
		t.Fatal(err)
	} else if len(rules) != 0 {
		t.Fatalf("SRS load must not surface raw text rules, got %v", rules)
	}
	wantRustRules := []string{
		"full:full.example",
		"domain:suffix.example",
		"keyword:kwterm",
		"regexp:^regex\\.example$",
	}
	if fmt.Sprint(rustRules) != fmt.Sprint(wantRustRules) {
		t.Fatalf("SRS Rust rules = %v, want %v", rustRules, wantRustRules)
	}
	tests := []struct {
		q    string
		want bool
	}{
		{"full.example", true},
		{"full.example.", true},
		{"sub.suffix.example", true},
		{"kwtermanywhere", true},
		{"regex.example", true},
		{"unrelated.example", false},
	}
	for _, tt := range tests {
		if _, ok := d.Match(tt.q); ok != tt.want {
			t.Errorf("Match(%q) = %v, want %v", tt.q, ok, tt.want)
		}
	}
}

type fakeRustDomainMatcher struct {
	pattern   string
	closed    atomic.Int32
	calls     atomic.Int32
	closeHook func()
}

func (m *fakeRustDomainMatcher) Match(s string) (bool, error) {
	m.calls.Add(1)
	if m.closed.Load() != 0 {
		return false, errors.New("fake matcher closed")
	}
	return strings.TrimSuffix(strings.TrimSpace(s), ".") == m.pattern, nil
}

func (m *fakeRustDomainMatcher) Close() error {
	if m.closeHook != nil {
		m.closeHook()
	}
	m.closed.Add(1)
	return nil
}

func postDomainSet(t *testing.T, d *DomainSet, values ...string) *httptest.ResponseRecorder {
	t.Helper()
	r := httptest.NewRecorder()
	body, err := json.Marshal(domainPayload{Values: values})
	if err != nil {
		t.Fatal(err)
	}
	d.api().ServeHTTP(r, httptest.NewRequest(http.MethodPost, "/post", bytes.NewReader(body)))
	return r
}

func TestDomainSetPostRustBuildFailurePublishesGoOnlyGeneration(t *testing.T) {
	oldBuilder := rustDomainMatcherBuilder
	t.Cleanup(func() { rustDomainMatcherBuilder = oldBuilder })
	rustDomainMatcherBuilder = func([]string) (RustMatcher, error) {
		return nil, errors.New("injected Rust build failure")
	}

	dir := t.TempDir()
	ruleFile := filepath.Join(dir, "rules.txt")
	if err := os.WriteFile(ruleFile, []byte("full:old.example\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	d := newTestDomainSet(nil)
	d.ruleFile = ruleFile
	d.rules = []string{"full:old.example"}
	d.rustRules = append([]string(nil), d.rules...)
	if err := d.mixM.Add("full:old.example", struct{}{}); err != nil {
		t.Fatal(err)
	}
	oldRust := &fakeRustDomainMatcher{pattern: "old.example"}
	oldRust.closeHook = func() {
		d.mu.RLock()
		published := d.rustMatcher == nil && len(d.rules) == 1 && d.rules[0] == "full:new.example"
		d.mu.RUnlock()
		if !published {
			t.Errorf("old Rust handle closed before the new Go-only generation was visible")
		}
	}
	d.rustMatcher = oldRust

	r := postDomainSet(t, d, "full:new.example")
	if r.Code != http.StatusOK {
		t.Fatalf("POST status = %d, want %d, body=%s", r.Code, http.StatusOK, r.Body.String())
	}
	if _, ok := d.Match("new.example"); !ok {
		t.Fatal("the new Go generation must be active after a Rust build failure")
	}
	if _, ok := d.Match("old.example"); ok {
		t.Fatal("the old Rust snapshot must not be mixed with the new Go generation")
	}
	rules, err := d.GetRules()
	if err != nil {
		t.Fatal(err)
	}
	if fmt.Sprint(rules) != "[full:new.example]" {
		t.Fatalf("rules after Rust build failure = %v", rules)
	}
	d.mu.RLock()
	rustNil := d.rustMatcher == nil
	d.mu.RUnlock()
	if !rustNil {
		t.Fatal("Rust build failure must publish a Go-only generation")
	}
	if oldRust.closed.Load() != 1 {
		t.Fatalf("old Rust handle close count = %d, want 1", oldRust.closed.Load())
	}
	disk, err := os.ReadFile(ruleFile)
	if err != nil {
		t.Fatal(err)
	}
	if string(disk) != "full:new.example\n" {
		t.Fatalf("persisted rules = %q, want new generation", disk)
	}
}

func TestDomainSetPostBuildDoesNotBlockMatch(t *testing.T) {
	oldBuilder := rustDomainMatcherBuilder
	t.Cleanup(func() { rustDomainMatcherBuilder = oldBuilder })

	buildStarted := make(chan struct{})
	releaseBuild := make(chan struct{})
	var releaseOnce sync.Once
	release := func() { releaseOnce.Do(func() { close(releaseBuild) }) }
	t.Cleanup(release)
	rustDomainMatcherBuilder = func([]string) (RustMatcher, error) {
		close(buildStarted)
		<-releaseBuild
		return nil, errors.New("injected Rust build failure")
	}

	d := newTestDomainSet(nil)
	d.ruleFile = filepath.Join(t.TempDir(), "rules.txt")
	if err := os.WriteFile(d.ruleFile, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	d.rules = []string{"full:go-only.example"}
	d.rustRules = append([]string(nil), d.rules...)
	if err := d.mixM.Add("full:go-only.example", struct{}{}); err != nil {
		t.Fatal(err)
	}
	oldRust := &fakeRustDomainMatcher{pattern: "old.example"}
	d.rustMatcher = oldRust

	postDone := make(chan *httptest.ResponseRecorder, 1)
	go func() { postDone <- postDomainSet(t, d, "full:new.example") }()
	select {
	case <-buildStarted:
	case <-time.After(time.Second):
		t.Fatal("POST did not reach the Rust candidate build")
	}

	matchDone := make(chan bool, 1)
	go func() {
		_, matched := d.Match("old.example")
		matchDone <- matched
	}()
	select {
	case matched := <-matchDone:
		if !matched {
			t.Fatal("the old generation must remain available during candidate build")
		}
	case <-time.After(time.Second):
		t.Fatal("Rust candidate build must not hold the domain match state lock")
	}

	release()
	select {
	case r := <-postDone:
		if r.Code != http.StatusOK {
			t.Fatalf("POST status = %d, body=%s", r.Code, r.Body.String())
		}
	case <-time.After(time.Second):
		t.Fatal("POST did not publish after candidate build completed")
	}
	if _, matched := d.Match("new.example"); !matched {
		t.Fatal("new Go-only generation must match after publication")
	}
	if _, matched := d.Match("old.example"); matched {
		t.Fatal("old generation must be replaced after publication")
	}
	if oldRust.closed.Load() != 1 {
		t.Fatalf("old Rust handle close count = %d, want 1", oldRust.closed.Load())
	}
}

func TestDomainSetPostPersistenceFailureRetainsGenerationAndClosesCandidate(t *testing.T) {
	oldBuilder := rustDomainMatcherBuilder
	t.Cleanup(func() { rustDomainMatcherBuilder = oldBuilder })
	candidate := &fakeRustDomainMatcher{pattern: "new.example"}
	rustDomainMatcherBuilder = func([]string) (RustMatcher, error) {
		return candidate, nil
	}

	d := newTestDomainSet(nil)
	d.ruleFile = filepath.Join(t.TempDir(), "missing", "rules.txt")
	if err := d.mixM.Add("full:old.example", struct{}{}); err != nil {
		t.Fatal(err)
	}
	d.rules = []string{"full:old.example"}
	oldRust := &fakeRustDomainMatcher{pattern: "old.example"}
	d.rustMatcher = oldRust

	r := postDomainSet(t, d, "full:new.example")
	if r.Code != http.StatusInternalServerError {
		t.Fatalf("POST status = %d, want %d", r.Code, http.StatusInternalServerError)
	}
	if _, ok := d.Match("old.example"); !ok {
		t.Fatal("persistence failure must retain the old Go/Rust generation")
	}
	if _, ok := d.Match("new.example"); ok {
		t.Fatal("persistence failure must not publish the new generation")
	}
	if candidate.closed.Load() != 1 {
		t.Fatalf("new Rust candidate close count = %d, want 1", candidate.closed.Load())
	}
	if oldRust.closed.Load() != 0 {
		t.Fatalf("old Rust handle close count = %d, want 0", oldRust.closed.Load())
	}
}

func TestDomainSetPostInvalidJSONRetainsGeneration(t *testing.T) {
	oldBuilder := rustDomainMatcherBuilder
	t.Cleanup(func() { rustDomainMatcherBuilder = oldBuilder })
	var buildCalls atomic.Int32
	rustDomainMatcherBuilder = func([]string) (RustMatcher, error) {
		buildCalls.Add(1)
		return nil, errors.New("Rust builder must not run for invalid JSON")
	}

	d := newTestDomainSet(nil)
	d.ruleFile = filepath.Join(t.TempDir(), "rules.txt")
	if err := d.mixM.Add("full:old.example", struct{}{}); err != nil {
		t.Fatal(err)
	}
	d.rules = []string{"full:old.example"}
	oldRust := &fakeRustDomainMatcher{pattern: "old.example"}
	d.rustMatcher = oldRust

	r := httptest.NewRecorder()
	d.api().ServeHTTP(r, httptest.NewRequest(http.MethodPost, "/post", strings.NewReader("{")))
	if r.Code != http.StatusBadRequest {
		t.Fatalf("POST status = %d, want %d", r.Code, http.StatusBadRequest)
	}
	if buildCalls.Load() != 0 {
		t.Fatalf("Rust builder calls = %d, want 0", buildCalls.Load())
	}
	if _, ok := d.Match("old.example"); !ok {
		t.Fatal("invalid JSON must retain the old generation")
	}
	if oldRust.closed.Load() != 0 {
		t.Fatalf("old Rust handle close count = %d, want 0", oldRust.closed.Load())
	}
}

func TestDomainSetNonASCIIQueryUsesGoWithoutDisablingRust(t *testing.T) {
	d := newTestDomainSet(nil)
	if err := d.mixM.Add("full:例.example", struct{}{}); err != nil {
		t.Fatal(err)
	}
	rust := &fakeRustDomainMatcher{pattern: "ascii.example"}
	d.rustMatcher = rust

	if _, ok := d.Match("例.example."); !ok {
		t.Fatal("non-ASCII query did not use the paired Go matcher")
	}
	if got := rust.calls.Load(); got != 0 {
		t.Fatalf("Rust calls for non-ASCII query = %d, want 0", got)
	}
	if _, ok := d.Match("ascii.example."); !ok {
		t.Fatal("ASCII query did not use Rust after the Go fallback")
	}
	if got := rust.calls.Load(); got != 1 {
		t.Fatalf("Rust calls after ASCII query = %d, want 1", got)
	}
	if got := rust.closed.Load(); got != 0 {
		t.Fatalf("non-ASCII query disabled Rust handle, close count = %d", got)
	}
}

func TestDomainSetConcurrentPostsPublishMatchingGenerations(t *testing.T) {
	oldBuilder := rustDomainMatcherBuilder
	t.Cleanup(func() { rustDomainMatcherBuilder = oldBuilder })
	aStarted := make(chan struct{})
	releaseA := make(chan struct{})
	rustDomainMatcherBuilder = func(rules []string) (RustMatcher, error) {
		pattern := strings.TrimPrefix(rules[0], "full:")
		if pattern == "a.example" {
			close(aStarted)
			<-releaseA
		}
		return &fakeRustDomainMatcher{pattern: pattern}, nil
	}

	dir := t.TempDir()
	d := newTestDomainSet(nil)
	d.ruleFile = filepath.Join(dir, "rules.txt")
	if err := os.WriteFile(d.ruleFile, nil, 0o644); err != nil {
		t.Fatal(err)
	}

	aDone := make(chan *httptest.ResponseRecorder, 1)
	go func() {
		r := postDomainSet(t, d, "full:a.example")
		aDone <- r
	}()
	select {
	case <-aStarted:
	case <-time.After(time.Second):
		t.Fatal("first POST did not reach the Rust build")
	}

	bDone := make(chan *httptest.ResponseRecorder, 1)
	go func() {
		r := postDomainSet(t, d, "full:b.example")
		bDone <- r
	}()
	select {
	case r := <-bDone:
		t.Fatalf("second POST completed before the first generation: status=%d", r.Code)
	case <-time.After(50 * time.Millisecond):
	}
	close(releaseA)

	select {
	case r := <-aDone:
		if r.Code != http.StatusOK {
			t.Fatalf("first POST status = %d", r.Code)
		}
	case <-time.After(time.Second):
		t.Fatal("first POST did not finish")
	}
	select {
	case r := <-bDone:
		if r.Code != http.StatusOK {
			t.Fatalf("second POST status = %d", r.Code)
		}
	case <-time.After(time.Second):
		t.Fatal("second POST did not finish")
	}

	if _, ok := d.Match("b.example"); !ok {
		t.Fatal("final Go/Rust generation must match the second POST")
	}
	if _, ok := d.Match("a.example"); ok {
		t.Fatal("final generation must not retain the first POST")
	}
}

// stubOtherMatcher is a trivial domain.Matcher used to prove composition
// ordering: the provider's own set is consulted before referenced sets.
type stubOtherMatcher struct {
	domain string
}

func (s stubOtherMatcher) Match(q string) (struct{}, bool) {
	if q == s.domain {
		return struct{}{}, true
	}
	return struct{}{}, false
}

func TestGoldenDomainSetComposition(t *testing.T) {
	d := newTestDomainSet(nil)
	if err := d.mixM.Add("full:self.example", struct{}{}); err != nil {
		t.Fatal(err)
	}
	d.otherM = append(d.otherM, stubOtherMatcher{domain: "other.example"})

	tests := []struct {
		q    string
		want bool
	}{
		{"self.example", true},
		{"other.example", true}, // via referenced matcher
		{"neither.example", false},
	}
	for _, tt := range tests {
		if _, ok := d.Match(tt.q); ok != tt.want {
			t.Errorf("Match(%q) = %v, want %v", tt.q, ok, tt.want)
		}
	}
}

func TestGoldenDomainSetReloadViaPost(t *testing.T) {
	dir := t.TempDir()
	ruleFile := filepath.Join(dir, "rules.txt")
	if err := os.WriteFile(ruleFile, []byte("full:old.example\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	d := newTestDomainSet(nil)
	d.ruleFile = ruleFile
	if err := d.mixM.Add("full:old.example", struct{}{}); err != nil {
		t.Fatal(err)
	}

	var notified atomic.Int32
	d.Subscribe(func() { notified.Add(1) })

	r := httptest.NewRecorder()
	body := `{"values":["full:new.example","regexp:[","domain:tail.example"]}`
	req := httptest.NewRequest(http.MethodPost, "/post", bytes.NewBufferString(body))
	d.api().ServeHTTP(r, req)
	if r.Code != http.StatusOK {
		t.Fatalf("POST status = %d, body=%s", r.Code, r.Body.String())
	}

	// Invalid regexp is ignored, valid rules replace the snapshot atomically.
	tests := []struct {
		q    string
		want bool
	}{
		{"new.example", true},
		{"sub.tail.example", true},
		{"old.example", false}, // replaced
	}
	for _, tt := range tests {
		if _, ok := d.Match(tt.q); ok != tt.want {
			t.Errorf("Match(%q) = %v, want %v", tt.q, ok, tt.want)
		}
	}

	rules, err := d.GetRules()
	if err != nil {
		t.Fatal(err)
	}
	if len(rules) != 2 || rules[0] != "full:new.example" || rules[1] != "domain:tail.example" {
		t.Fatalf("GetRules() = %v", rules)
	}

	// Rules persisted to the txt file.
	disk, err := os.ReadFile(ruleFile)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(disk, []byte("full:new.example")) {
		t.Fatalf("rule file not updated: %q", disk)
	}

	// Subscribers are notified asynchronously.
	deadline := time.After(time.Second)
	for notified.Load() == 0 {
		select {
		case <-deadline:
			t.Fatal("subscriber was not notified")
		default:
			time.Sleep(10 * time.Millisecond)
		}
	}
}

func TestGoldenDomainSetRuleExporter(t *testing.T) {
	d := newTestDomainSet(nil)
	d.rules = []string{"full:a.example", "domain:b.example"}
	got, err := d.GetRules()
	if err != nil {
		t.Fatal(err)
	}
	got[0] = "mutated"
	if d.rules[0] != "full:a.example" {
		t.Fatal("GetRules returned a mutable alias")
	}
	if fmt.Sprint(got[1]) != "domain:b.example" {
		t.Fatalf("unexpected rule: %v", got[1])
	}

	// Subscribing twice notifies twice.
	var n atomic.Int32
	d.Subscribe(func() { n.Add(1) })
	d.Subscribe(func() { n.Add(1) })
	d.notifySubscribers()
	deadline := time.After(time.Second)
	for n.Load() != 2 {
		select {
		case <-deadline:
			t.Fatalf("notified %d times, want 2", n.Load())
		default:
			time.Sleep(10 * time.Millisecond)
		}
	}
}

func TestGoldenRealRuleSetLoad(t *testing.T) {
	d := newTestDomainSet(nil)
	rules, err := d.loadFileInternal("testdata/real-rule-set.txt")
	if err != nil {
		t.Fatal(err)
	}
	// 6 full: + 70 domain: + 5 regexp: + 7 keyword: + 4 bare + 1 inline-comment
	// row = 93 loaded; the broken regexp row is skipped.
	if len(rules) != 93 {
		t.Fatalf("loaded %d rules, want 93 (broken regexp must be skipped)", len(rules))
	}

	tests := []struct {
		q    string
		want bool
	}{
		// full:
		{"doubleclick.net", true},
		{"pagead2.googlesyndication.com", true},
		// domain:
		{"sub.google-analytics.com", true},
		{"a.b.c.googletagmanager.com", true},
		{"amazon-adsystem.com", true},
		{"cdn.jsdelivr.net", true},
		// regexp:
		{"ads.foo.example", true},
		{"x.ads.example", true},
		{"adzz.tracker.example", true},
		// keyword:
		{"my-analytics-host.io", true},
		{"telemetry-collector.dev", true},
		// bare domain (default type = domain suffix):
		{"sub.adserver.example", true},
		// suffix semantics: "example.com" matches "x.example.com" but NOT a
		// deeper non-dot-aligned host like "...evil"
		{"sub.example.com", true},
		{"pixel.example.com.evil", false},
		// inline-comment row must not match the bare host
		{"with-inline-comment", false},
		// no-match
		{"unrelated-site.org", false},
		{"doubleclick.net.evil", false}, // full is exact; domain:.net not present
	}
	for _, tt := range tests {
		if _, ok := d.Match(tt.q); ok != tt.want {
			t.Errorf("Match(%q) = %v, want %v", tt.q, ok, tt.want)
		}
	}
}

var _ = json.Valid // keep encoding/json import if post payload shape changes
