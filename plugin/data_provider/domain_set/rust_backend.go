//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package domain_set

/*
#cgo CFLAGS: -I${SRCDIR}/../../../rust/runtime/include
#cgo LDFLAGS: -L${SRCDIR}/../../../rust/target/release -lmosdns_runtime -ldl -lm -lpthread
#include <stdlib.h>
#include "mosdns_cache_core.h"
*/
import "C"
import (
	"errors"
	"fmt"
	"os"
	"strings"
	"sync/atomic"
	"unsafe"
)

const rustMatcherBackendEnv = "MOSDNS_MATCHER_BACKEND"

type rustDomainMatcher struct {
	handle atomic.Uint64
}

func buildRustDomainMatcher(rules []string) (RustMatcher, error) {
	if strings.ToLower(strings.TrimSpace(os.Getenv(rustMatcherBackendEnv))) != "rust" {
		return nil, nil // not requested
	}
	if uint32(C.cache_abi_version()) != uint32(C.MOSDNS_CACHE_ABI_VERSION) {
		return nil, fmt.Errorf("rust ABI version %d, want %d", uint32(C.cache_abi_version()), uint32(C.MOSDNS_CACHE_ABI_VERSION))
	}
	caps := uint64(C.cache_abi_capabilities())
	if caps&uint64(C.MOSDNS_CACHE_CAPABILITY_MATCHER) == 0 {
		return nil, fmt.Errorf("rust matcher capability not available")
	}
	text := strings.Join(rules, "\n")
	var slice C.MosdnsCacheBorrowedSlice
	if len(text) > 0 {
		cRules := C.CString(text)
		defer C.free(unsafe.Pointer(cRules))
		slice = C.MosdnsCacheBorrowedSlice{
			ptr: (*C.uint8_t)(unsafe.Pointer(cRules)),
			len: C.uint64_t(len(text)),
		}
	}
	var handle C.uint64_t
	if status := C.domain_matcher_create(slice, 0, &handle); status != C.MOSDNS_CACHE_OK {
		return nil, fmt.Errorf("rust domain matcher create failed: status=%d", int(status))
	}
	m := &rustDomainMatcher{}
	m.handle.Store(uint64(handle))
	return m, nil
}

func (m *rustDomainMatcher) Match(domain string) (bool, error) {
	h := m.handle.Load()
	if h == 0 {
		return false, errors.New("rust domain matcher closed")
	}
	cDomain := C.CString(domain)
	defer C.free(unsafe.Pointer(cDomain))
	slice := C.MosdnsCacheBorrowedSlice{
		ptr: (*C.uint8_t)(unsafe.Pointer(cDomain)),
		len: C.uint64_t(len(domain)),
	}
	var matched C.bool
	if status := C.domain_matcher_match(C.uint64_t(h), slice, &matched); status != C.MOSDNS_CACHE_OK {
		return false, fmt.Errorf("rust domain matcher match failed: status=%d", int(status))
	}
	return bool(matched), nil
}

// Len reports the number of accepted rules in the immutable Rust snapshot.
func (m *rustDomainMatcher) Len() (uint64, error) {
	h := m.handle.Load()
	if h == 0 {
		return 0, errors.New("rust domain matcher closed")
	}
	var length C.uint64_t
	if status := C.domain_matcher_len(C.uint64_t(h), &length); status != C.MOSDNS_CACHE_OK {
		return 0, fmt.Errorf("rust domain matcher len failed: status=%d", int(status))
	}
	return uint64(length), nil
}

// Close releases the Rust handle. It is safe to call more than once.
func (m *rustDomainMatcher) Close() error { return m.close() }

func (m *rustDomainMatcher) close() error {
	h := m.handle.Swap(0)
	if h == 0 {
		return nil
	}
	if status := C.domain_matcher_close(C.uint64_t(h)); status != C.MOSDNS_CACHE_OK && status != C.MOSDNS_CACHE_CLOSED {
		return fmt.Errorf("rust domain matcher close failed: status=%d", int(status))
	}
	return nil
}
