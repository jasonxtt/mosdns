//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package cache

import (
	"bytes"
	"context"
	"sync"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
)

func TestRustCGOBackendLifecycleAndSemanticLookup(t *testing.T) {
	backend, err := newRustCacheBackend(&Args{Size: 16, LazyCacheTTL: 300})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := backend.Close(); err != nil {
			t.Fatal(err)
		}
	})

	now := time.Now().Truncate(time.Second)
	query := new(dns.Msg)
	query.SetQuestion("exact.example.", dns.TypeA)
	response := new(dns.Msg)
	response.SetReply(query)
	response.Answer = []dns.RR{mustRR(t, "exact.example. 60 IN A 192.0.2.1")}
	responseWire, err := response.Pack()
	if err != nil {
		t.Fatal(err)
	}
	value := &item{
		resp:           responseWire,
		storedTime:     now,
		expirationTime: now.Add(time.Minute),
		domainSet:      "rust-contract",
	}
	key := []byte("exact-go-key")
	if err := backend.Store(key, value, now.Add(5*time.Minute)); err != nil {
		t.Fatal(err)
	}

	fresh, err := backend.Lookup(key, now.Add(10*time.Second))
	if err != nil {
		t.Fatal(err)
	}
	if fresh.State != rustCacheFresh || fresh.DomainSet != "rust-contract" || !fresh.StoredAt.Equal(now) {
		t.Fatalf("fresh mismatch: %+v", fresh)
	}
	freshMsg := new(dns.Msg)
	if err := freshMsg.Unpack(fresh.Response); err != nil {
		t.Fatal(err)
	}
	if got := freshMsg.Answer[0].Header().Ttl; got != 50 {
		t.Fatalf("fresh TTL=%d, want 50", got)
	}

	lazy, err := backend.Lookup(key, now.Add(2*time.Minute))
	if err != nil {
		t.Fatal(err)
	}
	if lazy.State != rustCacheLazy {
		t.Fatalf("lazy state=%d", lazy.State)
	}
	lazyMsg := new(dns.Msg)
	if err := lazyMsg.Unpack(lazy.Response); err != nil {
		t.Fatal(err)
	}
	if got := lazyMsg.Answer[0].Header().Ttl; got != expiredMsgTtl {
		t.Fatalf("lazy TTL=%d, want %d", got, expiredMsgTtl)
	}

	if length, err := backend.Len(); err != nil || length != 1 {
		t.Fatalf("length=%d err=%v", length, err)
	}
	if err := backend.Flush(); err != nil {
		t.Fatal(err)
	}
	if length, err := backend.Len(); err != nil || length != 0 {
		t.Fatalf("length after flush=%d err=%v", length, err)
	}
}

func TestRustCGOBackendConcurrentLookupAndCloseIsSafe(t *testing.T) {
	backend, err := newRustCacheBackend(&Args{Size: 16})
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now().Truncate(time.Second)
	q := new(dns.Msg)
	q.SetQuestion("close.example.", dns.TypeA)
	r := new(dns.Msg)
	r.SetReply(q)
	r.Answer = []dns.RR{mustRR(t, "close.example. 60 IN A 192.0.2.1")}
	wire, err := r.Pack()
	if err != nil {
		t.Fatal(err)
	}
	if err := backend.Store([]byte("close-key"), &item{
		resp:           wire,
		storedTime:     now,
		expirationTime: now.Add(time.Minute),
	}, now.Add(time.Minute)); err != nil {
		t.Fatal(err)
	}

	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 100; j++ {
				_, _ = backend.Lookup([]byte("close-key"), now)
			}
		}()
	}
	if err := backend.Close(); err != nil {
		t.Fatal(err)
	}
	wg.Wait()
	if err := backend.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestRustCGOFacadeConcurrentLookupAndCloseIsSafe(t *testing.T) {
	t.Setenv(rustCacheBackendEnv, "rust")
	c := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = c.Close() })
	if !c.rustActive() {
		t.Fatal("Rust backend did not activate")
	}

	now := time.Now().Truncate(time.Second)
	q := new(dns.Msg)
	q.SetQuestion("facade-close.example.", dns.TypeA)
	r := new(dns.Msg)
	r.SetReply(q)
	r.Answer = []dns.RR{mustRR(t, "facade-close.example. 60 IN A 192.0.2.1")}
	wire, err := r.Pack()
	if err != nil {
		t.Fatal(err)
	}
	c.storeRust([]byte("facade-close-key"), &item{
		resp:           wire,
		storedTime:     now,
		expirationTime: now.Add(time.Minute),
	}, now.Add(time.Minute))

	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 100; j++ {
				_, _ = c.lookupRust([]byte("facade-close-key"), now)
			}
		}()
	}
	c.closeRust()
	wg.Wait()
	if c.rustActive() {
		t.Fatal("Rust backend remained active after close")
	}
	c.closeRust()
}

