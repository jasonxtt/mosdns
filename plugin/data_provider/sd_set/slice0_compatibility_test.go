package sd_set

import (
	"sync/atomic"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
)

type slice0CompatDomainSnapshot struct {
	calls  atomic.Int32
	closed atomic.Int32
}

func (s *slice0CompatDomainSnapshot) Match(string) (bool, error) {
	s.calls.Add(1)
	return true, nil
}

func (s *slice0CompatDomainSnapshot) Len() (uint64, error) { return 1, nil }

func (s *slice0CompatDomainSnapshot) Close() error {
	s.closed.Add(1)
	return nil
}

func TestSlice0SdSetNonASCIIQueryUsesGoWithoutDisablingRust(t *testing.T) {
	goMatcher := domain.NewDomainMixMatcher()
	if err := goMatcher.Add("full:例.example", struct{}{}); err != nil {
		t.Fatal(err)
	}
	rust := new(slice0CompatDomainSnapshot)
	p := &SdSet{generation: &sdGeneration{goMatcher: goMatcher, rustMatcher: rust}}

	if _, ok := p.Match("例.example."); !ok {
		t.Fatal("non-ASCII query did not use the paired Go matcher")
	}
	if got := rust.calls.Load(); got != 0 {
		t.Fatalf("Rust calls for non-ASCII query = %d, want 0", got)
	}
	if _, ok := p.Match("ascii.example."); !ok {
		t.Fatal("ASCII query did not use Rust after the Go fallback")
	}
	if got := rust.calls.Load(); got != 1 {
		t.Fatalf("Rust calls after ASCII query = %d, want 1", got)
	}
	if got := rust.closed.Load(); got != 0 {
		t.Fatalf("non-ASCII query disabled Rust handle, close count = %d", got)
	}
}
