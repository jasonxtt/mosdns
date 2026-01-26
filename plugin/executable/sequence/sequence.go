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

package sequence

import (
	"context"
	"fmt"
	"io"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"go.uber.org/zap"
)

const PluginType = "sequence"

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })

	MustRegExecQuickSetup("accept", setupAccept)
	MustRegExecQuickSetup("reject", setupReject)
	MustRegExecQuickSetup("return", setupReturn)
	MustRegExecQuickSetup("goto", setupGoto)
	MustRegExecQuickSetup("jump", setupJump)
	MustRegExecQuickSetup("exit", setupExit)
	MustRegExecQuickSetup("try", setupTry)
	MustRegMatchQuickSetup("_true", setupTrue)
	MustRegMatchQuickSetup("_false", setupFalse)
}

type Sequence struct {
	chain            []*ChainNode
	anonymousPlugins []any
	logger           *zap.Logger
	// [新增] 标记该序列是否为内联（隐式）序列
	isInline bool 
}

func (s *Sequence) Close() error {
	for _, plugin := range s.anonymousPlugins {
		closePlugin(plugin)
	}
	return nil
}

type Args = []RuleArgs

func Init(bp *coremain.BP, args any) (any, error) {
	return NewSequence(NewBQ(bp.M(), bp.L()), *args.(*Args))
}

func NewSequence(bq BQ, ra []RuleArgs) (*Sequence, error) {
	s := &Sequence{
		logger: bq.L(),
	}

	var rc []RuleConfig
	for _, ra := range ra {
		rc = append(rc, parseArgs(ra))
	}
	if err := s.buildChain(bq, rc); err != nil {
		_ = s.Close()
		return nil, err
	}
	return s, nil
}

// Exec 执行序列逻辑
func (s *Sequence) Exec(ctx context.Context, qCtx *query_context.Context) error {
    walker := NewChainWalker(s.chain, nil, s.logger)
    err := walker.ExecNext(ctx, qCtx)
    return err // 直接返回，不做任何判断
}

func (s *Sequence) buildChain(bq BQ, rs []RuleConfig) error {
	c := make([]*ChainNode, 0, len(rs))
	for ri, r := range rs {
		n, err := s.newNode(bq, r, ri)
		if err != nil {
			return fmt.Errorf("failed to init rule #%d, %w", ri, err)
		}
		c = append(c, n)
	}
	s.chain = c
	return nil
}

func (s *Sequence) newNode(bq BQ, r RuleConfig, ri int) (*ChainNode, error) {
	n := new(ChainNode)

	// PluginName generation
	if len(r.Execs) == 0 {
		n.PluginName = "no_exec"
	} else if len(r.Execs) == 1 {
		ec := r.Execs[0]
		if len(ec.Tag) > 0 {
			n.PluginName = ec.Tag
		} else {
			n.PluginName = fmt.Sprintf("anonymous_exec(%s: %v)", ec.Type, ec.Args)
		}
	} else {
		var names []string
		for _, ec := range r.Execs {
			if len(ec.Tag) > 0 {
				names = append(names, ec.Tag)
			} else {
				names = append(names, ec.Type) 
			}
		}
		n.PluginName = fmt.Sprintf("multi_exec[%s]", strings.Join(names, ","))
	}

	// init matches
	for mi, mc := range r.Matches {
		namedM, err := s.newMatcher(bq, mc, ri, mi)
		if err != nil {
			return nil, fmt.Errorf("failed to init matcher #%d, %w", mi, err)
		}
		n.Matches = append(n.Matches, namedM)
	}

	// init exec
	if len(r.Execs) == 1 {
		e, re, err := s.newExec(bq, r.Execs[0], ri)
		if err != nil {
			return nil, fmt.Errorf("failed to init exec, %w", err)
		}
		n.E = e
		n.RE = re
	} else if len(r.Execs) > 1 {
		var subRules []RuleConfig
		for _, ec := range r.Execs {
			subRules = append(subRules, RuleConfig{
				Execs: []ExecConfig{ec}, 
			})
		}

		// [修改] 创建子序列时，标记 isInline = true
		subSeq := &Sequence{
			logger:   s.logger,
			isInline: true, 
		}
		
		if err := subSeq.buildChain(bq, subRules); err != nil {
			return nil, fmt.Errorf("failed to build multi-exec sequence: %w", err)
		}

		s.anonymousPlugins = append(s.anonymousPlugins, subSeq)
		n.E = subSeq
	}

	return n, nil
}

