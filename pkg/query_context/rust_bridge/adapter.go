package rust_bridge

import (
	"bytes"
	"errors"
	"fmt"
	"math"
	"os"
	"strings"

	"github.com/miekg/dns"
)

const queryBackendEnv = "MOSDNS_QUERY_BACKEND"

const (
	queryABIVersion    uint32 = 1
	queryResultVersion uint32 = 1

	queryCapabilitySnapshot   uint64 = 1 << 5
	queryCapabilityInspect    uint64 = 1 << 6
	requiredQueryCapabilities        = queryCapabilitySnapshot | queryCapabilityInspect
)

// Backend identifies the implementation that produced a result.
type Backend string

const (
	BackendGo   Backend = "go"
	BackendRust Backend = "rust"
)

// TransportMode is the request transport discriminator frozen by the query
// ABI. HTTP includes DNS-over-HTTP, which has no TCP length prefix.
type TransportMode uint8

const (
	TransportUDP TransportMode = iota
	TransportStream
	TransportHTTP
)

// QueryRequest is the smallest input seam for the experimental query path.
// QueryWire remains caller-owned; Inspect never modifies it.
type QueryRequest struct {
	QueryWire         []byte
	FromUDP           bool
	AdvertisedUDPSize uint16
	Transport         TransportMode
	PreFastFlags      uint64
}

// ECSResult is a normalized ECS value. Address is always fixed-width: family
// 1 occupies the first four bytes and family 2 occupies all sixteen bytes.
type ECSResult struct {
	Family        uint16
	SourceNetmask uint8
	SourceScope   uint8
	Address       [16]byte
}

// QueryResult is caller-owned query inspection data. QueryWire and ECS are
// copied on every result and never alias the request, an ABI buffer, or a
// previous result.
type QueryResult struct {
	Backend           Backend
	QueryWire         []byte
	Flags             uint32
	ID                uint16
	QType             uint16
	QClass            uint16
	QNameWireLen      uint64
	AdvertisedUDPSize uint16
	EDNSUDPSize       uint16
	HasOPT            bool
	DO                bool
	ECS               *ECSResult
	FromUDP           bool
	Transport         TransportMode
	PreFastFlags      uint64
	RequiredLen       uint64
	WrittenLen        uint64
}

// ErrInvalidQueryRequest reports a request-level field outside the frozen
// transport contract. DNS wire errors remain ErrMalformedQuery or
// ErrUnsupportedQuery from the Go oracle.
var ErrInvalidQueryRequest = errors.New("invalid query request")

// FallbackReason identifies why a Rust result was not used.
type FallbackReason string

const (
	FallbackUnavailable    FallbackReason = "unavailable"
	FallbackABIMismatch    FallbackReason = "abi-mismatch"
	FallbackRuntime        FallbackReason = "runtime"
	FallbackPanic          FallbackReason = "panic"
	FallbackInvalidResult  FallbackReason = "invalid-result"
	FallbackResultMismatch FallbackReason = "result-mismatch"
)

// FallbackError preserves the Go oracle result while making an attempted Rust
// failure observable to callers. Unwrap keeps the underlying ABI/runtime
// cause available to errors.Is/errors.As.
type FallbackError struct {
	Reason FallbackReason
	Cause  error
}

func (e *FallbackError) Error() string {
	if e == nil {
		return "<nil>"
	}
	if e.Cause == nil {
		return fmt.Sprintf("rust query fallback (%s)", e.Reason)
	}
	return fmt.Sprintf("rust query fallback (%s): %v", e.Reason, e.Cause)
}

func (e *FallbackError) Unwrap() error { return e.Cause }

// IsFallbackError reports whether Rust was attempted and the Go oracle was
// returned instead.
func IsFallbackError(err error) bool {
	var fallback *FallbackError
	return errors.As(err, &fallback)
}

// GoOracle exposes the authoritative non-Rust query result for parity tests
// and later slices. It does not select or load any Rust implementation.
func GoOracle(request QueryRequest) (QueryResult, error) {
	return goOracle(request)
}

// Inspect returns a query result from the Go oracle by default. Rust is
// attempted only when MOSDNS_QUERY_BACKEND is exactly the explicit rust value
// (case and surrounding whitespace are ignored), and every Rust failure
// returns the same-generation Go result together with a FallbackError.
func Inspect(request QueryRequest) (result QueryResult, err error) {
	oracle, err := goOracle(request)
	if err != nil {
		return QueryResult{}, err
	}
	if !rustRequested() {
		return oracle, nil
	}

	native, err := inspectRust(request)
	if err != nil {
		return oracle, &FallbackError{Reason: fallbackReason(err), Cause: err}
	}
	if err := compareQueryResults(native, oracle); err != nil {
		return oracle, &FallbackError{Reason: FallbackResultMismatch, Cause: err}
	}
	native.Backend = BackendRust
	return native, nil
}

