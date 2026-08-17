package sd_set

import (
	"errors"
	"strings"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
)

func TestSlice2SdSetUnsafeRegexpPublishesGoOnlyGeneration(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	oldBuilder := rustDomainSnapshotBuilder
	t.Cleanup(func() { rustDomainSnapshotBuilder = oldBuilder })

	candidate := &slice2DomainSnapshot{}
	var gotRules []string
	rustDomainSnapshotBuilder = func(rules []string) (matcher_adapter.DomainSnapshot, error) {
		gotRules = append([]string(nil), rules...)
		for _, rule := range rules {
			if rule == `regexp:^\w+$` {
				return candidate, errors.New("unsupported Rust regexp")
			}
		}
		return nil, nil
	}

	path := t.TempDir() + "/rules.srs"
	writeSlice2SdFile(t, path, buildSlice0DomainSRS(t,
		[]string{"accepted.example"}, nil, nil, []string{`^\w+$`},
	))
	p := newSlice0SdSet(t, map[string]*RuleSource{
		"source": {Name: "source", Files: path, Enabled: true, EnableRegexp: true},
	})

	if err := p.reloadAllRules(); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(strings.Join(gotRules, "\n"), `regexp:^\w+$`) {
		t.Fatalf("Rust candidate did not receive the unsafe regexp: %v", gotRules)
	}
	p.generationMu.RLock()
	rustMatcher := p.generation.rustMatcher
	p.generationMu.RUnlock()
	if rustMatcher != nil {
		t.Fatal("unsafe regexp generation must publish Go-only")
	}
	if _, ok := p.Match("abc123"); !ok {
		t.Fatal("Go generation did not preserve the accepted unsafe regexp")
	}
	if _, ok := p.Match("accepted.example"); !ok {
		t.Fatal("Go generation did not preserve the accepted source batch")
	}
	if got := candidate.closed.Load(); got != 1 {
		t.Fatalf("rejected Rust candidate close count = %d, want 1", got)
	}
}
