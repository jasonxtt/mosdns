//go:build !linux || !cgo || !mosdns_rust

package rust_bridge

import "testing"

func TestStubBuildKeepsGoOracleWhenRustIsRequested(t *testing.T) {
	t.Setenv(queryBackendEnv, "rust")
	result, err := Inspect(QueryRequest{QueryWire: adapterValidQuery()})
	if !IsFallbackError(err) {
		t.Fatalf("err = %v, want typed fallback error", err)
	}
	if result.Backend != BackendGo || len(result.QueryWire) == 0 {
		t.Fatalf("stub fallback result = %+v", result)
	}
}
