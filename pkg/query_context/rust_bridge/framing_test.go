package rust_bridge_test

import (
	"bytes"
	"encoding/binary"
	"errors"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context/rust_bridge"
	"github.com/miekg/dns"
)

// The framing contract is frozen from the live raw-response fast path
// (pkg/server_handler packRawResponse) for the case where no message path is
// taken (no RespOpt and the response fits the effective UDP size). UDP frames
// are passed through byte-identical; stream (TCP, UrlPath == "") gets a
// 2-byte big-endian length prefix; HTTP (UrlPath != "") is passed through
// without a prefix. The maximum stream length is dns.MaxMsgSize (65535);
// anything longer fails the framing call without producing output. UDP uses
// dns.MinMsgSize (512) as the floor for the advertised client size. The
// decoded-message truncation path is out of scope for this oracle.

func TestFrameResponseUDPPassesThroughWithoutPrefix(t *testing.T) {
	wire := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	out, err := s.FrameResponse(rust_bridge.FrameUDP)
	if err != nil {
		t.Fatalf("FrameResponse: %v", err)
	}
	if !bytes.Equal(out, wire) {
		t.Fatal("UDP framing changed the response bytes")
	}
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("FrameResponse modified the snapshot")
	}
	out[0] ^= 0xff
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("UDP frame result aliases the snapshot storage")
	}
}

func TestFrameResponseStreamAddsTwoByteBigEndianPrefix(t *testing.T) {
	wire := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	out, err := s.FrameResponse(rust_bridge.FrameStream)
	if err != nil {
		t.Fatalf("FrameResponse: %v", err)
	}
	if len(out) != 2+len(wire) {
		t.Fatalf("framed length = %d, want %d", len(out), 2+len(wire))
	}
	if got := binary.BigEndian.Uint16(out[:2]); got != uint16(len(wire)) {
		t.Fatalf("stream prefix = %d, want %d", got, len(wire))
	}
	if !bytes.Equal(out[2:], wire) {
		t.Fatal("stream framing changed the response body")
	}
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("FrameResponse modified the snapshot")
	}
}

func TestFrameResponseHTTPPassesThroughWithoutPrefix(t *testing.T) {
	// packRawResponse uses streamTransport = !FromUDP && UrlPath == "": a
	// non-UDP transport with a UrlPath (DoH) does not get a TCP prefix.
	wire := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	out, err := s.FrameResponse(rust_bridge.FrameHTTP)
	if err != nil {
		t.Fatalf("FrameResponse: %v", err)
	}
	if !bytes.Equal(out, wire) {
		t.Fatal("HTTP framing changed the response bytes")
	}
}

func TestFrameResponseHTTPPassesThroughOverMaxMsgSize(t *testing.T) {
	// The 65535 ceiling is a stream-length guard only: packRawResponse
	// applies it inside the streamTransport branch (UrlPath == ""). A DoH
	// response (UrlPath != "") has no TCP prefix, is copied through, and is
	// never length-checked, so FrameHTTP must pass through even when the
	// response exceeds dns.MaxMsgSize.
	big := bytes.Repeat([]byte{0x00}, 65529)
	wire := respWire([][]byte{big}, nil, nil)
	if len(wire) <= dns.MaxMsgSize {
		t.Fatalf("fixture is not oversized: %d", len(wire))
	}
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	before := append([]byte(nil), wire...)
	out, err := s.FrameResponse(rust_bridge.FrameHTTP)
	if err != nil {
		t.Fatalf("FrameResponse oversized HTTP: %v", err)
	}
	if !bytes.Equal(out, wire) {
		t.Fatal("oversized HTTP frame did not pass through byte-identical")
	}
	// Caller-owned: mutating the result must not affect the snapshot.
	out[0] ^= 0xff
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("oversized HTTP frame result aliases the snapshot storage")
	}
	if !bytes.Equal(wire, before) {
		t.Fatal("framing modified the caller input")
	}
	if !bytes.Equal(out[1:], wire[1:]) {
		t.Fatal("oversized HTTP frame changed bytes beyond the mutated ID byte")
	}
}

