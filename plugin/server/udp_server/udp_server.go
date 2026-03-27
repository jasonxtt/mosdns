package udp_server

import (
	"context"
	"encoding/binary"
	"fmt"
	"hash/maphash"
//	"log"
	"net"
	"net/netip"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/pkg/server"
	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
	"github.com/IrineSistiana/mosdns/v5/plugin/server/server_utils"
	"github.com/cilium/ebpf"
	"github.com/cilium/ebpf/ringbuf"
	"github.com/miekg/dns"
	"go.uber.org/zap"
)

const (
	PluginType       = "udp_server"
	cacheSize        = 65536
	cacheMask        = cacheSize - 1
	internalTTL      = 5
	clientTTL        = 10
	asyncRefreshMark = 1 << 60
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
type DomainMapperPlugin interface {
	FastMatch(qname string) ([]uint8, string, bool)
	GetRunBit() uint8
}
type IPSetPlugin interface{ Match(addr netip.Addr) bool }

type eBpfCacheVal struct {
	ExpireNs uint64
	Updating uint32
	Len      uint16
	Pad      uint16
	Data     [512]byte
}

type fastCacheItem struct {
	expire    int64
	resp[]byte
	updating  uint32
	domainSet string
}

type fastCache struct {
	m       [cacheSize]atomic.Pointer[fastCacheItem]
	ebpfMap *ebpf.Map
	ebpfMu  sync.Mutex
}

func newFastCache() *fastCache {
	fc := &fastCache{}
	if m, err := ebpf.LoadPinnedMap("/sys/fs/bpf/mosdns_fast_cache", nil); err == nil {
		fc.ebpfMap = m
	}
	return fc
}

func (fc *fastCache) getEbpfMap() *ebpf.Map {
	if fc.ebpfMap != nil {
		return fc.ebpfMap
	}
	fc.ebpfMu.Lock()
	defer fc.ebpfMu.Unlock()
	if fc.ebpfMap != nil {
		return fc.ebpfMap
	}
	if m, err := ebpf.LoadPinnedMap("/sys/fs/bpf/mosdns_fast_cache", nil); err == nil {
		fc.ebpfMap = m
	}
	return fc.ebpfMap
}

func calcFNV1a(data[]byte) uint32 {
	h := uint32(0x811c9dc5)
	n := len(data)
	for i, b := range data {
		if i >= 128 {
			break
		}
		if i < n-4 && b >= 'A' && b <= 'Z' {
			b += 32
		}
		h ^= uint32(b)
		h *= 0x01000193
	}
	return h
}

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

func (fc *fastCache) Store(resp []byte, dset string) {
	bakedResp := make([]byte, len(resp))
	copy(bakedResp, resp)
	offsets := findTTLOffsets(bakedResp)
	for _, off := range offsets {
		if off+4 <= len(bakedResp) {
			binary.BigEndian.PutUint32(bakedResp[off:off+4], uint32(clientTTL))
		}
	}

	if len(bakedResp) <= 16 {
		return
	}

	qdcount := binary.BigEndian.Uint16(bakedResp[4:6])
	if qdcount == 1 {
		q_off := 12
		for q_off < len(bakedResp) {
			l := int(bakedResp[q_off])
			if l == 0 {
				q_off++
				break
			}
			if l&0xC0 == 0xC0 {
				q_off += 2
				break
			}
			q_off += (l & 0x3F) + 1
		}
		q_off += 4

		if q_off <= len(bakedResp) && q_off <= 256 {
			rawQuestion := bakedResp[12:q_off]
			hash := calcFNV1a(rawQuestion)

			em := fc.getEbpfMap()
			if em != nil {
				nowNs := getBootTimeNano()
				expireNs := nowNs + uint64(internalTTL)*1e9

				val := eBpfCacheVal{
					ExpireNs: expireNs,
					Updating: 0,
					Len:      uint16(len(bakedResp)),
				}
				copy(val.Data[:], bakedResp)
				em.Put(&hash, &val)
			}

			item := &fastCacheItem{
				resp:      bakedResp,
				expire:    time.Now().Add(internalTTL * time.Second).Unix(),
				updating:  0,
				domainSet: dset,
			}
			fc.m[uint64(hash)&cacheMask].Store(item)
		}
	}
}

type fastHandler struct {
	next server.Handler
	fc   *fastCache
	dm   DomainMapperPlugin
	sw   SwitchPlugin
}

func (h *fastHandler) Handle(ctx context.Context, q *dns.Msg, meta server.QueryMeta, pack func(*dns.Msg) (*[]byte, error)) *[]byte {
	payload := h.next.Handle(ctx, q, meta, pack)

	if (meta.PreFastFlags & asyncRefreshMark) != 0 {
		if payload != nil && q.Opcode == dns.OpcodeQuery && len(q.Question) > 0 {
			var dsetName string
			if h.dm != nil {
				_, dsetName, _ = h.dm.FastMatch(q.Question[0].Name)
			}
			h.fc.Store(*payload, dsetName)
		}
		return nil
	}

	if h.sw != nil && h.sw.GetValue() != "A" {
		return payload
	}

	if payload != nil && (meta.PreFastFlags&(1<<30)) == 0 && q.Opcode == dns.OpcodeQuery && len(q.Question) > 0 {
		var dsetName string
		if h.dm != nil {
			_, dsetName, _ = h.dm.FastMatch(q.Question[0].Name)
		}
		h.fc.Store(*payload, dsetName)
	}
	return payload
}

func startRingbufListener(bp *coremain.BP, h *fastHandler, rd *ringbuf.Reader) {
	for {
		rec, err := rd.Read()
		if err != nil {
			return
		}
		if len(rec.RawSample) < 280 {
			continue
		}

		isV6 := binary.LittleEndian.Uint16(rec.RawSample[0:2])
		dnsLen := binary.LittleEndian.Uint16(rec.RawSample[4:6])
		if dnsLen == 0 || dnsLen > 256 {
			continue
		}

		msg := new(dns.Msg)
		if err := msg.Unpack(rec.RawSample[24 : 24+int(dnsLen)]); err != nil {
			continue
		}

		var clientIP netip.Addr
		if isV6 == 0 {
			clientIP = netip.AddrFrom4(*(*[4]byte)(rec.RawSample[8:12]))
		} else {
			clientIP = netip.AddrFrom16(*(*[16]byte)(rec.RawSample[8:24]))
		}

		meta := server.QueryMeta{
			ClientAddr:   clientIP,
			PreFastFlags: asyncRefreshMark,
		}

		packFunc := func(m *dns.Msg) (*[]byte, error) {
			b, err := m.Pack()
			return &b, err
		}

//		log.Printf("[Go-Ringbuf] Triggering async refresh for Stale Cache.")
		h.Handle(context.Background(), msg, meta, packFunc)
	}
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

	socketOpt := server_utils.ListenerSocketOpts{
		SO_REUSEPORT: true,
		SO_RCVBUF:    2 * 1024 * 1024,
	}
	lc := net.ListenConfig{Control: server_utils.ListenerControl(socketOpt)}
	c, err := lc.ListenPacket(context.Background(), "udp", args.Listen)
	if err != nil {
		return nil, fmt.Errorf("failed to create socket, %w", err)
	}

	isEbpfPort := false
	if udpAddr, ok := c.LocalAddr().(*net.UDPAddr); ok {
		if udpAddr.Port == 53 {
			isEbpfPort = true
		}
	}

	var wrappedHandler server.Handler = dh
	var fastBypass func(int,[]byte, netip.AddrPort) (int, int, uint64, string)

	if isEbpfPort {
		var dm DomainMapperPlugin
		if p := bp.M().GetPlugin("unified_matcher1"); p != nil {
			dm, _ = p.(DomainMapperPlugin)
		}

		var sw15 SwitchPlugin
		if p := bp.M().GetPlugin("switch15"); p != nil {
			sw15, _ = p.(SwitchPlugin)
		}

		fc := newFastCache()
		wrappedFastHandler := &fastHandler{next: dh, fc: fc, dm: dm, sw: sw15}
		wrappedHandler = wrappedFastHandler

		go func() {
			for {
				rm, err := ebpf.LoadPinnedMap("/sys/fs/bpf/mosdns_ringbuf", nil)
				if err != nil {
					time.Sleep(3 * time.Second)
					continue
				}
				rd, err := ringbuf.NewReader(rm)
				if err != nil {
					rm.Close()
					time.Sleep(3 * time.Second)
					continue
				}
				startRingbufListener(bp, wrappedFastHandler, rd)
				rd.Close()
				rm.Close()
				time.Sleep(3 * time.Second)
			}
		}()

		fastBypass = buildFastBypass(bp, fc, c.(*net.UDPConn))
		bp.L().Info("udp server started with eBPF fast-path", zap.Stringer("addr", c.LocalAddr()))
	} else {
		bp.L().Info("udp server started normally (no eBPF fast-path)", zap.Stringer("addr", c.LocalAddr()))
	}

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

func buildFastBypass(bp *coremain.BP, fc *fastCache, conn *net.UDPConn) func(int,[]byte, netip.AddrPort) (int, int, uint64, string) {
	var once sync.Once
	var sw15 SwitchPlugin
	var dm DomainMapperPlugin
	var ipSet IPSetPlugin

	return func(reqLen int, buf[]byte, remoteAddr netip.AddrPort) (int, int, uint64, string) {
		once.Do(func() {
			if p := bp.M().GetPlugin("switch15"); p != nil {
				sw15, _ = p.(SwitchPlugin)
			}
			if p := bp.M().GetPlugin("unified_matcher1"); p != nil {
				dm, _ = p.(DomainMapperPlugin)
			}
			if p := bp.M().GetPlugin("client_ip"); p != nil {
				ipSet, _ = p.(IPSetPlugin)
			}
		})

		var marks uint64 = query_context.GlobalSwitchMask.Load()

		if sw15 == nil || (marks&(1<<46)) == 0 {
			return server.FastActionContinue, 0, 0, ""
		}
		if reqLen < 12 {
			return server.FastActionContinue, 0, 0, ""
		}

		qtypeOff := 12
		for qtypeOff < reqLen {
			l := int(buf[qtypeOff])
			if l == 0 {
				qtypeOff++
				break
			}
			if l&0xC0 == 0xC0 {
				qtypeOff += 2
				break
			}
			qtypeOff += l + 1
		}
		if qtypeOff+2 > reqLen {
			return server.FastActionContinue, 0, 0, ""
		}
		qtype := binary.BigEndian.Uint16(buf[qtypeOff : qtypeOff+2])

		if qtype == 6 || qtype == 12 || qtype == 65 {
			if (marks & (1 << 36)) != 0 {
				return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 0), 0, ""
			}
		}
		if qtype == 28 {
			if (marks & (1 << 37)) != 0 {
				return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 0), 0, ""
			}
		}

		offset := 12
		var nameBuf [256]byte
		nameLen := 0
		for offset < reqLen {
			l := int(buf[offset])
			if l == 0 {
				offset++
				if nameLen == 0 {
					nameBuf[0] = '.'
					nameLen = 1
				}
				break
			}
			if l&0xC0 == 0xC0 {
				return server.FastActionContinue, 0, 0, ""
			}
			offset++
			if offset+l > reqLen || nameLen+l+1 > 256 {
				return server.FastActionContinue, 0, 0, ""
			}
			copy(nameBuf[nameLen:], buf[offset:offset+l])
			nameLen += l
			nameBuf[nameLen] = '.'
			nameLen++
			offset += l
		}

		qname := string(nameBuf[:nameLen])

		var dset string
		if dm != nil {
			marks |= (1 << dm.GetRunBit())
			if mList, dsName, match := dm.FastMatch(qname); match {
				for _, v := range mList {
					if v < 64 {
						marks |= (1 << v)
					}
				}
				dset = dsName
			}
		}

		if (marks & (1 << 32)) != 0 {
			if (marks & (1 << 1)) != 0 {
				return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 3), 0, ""
			}
			if (marks & (1 << 2)) != 0 && qtype == 1 {
				return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 0), 0, ""
			}
			if (marks & (1 << 3)) != 0 && qtype == 28 {
				return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 0), 0, ""
			}
		}
		if (marks & (1 << 38)) != 0 {
			if (marks & (1 << 5)) != 0 {
				return server.FastActionReply, makeReject(reqLen, buf, qtypeOff+4, 3), 0, ""
			}
		}

		ipMatch := false
		if ipSet != nil {
			ipMatch = ipSet.Match(remoteAddr.Addr().Unmap())
			marks |= (1 << 48)
		}

		sw2A := (marks & (1 << 33)) != 0
		sw2B := (marks & (1 << 33)) == 0
		sw12A := (marks & (1 << 43)) != 0
		sw12B := (marks & (1 << 43)) == 0

		if (sw2A && sw12B && !ipMatch) || (sw2B && sw12A && ipMatch) {
			marks |= (1 << 30)
		}

		if (marks & (1 << 6)) != 0 || (marks & (1 << 30)) != 0 {
			return server.FastActionContinue, 0, marks, dset
		}

		q_end := 12
		for q_end < reqLen {
			l := int(buf[q_end])
			if l == 0 {
				q_end++
				break
			}
			if l&0xC0 == 0xC0 {
				q_end += 2
				break
			}
			q_end += (l & 0x3F) + 1
		}
		if q_end+4 > reqLen {
			return server.FastActionContinue, 0, 0, ""
		}
		q_end += 4
		qRawBytes := buf[12:q_end]
		hKey := calcFNV1a(qRawBytes)

		ptr := fc.m[uint64(hKey)&cacheMask].Load()

		if ptr != nil {
			now := time.Now().Unix()
			expireTime := atomic.LoadInt64(&ptr.expire)
			if now > expireTime {
				isStuck := now > expireTime+10
				if atomic.LoadUint32(&ptr.updating) == 0 || isStuck {
					if atomic.CompareAndSwapUint32(&ptr.updating, 0, 1) || (isStuck && atomic.CompareAndSwapUint32(&ptr.updating, 1, 1)) {
						atomic.StoreInt64(&ptr.expire, now+5)

						respLen := len(ptr.resp)
						bakedStale := make([]byte, respLen)
						copy(bakedStale, ptr.resp)
						bakedStale[0], bakedStale[1] = buf[0], buf[1]

						_, _ = conn.WriteToUDPAddrPort(bakedStale, remoteAddr)

						return server.FastActionContinue, 0, marks|asyncRefreshMark, dset
					}
				}
			}
			respLen := len(ptr.resp)
			txid0, txid1 := buf[0], buf[1]
			copy(buf, ptr.resp)
			buf[0], buf[1] = txid0, txid1
			return server.FastActionReply, respLen, 0, ptr.domainSet
		}
		return server.FastActionContinue, 0, marks, dset
	}
}

