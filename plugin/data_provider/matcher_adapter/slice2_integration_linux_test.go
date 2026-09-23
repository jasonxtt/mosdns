//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package matcher_adapter

import "testing"

func TestSlice2RealDomainAdapterRejectsUnsafeRegexp(t *testing.T) {
	t.Setenv(BackendEnv, "rust")
	snapshot, err := BuildDomainSnapshot([]string{`regexp:^\w+$`})
	if err == nil {
		t.Fatal("unsafe regexp unexpectedly built in the Rust domain adapter")
	}
	if snapshot != nil {
		t.Fatalf("unsafe regexp returned a non-nil Rust snapshot despite the build error: %T", snapshot)
	}
}

func TestSlice2RealIPAdapterRejectsInvalidPrefix(t *testing.T) {
	t.Setenv(BackendEnv, "rust")
	snapshot, err := BuildIPSnapshot([]string{"not-an-ip"})
	if err == nil {
		t.Fatal("invalid prefix unexpectedly built in the Rust IP adapter")
	}
	if snapshot != nil {
		t.Fatalf("invalid prefix returned a non-nil Rust IP snapshot despite the build error: %T", snapshot)
	}
}

func TestSlice2RealDomainAdapterAcceptsSafeRegexp(t *testing.T) {
	t.Setenv(BackendEnv, "rust")
	snapshot, err := BuildDomainSnapshot([]string{`regexp:^safe\.example$`})
	if err != nil {
		t.Fatal(err)
	}
	if snapshot == nil {
		t.Fatal("safe regexp did not produce a Rust domain snapshot")
	}
	t.Cleanup(func() { _ = snapshot.Close() })
	matched, err := snapshot.Match("safe.example.")
	if err != nil || !matched {
		t.Fatalf("Rust safe regexp match = (%v, %v), want (true, nil)", matched, err)
	}
}

func TestSlice2RealValuedAdapterRejectsUnsafeRegexp(t *testing.T) {
	t.Setenv(BackendEnv, "rust")
	snapshot, err := BuildValuedDomainSnapshot([]ValuedRule{{Rule: `regexp:^\w+$`}})
	if err == nil {
		t.Fatal("unsafe regexp unexpectedly built in the Rust valued adapter")
	}
	if snapshot != nil {
		t.Fatalf("unsafe regexp returned a non-nil valued Rust snapshot despite the build error: %T", snapshot)
	}
}
