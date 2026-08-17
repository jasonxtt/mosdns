package rust_bridge

import (
	"bytes"
	"errors"
	"fmt"
	"strings"
	"testing"
)

func adapterValidQuery() []byte {
	return []byte{
		0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
		0x07, 'e', 'x', 'a', 'm', 'p', 'l', 'e', 0x03, 'o', 'r', 'g', 0x00,
		0x00, 0x01, 0x00, 0x01,
	}
}

func adapterQueryWithECS(family uint16, mask, scope uint8, address ...byte) []byte {
	option := []byte{0x00, 0x08, 0x00, byte(4 + len(address)), byte(family >> 8), byte(family), mask, scope}
	option = append(option, address...)
	opt := []byte{0x00, 0x00, 0x29, 0x04, 0xd0, 0x00, 0x00, 0x00, 0x00, byte(len(option) >> 8), byte(len(option))}
	opt = append(opt, option...)
	wire := append([]byte(nil), adapterValidQuery()...)
	wire[11] = 1
	return append(wire, opt...)
}

func adapterRequest(wire []byte) QueryRequest {
	return QueryRequest{
		QueryWire:         wire,
		FromUDP:           true,
		AdvertisedUDPSize: 1232,
		Transport:         TransportUDP,
		PreFastFlags:      0x1234,
	}
}

type fakeQueryABI struct {
	version      uint32
	capabilities uint64
	createErr    error
	createStatus abiStatus
	input        queryABIInput
	wire         []byte
	required     uint64
	inspect      []queryABIResult
	closeStatus  abiStatus
	closed       bool
	inspectCalls int
}

func (f *fakeQueryABI) Version() uint32 { return f.version }

func (f *fakeQueryABI) Capabilities() uint64 { return f.capabilities }

func (f *fakeQueryABI) Create(input queryABIInput) (uint64, error) {
	f.input = input
	if f.createErr != nil {
		return 0, f.createErr
	}
	if f.createStatus != abiStatusOK {
		return 0, statusError("create", f.createStatus)
	}
	return 0x4000000000000001, nil
}

func (f *fakeQueryABI) RequiredLen(uint64) (uint64, error) { return f.required, nil }

func (f *fakeQueryABI) Inspect(_ uint64, output []byte) (queryABIResult, error) {
	if f.inspectCalls >= len(f.inspect) {
		return queryABIResult{}, errors.New("fake inspect exhausted")
	}
	result := f.inspect[f.inspectCalls]
	f.inspectCalls++
	if result.Status == abiStatusOK {
		copy(output, f.wire)
	}
	return result, nil
}

func (f *fakeQueryABI) Close(uint64) error {
	f.closed = true
	if f.closeStatus != abiStatusOK && f.closeStatus != abiStatusClosed {
		return statusError("close", f.closeStatus)
	}
	return nil
}

func fakeRustFactory(fake *fakeQueryABI) func() (queryABI, error) {
	return func() (queryABI, error) { return fake, nil }
}

func rawFromResult(result QueryResult, status abiStatus) queryABIResult {
	raw := queryABIResult{
		Status:            status,
		Version:           queryResultVersion,
		ID:                result.ID,
		QType:             result.QType,
		QClass:            result.QClass,
		AdvertisedUDPSize: result.AdvertisedUDPSize,
		EDNSUDPSize:       result.EDNSUDPSize,
		FromUDP:           boolByte(result.FromUDP),
		Transport:         result.Transport,
		HasOPT:            boolByte(result.HasOPT),
		DOBit:             boolByte(result.DO),
		ECS:               boolByte(result.ECS != nil),
		QNameLen:          result.QNameWireLen,
		PreFastFlags:      result.PreFastFlags,
		RequiredLen:       result.RequiredLen,
		WrittenLen:        result.WrittenLen,
	}
	if result.ECS != nil {
		raw.ECSFamily = result.ECS.Family
		raw.ECSSourceNetmask = result.ECS.SourceNetmask
		raw.ECSSourceScope = result.ECS.SourceScope
		raw.ECSAddress = result.ECS.Address
	}
	return raw
}

