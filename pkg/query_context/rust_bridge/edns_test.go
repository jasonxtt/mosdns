package rust_bridge_test

import (
	"bytes"
	"errors"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context/rust_bridge"
)

// optRR builds a wire-format OPT record: root name (0x00), type OPT (0x0029),
// class = advertised UDP payload size, TTL (DO bit), then the option list.
func optRR(udp uint16, ttl uint32, options ...[]byte) []byte {
	b := []byte{0x00, 0x00, 0x29, byte(udp >> 8), byte(udp), byte(ttl >> 24), byte(ttl >> 16), byte(ttl >> 8), byte(ttl)}
	rd := []byte{}
	for _, o := range options {
		rd = append(rd, o...)
	}
	b = append(b, byte(len(rd)>>8), byte(len(rd)))
	b = append(b, rd...)
	return b
}

// ecsOpt builds an EDNS0 client-subnet option: code 8, length, then family,
// source netmask, scope, and the (netmask-masked) address bytes.
func ecsOpt(family uint16, mask, scope uint8, addr ...byte) []byte {
	b := []byte{0x00, 0x08, 0x00, 0x00, byte(family >> 8), byte(family), mask, scope}
	b = append(b, addr...)
	b[2] = byte((4 + len(addr)) >> 8)
	b[3] = byte(4 + len(addr))
	return b
}

func queryWithExtra(extra []byte) []byte {
	w := hdr(0x01, 0, 1, 0, 0, 1)
	w = append(w, name("example", "org")...)
	w = append(w, 0x00, 0x01, 0x00, 0x01)
	return append(w, extra...)
}

func TestEDNSAbsentWithoutOPT(t *testing.T) {
	s, err := rust_bridge.NewSnapshot(validQuery())
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}
	e := s.EDNS()
	if e.HasOPT || e.UDPSize != 0 || e.DO || e.ECS != nil {
		t.Fatalf("absent EDNS mismatch: %+v", e)
	}
}

func TestEDNSPresentUDPAndDO(t *testing.T) {
	s, err := rust_bridge.NewSnapshot(queryWithExtra(optRR(1232, 0x8000)))
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}
	e := s.EDNS()
	if !e.HasOPT {
		t.Fatal("EDNS presence not reported")
	}
	if e.UDPSize != 1232 {
		t.Fatalf("UDPSize = %d, want 1232", e.UDPSize)
	}
	if !e.DO {
		t.Fatal("DO bit not reported")
	}
	if e.ECS != nil {
		t.Fatalf("unexpected ECS: %+v", e.ECS)
	}
}

func TestEDNSECSIPv4(t *testing.T) {
	s, err := rust_bridge.NewSnapshot(queryWithExtra(optRR(1232, 0, ecsOpt(1, 24, 0, 1, 2, 3))))
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}
	ecs := s.EDNS().ECS
	if ecs == nil {
		t.Fatal("ECS not extracted")
	}
	if ecs.Family != 1 || ecs.SourceNetmask != 24 || ecs.SourceScope != 0 {
		t.Fatalf("ECS fields mismatch: %+v", ecs)
	}
	if !bytes.Equal(ecs.Address, []byte{1, 2, 3, 0}) {
		t.Fatalf("ECS address = %x, want 01020300", ecs.Address)
	}
}

func TestEDNSECSIPv6(t *testing.T) {
	s, err := rust_bridge.NewSnapshot(queryWithExtra(optRR(1232, 0, ecsOpt(2, 56, 0, 1, 2, 3, 4, 5, 6, 7))))
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}
	ecs := s.EDNS().ECS
	if ecs == nil {
		t.Fatal("ECS not extracted")
	}
	if ecs.Family != 2 || ecs.SourceNetmask != 56 || ecs.SourceScope != 0 {
		t.Fatalf("ECS fields mismatch: %+v", ecs)
	}
	want := []byte{1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0}
	if !bytes.Equal(ecs.Address, want) {
		t.Fatalf("ECS address = %x, want %x", ecs.Address, want)
	}
}

func TestEDNSECSWithDOAndFullMask(t *testing.T) {
	s, err := rust_bridge.NewSnapshot(queryWithExtra(optRR(4096, 0x8000, ecsOpt(1, 32, 0, 10, 0, 0, 1))))
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}
	e := s.EDNS()
	if e.UDPSize != 4096 || !e.DO {
		t.Fatalf("UDP/DO mismatch: udp=%d do=%v", e.UDPSize, e.DO)
	}
	if e.ECS == nil || !bytes.Equal(e.ECS.Address, []byte{10, 0, 0, 1}) {
		t.Fatalf("ECS mismatch: %+v", e.ECS)
	}
}

