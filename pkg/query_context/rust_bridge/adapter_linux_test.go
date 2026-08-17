//go:build linux && cgo && mosdns_rust

// Real Linux+cgo integration tests for the query snapshot ABI. These tests
// deliberately bypass installFakeFactory and the fakeQueryABI seam: Inspect
// selects the real static library through MOSDNS_QUERY_BACKEND=rust, and the
// direct ABI test drives cgoQueryABI (newNativeABI) against the actual
// mosdns_runtime staticlib. They prove the full Go -> cgo -> Rust
// MosdnsQuerySnapshotInput -> MosdnsQueryInspectResult chain, caller-owned
// output buffers, BUFFER_TOO_SMALL required-length retry, ECS/QueryWire
// no-alias behavior, and handle lifecycle without any fake.

package rust_bridge

import (
	"bytes"
	"errors"
	"testing"
)

func realRustRequest(wire []byte) QueryRequest {
	return adapterRequest(wire)
}

// adapterQueryWithDO builds example.com A with one EDNS OPT record carrying
// DO=true and no options: it isolates the DO contract without ECS.
func adapterQueryWithDO() []byte {
	// 0x00 root name, type OPT (0x0029), class 1232 (advertised UDP size),
	// TTL 0x00008000 (DO bit set; bit 15 per RFC 3225/6891), RDLENGTH 0.
	opt := []byte{0x00, 0x00, 0x29, 0x04, 0xd0, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00}
	wire := append([]byte(nil), adapterValidQuery()...)
	wire[11] = 1
	return append(wire, opt...)
}

func TestRealRustABIPlainQueryParity(t *testing.T) {
	wire := adapterValidQuery()
	request := realRustRequest(wire)
	oracle, err := goOracle(request)
	if err != nil {
		t.Fatalf("goOracle: %v", err)
	}
	t.Setenv(queryBackendEnv, "rust")

	result, err := Inspect(request)
	if err != nil {
		t.Fatalf("real Rust Inspect: %v", err)
	}
	if result.Backend != BackendRust {
		t.Fatalf("backend = %q, want real Rust result", result.Backend)
	}
	if !equalQueryResults(result, oracle) {
		t.Fatalf("real Rust result differs from Go oracle:\nRust=%+v\nGo=%+v", result, oracle)
	}
	if !bytes.Equal(result.QueryWire, wire) {
		t.Fatal("real Rust returned mutated query wire")
	}
	if result.ID != 0x1234 || result.QType != 1 || result.QClass != 1 {
		t.Fatalf("normalized header/question = id %#x qtype %d qclass %d", result.ID, result.QType, result.QClass)
	}
	if result.ECS != nil || result.HasOPT {
		t.Fatalf("plain query must not report EDNS/ECS, got %+v", result.ECS)
	}
}

func TestRealRustABIECSQueryParity(t *testing.T) {
	wire := adapterQueryWithECS(1, 24, 0, 1, 2, 3, 0)
	request := realRustRequest(wire)
	oracle, err := goOracle(request)
	if err != nil {
		t.Fatalf("goOracle: %v", err)
	}
	t.Setenv(queryBackendEnv, "rust")

	result, err := Inspect(request)
	if err != nil {
		t.Fatalf("real Rust ECS Inspect: %v", err)
	}
	if result.Backend != BackendRust {
		t.Fatalf("backend = %q, want real Rust result", result.Backend)
	}
	if !equalQueryResults(result, oracle) {
		t.Fatalf("real Rust ECS result differs from Go oracle:\nRust=%+v\nGo=%+v", result, oracle)
	}
	if result.ECS == nil {
		t.Fatal("real Rust did not report ECS")
	}
	if result.ECS.Family != 1 || result.ECS.SourceNetmask != 24 || result.ECS.SourceScope != 0 {
		t.Fatalf("ECS metadata = %+v, want family 1 / mask 24 / scope 0", result.ECS)
	}
	if !bytes.Equal(result.ECS.Address[:4], []byte{1, 2, 3, 0}) {
		t.Fatalf("ECS address = %v, want prefix 1.2.3.0", result.ECS.Address)
	}
}

