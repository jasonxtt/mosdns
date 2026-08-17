// Response TTL oracle contract.
//
// This file freezes the Go oracle for response TTL observation, aging, and
// replacement at the wire level. It mirrors the current cache wire walk
// (rust/cache-core wire.rs age_ttls/set_ttls/validate_response): the declared
// question, answer, authority, and extra counts are walked in header order,
// names and record bounds must be wire-legal, and OPT records (type 41) are
// skipped for TTL purposes. Aging subtracts a whole-second delta with
// saturating arithmetic (floor 0) and replacement sets every non-OPT record's
// TTL, both on a caller-owned copy. Observation reports the minimal TTL and
// the number of TTL-bearing records, which is what pkg/dnsutils.GetMinimalTTL
// and the cache expiry path consume.
//
// Like the query seam, construction copies its input and any failure happens
// before a snapshot exists, so no caller-visible output is ever partially
// updated. The response TTL contract is narrower than the decoded message
// helpers in pkg/dnsutils: dnsutils.SubtractTTL decodes first and floors a
// clamped TTL at 1, while this wire oracle saturates at 0 (design.md
// "saturating arithmetic"; byte-compatible with the existing cache-core wire
// behavior). See ErrMalformedResponse / ErrUnsupportedResponse for the frozen
// classification.
package rust_bridge

import (
	"encoding/binary"
	"errors"
	"fmt"
)

const (
	dnsHeaderLen = 12
	rrFixedLen   = 10 // type(2) class(2) ttl(4) rdlength(2)
	rrTTLOffset  = 4  // TTL bytes start after type and class
	dnsTypeOPT   = 41
	dnsMaxLabel  = 63

	// flagRA is the Recursion-Available bit of the DNS header word at offset
	// 2 (big endian): 1<<7 lands in wire byte 3 bit 7 (0x80), matching
	// miekg/dns _RA and the raw fast path's OR mask
	// rawResp[3] |= 0x80 in pkg/server_handler/packRawResponse.
	flagRA = 1 << 7
)

// ErrMalformedResponse reports a wire-level defect in a DNS response: the
// message is shorter than a DNS header, a question or record name has an
// illegal label or compression pointer, or a question type/class, record
// fixed field, or record rdata is truncated or out of bounds.
var ErrMalformedResponse = errors.New("malformed DNS response")

// ErrUnsupportedResponse reports a wire-legal DNS message that is not a
// response: the QR bit is clear.
var ErrUnsupportedResponse = errors.New("unsupported DNS response")

// TTLInfo is the observed TTL state of a response snapshot. MinimalTTL is the
// smallest TTL over every answer/authority/extra record, ignoring OPT records;
// it is 0 when the response carries no non-OPT record, matching
// pkg/dnsutils.GetMinimalTTL. RecordCount is the number of non-OPT records
// whose TTL was observed.
type TTLInfo struct {
	MinimalTTL  uint32
	RecordCount uint32
}

// ResponseSnapshot is an immutable copy of a raw DNS response wire handed to
// NewResponseSnapshot. Each inspection or transform returns a caller-owned
// result and never aliases or modifies the snapshot.
type ResponseSnapshot struct {
	wire []byte
}

// NewResponseSnapshot validates resp as a DNS response and copies it into an
// immutable ResponseSnapshot. The caller keeps ownership of resp and may reuse
// or mutate it afterwards.
//
// Validation requires a header, the QR bit set, and a walkable question and
// record section: every declared question and record name must be wire-legal
// (lengths and compression pointers bounded by the packet), and question
// type/class, record fixed fields, and record rdata must fit inside the
// packet. A message shorter than a DNS header or with a defective walk fails
// with ErrMalformedResponse; a wire-legal message with QR clear fails with
// ErrUnsupportedResponse. Trailing bytes after the last declared record are
// tolerated, matching the live unpack path.
//
// On failure no snapshot is created and resp is never modified.
func NewResponseSnapshot(resp []byte) (*ResponseSnapshot, error) {
	if len(resp) < dnsHeaderLen {
		return nil, fmt.Errorf("%w: %d bytes is shorter than a DNS header", ErrMalformedResponse, len(resp))
	}
	if resp[2]&0x80 == 0 {
		return nil, fmt.Errorf("%w: QR bit is clear", ErrUnsupportedResponse)
	}
	if err := walkTTLOffsets(resp, func(int) {}); err != nil {
		return nil, err
	}
	return &ResponseSnapshot{wire: append([]byte(nil), resp...)}, nil
}

// Wire returns a caller-owned copy of the snapshot's response wire bytes.
func (s *ResponseSnapshot) Wire() []byte {
	return append([]byte(nil), s.wire...)
}

// RequiredResultLength returns the exact number of bytes AgeTTL and ReplaceTTL
// write. TTL patching preserves the wire length, so every transform result has
// this length.
func (s *ResponseSnapshot) RequiredResultLength() int {
	return len(s.wire)
}

// ObserveTTL reports the response's TTL observations: the minimal TTL over all
// non-OPT records and the count of observed records. It allocates nothing and
// never modifies the snapshot.
func (s *ResponseSnapshot) ObserveTTL() TTLInfo {
	var minTTL uint32 = 0xffff_ffff
	var count uint32
	_ = walkTTLOffsets(s.wire, func(off int) {
		count++
		if ttl := binary.BigEndian.Uint32(s.wire[off : off+4]); ttl < minTTL {
			minTTL = ttl
		}
	})
	if count == 0 {
		return TTLInfo{}
	}
	return TTLInfo{MinimalTTL: minTTL, RecordCount: count}
}