func rustRequested() bool {
	return strings.EqualFold(strings.TrimSpace(os.Getenv(queryBackendEnv)), string(BackendRust))
}

func goOracle(request QueryRequest) (QueryResult, error) {
	if request.Transport > TransportHTTP {
		return QueryResult{}, fmt.Errorf("%w: transport mode %d", ErrInvalidQueryRequest, request.Transport)
	}

	snapshot, err := NewSnapshot(request.QueryWire)
	if err != nil {
		return QueryResult{}, err
	}
	wire := snapshot.Wire()
	_, nameEnd, err := dns.UnpackDomainName(wire, 12)
	if err != nil {
		return QueryResult{}, fmt.Errorf("%w: question name: %v", ErrMalformedQuery, err)
	}
	if nameEnd < 12 || nameEnd+4 > len(wire) {
		return QueryResult{}, fmt.Errorf("%w: question type/class truncated", ErrMalformedQuery)
	}

	edns := snapshot.EDNS()
	result := QueryResult{
		Backend:           BackendGo,
		QueryWire:         wire,
		ID:                uint16(wire[0])<<8 | uint16(wire[1]),
		QType:             uint16(wire[nameEnd])<<8 | uint16(wire[nameEnd+1]),
		QClass:            uint16(wire[nameEnd+2])<<8 | uint16(wire[nameEnd+3]),
		QNameWireLen:      uint64(nameEnd - 12),
		AdvertisedUDPSize: request.AdvertisedUDPSize,
		FromUDP:           request.FromUDP,
		Transport:         request.Transport,
		PreFastFlags:      request.PreFastFlags,
		RequiredLen:       uint64(len(wire)),
		WrittenLen:        uint64(len(wire)),
		HasOPT:            edns.HasOPT,
		EDNSUDPSize:       edns.UDPSize,
		DO:                edns.DO,
	}
	if edns.ECS != nil {
		ecs := &ECSResult{
			Family:        edns.ECS.Family,
			SourceNetmask: edns.ECS.SourceNetmask,
			SourceScope:   edns.ECS.SourceScope,
		}
		copyNormalizedAddress(&ecs.Address, edns.ECS.Family, edns.ECS.Address)
		result.ECS = ecs
	}
	return result, nil
}

func copyNormalizedAddress(dst *[16]byte, family uint16, src []byte) {
	width := len(dst)
	if family == 1 {
		width = 4
	}
	if len(src) < width {
		width = len(src)
	}
	copy(dst[:width], src[:width])
}

func compareQueryResults(native, oracle QueryResult) error {
	if !bytes.Equal(native.QueryWire, oracle.QueryWire) {
		return errors.New("query wire differs from Go oracle")
	}
	if native.Flags != oracle.Flags || native.ID != oracle.ID || native.QType != oracle.QType ||
		native.QClass != oracle.QClass || native.QNameWireLen != oracle.QNameWireLen ||
		native.AdvertisedUDPSize != oracle.AdvertisedUDPSize || native.EDNSUDPSize != oracle.EDNSUDPSize ||
		native.HasOPT != oracle.HasOPT || native.DO != oracle.DO || native.FromUDP != oracle.FromUDP ||
		native.Transport != oracle.Transport || native.PreFastFlags != oracle.PreFastFlags ||
		native.RequiredLen != oracle.RequiredLen || native.WrittenLen != oracle.WrittenLen {
		return errors.New("query metadata differs from Go oracle")
	}
	if (native.ECS == nil) != (oracle.ECS == nil) {
		return errors.New("ECS presence differs from Go oracle")
	}
	if native.ECS != nil && *native.ECS != *oracle.ECS {
		return errors.New("ECS metadata differs from Go oracle")
	}
	return nil
}

func equalQueryResults(a, b QueryResult) bool {
	return compareQueryResults(a, b) == nil
}

type abiStatus uint32

const (
	abiStatusOK abiStatus = iota
	abiStatusInvalidArgument
	abiStatusClosed
	abiStatusPanic
	abiStatusInternal
	abiStatusBufferTooSmall
)

type queryABIInput struct {
	Version           uint32
	Flags             uint32
	QueryWire         []byte
	FromUDP           bool
	Transport         TransportMode
	AdvertisedUDPSize uint16
	PreFastFlags      uint64
}

