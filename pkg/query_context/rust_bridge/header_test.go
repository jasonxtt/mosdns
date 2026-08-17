package rust_bridge_test

import (
	"bytes"
	"encoding/binary"
	"errors"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context/rust_bridge"
	"github.com/miekg/dns"
)

// The response header ID/RA contract is frozen from the live raw-response
// fast path (pkg/server_handler/entry_handler.go packRawResponse). With no
// RespOpt and no UDP oversize, the handler walks the decoded message path:
// resp.Id = q.Id (a full big-endian write to the ID field) and
// resp.RecursionAvailable = true before packing. The pack path in miekg/dns
// then encodes the ID into wire bytes 0-1 and sets the RA flag bit (0x80 in
// wire byte 3) unconditionally. The oracle below freezes exactly those two
// header mutations on a caller-owned copy, without touching the QR, TC, RD,
// opcode, rcode, counts, or any record bytes.

func TestPatchResponseHeaderSetsIDWithBigEndianWrite(t *testing.T) {
	// One RR so the wire lands on the raw path's ordinary response shape.
	wire := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}

	out := s.PatchResponseHeader(0x1235)
	if len(out) != len(wire) {
		t.Fatalf("patched length = %d, want %d", len(out), len(wire))
	}
	// The ID is bytes 0-1, big-endian, exactly as packRawResponse writes
	// q.Id with binary.BigEndian.PutUint16(rawResp[:2], q.Id).
	if got := binary.BigEndian.Uint16(out[0:2]); got != 0x1235 {
		t.Fatalf("ID = %#04x, want 0x1235", got)
	}
	// The QR bit must still be set (the packet remains a response) and the
	// outer byte 2 (flags high) must carry the RA mirror.
	if out[2]&0x80 == 0 {
		t.Fatal("QR bit was cleared by PatchResponseHeader")
	}
	// RA is the bit 0x80 of wire byte 3 (Flags&_RA with mid byte 0x01), i.e.
	// the exact OR that packRawResponse applies.
	if out[3]&0x80 == 0 {
		t.Fatal("RA flag not set by PatchResponseHeader")
	}
	// Everything after the header must be byte-identical (no record mutation).
	if !bytes.Equal(out[12:], wire[12:]) {
		t.Fatal("PatchResponseHeader modified bytes past the 12-byte header")
	}
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("PatchResponseHeader modified the snapshot")
	}
	out[0] ^= 0xff
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("patched result aliases the snapshot storage")
	}
}

func TestPatchResponseHeaderSetsIDAndSetsRABit(t *testing.T) {
	wire := respWire(nil, nil, nil)
	// Pre-clear any RA bit so the test proves it is set, matching the raw path
	// where q's RecursionAvailable is written from a decoded message.
	wire[3] &^= 0x80
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	out := s.PatchResponseHeader(0xcafe)
	if got := binary.BigEndian.Uint16(out[0:2]); got != 0xcafe {
		t.Fatalf("ID = %#04x, want 0xcafe", got)
	}
	if out[3]&0x80 == 0 {
		t.Fatal("RA bit 0x80 not set in wire byte 3")
	}
	// Bytes other than ID (0-1) and the RA bit (byte3 bit7) must be preserved:
	// byte 2 unchanged (QR kept), the RCODE low nibble of byte 3 kept, and the
	// four counts in bytes 4-11 untouched.
	if out[2] != wire[2] {
		t.Fatalf("flags byte 2 changed: %#04x -> %#04x", wire[2], out[2])
	}
	if out[3]&0x0f != wire[3]&0x0f {
		t.Fatalf("RCODE nibble changed: %#04x -> %#04x", wire[3], out[3])
	}
	if !bytes.Equal(out[4:12], wire[4:12]) {
		t.Fatal("PatchResponseHeader modified the header counts")
	}
}