func installFakeFactory(t *testing.T, factory func() (queryABI, error)) {
	t.Helper()
	old := nativeABIFactory
	nativeABIFactory = factory
	t.Cleanup(func() { nativeABIFactory = old })
}

func TestSelectorUsesGoOracleUnlessRustIsExplicit(t *testing.T) {
	values := []string{"", "go", "unknown", "GO", " rusted "}
	for _, value := range values {
		t.Run(fmt.Sprintf("%q", value), func(t *testing.T) {
			t.Setenv(queryBackendEnv, value)
			called := false
			installFakeFactory(t, func() (queryABI, error) {
				called = true
				return nil, errors.New("rust must not be selected")
			})
			result, err := Inspect(adapterRequest(adapterValidQuery()))
			if err != nil {
				t.Fatalf("Inspect: %v", err)
			}
			if called {
				t.Fatal("non-explicit selector loaded Rust")
			}
			if result.Backend != BackendGo {
				t.Fatalf("backend = %q, want Go", result.Backend)
			}
		})
	}
}

func TestSelectorAttemptsRustOnlyForExplicitRust(t *testing.T) {
	t.Setenv(queryBackendEnv, "rust")
	called := false
	installFakeFactory(t, func() (queryABI, error) {
		called = true
		return nil, errors.New("test rust construction failure")
	})
	result, err := Inspect(adapterRequest(adapterValidQuery()))
	if !called {
		t.Fatal("explicit rust selector did not attempt Rust")
	}
	if !IsFallbackError(err) {
		t.Fatalf("err = %v, want typed fallback error", err)
	}
	if result.Backend != BackendGo || len(result.QueryWire) == 0 {
		t.Fatalf("fallback result lost Go oracle: %+v", result)
	}
}

func TestRustABIPanicStatusMapsToFallbackPanic(t *testing.T) {
	// The Rust FFI boundary converts a caught Rust panic into the numeric
	// Status::Panic ABI status. The adapter must classify that exact status as
	// FallbackPanic structurally, independently of any error text, and still
	// return the Go oracle result.
	request := adapterRequest(adapterValidQuery())
	oracle, err := goOracle(request)
	if err != nil {
		t.Fatalf("goOracle: %v", err)
	}
	cases := []struct {
		name         string
		createStatus abiStatus
		inspect      []queryABIResult
	}{
		{name: "create", createStatus: abiStatusPanic},
		{name: "inspect", inspect: []queryABIResult{{Status: abiStatusPanic, Version: queryResultVersion}}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			fake := &fakeQueryABI{
				version:      queryABIVersion,
				capabilities: requiredQueryCapabilities,
				wire:         append([]byte(nil), request.QueryWire...),
				required:     uint64(len(request.QueryWire)),
				createStatus: tc.createStatus,
				inspect:      tc.inspect,
			}
			installFakeFactory(t, fakeRustFactory(fake))
			t.Setenv(queryBackendEnv, "rust")
			result, err := Inspect(request)
			if !IsFallbackError(err) {
				t.Fatalf("err = %v, want typed fallback error", err)
			}
			var fallback *FallbackError
			if !errors.As(err, &fallback) {
				t.Fatalf("err = %v, want FallbackError", err)
			}
			if fallback.Reason != FallbackPanic {
				t.Fatalf("fallback reason = %q, want %q", fallback.Reason, FallbackPanic)
			}
			if !equalQueryResults(result, oracle) {
				t.Fatalf("ABI panic fallback changed oracle:\nGot=%+v\nWant=%+v", result, oracle)
			}
			if result.Backend != BackendGo {
				t.Fatalf("ABI panic fallback backend = %q, want Go", result.Backend)
			}
		})
	}
}

