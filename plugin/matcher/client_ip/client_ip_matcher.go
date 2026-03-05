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

package client_ip

import (
	"context"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/IrineSistiana/mosdns/v5/plugin/matcher/base_ip"
)

const PluginType = "client_ip"

func init() {
	sequence.MustRegMatchQuickSetup(PluginType, QuickSetup)
}

type Args = base_ip.Args

func QuickSetup(bq sequence.BQ, s string) (sequence.Matcher, error) {
	m, err := base_ip.NewMatcher(bq, base_ip.ParseQuickSetupArgs(s), matchClientAddr)
	if err != nil {
		return nil, err
	}
	return &fastClientIPMatcher{Matcher: m}, nil
}

func matchClientAddr(qCtx *query_context.Context, m netlist.Matcher) (bool, error) {
	addr := qCtx.ServerMeta.ClientAddr
	if !addr.IsValid() {
		return false, nil
	}
	return m.Match(addr), nil
}

type fastClientIPMatcher struct {
	sequence.Matcher
}

func (f *fastClientIPMatcher) GetFastCheck() func(qCtx *query_context.Context) bool {
	return func(qCtx *query_context.Context) bool {
		ok, _ := f.Matcher.Match(context.Background(), qCtx)
		return ok
	}
}
