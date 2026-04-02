package flow_setter

import (
	"context"
	"fmt"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
)

const PluginType = "flow_setter"

func init() {
	coremain.RegNewPluginFunc(PluginType, NewFlowSetter, func() any { return new(Args) })
	sequence.MustRegExecQuickSetup(PluginType, func(_ sequence.BQ, args string) (any, error) {
		cfg := &Args{}
		for _, part := range strings.Fields(args) {
			kv := strings.SplitN(part, "=", 2)
			if len(kv) != 2 {
				return nil, fmt.Errorf("invalid flow_setter arg %q", part)
			}
			switch kv[0] {
			case "group":
				cfg.MatchedGroup = kv[1]
			case "sequence":
				cfg.FinalSequence = kv[1]
			case "upstream":
				cfg.FinalUpstream = kv[1]
			default:
				return nil, fmt.Errorf("unknown flow_setter key %q", kv[0])
			}
		}
		return &FlowSetter{args: *cfg}, nil
	})
}

type Args struct {
	MatchedGroup  string `yaml:"matched_group"`
	FinalSequence string `yaml:"final_sequence"`
	FinalUpstream string `yaml:"final_upstream"`
}

type FlowSetter struct {
	args Args
}

var _ sequence.Executable = (*FlowSetter)(nil)

func NewFlowSetter(_ *coremain.BP, args any) (any, error) {
	return &FlowSetter{args: *(args.(*Args))}, nil
}

func (s *FlowSetter) Exec(_ context.Context, qCtx *query_context.Context) error {
	if s.args.MatchedGroup != "" {
		qCtx.StoreValue(query_context.KeyMatchedGroup, s.args.MatchedGroup)
	}
	if s.args.FinalSequence != "" {
		qCtx.StoreValue(query_context.KeyFinalSequence, s.args.FinalSequence)
	}
	if s.args.FinalUpstream != "" {
		qCtx.StoreValue(query_context.KeyFinalUpstream, s.args.FinalUpstream)
	}
	return nil
}
