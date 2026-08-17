package rust_bridge

import "testing"

// Fixed ECS query fixture shared by the Go-oracle and real-Rust benchmarks:
// example.com A with EDNS Client Subnet family 1 / mask 24 / 1.2.3.0.
func benchmarkQueryFixture() QueryRequest {
	return adapterRequest(adapterQueryWithECS(1, 24, 0, 1, 2, 3, 0))
}

// BenchmarkQueryInspectGoOracle measures the Go oracle that every Rust fallback
// returns. Run with -benchmem for allocs/op and bytes/op.
func BenchmarkQueryInspectGoOracle(b *testing.B) {
	request := benchmarkQueryFixture()
	for i := 0; i < b.N; i++ {
		result, err := GoOracle(request)
		if err != nil {
			b.Fatal(err)
		}
		if result.Backend != BackendGo {
			b.Fatalf("backend = %q, want Go", result.Backend)
		}
	}
}
