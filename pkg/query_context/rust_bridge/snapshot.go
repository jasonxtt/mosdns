/*
 * Copyright (C) 2020-2022, IrineSistiana
 *
 * This file is part of mosdns.
 *
 * mosdns is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * mosdns is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

// Package rust_bridge is the Go-side seam for the experimental query snapshot
// contract. It owns the snapshot encoding and the Go oracle that later slices
// compare against the Rust implementation.
//
// This slice establishes the ownership contract, the query header/question
// validation contract, and EDNS/DO/ECS extraction. Snapshots copy their input
// bytes, inspection results are written into caller-owned buffers, and a
// failed call never modifies the snapshot input or the caller's result
// buffer. NewSnapshot accepts only a wire-legal, non-response QUERY with
// exactly one question, no answer or authority records, and at most one extra
// record; see ErrMalformedQuery and ErrUnsupportedQuery for the frozen
// classification. EDNS returns a caller-owned copy of the OPT presence,
// advertised UDP size, DO bit, and the first ECS option's family, netmask,
// scope, and masked address.
package rust_bridge

import (
	"encoding/binary"
	"errors"
	"fmt"
	"net"

	"github.com/miekg/dns"
)

// ErrResultTooSmall reports that the caller's result buffer is shorter than
// the snapshot's required result length. The reported byte count is the
// required length; nothing has been written to the buffer.
var ErrResultTooSmall = errors.New("query snapshot result buffer too small")

// ErrMalformedQuery reports a wire-level defect: the message is shorter than
// a DNS header, the question name has an illegal label or compression
// pointer, or the question's type/class fields are truncated.
var ErrMalformedQuery = errors.New("malformed DNS query")

// ErrUnsupportedQuery reports a structurally valid message that is not a
// supported query: the QR bit is set, the opcode is not QUERY, the question
// count is not exactly one, or answer/authority/extra sections are present
// beyond the single allowed extra record.
var ErrUnsupportedQuery = errors.New("unsupported DNS query")

// Snapshot is an immutable copy of the DNS query wire bytes handed to
// NewSnapshot. It never retains or aliases the caller's input slice.
type Snapshot struct {
	wire []byte
	edns EDNSInfo
}

// ECSInfo is the decoded EDNS0 client-subnet option. Address is a
// caller-owned copy: 4 bytes for family 1 (IPv4) and 16 bytes for family 2
// (IPv6), zero-padded on the right when the source netmask covers fewer
// bytes than the family's full width.
type ECSInfo struct {
	Family        uint16
	SourceNetmask uint8
	SourceScope   uint8
	Address       []byte
}

// EDNSInfo is the decoded EDNS0 state of a query snapshot. UDPSize is the
// advertised UDP payload size as sent, without any clamping. DO is the
// DNSSEC OK bit. ECS is nil when the OPT carries no client-subnet option.
// When HasOPT is false the remaining fields are zero and ECS is nil.
type EDNSInfo struct {
	HasOPT  bool
	UDPSize uint16
	DO      bool
	ECS     *ECSInfo
}

// EDNS returns a caller-owned copy of the snapshot's EDNS state. Mutating
// the returned struct or its ECS address never affects the snapshot.
func (s *Snapshot) EDNS() EDNSInfo {
	out := s.edns
	if out.ECS != nil {
		ecs := *out.ECS
		ecs.Address = append([]byte(nil), out.ECS.Address...)
		out.ECS = &ecs
	}
	return out
}

// NewSnapshot validates queryWire as a supported DNS query and copies it into
// a new immutable Snapshot. The caller keeps ownership of queryWire and may
// reuse or mutate it freely afterwards.
//
// Validation follows the existing Go behavior: the header must be a non-
// response with opcode QUERY and exactly one question, with no answer or
// authority records and at most one extra record (pkg/server_handler
// EntryHandler), and the question name must parse under miekg/dns
// UnpackDomainName. Unlike the lenient Msg.Unpack question loop, a question
// whose type or class fields are truncated is rejected as malformed. Trailing
// bytes after the question are tolerated, matching the live unpack path.
//
// When the single extra record is present it is unpacked with miekg/dns; a
// malformed or truncated record (including an OPT with an illegal option
// length) fails as ErrMalformedQuery. If it is an OPT record, its EDNS/DO/ECS
// state is extracted and available through EDNS; a non-OPT extra record is
// accepted and reports EDNS as absent.
//
// On failure no snapshot is created and queryWire is never modified.
func NewSnapshot(queryWire []byte) (*Snapshot, error) {
	if len(queryWire) < 12 {
		return nil, fmt.Errorf("%w: %d bytes is shorter than a DNS header", ErrMalformedQuery, len(queryWire))
	}
	if queryWire[2]&0x80 != 0 {
		return nil, fmt.Errorf("%w: QR bit set", ErrUnsupportedQuery)
	}
	if opcode := queryWire[2] >> 3 & 0x0f; opcode != dns.OpcodeQuery {
		return nil, fmt.Errorf("%w: opcode %d", ErrUnsupportedQuery, opcode)
	}
	if qd := binary.BigEndian.Uint16(queryWire[4:6]); qd != 1 {
		return nil, fmt.Errorf("%w: question count %d, want 1", ErrUnsupportedQuery, qd)
	}
	if an := binary.BigEndian.Uint16(queryWire[6:8]); an > 0 {
		return nil, fmt.Errorf("%w: answer section is not empty", ErrUnsupportedQuery)
	}
	if ns := binary.BigEndian.Uint16(queryWire[8:10]); ns > 0 {
		return nil, fmt.Errorf("%w: authority section is not empty", ErrUnsupportedQuery)
	}
	ar := binary.BigEndian.Uint16(queryWire[10:12])
	if ar > 1 {
		return nil, fmt.Errorf("%w: %d extra records, want at most 1", ErrUnsupportedQuery, ar)
	}
	_, off, err := dns.UnpackDomainName(queryWire, 12)
	if err != nil {
		return nil, fmt.Errorf("%w: question name: %v", ErrMalformedQuery, err)
	}
	if off+4 > len(queryWire) {
		return nil, fmt.Errorf("%w: question type/class truncated", ErrMalformedQuery)
	}
	edns := EDNSInfo{}
	if ar == 1 {
		rr, _, err := dns.UnpackRR(queryWire, off+4)
		if err != nil {
			return nil, fmt.Errorf("%w: extra record: %v", ErrMalformedQuery, err)
		}
		if opt, ok := rr.(*dns.OPT); ok {
			edns.HasOPT = true
			edns.UDPSize = opt.UDPSize()
			edns.DO = opt.Do()
			for _, o := range opt.Option {
				if ecs, ok := o.(*dns.EDNS0_SUBNET); ok {
					// The first ECS option wins, matching the existing
					// Go plugin loops over OPT options.
					addr := append(net.IP(nil), ecs.Address...)
					if ecs.Family == 1 {
						if v4 := addr.To4(); v4 != nil {
							addr = v4
						}
					}
					edns.ECS = &ECSInfo{
						Family:        ecs.Family,
						SourceNetmask: ecs.SourceNetmask,
						SourceScope:   ecs.SourceScope,
						Address:       addr,
					}
					break
				}
			}
		}
	}
	return &Snapshot{wire: append([]byte(nil), queryWire...), edns: edns}, nil
}

// Wire returns a caller-owned copy of the snapshot's query wire bytes.
func (s *Snapshot) Wire() []byte {
	return append([]byte(nil), s.wire...)
}

// RequiredResultLength returns the exact number of bytes a successful Inspect
// call writes.
func (s *Snapshot) RequiredResultLength() int {
	return len(s.wire)
}

// Inspect writes the snapshot's inspection result into the caller-owned
// result buffer and returns the number of bytes written.
//
// If result is shorter than RequiredResultLength, Inspect returns
// ErrResultTooSmall together with the required length and does not modify
// result or the snapshot. Bytes past the written prefix are never touched.
func (s *Snapshot) Inspect(result []byte) (int, error) {
	if len(result) < len(s.wire) {
		return len(s.wire), ErrResultTooSmall
	}
	copy(result, s.wire)
	return len(s.wire), nil
}