func (s *Sequence) newMatcher(bq BQ, mc MatchConfig, ri, mi int) (NamedMatcher, error) {
	var m Matcher
	var name string

	switch {
	case len(mc.Tag) > 0:
		name = mc.Tag
		p, _ := bq.M().GetPlugin(name).(Matcher)
		if p == nil {
			return NamedMatcher{}, fmt.Errorf("can not find matcher %s", name)
		}
		if qc, ok := p.(QuickConfigurableMatch); ok {
			v, err := qc.QuickConfigureMatch(mc.Args)
			if err != nil {
				return NamedMatcher{}, fmt.Errorf("fail to configure plugin %s, %w", name, err)
			}
			m = v
		} else {
			m = p
		}

	case len(mc.Type) > 0:
		name = fmt.Sprintf("anonymous_match(%s: %v)", mc.Type, mc.Args)
		f := GetMatchQuickSetup(mc.Type)
		if f == nil {
			return NamedMatcher{}, fmt.Errorf("invalid matcher type %s", mc.Type)
		}
		p, err := f(NewBQ(bq.M(), bq.L().Named(fmt.Sprintf("r%d.m%d", ri, mi))), mc.Args)
		if err != nil {
			return NamedMatcher{}, fmt.Errorf("failed to init matcher, %w", err)
		}
		s.anonymousPlugins = append(s.anonymousPlugins, p)
		m = p
	default:
		return NamedMatcher{}, fmt.Errorf("missing args")
	}

	if mc.Reverse {
		m = reverseMatcher(m)
		name = fmt.Sprintf("not(%s)", name)
	}
	return NamedMatcher{Name: name, Matcher: m}, nil
}

func (s *Sequence) newExec(bq BQ, ec ExecConfig, ri int) (Executable, RecursiveExecutable, error) {
	var exec any
	switch {
	case len(ec.Tag) > 0:
		p := bq.M().GetPlugin(ec.Tag)
		if p == nil {
			return nil, nil, fmt.Errorf("can not find executable %s", ec.Tag)
		}
		if qc, ok := p.(QuickConfigurableExec); ok {
			v, err := qc.QuickConfigureExec(ec.Args)
			if err != nil {
				return nil, nil, fmt.Errorf("fail to configure plugin %s, %w", ec.Tag, err)
			}
			exec = v
		} else {
			exec = p
		}
	case len(ec.Type) > 0:
		f := GetExecQuickSetup(ec.Type)
		if f == nil {
			return nil, nil, fmt.Errorf("invalid executable type %s", ec.Type)
		}
		v, err := f(NewBQ(bq.M(), bq.L().Named(fmt.Sprintf("r%d", ri))), ec.Args)
		if err != nil {
			return nil, nil, fmt.Errorf("failed to init executable, %w", err)
		}
		s.anonymousPlugins = append(s.anonymousPlugins, v)
		exec = v
	default:
		return nil, nil, fmt.Errorf("missing args")
	}

	e, _ := exec.(Executable)
	re, _ := exec.(RecursiveExecutable)

	if re == nil && e == nil {
		return nil, nil, fmt.Errorf("invalid args, initialized object is not executable")
	}
	return e, re, nil
}

func closePlugin(p any) {
	if c, ok := p.(io.Closer); ok {
		_ = c.Close()
	}
}

func reverseMatcher(m Matcher) Matcher {
	return reverseMatch{m: m}
}

type reverseMatch struct {
	m Matcher
}

func (r reverseMatch) Match(ctx context.Context, qCtx *query_context.Context) (bool, error) {
	ok, err := r.m.Match(ctx, qCtx)
	if err != nil {
		return false, err
	}
	return !ok, nil
}
