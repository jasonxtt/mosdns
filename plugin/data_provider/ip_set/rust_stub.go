//go:build !linux || !cgo || (!mosdns_rust && !mosdns_rust_cache)

package ip_set

// buildRustIPMatcher returns nil for non-Rust builds (the default).
func buildRustIPMatcher(prefixes []string) (RustMatcher, error) { return nil, nil }
