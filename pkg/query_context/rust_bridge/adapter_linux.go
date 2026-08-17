//go:build linux && cgo && mosdns_rust

package rust_bridge

/*
#cgo CFLAGS: -I${SRCDIR}/../../../rust/runtime/include
#cgo LDFLAGS: -L${SRCDIR}/../../../rust/target/release -lmosdns_runtime -ldl -lm -lpthread
#include "mosdns_cache_core.h"
*/
import "C"

import (
	"runtime"
	"unsafe"
)

type cgoQueryABI struct{}

func newNativeABI() (queryABI, error) { return cgoQueryABI{}, nil }

func (cgoQueryABI) Version() uint32 { return uint32(C.query_abi_version()) }

func (cgoQueryABI) Capabilities() uint64 { return uint64(C.query_abi_capabilities()) }

func (cgoQueryABI) Create(input queryABIInput) (uint64, error) {
	var rawInput C.MosdnsQuerySnapshotInput
	rawInput.struct_size = C.uint32_t(C.sizeof_MosdnsQuerySnapshotInput)
	rawInput.version = C.uint32_t(input.Version)
	rawInput.flags = C.uint32_t(input.Flags)
	rawInput.query_wire = borrowedSlice(input.QueryWire)
	if input.FromUDP {
		rawInput.from_udp = 1
	}
	rawInput.transport_mode = C.uint8_t(input.Transport)
	rawInput.advertised_udp_size = C.uint16_t(input.AdvertisedUDPSize)
	rawInput.pre_fast_flags = C.uint64_t(input.PreFastFlags)

	var handle C.uint64_t
	status := C.query_snapshot_create(rawInput, &handle)
	runtime.KeepAlive(input.QueryWire)
	if status != C.MOSDNS_CACHE_OK {
		return 0, statusError("create", abiStatus(uint32(status)))
	}
	return uint64(handle), nil
}

func (cgoQueryABI) RequiredLen(handle uint64) (uint64, error) {
	var length C.uint64_t
	status := C.query_snapshot_required_len(C.uint64_t(handle), &length)
	if status != C.MOSDNS_CACHE_OK {
		return 0, statusError("required length", abiStatus(uint32(status)))
	}
	return uint64(length), nil
}

func (cgoQueryABI) Inspect(handle uint64, output []byte) (queryABIResult, error) {
	var raw C.MosdnsQueryInspectResult
	status := C.query_snapshot_inspect(
		C.uint64_t(handle),
		writableSlice(output),
		&raw,
	)
	runtime.KeepAlive(output)
	return convertResult(raw, status)
}

func (cgoQueryABI) Close(handle uint64) error {
	status := C.query_snapshot_close(C.uint64_t(handle))
	if status != C.MOSDNS_CACHE_OK && status != C.MOSDNS_CACHE_CLOSED {
		return statusError("close", abiStatus(uint32(status)))
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

func convertResult(raw C.MosdnsQueryInspectResult, callStatus C.MosdnsCacheStatus) (queryABIResult, error) {
	result := queryABIResult{
		Status:            abiStatus(uint32(raw.status)),
		Version:           uint32(raw.version),
		Flags:             uint32(raw.flags),
		ID:                uint16(raw.id),
		QType:             uint16(raw.qtype),
		QClass:            uint16(raw.qclass),
		AdvertisedUDPSize: uint16(raw.advertised_udp_size),
		EDNSUDPSize:       uint16(raw.edns_udp_size),
		ECSFamily:         uint16(raw.ecs_family),
		ECSSourceNetmask:  uint8(raw.ecs_source_netmask),
		ECSSourceScope:    uint8(raw.ecs_source_scope),
		FromUDP:           uint8(raw.from_udp),
		Transport:         TransportMode(uint8(raw.transport_mode)),
		HasOPT:            uint8(raw.has_opt),
		DOBit:             uint8(raw.do_bit),
		ECS:               uint8(raw.ecs_present),
		Reserved:          uint8(raw.reserved),
		QNameLen:          uint64(raw.qname_len),
		PreFastFlags:      uint64(raw.pre_fast_flags),
		RequiredLen:       uint64(raw.required_len),
		WrittenLen:        uint64(raw.written_len),
	}
	for i := range result.ECSAddress {
		result.ECSAddress[i] = byte(raw.ecs_address[i])
	}
	if result.Status != abiStatus(uint32(callStatus)) {
		return result, statusError("inspect result status", abiStatus(uint32(callStatus)))
	}
	return result, nil
}
