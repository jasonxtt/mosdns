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
	"errors"
	"fmt"
	"strconv"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/miekg/dns"
)

// [新增] 定义一个全局导出的错误变量，用于 exit 信号
var ErrExit = fmt.Errorf("exit sequence")

var _ RecursiveExecutable = (*ActionAccept)(nil)

type ActionAccept struct{}

func (a ActionAccept) Exec(_ context.Context, _ *query_context.Context, _ ChainWalker) error {
	return nil
}

func setupAccept(_ BQ, _ string) (any, error) {
	return ActionAccept{}, nil
}

// [新增] Exit 插件实现
var _ RecursiveExecutable = (*ActionExit)(nil)

type ActionExit struct{}

func (a ActionExit) Exec(_ context.Context, _ *query_context.Context, _ ChainWalker) error {
	// 返回特殊错误，强制中断
	return ErrExit
}

func setupExit(_ BQ, _ string) (any, error) {
	return ActionExit{}, nil
}

// [新增] Try 插件实现
var _ RecursiveExecutable = (*ActionTry)(nil)

type ActionTry struct {
	Target Executable
}

func (a *ActionTry) Exec(ctx context.Context, qCtx *query_context.Context, next ChainWalker) error {
	// 1. 执行目标插件/序列
	err := a.Target.Exec(ctx, qCtx)

	// 2. 关键判断：
	// 如果 err 不为空，且包含 ErrExit 信号（errors.Is 会自动解包检查）
	// 那么我们认为这是子序列想退出它自己，对于父序列来说，这视为“执行完毕”，
	// 所以我们将 err 置为 nil，表示“已处理”。
	if err != nil && errors.Is(err, ErrExit) {
		err = nil
	}

	// 3. 如果是其他真正的错误（比如网络IO错误），则直接返回，中断整个链条
	if err != nil {
		return err
	}

	// 4. [最关键的一步]
	// 错误已经被吞掉了（或者原本就没错误），现在必须告诉 MosDNS：
	// “我这边完事了，请继续执行当前序列的下一个插件！”
	// 如果不调用这一句，父序列就会在这里停下。
	return next.ExecNext(ctx, qCtx)
}

func setupTry(bq BQ, s string) (any, error) {
	// 解析参数，例如 "try $sequence_6666" 中的 "$sequence_6666"
	pluginName := s
	if len(s) > 0 && s[0] == '$' {
		pluginName = s[1:]
	}

	p := bq.M().GetPlugin(pluginName)
	if p == nil {
		return nil, fmt.Errorf("can not find try target %s", s)
	}

	exec := ToExecutable(p)
	if exec == nil {
		return nil, fmt.Errorf("plugin %s is not executable", s)
	}

	return &ActionTry{Target: exec}, nil
}

var _ RecursiveExecutable = (*ActionReject)(nil)

type ActionReject struct {
	Rcode int
}

func (a ActionReject) Exec(_ context.Context, qCtx *query_context.Context, _ ChainWalker) error {
	r := new(dns.Msg)
	r.SetReply(qCtx.Q())
	r.Rcode = a.Rcode
	qCtx.SetResponse(r)
	return nil
}

func setupReject(_ BQ, s string) (any, error) {
	rcode := dns.RcodeRefused
	if len(s) > 0 {
		n, err := strconv.Atoi(s)
		if err != nil || n < 0 || n > 0xFFF {
			return nil, fmt.Errorf("invalid rcode [%s]", s)
		}
		rcode = n
	}
	return ActionReject{Rcode: rcode}, nil
}

var _ RecursiveExecutable = (*ActionReturn)(nil)

type ActionReturn struct{}

func (a ActionReturn) Exec(ctx context.Context, qCtx *query_context.Context, next ChainWalker) error {
	if next.jumpBack != nil {
		return next.jumpBack.ExecNext(ctx, qCtx)
	}
	return nil
}

func setupReturn(_ BQ, _ string) (any, error) {
	return ActionReturn{}, nil
}

var _ RecursiveExecutable = (*ActionJump)(nil)

type ActionJump struct {
	To []*ChainNode
}

func (a *ActionJump) Exec(ctx context.Context, qCtx *query_context.Context, next ChainWalker) error {
	w := NewChainWalker(a.To, &next, next.logger)
	return w.ExecNext(ctx, qCtx)
}

func setupJump(bq BQ, s string) (any, error) {
	target, _ := bq.M().GetPlugin(s).(*Sequence)
	if target == nil {
		return nil, fmt.Errorf("can not find jump target %s", s)
	}
	return &ActionJump{To: target.chain}, nil
}

var _ RecursiveExecutable = (*ActionGoto)(nil)

type ActionGoto struct {
	To []*ChainNode
}

func (a ActionGoto) Exec(ctx context.Context, qCtx *query_context.Context, next ChainWalker) error {
	w := NewChainWalker(a.To, nil, next.logger)
	return w.ExecNext(ctx, qCtx)
}

func setupGoto(bq BQ, s string) (any, error) {
	gt, _ := bq.M().GetPlugin(s).(*Sequence)
	if gt == nil {
		return nil, fmt.Errorf("can not find goto target %s", s)
	}
	return &ActionGoto{To: gt.chain}, nil
}

var _ Matcher = (*MatchAlwaysTrue)(nil)

type MatchAlwaysTrue struct{}

func (m MatchAlwaysTrue) Match(_ context.Context, _ *query_context.Context) (bool, error) {
	return true, nil
}

func setupTrue(_ BQ, _ string) (Matcher, error) {
	return MatchAlwaysTrue{}, nil
}

var _ Matcher = (*MatchAlwaysFalse)(nil)

type MatchAlwaysFalse struct{}

func (m MatchAlwaysFalse) Match(_ context.Context, _ *query_context.Context) (bool, error) {
	return false, nil
}

func setupFalse(_ BQ, _ string) (Matcher, error) {
	return MatchAlwaysFalse{}, nil
}
