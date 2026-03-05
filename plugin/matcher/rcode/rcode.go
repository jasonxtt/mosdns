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

package rcode

import (
	"context"
	"strconv"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
)

const PluginType = "rcode"

func init() {
	sequence.MustRegMatchQuickSetup(PluginType, QuickSetup)
}

type rcodeMatcher struct {
	codes []int
}

func (m *rcodeMatcher) Match(_ context.Context, qCtx *query_context.Context) (bool, error) {
	r := qCtx.R()
	if r == nil {
		return false, nil
	}
	for _, c := range m.codes {
		if r.Rcode == c {
			return true, nil
		}
	}
	return false, nil
}

func (m *rcodeMatcher) GetFastCheck() func(qCtx *query_context.Context) bool {
	targets := m.codes
	return func(qCtx *query_context.Context) bool {
		r := qCtx.R()
		if r == nil {
			return false
		}
		rc := r.Rcode
		for _, c := range targets {
			if rc == c {
				return true
			}
		}
		return false
	}
}

func QuickSetup(_ sequence.BQ, s string) (sequence.Matcher, error) {
	var codes []int
	for _, f := range strings.Fields(s) {
		v, err := strconv.Atoi(f)
		if err != nil {
			return nil, err
		}
		codes = append(codes, v)
	}
	return &rcodeMatcher{codes: codes}, nil
}
