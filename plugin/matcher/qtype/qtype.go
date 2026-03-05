package qtype

import (
	"context"
	"strconv"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
)

const PluginType = "qtype"

func init() {
	sequence.MustRegMatchQuickSetup(PluginType, QuickSetup)
}

type qtypeMatcher struct {
	types []uint16
}

func (m *qtypeMatcher) Match(_ context.Context, qCtx *query_context.Context) (bool, error) {
	// 保持原版逻辑：遍历所有 Question (虽然通常只有一个)
	for _, question := range qCtx.Q().Question {
		qt := question.Qtype
		for _, t := range m.types {
			if qt == t {
				return true, nil
			}
		}
	}
	return false, nil
}

func (m *qtypeMatcher) GetFastCheck() func(qCtx *query_context.Context) bool {
	// 预先拷贝，确保闭包安全且高效
	tList := make([]uint16, len(m.types))
	copy(tList, m.types)
	return func(qCtx *query_context.Context) bool {
		// DNS 请求通常只有一个 question，这里做极速提取
		q := qCtx.Q()
		if len(q.Question) == 0 {
			return false
		}
		qt := q.Question[0].Qtype
		for _, t := range tList {
			if qt == t {
				return true
			}
		}
		// 如果有多个 question (极少见)，走完整遍历
		if len(q.Question) > 1 {
			for i := 1; i < len(q.Question); i++ {
				qt := q.Question[i].Qtype
				for _, t := range tList {
					if qt == t {
						return true
					}
				}
			}
		}
		return false
	}
}

func QuickSetup(_ sequence.BQ, s string) (sequence.Matcher, error) {
	var types []uint16
	for _, f := range strings.Fields(s) {
		v, err := strconv.ParseUint(f, 10, 16)
		if err != nil {
			return nil, err
		}
		types = append(types, uint16(v))
	}
	return &qtypeMatcher{types: types}, nil
}
