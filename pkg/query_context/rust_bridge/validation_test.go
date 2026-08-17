package rust_bridge_test

import (
	"bytes"
	"errors"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context/rust_bridge"
)

// Wire fixtures are built by hand so each case isolates exactly one wire
// defect. Expected outcomes follow the existing Go DNS wire behavior:
// name/compression legality from miekg/dns UnpackDomainName (the parser used
// by the live query path) and the query-shape checks from
// pkg/server_handler EntryHandler.

func hdr(flags, opcode byte, qd, an, ns, ar uint16) []byte {
	b := make([]byte, 12)
	b[0], b[1] = 0x12, 0x34
	b[2] = flags | opcode<<3 // QR(1) Opcode(4) AA TC RD
	b[3] = 0
	b[4] = byte(qd >> 8)
	b[5] = byte(qd)
	b[6] = byte(an >> 8)
	b[7] = byte(an)
	b[8] = byte(ns >> 8)
	b[9] = byte(ns)
	b[10] = byte(ar >> 8)
	b[11] = byte(ar)
	return b
}

func name(labels ...string) []byte {
	var b []byte
	for _, l := range labels {
		b = append(b, byte(len(l)))
		b = append(b, l...)
	}
	return append(b, 0)
}

func validQuery() []byte {
	w := hdr(0x01, 0, 1, 0, 0, 0)
	w = append(w, name("example", "org")...)
	return append(w, 0x00, 0x01, 0x00, 0x01) // A IN
}

func TestNewSnapshotAcceptsValidQueries(t *testing.T) {
	// ar=1 with one OPT: the handler allows at most one extra record.
	withOpt := hdr(0x01, 0, 1, 0, 0, 1)
	withOpt = append(withOpt, name("example", "org")...)
	withOpt = append(withOpt, 0x00, 0x01, 0x00, 0x01)
	withOpt = append(withOpt, 0x00, 0x00, 0x29, 0x04, 0xd0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00)

	rootName := hdr(0x01, 0, 1, 0, 0, 0)
	rootName = append(rootName, 0x00)
	rootName = append(rootName, 0x00, 0x1c, 0x00, 0x01) // AAAA IN

	unknownQT := hdr(0x01, 0, 1, 0, 0, 0)
	unknownQT = append(unknownQT, name("example", "org")...)
	unknownQT = append(unknownQT, 0x00, 0xff, 0x00, 0xfe) // qtype 255, qclass 254

	compressed := hdr(0x01, 0, 1, 0, 0, 0)
	compressed = append(compressed, name("example")...)
	compressed = append(compressed, 0xc0, 0x0c) // backward pointer, legal compression
	compressed = append(compressed, 0x00, 0x01, 0x00, 0x01)

	trailing := append(validQuery(), 0xff, 0x00) // trailing bytes are ignored, as in the live unpack path

	tests := []struct {
		name string
		wire []byte
	}{
		{name: "plain", wire: validQuery()},
		{name: "with-opt", wire: withOpt},
		{name: "root-name", wire: rootName},
		{name: "unknown-qtype-qclass", wire: unknownQT},
		{name: "compressed-name", wire: compressed},
		{name: "trailing-bytes", wire: trailing},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			s, err := rust_bridge.NewSnapshot(tt.wire)
			if err != nil {
				t.Fatalf("NewSnapshot: %v", err)
			}
			if !bytes.Equal(s.Wire(), tt.wire) {
				t.Fatal("snapshot content does not match accepted input")
			}
		})
	}
}

func TestNewSnapshotRejectsMalformedWire(t *testing.T) {
	selfPtr := hdr(0x01, 0, 1, 0, 0, 0)
	selfPtr = append(selfPtr, 0xc0, 0x0c) // pointer to itself

	ptrOutOfBounds := hdr(0x01, 0, 1, 0, 0, 0)
	ptrOutOfBounds = append(ptrOutOfBounds, 0xc0, 0x14) // forward/out-of-bounds pointer

	longLabel := hdr(0x01, 0, 1, 0, 0, 0)
	longLabel = append(longLabel, 0x40) // 64-byte label
	longLabel = append(longLabel, make([]byte, 64)...)

	noNameBytes := hdr(0x01, 0, 1, 0, 0, 0) // QD=1 but question is absent

	nameOnly := hdr(0x01, 0, 1, 0, 0, 0)
	nameOnly = append(nameOnly, name("example")...) // no qtype/qclass

	missingQclass := hdr(0x01, 0, 1, 0, 0, 0)                 // miekg tolerates this as Qclass 0;
	missingQclass = append(missingQclass, name("example")...) // the strict contract rejects it
	missingQclass = append(missingQclass, 0x00, 0x01)

	tests := []struct {
		name string
		wire []byte
	}{
		{name: "empty", wire: nil},
		{name: "shorter-than-header", wire: []byte{1, 2, 3, 4, 5, 6}},
		{name: "self-pointer", wire: selfPtr},
		{name: "pointer-out-of-bounds", wire: ptrOutOfBounds},
		{name: "label-too-long", wire: longLabel},
		{name: "question-missing", wire: noNameBytes},
		{name: "question-name-only", wire: nameOnly},
		{name: "question-missing-qclass", wire: missingQclass},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			before := append([]byte(nil), tt.wire...)
			s, err := rust_bridge.NewSnapshot(tt.wire)
			if !errors.Is(err, rust_bridge.ErrMalformedQuery) {
				t.Fatalf("err = %v, want ErrMalformedQuery", err)
			}
			if s != nil {
				t.Fatal("malformed input produced a snapshot")
			}
			if !bytes.Equal(tt.wire, before) {
				t.Fatal("NewSnapshot modified the caller's input bytes")
			}
		})
	}
}

