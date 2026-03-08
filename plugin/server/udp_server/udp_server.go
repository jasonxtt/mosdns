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

package udp_server

import (
	"context"
	"encoding/binary"
	"fmt"
	"hash/maphash"
	"net"
	"net/netip"
	"sync"
	"sync/atomic"
	"time"
	"unsafe"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/server"
	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
	"github.com/IrineSistiana/mosdns/v5/plugin/server/server_utils"
	"github.com/miekg/dns"
	"go.uber.org/zap"
)

const (
	PluginType  = "udp_server"
	cacheSize   = 65536
	cacheMask   = cacheSize - 1
	internalTTL = 5
	clientTTL   = 10
)

var maphashSeed = maphash.MakeSeed()

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })
}

type Args struct {
	Entry       string `yaml:"entry"`
	Listen      string `yaml:"listen"`
	EnableAudit bool   `yaml:"enable_audit"`
}

func (a *Args) init() {
	utils.SetDefaultString(&a.Listen, "127.0.0.1:53")
}

type UdpServer struct {
	args *Args
	c    net.PacketConn
}

func (s *UdpServer) Close() error {
	return s.c.Close()
}

type SwitchPlugin interface{ GetValue() string }
type DomainMapperPlugin interface{ 
	FastMatch(qname string) ([]uint8, string, bool) 
	GetRunBit() uint8
}
type IPSetPlugin interface{ Match(addr netip.Addr) bool }

type fastCacheItem struct {
	expire    int64
	resp      []byte
	updating  uint32
	domainSet string
}

type fastCache struct {
	m [cacheSize]atomic.Pointer[fastCacheItem]
}

func newFastCache() *fastCache { return &fastCache{} }

func (fc *fastCache) GetOrUpdating(hash uint64, reqLen int, buf []byte) (int, int, uint64, string) {
	ptr := fc.m[hash&cacheMask].Load()
	if ptr == nil {
		return server.FastActionContinue, 0, 0, ""
	}

	now := time.Now().Unix()
	if now > atomic.LoadInt64(&ptr.expire) {
		if atomic.CompareAndSwapUint32(&ptr.updating, 0, 1) {
			return server.FastActionContinue, 0, 0, ""
		}
	}

	if ptr.resp != nil {
		respLen := len(ptr.resp)
		txid0, txid1 := buf[0], buf[1]
		copy(buf, ptr.resp)
		buf[0], buf[1] = txid0, txid1
		return server.FastActionReply, respLen, 0, ptr.domainSet
	}
	return server.FastActionContinue, 0, 0, ""
}

func (fc *fastCache) Store(qname string, qtype uint16, resp []byte, dset string) {
	h := maphash.String(maphashSeed, qname) ^ uint64(qtype)
	
	bakedResp := make([]byte, len(resp))
	copy(bakedResp, resp)
	offsets := findTTLOffsets(bakedResp)
	for _, off := range offsets {
		if off+4 <= len(bakedResp) {
			binary.BigEndian.PutUint32(bakedResp[off:off+4], uint32(clientTTL))
		}
	}

	item := &fastCacheItem{
		resp:      bakedResp,
		expire:    time.Now().Add(internalTTL * time.Second).Unix(),
		updating:  0,
		domainSet: dset,
	}
	fc.m[h&cacheMask].Store(item)
}

type fastHandler struct {
	next server.Handler
	fc   *fastCache
	dm   DomainMapperPlugin
	sw   SwitchPlugin
}

func (h *fastHandler) Handle(ctx context.Context, q *dns.Msg, meta server.QueryMeta, pack func(*dns.Msg) (*[]byte, error)) *[]byte {
	payload := h.next.Handle(ctx, q, meta, pack)
	
	if h.sw != nil && h.sw.GetValue() != "A" {
		return payload
	}

	if payload != nil && (meta.PreFastFlags&(1<<39)) == 0 && q.Opcode == dns.OpcodeQuery && len(q.Question) > 0 {
		var dsetName string
		if h.dm != nil {
			_, dsetName, _ = h.dm.FastMatch(q.Question[0].Name)
		}
		h.fc.Store(q.Question[0].Name, q.Question[0].Qtype, *payload, dsetName)
	}
	return payload
}

func Init(bp *coremain.BP, args any) (any, error) {
	a := args.(*Args)
	a.init()
	return StartServer(bp, a)
}

