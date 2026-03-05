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
	"strings"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"go.uber.org/zap"
)

type instruction struct {
	isSimple   bool
	isTerminal  bool
	fastChecks []func(qCtx *query_context.Context) bool
	fastExec   func(ctx context.Context, qCtx *query_context.Context) error
	
	node *ChainNode
}

// NamedMatcher holds a matcher and its name for logging.
type NamedMatcher struct {
	Name    string
	Matcher Matcher
}

type ChainNode struct {
	PluginName string
	// Use the new struct to store matchers.
	Matches []NamedMatcher 

	// At least one of E or RE must not nil.
	// In case both are set. E is preferred.
	E  Executable
	RE RecursiveExecutable
}

type ChainWalker struct {
	p        int
	chain    []*ChainNode
	ins      []instruction
	jumpBack *ChainWalker
	logger   *zap.Logger
}

func NewChainWalker(ins []instruction, chain[]*ChainNode, jumpBack *ChainWalker, logger *zap.Logger) ChainWalker {
	return ChainWalker{
		ins:      ins,
		chain:    chain,
		jumpBack: jumpBack,
		logger:   logger,
	}
}

func (w *ChainWalker) ExecNext(ctx context.Context, qCtx *query_context.Context) error {
	ins := w.ins
	for w.p < len(ins) {
		instr := &ins[w.p]
		n := instr.node

		// --- 【极速快路径】 ---
		if instr.isSimple {
			matched := true
			checks := instr.fastChecks
			for j := 0; j < len(checks); j++ {
				matchRes := checks[j](qCtx)

				if w.logger != nil {
					if ce := w.logger.Check(zap.DebugLevel, "dns query flows through matcher"); ce != nil {
						domain := "(no question)"
						if len(qCtx.Q().Question) > 0 {
							domain = strings.TrimSuffix(qCtx.Q().Question[0].Name, ".")
						}
						ce.Write(zap.String("trace_id", qCtx.TraceID), zap.String("domain", domain), zap.String("matcher_name", n.Matches[j].Name), zap.Bool("match_result", matchRes), zap.Time("time", time.Now()))
					}
				}

				if !matchRes {
					matched = false
					break
				}
			}

			if matched {
				if w.logger != nil {
					if ce := w.logger.Check(zap.DebugLevel, "dns query flows through plugin"); ce != nil {
						domain := "(no question)"
						if len(qCtx.Q().Question) > 0 {
							domain = strings.TrimSuffix(qCtx.Q().Question[0].Name, ".")
						}
						ce.Write(zap.String("trace_id", qCtx.TraceID), zap.Uint16("query_id", qCtx.Q().Id), zap.String("domain", domain), zap.String("plugin_name", n.PluginName), zap.Time("time", time.Now()))
					}
				}

				err := instr.fastExec(ctx, qCtx)
				if err != nil || instr.isTerminal {
					return err
				}
			}
			w.p++
			continue
		}

		// --- 【回退路径 (包含不支持快路径的复杂插件)】 ---
		matched := true
		for _, m := range n.Matches {
			ok, err := m.Matcher.Match(ctx, qCtx)
			if err != nil {
				return err
			}

			// 【恢复被误删的 Matcher 日志】
			if w.logger != nil {
				if ce := w.logger.Check(zap.DebugLevel, "dns query flows through matcher"); ce != nil {
					domain := "(no question)"
					if len(qCtx.Q().Question) > 0 {
						domain = strings.TrimSuffix(qCtx.Q().Question[0].Name, ".")
					}
					ce.Write(zap.String("trace_id", qCtx.TraceID), zap.String("domain", domain), zap.String("matcher_name", m.Name), zap.Bool("match_result", ok), zap.Time("time", time.Now()))
				}
			}

			if ok {
				// 保留原版的 KeyDomainSet 处理
				if _, exists := qCtx.GetValue(query_context.KeyDomainSet); !exists {
					name := m.Name
					if strings.HasPrefix(name, "anonymous_match(") {
						if strings.HasPrefix(name, "anonymous_match(switch6:") {
							if len(qCtx.Q().Question) > 0 && qCtx.Q().Question[0].Qtype == 28 {
								qCtx.StoreValue(query_context.KeyDomainSet, "BANAAAA")
							}
						} else if strings.HasPrefix(name, "anonymous_match(switch5:") {
							if len(qCtx.Q().Question) > 0 {
								qtype := qCtx.Q().Question[0].Qtype
								var domainSetName string
								switch qtype {
								case 6:
									domainSetName = "BANSOA"
								case 12:
									domainSetName = "BANPTR"
								case 65:
									domainSetName = "BANHTTPS"
								}
								if domainSetName != "" {
									qCtx.StoreValue(query_context.KeyDomainSet, domainSetName)
								}
							}
						} else {
							var ruleName string
							const suffix = ")"
							const prefixWithDollar = "anonymous_match(qname: $"
							if strings.HasPrefix(name, prefixWithDollar) {
								ruleName = strings.TrimPrefix(name, prefixWithDollar)
								ruleName = strings.TrimSuffix(ruleName, suffix)
							} else {
								const prefixWithoutDollar = "anonymous_match(qname: "
								if strings.HasPrefix(name, prefixWithoutDollar) {
									ruleName = strings.TrimPrefix(name, prefixWithoutDollar)
									ruleName = strings.TrimSuffix(ruleName, suffix)
								}
							}
							if ruleName != "" {
								qCtx.StoreValue(query_context.KeyDomainSet, ruleName)
							}
						}
					}
				}
			} else {
				matched = false
				break
			}
		}

		if matched {
			// 【恢复被误删的 Plugin 日志】
			if w.logger != nil {
				if ce := w.logger.Check(zap.DebugLevel, "dns query flows through plugin"); ce != nil {
					domain := "(no question)"
					if len(qCtx.Q().Question) > 0 {
						domain = strings.TrimSuffix(qCtx.Q().Question[0].Name, ".")
					}
					ce.Write(zap.String("trace_id", qCtx.TraceID), zap.Uint16("query_id", qCtx.Q().Id), zap.String("domain", domain), zap.String("plugin_name", n.PluginName), zap.Time("time", time.Now()))
				}
			}

			if n.E != nil {
				if err := n.E.Exec(ctx, qCtx); err != nil {
					return err
				}
			} else if n.RE != nil {
				nextWalker := ChainWalker{
					p:        w.p + 1,
					chain:    w.chain,
					ins:      w.ins,
					jumpBack: w.jumpBack,
					logger:   w.logger,
				}
				return n.RE.Exec(ctx, qCtx, nextWalker)
			}
		}
		w.p++
	}

	// 序列执行到底部时，无缝跳回父级
	if w.jumpBack != nil {
		return w.jumpBack.ExecNext(ctx, qCtx)
	}

	return nil
}

func (w *ChainWalker) nop() bool {
	return w.p >= len(w.chain)
}