type queryABIResult struct {
	Status            abiStatus
	Version           uint32
	Flags             uint32
	ID                uint16
	QType             uint16
	QClass            uint16
	AdvertisedUDPSize uint16
	EDNSUDPSize       uint16
	ECSFamily         uint16
	ECSSourceNetmask  uint8
	ECSSourceScope    uint8
	FromUDP           uint8
	Transport         TransportMode
	HasOPT            uint8
	DOBit             uint8
	ECS               uint8
	Reserved          uint8
	QNameLen          uint64
	PreFastFlags      uint64
	RequiredLen       uint64
	WrittenLen        uint64
	ECSAddress        [16]byte
}

type queryABI interface {
	Version() uint32
	Capabilities() uint64
	Create(queryABIInput) (uint64, error)
	RequiredLen(uint64) (uint64, error)
	Inspect(uint64, []byte) (queryABIResult, error)
	Close(uint64) error
}

var nativeABIFactory = newNativeABI

func inspectRust(request QueryRequest) (result QueryResult, err error) {
	defer func() {
		if recovered := recover(); recovered != nil {
			err = fmt.Errorf("rust query adapter panic: %v", recovered)
			result = QueryResult{}
		}
	}()

	abi, err := nativeABIFactory()
	if err != nil {
		return QueryResult{}, err
	}
	if abi == nil {
		return QueryResult{}, errors.New("rust query ABI is unavailable")
	}
	if version := abi.Version(); version != queryABIVersion {
		return QueryResult{}, fmt.Errorf("rust query ABI version %d, want %d", version, queryABIVersion)
	}
	if capabilities := abi.Capabilities(); capabilities&requiredQueryCapabilities != requiredQueryCapabilities {
		return QueryResult{}, fmt.Errorf("rust query capabilities %#x do not include %#x", capabilities, requiredQueryCapabilities)
	}

	wire := append([]byte(nil), request.QueryWire...)
	handle, err := abi.Create(queryABIInput{
		Version:           queryABIVersion,
		QueryWire:         wire,
		FromUDP:           request.FromUDP,
		Transport:         request.Transport,
		AdvertisedUDPSize: request.AdvertisedUDPSize,
		PreFastFlags:      request.PreFastFlags,
	})
	if err != nil {
		return QueryResult{}, err
	}
	if handle == 0 {
		return QueryResult{}, errors.New("rust query ABI returned an empty handle")
	}
	defer func() {
		closeErr := abi.Close(handle)
		if closeErr != nil {
			if err == nil {
				err = closeErr
				result = QueryResult{}
			} else {
				err = errors.Join(err, closeErr)
			}
		}
	}()

	required, err := abi.RequiredLen(handle)
	if err != nil {
		return QueryResult{}, err
	}
	outputLen, err := checkedOutputLength(required, len(wire))
	if err != nil {
		return QueryResult{}, err
	}
	output := make([]byte, outputLen)
	for attempt := 0; attempt < 2; attempt++ {
		raw, callErr := abi.Inspect(handle, output)
		if callErr != nil {
			return QueryResult{}, callErr
		}
		if raw.Status == abiStatusBufferTooSmall && attempt == 0 {
			if err := validateRetryResult(raw); err != nil {
				return QueryResult{}, err
			}
			nextLen, lengthErr := checkedOutputLength(raw.RequiredLen, len(wire))
			if lengthErr != nil {
				return QueryResult{}, lengthErr
			}
			if nextLen <= len(output) {
				return QueryResult{}, errors.New("rust query retry did not increase output length")
			}
			output = make([]byte, nextLen)
			continue
		}
		return decodeQueryResult(raw, output)
	}
	return QueryResult{}, errors.New("rust query inspect retry exhausted")
}

func checkedOutputLength(length uint64, inputLength int) (int, error) {
	if length == 0 || length > uint64(math.MaxInt) {
		return 0, fmt.Errorf("rust query output length %d is not a safe Go int", length)
	}
	if length > uint64(inputLength) {
		return 0, fmt.Errorf("rust query output length %d exceeds input length %d", length, inputLength)
	}
	return int(length), nil
}

