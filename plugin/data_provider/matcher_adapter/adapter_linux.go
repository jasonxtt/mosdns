//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package matcher_adapter

/*
#cgo CFLAGS: -I${SRCDIR}/../../../rust/runtime/include
#cgo LDFLAGS: -L${SRCDIR}/../../../rust/target/release -lmosdns_runtime -ldl -lm -lpthread
#include "mosdns_cache_core.h"
*/
import "C"

import (
	"encoding/binary"
	"fmt"
	"os"
	"runtime"
	"strings"
	"sync/atomic"
	"unicode/utf8"
	"unsafe"
)

const requiredMatcherCapabilities = uint64(C.MOSDNS_CACHE_CAPABILITY_MATCHER)

type matcherKind uint8

const (
	domainKind matcherKind = iota + 1
	ipKind
	valuedKind
)

type snapshot struct {
	kind    matcherKind
	handle  atomic.Uint64
	tripped atomic.Bool
}

func BuildDomainSnapshot(rules []string) (DomainSnapshot, error) {
	if !rustRequested() {
		return nil, nil
	}
	if !RustDomainRulesSupported(rules) {
		return nil, fmt.Errorf("rust domain matcher requires ASCII rules")
	}
	return buildSnapshot(domainKind, strings.Join(rules, "\n"))
}

func BuildIPSnapshot(prefixes []string) (IPSnapshot, error) {
	if !rustRequested() {
		return nil, nil
	}
	return buildSnapshot(ipKind, strings.Join(prefixes, "\n"))
}

func BuildValuedDomainSnapshot(rules []ValuedRule) (ValuedSnapshot, error) {
	if !rustRequested() {
		return nil, nil
	}
	if !RustValuedRulesSupported(rules) {
		return nil, fmt.Errorf("rust valued domain matcher requires ASCII rules")
	}
	return buildValuedSnapshot(rules)
}

func rustRequested() bool {
	return strings.EqualFold(strings.TrimSpace(os.Getenv(BackendEnv)), "rust")
}

func buildSnapshot(kind matcherKind, text string) (*snapshot, error) {
	version := uint32(C.cache_abi_version())
	if version != uint32(C.MOSDNS_CACHE_ABI_VERSION) {
		return nil, fmt.Errorf("rust matcher ABI version %d, want %d", version, uint32(C.MOSDNS_CACHE_ABI_VERSION))
	}
	capabilities := uint64(C.cache_abi_capabilities())
	if capabilities&requiredMatcherCapabilities != requiredMatcherCapabilities {
		return nil, fmt.Errorf("rust matcher capability not available")
	}

	data := []byte(text)
	var handle C.uint64_t
	var status C.MosdnsCacheStatus
	switch kind {
	case domainKind:
		status = C.domain_matcher_create(borrowedSlice(data), 0, &handle)
	case ipKind:
		status = C.ip_matcher_create(borrowedSlice(data), &handle)
	default:
		return nil, fmt.Errorf("unknown rust matcher kind %d", kind)
	}
	runtime.KeepAlive(data)
	if status != C.MOSDNS_CACHE_OK {
		return nil, statusError(kind, "create", status)
	}

	matcher := &snapshot{kind: kind}
	matcher.handle.Store(uint64(handle))
	return matcher, nil
}

type valuedSnapshot struct {
	handle  atomic.Uint64
	tripped atomic.Bool
}

func buildValuedSnapshot(rules []ValuedRule) (*valuedSnapshot, error) {
	version := uint32(C.cache_abi_version())
	if version != uint32(C.MOSDNS_CACHE_ABI_VERSION) {
		return nil, fmt.Errorf("rust matcher ABI version %d, want %d", version, uint32(C.MOSDNS_CACHE_ABI_VERSION))
	}
	capabilities := uint64(C.cache_abi_capabilities())
	required := uint64(C.MOSDNS_CACHE_CAPABILITY_VALUED_MATCHER)
	if capabilities&required != required {
		return nil, fmt.Errorf("rust valued matcher capability not available")
	}

	data, err := encodeValuedRules(rules)
	if err != nil {
		return nil, err
	}
	var handle C.uint64_t
	status := C.valued_domain_matcher_create(borrowedSlice(data), &handle)
	runtime.KeepAlive(data)
	if status != C.MOSDNS_CACHE_OK {
		return nil, statusError(valuedKind, "create", status)
	}

	snapshot := &valuedSnapshot{}
	snapshot.handle.Store(uint64(handle))
	return snapshot, nil
}