func StartServer(bp *coremain.BP, args *Args) (*UdpServer, error) {
	dh, err := server_utils.NewHandler(bp, args.Entry, args.EnableAudit)
	if err != nil {
		return nil, fmt.Errorf("failed to init dns handler, %w", err)
	}

	var dm DomainMapperPlugin
	if p := bp.M().GetPlugin("unified_matcher1"); p != nil {
		dm, _ = p.(DomainMapperPlugin)
	}

	var sw15 SwitchPlugin
	if p := bp.M().GetPlugin("switch15"); p != nil {
		sw15, _ = p.(SwitchPlugin)
	}

	fc := newFastCache()
	wrappedHandler := &fastHandler{next: dh, fc: fc, dm: dm, sw: sw15}
	fastBypass := buildFastBypass(bp, fc)

	socketOpt := server_utils.ListenerSocketOpts{
		SO_REUSEPORT: true,
		SO_RCVBUF:    2 * 1024 * 1024, 
	}
	lc := net.ListenConfig{Control: server_utils.ListenerControl(socketOpt)}
	c, err := lc.ListenPacket(context.Background(), "udp", args.Listen)
	if err != nil {
		return nil, fmt.Errorf("failed to create socket, %w", err)
	}
	bp.L().Info("udp server started with fixed fast-path", zap.Stringer("addr", c.LocalAddr()))

	go func() {
		defer c.Close()
		err := server.ServeUDP(c.(*net.UDPConn), wrappedHandler, server.UDPServerOpts{
			Logger:     bp.L(),
			FastBypass: fastBypass,
		})
		bp.M().GetSafeClose().SendCloseSignal(err)
	}()
	return &UdpServer{args: args, c: c}, nil
}

