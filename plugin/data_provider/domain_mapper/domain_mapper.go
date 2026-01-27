package domain_mapper

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"go.uber.org/zap"
)

const PluginType = "domain_mapper"

func init() {
	coremain.RegNewPluginFunc(PluginType, NewMapper, func() any { return new(Args) })
}

type RuleConfig struct {
	Tag       string `yaml:"tag"`        // 对应 domain_set 或 sd_set 的 tag (数据源)
	Mark      uint8  `yaml:"mark"`       // [修改] 匹配命中时设置的 fast_mark (0-63)
	OutputTag string `yaml:"output_tag"` // 自定义输出的 tag，如果不填则使用源 tag
}

type Args struct {
	Rules       []RuleConfig `yaml:"rules"`
	DefaultMark uint8        `yaml:"default_mark"` // [修改] 未匹配任何规则时设置的默认 fast_mark (0-63)
	DefaultTag  string       `yaml:"default_tag"`  // [新增] 未匹配任何规则时设置的默认 tag
}

// MatchResult 存储在 Trie 树叶子节点中的数据
type MatchResult struct {
	Marks []uint8 // [修改] 存储 fast_mark
	Tags  []string
}

type DomainMapper struct {
	logger *zap.Logger
	mu     sync.RWMutex
	// 核心匹配器，Payload 是 MatchResult 指针
	matcher *domain.MixMatcher[*MatchResult]

	// 防抖相关
	updateMu    sync.Mutex
	updateTimer *time.Timer

	// 保存配置引用以便 rebuild 时保持顺序
	ruleConfigs []RuleConfig
	defaultMark uint8 // [修改]
	defaultTag  string // [新增] 保存默认 tag 配置
	// 插件提供者映射 (Tag -> Exporter)
	providers map[string]data_provider.RuleExporter
}

// 确保实现了 Executable (Exec)
var _ sequence.Executable = (*DomainMapper)(nil)

