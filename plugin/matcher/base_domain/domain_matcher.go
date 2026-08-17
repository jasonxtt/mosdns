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

package base_domain

import (
	"context"
	"fmt"
	"strings"
	"sync/atomic"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/domain_set"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
)

var _ sequence.Matcher = (*Matcher)(nil)

type Args struct {
	Exps       []string `yaml:"exps"`
	DomainSets []string `yaml:"domain_sets"`
	Files      []string `yaml:"files"`
}

type MatchFunc func(qCtx *query_context.Context, m domain.Matcher[struct{}]) (bool, error)

// rustDomainWrapper adapts domain_set.RustMatcher to domain.Matcher[struct{}]
// so it integrates naturally with qname/cname MatchFunc selection.
type rustDomainWrapper struct {
	backend  domain_set.RustMatcher
	disabled atomic.Bool
}

func (w *rustDomainWrapper) Match(s string) (struct{}, bool) {
	if w.disabled.Load() {
		return struct{}{}, false
	}
	if !matcher_adapter.RustDomainInputSupported(s) {
		return struct{}{}, false
	}
	matched, err := w.backend.Match(s)
	if err != nil {
		w.Close()
		return struct{}{}, false
	}
	return struct{}{}, matched
}

func (w *rustDomainWrapper) Close() error {
	if !w.disabled.CompareAndSwap(false, true) {
		return nil
	}
	return w.backend.Close()
}

type Matcher struct {
	match       MatchFunc
	mg          []domain.Matcher[struct{}]
	rustBackend *rustDomainWrapper
}

func (m *Matcher) Match(_ context.Context, qCtx *query_context.Context) (bool, error) {
	return m.match(qCtx, domain_set.MatcherGroup(m.mg))
}

// Close implements io.Closer for Rust backend lifecycle cleanup.
func (m *Matcher) Close() error {
	if m.rustBackend != nil {
		return m.rustBackend.Close()
	}
	return nil
}

func NewMatcher(bq sequence.BQ, args *Args, f MatchFunc) (m *Matcher, err error) {
	m = &Matcher{
		match: f,
	}

	// Acquire matchers from other plugins.
	for _, tag := range args.DomainSets {
		p := bq.M().GetPlugin(tag)
		dsProvider, _ := p.(data_provider.DomainMatcherProvider)
		if dsProvider == nil {
			return nil, fmt.Errorf("cannot find domain set %s", tag)
		}
		dm := dsProvider.GetDomainMatcher()
		m.mg = append(m.mg, dm)
	}

	// Anonymous set from plugin's args and files.
	if len(args.Exps)+len(args.Files) > 0 {
		anonymousSet := domain.NewDomainMixMatcher()
		rustRules, err := domain_set.LoadExpsAndFilesWithRules(args.Exps, args.Files, anonymousSet)
		if err != nil {
			return nil, err
		}
		// Keep the Rust candidate at the same position as the anonymous Go
		// matcher. Referenced domain sets retain their established order.
		if rb := domain_set.InitRustDomainMatcher(rustRules); rb != nil {
			wrapper := &rustDomainWrapper{backend: rb}
			m.rustBackend = wrapper
			m.mg = append(m.mg, wrapper)
		}
		if anonymousSet.Len() > 0 {
			m.mg = append(m.mg, anonymousSet)
		}
	}

	return m, nil
}

// ParseQuickSetupArgs parses expressions and domain set to args.
// Format: "([exp] | [$domain_set_tag] | [&domain_list_file])..."
func ParseQuickSetupArgs(s string) *Args {
	cutPrefix := func(s string, p string) (string, bool) {
		if strings.HasPrefix(s, p) {
			return strings.TrimPrefix(s, p), true
		}
		return s, false
	}

	args := new(Args)
	for _, exp := range strings.Fields(s) {
		if tag, ok := cutPrefix(exp, "$"); ok {
			args.DomainSets = append(args.DomainSets, tag)
		} else if path, ok := cutPrefix(exp, "&"); ok {
			args.Files = append(args.Files, path)
		} else {
			args.Exps = append(args.Exps, exp)
		}
	}
	return args
}
