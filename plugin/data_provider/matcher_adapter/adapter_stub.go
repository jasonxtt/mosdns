//go:build !linux || !cgo || (!mosdns_rust && !mosdns_rust_cache)

package matcher_adapter

// BuildDomainSnapshot returns nil unless the Linux+cgo Rust build is selected.
func BuildDomainSnapshot([]string) (DomainSnapshot, error) { return nil, nil }

// BuildIPSnapshot returns nil unless the Linux+cgo Rust build is selected.
func BuildIPSnapshot([]string) (IPSnapshot, error) { return nil, nil }

// BuildValuedDomainSnapshot returns nil unless the Linux+cgo Rust build is selected.
func BuildValuedDomainSnapshot([]ValuedRule) (ValuedSnapshot, error) { return nil, nil }
