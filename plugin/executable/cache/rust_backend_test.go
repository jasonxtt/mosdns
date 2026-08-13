package cache

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
	"github.com/prometheus/client_golang/prometheus"
	"go.uber.org/zap"
)

func TestRustBackendSelectionIsExplicitAndFallsBack(t *testing.T) {
	originalFactory := rustBackendFactory
	t.Cleanup(func() { rustBackendFactory = originalFactory })

	called := false
	rustBackendFactory = func(*Args) (rustCacheBackend, error) {
		called = true
		return nil, errors.New("test ABI mismatch")
	}

	t.Setenv(rustCacheBackendEnv, "")
	if backend, requested := openRustCacheBackend(&Args{Size: 16}, zap.NewNop()); backend != nil || requested || called {
		t.Fatalf("default selection changed: backend=%v requested=%v called=%v", backend, requested, called)
	}

	t.Setenv(rustCacheBackendEnv, "rust")
	if backend, requested := openRustCacheBackend(&Args{Size: 16}, zap.NewNop()); backend != nil || !requested || !called {
		t.Fatalf("fallback mismatch: backend=%v requested=%v called=%v", backend, requested, called)
	}
}

type fakeRustCacheBackend struct {
	closed    bool
	lookup    rustCacheLookupResult
	lookupErr error
	stores    int
	flushes   int
	len       int
}

func (*fakeRustCacheBackend) Name() string { return "fake-rust" }
func (b *fakeRustCacheBackend) Lookup([]byte, time.Time) (rustCacheLookupResult, error) {
	return b.lookup, b.lookupErr
}
func (b *fakeRustCacheBackend) Store([]byte, *item, time.Time) error {
	b.stores++
	b.len++
	return nil
}
func (b *fakeRustCacheBackend) Len() (int, error) { return b.len, nil }
func (b *fakeRustCacheBackend) Flush() error {
	b.flushes++
	b.len = 0
	return nil
}
func (b *fakeRustCacheBackend) Close() error {
	b.closed = true
	return nil
}

func TestRustBackendSelectionActivatesAvailableRuntime(t *testing.T) {
	originalFactory := rustBackendFactory
	t.Cleanup(func() { rustBackendFactory = originalFactory })
	t.Setenv(rustCacheBackendEnv, "rust")
	fake := new(fakeRustCacheBackend)
	rustBackendFactory = func(*Args) (rustCacheBackend, error) { return fake, nil }

	backend, requested := openRustCacheBackend(&Args{Size: 16}, zap.NewNop())
	if !requested || backend != fake {
		t.Fatalf("runtime was not activated: backend=%v requested=%v", backend, requested)
	}
}

func TestRustBackendFreshHitPreservesGoFacadeContract(t *testing.T) {
	originalFactory := rustBackendFactory
	t.Cleanup(func() { rustBackendFactory = originalFactory })
	t.Setenv(rustCacheBackendEnv, "rust")

	query := new(dns.Msg)
	query.SetQuestion("example.org.", dns.TypeA)
	response := new(dns.Msg)
	response.SetReply(query)
	response.Answer = []dns.RR{mustRR(t, "example.org. 60 IN A 192.0.2.1")}
	packed, err := response.Pack()
	if err != nil {
		t.Fatal(err)
	}
	fake := &fakeRustCacheBackend{lookup: rustCacheLookupResult{
		State:     rustCacheFresh,
		Response:  packed,
		DomainSet: "rust-set",
		StoredAt:  time.Now(),
	}}
	rustBackendFactory = func(*Args) (rustCacheBackend, error) { return fake, nil }
	c := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = c.Close() })

	query.Id = 0x4321
	qCtx := query_context.NewContext(query)
	if err := c.Exec(context.Background(), qCtx, sequence.ChainWalker{}); err != nil {
		t.Fatal(err)
	}
	if len(qCtx.RawResponse()) == 0 {
		t.Fatal("rust hit decoded the response before a message consumer requested it")
	}
	if qCtx.R() == nil || qCtx.R().Id != query.Id || len(qCtx.R().Answer) != 1 {
		t.Fatalf("rust hit did not produce facade response: %v", qCtx.R())
	}
	if got, ok := qCtx.GetValue(query_context.KeyDomainSet); !ok || got != "rust-set" {
		t.Fatalf("domain_set mismatch: got=%v ok=%v", got, ok)
	}
}

func TestRustBackendMissStoresMirrorAndFailureTripsCircuitBreaker(t *testing.T) {
	originalFactory := rustBackendFactory
	t.Cleanup(func() { rustBackendFactory = originalFactory })
	t.Setenv(rustCacheBackendEnv, "rust")
	fake := &fakeRustCacheBackend{lookup: rustCacheLookupResult{State: rustCacheMiss}}
	rustBackendFactory = func(*Args) (rustCacheBackend, error) { return fake, nil }
	c := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = c.Close() })

	query := new(dns.Msg)
	query.SetQuestion("example.org.", dns.TypeA)
	response := new(dns.Msg)
	response.SetReply(query)
	response.Answer = []dns.RR{mustRR(t, "example.org. 60 IN A 192.0.2.1")}
	qCtx := query_context.NewContext(query)
	qCtx.SetResponse(response)
	if err := c.Exec(context.Background(), qCtx, sequence.ChainWalker{}); err != nil {
		t.Fatal(err)
	}
	if fake.stores != 1 || c.backend.Len() != 1 {
		t.Fatalf("mirror store mismatch: rust=%d go=%d", fake.stores, c.backend.Len())
	}

	fake.lookupErr = errors.New("simulated rust failure")
	miss := new(dns.Msg)
	miss.SetQuestion("other.example.", dns.TypeA)
	missCtx := query_context.NewContext(miss)
	if err := c.Exec(context.Background(), missCtx, sequence.ChainWalker{}); err != nil {
		t.Fatal(err)
	}
	if c.rustActive() || !fake.closed {
		t.Fatalf("circuit breaker did not disable and close rust: active=%v closed=%v", c.rustActive(), fake.closed)
	}
}

func TestRustBackendDrivesExistingSizeMetricAndFlushFacade(t *testing.T) {
	originalFactory := rustBackendFactory
	t.Cleanup(func() { rustBackendFactory = originalFactory })
	t.Setenv(rustCacheBackendEnv, "rust")
	fake := &fakeRustCacheBackend{len: 7}
	rustBackendFactory = func(*Args) (rustCacheBackend, error) { return fake, nil }
	c := NewCache(&Args{Size: 16}, Opts{MetricsTag: "rust-contract"})
	t.Cleanup(func() { _ = c.Close() })

	registry := prometheus.NewRegistry()
	if err := c.RegMetricsTo(prometheus.WrapRegistererWithPrefix(PluginType+"_", registry)); err != nil {
		t.Fatal(err)
	}
	families, err := registry.Gather()
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, family := range families {
		if family.GetName() == "cache_size_current" {
			found = true
			if got := family.Metric[0].GetGauge().GetValue(); got != 7 {
				t.Fatalf("Rust size metric=%v, want 7", got)
			}
		}
	}
	if !found {
		t.Fatal("cache_size_current metric was not gathered")
	}
	if err := c.Flush(); err != nil {
		t.Fatal(err)
	}
	if fake.flushes != 1 || fake.len != 0 {
		t.Fatalf("flush facade mismatch: flushes=%d len=%d", fake.flushes, fake.len)
	}
}
