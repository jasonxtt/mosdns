//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package si_set

import (
	"net/netip"
	"testing"
)

func TestSlice2SiSetPublishesRealRustGeneration(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	p := newSlice0SiSet(t, map[string]*RuleSource{})
	path := t.TempDir() + "/rules.srs"
	writeSlice2SiFile(t, path, buildSlice0IPSRS(t, [][2]netip.Addr{
		{netip.MustParseAddr("192.0.2.0"), netip.MustParseAddr("192.0.2.255")},
	}))
	p.sources["source"] = &RuleSource{Name: "source", Files: path, Enabled: true}
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}

	p.generationMu.RLock()
	rustMatcher := p.generation.rustMatcher
	p.generationMu.RUnlock()
	if rustMatcher == nil {
		t.Fatal("si_set did not publish a real Rust generation")
	}
	matched, err := rustMatcher.Match("192.0.2.1")
	if err != nil || !matched {
		t.Fatalf("Rust si_set match = (%v, %v), want (true, nil)", matched, err)
	}
}