func NewMapper(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)

	// [新增] 校验 fast_mark 范围 (必须 < 64)
	if cfg.DefaultMark > 63 {
		return nil, fmt.Errorf("default_mark must be between 0 and 63, got %d", cfg.DefaultMark)
	}
	for _, r := range cfg.Rules {
		if r.Mark > 63 {
			return nil, fmt.Errorf("rule mark for tag '%s' must be between 0 and 63, got %d", r.Tag, r.Mark)
		}
	}

	dm := &DomainMapper{
		logger:      bp.L(),
		matcher:     domain.NewMixMatcher[*MatchResult](),
		ruleConfigs: cfg.Rules,
		defaultMark: cfg.DefaultMark,
		defaultTag:  cfg.DefaultTag, // [新增] 初始化
		providers:   make(map[string]data_provider.RuleExporter),
	}

	// 1. 验证并收集所有依赖的插件
	for _, r := range cfg.Rules {
		if _, loaded := dm.providers[r.Tag]; loaded {
			continue
		}

		pluginInterface := bp.M().GetPlugin(r.Tag)
		if pluginInterface == nil {
			return nil, fmt.Errorf("plugin %s not found", r.Tag)
		}
		exporter, ok := pluginInterface.(data_provider.RuleExporter)
		if !ok {
			return nil, fmt.Errorf("plugin %s does not support rule export (is it domain_set or sd_set?)", r.Tag)
		}
		dm.providers[r.Tag] = exporter
	}

	// 2. 定义重构建逻辑
	rebuild := func() {
		dm.logger.Info("rebuilding domain map...")
		start := time.Now()

		// 临时 Map 用于聚合数据： Domain -> Result
		domainMap := make(map[string]*MatchResult)

		totalRulesProcessed := 0

		// 遍历配置文件中的规则列表
		for _, ruleCfg := range dm.ruleConfigs {
			provider, ok := dm.providers[ruleCfg.Tag]
			if !ok {
				continue
			}

			rules, err := provider.GetRules()
			if err != nil {
				dm.logger.Error("failed to get rules from provider", zap.String("tag", ruleCfg.Tag), zap.Error(err))
				continue
			}

			// 确定要写入的 Tag 字符串
			targetTagStr := ruleCfg.OutputTag
			if targetTagStr == "" {
				targetTagStr = ruleCfg.Tag
			}

			for _, ruleStr := range rules {
				entry, exists := domainMap[ruleStr]
				if !exists {
					entry = &MatchResult{
						Marks: make([]uint8, 0, 1), // [修改] make uint8
						Tags:  make([]string, 0, 1),
					}
					domainMap[ruleStr] = entry
				}

				// 处理 Mark
				if ruleCfg.Mark != 0 {
					markExists := false
					for _, m := range entry.Marks {
						if m == ruleCfg.Mark {
							markExists = true
							break
						}
					}
					if !markExists {
						entry.Marks = append(entry.Marks, ruleCfg.Mark)
					}
				}

				// 处理 Tag
				tagExists := false
				for _, t := range entry.Tags {
					if t == targetTagStr {
						tagExists = true
						break
					}
				}
				if !tagExists {
					entry.Tags = append(entry.Tags, targetTagStr)
				}
			}
			totalRulesProcessed += len(rules)
		}

		// 构建新的 Matcher
		newMatcher := domain.NewMixMatcher[*MatchResult]()
		for ruleStr, result := range domainMap {
			if err := newMatcher.Add(ruleStr, result); err != nil {
				dm.logger.Warn("failed to add rule to mapper", zap.String("rule", ruleStr), zap.Error(err))
			}
		}

		dm.mu.Lock()
		dm.matcher = newMatcher
		dm.mu.Unlock()

		dm.logger.Info("rebuild finished",
			zap.Int("total_rules_processed", totalRulesProcessed),
			zap.Int("unique_domains", len(domainMap)),
			zap.Duration("duration", time.Since(start)))

		domainMap = nil 
		coremain.ManualGC() 
	}

	// 3. 触发防抖更新
	triggerUpdate := func() {
		dm.updateMu.Lock()
		defer dm.updateMu.Unlock()
		if dm.updateTimer != nil {
			dm.updateTimer.Stop()
		}
		dm.updateTimer = time.AfterFunc(1*time.Second, rebuild)
	}

	// 4. 注册订阅
	for t, p := range dm.providers {
		pluginTag := t
		p.Subscribe(func() {
			dm.logger.Info("upstream rule provider updated", zap.String("plugin", pluginTag))
			triggerUpdate()
		})
	}

	// 5. 初始构建
	rebuild()

	return dm, nil
}

func (dm *DomainMapper) Exec(ctx context.Context, qCtx *query_context.Context) error {
	q := qCtx.Q()
	if len(q.Question) == 0 {
		return nil
	}
	qname := q.Question[0].Name

	dm.mu.RLock()
	matcher := dm.matcher
	dm.mu.RUnlock()

	// O(1) 匹配
	result, ok := matcher.Match(qname)
	if ok && result != nil {
		// 命中逻辑：设置所有 Mark
		for _, mark := range result.Marks {
			qCtx.SetFastFlag(mark) // [修改] SetMark -> SetFastFlag
		}
		// 命中逻辑：设置所有 Tag
		if len(result.Tags) > 0 {
			val := strings.Join(result.Tags, "|")
			qCtx.StoreValue(query_context.KeyDomainSet, val)
		}
	} else {
		// [修改] 未命中逻辑：应用默认 Mark 和 默认 Tag
		if dm.defaultMark != 0 {
			qCtx.SetFastFlag(dm.defaultMark) // [修改] SetMark -> SetFastFlag
		}
		if dm.defaultTag != "" {
			qCtx.StoreValue(query_context.KeyDomainSet, dm.defaultTag)
		}
	}

	return nil
}
