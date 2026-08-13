//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package cache

/*
#cgo CFLAGS: -I${SRCDIR}/../../../rust/runtime/include
#cgo LDFLAGS: -L${SRCDIR}/../../../rust/target/release -lmosdns_runtime -ldl -lm -lpthread
#include "mosdns_cache_core.h"
*/
import "C"

import (
	"errors"
	"fmt"
	"math"
	"os"
	"runtime"
	"sync/atomic"
	"time"
	"unsafe"

	"github.com/IrineSistiana/mosdns/v5/pkg/pool"
	"github.com/miekg/dns"
)

const (
	requiredRustCacheCapabilities  = uint64(C.MOSDNS_CACHE_CAPABILITY_LIFECYCLE | C.MOSDNS_CACHE_CAPABILITY_CACHE | C.MOSDNS_CACHE_CAPABILITY_LOOKUP_INTO)
	initialRustResponseBufferSize  = 4096
	initialRustDomainSetBufferSize = 256
)

type cgoRustCacheBackend struct {
	handle atomic.Uint64
	// faultInjectLookup is set from MOSDNS_CACHE_FAULT_INJECT=1 so the
	// test-host verification can deterministically trip the facade's one-way
	// circuit breaker mid-traffic. It only makes Lookup fail; seeding, store,
	// len, flush, and close stay functional so the failure is a clean runtime
	// fault rather than an init failure. It is never enabled in a normal build.
	faultInjectLookup atomic.Bool
}

func newRustCacheBackend(args *Args) (rustCacheBackend, error) {
	if version := uint32(C.cache_abi_version()); version != uint32(C.MOSDNS_CACHE_ABI_VERSION) {
		return nil, fmt.Errorf("rust cache ABI version %d, want %d", version, uint32(C.MOSDNS_CACHE_ABI_VERSION))
	}
	capabilities := uint64(C.cache_abi_capabilities())
	if capabilities&requiredRustCacheCapabilities != requiredRustCacheCapabilities {
		return nil, fmt.Errorf("rust cache capabilities %#x do not include %#x", capabilities, requiredRustCacheCapabilities)
	}
	if args.Size <= 0 {
		return nil, fmt.Errorf("invalid rust cache size %d", args.Size)
	}
	config := C.MosdnsCacheConfig{
		capacity:            C.uint64_t(args.Size),
		lazy_cache_ttl_secs: C.uint32_t(args.LazyCacheTTL),
		flags:               0,
	}
	var handle C.uint64_t
	if status := C.cache_create(&config, &handle); status != C.MOSDNS_CACHE_OK {
		return nil, statusError("create", status)
	}
	backend := new(cgoRustCacheBackend)
	backend.handle.Store(uint64(handle))
	backend.faultInjectLookup.Store(os.Getenv("MOSDNS_CACHE_FAULT_INJECT") == "1")
	return backend, nil
}

func (*cgoRustCacheBackend) Name() string {
	return fmt.Sprintf("mosdns-cache-core/abi-%d", uint32(C.cache_abi_version()))
}

func (b *cgoRustCacheBackend) Lookup(key []byte, now time.Time) (rustCacheLookupResult, error) {
	if b.faultInjectLookup.Load() {
		return rustCacheLookupResult{}, errors.New("injected rust cache lookup fault")
	}
	handle := b.handle.Load()
	if handle == 0 {
		return rustCacheLookupResult{}, errRustCacheUnavailable
	}
	responseBuffer := pool.GetBuf(initialRustResponseBufferSize)
	domainSetBuffer := pool.GetBuf(initialRustDomainSetBufferSize)
	defer func() {
		pool.ReleaseBuf(responseBuffer)
		pool.ReleaseBuf(domainSetBuffer)
	}()
	var result C.MosdnsCacheLookupIntoResult
	for attempt := 0; attempt < 2; attempt++ {
		status := C.cache_lookup_into(
			C.uint64_t(handle),
			borrowedSlice(key),
			C.int64_t(now.Unix()),
			writableSlice(*responseBuffer),
			writableSlice(*domainSetBuffer),
			&result,
		)
		runtime.KeepAlive(key)
		if status == C.MOSDNS_CACHE_BUFFER_TOO_SMALL && attempt == 0 {
			responseLen, err := checkedBufferLength(result.response_len, dns.MaxMsgSize)
			if err != nil {
				return rustCacheLookupResult{}, err
			}
			domainSetLen, err := checkedBufferLength(result.domain_set_len, dumpMaximumBlockLength)
			if err != nil {
				return rustCacheLookupResult{}, err
			}
			pool.ReleaseBuf(responseBuffer)
			pool.ReleaseBuf(domainSetBuffer)
			responseBuffer = pool.GetBuf(responseLen)
			domainSetBuffer = pool.GetBuf(domainSetLen)
			continue
		}
		if status != C.MOSDNS_CACHE_OK {
			return rustCacheLookupResult{}, statusError("lookup", status)
		}
		break
	}
	responseLen, err := checkedBufferLength(result.response_len, len(*responseBuffer))
	if err != nil {
		return rustCacheLookupResult{}, err
	}
	domainSetLen, err := checkedBufferLength(result.domain_set_len, len(*domainSetBuffer))
	if err != nil {
		return rustCacheLookupResult{}, err
	}
	response := append([]byte(nil), (*responseBuffer)[:responseLen]...)
	domainSet := string((*domainSetBuffer)[:domainSetLen])
	state := rustCacheMiss
	switch result.state {
	case C.MOSDNS_CACHE_FRESH:
		state = rustCacheFresh
	case C.MOSDNS_CACHE_LAZY:
		state = rustCacheLazy
	}
	return rustCacheLookupResult{
		State:     state,
		Response:  response,
		DomainSet: domainSet,
		StoredAt:  time.Unix(int64(result.stored_at_unix), 0),
	}, nil
}