func TestRustABIRuntimeStatusMapsToFallbackRuntime(t *testing.T) {
	// Non-panic ABI statuses must remain classified as runtime failures; the
	// fallback result is still the Go oracle.
	request := adapterRequest(adapterValidQuery())
	oracle, err := goOracle(request)
	if err != nil {
		t.Fatalf("goOracle: %v", err)
	}
	fake := &fakeQueryABI{
		version:      queryABIVersion,
		capabilities: requiredQueryCapabilities,
		wire:         append([]byte(nil), request.QueryWire...),
		required:     uint64(len(request.QueryWire)),
		inspect:      []queryABIResult{{Status: abiStatusInternal, Version: queryResultVersion}},
	}
	installFakeFactory(t, fakeRustFactory(fake))
	t.Setenv(queryBackendEnv, "rust")
	result, err := Inspect(request)
	if !IsFallbackError(err) {
		t.Fatalf("err = %v, want typed fallback error", err)
	}
	var fallback *FallbackError
	if !errors.As(err, &fallback) {
		t.Fatalf("err = %v, want FallbackError", err)
	}
	if fallback.Reason != FallbackRuntime {
		t.Fatalf("fallback reason = %q, want %q", fallback.Reason, FallbackRuntime)
	}
	if !equalQueryResults(result, oracle) {
		t.Fatalf("ABI runtime fallback changed oracle:\nGot=%+v\nWant=%+v", result, oracle)
	}
}

func TestRustABIUsesRequiredLengthAndRetriesBufferTooSmall(t *testing.T) {
	wire := adapterValidQuery()
	request := adapterRequest(wire)
	oracle, err := goOracle(request)
	if err != nil {
		t.Fatalf("goOracle: %v", err)
	}
	fake := &fakeQueryABI{
		version:      queryABIVersion,
		capabilities: requiredQueryCapabilities,
		wire:         append([]byte(nil), wire...),
		required:     uint64(len(wire) - 1),
	}
	fake.inspect = []queryABIResult{
		{Status: abiStatusBufferTooSmall, Version: queryResultVersion, RequiredLen: uint64(len(wire))},
		rawFromResult(oracle, abiStatusOK),
	}
	installFakeFactory(t, fakeRustFactory(fake))
	t.Setenv(queryBackendEnv, "rust")

	result, err := Inspect(request)
	if err != nil {
		t.Fatalf("Inspect: %v", err)
	}
	if fake.inspectCalls != 2 {
		t.Fatalf("inspect calls = %d, want one retry", fake.inspectCalls)
	}
	if !fake.closed {
		t.Fatal("successful handle was not closed")
	}
	if !equalQueryResults(result, oracle) {
		t.Fatalf("Rust result differs from oracle:\nRust=%+v\nGo=%+v", result, oracle)
	}
}

func TestRustABIRejectsVersionCapabilityAndRuntimeFailuresWithOracle(t *testing.T) {
	cases := []struct {
		name         string
		version      uint32
		capabilities uint64
		inspect      []queryABIResult
		closeStatus  abiStatus
	}{
		{name: "version", version: queryABIVersion + 1, capabilities: requiredQueryCapabilities},
		{name: "capability", version: queryABIVersion, capabilities: queryCapabilitySnapshot},
		{name: "closed-handle", version: queryABIVersion, capabilities: requiredQueryCapabilities, inspect: []queryABIResult{{Status: abiStatusClosed, Version: queryResultVersion}}},
		{name: "runtime-status", version: queryABIVersion, capabilities: requiredQueryCapabilities, inspect: []queryABIResult{{Status: abiStatusInternal, Version: queryResultVersion}}},
		{name: "close-status", version: queryABIVersion, capabilities: requiredQueryCapabilities, closeStatus: abiStatusInternal, inspect: []queryABIResult{{Status: abiStatusOK, Version: queryResultVersion}}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			wire := adapterValidQuery()
			request := adapterRequest(wire)
			oracle, err := goOracle(request)
			if err != nil {
				t.Fatalf("goOracle: %v", err)
			}
			fake := &fakeQueryABI{
				version:      tc.version,
				capabilities: tc.capabilities,
				wire:         append([]byte(nil), wire...),
				required:     uint64(len(wire)),
				inspect:      tc.inspect,
				closeStatus:  tc.closeStatus,
			}
			if fake.closeStatus == 0 {
				fake.closeStatus = abiStatusOK
			}
			if len(fake.inspect) > 0 && fake.inspect[0].Status == abiStatusOK {
				fake.inspect[0] = rawFromResult(oracle, abiStatusOK)
			}
			installFakeFactory(t, fakeRustFactory(fake))
			t.Setenv(queryBackendEnv, "rust")

			result, err := Inspect(request)
			if !IsFallbackError(err) {
				t.Fatalf("err = %v, want typed fallback error", err)
			}
			if !equalQueryResults(result, oracle) {
				t.Fatalf("fallback changed oracle:\nGot=%+v\nWant=%+v", result, oracle)
			}
		})
	}
}

