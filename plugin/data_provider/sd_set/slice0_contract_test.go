package sd_set

import (
	"bytes"
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sort"
	"sync/atomic"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/testutil"
)

func buildSlice0DomainSRS(t *testing.T, domains, suffixes, keywords, regexes []string) []byte {
	t.Helper()
	data, err := testutil.BuildDomainSRS(domains, suffixes, keywords, regexes)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func newSlice0SdSet(t *testing.T, sources map[string]*RuleSource) *SdSet {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	p := &SdSet{
		sources:         sources,
		localConfigFile: filepath.Join(t.TempDir(), "sources.json"),
		httpClient:      &http.Client{},
		ctx:             ctx,
		cancel:          cancel,
		subscribers:     make([]func(), 0),
	}
	p.matcher.Store(domain.NewDomainMixMatcher())
	t.Cleanup(func() { _ = p.Close() })
	return p
}

func writeSlice0File(t *testing.T, path string, data []byte) {
	t.Helper()
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
}

func sortedRules(rules []string) []string {
	out := append([]string(nil), rules...)
	sort.Strings(out)
	return out
}

func TestSlice0SdSetRulesSourcesReloadAndClose(t *testing.T) {
	dir := t.TempDir()
	first := buildSlice0DomainSRS(t,
		[]string{"alpha.example"},
		[]string{"suffix.example"},
		[]string{"tracker"},
		[]string{`^regex\.example$`},
	)
	second := buildSlice0DomainSRS(t, []string{"other.example"}, nil, nil, nil)
	firstPath := filepath.Join(dir, "first.srs")
	secondPath := filepath.Join(dir, "second.srs")
	badPath := filepath.Join(dir, "bad.srs")
	writeSlice0File(t, firstPath, first)
	writeSlice0File(t, secondPath, second)
	writeSlice0File(t, badPath, []byte("not an srs"))

	p := newSlice0SdSet(t, map[string]*RuleSource{
		"first": {
			Name: "first", Type: "local", Files: firstPath, Enabled: true,
			EnableRegexp: true, RuleCount: 4,
		},
		"second": {
			Name: "second", Type: "local", Files: secondPath, Enabled: true,
			RuleCount: 1,
		},
		"bad": {
			Name: "bad", Type: "local", Files: badPath, Enabled: true,
		},
		"disabled": {
			Name: "disabled", Type: "local", Files: secondPath, Enabled: false,
		},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}

	wantRules := []string{
		"domain:suffix.example",
		"full:alpha.example",
		"full:other.example",
		"keyword:tracker",
		`regexp:^regex\.example$`,
	}
	if got := sortedRules(mustSdRules(t, p)); !equalStrings(got, sortedRules(wantRules)) {
		t.Fatalf("GetRules() = %v, want %v", got, sortedRules(wantRules))
	}
	entries, err := p.GetRuleEntries()
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != len(wantRules) {
		t.Fatalf("GetRuleEntries() returned %d entries, want %d", len(entries), len(wantRules))
	}
	for _, entry := range entries {
		if entry.SourceName != "first" && entry.SourceName != "second" {
			t.Fatalf("entry has unexpected source metadata: %+v", entry)
		}
		if entry.SourceFile == "" || entry.SourceType != "local" {
			t.Fatalf("entry lost source metadata: %+v", entry)
		}
	}

	for _, tt := range []struct {
		name string
		want bool
	}{
		{"alpha.example", true},
		{"child.suffix.example", true},
		{"host-tracker.example", true},
		{"regex.example", true},
		{"other.example", true},
		{"disabled.example", false},
		{"unrelated.example", false},
	} {
		if _, ok := p.Match(tt.name); ok != tt.want {
			t.Errorf("Match(%q) = %v, want %v", tt.name, ok, tt.want)
		}
	}

	updated := buildSlice0DomainSRS(t, []string{"updated.example"}, nil, nil, nil)
	notified := make(chan struct{}, 1)
	p.Subscribe(func() { notified <- struct{}{} })
	writeSlice0File(t, firstPath, updated)
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	select {
	case <-notified:
	case <-time.After(time.Second):
		t.Fatal("reload did not notify subscribers")
	}
	if _, ok := p.Match("updated.example"); !ok {
		t.Fatal("reload did not publish the new source generation")
	}
	if _, ok := p.Match("alpha.example"); ok {
		t.Fatal("reload retained a removed rule")
	}

	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
}

func mustSdRules(t *testing.T, p *SdSet) []string {
	t.Helper()
	rules, err := p.GetRules()
	if err != nil {
		t.Fatal(err)
	}
	return rules
}

func equalStrings(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func TestSlice0SdSetOnlineReloadRetainsFileOnInvalidSource(t *testing.T) {
	dir := t.TempDir()
	oldData := buildSlice0DomainSRS(t, []string{"old.example"}, nil, nil, nil)
	newData := buildSlice0DomainSRS(t, []string{"new.example"}, nil, nil, nil)
	var body atomic.Value
	body.Store(oldData)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(body.Load().([]byte))
	}))
	defer server.Close()

	path := filepath.Join(dir, "online.srs")
	writeSlice0File(t, path, oldData)
	p := newSlice0SdSet(t, map[string]*RuleSource{
		"online": {
			Name: "online", Type: "subscription", Files: path, URL: server.URL,
			Enabled: true, RuleCount: 1,
		},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}

	body.Store(newData)
	if err := p.downloadAndUpdateLocalFile(context.Background(), "online"); err != nil {
		t.Fatal(err)
	}
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if _, ok := p.Match("new.example"); !ok {
		t.Fatal("valid online reload was not published")
	}
	if _, ok := p.Match("old.example"); ok {
		t.Fatal("valid online reload retained the old rule")
	}

	body.Store([]byte("invalid"))
	if err := p.downloadAndUpdateLocalFile(context.Background(), "online"); err == nil {
		t.Fatal("invalid online source must be rejected")
	}
	disk, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(disk, newData) {
		t.Fatal("invalid online source replaced the established file")
	}
	if _, ok := p.Match("new.example"); !ok {
		t.Fatal("invalid online reload changed the established matcher")
	}

	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
}