func TestRustCGOBackendMatchesGoCacheSemantics(t *testing.T) {
	backend, err := newRustCacheBackend(&Args{Size: 32, LazyCacheTTL: 300})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = backend.Close() })

	tests := []struct {
		name    string
		rcode   int
		answers []dns.RR
	}{
		{name: "positive", rcode: dns.RcodeSuccess, answers: []dns.RR{mustRR(t, "example.org. 60 IN A 192.0.2.1")}},
		{name: "nxdomain", rcode: dns.RcodeNameError},
		{name: "servfail", rcode: dns.RcodeServerFailure},
		{name: "empty", rcode: dns.RcodeSuccess},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			goCache := NewCache(&Args{Size: 16, LazyCacheTTL: 300}, Opts{})
			t.Cleanup(func() { _ = goCache.Close() })
			query := new(dns.Msg)
			query.SetQuestion("example.org.", dns.TypeA)
			qCtx := query_context.NewContext(query)
			response := new(dns.Msg)
			response.SetReply(query)
			response.Rcode = tt.rcode
			response.Answer = tt.answers
			qCtx.SetResponse(response)
			qCtx.StoreValue(query_context.KeyDomainSet, "parity-set")
			keyBytes, pooled := getMsgKeyBytes(query, qCtx, false)
			keyCopy := append([]byte(nil), keyBytes...)
			keyBufferPool.Put(pooled)
			if !saveRespToCache(string(keyCopy), qCtx, goCache.backend, 300) {
				t.Fatal("Go backend rejected parity response")
			}
			value, cacheExpiresAt, ok := goCache.backend.Get(key(keyCopy))
			if !ok || value == nil {
				t.Fatal("Go backend lost parity response")
			}
			if err := backend.Store(keyCopy, value, cacheExpiresAt); err != nil {
				t.Fatal(err)
			}

			goResponse, goLazy, goDomainSet := getRespFromCache(string(keyCopy), goCache.backend, true, expiredMsgTtl)
			rustResponse, err := backend.Lookup(keyCopy, time.Now())
			if err != nil {
				t.Fatal(err)
			}
			if goResponse == nil || goLazy || rustResponse.State != rustCacheFresh || goDomainSet != rustResponse.DomainSet {
				t.Fatalf("state mismatch: go_response=%v go_lazy=%v rust=%+v", goResponse != nil, goLazy, rustResponse)
			}
			unpacked := new(dns.Msg)
			if err := unpacked.Unpack(rustResponse.Response); err != nil {
				t.Fatal(err)
			}
			if unpacked.Rcode != goResponse.Rcode || len(unpacked.Answer) != len(goResponse.Answer) {
				t.Fatalf("response mismatch: go=%v rust=%v", goResponse, unpacked)
			}
		})
	}
}

func TestRustCGOCircuitBreakerFallsBackToGoOnInjectedLookupFault(t *testing.T) {
	t.Setenv(rustCacheBackendEnv, "rust")
	t.Setenv("MOSDNS_CACHE_FAULT_INJECT", "1")
	c := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = c.Close() })
	if !c.rustActive() {
		t.Fatal("Rust backend did not activate before fault injection")
	}

	// The first real lookup must fail and trip the facade's one-way circuit
	// breaker, leaving the Go backend available for subsequent queries.
	if _, ok := c.lookupRust([]byte("fault-key"), time.Now()); ok {
		t.Fatal("injected lookup did not fail")
	}
	if c.rustActive() {
		t.Fatal("circuit breaker did not disable the rust backend")
	}
}

func TestRustCGOFacadeHonorsECSAndExcludeIP(t *testing.T) {
	t.Setenv(rustCacheBackendEnv, "rust")
	c := NewCache(&Args{
		Size:       16,
		EnableECS:  true,
		ExcludeIPs: []string{"192.0.2.0/24"},
	}, Opts{})
	t.Cleanup(func() { _ = c.Close() })
	if !c.rustActive() {
		t.Fatal("Rust backend did not activate")
	}

	query := new(dns.Msg)
	query.SetQuestion("example.org.", dns.TypeA)
	qCtx := query_context.NewContext(query)
	qCtx.QOpt().Option = append(qCtx.QOpt().Option, &dns.EDNS0_SUBNET{
		Code:          dns.EDNS0SUBNET,
		Family:        1,
		SourceNetmask: 24,
		Address:       []byte{198, 51, 100, 0},
	})
	response := new(dns.Msg)
	response.SetReply(query)
	response.Answer = []dns.RR{mustRR(t, "example.org. 60 IN A 192.0.2.8")}
	qCtx.SetResponse(response)
	if err := c.Exec(context.Background(), qCtx, sequence.ChainWalker{}); err != nil {
		t.Fatal(err)
	}
	if length := c.currentCacheLen(); length != 0 {
		t.Fatalf("excluded response entered Rust cache: len=%d", length)
	}
}

func TestRustCGODumpFacadeImportsOnlyValidatedEntries(t *testing.T) {
	t.Setenv(rustCacheBackendEnv, "rust")
	c := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = c.Close() })
	if !c.rustActive() {
		t.Fatal("Rust backend did not activate")
	}

	entry := dumpEntryFixture(t, "rust-dump-key")
	payload := dumpPayload(t, entry, false)
	if count, err := c.readDump(bytes.NewReader(payload)); err != nil || count != 1 {
		t.Fatalf("readDump count=%d err=%v", count, err)
	}
	result, ok := c.lookupRust(entry.GetKey(), time.Now())
	if !ok || result.State != rustCacheFresh || result.DomainSet != entry.GetDomainSet() {
		t.Fatalf("Rust dump lookup mismatch: ok=%v result=%+v", ok, result)
	}

	before := c.currentCacheLen()
	bad := dumpPayload(t, dumpEntryFixture(t, "must-not-import"), true)
	if _, err := c.readDump(bytes.NewReader(bad)); err == nil {
		t.Fatal("malformed dump was accepted")
	}
	if after := c.currentCacheLen(); after != before {
		t.Fatalf("malformed dump changed Rust cache size: before=%d after=%d", before, after)
	}
}
