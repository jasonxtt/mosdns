package domain_mapper

import (
	"sync/atomic"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
)

type slice0CompatValuedSnapshot struct {
	calls  atomic.Int32
	closed atomic.Int32
}

func (s *slice0CompatValuedSnapshot) Match(name string) (matcher_adapter.ValuedResult, error) {
	s.calls.Add(1)
	return matcher_adapter.ValuedResult{Matched: name == "ascii.example."}, nil
}

func (s *slice0CompatValuedSnapshot) Len() (uint64, error) { return 1, nil }

func (s *slice0CompatValuedSnapshot) Close() error {
	s.closed.Add(1)
	return nil
}

func TestSlice0MapperNonASCIIQueryUsesGoWithoutDisablingRust(t *testing.T) {
	goMatcher := domain.NewMixMatcher[*MatchResult]()
	goResult := &MatchResult{FastMarks: []uint8{7}}
	if err := goMatcher.Add("full:例.example", goResult); err != nil {
		t.Fatal(err)
	}
	snapshot := new(slice0CompatValuedSnapshot)
	dm := &DomainMapper{rustGeneration: &valuedGeneration{snapshot: snapshot}}
	dm.matcher.Store(&compiledMatcher{domainRules: goMatcher})

	if result, ok := dm.lookupMatchResult("例.example."); !ok || len(result.FastMarks) != 1 || result.FastMarks[0] != 7 {
		t.Fatalf("non-ASCII query did not use the paired Go matcher: result=%+v ok=%v", result, ok)
	}
	if got := snapshot.calls.Load(); got != 0 {
		t.Fatalf("Rust calls for non-ASCII query = %d, want 0", got)
	}
	if result, ok := dm.lookupMatchResult("ascii.example."); !ok || result == nil {
		t.Fatalf("ASCII query did not use Rust after the Go fallback: result=%+v ok=%v", result, ok)
	}
	if got := snapshot.calls.Load(); got != 1 {
		t.Fatalf("Rust calls after ASCII query = %d, want 1", got)
	}
	if got := snapshot.closed.Load(); got != 0 {
		t.Fatalf("non-ASCII query disabled Rust handle, close count = %d", got)
	}
}
