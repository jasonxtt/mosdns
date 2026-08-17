package rust_bridge_test

import (
	"bytes"
	"encoding/binary"
	"errors"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context/rust_bridge"
	"github.com/miekg/dns"
)

// The response fixtures are hand-packed like the query fixtures so each case
// isolates exactly one wire property. The frozen response TTL contract follows
// the current cache wire behavior (rust/cache-core wire.rs): the declared
// answer/authority/extra counts are walked as one record section, OPT records
// (type 41) are skipped for TTL purposes, and every other record's TTL is
// observed or patched on a copy.

// respWire packs a response with exactly one question (example.org A IN), QR
// set, and the given records. Each section entry is a complete wire RR.
func respWire(answers, authorities, extras [][]byte) []byte {
	w := hdr(0x81, 0, 1, uint16(len(answers)), uint16(len(authorities)), uint16(len(extras)))
	w = append(w, name("example", "org")...)
	w = append(w, 0x00, 0x01, 0x00, 0x01) // QTYPE A, QCLASS IN
	for _, sec := range [][][]byte{answers, authorities, extras} {
		for _, rr := range sec {
			w = append(w, rr...)
		}
	}
	return w
}

// ptr returns a two-byte compression pointer to off.
func ptr(off int) []byte { return []byte{0xc0 | byte(off>>8), byte(off)} }

// aRR packs an A record whose owner is a compression pointer to the question
// name at offset 12.
func aRR(ttl uint32, ip ...byte) []byte {
	rr := ptr(12)
	rr = append(rr, 0x00, 0x01, 0x00, 0x01) // A IN
	rr = append(rr, byte(ttl>>24), byte(ttl>>16), byte(ttl>>8), byte(ttl))
	rr = append(rr, 0x00, 0x04)
	return append(rr, ip...)
}

// unpack decodes wire with miekg/dns. Every valid fixture must round-trip so
// the readback asserts the oracle result at the wire level.
func unpack(t *testing.T, wire []byte) *dns.Msg {
	t.Helper()
	m := new(dns.Msg)
	if err := m.Unpack(wire); err != nil {
		t.Fatalf("fixture fails to unpack: %v", err)
	}
	return m
}

func TestNewResponseSnapshotAcceptsValidResponses(t *testing.T) {
	simple := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	mixed := respWire(
		[][]byte{aRR(60, 192, 0, 2, 1), aRR(120, 192, 0, 2, 2)},
		[][]byte{aRR(30, 198, 51, 100, 1)},
		[][]byte{optRR(1232, 0x0000_8000)},
	)
	empty := respWire(nil, nil, nil)
	onlyOpt := respWire(nil, nil, [][]byte{optRR(1232, 0x0000_8000)})
	nxdomain := respWire(nil, nil, nil)
	nxdomain[3] = 3 // RCODE NXDOMAIN

	// An answer with an uncompressed owner name exercises label walking.
	owner := name("www", "example", "org")
	uncompressed := owner
	uncompressed = append(uncompressed, 0x00, 0x01, 0x00, 0x01) // A IN
	uncompressed = append(uncompressed, 0x00, 0x00, 0x00, 0x3c) // ttl 60
	uncompressed = append(uncompressed, 0x00, 0x04)
	uncompressed = append(uncompressed, 192, 0, 2, 9)

	// qd=0 with a declared answer: the walk honors the declared counts instead
	// of assuming exactly one question, matching the current cache wire walk.
	noQuestion := hdr(0x81, 0, 0, 1, 0, 0)
	noQuestion = append(noQuestion, name("example", "org")...)
	noQuestion = append(noQuestion, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 1, 2, 3, 4)

	// A question name that is a self-referential in-packet pointer is accepted:
	// the cache wire walk never follows pointers, so its only pointer rule is
	// that the target lies inside the packet. Compression-loop detection is not
	// part of the frozen response TTL contract.
	selfPtrQuestion := hdr(0x81, 0, 1, 0, 0, 0)
	selfPtrQuestion = append(selfPtrQuestion, 0xc0, 0x0c) // target 12 = its own offset
	selfPtrQuestion = append(selfPtrQuestion, 0x00, 0x01, 0x00, 0x01)

	// Trailing bytes after the last declared record are ignored, matching the
	// live unpack path.
	trailing := append(respWire([][]byte{aRR(60, 1, 2, 3, 4)}, nil, nil), 0xff, 0x00)

	tests := []struct {
		name string
		wire []byte
	}{
		{name: "simple", wire: simple},
		{name: "mixed-sections-with-opt", wire: mixed},
		{name: "empty", wire: empty},
		{name: "only-opt", wire: onlyOpt},
		{name: "nxdomain", wire: nxdomain},
		{name: "uncompressed-owner", wire: respWire([][]byte{uncompressed}, nil, nil)},
		{name: "no-question-declared-answer", wire: noQuestion},
		{name: "self-pointer-question", wire: selfPtrQuestion},
		{name: "trailing-bytes", wire: trailing},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			before := append([]byte(nil), tt.wire...)
			s, err := rust_bridge.NewResponseSnapshot(tt.wire)
			if err != nil {
				t.Fatalf("NewResponseSnapshot: %v", err)
			}
			if s == nil {
				t.Fatal("NewResponseSnapshot returned nil snapshot")
			}
			if !bytes.Equal(s.Wire(), tt.wire) {
				t.Fatal("snapshot content does not match accepted input")
			}
			if got := s.RequiredResultLength(); got != len(tt.wire) {
				t.Fatalf("RequiredResultLength = %d, want %d", got, len(tt.wire))
			}
			if !bytes.Equal(tt.wire, before) {
				t.Fatal("NewResponseSnapshot modified the caller's input bytes")
			}
		})
	}
}

