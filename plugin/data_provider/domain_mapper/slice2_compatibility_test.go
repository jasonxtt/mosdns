package domain_mapper

import (
	"errors"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
)

func TestSlice2ValuedMapperUnsafeRegexpPublishesGoOnlyGeneration(t *testing.T) {
	oldBuilder := valuedSnapshotBuilder
	t.Cleanup(func() { valuedSnapshotBuilder = oldBuilder })

	candidate := &slice4ValuedSnapshot{}
	var gotRules []matcher_adapter.ValuedRule
	valuedSnapshotBuilder = func(rules []matcher_adapter.ValuedRule) (matcher_adapter.ValuedSnapshot, error) {
		gotRules = append([]matcher_adapter.ValuedRule(nil), rules...)
		for _, rule := range rules {
			if rule.Rule == `regexp:^\w+$` {
				return candidate, errors.New("unsupported Rust regexp")
			}
		}
		return nil, nil
	}

	dm := newSlice4Mapper(t, map[string]*slice0DetailedExporter{
		"source": {entries: []data_provider.RuleEntry{
			{Rule: "full:accepted.example", SourceName: "source"},
			{Rule: `regexp:^\w+$`, SourceName: "source"},
		}},
	}, []RuleConfig{{Tag: "source", Mark: 7}})
	t.Cleanup(func() { _ = dm.Close() })

	var sawUnsafe bool
	for _, rule := range gotRules {
		if rule.Rule == `regexp:^\w+$` {
			sawUnsafe = true
			break
		}
	}
	if !sawUnsafe {
		t.Fatalf("valued Rust candidate did not receive the unsafe regexp: %+v", gotRules)
	}
	dm.generationMu.RLock()
	rustGeneration := dm.rustGeneration
	dm.generationMu.RUnlock()
	if rustGeneration != nil {
		t.Fatal("unsafe regexp generation must publish Go-only")
	}
	if marks, _, ok := dm.FastMatch("abc123."); !ok || !sameSlice0Uint8Set(marks, []uint8{7}) {
		t.Fatalf("Go generation did not preserve the unsafe regexp: marks=%v ok=%v", marks, ok)
	}
	if _, _, ok := dm.FastMatch("accepted.example."); !ok {
		t.Fatal("Go generation did not preserve the accepted source batch")
	}
	if got := candidate.closeN.Load(); got != 1 {
		t.Fatalf("rejected valued Rust candidate close count = %d, want 1", got)
	}
}