func TestRealRustABIDOQueryParity(t *testing.T) {
	// EDNS OPT present with DO=true and no ECS isolates the DO contract on
	// the real Go -> cgo -> Rust staticlib path. The fake ABI seam is never
	// installed here; Inspect selects the real static library through
	// MOSDNS_QUERY_BACKEND=rust.
	wire := adapterQueryWithDO()
	request := realRustRequest(wire)
	oracle, err := goOracle(request)
	if err != nil {
		t.Fatalf("goOracle: %v", err)
	}
	t.Setenv(queryBackendEnv, "rust")

	result, err := Inspect(request)
	if err != nil {
		t.Fatalf("real Rust DO Inspect: %v", err)
	}
	if result.Backend != BackendRust {
		t.Fatalf("backend = %q, want real Rust result", result.Backend)
	}
	if !result.HasOPT {
		t.Fatal("real Rust did not report OPT presence")
	}
	if !result.DO {
		t.Fatal("real Rust did not report DO=true")
	}
	if result.EDNSUDPSize != 1232 {
		t.Fatalf("EDNS UDP size = %d, want 1232", result.EDNSUDPSize)
	}
	if result.ECS != nil {
		t.Fatalf("DO fixture without ECS must not report ECS, got %+v", result.ECS)
	}
	if !equalQueryResults(result, oracle) {
		t.Fatalf("real Rust DO result differs from Go oracle:\nRust=%+v\nGo=%+v", result, oracle)
	}
	if !bytes.Equal(result.QueryWire, wire) {
		t.Fatal("real Rust returned mutated query wire")
	}
	if oracle.HasOPT != true || oracle.DO != true || oracle.EDNSUDPSize != 1232 {
		t.Fatalf("Go oracle fixture check failed: has_opt=%v do=%v udp_size=%d", oracle.HasOPT, oracle.DO, oracle.EDNSUDPSize)
	}
}

func TestRealRustABIRepeatedInspectDoesNotAlias(t *testing.T) {
	// The first result must be fully caller-owned: mutating its QueryWire and
	// ECS.Address must not affect the second result or the original input.
	wire := adapterQueryWithECS(1, 24, 0, 1, 2, 3, 0)
	original := append([]byte(nil), wire...)
	request := realRustRequest(wire)
	t.Setenv(queryBackendEnv, "rust")

	first, err := Inspect(request)
	if err != nil {
		t.Fatalf("first Inspect: %v", err)
	}
	second, err := Inspect(request)
	if err != nil {
		t.Fatalf("second Inspect: %v", err)
	}
	if first.ECS == nil || second.ECS == nil {
		t.Fatal("ECS fixture lost ECS on the real Rust path")
	}

	first.QueryWire[0] ^= 0xff
	first.ECS.Address[0] ^= 0xff

	if !bytes.Equal(wire, original) {
		t.Fatal("mutating a result modified the caller input wire")
	}
	if !bytes.Equal(second.QueryWire, original) {
		t.Fatal("mutating first.QueryWire changed second.QueryWire")
	}
	if !bytes.Equal(second.ECS.Address[:4], []byte{1, 2, 3, 0}) {
		t.Fatal("mutating first.ECS.Address changed second.ECS.Address")
	}
	if bytes.Equal(first.QueryWire, second.QueryWire) {
		t.Fatal("results share the same QueryWire storage")
	}
	if bytes.Equal(first.ECS.Address[:], second.ECS.Address[:]) {
		t.Fatal("results share the same ECS address storage")
	}
}

func TestRealRustABIRepeatedCallsExerciseHandleLifecycle(t *testing.T) {
	// Repeated Inspect calls drive create -> required len -> inspect -> close
	// through the real staticlib each time. Any handle leak, stale pointer,
	// or released-buffer reuse surfaces here as a wrong result or a crash.
	wire := adapterQueryWithECS(2, 56, 0, 0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1)
	request := realRustRequest(wire)
	oracle, err := goOracle(request)
	if err != nil {
		t.Fatalf("goOracle: %v", err)
	}
	t.Setenv(queryBackendEnv, "rust")

	for i := 0; i < 64; i++ {
		result, err := Inspect(request)
		if err != nil {
			t.Fatalf("Inspect call %d: %v", i, err)
		}
		if result.Backend != BackendRust {
			t.Fatalf("call %d backend = %q, want Rust", i, result.Backend)
		}
		if !equalQueryResults(result, oracle) {
			t.Fatalf("call %d result differs from oracle:\nRust=%+v\nGo=%+v", i, result, oracle)
		}
	}
}

