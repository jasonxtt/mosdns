//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package sd_set

import "testing"

func TestSlice2SdSetPublishesRealRustGeneration(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	p := newSlice0SdSet(t, map[string]*RuleSource{})
	path := t.TempDir() + "/rules.srs"
	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t, []string{"real.example"}, nil, nil, nil))
	p.sources["source"] = &RuleSource{Name: "source", Files: path, Enabled: true}
	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}

	p.generationMu.RLock()
	rustMatcher := p.generation.rustMatcher
	p.generationMu.RUnlock()
	if rustMatcher == nil {
		t.Fatal("sd_set did not publish a real Rust generation")
	}
	matched, err := rustMatcher.Match("real.example")
	if err != nil || !matched {
		t.Fatalf("Rust sd_set match = (%v, %v), want (true, nil)", matched, err)
	}
}
