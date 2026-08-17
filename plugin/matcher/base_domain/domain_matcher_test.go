package base_domain

import (
	"strings"
	"sync/atomic"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/domain_set"
)

type compatibilityRustDomainBackend struct {
	calls  atomic.Int32
	closed atomic.Int32
}

func (b *compatibilityRustDomainBackend) Match(value string) (bool, error) {
	b.calls.Add(1)
	return strings.TrimSuffix(value, ".") == "ascii.example", nil
}

func (b *compatibilityRustDomainBackend) Close() error {
	b.closed.Add(1)
	return nil
}

func TestRustDomainWrapperFallsThroughToGoForNonASCIIQuery(t *testing.T) {
	backend := new(compatibilityRustDomainBackend)
	wrapper := &rustDomainWrapper{backend: backend}
	goMatcher := domain.NewDomainMixMatcher()
	if err := goMatcher.Add("full:例.example", struct{}{}); err != nil {
		t.Fatal(err)
	}

	group := domain_set.MatcherGroup{wrapper, goMatcher}
	if _, ok := group.Match("例.example."); !ok {
		t.Fatal("non-ASCII query did not fall through to the anonymous Go matcher")
	}
	if got := backend.calls.Load(); got != 0 {
		t.Fatalf("Rust backend calls for non-ASCII query = %d, want 0", got)
	}
	if wrapper.disabled.Load() {
		t.Fatal("non-ASCII query disabled the healthy Rust wrapper")
	}

	if _, ok := group.Match("ascii.example."); !ok {
		t.Fatal("ASCII query did not use the Rust matcher")
	}
	if got := backend.calls.Load(); got != 1 {
		t.Fatalf("Rust backend calls after ASCII query = %d, want 1", got)
	}
}