func TestRustPanicAndInvalidResultFallBackToOracle(t *testing.T) {
	t.Run("panic", func(t *testing.T) {
		installFakeFactory(t, func() (queryABI, error) { panic("fake cgo panic") })
		t.Setenv(queryBackendEnv, "rust")
		result, err := Inspect(adapterRequest(adapterValidQuery()))
		if !IsFallbackError(err) || result.Backend != BackendGo {
			t.Fatalf("panic fallback = result=%+v err=%v", result, err)
		}
	})

	t.Run("invalid-result", func(t *testing.T) {
		wire := adapterValidQuery()
		request := adapterRequest(wire)
		oracle, err := goOracle(request)
		if err != nil {
			t.Fatalf("goOracle: %v", err)
		}
		fake := &fakeQueryABI{
			version:      queryABIVersion,
			capabilities: requiredQueryCapabilities,
			wire:         append([]byte(nil), wire...),
			required:     uint64(len(wire)),
			inspect:      []queryABIResult{rawFromResult(oracle, abiStatusOK)},
		}
		fake.inspect[0].Version++
		installFakeFactory(t, fakeRustFactory(fake))
		t.Setenv(queryBackendEnv, "rust")
		result, err := Inspect(request)
		if !IsFallbackError(err) || !equalQueryResults(result, oracle) {
			t.Fatalf("invalid result fallback = result=%+v err=%v", result, err)
		}
	})
}

func TestMalformedAndUnsupportedQueriesKeepGoErrorClassification(t *testing.T) {
	for _, tc := range []struct {
		name string
		wire []byte
		want error
	}{
		{name: "malformed", wire: []byte{1, 2, 3}, want: ErrMalformedQuery},
		{name: "unsupported", wire: func() []byte {
			wire := adapterValidQuery()
			wire[2] |= 0x80
			return wire
		}(), want: ErrUnsupportedQuery},
	} {
		t.Run(tc.name, func(t *testing.T) {
			called := false
			installFakeFactory(t, func() (queryABI, error) {
				called = true
				return nil, errors.New("oracle errors must not load Rust")
			})
			t.Setenv(queryBackendEnv, "rust")
			_, err := Inspect(adapterRequest(tc.wire))
			if !errors.Is(err, tc.want) {
				t.Fatalf("err = %v, want %v", err, tc.want)
			}
			if called {
				t.Fatal("malformed/unsupported input loaded Rust")
			}
		})
	}
}

func TestRustPathDoesNotMutateInputAndReturnsIndependentCopies(t *testing.T) {
	wire := adapterQueryWithECS(1, 24, 0, 1, 2, 3, 0)
	original := append([]byte(nil), wire...)
	request := adapterRequest(wire)
	oracle, err := goOracle(request)
	if err != nil {
		t.Fatalf("goOracle: %v", err)
	}
	if oracle.ECS == nil {
		t.Fatal("Go oracle did not report ECS on the ECS fixture")
	}
	fake := &fakeQueryABI{
		version:      queryABIVersion,
		capabilities: requiredQueryCapabilities,
		wire:         append([]byte(nil), wire...),
		required:     uint64(len(wire)),
		inspect:      []queryABIResult{rawFromResult(oracle, abiStatusOK)},
	}
	installFakeFactory(t, fakeRustFactory(fake))
	t.Setenv(queryBackendEnv, "rust")
	first, err := Inspect(request)
	if err != nil {
		t.Fatalf("first Inspect: %v", err)
	}
	if first.ECS == nil {
		t.Fatal("Rust path lost the ECS result")
	}
	first.QueryWire[0] ^= 0xff
	first.ECS.Address[0] ^= 0xff
	fake.inspectCalls = 0
	second, err := Inspect(request)
	if err != nil {
		t.Fatalf("second Inspect: %v", err)
	}
	if second.ECS == nil {
		t.Fatal("Rust path lost the ECS result on the second call")
	}
	if !bytes.Equal(wire, original) {
		t.Fatal("Rust path modified caller input wire")
	}
	if !bytes.Equal(second.QueryWire, original) {
		t.Fatal("repeated result does not preserve the oracle wire")
	}
	if bytes.Equal(second.QueryWire, first.QueryWire) {
		t.Fatal("repeated result aliases the previous caller-owned wire")
	}
	if bytes.Equal(second.ECS.Address[:], first.ECS.Address[:]) {
		t.Fatal("repeated result aliases the previous ECS address storage")
	}
	if !bytes.Equal(second.ECS.Address[:4], []byte{1, 2, 3, 0}) {
		t.Fatal("mutating first.ECS.Address changed the repeated ECS result")
	}
	if !bytes.Equal(second.ECS.Address[:4], oracle.ECS.Address[:4]) {
		t.Fatal("repeated ECS result differs from the oracle address")
	}
}