func makeReject(reqLen int, buf[]byte, offset int, rcode byte) int {
	if offset > reqLen {
		offset = reqLen
	}
	buf[2] |= 0x80
	buf[3] |= 0x80
	buf[3] = (buf[3] & 0xF0) | (rcode & 0x0F)
	buf[6], buf[7] = 0, 0
	buf[8], buf[9] = 0, 0
	buf[10], buf[11] = 0, 0
	return offset
}

func findTTLOffsets(msg[]byte)[]int {
	if len(msg) < 12 {
		return nil
	}
	qdcount := binary.BigEndian.Uint16(msg[4:6])
	ancount := binary.BigEndian.Uint16(msg[6:8])
	if ancount == 0 {
		return nil
	}
	offset := 12
	for i := 0; i < int(qdcount); i++ {
		for offset < len(msg) {
			l := int(msg[offset])
			if l == 0 {
				offset++
				break
			}
			if l&0xC0 == 0xC0 {
				offset += 2
				break
			}
			offset += l + 1
		}
		offset += 4
	}
	var offsets[]int
	for i := 0; i < int(ancount); i++ {
		for offset < len(msg) {
			l := int(msg[offset])
			if l == 0 {
				offset++
				break
			}
			if l&0xC0 == 0xC0 {
				offset += 2
				break
			}
			offset += l + 1
		}
		if offset+10 > len(msg) {
			break
		}
		offset += 4
		offsets = append(offsets, offset)
		offset += 4
		rdlen := binary.BigEndian.Uint16(msg[offset : offset+2])
		offset += 2 + int(rdlen)
	}
	return offsets
}