func TestNewResponseSnapshotRejectsMalformedResponses(t *testing.T) {
	questionLongLabel := hdr(0x81, 0, 1, 0, 0, 0)
	questionLongLabel = append(questionLongLabel, 0x40) // 64-byte label
	questionLongLabel = append(questionLongLabel, make([]byte, 64)...)
	questionLongLabel = append(questionLongLabel, 0x00, 0x01, 0x00, 0x01)

	questionOOBPointer := hdr(0x81, 0, 1, 0, 0, 0)
	questionOOBPointer = append(questionOOBPointer, 0xc0, 0x20) // target 32 past the packet end
	questionOOBPointer = append(questionOOBPointer, 0x00, 0x01, 0x00, 0x01)

	questionTruncated := hdr(0x81, 0, 1, 0, 0, 0)
	questionTruncated = append(questionTruncated, name("example", "org")...)
	questionTruncated = append(questionTruncated, 0x00, 0x01) // type only, class missing

	// Keywords: rdlength claims 200 bytes but the packet ends first.
	badRR := ptr(12)
	badRR = append(badRR, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0xc8)
	recordLengthOverflow := respWire([][]byte{badRR}, nil, nil)

	// Keyword: AN=2 claims a second answer that is not present.
	declaredMissing := respWire([][]byte{aRR(60, 1, 2, 3, 4)}, nil, nil)
	declaredMissing[6] = 2

	// A record owner with an illegal label prefix (0x80).
	badOwner := []byte{0x80}
	badOwner = append(badOwner, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 1, 2, 3, 4)
	badOwnerName := respWire([][]byte{badOwner}, nil, nil)

	tests := []struct {
		name string
		wire []byte
	}{
		{name: "empty", wire: nil},
		{name: "shorter-than-header", wire: []byte{1, 2, 3, 4, 5, 6}},
		{name: "header-only-no-question", wire: hdr(0x81, 0, 1, 0, 0, 0)},
		{name: "question-label-too-long", wire: questionLongLabel},
		{name: "question-pointer-out-of-range", wire: questionOOBPointer},
		{name: "question-truncated-type-class", wire: questionTruncated},
		{name: "record-rdlength-overflow", wire: recordLengthOverflow},
		{name: "declared-records-missing", wire: declaredMissing},
		{name: "record-owner-illegal-label", wire: badOwnerName},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			before := append([]byte(nil), tt.wire...)
			s, err := rust_bridge.NewResponseSnapshot(tt.wire)
			if !errors.Is(err, rust_bridge.ErrMalformedResponse) {
				t.Fatalf("err = %v, want ErrMalformedResponse", err)
			}
			if s != nil {
				t.Fatal("malformed input produced a snapshot")
			}
			if !bytes.Equal(tt.wire, before) {
				t.Fatal("NewResponseSnapshot modified the caller's input bytes")
			}
		})
	}
}