func (s *snapshot) Match(value string) (bool, error) {
	if s.tripped.Load() {
		return false, circuitBrokenError(s.kind)
	}
	handle := s.handle.Load()
	if handle == 0 {
		return false, statusError(s.kind, "match", C.MOSDNS_CACHE_CLOSED)
	}

	data := []byte(value)
	var matched C.bool
	var status C.MosdnsCacheStatus
	switch s.kind {
	case domainKind:
		status = C.domain_matcher_match(C.uint64_t(handle), borrowedSlice(data), &matched)
	case ipKind:
		status = C.ip_matcher_match(C.uint64_t(handle), borrowedSlice(data), &matched)
	default:
		return false, fmt.Errorf("unknown rust matcher kind %d", s.kind)
	}
	runtime.KeepAlive(data)
	if status != C.MOSDNS_CACHE_OK {
		return false, s.classifyRuntimeFailure("match", status)
	}
	return bool(matched), nil
}

func (s *snapshot) Len() (uint64, error) {
	if s.tripped.Load() {
		return 0, circuitBrokenError(s.kind)
	}
	handle := s.handle.Load()
	if handle == 0 {
		return 0, statusError(s.kind, "len", C.MOSDNS_CACHE_CLOSED)
	}

	var length C.uint64_t
	var status C.MosdnsCacheStatus
	switch s.kind {
	case domainKind:
		status = C.domain_matcher_len(C.uint64_t(handle), &length)
	case ipKind:
		status = C.ip_matcher_len(C.uint64_t(handle), &length)
	default:
		return 0, fmt.Errorf("unknown rust matcher kind %d", s.kind)
	}
	if status != C.MOSDNS_CACHE_OK {
		return 0, s.classifyRuntimeFailure("len", status)
	}
	return uint64(length), nil
}

// Close releases the Rust handle. Repeated and concurrent calls are harmless.
func (s *snapshot) Close() error {
	handle := s.handle.Swap(0)
	if handle == 0 {
		return nil
	}

	var status C.MosdnsCacheStatus
	switch s.kind {
	case domainKind:
		status = C.domain_matcher_close(C.uint64_t(handle))
	case ipKind:
		status = C.ip_matcher_close(C.uint64_t(handle))
	default:
		return fmt.Errorf("unknown rust matcher kind %d", s.kind)
	}
	if status != C.MOSDNS_CACHE_OK && status != C.MOSDNS_CACHE_CLOSED {
		return statusError(s.kind, "close", status)
	}
	return nil
}

func (s *snapshot) classifyRuntimeFailure(operation string, status C.MosdnsCacheStatus) error {
	err := statusError(s.kind, operation, status)
	if IsCircuitBreakerError(err) {
		s.tripped.Store(true)
		_ = s.Close()
	}
	return err
}

func (s *valuedSnapshot) Match(value string) (ValuedResult, error) {
	if s.tripped.Load() {
		return ValuedResult{}, circuitBrokenError(valuedKind)
	}
	handle := s.handle.Load()
	if handle == 0 {
		return ValuedResult{}, statusError(valuedKind, "match", C.MOSDNS_CACHE_CLOSED)
	}

	data := []byte(value)
	var raw C.MosdnsValuedMatchResult
	status := C.valued_domain_matcher_match(
		C.uint64_t(handle),
		borrowedSlice(data),
		writableSlice(nil),
		&raw,
	)
	runtime.KeepAlive(data)
	if status == C.MOSDNS_CACHE_OK {
		if raw.status != C.MOSDNS_CACHE_OK {
			return ValuedResult{}, s.malformedResult("match status")
		}
		if raw.matched == 0 {
			return ValuedResult{}, nil
		}
		if raw.matched != 1 {
			return ValuedResult{}, s.malformedResult("match flag")
		}
		return ValuedResult{}, s.classifyRuntimeFailure("match result", C.MOSDNS_CACHE_INTERNAL)
	}
	if status != C.MOSDNS_CACHE_BUFFER_TOO_SMALL {
		return ValuedResult{}, s.classifyRuntimeFailure("match", status)
	}
	if raw.status != C.MOSDNS_CACHE_BUFFER_TOO_SMALL {
		return ValuedResult{}, s.malformedResult("match buffer status")
	}
	if raw.matched != 1 || raw.required_len == 0 || uint64(raw.required_len) > uint64(^uint(0)>>1) {
		return ValuedResult{}, s.malformedResult("match size")
	}

	output := make([]byte, int(raw.required_len))
	var complete C.MosdnsValuedMatchResult
	status = C.valued_domain_matcher_match(
		C.uint64_t(handle),
		borrowedSlice(data),
		writableSlice(output),
		&complete,
	)
	runtime.KeepAlive(data)
	runtime.KeepAlive(output)
	if status != C.MOSDNS_CACHE_OK {
		return ValuedResult{}, s.classifyRuntimeFailure("match", status)
	}
	if complete.status != C.MOSDNS_CACHE_OK {
		return ValuedResult{}, s.malformedResult("match complete status")
	}
	if complete.matched != 1 || uint64(complete.required_len) != uint64(len(output)) {
		return ValuedResult{}, s.malformedResult("match result size")
	}
	result, err := decodeValuedResult(output)
	if err != nil {
		return ValuedResult{}, s.malformedResult(err.Error())
	}
	result.Matched = true
	return result, nil
}