func TestRealRustABIHandleLifecycleAndBufferTooSmall(t *testing.T) {
	// Drive the real cgo ABI directly: version/capability negotiation, handle
	// creation, required length, a too-small caller buffer reporting the exact
	// required length without writing, the retried exact-size call, close,
	// idempotent double close, and closed-handle use after close.
	abi, err := newNativeABI()
	if err != nil {
		t.Fatalf("newNativeABI: %v", err)
	}
	if version := abi.Version(); version != queryABIVersion {
		t.Fatalf("real ABI version = %d, want %d", version, queryABIVersion)
	}
	if capabilities := abi.Capabilities(); capabilities&requiredQueryCapabilities != requiredQueryCapabilities {
		t.Fatalf("real ABI capabilities %#x do not include %#x", capabilities, requiredQueryCapabilities)
	}

	wire := adapterValidQuery()
	original := append([]byte(nil), wire...)
	// inputWire is the exact slice handed to Rust through the borrowed
	// descriptor. The snapshot must copy it before Create returns, so mutating
	// this same slice afterwards must not change what Rust inspects.
	inputWire := append([]byte(nil), wire...)
	handle, err := abi.Create(queryABIInput{
		Version:           queryABIVersion,
		QueryWire:         inputWire,
		FromUDP:           true,
		Transport:         TransportUDP,
		AdvertisedUDPSize: 1232,
		PreFastFlags:      0x1234,
	})
	if err != nil {
		t.Fatalf("real Create: %v", err)
	}
	if handle == 0 {
		t.Fatal("real Create returned an empty handle")
	}

	// The snapshot must own its bytes: mutating the exact buffer that was
	// passed to Create must not change what the real runtime inspects.
	inputWire[0] ^= 0xff
	defer func() { inputWire[0] ^= 0xff }()

	required, err := abi.RequiredLen(handle)
	if err != nil {
		t.Fatalf("real RequiredLen: %v", err)
	}
	if required != uint64(len(original)) {
		t.Fatalf("required len = %d, want %d", required, len(original))
	}

	small := make([]byte, int(required)-1)
	fill := byte(0xa5)
	for i := range small {
		small[i] = fill
	}
	raw, err := abi.Inspect(handle, small)
	if err != nil {
		t.Fatalf("real Inspect with short buffer: %v", err)
	}
	if raw.Status != abiStatusBufferTooSmall {
		t.Fatalf("short buffer status = %d, want BUFFER_TOO_SMALL", raw.Status)
	}
	if raw.RequiredLen != required {
		t.Fatalf("short buffer required len = %d, want %d", raw.RequiredLen, required)
	}
	for _, b := range small {
		if b != fill {
			t.Fatal("short inspect wrote into the caller buffer")
		}
	}

	full := make([]byte, int(required))
	raw, err = abi.Inspect(handle, full)
	if err != nil {
		t.Fatalf("real Inspect with exact buffer: %v", err)
	}
	if raw.Status != abiStatusOK {
		t.Fatalf("exact buffer status = %d, want OK", raw.Status)
	}
	if raw.Version != queryResultVersion || raw.WrittenLen != required || raw.RequiredLen != required {
		t.Fatalf("inspect result = %+v, want version %d written/required %d", raw, queryResultVersion, required)
	}
	if raw.ID != 0x1234 || raw.QType != 1 || raw.QClass != 1 {
		t.Fatalf("normalized fields = id %#x qtype %d qclass %d", raw.ID, raw.QType, raw.QClass)
	}
	if !bytes.Equal(full, original) {
		t.Fatal("real Inspect returned mutated query wire")
	}

	if err := abi.Close(handle); err != nil {
		t.Fatalf("real Close: %v", err)
	}
	if err := abi.Close(handle); err != nil {
		t.Fatalf("real double Close must be idempotent: %v", err)
	}

	afterClose, err := abi.Inspect(handle, full)
	if err != nil {
		t.Fatalf("real Inspect after close: %v", err)
	}
	if afterClose.Status != abiStatusClosed {
		t.Fatalf("use after close status = %d, want CLOSED", afterClose.Status)
	}

	if _, err := abi.RequiredLen(handle); err == nil {
		t.Fatal("RequiredLen after close must fail")
	} else {
		var abiErr *abiStatusError
		if !errors.As(err, &abiErr) || abiErr.Status != abiStatusClosed {
			t.Fatalf("RequiredLen after close err = %v, want abiStatusError with CLOSED", err)
		}
	}
}