func TestNewResponseSnapshotRejectsNonResponse(t *testing.T) {
	// A structurally valid DNS message with QR clear (a query) is not a response.
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

func TestObserveTTLMinimalAndCountOverAllRecords(t *testing.T) {
	s, err := rust_bridge.NewResponseSnapshot(respWire(
		[][]byte{aRR(60, 192, 0, 2, 1), aRR(120, 192, 0, 2, 2)},
		[][]byte{aRR(30, 198, 51, 100, 1)},
		[][]byte{optRR(1232, 0x0000_8000)},
	))
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	got := s.ObserveTTL()
	if got.MinimalTTL != 30 {
		t.Fatalf("MinimalTTL = %d, want 30", got.MinimalTTL)
	}
	if got.RecordCount != 3 {
		t.Fatalf("RecordCount = %d, want 3", got.RecordCount)
	}
}

func TestObserveTTLNoRecords(t *testing.T) {
	for name, wire := range map[string][]byte{
		"empty":    respWire(nil, nil, nil),
		"only-opt": respWire(nil, nil, [][]byte{optRR(1232, 0x0000_8000)}),
		"nxdomain": setRcode(respWire(nil, nil, nil), dns.RcodeNameError),
		"servfail": setRcode(respWire(nil, nil, nil), dns.RcodeServerFailure),
	} {
		t.Run(name, func(t *testing.T) {
			s, err := rust_bridge.NewResponseSnapshot(wire)
			if err != nil {
				t.Fatalf("NewResponseSnapshot: %v", err)
			}
			got := s.ObserveTTL()
			if got.MinimalTTL != 0 {
				t.Fatalf("MinimalTTL = %d, want 0", got.MinimalTTL)
			}
			if got.RecordCount != 0 {
				t.Fatalf("RecordCount = %d, want 0", got.RecordCount)
			}
		})
	}
}

func TestAgeTTLReducesEachNonOPTRecord(t *testing.T) {
	wire := respWire(
		[][]byte{aRR(60, 192, 0, 2, 1), aRR(120, 192, 0, 2, 2)},
		[][]byte{aRR(30, 198, 51, 100, 1)},
		[][]byte{optRR(1232, 0x0000_8000)},
	)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	aged := s.AgeTTL(17)
	if len(aged) != s.RequiredResultLength() {
		t.Fatalf("aged length = %d, want %d", len(aged), s.RequiredResultLength())
	}
	m := unpack(t, aged)
	got := []uint32{
		m.Answer[0].Header().Ttl,
		m.Answer[1].Header().Ttl,
		m.Ns[0].Header().Ttl,
	}
	want := []uint32{43, 103, 13}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("aged TTL[%d] = %d, want %d", i, got[i], want[i])
		}
	}
	if opt := m.IsEdns0(); opt == nil || opt.Hdr.Ttl != 0x0000_8000 {
		t.Fatalf("OPT TTL was aged: %#v", opt)
	}
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("AgeTTL modified the snapshot")
	}
}

func TestAgeTTLSaturatesAtZero(t *testing.T) {
	// The wire contract saturates age at zero (design.md "saturating
	// arithmetic", matching rust/cache-core age_ttls). This differs from the
	// decoded-path dnsutils.SubtractTTL, which floors at 1.
	tests := []struct {
		ttl     uint32
		elapsed uint32
		want    uint32
	}{
		{ttl: 3, elapsed: 10, want: 0},
		{ttl: 5, elapsed: 5, want: 0},
		{ttl: 5, elapsed: 4, want: 1},
		{ttl: 7, elapsed: 0, want: 7},
	}
	for _, tt := range tests {
		s, err := rust_bridge.NewResponseSnapshot(respWire([][]byte{aRR(tt.ttl, 192, 0, 2, 1)}, nil, nil))
		if err != nil {
			t.Fatalf("NewResponseSnapshot: %v", err)
		}
		m := unpack(t, s.AgeTTL(tt.elapsed))
		if got := m.Answer[0].Header().Ttl; got != tt.want {
			t.Fatalf("AgeTTL(%d) on ttl %d = %d, want %d", tt.elapsed, tt.ttl, got, tt.want)
		}
	}
}

func TestAgeTTLZeroElapsedIsNoOp(t *testing.T) {
	wire := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	if aged := s.AgeTTL(0); !bytes.Equal(aged, wire) {
		t.Fatal("AgeTTL(0) changed the response")
	}
}

