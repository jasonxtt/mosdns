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
	jumpBack *ChainWalker
	logger   *zap.Logger
}

func NewChainWalker(chain []*ChainNode, jumpBack *ChainWalker, logger *zap.Logger) ChainWalker {
	return ChainWalker{
		chain:    chain,
		jumpBack: jumpBack,
		logger:   logger,
	}
}

func (w *ChainWalker) ExecNext(ctx context.Context, qCtx *query_context.Context) error {
	p := w.p
	// Evaluate rules' matchers in loop.
checkMatchesLoop:
	for p < len(w.chain) {
		n := w.chain[p]

		for _, namedMatch := range n.Matches {
			ok, err := namedMatch.Matcher.Match(ctx, qCtx)
			if err != nil {
				return err
			}

			// Log matcher execution and result.
			if w.logger != nil {
				if ce := w.logger.Check(zap.DebugLevel, "dns query flows through matcher"); ce != nil {
					domain := "(no question)"
					if len(qCtx.Q().Question) > 0 {
						domain = strings.TrimSuffix(qCtx.Q().Question[0].Name, ".")
					}
					ce.Write(
						zap.String("trace_id", qCtx.TraceID),
						zap.String("domain", domain),
						zap.String("matcher_name", namedMatch.Name),
						zap.Bool("match_result", ok),
						zap.Time("time", time.Now()),
					)
				}
			}

			if ok {
				// Check if a domain_set name has already been stored.
				if _, exists := qCtx.GetValue(query_context.KeyDomainSet); !exists {
					// START OF MODIFICATION
					name := namedMatch.Name
					// 性能优化：HasPrefix 性能远高于 Contains，且 logic 完全一致
					if strings.HasPrefix(name, "anonymous_match(") {
						// Priority 1: Check for switch6 match to identify BANAAAA.
						if strings.HasPrefix(name, "anonymous_match(switch6:") {
							if len(qCtx.Q().Question) > 0 && qCtx.Q().Question[0].Qtype == 28 { // 28 is dns.TypeAAAA
								qCtx.StoreValue(query_context.KeyDomainSet, "BANAAAA")
							}
						// Priority 2: Check for switch5 match to identify BANSOA, BANPTR, BANHTTPS.
						} else if strings.HasPrefix(name, "anonymous_match(switch5:") {
							if len(qCtx.Q().Question) > 0 {
								qtype := qCtx.Q().Question[0].Qtype
								var domainSetName string
								switch qtype {
								case 6: // dns.TypeSOA
									domainSetName = "BANSOA"
								case 12: // dns.TypePTR
									domainSetName = "BANPTR"
								case 65: // dns.TypeHTTPS
									domainSetName = "BANHTTPS"
								}
								if domainSetName != "" {
									qCtx.StoreValue(query_context.KeyDomainSet, domainSetName)
								}
							}
						// Priority 3: Original logic for qname match.
						} else {
							var ruleName string
							const suffix = ")"

							// Handle "qname: $rule" format
							const prefixWithDollar = "anonymous_match(qname: $"
							if strings.HasPrefix(name, prefixWithDollar) {
								ruleName = strings.TrimPrefix(name, prefixWithDollar)
								ruleName = strings.TrimSuffix(ruleName, suffix)
							} else {
								// Handle "qname: rule" format
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
					// END OF MODIFICATION
				}
			} else {
				// Skip this node if a condition was not met.
				p++
				continue checkMatchesLoop
			}
		}

		if w.logger != nil {
			if ce := w.logger.Check(zap.DebugLevel, "dns query flows through plugin"); ce != nil {
				domain := "(no question)"
				if len(qCtx.Q().Question) > 0 {
					domain = strings.TrimSuffix(qCtx.Q().Question[0].Name, ".")
				}

				ce.Write(
					zap.String("trace_id", qCtx.TraceID),
					zap.Uint16("query_id", qCtx.Q().Id),
					zap.String("domain", domain),
					zap.String("plugin_name", n.PluginName),
					zap.Time("time", time.Now()),
				)
			}
		}

		// Exec rules' executables in loop, or in stack if it is a recursive executable.
		switch {
		case n.E != nil:
			if err := n.E.Exec(ctx, qCtx); err != nil {
				return err
			}
			p++
			continue
		case n.RE != nil:
			next := ChainWalker{
				p:        p + 1,
				chain:    w.chain,
				jumpBack: w.jumpBack,
				logger:   w.logger,
			}
			return n.RE.Exec(ctx, qCtx, next)
		default:
			panic("n cannot be executed")
		}
	}

	if w.jumpBack != nil { // End of chain, time to jump back.
		return w.jumpBack.ExecNext(ctx, qCtx)
	}

	// EoC.
	return nil
}

func (w *ChainWalker) nop() bool {
	return w.p >= len(w.chain)
}
