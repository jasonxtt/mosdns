//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package ip_set

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/coremain"
)

func TestRustIPSetEmptyFlushCreatesAndClosesMatcher(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	m := coremain.NewTestMosdnsWithPlugins(nil)
	bp := coremain.NewBP("ip-set-test", m)
	p, err := NewIPSet(bp, &Args{IPs: []string{"10.0.0.0/8"}})
	if err != nil {
		t.Fatal(err)
	}
	if p.rustMatcher == nil {
		t.Fatal("ip_set must initialize the Rust matcher")
	}
	if matched, err := p.rustMatcher.Match("10.1.2.3"); err != nil || !matched {
		t.Fatalf("direct Rust IP hit = (%v, %v), want (true, nil)", matched, err)
	}
	if matched, err := p.rustMatcher.Match("11.1.2.3"); err != nil || matched {
		t.Fatalf("direct Rust IP miss = (%v, %v), want (false, nil)", matched, err)
	}

	r := httptest.NewRecorder()
	p.api().ServeHTTP(r, httptest.NewRequest(http.MethodGet, "/flush", nil))
	if r.Code != http.StatusOK {
		t.Fatalf("empty flush status = %d, body=%s", r.Code, r.Body.String())
	}
	if p.rustMatcher == nil {
		t.Fatal("empty flush must publish an empty Rust matcher handle")
	}
	if matched, err := p.rustMatcher.Match("10.1.2.3"); err != nil || matched {
		t.Fatalf("direct empty Rust IP match = (%v, %v), want (false, nil)", matched, err)
	}
	if p.Match(mustAddr(t, "10.1.2.3")) {
		t.Fatal("empty flush must remove the old rule")
	}
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
}