func (s *valuedSnapshot) Len() (uint64, error) {
	if s.tripped.Load() {
		return 0, circuitBrokenError(valuedKind)
	}
	handle := s.handle.Load()
	if handle == 0 {
		return 0, statusError(valuedKind, "len", C.MOSDNS_CACHE_CLOSED)
	}
	var length C.uint64_t
	status := C.valued_domain_matcher_len(C.uint64_t(handle), &length)
	if status != C.MOSDNS_CACHE_OK {
		return 0, s.classifyRuntimeFailure("len", status)
	}
	return uint64(length), nil
}

func (s *valuedSnapshot) Close() error {
	handle := s.handle.Swap(0)
	if handle == 0 {
		return nil
	}
	status := C.valued_domain_matcher_close(C.uint64_t(handle))
	if status != C.MOSDNS_CACHE_OK && status != C.MOSDNS_CACHE_CLOSED {
		return statusError(valuedKind, "close", status)
	}
	return nil
}

func (s *valuedSnapshot) classifyRuntimeFailure(operation string, status C.MosdnsCacheStatus) error {
	err := statusError(valuedKind, operation, status)
	if IsCircuitBreakerError(err) {
		s.tripped.Store(true)
		_ = s.Close()
	}
	return err
}

func (s *valuedSnapshot) malformedResult(detail string) error {
	err := &Error{
		Operation: fmt.Sprintf("rust %s decode: %s", matcherLabel(valuedKind), detail),
		Code:      uint32(C.MOSDNS_CACHE_INTERNAL),
		Class:     ErrorClassRuntime,
	}
	s.tripped.Store(true)
	_ = s.Close()
	return err
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

func statusError(kind matcherKind, operation string, status C.MosdnsCacheStatus) error {
	class := ErrorClassRuntime
	switch status {
	case C.MOSDNS_CACHE_INVALID_ARGUMENT:
		class = ErrorClassInvalidArgument
	case C.MOSDNS_CACHE_CLOSED:
		class = ErrorClassClosed
	}
	return &Error{Operation: fmt.Sprintf("rust %s %s", matcherLabel(kind), operation), Code: uint32(status), Class: class}
}

func circuitBrokenError(kind matcherKind) error {
	return &Error{Operation: fmt.Sprintf("rust %s circuit breaker", matcherLabel(kind)), Code: uint32(C.MOSDNS_CACHE_INTERNAL), Class: ErrorClassCircuitBroken}
}

func matcherLabel(kind matcherKind) string {
	switch kind {
	case domainKind:
		return "domain matcher"
	case ipKind:
		return "IP matcher"
	case valuedKind:
		return "valued domain matcher"
	default:
		return "matcher"
	}
}

func encodeValuedRules(rules []ValuedRule) ([]byte, error) {
	if uint64(len(rules)) > uint64(^uint32(0)) {
		return nil, fmt.Errorf("too many valued rules: %d", len(rules))
	}
	out := make([]byte, 0)
	out = append(out, byte(C.MOSDNS_VALUED_RULE_BATCH_VERSION))
	out = appendUint32(out, uint32(len(rules)))
	for _, rule := range rules {
		var err error
		out, err = appendValuedString(out, rule.Rule)
		if err != nil {
			return nil, err
		}
		out = appendUint64(out, rule.FastMarks)
		if uint64(len(rule.CtxMarks)) > uint64(^uint32(0)) {
			return nil, fmt.Errorf("too many context marks for valued rule %q", rule.Rule)
		}
		out = appendUint32(out, uint32(len(rule.CtxMarks)))
		for _, mark := range rule.CtxMarks {
			out = appendUint32(out, mark)
		}
		out, err = appendValuedString(out, rule.JoinedTags)
		if err != nil {
			return nil, err
		}
		out, err = appendValuedString(out, rule.JoinedSources)
		if err != nil {
			return nil, err
		}
	}
	return out, nil
}

func appendValuedString(out []byte, value string) ([]byte, error) {
	if uint64(len(value)) > uint64(^uint32(0)) {
		return nil, fmt.Errorf("valued string is too long: %d", len(value))
	}
	out = appendUint32(out, uint32(len(value)))
	return append(out, value...), nil
}

func appendUint32(out []byte, value uint32) []byte {
	var encoded [4]byte
	binary.LittleEndian.PutUint32(encoded[:], value)
	return append(out, encoded[:]...)
}

func appendUint64(out []byte, value uint64) []byte {
	var encoded [8]byte
	binary.LittleEndian.PutUint64(encoded[:], value)
	return append(out, encoded[:]...)
}

func decodeValuedResult(data []byte) (ValuedResult, error) {
	if len(data) == 0 || data[0] != byte(C.MOSDNS_VALUED_RESULT_VERSION) {
		return ValuedResult{}, fmt.Errorf("invalid valued result version")
	}
	cursor := 1
	fastMask, err := readValuedUint64(data, &cursor)
	if err != nil {
		return ValuedResult{}, err
	}
	if fastMask&(uint64(1)<<63) != 0 {
		return ValuedResult{}, fmt.Errorf("valued result uses unsupported fast mark 64")
	}
	ctxCount, err := readValuedUint32(data, &cursor)
	if err != nil {
		return ValuedResult{}, err
	}
	if cursor < 0 || cursor > len(data) || uint64(ctxCount) > uint64(len(data)-cursor)/4 {
		return ValuedResult{}, fmt.Errorf("valued result context mark count is truncated")
	}
	ctxMarks := make([]uint32, 0, ctxCount)
	for i := uint32(0); i < ctxCount; i++ {
		mark, err := readValuedUint32(data, &cursor)
		if err != nil {
			return ValuedResult{}, err
		}
		if mark == 0 || (len(ctxMarks) > 0 && mark <= ctxMarks[len(ctxMarks)-1]) {
			return ValuedResult{}, fmt.Errorf("valued result context marks are not sorted")
		}
		ctxMarks = append(ctxMarks, mark)
	}
	tags, err := readValuedString(data, &cursor)
	if err != nil {
		return ValuedResult{}, err
	}
	sources, err := readValuedString(data, &cursor)
	if err != nil {
		return ValuedResult{}, err
	}
	if cursor != len(data) {
		return ValuedResult{}, fmt.Errorf("valued result has trailing bytes")
	}
	fastMarks := make([]uint8, 0, 63)
	for i := uint8(0); i < 63; i++ {
		if fastMask&(uint64(1)<<i) != 0 {
			fastMarks = append(fastMarks, i+1)
		}
	}
	return ValuedResult{
		Matched:       true,
		FastMarks:     fastMarks,
		CtxMarks:      ctxMarks,
		JoinedTags:    tags,
		JoinedSources: sources,
	}, nil
}

func readValuedUint32(data []byte, cursor *int) (uint32, error) {
	if *cursor < 0 || len(data)-*cursor < 4 {
		return 0, fmt.Errorf("valued result is truncated")
	}
	value := binary.LittleEndian.Uint32(data[*cursor : *cursor+4])
	*cursor += 4
	return value, nil
}

func readValuedUint64(data []byte, cursor *int) (uint64, error) {
	if *cursor < 0 || len(data)-*cursor < 8 {
		return 0, fmt.Errorf("valued result is truncated")
	}
	value := binary.LittleEndian.Uint64(data[*cursor : *cursor+8])
	*cursor += 8
	return value, nil
}

func readValuedString(data []byte, cursor *int) (string, error) {
	length, err := readValuedUint32(data, cursor)
	if err != nil {
		return "", err
	}
	if uint64(length) > uint64(len(data)-*cursor) {
		return "", fmt.Errorf("valued result string is truncated")
	}
	end := *cursor + int(length)
	value := data[*cursor:end]
	*cursor = end
	if !utf8.Valid(value) {
		return "", fmt.Errorf("valued result string is not UTF-8")
	}
	return string(value), nil
}