func TestRustResultParityIncludesTransportAndECSFields(t *testing.T) {
	wire := adapterValidQuery()
	request := adapterRequest(wire)
	request.FromUDP = false
	request.AdvertisedUDPSize = 4096
	request.Transport = TransportHTTP
	request.PreFastFlags = 0xfeedbeef
	oracle, err := goOracle(request)
	if err != nil {
		t.Fatalf("goOracle: %v", err)
	}
	fake := &fakeQueryABI{
		version:      queryABIVersion,
		capabilities: requiredQueryCapabilities,
		wire:         append([]byte(nil), wire...),
		required:     uint64(len(wire)),
		inspect:      []queryABIResult{rawFromResult(oracle, abiStatusOK)},
	}
	installFakeFactory(t, fakeRustFactory(fake))
	t.Setenv(queryBackendEnv, "rust")
	result, err := Inspect(request)
	if err != nil {
		t.Fatalf("Inspect: %v", err)
	}
	if !equalQueryResults(result, oracle) {
		t.Fatalf("field parity mismatch:\nRust=%+v\nGo=%+v", result, oracle)
	}
}

func TestRustResultParityNormalizesECSAddressWidth(t *testing.T) {
	cases := []struct {
		name   string
		family uint16
		mask   uint8
		addr   []byte
	}{
		{name: "ipv4", family: 1, mask: 24, addr: []byte{1, 2, 3}},
		{name: "ipv6", family: 2, mask: 56, addr: []byte{1, 2, 3, 4, 5, 6, 7}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			wire := adapterQueryWithECS(tc.family, tc.mask, 0, tc.addr...)
			request := adapterRequest(wire)
			oracle, err := goOracle(request)
			if err != nil {
				t.Fatalf("goOracle: %v", err)
			}
			if oracle.ECS == nil {
				t.Fatal("Go oracle did not report ECS")
			}
			fake := &fakeQueryABI{
				version:      queryABIVersion,
				capabilities: requiredQueryCapabilities,
				wire:         append([]byte(nil), wire...),
				required:     uint64(len(wire)),
				inspect:      []queryABIResult{rawFromResult(oracle, abiStatusOK)},
			}
			installFakeFactory(t, fakeRustFactory(fake))
			t.Setenv(queryBackendEnv, "rust")
			result, err := Inspect(request)
			if err != nil {
				t.Fatalf("Inspect: %v", err)
			}
			if !equalQueryResults(result, oracle) {
				t.Fatalf("ECS parity mismatch:\nRust=%+v\nGo=%+v", result, oracle)
			}
		})
	}
}

func TestFallbackErrorRetainsUnderlyingRuntimeCause(t *testing.T) {
	want := errors.New("runtime failed")
	installFakeFactory(t, func() (queryABI, error) { return nil, want })
	t.Setenv(queryBackendEnv, "rust")
	_, err := Inspect(adapterRequest(adapterValidQuery()))
	if !IsFallbackError(err) || !errors.Is(err, want) {
		t.Fatalf("err = %v, want fallback wrapping runtime cause", err)
	}
	if !strings.Contains(err.Error(), "fallback") {
		t.Fatalf("err = %v, want observable fallback wording", err)
	}
}
