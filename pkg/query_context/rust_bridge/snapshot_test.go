package rust_bridge_test

import (
	"bytes"
	"errors"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context/rust_bridge"
)

// queryWire is a packed DNS query: ID 0x1234, RD, one question example.org. A IN.
var queryWire = []byte{
	0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x07, 'e', 'x', 'a', 'm', 'p', 'l', 'e',
	0x03, 'o', 'r', 'g',
	0x00,
	0x00, 0x01, // QTYPE A
	0x00, 0x01, // QCLASS IN
}

func TestSnapshotCopiesInputBytes(t *testing.T) {
	wire := append([]byte(nil), queryWire...)
	original := append([]byte(nil), wire...)

	s, err := rust_bridge.NewSnapshot(wire)
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}

	// Mutating the caller's slice after NewSnapshot must not affect the snapshot.
	for i := range wire {
		wire[i] ^= 0xff
	}
	if got := s.Wire(); !bytes.Equal(got, original) {
		t.Fatal("snapshot aliases caller input: content changed after NewSnapshot")
	}

	// Wire must return a caller-owned copy: mutating it must not affect the snapshot.
	got := s.Wire()
	for i := range got {
		got[i] ^= 0xff
	}
	if again := s.Wire(); !bytes.Equal(again, original) {
		t.Fatal("Wire does not return a copy: snapshot storage was mutated")
	}
}

func TestInspectWritesCallerOwnedResultWithRequiredLength(t *testing.T) {
	wire := append([]byte(nil), queryWire...)
	s, err := rust_bridge.NewSnapshot(wire)
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}

	required := s.RequiredResultLength()
	if required != len(wire) {
		t.Fatalf("RequiredResultLength = %d, want %d", required, len(wire))
	}

	// Caller owns the buffer; bytes past the required length must stay untouched.
	buf := make([]byte, required+8)
	for i := range buf {
		buf[i] = 0xaa
	}
	n, err := s.Inspect(buf)
	if err != nil {
		t.Fatalf("Inspect: %v", err)
	}
	if n != required {
		t.Fatalf("Inspect wrote %d bytes, want %d", n, required)
	}
	if !bytes.Equal(buf[:n], wire) {
		t.Fatal("Inspect result does not match snapshot content")
	}
	for _, b := range buf[n:] {
		if b != 0xaa {
			t.Fatal("Inspect wrote past the required length")
		}
	}

	// Mutating the caller-owned result must not affect the snapshot.
	buf[0] ^= 0xff
	if got := s.Wire(); !bytes.Equal(got, wire) {
		t.Fatal("Inspect result aliases snapshot storage")
	}
}

func TestInspectTooSmallBufferFailsWithoutModifyingInputOrResult(t *testing.T) {
	wire := append([]byte(nil), queryWire...)
	inputCopy := append([]byte(nil), wire...)
	s, err := rust_bridge.NewSnapshot(wire)
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}

	required := s.RequiredResultLength()
	if required == 0 {
		t.Fatal("fixture must have a non-empty required length")
	}
	buf := make([]byte, required-1)
	for i := range buf {
		buf[i] = 0x5a
	}
	bufCopy := append([]byte(nil), buf...)

	n, err := s.Inspect(buf)
	if !errors.Is(err, rust_bridge.ErrResultTooSmall) {
		t.Fatalf("err = %v, want ErrResultTooSmall", err)
	}
	if n != required {
		t.Fatalf("n = %d, want required length %d", n, required)
	}
	if !bytes.Equal(buf, bufCopy) {
		t.Fatal("failed Inspect modified the caller result buffer")
	}
	if !bytes.Equal(wire, inputCopy) {
		t.Fatal("failed Inspect modified the snapshot input bytes")
	}
}
