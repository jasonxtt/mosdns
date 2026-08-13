package sd_set_light

import (
	"bytes"
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sort"
	"testing"
	"time"

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

func newSlice0SdSetLight(t *testing.T, sources map[string]*RuleSource) *SdSetLight {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	p := &SdSetLight{
		sources:         sources,
		localConfigFile: filepath.Join(t.TempDir(), "sources.json"),
		httpClient:      &http.Client{},
		ctx:             ctx,
		cancel:          cancel,
		subscribers:     make([]func(), 0),
	}
	t.Cleanup(func() { _ = p.Close() })
	return p
}

func writeSlice0File(t *testing.T, path string, data []byte) {
	t.Helper()
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestSlice0SdSetLightExportsRulesButNeverMatches(t *testing.T) {
	dir := t.TempDir()
	data := buildSlice0DomainSRS(t,
		[]string{"alpha.example"},
		[]string{"suffix.example"},
		[]string{"tracker"},
		[]string{`^regex\.example$`},
	)
	path := filepath.Join(dir, "rules.srs")
	writeSlice0File(t, path, data)
	p := newSlice0SdSetLight(t, map[string]*RuleSource{
		"source": {
			Name: "source", Type: "subscription", Files: path, Enabled: true,
			EnableRegexp: false, RuleCount: 3,
		},
	})
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}

	rules, err := p.GetRules()
	if err != nil {
		t.Fatal(err)
	}
	sort.Strings(rules)
	want := []string{"domain:suffix.example", "full:alpha.example", "keyword:tracker"}
	if !equalSlice0Strings(rules, want) {
		t.Fatalf("GetRules() = %v, want %v", rules, want)
	}
	entries, err := p.GetRuleEntries()
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != len(want) {
		t.Fatalf("GetRuleEntries() returned %d entries, want %d", len(entries), len(want))
	}
	for _, entry := range entries {
		if entry.SourceName != "source" || entry.SourceType != "subscription" || entry.SourceFile != path {
			t.Fatalf("entry lost source metadata: %+v", entry)
		}
	}
	if _, ok := p.Match("alpha.example"); ok {
		t.Fatal("sd_set_light must remain a constant-false matcher")
	}

	notified := make(chan struct{}, 1)
	p.Subscribe(func() { notified <- struct{}{} })
	updated := buildSlice0DomainSRS(t, []string{"updated.example"}, nil, nil, nil)
	writeSlice0File(t, path, updated)
	p.sources["source"].RuleCount = 1
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	select {
	case <-notified:
	case <-time.After(time.Second):
		t.Fatal("reload did not notify subscribers")
	}
	rules, err = p.GetRules()
	if err != nil {
		t.Fatal(err)
	}
	if len(rules) != 1 || rules[0] != "full:updated.example" {
		t.Fatalf("GetRules after reload = %v", rules)
	}
	if _, ok := p.Match("updated.example"); ok {
		t.Fatal("sd_set_light must not start matching after reload")
	}

	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
}

func equalSlice0Strings(a, b []string) bool {
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

func TestSlice0SdSetLightOnlineInvalidSourceDoesNotOverwriteFile(t *testing.T) {
	dir := t.TempDir()
	oldData := buildSlice0DomainSRS(t, []string{"old.example"}, nil, nil, nil)
	newData := buildSlice0DomainSRS(t, []string{"new.example"}, nil, nil, nil)
	body := oldData
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(body)
	}))
	defer server.Close()

	path := filepath.Join(dir, "online.srs")
	writeSlice0File(t, path, oldData)
	p := newSlice0SdSetLight(t, map[string]*RuleSource{
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
	rules, err := p.GetRules()
	if err != nil {
		t.Fatal(err)
	}
	if len(rules) != 1 || rules[0] != "full:new.example" {
		t.Fatalf("valid online reload = %v", rules)
	}

	body = []byte("invalid")
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
}