func TestReplaceTTLSetsEveryNonOPTTTL(t *testing.T) {
	s, err := rust_bridge.NewResponseSnapshot(respWire(
		[][]byte{aRR(60, 192, 0, 2, 1), aRR(120, 192, 0, 2, 2)},
		[][]byte{aRR(30, 198, 51, 100, 1)},
		[][]byte{optRR(1232, 0x0000_8000)},
	))
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	out := s.ReplaceTTL(5)
	m := unpack(t, out)
	if m.Answer[0].Header().Ttl != 5 || m.Answer[1].Header().Ttl != 5 || m.Ns[0].Header().Ttl != 5 {
		t.Fatalf("replaced TTLs not set to 5: answer=%d,%d ns=%d",
			m.Answer[0].Header().Ttl, m.Answer[1].Header().Ttl, m.Ns[0].Header().Ttl)
	}
	if opt := m.IsEdns0(); opt == nil || opt.Hdr.Ttl != 0x0000_8000 {
		t.Fatalf("OPT TTL was replaced: %#v", opt)
	}
}

func TestReplaceTTLZeroIsAllowed(t *testing.T) {
	s, err := rust_bridge.NewResponseSnapshot(respWire([][]byte{aRR(60, 1, 2, 3, 4)}, nil, nil))
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	m := unpack(t, s.ReplaceTTL(0))
	if m.Answer[0].Header().Ttl != 0 {
		t.Fatalf("replacement with 0 produced ttl %d", m.Answer[0].Header().Ttl)
	}
}

func TestResponseTTLResultsAreCallerOwnedCopies(t *testing.T) {
	wire := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}

	// Mutating any returned slice must not affect the snapshot.
	got := s.Wire()
	for i := range got {
		got[i] ^= 0xff
	}
	aged := s.AgeTTL(1)
	for i := range aged {
		aged[i] ^= 0xff
	}
	replaced := s.ReplaceTTL(1)
	for i := range replaced {
		replaced[i] ^= 0xff
	}
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("a returned result aliases the snapshot storage")
	}

	// Repeated calls are independent. Snapshot the second result, mutate the
	// first, and require the second to be unchanged while the first now differs.
	a1 := s.AgeTTL(1)
	a2 := s.AgeTTL(1)
	expected := append([]byte(nil), a2...)
	a1[0] ^= 0xff
	if !bytes.Equal(a2, expected) {
		t.Fatal("AgeTTL result aliases storage: mutating a1 changed a2")
	}
	if bytes.Equal(a1, a2) {
		t.Fatal("AgeTTL result aliases storage: a1 mutation is visible in a2")
	}
}

func TestAgeTTLCacheWireFixtureParity(t *testing.T) {
	// Bit-for-bit parity with the existing cache wire fixture
	// (rust/cache-core wire.rs: response_with_compressed_answer_and_opt), where
	// the answer TTL lives at fixed offset 35 and the OPT TTL at offset 50.
	// The oracle must age offset 35 and leave offset 50 byte-identical, exactly
	// like cache-core's age_ttls.
	packet := []byte{
		0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, 'e',
		'x', 'a', 'm', 'p', 'l', 'e', 0x03, 'o', 'r', 'g', 0x00, 0x00, 0x01, 0x00,
		0x01, 0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0x00, 0x04, 192, 0, 2, 1, 0x00,
		0x00, 0x29, 0x04, 0xd0, 0, 0, 0, 0, 0x00, 0x00,
	}
	binary.BigEndian.PutUint32(packet[35:39], 60)          // answer TTL
	binary.BigEndian.PutUint32(packet[50:54], 0x0000_8000) // OPT TTL flags (DO bit)

	s, err := rust_bridge.NewResponseSnapshot(packet)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	aged := s.AgeTTL(17)
	if got := binary.BigEndian.Uint32(aged[35:39]); got != 43 {
		t.Fatalf("age at cache fixture offset 35 = %d, want 43", got)
	}
	if got := aged[50 : 50+4]; !bytes.Equal(got, []byte{0x00, 0x00, 0x80, 0x00}) {
		t.Fatalf("OPT TTL at offset 50 = %x, want 00008000 (DO flags preserved)", got)
	}
	if bytes.Equal(aged, packet) {
		t.Fatal("aging left the response unchanged")
	}
	if !bytes.Equal(s.Wire(), packet) {
		t.Fatal("AgeTTL modified the snapshot")
	}
}

// setRcode returns a copy of wire with the low RCODE nibble set.
func setRcode(wire []byte, rcode int) []byte {
	out := append([]byte(nil), wire...)
	out[3] = out[3]&0xf0 | byte(rcode)
	return out
}
