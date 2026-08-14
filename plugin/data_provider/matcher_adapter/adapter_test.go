package matcher_adapter

import "testing"

func TestMatcherAdapterIsOptInAndExposesTypedSnapshots(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "")

	domainSnapshot, err := BuildDomainSnapshot([]string{"full:example.com"})
	if err != nil {
		t.Fatal(err)
	}
	if domainSnapshot != nil {
		t.Fatal("default matcher adapter must not select Rust")
	}

	ipSnapshot, err := BuildIPSnapshot([]string{"192.0.2.0/24"})
	if err != nil {
		t.Fatal(err)
	}
	if ipSnapshot != nil {
		t.Fatal("default matcher adapter must not select Rust")
	}

	valuedSnapshot, err := BuildValuedDomainSnapshot([]ValuedRule{{Rule: "full:example.com"}})
	if err != nil {
		t.Fatal(err)
	}
	if valuedSnapshot != nil {
		t.Fatal("default valued matcher adapter must not select Rust")
	}
}

func TestMatcherAdapterClassifiesRuntimeFailures(t *testing.T) {
	if !IsCircuitBreakerError(&Error{Class: ErrorClassRuntime}) {
		t.Fatal("runtime failures must trip the matcher circuit breaker")
	}
	if !IsCircuitBreakerError(&Error{Class: ErrorClassCircuitBroken}) {
		t.Fatal("circuit-breaker failures must remain classified as disabling")
	}
	if IsCircuitBreakerError(&Error{Class: ErrorClassInvalidArgument}) {
		t.Fatal("invalid arguments must not be classified as runtime failures")
	}
}