func TestNewSnapshotRejectsDomainNameOver255WireBytes(t *testing.T) {
	wire := hdr(0x01, 0, 1, 0, 0, 0)
	for i := 0; i < 4; i++ {
		wire = append(wire, 63)
		wire = append(wire, bytes.Repeat([]byte{'x'}, 63)...)
	}
	wire = append(wire, 0, 0, 1, 0, 1)

	s, err := rust_bridge.NewSnapshot(wire)
	if !errors.Is(err, rust_bridge.ErrMalformedQuery) {
		t.Fatalf("err = %v, want ErrMalformedQuery", err)
	}
	if s != nil {
		t.Fatal("overlong domain name produced a snapshot")
	}
}

func TestNewSnapshotRejectsUnsupportedQueries(t *testing.T) {
	qrSet := validQuery()
	qrSet[2] |= 0x80 // QR bit

	opcodeStatus := hdr(0x01, 2, 1, 0, 0, 0)
	opcodeStatus = append(opcodeStatus, name("example", "org")...)
	opcodeStatus = append(opcodeStatus, 0x00, 0x01, 0x00, 0x01)

	opcode15 := hdr(0x01, 15, 1, 0, 0, 0)
	opcode15 = append(opcode15, name("example", "org")...)
	opcode15 = append(opcode15, 0x00, 0x01, 0x00, 0x01)

	qd0 := hdr(0x01, 0, 0, 0, 0, 0)

	qd2 := hdr(0x01, 0, 2, 0, 0, 0)
	qd2 = append(qd2, name("example", "org")...)
	qd2 = append(qd2, 0x00, 0x01, 0x00, 0x01)
	qd2 = append(qd2, name("www", "example", "org")...)
	qd2 = append(qd2, 0x00, 0x01, 0x00, 0x01)

	withAnswer := hdr(0x01, 0, 1, 1, 0, 0)
	withAnswer = append(withAnswer, name("example", "org")...)
	withAnswer = append(withAnswer, 0x00, 0x01, 0x00, 0x01)
	withAnswer = append(withAnswer, 0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 1, 2, 3, 4)

	withNs := hdr(0x01, 0, 1, 0, 1, 0)
	withNs = append(withNs, name("example", "org")...)
	withNs = append(withNs, 0x00, 0x01, 0x00, 0x01)
	withNs = append(withNs, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x00)

	ar2 := hdr(0x01, 0, 1, 0, 0, 2)
	ar2 = append(ar2, name("example", "org")...)
	ar2 = append(ar2, 0x00, 0x01, 0x00, 0x01)

	// an=65535 with ns=1: the counts must be checked separately, since their
	// uint16 sum wraps to zero and would pass an an+ns>0 comparison.
	anMaxNsOne := hdr(0x01, 0, 1, 0xffff, 1, 0)
	anMaxNsOne = append(anMaxNsOne, name("example", "org")...)
	anMaxNsOne = append(anMaxNsOne, 0x00, 0x01, 0x00, 0x01)

	tests := []struct {
		name string
		wire []byte
	}{
		{name: "qr-set", wire: qrSet},
		{name: "opcode-status", wire: opcodeStatus},
		{name: "opcode-15", wire: opcode15},
		{name: "zero-questions", wire: qd0},
		{name: "two-questions", wire: qd2},
		{name: "has-answer", wire: withAnswer},
		{name: "has-authority", wire: withNs},
		{name: "two-extras", wire: ar2},
		{name: "answer-count-wraps-with-authority", wire: anMaxNsOne},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			before := append([]byte(nil), tt.wire...)
			s, err := rust_bridge.NewSnapshot(tt.wire)
			if !errors.Is(err, rust_bridge.ErrUnsupportedQuery) {
				t.Fatalf("err = %v, want ErrUnsupportedQuery", err)
			}
			if s != nil {
				t.Fatal("unsupported input produced a snapshot")
			}
			if !bytes.Equal(tt.wire, before) {
				t.Fatal("NewSnapshot modified the caller's input bytes")
			}
		})
	}
}
