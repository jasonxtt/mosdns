package tag_setter

import (
	"context"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
)

const PluginType = "tag_setter"

func init() {
	// 1. 注册标准 YAML 插件加载方式
	coremain.RegNewPluginFunc(PluginType, NewTagSetter, func() any { return new(Args) })

	// 2. 注册 sequence QuickSetup 方式
	// 允许在 sequence 中直接写: - exec: tag_setter "你的标签"
	// 或者清空: - exec: tag_setter ""
	sequence.MustRegExecQuickSetup(PluginType, func(_ sequence.BQ, args string) (any, error) {
		return &TagSetter{tag: args}, nil
	})
}

type Args struct {
	Tag string `yaml:"tag"`
}

type TagSetter struct {
	tag string
}

// 确保实现了 Executable 接口
var _ sequence.Executable = (*TagSetter)(nil)

func NewTagSetter(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)
	return &TagSetter{
		tag: cfg.Tag,
	}, nil
}

func (s *TagSetter) Exec(ctx context.Context, qCtx *query_context.Context) error {
	// 核心逻辑：
	// 如果 tag 为空，则删除 KeyDomainSet (清除标签)
	// 如果 tag 不为空，则设置 KeyDomainSet (覆盖标签)
	if s.tag == "" {
		qCtx.DeleteValue(query_context.KeyDomainSet)
	} else {
		qCtx.StoreValue(query_context.KeyDomainSet, s.tag)
	}
	return nil
}
