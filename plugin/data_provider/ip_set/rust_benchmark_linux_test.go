//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package ip_set

import (
	"fmt"
	"net/netip"
	"runtime"
	"strings"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
)

var rustIPBenchmarkSink bool

func rustIPBenchmarkPrefixes() []string {
	prefixes := make([]string, 0, 512)
	for i := 0; i < 256; i++ {
		prefixes = append(prefixes, fmt.Sprintf("192.0.%d.0/24", i))
	}
	for i := 0; i < 256; i++ {
		prefixes = append(prefixes, fmt.Sprintf("2001:db8:%x::/48", i))
	}
	return prefixes
}

func buildGoIPBenchmarkMatcher(prefixes []string) *netlist.List {
	list := netlist.NewList()
	for _, prefix := range prefixes {
		parsed, err := netip.ParsePrefix(prefix)
		if err != nil {
			panic(err)
		}
		list.Append(parsed)
	}
	list.Sort()
	return list
}

func BenchmarkRustIPBuild(b *testing.B) {
	prefixes := rustIPBenchmarkPrefixes()
	prefixBytes := len(strings.Join(prefixes, "\n"))

	b.Run("go", func(b *testing.B) {
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			_ = buildGoIPBenchmarkMatcher(prefixes)
		}
		b.StopTimer()
		b.ReportMetric(float64(len(prefixes)), "fixture_prefixes")
		b.ReportMetric(float64(prefixBytes), "fixture_bytes")
	})

	b.Run("rust", func(b *testing.B) {
		b.Setenv(rustIPMatcherBackendEnv, "rust")
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			matcher, err := BuildRustIPMatcher(prefixes)
			if err != nil {
				b.Fatal(err)
			}
			if matcher == nil {
				b.Fatal("Rust matcher was not enabled")
			}
			b.StopTimer()
			if err := matcher.Close(); err != nil {
				b.Fatal(err)
			}
			b.StartTimer()
		}
		b.StopTimer()
		b.ReportMetric(float64(len(prefixes)), "fixture_prefixes")
		b.ReportMetric(float64(prefixBytes), "fixture_bytes")
	})
}

func BenchmarkRustIPLookup(b *testing.B) {
	prefixes := rustIPBenchmarkPrefixes()
	prefixBytes := len(strings.Join(prefixes, "\n"))
	goMatcher := buildGoIPBenchmarkMatcher(prefixes)
	b.Setenv(rustIPMatcherBackendEnv, "rust")
	rustMatcher, err := BuildRustIPMatcher(prefixes)
	if err != nil {
		b.Fatal(err)
	}
	if rustMatcher == nil {
		b.Fatal("Rust matcher was not enabled")
	}
	defer rustMatcher.Close()
	prefixesInIndex, ok := rustMatcher.(interface{ Len() (uint64, error) })
	if !ok {
		b.Fatal("Rust matcher does not expose snapshot length")
	}
	indexLen, err := prefixesInIndex.Len()
	if err != nil {
		b.Fatal(err)
	}

	queries := []netip.Addr{
		netip.MustParseAddr("192.0.42.1"),
		netip.MustParseAddr("198.51.100.1"),
		netip.MustParseAddr("2001:db8:2a::1"),
		netip.MustParseAddr("2001:db9::1"),
	}
	b.Run("go", func(b *testing.B) {
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			rustIPBenchmarkSink = goMatcher.Contains(queries[i%len(queries)])
		}
		b.StopTimer()
		b.ReportMetric(float64(len(prefixes)), "fixture_prefixes")
		b.ReportMetric(float64(prefixBytes), "fixture_bytes")
		b.ReportMetric(float64(indexLen), "index_entries")
	})

	b.Run("rust", func(b *testing.B) {
		cgoCallsBefore := runtime.NumCgoCall()
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			matched, err := rustMatcher.Match(queries[i%len(queries)].String())
			if err != nil {
				b.Fatal(err)
			}
			rustIPBenchmarkSink = matched
		}
		b.StopTimer()
		b.ReportMetric(float64(runtime.NumCgoCall()-cgoCallsBefore)/float64(b.N), "cgo_calls/op")
		b.ReportMetric(float64(len(prefixes)), "fixture_prefixes")
		b.ReportMetric(float64(prefixBytes), "fixture_bytes")
		b.ReportMetric(float64(indexLen), "index_entries")
	})
}
