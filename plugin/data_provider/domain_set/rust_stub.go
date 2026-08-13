//go:build !linux || !cgo || (!mosdns_rust && !mosdns_rust_cache)

package domain_set

func buildRustDomainMatcher(rules []string) (RustMatcher, error) { return nil, nil }
