//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package cache

import (
	"context"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
)

func BenchmarkCacheFacadeHit(b *testing.B) {
	for _, backend := range []string{"go", "rust"} {
		b.Run(backend, func(b *testing.B) {
			b.Setenv(rustCacheBackendEnv, backend)
			c := NewCache(&Args{Size: 4096}, Opts{})
			b.Cleanup(func() { _ = c.Close() })
			if backend == "rust" && !c.rustActive() {
				b.Skip("Rust cache backend is not available")
			}

			query := new(dns.Msg)
			query.SetQuestion("benchmark.example.", dns.TypeA)
			response := new(dns.Msg)
			response.SetReply(query)
			rr, err := dns.NewRR("benchmark.example. 300 IN A 192.0.2.1")
			if err != nil {
				b.Fatal(err)
			}
			response.Answer = []dns.RR{rr}
			seed := query_context.NewContext(query.Copy())
			seed.SetResponse(response)
			if err := c.Exec(context.Background(), seed, sequence.ChainWalker{}); err != nil {
				b.Fatal(err)
			}

			b.ReportAllocs()
			b.ResetTimer()
			b.RunParallel(func(pb *testing.PB) {
				for pb.Next() {
					qCtx := query_context.NewContext(query.Copy())
					if err := c.Exec(context.Background(), qCtx, sequence.ChainWalker{}); err != nil {
						b.Fatal(err)
					}
				}
			})
		})
	}
}
