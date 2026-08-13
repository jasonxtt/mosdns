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

package server_handler

import (
	"context"
	"encoding/binary"
	"fmt"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain" // ADDED: Import coremain for audit collector
	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/IrineSistiana/mosdns/v5/pkg/pool"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/pkg/server"
	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
	"go.uber.org/zap"
)

const (
	defaultQueryTimeout = time.Second * 5
)

var (
	nopLogger = mlog.Nop()

	// options that can forward to upstream
	queryForwardEDNS0Option = map[uint16]struct{}{
		dns.EDNS0SUBNET: {},
	}

	// options that useless for downstream
	respRemoveEDNS0Option = map[uint16]struct{}{
		dns.EDNS0PADDING: {},
	}
)

type EntryHandlerOpts struct {
	// Logger is used for logging. Default is a noop logger.
	Logger *zap.Logger

	// Required.
	Entry sequence.Executable

	// QueryTimeout limits the timeout value of each query.
	// Default is defaultQueryTimeout.
	QueryTimeout time.Duration

	// ADDED: Flag to enable audit and process logging for this handler instance.
	EnableAudit bool
}

func (opts *EntryHandlerOpts) init() {
	if opts.Logger == nil {
		opts.Logger = nopLogger
	}
	utils.SetDefaultNum(&opts.QueryTimeout, defaultQueryTimeout)
}

type EntryHandler struct {
	opts EntryHandlerOpts
}

var _ server.Handler = (*EntryHandler)(nil)

func NewEntryHandler(opts EntryHandlerOpts) *EntryHandler {
	opts.init()
	return &EntryHandler{opts: opts}
}

// Handle implements server.Handler.
// If entry returns an error, a SERVFAIL response will be returned.
// If entry returns without a response, a REFUSED response will be returned.
func (h *EntryHandler) Handle(ctx context.Context, q *dns.Msg, serverMeta server.QueryMeta, packMsgPayload func(m *dns.Msg) (*[]byte, error)) *[]byte {
	// basic query check.
	if q.Response || len(q.Question) != 1 || len(q.Answer)+len(q.Ns) > 0 || len(q.Extra) > 1 {
		return nil
	}

	ddl := time.Now().Add(h.opts.QueryTimeout)
	ctx, cancel := context.WithDeadline(ctx, ddl)
	defer cancel()

	qCtx := query_context.NewContext(q)
	qCtx.ServerMeta = serverMeta
	qCtx.ApplyFastFlags(serverMeta.PreFastFlags)
	if serverMeta.PreFastDomainSet != "" {
		qCtx.StoreValue(query_context.KeyDomainSet, serverMeta.PreFastDomainSet)
	}

	// --- FINAL MODIFICATION: The definitive logic to avoid double logging ---
	// This single flag, passed from the server config, now controls both logging systems.
	if h.opts.EnableAudit {
		// Mark this context so that downstream sequence loggers know to capture it.
		qCtx.StoreValue(coremain.LogMarkKey, true)
		// Defer the final audit log collection.
		defer coremain.GlobalAuditCollector.Collect(qCtx)
	}
	// --- END OF MODIFICATION ---

	// exec entry
	err := h.opts.Entry.Exec(ctx, qCtx)
	var resp *dns.Msg
	if err != nil {
		// [修改开始] 特殊处理 ErrExit
		// 如果错误是 sequence.ErrExit，说明是插件主动要求退出（任务完成或拦截）
		// 我们将其视为“无错误”，继续向下执行，尝试返回 qCtx.R()
		if err == sequence.ErrExit {
			err = nil
		} else {
			// 只有真正的错误才记录日志并返回 SERVFAIL
			h.opts.Logger.Warn("entry err", qCtx.InfoField(), zap.Error(err))
			resp = new(dns.Msg)
			resp.SetReply(q)
			resp.Rcode = dns.RcodeServerFailure
		}
		// [修改结束]
	}

	// 注意：上面的 if err != nil 块如果被 ErrExit 绕过了（err被置为nil），
	// 这里 resp 依然是 nil，需要从 qCtx 获取
	if resp == nil {
		if rawResp := qCtx.RawResponse(); len(rawResp) > 0 {
			payload, rawErr := h.packRawResponse(qCtx, q, rawResp, packMsgPayload)
			if rawErr == nil {
				return payload
			}
			h.opts.Logger.Warn("failed to pack raw response", qCtx.InfoField(), zap.Error(rawErr))
			resp = new(dns.Msg)
			resp.SetReply(q)
			resp.Rcode = dns.RcodeServerFailure
		} else {
			resp = qCtx.R()
		}
	}

	if resp == nil {
		resp = new(dns.Msg)
		resp.SetReply(q)
		resp.Rcode = dns.RcodeRefused
	}
	// We assume that our server is a forwarder.
	resp.RecursionAvailable = true

	// add respOpt back to resp
	if respOpt := qCtx.RespOpt(); respOpt != nil {
		resp.Extra = append(resp.Extra, respOpt)
	}

	if serverMeta.FromUDP {
		udpSize := getValidUDPSize(qCtx.ClientOpt())
		resp.Truncate(udpSize)
	}

	payload, err := packMsgPayload(resp)
	if err != nil {
		h.opts.Logger.Error("internal err: failed to pack resp msg", qCtx.InfoField(), zap.Error(err))
		return nil
	}
	return payload
}

