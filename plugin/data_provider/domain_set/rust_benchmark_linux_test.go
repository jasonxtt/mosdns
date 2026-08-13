//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package domain_set

import (
	"fmt"
	"runtime"
	"strings"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
)

var rustDomainBenchmarkSink bool

func rustDomainBenchmarkRules() []string {
	rules := make([]string, 0, 512)
	for i := 0; i < 128; i++ {
		rules = append(rules,
			fmt.Sprintf("full:full-%04d.bench.example", i),
			fmt.Sprintf("domain:suffix-%04d.bench.example", i),
			fmt.Sprintf(`regexp:^regex-%04d\.bench\.example$`, i),
			fmt.Sprintf("keyword:keyword-%04d", i),
		)
	}
	return rules
}

func buildRustDomainBenchmarkMatcher(rules []string) (RustMatcher, error) {
	return BuildRustDomainMatcher(rules)
}

func buildGoDomainBenchmarkMatcher(rules []string) *domain.MixMatcher[struct{}] {
	m := domain.NewDomainMixMatcher()
	for _, rule := range rules {
		if err := m.Add(rule, struct{}{}); err != nil {
			panic(err)
		}
	}
	return m
}

func BenchmarkRustDomainBuild(b *testing.B) {
	rules := rustDomainBenchmarkRules()
	ruleBytes := len(strings.Join(rules, "\n"))

	b.Run("go", func(b *testing.B) {
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			_ = buildGoDomainBenchmarkMatcher(rules)
		}
		b.StopTimer()
		b.ReportMetric(float64(len(rules)), "fixture_rules")
		b.ReportMetric(float64(ruleBytes), "fixture_bytes")
	})

	b.Run("rust", func(b *testing.B) {
		b.Setenv(rustMatcherBackendEnv, "rust")
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			matcher, err := buildRustDomainBenchmarkMatcher(rules)
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
		b.ReportMetric(float64(len(rules)), "fixture_rules")
		b.ReportMetric(float64(ruleBytes), "fixture_bytes")
	})
}

func BenchmarkRustDomainLookup(b *testing.B) {
	rules := rustDomainBenchmarkRules()
	ruleBytes := len(strings.Join(rules, "\n"))
	goMatcher := buildGoDomainBenchmarkMatcher(rules)
	b.Setenv(rustMatcherBackendEnv, "rust")
	rustMatcher, err := buildRustDomainBenchmarkMatcher(rules)
	if err != nil {
		b.Fatal(err)
	}
	if rustMatcher == nil {
		b.Fatal("Rust matcher was not enabled")
	}
	defer rustMatcher.Close()
	rulesInIndex, ok := rustMatcher.(interface{ Len() (uint64, error) })
	if !ok {
		b.Fatal("Rust matcher does not expose snapshot length")
	}
	indexLen, err := rulesInIndex.Len()
	if err != nil {
		b.Fatal(err)
	}

	queries := []string{"full-0042.bench.example.", "not-present.bench.example."}
	b.Run("go", func(b *testing.B) {
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			_, matched := goMatcher.Match(queries[i%len(queries)])
			rustDomainBenchmarkSink = matched
		}
		b.StopTimer()
		b.ReportMetric(float64(len(rules)), "fixture_rules")
		b.ReportMetric(float64(ruleBytes), "fixture_bytes")
		b.ReportMetric(float64(indexLen), "index_entries")
	})

	b.Run("rust", func(b *testing.B) {
		cgoCallsBefore := runtime.NumCgoCall()
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			matched, err := rustMatcher.Match(queries[i%len(queries)])
			if err != nil {
				b.Fatal(err)
			}
			rustDomainBenchmarkSink = matched
		}
		b.StopTimer()
		b.ReportMetric(float64(runtime.NumCgoCall()-cgoCallsBefore)/float64(b.N), "cgo_calls/op")
		b.ReportMetric(float64(len(rules)), "fixture_rules")
		b.ReportMetric(float64(ruleBytes), "fixture_bytes")
		b.ReportMetric(float64(indexLen), "index_entries")
	})
}
