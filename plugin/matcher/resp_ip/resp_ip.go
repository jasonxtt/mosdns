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

package resp_ip

import (
	"context"
	"net"
	"net/netip"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/IrineSistiana/mosdns/v5/plugin/matcher/base_ip"
	"github.com/miekg/dns"
)

const PluginType = "resp_ip"

func init() {
	sequence.MustRegMatchQuickSetup(PluginType, QuickSetup)
}

type Args = base_ip.Args

func QuickSetup(bq sequence.BQ, s string) (sequence.Matcher, error) {
	m, err := base_ip.NewMatcher(bq, base_ip.ParseQuickSetupArgs(s), matchRespAddr)
	if err != nil {
		return nil, err
	}
	return &fastRespIPMatcher{Matcher: m}, nil
}

func matchRespAddr(qCtx *query_context.Context, m netlist.Matcher) (bool, error) {
	r := qCtx.R()
	if r == nil {
		return false, nil
	}
	for _, rr := range r.Answer {
		var ip net.IP
		switch rr := rr.(type) {
		case *dns.A:
			ip = rr.A
		case *dns.AAAA:
			ip = rr.AAAA
		default:
			continue
		}
		addr, ok := netip.AddrFromSlice(ip)
		if ok && m.Match(addr) {
			return true, nil
		}
	}
	return false, nil
}

type fastRespIPMatcher struct {
	sequence.Matcher
}

func (f *fastRespIPMatcher) GetFastCheck() func(qCtx *query_context.Context) bool {
	return func(qCtx *query_context.Context) bool {
		ok, _ := f.Matcher.Match(context.Background(), qCtx)
		return ok
	}
}