func TestFrameResponseStreamRejectsOverMaxMsgSize(t *testing.T) {
	// Stream max is dns.MaxMsgSize = 65535. One answer of 65529 bytes + 7
	// bytes of header + prefix exceeds it.
	big := bytes.Repeat([]byte{0x00}, 65529)
	wire := respWire([][]byte{big}, nil, nil)
	if len(wire) <= dns.MaxMsgSize {
		t.Fatalf("fixture is not oversized: %d", len(wire))
	}
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	before := append([]byte(nil), wire...)
	out, err := s.FrameResponse(rust_bridge.FrameStream)
	if !errors.Is(err, rust_bridge.ErrResponseTooLarge) {
		t.Fatalf("err = %v, want ErrResponseTooLarge", err)
	}
	if out != nil {
		t.Fatal("oversized stream produced a frame")
	}
	if !bytes.Equal(s.Wire(), wire) {
		t.Fatal("failed framing modified the snapshot")
	}
	if !bytes.Equal(wire, before) {
		t.Fatal("failed framing modified the caller input")
	}
}

func TestFrameResponseUDPRejectsOverMaxMsgSize(t *testing.T) {
	// The legacy 64 KiB ceiling (dns.MaxMsgSize) applies to framing as a whole:
	// the handler's Truncate path is a later concern, but a raw packet past the
	// ceiling is rejected instead of silently framing a >65535-byte datagram.
	// This is the same size gate the stream path enforces.
	big := bytes.Repeat([]byte{0x00}, 65529)
	wire := respWire([][]byte{big}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	if _, err := s.FrameResponse(rust_bridge.FrameUDP); !errors.Is(err, rust_bridge.ErrResponseTooLarge) {
		t.Fatalf("err = %v, want ErrResponseTooLarge", err)
	}
}

func TestFrameResponseUDPFreezesEffectiveSizeFloor(t *testing.T) {
	// The raw fast path compares the response against getValidUDPSize, which
	// floors the advertised client size at dns.MinMsgSize (512) before the
	// >-test that decides whether to truncate. The truncation decision itself
	// is the decoded-message path (out of scope); this oracle only exposes the
	// floor so a caller can reproduce the effective-size comparison. A
	// response larger than that floor is still framed as-is (pass-through).
	wire := respWire([][]byte{aRR(60, 192, 0, 2, 1)}, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	if got := s.EffectiveUDPSize(0); got != 512 {
		t.Fatalf("EffectiveUDPSize(0) = %d, want dns.MinMsgSize %d", got, 512)
	}
	// A response bigger than that floor (512 bytes) is below the legacy 64KiB
	// ceiling, so UDP framing still succeeds and passes the bytes through.
	// Build a 570+ byte response from 34 fixed 18-byte A records plus the
	// 30-byte header+question.
	answers := make([][]byte, 0, 34)
	for i := 0; i < 34; i++ {
		answers = append(answers, aRR(60, 192, 0, 2, byte(i)))
	}
	big := respWire(answers, nil, nil)
	if len(big) <= dns.MinMsgSize || len(big) >= dns.MaxMsgSize {
		t.Fatalf("fixture %d bytes must exceed MinMsgSize and fit under the ceiling", len(big))
	}
	s2, err := rust_bridge.NewResponseSnapshot(big)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	bigBefore := append([]byte(nil), big...)
	out, err := s2.FrameResponse(rust_bridge.FrameUDP)
	if err != nil {
		t.Fatalf("FrameResponse on %d-byte UDP: %v", len(big), err)
	}
	if !bytes.Equal(out, big) {
		t.Fatal("UDP frame of a >MinMsgSize response was not passed through")
	}
	if !bytes.Equal(big, bigBefore) {
		t.Fatal("framing modified the caller input")
	}
}

func TestFrameResponseZeroLengthResponseIsFramed(t *testing.T) {
	// An empty response (header only) is a valid frame; the walk accepts it.
	wire := respWire(nil, nil, nil)
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if err != nil {
		t.Fatalf("NewResponseSnapshot: %v", err)
	}
	out, err := s.FrameResponse(rust_bridge.FrameStream)
	if err != nil {
		t.Fatalf("FrameResponse: %v", err)
	}
	if got := binary.BigEndian.Uint16(out[:2]); got != uint16(len(wire)) {
		t.Fatalf("stream prefix = %d, want %d", got, len(wire))
	}
}

func TestFrameResponseIsNotAValidMessageInput(t *testing.T) {
	// A query is not a valid response snapshot (QR clear), so no framing can
	// be produced; construction fails before any output.
	wire := validQuery()
	s, err := rust_bridge.NewResponseSnapshot(wire)
	if !errors.Is(err, rust_bridge.ErrUnsupportedResponse) {
		t.Fatalf("err = %v, want ErrUnsupportedResponse", err)
	}
	if s != nil {
		t.Fatal("non-response produced a snapshot")
	}
}
