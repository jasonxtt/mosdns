package domain_set_light

import (
	"bytes"
	"encoding/json"
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

func TestSlice0DomainSetLightComposesSourcesAndStaysConstantFalse(t *testing.T) {
	dir := t.TempDir()
	textPath := filepath.Join(dir, "rules.txt")
	srsPath := filepath.Join(dir, "rules.srs")
	missingPath := filepath.Join(dir, "missing.txt")
	if err := os.WriteFile(textPath, []byte("# comment\nfull:text.example\nkeyword:tracker\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(srsPath, buildSlice0DomainSRS(t,
		[]string{"srs.example"}, []string{"suffix.example"}, nil, []string{`^regex\.example$`}), 0o644); err != nil {
		t.Fatal(err)
	}

	d := &DomainSetLight{subscribers: make([]func(), 0)}
	rules, err := d.initAndLoadRules(
		[]string{"full:expression.example"},
		[]string{textPath, srsPath, missingPath},
	)
	if err != nil {
		t.Fatal(err)
	}
	sort.Strings(rules)
	want := []string{
		"domain:suffix.example",
		"full:expression.example",
		"full:srs.example",
		"full:text.example",
		"keyword:tracker",
		`regexp:^regex\.example$`,
	}
	if !equalSlice0DomainStrings(rules, want) {
		t.Fatalf("composed rules = %v, want %v", rules, want)
	}
	d.rules = append([]string(nil), rules...)
	got, err := d.GetRules()
	if err != nil {
		t.Fatal(err)
	}
	got[0] = "mutated"
	if d.rules[0] == "mutated" {
		t.Fatal("GetRules returned a mutable alias")
	}
	for _, name := range []string{"expression.example", "srs.example", "child.suffix.example"} {
		if _, ok := d.Match(name); ok {
			t.Fatalf("domain_set_light unexpectedly matched %q", name)
		}
	}

	d.ruleFile = filepath.Join(dir, "posted.txt")
	if err := os.WriteFile(d.ruleFile, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	notified := make(chan struct{}, 1)
	d.Subscribe(func() { notified <- struct{}{} })
	recorder := httptest.NewRecorder()
	body, err := json.Marshal(domainPayload{Values: []string{"full:posted.example", "domain:posted-suffix.example"}})
	if err != nil {
		t.Fatal(err)
	}
	d.api().ServeHTTP(recorder, httptest.NewRequest(http.MethodPost, "/post", bytes.NewReader(body)))
	if recorder.Code != http.StatusOK {
		t.Fatalf("POST status = %d, body=%s", recorder.Code, recorder.Body.String())
	}
	select {
	case <-notified:
	case <-time.After(time.Second):
		t.Fatal("POST did not notify subscribers")
	}
	disk, err := os.ReadFile(d.ruleFile)
	if err != nil {
		t.Fatal(err)
	}
	if string(disk) != "full:posted.example\ndomain:posted-suffix.example\n" {
		t.Fatalf("posted rule file = %q", disk)
	}
	if _, ok := d.Match("posted.example"); ok {
		t.Fatal("domain_set_light must remain a constant-false matcher after POST")
	}
}

func equalSlice0DomainStrings(a, b []string) bool {
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