func (b *cgoRustCacheBackend) Store(key []byte, value *item, cacheExpiresAt time.Time) error {
	if value == nil || len(key) == 0 || len(value.resp) == 0 {
		return fmt.Errorf("invalid empty rust cache store")
	}
	handle := b.handle.Load()
	if handle == 0 {
		return errRustCacheUnavailable
	}
	domainSet := []byte(value.domainSet)
	status := C.cache_store(
		C.uint64_t(handle),
		borrowedSlice(key),
		borrowedSlice(value.resp),
		borrowedSlice(domainSet),
		C.int64_t(value.storedTime.Unix()),
		C.int64_t(value.expirationTime.Unix()),
		C.int64_t(cacheExpiresAt.Unix()),
	)
	runtime.KeepAlive(key)
	runtime.KeepAlive(value.resp)
	runtime.KeepAlive(domainSet)
	if status != C.MOSDNS_CACHE_OK {
		return statusError("store", status)
	}
	return nil
}

func (b *cgoRustCacheBackend) Len() (int, error) {
	handle := b.handle.Load()
	if handle == 0 {
		return 0, errRustCacheUnavailable
	}
	var length C.uint64_t
	if status := C.cache_len(C.uint64_t(handle), &length); status != C.MOSDNS_CACHE_OK {
		return 0, statusError("len", status)
	}
	if uint64(length) > uint64(math.MaxInt) {
		return 0, fmt.Errorf("rust cache length %d exceeds Go int", uint64(length))
	}
	return int(length), nil
}

func (b *cgoRustCacheBackend) Flush() error {
	handle := b.handle.Load()
	if handle == 0 {
		return nil
	}
	if status := C.cache_flush(C.uint64_t(handle)); status != C.MOSDNS_CACHE_OK {
		return statusError("flush", status)
	}
	return nil
}

func (b *cgoRustCacheBackend) Close() error {
	handle := b.handle.Swap(0)
	if handle == 0 {
		return nil
	}
	status := C.cache_close(C.uint64_t(handle))
	if status != C.MOSDNS_CACHE_OK && status != C.MOSDNS_CACHE_CLOSED {
		return statusError("close", status)
	}
	return nil
}

func borrowedSlice(data []byte) C.MosdnsCacheBorrowedSlice {
	if len(data) == 0 {
		return C.MosdnsCacheBorrowedSlice{}
	}
	return C.MosdnsCacheBorrowedSlice{
		ptr: (*C.uint8_t)(unsafe.Pointer(&data[0])),
		len: C.uint64_t(len(data)),
	}
}

func writableSlice(data []byte) C.MosdnsCacheWritableSlice {
	if len(data) == 0 {
		return C.MosdnsCacheWritableSlice{}
	}
	return C.MosdnsCacheWritableSlice{
		ptr: (*C.uint8_t)(unsafe.Pointer(&data[0])),
		len: C.uint64_t(len(data)),
	}
}

func checkedBufferLength(length C.uint64_t, limit int) (int, error) {
	if uint64(length) > uint64(limit) {
		return 0, fmt.Errorf("rust cache output length %d exceeds limit %d", uint64(length), limit)
	}
	return int(length), nil
}

func copyOwnedBuffer(buffer C.MosdnsCacheOwnedBuffer) ([]byte, error) {
	if buffer.ptr == nil && buffer.len == 0 {
		return nil, nil
	}
	if buffer.ptr == nil {
		return nil, fmt.Errorf("invalid rust-owned buffer length %d", uint64(buffer.len))
	}
	if uint64(buffer.len) > uint64(math.MaxInt32) {
		if status := C.cache_buffer_release(buffer); status != C.MOSDNS_CACHE_OK {
			return nil, statusError("oversized buffer release", status)
		}
		return nil, fmt.Errorf("rust-owned buffer length %d exceeds cgo copy limit", uint64(buffer.len))
	}
	data := C.GoBytes(unsafe.Pointer(buffer.ptr), C.int(buffer.len))
	if status := C.cache_buffer_release(buffer); status != C.MOSDNS_CACHE_OK {
		return nil, statusError("buffer release", status)
	}
	return data, nil
}

func statusError(operation string, status C.MosdnsCacheStatus) error {
	return fmt.Errorf("rust cache %s failed with status %d", operation, int(status))
}