func (h *EntryHandler) packRawResponse(
	qCtx *query_context.Context,
	q *dns.Msg,
	rawResp []byte,
	packMsgPayload func(m *dns.Msg) (*[]byte, error),
) (*[]byte, error) {
	if len(rawResp) < 12 || rawResp[2]&0x80 == 0 {
		return nil, fmt.Errorf("invalid raw DNS response")
	}

	needMessagePath := qCtx.RespOpt() != nil
	if qCtx.ServerMeta.FromUDP && len(rawResp) > getValidUDPSize(qCtx.ClientOpt()) {
		needMessagePath = true
	}
	if needMessagePath {
		resp := qCtx.R()
		if resp == nil {
			return nil, fmt.Errorf("failed to decode raw DNS response")
		}
		resp.Id = q.Id
		resp.RecursionAvailable = true
		if respOpt := qCtx.RespOpt(); respOpt != nil {
			resp.Extra = append(resp.Extra, respOpt)
		}
		if qCtx.ServerMeta.FromUDP {
			resp.Truncate(getValidUDPSize(qCtx.ClientOpt()))
		}
		return packMsgPayload(resp)
	}

	streamTransport := !qCtx.ServerMeta.FromUDP && qCtx.ServerMeta.UrlPath == ""
	binary.BigEndian.PutUint16(rawResp[:2], q.Id)
	rawResp[3] |= 0x80
	if streamTransport {
		if len(rawResp) > dns.MaxMsgSize {
			return nil, fmt.Errorf("raw DNS response length %d exceeds maximum", len(rawResp))
		}
		payload := pool.GetBuf(2 + len(rawResp))
		binary.BigEndian.PutUint16((*payload)[:2], uint16(len(rawResp)))
		copy((*payload)[2:], rawResp)
		return payload, nil
	}

	payload := pool.GetBuf(len(rawResp))
	copy(*payload, rawResp)
	return payload, nil
}

// opt can be nil.
func getValidUDPSize(opt *dns.OPT) int {
	var s uint16
	if opt != nil {
		s = opt.UDPSize()
	}
	if s < dns.MinMsgSize {
		s = dns.MinMsgSize
	}
	return int(s)
}

func newOpt() *dns.OPT {
	opt := new(dns.OPT)
	opt.Hdr.Name = "."
	opt.Hdr.Rrtype = dns.TypeOPT
	return opt
}
