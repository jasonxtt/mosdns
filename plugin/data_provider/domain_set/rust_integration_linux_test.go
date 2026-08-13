//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package domain_set

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/coremain"
)

func TestRustDomainSetEmptyPostCreatesAndClosesMatcher(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	m := coremain.NewTestMosdnsWithPlugins(nil)
	bp := coremain.NewBP("domain-set-test", m)
	ruleFile := filepath.Join(t.TempDir(), "rules.txt")
	if err := os.WriteFile(ruleFile, []byte("full:initial.example\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	value, err := Init(bp, &Args{Files: []string{ruleFile}})
	if err != nil {
		t.Fatal(err)
	}
	d := value.(*DomainSet)
	if d.rustMatcher == nil {
		t.Fatal("domain_set must initialize the Rust matcher")
	}
	if matched, err := d.rustMatcher.Match("initial.example."); err != nil || !matched {
		t.Fatalf("direct Rust domain hit = (%v, %v), want (true, nil)", matched, err)
	}
	if matched, err := d.rustMatcher.Match("other.example."); err != nil || matched {
		t.Fatalf("direct Rust domain miss = (%v, %v), want (false, nil)", matched, err)
	}

	body, err := json.Marshal(domainPayload{Values: nil})
	if err != nil {
		t.Fatal(err)
	}
	r := httptest.NewRecorder()
	d.api().ServeHTTP(r, httptest.NewRequest(http.MethodPost, "/post", bytes.NewReader(body)))
	if r.Code != http.StatusOK {
		t.Fatalf("empty POST status = %d, body=%s", r.Code, r.Body.String())
	}
	if d.rustMatcher == nil {
		t.Fatal("empty POST must publish an empty Rust matcher handle")
	}
	if matched, err := d.rustMatcher.Match("initial.example."); err != nil || matched {
		t.Fatalf("direct empty Rust domain match = (%v, %v), want (false, nil)", matched, err)
	}
	if _, ok := d.Match("initial.example"); ok {
		t.Fatal("empty POST must remove the old rule")
	}
	if err := d.Close(); err != nil {
		t.Fatal(err)
	}
	if err := d.Close(); err != nil {
		t.Fatal(err)
	}
}