func TestPatchResponseHeaderPreservesFlagsExceptIDAndRA(t *testing.T) {
	// The oracle is a strict response-headers-only patch. TC = wire byte 2 bit
	// 1 (0x02). A TCP-framed or oversized message would take the decoded
	// message path, not the raw fast path, but an oracle caller with a TC-
	// flagged packet must still keep every other flag: the patch touches only
	// the ID and the RA bit.
	wire := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	wire[2] |= 0x02 // TC
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	out := s.PatchResponseHeader(0x0001)
	if out[2]&0x02 == 0 {
		t.Fatal("TC flag was cleared")
	}
	// Unpack to confirm the patch produced a valid message with the expected
	// header fields and preserved flags.
	m := unpack(t, out)
	if m.Id != 0x0001 || !m.RecursionAvailable || !m.Truncated {
		t.Fatalf("decoded mismatch: id=%#04x ra=%v tc=%v", m.Id, m.RecursionAvailable, m.Truncated)
	}
}

func TestPatchResponseHeaderZeroIDAllowed(t *testing.T) {
	// Replacing the ID with 0 is a valid forwarder reset, same as q.Id=0.
	wire := respWire([][]byte{aRR(60, 1, 2, 3, 4)}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	out := s.PatchResponseHeader(0)
	if got := binary.BigEndian.Uint16(out[0:2]); got != 0 {
		t.Fatalf("ID = %#04x, want 0", got)
	}
	// The answer record must survive the header patch unchanged.
	m := unpack(t, out)
	if len(m.Answer) != 1 {
		t.Fatalf("answers = %d, want 1", len(m.Answer))
	}
}

func TestPatchResponseHeaderResultIsCallerOwnedCopy(t *testing.T) {
	wire := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	// Repeated patches with different IDs must be independent: capture the
	// second result, mutate the first, and require the second unchanged.
	p1 := s.PatchResponseHeader(0x1111)
	p2 := s.PatchResponseHeader(0x2222)
	expected := append([]byte(nil), p2...)
	p1[0] = 0xff
	if !bytes.Equal(p2, expected) {
		t.Fatal("PatchResponseHeader result aliases storage: mutating p1 changed p2")
	}
	if got := p2[0]; got != 0x22 {
		t.Fatalf("mutating p1 changed p2[0] = %#02x, want 0x22", got)
	}
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("PatchResponseHeader modified the snapshot")
	}
}

func TestPatchResponseHeaderAcceptsEmptyAndNxdomainResponses(t *testing.T) {
	for name, wire := range map[string][]byte{
		"empty":    respWire(nil, nil, nil),
		"nxdomain": setRcode(respWire(nil, nil, nil), dns.RcodeNameError),
		"servfail": setRcode(respWire(nil, nil, nil), dns.RcodeServerFailure),
		"only-opt": respWire(nil, nil, [][]byte{optRR(1232, 0)}),
	} {
		t.Run(name, func(t *testing.T) {
			s, err := rust_bridge.NewResponseSnapshot(wire)
			if err != nil {
				t.Fatalf("NewResponseSnapshot: %v", err)
			}
			out := s.PatchResponseHeader(0x9999)
			if got := binary.BigEndian.Uint16(out[0:2]); got != 0x9999 {
				t.Fatalf("ID = %#04x, want 0x9999", got)
			}
			if bytes.Equal(out, wire) {
				t.Fatal("PatchResponseHeader produced no change on a response")
			}
		})
	}
}

func TestPatchResponseHeaderIsNotAValidMessageInput(t *testing.T) {
	// The oracle is a response-headers-only helper: the raw fast path already
	// rejects short or QR-clear packets before patching (packRawResponse:
	// len<12 || !QR → error, no output). A non-response packet is not a valid
	// snapshot and construction must fail without producing any output.
	wire := validQuery()
	before := append([]byte(nil), wire...)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if !errors.Is(err, rust_bridge.ErrUnsupportedResponse) {
		t.Fatalf("err = %v, want ErrUnsupportedResponse", err)
	}
	if s != nil {
		t.Fatal("non-response produced a snapshot")
	}
	if !bytes.Equal(wire, before) {
		t.Fatal("NewResponseSnapshot modified the caller's input bytes")
	}
}