func decodeQueryResult(raw queryABIResult, output []byte) (QueryResult, error) {
	if raw.Status != abiStatusOK {
		return QueryResult{}, statusError("inspect", raw.Status)
	}
	if raw.Version != queryResultVersion {
		return QueryResult{}, fmt.Errorf("rust query result version %d, want %d", raw.Version, queryResultVersion)
	}
	if raw.Flags != 0 || raw.Reserved != 0 {
		return QueryResult{}, errors.New("rust query result has unsupported flags or reserved fields")
	}
	if !validByte(raw.FromUDP) || raw.Transport > TransportHTTP ||
		!validByte(raw.HasOPT) || !validByte(raw.DOBit) || !validByte(raw.ECS) {
		return QueryResult{}, errors.New("rust query result has invalid boolean or transport fields")
	}
	if raw.RequiredLen != uint64(len(output)) || raw.WrittenLen != raw.RequiredLen {
		return QueryResult{}, errors.New("rust query result has invalid required/written lengths")
	}
	if raw.QNameLen > raw.RequiredLen {
		return QueryResult{}, errors.New("rust query result has an invalid qname length")
	}
	if raw.HasOPT == 0 && (raw.EDNSUDPSize != 0 || raw.DOBit != 0 || raw.ECS != 0 || raw.ECSFamily != 0 || raw.ECSSourceNetmask != 0 || raw.ECSSourceScope != 0 || raw.ECSAddress != [16]byte{}) {
		return QueryResult{}, errors.New("rust query result has EDNS fields without OPT")
	}
	if raw.ECS == 0 && (raw.ECSFamily != 0 || raw.ECSSourceNetmask != 0 || raw.ECSSourceScope != 0 || raw.ECSAddress != [16]byte{}) {
		return QueryResult{}, errors.New("rust query result has ECS fields without ECS")
	}

	result := QueryResult{
		Backend:           BackendRust,
		QueryWire:         append([]byte(nil), output[:int(raw.WrittenLen)]...),
		Flags:             raw.Flags,
		ID:                raw.ID,
		QType:             raw.QType,
		QClass:            raw.QClass,
		QNameWireLen:      raw.QNameLen,
		AdvertisedUDPSize: raw.AdvertisedUDPSize,
		EDNSUDPSize:       raw.EDNSUDPSize,
		HasOPT:            raw.HasOPT != 0,
		DO:                raw.DOBit != 0,
		FromUDP:           raw.FromUDP != 0,
		Transport:         raw.Transport,
		PreFastFlags:      raw.PreFastFlags,
		RequiredLen:       raw.RequiredLen,
		WrittenLen:        raw.WrittenLen,
	}
	if raw.ECS != 0 {
		result.ECS = &ECSResult{
			Family:        raw.ECSFamily,
			SourceNetmask: raw.ECSSourceNetmask,
			SourceScope:   raw.ECSSourceScope,
			Address:       raw.ECSAddress,
		}
	}
	return result, nil
}

func validateRetryResult(raw queryABIResult) error {
	if raw.Version != queryResultVersion {
		return fmt.Errorf("rust query result version %d, want %d", raw.Version, queryResultVersion)
	}
	if raw.Flags != 0 || raw.Reserved != 0 {
		return errors.New("rust query retry result has unsupported flags or reserved fields")
	}
	if !validByte(raw.FromUDP) || raw.Transport > TransportHTTP ||
		!validByte(raw.HasOPT) || !validByte(raw.DOBit) || !validByte(raw.ECS) {
		return errors.New("rust query retry result has invalid boolean or transport fields")
	}
	if raw.WrittenLen != 0 || raw.RequiredLen == 0 || raw.QNameLen > raw.RequiredLen {
		return errors.New("rust query retry result has invalid required/written lengths")
	}
	return nil
}

func validByte(value uint8) bool { return value <= 1 }

func boolByte(value bool) uint8 {
	if value {
		return 1
	}
	return 0
}

// abiStatusError reports a non-OK status returned by the Rust query ABI. It
// carries the operation name and the exact ABI status so that fallback
// classification never depends on error text.
type abiStatusError struct {
	Operation string
	Status    abiStatus
}

func (e *abiStatusError) Error() string {
	return fmt.Sprintf("rust query %s failed with status %d", e.Operation, uint32(e.Status))
}

func statusError(operation string, status abiStatus) error {
	return &abiStatusError{Operation: operation, Status: status}
}

// fallbackReason classifies a Rust-path failure for the typed FallbackError.
// ABI status mapping is structural: statusError returns abiStatusError and the
// exact abiStatusPanic value maps to FallbackPanic. Text heuristics cover only
// adapter-level errors that carry no ABI status (construction, version checks,
// buffer sizing, and the Go-side panic recovery path).
func fallbackReason(err error) FallbackReason {
	if err == nil {
		return FallbackRuntime
	}
	var abiErr *abiStatusError
	if errors.As(err, &abiErr) {
		switch abiErr.Status {
		case abiStatusPanic:
			return FallbackPanic
		case abiStatusBufferTooSmall:
			return FallbackInvalidResult
		default:
			return FallbackRuntime
		}
	}
	message := strings.ToLower(err.Error())
	switch {
	case strings.Contains(message, "panic"):
		return FallbackPanic
	case strings.Contains(message, "version"), strings.Contains(message, "capabilit"):
		return FallbackABIMismatch
	case strings.Contains(message, "unavailable"):
		return FallbackUnavailable
	case strings.Contains(message, "result"), strings.Contains(message, "output length"):
		return FallbackInvalidResult
	default:
		return FallbackRuntime
	}
}