func TestEDNSResultIsACopy(t *testing.T) {
	s, err := rust_bridge.NewSnapshot(queryWithExtra(optRR(1232, 0, ecsOpt(1, 24, 0, 1, 2, 3))))
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}
	r1 := s.EDNS()
	r1.ECS.Address[0] = 0xff
	r1.UDPSize = 1
	r1.DO = true
	r2 := s.EDNS()
	if r2.ECS == nil || !bytes.Equal(r2.ECS.Address, []byte{1, 2, 3, 0}) {
		t.Fatal("EDNS result aliases snapshot storage")
	}
	if r2.UDPSize != 1232 || r2.DO {
		t.Fatal("EDNS scalar fields aliased")
	}
}

func TestEDNSMalformedOPTRejectedWithoutModifyingInput(t *testing.T) {
	// OPT rdlength claims 8 bytes but none are present.
	truncated := optRR(1232, 0)
	truncated[9], truncated[10] = 0x00, 0x08
	// ECS option length claims 100 bytes but only 7 follow. The option data
	// starts at offset 11 (owner/type/class/ttl/rdlength is 11 bytes); the
	// option length field is at offsets 13-14, not the family field at 15-16.
	badLen := optRR(1232, 0, ecsOpt(1, 24, 0, 1, 2, 3))
	badLen[13], badLen[14] = 0x00, 0x64
	// ECS option length 2: shorter than family/netmask/scope.
	tooShort := optRR(1232, 0, []byte{0x00, 0x08, 0x00, 0x02, 0x00, 0x01})

	for name, extra := range map[string][]byte{
		"rdlength-overflow":         truncated,
		"option-length-overflow":    badLen,
		"option-shorter-than-fixed": tooShort,
	} {
		t.Run(name, func(t *testing.T) {
			wire := queryWithExtra(extra)
			before := append([]byte(nil), wire...)
			s, err := rust_bridge.NewSnapshot(wire)
			if !errors.Is(err, rust_bridge.ErrMalformedQuery) {
				t.Fatalf("err = %v, want ErrMalformedQuery", err)
			}
			if s != nil {
				t.Fatal("malformed OPT produced a snapshot")
			}
			if !bytes.Equal(wire, before) {
				t.Fatal("NewSnapshot modified the caller's input bytes")
			}
		})
	}
}

func TestEDNSUnknownOptionDoesNotProduceECS(t *testing.T) {
	// Unknown option code 0x20 (miekg parses it as EDNS0_LOCAL) alongside a
	// real ECS option must not be misreported as ECS.
	s, err := rust_bridge.NewSnapshot(queryWithExtra(optRR(1232, 0, []byte{0x00, 0x20, 0x00, 0x00}, ecsOpt(1, 24, 0, 1, 2, 3))))
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}
	e := s.EDNS()
	if !e.HasOPT {
		t.Fatal("EDNS presence not reported")
	}
	if e.ECS == nil || e.ECS.Family != 1 || e.ECS.SourceNetmask != 24 {
		t.Fatalf("ECS not extracted from mixed options: %+v", e.ECS)
	}
	if !bytes.Equal(e.ECS.Address, []byte{1, 2, 3, 0}) {
		t.Fatalf("ECS address = %x, want 01020300", e.ECS.Address)
	}
}

func TestEDNSNonOPTExtraTreatedAsAbsent(t *testing.T) {
	// A single non-OPT extra record (TXT "abc") is accepted by the header
	// validation and reports EDNS as absent.
	txt := []byte{
		0x00,       // root owner
		0x00, 0x10, // type TXT
		0x00, 0x01, // class IN
		0x00, 0x00, 0x00, 0x00, // ttl
		0x00, 0x04, // rdlength
		0x03, 'a', 'b', 'c',
	}
	s, err := rust_bridge.NewSnapshot(queryWithExtra(txt))
	if err != nil {
		t.Fatalf("NewSnapshot: %v", err)
	}
	e := s.EDNS()
	if e.HasOPT || e.ECS != nil {
		t.Fatalf("non-OPT extra reported as EDNS: %+v", e)
	}
}

func TestEDNSMalformedNonOPTExtraRejected(t *testing.T) {
	// TXT RDLENGTH claims 20 bytes but only three bytes of RDATA exist.
	malformed := []byte{
		0x00,       // root owner
		0x00, 0x10, // type TXT
		0x00, 0x01, // class IN
		0x00, 0x00, 0x00, 0x00, // ttl
		0x00, 0x14, // rdlength 20
		0x03, 'a', 'b', 'c',
	}
	if _, err := rust_bridge.NewSnapshot(queryWithExtra(malformed)); !errors.Is(err, rust_bridge.ErrMalformedQuery) {
		t.Fatalf("err = %v, want ErrMalformedQuery", err)
	}
}