func buildFastBypass(bp *coremain.BP, fc *fastCache) func(int, []byte, netip.AddrPort) (int, int, uint64, string) {
	var once sync.Once
	var sw15, sw5, sw6, sw1, sw7, sw2, sw12 SwitchPlugin
	var dm DomainMapperPlugin
	var ipSet IPSetPlugin

	return func(reqLen int, buf []byte, remoteAddr netip.AddrPort) (int, int, uint64, string) {
		once.Do(func() {
			if p := bp.M().GetPlugin("switch15"); p != nil { sw15, _ = p.(SwitchPlugin) }
			if p := bp.M().GetPlugin("switch5"); p != nil { sw5, _ = p.(SwitchPlugin) }
			if p := bp.M().GetPlugin("switch6"); p != nil { sw6, _ = p.(SwitchPlugin) }
			if p := bp.M().GetPlugin("switch1"); p != nil { sw1, _ = p.(SwitchPlugin) }
			if p := bp.M().GetPlugin("switch7"); p != nil { sw7, _ = p.(SwitchPlugin) }
			if p := bp.M().GetPlugin("switch2"); p != nil { sw2, _ = p.(SwitchPlugin) }
			if p := bp.M().GetPlugin("switch12"); p != nil { sw12, _ = p.(SwitchPlugin) }
			if p := bp.M().GetPlugin("unified_matcher1"); p != nil { dm, _ = p.(DomainMapperPlugin) }
			if p := bp.M().GetPlugin("client_ip"); p != nil { ipSet, _ = p.(IPSetPlugin) }
		})

		if sw15 == nil || sw15.GetValue() != "A" { return server.FastActionContinue, 0, 0, "" }
		if reqLen < 12 { return server.FastActionContinue, 0, 0, "" }

		// Phase 1: Protocol Blocking (QType 6, 12, 65) - Highest Priority
		qtypeOff := 12
		for qtypeOff < reqLen {
			l := int(buf[qtypeOff])
			if l == 0 { qtypeOff++; break }
			if l&0xC0 == 0xC0 { qtypeOff += 2; break }
			qtypeOff += l + 1
		}
		if qtypeOff+2 > reqLen { return server.FastActionContinue, 0, 0, "" }
		qtype := binary.BigEndian.Uint16(buf[qtypeOff : qtypeOff+2])

		if qtype == 6 || qtype == 12 || qtype == 65 {
			if sw5 != nil && sw5.GetValue() == "A" { return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 0), 0, "" }
		}
		if qtype == 28 {
			if sw6 != nil && sw6.GetValue() == "A" { return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 0), 0, "" }
		}

		// Phase 2: Domain Parsing (Restore Trailing Dot Logic)
		offset := 12
		var nameBuf [256]byte
		nameLen := 0
		for offset < reqLen {
			l := int(buf[offset])
			if l == 0 {
				offset++
				if nameLen == 0 { nameBuf[0] = '.'; nameLen = 1 }
				break
			}
			if l&0xC0 == 0xC0 { return server.FastActionContinue, 0, 0, "" }
			offset++
			if offset+l > reqLen || nameLen+l+1 > 256 { return server.FastActionContinue, 0, 0, "" }
			copy(nameBuf[nameLen:], buf[offset:offset+l])
			nameLen += l
			nameBuf[nameLen] = '.' // Restore: Always add dot after label
			nameLen++
			offset += l
		}
		qname := unsafe.String(&nameBuf[0], nameLen)

		// Phase 3: Domain Set Matching
		var marks uint64
		var dset string
		if dm != nil {
			marks |= (1 << dm.GetRunBit())
			if mList, dsName, match := dm.FastMatch(qname); match {
				for _, v := range mList {
					if v < 64 { marks |= (1 << v) }
				}
				dset = dsName
			}
		}

		// Phase 4: Absolute Rejections (Mark 1, 2, 3, 5) - Bypass Cache
		if sw1 != nil && sw1.GetValue() == "A" {
			if (marks & (1 << 1)) != 0 { return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 3), 0, "" }
			if (marks & (1 << 2)) != 0 && qtype == 1 { return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 0), 0, "" }
			if (marks & (1 << 3)) != 0 && qtype == 28 { return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 0), 0, "" }
		}
		if sw7 != nil && sw7.GetValue() == "A" {
			if (marks & (1 << 5)) != 0 { return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 3), 0, "" }
		}

		// Phase 5: Passthrough Routing (Mark 6, 39) - Bypass Cache
		ipMatch := false
		if ipSet != nil {
			ipMatch = ipSet.Match(remoteAddr.Addr().Unmap())
			marks |= (1 << 48)
		}
		sw2Val, sw12Val := "", ""
		if sw2 != nil { sw2Val = sw2.GetValue() }
		if sw12 != nil { sw12Val = sw12.GetValue() }

		if (sw2Val == "A" && sw12Val == "B" && !ipMatch) || (sw2Val == "B" && sw12Val == "A" && ipMatch) {
			marks |= (1 << 39)
		}

		if (marks & (1 << 6)) != 0 || (marks & (1 << 39)) != 0 {
			return server.FastActionContinue, 0, marks, dset
		}

		// Phase 6: Normal Cache Logic
		hKey := maphash.String(maphashSeed, qname) ^ uint64(qtype)
		action, rLen, _, ds := fc.GetOrUpdating(hKey, reqLen, buf)
		if action == server.FastActionReply { return action, rLen, 0, ds }

		return server.FastActionContinue, 0, marks, dset
	}
}

func makeReject(reqLen int, buf []byte, offset int, rcode byte) int {
	if offset > reqLen { offset = reqLen }
	buf[2] |= 0x80
	buf[3] |= 0x80
	buf[3] = (buf[3] & 0xF0) | (rcode & 0x0F)
	return offset
}

func findTTLOffsets(msg []byte) []int {
	if len(msg) < 12 { return nil }
	qdcount := binary.BigEndian.Uint16(msg[4:6])
	ancount := binary.BigEndian.Uint16(msg[6:8])
	if ancount == 0 { return nil }
	offset := 12
	for i := 0; i < int(qdcount); i++ {
		for offset < len(msg) {
			l := int(msg[offset]); if l == 0 { offset++; break }; if l&0xC0 == 0xC0 { offset += 2; break }
			offset += l + 1
		}
		offset += 4
	}
	var offsets []int
	for i := 0; i < int(ancount); i++ {
		for offset < len(msg) {
			l := int(msg[offset]); if l == 0 { offset++; break }; if l&0xC0 == 0xC0 { offset += 2; break }
			offset += l + 1
		}
		if offset+10 > len(msg) { break }
		offset += 4; offsets = append(offsets, offset); offset += 4
		rdlen := binary.BigEndian.Uint16(msg[offset : offset+2]); offset += 2 + int(rdlen)
	}
	return offsets
}
