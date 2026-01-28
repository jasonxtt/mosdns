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
	Tag       string `yaml:"tag"`
	Mark      uint8  `yaml:"mark"`
	OutputTag string `yaml:"output_tag"`
}

type Args struct {
	Rules       []RuleConfig `yaml:"rules"`
	DefaultMark uint8        `yaml:"default_mark"`
	DefaultTag  string       `yaml:"default_tag"`
}

type MatchResult struct {
	Marks []uint8
	Tags  []string
}

type DomainMapper struct {
	logger      *zap.Logger
	mu          sync.RWMutex
	matcher     *domain.MixMatcher[*MatchResult]
	updateMu    sync.Mutex
	updateTimer *time.Timer
	ruleConfigs []RuleConfig
	defaultMark uint8
	defaultTag  string
	providers   map[string]data_provider.RuleExporter
}

var _ sequence.Executable = (*DomainMapper)(nil)

func NewMapper(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)

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
		defaultTag:  cfg.DefaultTag,
		providers:   make(map[string]data_provider.RuleExporter),
	}

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
			return nil, fmt.Errorf("plugin %s does not support rule export", r.Tag)
		}
		dm.providers[r.Tag] = exporter
	}

rebuild := func() {
		dm.logger.Info("rebuilding with ultra-low fragmentation logic...")
		start := time.Now() // 记录开始时间

		// 1. 聚合阶段：使用位掩码和拼接字符串，完全不产生 MatchResult 对象
		markMap := make(map[string]uint64)
		tagMap := make(map[string]string)
		totalRules := 0

		for _, ruleCfg := range dm.ruleConfigs {
			provider, ok := dm.providers[ruleCfg.Tag]
			if !ok {
				continue
			}
			rules, err := provider.GetRules()
			if err != nil {
				continue
			}

			targetTag := ruleCfg.OutputTag
			if targetTag == "" {
				targetTag = ruleCfg.Tag
			}

			for _, ruleStr := range rules {
				if ruleCfg.Mark > 0 && ruleCfg.Mark <= 63 {
					markMap[ruleStr] |= (1 << (ruleCfg.Mark - 1))
				}
				oldTags := tagMap[ruleStr]
				if oldTags == "" {
					tagMap[ruleStr] = targetTag
				} else if !strings.Contains(oldTags, targetTag) {
					tagMap[ruleStr] = oldTags + "|" + targetTag
				}
			}
			totalRules += len(rules)
		}

		// 2. 构建阶段：对象池化
		pool := make(map[string]*MatchResult)
		newMatcher := domain.NewMixMatcher[*MatchResult]()

		for ruleStr, mask := range markMap {
			tagsStr := tagMap[ruleStr]
			sig := fmt.Sprintf("%d-%s", mask, tagsStr)
			
			res, exists := pool[sig]
			if !exists {
				res = &MatchResult{}
				for i := uint8(0); i < 64; i++ {
					if mask&(1<<i) != 0 {
						res.Marks = append(res.Marks, i+1)
					}
				}
				if tagsStr != "" {
					res.Tags = strings.Split(tagsStr, "|")
				}
				pool[sig] = res
			}
			newMatcher.Add(ruleStr, res)
		}

		// 3. 替换与日志
		dm.mu.Lock()
		dm.matcher = newMatcher
		dm.mu.Unlock()

		// 修复编译错误：在这里引用 start
		dm.logger.Info("rebuild finished",
			zap.Int("rules", totalRules),
			zap.Int("pooled", len(pool)),
			zap.Duration("duration", time.Since(start)))

		// 4. 清理
		markMap = nil
		tagMap = nil
		pool = nil

		go func() {
			time.Sleep(3 * time.Second)
			coremain.ManualGC()
			dm.logger.Info("domain_mapper: post-rebuild cleanup finished")
		}()
	}

	triggerUpdate := func() {
		dm.updateMu.Lock()
		defer dm.updateMu.Unlock()
		if dm.updateTimer != nil {
			dm.updateTimer.Stop()
		}
		dm.updateTimer = time.AfterFunc(1*time.Second, rebuild)
	}

	for t, p := range dm.providers {
		pluginTag := t
		p.Subscribe(func() {
			dm.logger.Info("upstream rule provider updated", zap.String("plugin", pluginTag))
			triggerUpdate()
		})
	}

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

	result, ok := matcher.Match(qname)
	if ok && result != nil {
		for _, mark := range result.Marks {
			qCtx.SetFastFlag(mark)
		}
		if len(result.Tags) > 0 {
			val := strings.Join(result.Tags, "|")
			qCtx.StoreValue(query_context.KeyDomainSet, val)
		}
	} else {
		if dm.defaultMark != 0 {
			qCtx.SetFastFlag(dm.defaultMark)
		}
		if dm.defaultTag != "" {
			qCtx.StoreValue(query_context.KeyDomainSet, dm.defaultTag)
		}
	}
	return nil
}
