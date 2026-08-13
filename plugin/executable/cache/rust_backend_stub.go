//go:build !linux || !cgo || (!mosdns_rust && !mosdns_rust_cache)

package cache

func newRustCacheBackend(*Args) (rustCacheBackend, error) {
	return nil, errRustCacheUnavailable
}
