//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package ip_set

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

const rustIPMatcherBackendEnv = "MOSDNS_MATCHER_BACKEND"

type rustIPMatcherImpl struct {
	handle atomic.Uint64
}

func buildRustIPMatcher(prefixes []string) (RustMatcher, error) {
	if strings.ToLower(strings.TrimSpace(os.Getenv(rustIPMatcherBackendEnv))) != "rust" {
		return nil, nil
	}
	if uint32(C.cache_abi_version()) != uint32(C.MOSDNS_CACHE_ABI_VERSION) {
		return nil, fmt.Errorf("rust ABI version %d, want %d", uint32(C.cache_abi_version()), uint32(C.MOSDNS_CACHE_ABI_VERSION))
	}
	caps := uint64(C.cache_abi_capabilities())
	if caps&uint64(C.MOSDNS_CACHE_CAPABILITY_MATCHER) == 0 {
		return nil, fmt.Errorf("rust matcher capability not available")
	}
	text := strings.Join(prefixes, "\n")
	var slice C.MosdnsCacheBorrowedSlice
	if len(text) > 0 {
		cData := C.CString(text)
		defer C.free(unsafe.Pointer(cData))
		slice = C.MosdnsCacheBorrowedSlice{
			ptr: (*C.uint8_t)(unsafe.Pointer(cData)),
			len: C.uint64_t(len(text)),
		}
	}
	var handle C.uint64_t
	if status := C.ip_matcher_create(slice, &handle); status != C.MOSDNS_CACHE_OK {
		return nil, fmt.Errorf("rust IP matcher create failed: status=%d", int(status))
	}
	m := &rustIPMatcherImpl{}
	m.handle.Store(uint64(handle))
	return m, nil
}

func (m *rustIPMatcherImpl) Match(addr string) (bool, error) {
	h := m.handle.Load()
	if h == 0 {
		return false, errors.New("rust IP matcher closed")
	}
	cAddr := C.CString(addr)
	defer C.free(unsafe.Pointer(cAddr))
	slice := C.MosdnsCacheBorrowedSlice{
		ptr: (*C.uint8_t)(unsafe.Pointer(cAddr)),
		len: C.uint64_t(len(addr)),
	}
	var matched C.bool
	if status := C.ip_matcher_match(C.uint64_t(h), slice, &matched); status != C.MOSDNS_CACHE_OK {
		return false, fmt.Errorf("rust IP matcher match failed: status=%d", int(status))
	}
	return bool(matched), nil
}

// Len reports the number of accepted prefix entries in the immutable Rust snapshot.
func (m *rustIPMatcherImpl) Len() (uint64, error) {
	h := m.handle.Load()
	if h == 0 {
		return 0, errors.New("rust IP matcher closed")
	}
	var length C.uint64_t
	if status := C.ip_matcher_len(C.uint64_t(h), &length); status != C.MOSDNS_CACHE_OK {
		return 0, fmt.Errorf("rust IP matcher len failed: status=%d", int(status))
	}
	return uint64(length), nil
}

func (m *rustIPMatcherImpl) Close() error {
	h := m.handle.Swap(0)
	if h == 0 {
		return nil
	}
	if status := C.ip_matcher_close(C.uint64_t(h)); status != C.MOSDNS_CACHE_OK && status != C.MOSDNS_CACHE_CLOSED {
		return fmt.Errorf("rust IP matcher close failed: status=%d", int(status))
	}
	return nil
}
