// Response framing oracle contract.
//
// This file freezes the transport framing decisions of the live raw-response
// fast path (pkg/server_handler EntryHandler.packRawResponse) for the case
// where no message path is taken: no RespOpt hat to append, and for UDP the
// response fits the effective client size. UDP frames are passed through
// byte-identical (no length prefix); stream/TCP (UrlPath == "") gets a
// two-byte big-endian length prefix and rejects a response longer than
// dns.MaxMsgSize (65535); HTTP/DNS-over-HTTP (UrlPath != "") is passed
// through without a prefix. The udpFloor of 512 matches getValidUDPSize's
// clamp of the advertised client size; the truncation decision for oversized
// UDP is the decoded-message path and is deliberately out of this oracle's
// scope (see the framing_test.go contract note).
//
// Like the other response helpers, the input is the immutable snapshot, the
// output is a caller-owned copy, and any failure (oversized stream or UDP)
// happens before any output exists. The snapshot is never modified.
package rust_bridge

import (
	"encoding/binary"
	"errors"
	"fmt"
)

const (
	// dnsMaxMsgSize is the classic 64 KiB ceiling (dns.MaxMsgSize). Stream
	// framing rejects a response longer than this because the two-byte prefix
	// cannot encode it; UDP framing applies the same ceiling as a guard.
	dnsMaxMsgSize = 65535

	// udpMinSize mirrors dns.MinMsgSize, the floor the handler applies to the
	// client's advertised UDP payload size before comparing a response against
	// it. It is exposed through EffectiveUDPSize.
	udpMinSize = 512
)

// ErrResponseTooLarge reports a raw response too long to frame: a stream or
// UDP response longer than dns.MaxMsgSize cannot be represented by the
// transport framing. No output is produced.
var ErrResponseTooLarge = errors.New("DNS response too large to frame")

// FrameMode selects the transport framing for FrameResponse.
type FrameMode uint8

const (
	// FrameUDP produces a UDP frame: the response bytes are passed through
	// unchanged, without a length prefix.
	FrameUDP FrameMode = iota

	// FrameStream produces a stream (TCP) frame: two big-endian bytes encode
	// the response length, then the response bytes follow.
	FrameStream

	// FrameHTTP produces a DNS-over-HTTP body: the response bytes are passed
	// through unchanged, without a length prefix.
	FrameHTTP
)

// EffectiveUDPSize clamps an advertised UDP payload size to at least
// dns.MinMsgSize (512), matching pkg/server_handler getValidUDPSize. This is
// the floor used to decide whether a UDP response needs truncation on the
// decoded-message path; truncation itself is out of this oracle's scope.
func (s *ResponseSnapshot) EffectiveUDPSize(advertised uint16) int {
	if advertised < udpMinSize {
		return int(udpMinSize)
	}
	return int(advertised)
}

// FrameResponse returns a caller-owned copy of the response framed for mode,
// mirroring the transport decision of the raw fast path (packRawResponse).
// FrameUDP and FrameHTTP pass the response through unchanged; FrameStream
// prefixes it with a two-byte big-endian length. The stream ceiling applies
// only to FrameStream (whose two-byte prefix cannot encode a longer length)
// and, with the same guard, to FrameUDP. FrameHTTP (DoH, UrlPath != "") is
// the streamTransport=false branch: it is copied through without any length
// check, so an oversized body still succeeds byte-identical. Every failure
// produces no output. The snapshot is never modified and the returned slice
// never aliases it.
func (s *ResponseSnapshot) FrameResponse(mode FrameMode) ([]byte, error) {
	switch mode {
	case FrameStream, FrameUDP:
		if len(s.wire) > dnsMaxMsgSize {
			return nil, fmt.Errorf("%w: %d bytes", ErrResponseTooLarge, len(s.wire))
		}
		if mode == FrameStream {
			out := make([]byte, 2+len(s.wire))
			binary.BigEndian.PutUint16(out[:2], uint16(len(s.wire)))
			copy(out[2:], s.wire)
			return out, nil
		}
		return append([]byte(nil), s.wire...), nil
	case FrameHTTP:
		return append([]byte(nil), s.wire...), nil
	default:
		return nil, fmt.Errorf("unknown framing mode %d", mode)
	}
}
