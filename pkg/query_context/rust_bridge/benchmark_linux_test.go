//go:build linux && cgo && mosdns_rust

package rust_bridge

import (
	"runtime"
	"testing"
)

// BenchmarkQueryInspectRustAdapter measures the full opt-in Rust path through
// the real static library: create -> required len -> inspect -> close per
// call. It deliberately runs the entire adapter so the reported cgo_calls/op
// reflects the real ABI round trips for one Inspect.
func BenchmarkQueryInspectRustAdapter(b *testing.B) {
	request := benchmarkQueryFixture()
	b.Setenv(queryBackendEnv, "rust")
	// Pre-check that the real backend is actually selected so a silent Go
	// fallback cannot skew the numbers.
	probe, err := Inspect(request)
	if err != nil {
		b.Fatalf("probe Inspect: %v", err)
	}
	if probe.Backend != BackendRust {
		b.Fatalf("probe backend = %q, want real Rust", probe.Backend)
	}

	cgoBefore := runtime.NumCgoCall()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		result, err := Inspect(request)
		if err != nil {
			b.Fatal(err)
		}
		if result.Backend != BackendRust {
			b.Fatalf("backend = %q, want Rust", result.Backend)
		}
	}
	b.StopTimer()
	b.ReportMetric(float64(runtime.NumCgoCall()-cgoBefore)/float64(b.N), "cgo_calls/op")
}