// AgeTTL returns a caller-owned copy of the response with every non-OPT
// record's TTL reduced by elapsed whole seconds using saturating arithmetic
// (a record whose TTL is not greater than elapsed reaches 0, never negative).
// The OPT record, if any, is left byte-identical. The snapshot is never
// modified and the returned slice never aliases it.
func (s *ResponseSnapshot) AgeTTL(elapsed uint32) []byte {
	out := append([]byte(nil), s.wire...)
	_ = walkTTLOffsets(out, func(off int) {
		ttl := binary.BigEndian.Uint32(out[off : off+4])
		var newTTL uint32
		if elapsed >= ttl {
			newTTL = 0
		} else {
			newTTL = ttl - elapsed
		}
		binary.BigEndian.PutUint32(out[off:off+4], newTTL)
	})
	return out
}

// ReplaceTTL returns a caller-owned copy of the response with every non-OPT
// record's TTL set to ttl. The OPT record, if any, is left byte-identical.
// The snapshot is never modified and the returned slice never aliases it.
func (s *ResponseSnapshot) ReplaceTTL(ttl uint32) []byte {
	out := append([]byte(nil), s.wire...)
	_ = walkTTLOffsets(out, func(off int) {
		binary.BigEndian.PutUint32(out[off:off+4], ttl)
	})
	return out
}

// PatchResponseHeader returns a caller-owned copy of the response with the
// request ID written over the ID field and the Recursion-Available flag set.
// It freezes exactly the header mutations of the live raw-response fast path
// (pkg/server_handler EntryHandler.packRawResponse): the ID is a full
// big-endian write to wire bytes 0-1 (resp.Id = q.Id) and the RA bit is ORed
// into byte 3 bit 7 (resp.RecursionAvailable = true, then packed). Every
// other header field — QR, TC, RD, opcode, RCODE, and all four counts — and
// every byte after the header is left byte-identical. The snapshot is never
// modified and the returned slice never aliases it.
func (s *ResponseSnapshot) PatchResponseHeader(id uint16) []byte {
	out := append([]byte(nil), s.wire...)
	binary.BigEndian.PutUint16(out[0:2], id)
	out[3] |= flagRA
	return out
}

// walkTTLOffsets walks the response and calls visit at the wire offset of every
// non-OPT record's TTL field. The caller packet may be the snapshot or a
// byte-identical copy, so no call can observe partial patching. All errors are
// wrapped as ErrMalformedResponse; the header shape (length, QR bit) must have
// been checked before calling.
func walkTTLOffsets(packet []byte, visit func(ttlOffset int)) error {
	qd := int(binary.BigEndian.Uint16(packet[4:6]))
	recordCount := int(binary.BigEndian.Uint16(packet[6:8])) +
		int(binary.BigEndian.Uint16(packet[8:10])) +
		int(binary.BigEndian.Uint16(packet[10:12]))

	pos := dnsHeaderLen
	for i := 0; i < qd; i++ {
		next, ok := skipName(packet, pos)
		if !ok {
			return fmt.Errorf("%w: question name at offset %d", ErrMalformedResponse, pos)
		}
		pos = next + 4
		if pos > len(packet) {
			return fmt.Errorf("%w: question type/class truncated", ErrMalformedResponse)
		}
	}

	for i := 0; i < recordCount; i++ {
		next, ok := skipName(packet, pos)
		if !ok {
			return fmt.Errorf("%w: record name at offset %d", ErrMalformedResponse, pos)
		}
		fixedEnd := next + rrFixedLen
		if fixedEnd > len(packet) {
			return fmt.Errorf("%w: record fixed field truncated at offset %d", ErrMalformedResponse, next)
		}
		if rrtype := binary.BigEndian.Uint16(packet[next : next+2]); rrtype != dnsTypeOPT {
			visit(next + rrTTLOffset)
		}
		dataLen := int(binary.BigEndian.Uint16(packet[next+8 : next+10]))
		pos = fixedEnd + dataLen
		if pos > len(packet) {
			return fmt.Errorf("%w: record rdata truncated at offset %d", ErrMalformedResponse, fixedEnd)
		}
	}
	return nil
}

// skipName advances pos past one wire name and reports whether the name is
// legal: zero-length end, a bounded compression pointer with an in-packet
// target, or labels of at most 63 bytes. It does not follow compression
// pointers, so it cannot detect pointer loops and accepts a pointer whose
// target is any in-packet offset; this matches the current cache wire walk
// exactly. Name bounds are therefore exactly the byte/field bounds that
// rust/cache-core wire.rs enforces.
func skipName(packet []byte, pos int) (int, bool) {
	for {
		if pos >= len(packet) {
			return 0, false
		}
		label := packet[pos]
		switch {
		case label == 0:
			return pos + 1, true
		case label&0xc0 == 0xc0:
			if pos+1 >= len(packet) {
				return 0, false
			}
			target := int(label&0x3f)<<8 | int(packet[pos+1])
			if target >= len(packet) {
				return 0, false
			}
			return pos + 2, true
		case label&0xc0 != 0 || label > dnsMaxLabel:
			return 0, false
		default:
			pos += 1 + int(label)
			if pos > len(packet) {
				return 0, false
			}
		}
	}
}
