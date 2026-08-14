//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package domain_mapper

import (
	"runtime"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
)

var slice5MapperBenchmarkSink matcher_adapter.ValuedResult

func logicalValuedResultBytes(result matcher_adapter.ValuedResult) int {
	return 1 + len(result.FastMarks) + 4*len(result.CtxMarks) + len(result.JoinedTags) + len(result.JoinedSources)
}

func BenchmarkRustMapperBuild(b *testing.B) {
	fixture := newSlice5MapperBenchmarkFixture()
	b.Run("go", func(b *testing.B) {
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			candidate := (&DomainMapper{}).buildGoCandidate(slice5MapperBenchmarkAggregation(fixture))
			if candidate.matcher == nil {
				b.Fatal("Go mapper candidate is nil")
			}
		}
		b.StopTimer()
		b.ReportMetric(float64(len(fixture.rules)), "fixture_rules")
		b.ReportMetric(float64(fixture.fixtureBytes), "fixture_bytes")
		b.ReportMetric(float64(fixture.resultBytes), "result_bytes")
	})

	b.Run("rust", func(b *testing.B) {
		b.Setenv(matcher_adapter.BackendEnv, "rust")
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			snapshot, err := matcher_adapter.BuildValuedDomainSnapshot(fixture.rules)
			if err != nil {
				b.Fatal(err)
			}
			if snapshot == nil {
				b.Fatal("Rust valued mapper snapshot was not enabled")
			}
			b.StopTimer()
			if err := snapshot.Close(); err != nil {
				b.Fatal(err)
			}
			b.StartTimer()
		}
		b.StopTimer()
		b.ReportMetric(float64(len(fixture.rules)), "fixture_rules")
		b.ReportMetric(float64(fixture.fixtureBytes), "fixture_bytes")
		b.ReportMetric(float64(fixture.resultBytes), "result_bytes")
	})
}

func BenchmarkRustMapperLookup(b *testing.B) {
	fixture := newSlice5MapperBenchmarkFixture()
	goCandidate := (&DomainMapper{}).buildGoCandidate(slice5MapperBenchmarkAggregation(fixture))
	b.Setenv(matcher_adapter.BackendEnv, "rust")
	rustSnapshot, err := matcher_adapter.BuildValuedDomainSnapshot(fixture.rules)
	if err != nil {
		b.Fatal(err)
	}
	if rustSnapshot == nil {
		b.Fatal("Rust valued mapper snapshot was not enabled")
	}
	defer rustSnapshot.Close()
	probe, err := rustSnapshot.Match(fixture.queries[0])
	if err != nil {
		b.Fatal(err)
	}
	resultBytes := logicalValuedResultBytes(probe)

	b.Run("go", func(b *testing.B) {
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			result, ok := goCandidate.matcher.match(fixture.queries[i%len(fixture.queries)])
			if !ok || result == nil {
				b.Fatal("Go mapper fixture unexpectedly missed")
			}
			slice5MapperBenchmarkSink = matcher_adapter.ValuedResult{
				Matched:       true,
				FastMarks:     append([]uint8(nil), result.FastMarks...),
				CtxMarks:      append([]uint32(nil), result.CtxMarks...),
				JoinedTags:    result.JoinedTags,
				JoinedSources: result.JoinedSources,
			}
		}
		b.StopTimer()
		b.ReportMetric(float64(len(fixture.rules)), "fixture_rules")
		b.ReportMetric(float64(fixture.fixtureBytes), "fixture_bytes")
		b.ReportMetric(float64(resultBytes), "result_bytes")
	})

	b.Run("rust", func(b *testing.B) {
		cgoCallsBefore := runtime.NumCgoCall()
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			result, err := rustSnapshot.Match(fixture.queries[i%len(fixture.queries)])
			if err != nil {
				b.Fatal(err)
			}
			slice5MapperBenchmarkSink = result
		}
		b.StopTimer()
		b.ReportMetric(float64(runtime.NumCgoCall()-cgoCallsBefore)/float64(b.N), "cgo_calls/op")
		b.ReportMetric(float64(len(fixture.rules)), "fixture_rules")
		b.ReportMetric(float64(fixture.fixtureBytes), "fixture_bytes")
		b.ReportMetric(float64(resultBytes), "result_bytes")
	})
}
