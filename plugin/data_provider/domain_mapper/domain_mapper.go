package domain_mapper

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"go.uber.org/zap"
)

const PluginType = "domain_mapper"

var nextRunBit atomic.Uint32

func init() {
	nextRunBit.Store(64)
	coremain.RegNewPluginFunc(PluginType, NewMapper, func() any { return new(Args) })
}

type RuleConfig struct {
	Tag       string `yaml:"tag"`
	Mark      uint8  `yaml:"mark"`
	CtxMark   uint32 `yaml:"ctx_mark"`
	OutputTag string `yaml:"output_tag"`
}

type Args struct {
	Rules          []RuleConfig `yaml:"rules"`
	DefaultMark    uint8        `yaml:"default_mark"`
	DefaultCtxMark uint32       `yaml:"default_ctx_mark"`
	DefaultTag     string       `yaml:"default_tag"`
}

type MatchResult struct {
	FastMarks     []uint8
	CtxMarks      []uint32
	JoinedTags    string
	JoinedSources string
}

type DomainMapper struct {
	logger         *zap.Logger
	matcher        atomic.Value
	updateMu       sync.Mutex
	updateTimer    *time.Timer
	ruleConfigs    []RuleConfig
	defaultMark    uint8
	defaultCtxMark uint32
	defaultTag     string
	providers      map[string]data_provider.RuleExporter
	runBit         uint8

	hotMap sync.Map
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
		logger:         bp.L(),
		ruleConfigs:    cfg.Rules,
		defaultMark:    cfg.DefaultMark,
		defaultCtxMark: cfg.DefaultCtxMark,
		defaultTag:     cfg.DefaultTag,
		providers:      make(map[string]data_provider.RuleExporter),
		runBit:         uint8(nextRunBit.Add(^uint32(0))),
	}
	dm.matcher.Store(domain.NewMixMatcher[*MatchResult]())

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
		dm.logger.Info("rebuilding domain_mapper with logic inheritance...")
		start := time.Now()

		fastMarkMap := make(map[string]uint64)
		ctxMarkMap := make(map[string]map[uint32]struct{})
		tagMap := make(map[string]string)
		sourceMap := make(map[string]string)
		totalRules := 0

		for _, ruleCfg := range dm.ruleConfigs {
			provider, ok := dm.providers[ruleCfg.Tag]
			if !ok {
				continue
			}
			ruleEntries, err := getRuleEntriesFromProvider(ruleCfg, provider)
			if err != nil {
				continue
			}

			targetTag := ruleCfg.OutputTag
			if targetTag == "" {
				targetTag = ruleCfg.Tag
			}

			for _, ruleEntry := range ruleEntries {
				ruleStr := ruleEntry.Rule
				sourceName := firstNonEmpty(ruleEntry.SourceName, ruleCfg.Tag)
				if ruleCfg.Mark > 0 && ruleCfg.Mark <= 63 {
					fastMarkMap[ruleStr] |= (1 << (ruleCfg.Mark - 1))
				}
				if ruleCfg.CtxMark > 0 {
					if ctxMarkMap[ruleStr] == nil {
						ctxMarkMap[ruleStr] = make(map[uint32]struct{})
					}
					ctxMarkMap[ruleStr][ruleCfg.CtxMark] = struct{}{}
				}
				tagMap[ruleStr] = appendJoinedValue(tagMap[ruleStr], targetTag)
				sourceMap[ruleStr] = appendJoinedValue(sourceMap[ruleStr], sourceName)
			}
			totalRules += len(ruleEntries)
		}

		for _, ruleStr := range collectRuleKeys(fastMarkMap, ctxMarkMap, tagMap, sourceMap) {
			dotPos := strings.Index(ruleStr, ":")
			if dotPos == -1 {
				continue
			}
			originalDName := ruleStr[dotPos+1:]
			dName := originalDName

			if strings.HasPrefix(ruleStr, "full:") {
				ancestorKey := "domain:" + originalDName
				if aMask, ok := fastMarkMap[ancestorKey]; ok {
					fastMarkMap[ruleStr] |= aMask
				}
				if aMarks, ok := ctxMarkMap[ancestorKey]; ok {
					if ctxMarkMap[ruleStr] == nil {
						ctxMarkMap[ruleStr] = make(map[uint32]struct{})
					}
					for m := range aMarks {
						ctxMarkMap[ruleStr][m] = struct{}{}
					}
				}
				aTags := tagMap[ancestorKey]
				if aTags != "" {
					tagMap[ruleStr] = appendJoinedValue(tagMap[ruleStr], aTags)
				}
				aSources := sourceMap[ancestorKey]
				if aSources != "" {
					sourceMap[ruleStr] = appendJoinedValue(sourceMap[ruleStr], aSources)
				}
			}

			for {
				nextDot := strings.Index(dName, ".")
				if nextDot == -1 {
					break
				}
				dName = dName[nextDot+1:]
				ancestorKey := "domain:" + dName

				if aMask, ok := fastMarkMap[ancestorKey]; ok {
					fastMarkMap[ruleStr] |= aMask
				}
				if aMarks, ok := ctxMarkMap[ancestorKey]; ok {
					if ctxMarkMap[ruleStr] == nil {
						ctxMarkMap[ruleStr] = make(map[uint32]struct{})
					}
					for m := range aMarks {
						ctxMarkMap[ruleStr][m] = struct{}{}
					}
				}
				aTags := tagMap[ancestorKey]
				if aTags != "" {
					tagMap[ruleStr] = appendJoinedValue(tagMap[ruleStr], aTags)
				}
				aSources := sourceMap[ancestorKey]
				if aSources != "" {
					sourceMap[ruleStr] = appendJoinedValue(sourceMap[ruleStr], aSources)
				}
			}
		}

		pool := make(map[string]*MatchResult)
		newMatcher := domain.NewMixMatcher[*MatchResult]()

		type hotEntry struct {
			name string
			res  *MatchResult
		}
		var hotEntries []hotEntry

		for _, ruleStr := range collectRuleKeys(fastMarkMap, ctxMarkMap, tagMap, sourceMap) {
			fastMask := fastMarkMap[ruleStr]
			tagsStr := tagMap[ruleStr]
			sourcesStr := sourceMap[ruleStr]
			ctxMarks := ctxMarkMap[ruleStr]
			sig := fmt.Sprintf("%d-%v-%s-%s", fastMask, sortedCtxMarks(ctxMarks), tagsStr, sourcesStr)

			res, exists := pool[sig]
			if !exists {
				res = &MatchResult{
					JoinedTags:    tagsStr,
					JoinedSources: sourcesStr,
				}
				for i := uint8(0); i < 64; i++ {
					if fastMask&(1<<i) != 0 {
						res.FastMarks = append(res.FastMarks, i+1)
					}
				}
				res.CtxMarks = sortedCtxMarks(ctxMarks)
				pool[sig] = res
			}

			if strings.HasPrefix(ruleStr, "full:") {
				name := strings.TrimPrefix(ruleStr, "full:")
				if !strings.HasSuffix(name, ".") {
					name += "."
				}
				hotEntries = append(hotEntries, hotEntry{name: name, res: res})
			} else {
				newMatcher.Add(ruleStr, res)
			}
		}

		dm.matcher.Store(newMatcher)
		dm.hotMap.Range(func(key, value any) bool {
			dm.hotMap.Delete(key)
			return true
		})

		for _, e := range hotEntries {
			dm.hotMap.Store(e.name, e.res)
		}

		dm.logger.Info("rebuild finished",
			zap.Int("rules", totalRules),
			zap.Int("pooled_results", len(pool)),
			zap.Int("hot_entries", len(hotEntries)),
			zap.Duration("duration", time.Since(start)))

		fastMarkMap = nil
		ctxMarkMap = nil
		tagMap = nil
		sourceMap = nil
		pool = nil
		hotEntries = nil

		go func() {
			time.Sleep(3 * time.Second)
			coremain.ManualGC()
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
			dm.logger.Info("upstream rule provider updated", zap.String(PluginType, pluginTag))
			triggerUpdate()
		})
	}

	rebuild()
	return dm, nil
}

func sortedCtxMarks(m map[uint32]struct{}) []uint32 {
	if len(m) == 0 {
		return nil
	}
	out := make([]uint32, 0, len(m))
	for v := range m {
		out = append(out, v)
	}
	for i := 0; i < len(out)-1; i++ {
		for j := i + 1; j < len(out); j++ {
			if out[j] < out[i] {
				out[i], out[j] = out[j], out[i]
			}
		}
	}
	return out
}

func getRuleEntriesFromProvider(ruleCfg RuleConfig, provider data_provider.RuleExporter) ([]data_provider.RuleEntry, error) {
	if detailedProvider, ok := provider.(data_provider.DetailedRuleExporter); ok {
		return detailedProvider.GetRuleEntries()
	}

	rules, err := provider.GetRules()
	if err != nil {
		return nil, err
	}

	entries := make([]data_provider.RuleEntry, 0, len(rules))
	for _, rule := range rules {
		entries = append(entries, data_provider.RuleEntry{
			Rule:       rule,
			SourceName: ruleCfg.Tag,
		})
	}
	return entries, nil
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}

func appendJoinedValue(joined, value string) string {
	for _, part := range strings.Split(value, "|") {
		part = strings.TrimSpace(part)
		if part == "" || hasJoinedValue(joined, part) {
			continue
		}
		if joined == "" {
			joined = part
		} else {
			joined += "|" + part
		}
	}
	return joined
}

func hasJoinedValue(joined, target string) bool {
	for _, part := range strings.Split(joined, "|") {
		if strings.TrimSpace(part) == target {
			return true
		}
	}
	return false
}

func collectRuleKeys(
	fastMarkMap map[string]uint64,
	ctxMarkMap map[string]map[uint32]struct{},
	tagMap map[string]string,
	sourceMap map[string]string,
) []string {
	keys := make(map[string]struct{}, len(fastMarkMap)+len(ctxMarkMap)+len(tagMap)+len(sourceMap))
	for rule := range fastMarkMap {
		keys[rule] = struct{}{}
	}
	for rule := range ctxMarkMap {
		keys[rule] = struct{}{}
	}
	for rule := range tagMap {
		keys[rule] = struct{}{}
	}
	for rule := range sourceMap {
		keys[rule] = struct{}{}
	}
	out := make([]string, 0, len(keys))
	for rule := range keys {
		out = append(out, rule)
	}
	return out
}

func (dm *DomainMapper) QuickAdd(domainName string, marks []uint8, joinedTags string) {
	key := domainName
	if !strings.HasSuffix(key, ".") {
		key = key + "."
	}

	for {
		val, ok := dm.hotMap.Load(key)
		if !ok {
			newMarks := make([]uint8, len(marks))
			copy(newMarks, marks)
			newTags := joinedTags
			var newSources string

			matcher := dm.matcher.Load().(*domain.MixMatcher[*MatchResult])
			if res, matchOk := matcher.Match(key); matchOk && res != nil {
				for _, m := range res.FastMarks {
					found := false
					for _, existingM := range newMarks {
						if existingM == m {
							found = true
							break
						}
					}
					if !found {
						newMarks = append(newMarks, m)
					}
				}
				if res.JoinedTags != "" {
					newTags = appendJoinedValue(newTags, res.JoinedTags)
				}
				newSources = appendJoinedValue(newSources, res.JoinedSources)
			}
			actual, loaded := dm.hotMap.LoadOrStore(key, &MatchResult{
				FastMarks:     newMarks,
				JoinedTags:    newTags,
				JoinedSources: newSources,
			})
			if !loaded {
				return
			}
			val = actual
		}

		oldRes := val.(*MatchResult)
		newMarks := make([]uint8, len(oldRes.FastMarks))
		copy(newMarks, oldRes.FastMarks)
		for _, m := range marks {
			found := false
			for _, om := range oldRes.FastMarks {
				if om == m {
					found = true
					break
				}
			}
			if !found {
				newMarks = append(newMarks, m)
			}
		}

		newTags := oldRes.JoinedTags
		if joinedTags != "" {
			newTags = appendJoinedValue(newTags, joinedTags)
		}

		newRes := &MatchResult{
			FastMarks:     newMarks,
			CtxMarks:      oldRes.CtxMarks,
			JoinedTags:    newTags,
			JoinedSources: oldRes.JoinedSources,
		}

		if dm.hotMap.CompareAndSwap(key, val, newRes) {
			return
		}
	}
}

func (dm *DomainMapper) FastMatch(qname string) ([]uint8, string, bool) {
	if val, ok := dm.hotMap.Load(qname); ok {
		res := val.(*MatchResult)
		return res.FastMarks, res.JoinedTags, true
	}
	matcher := dm.matcher.Load().(*domain.MixMatcher[*MatchResult])
	result, ok := matcher.Match(qname)
	if ok && result != nil {
		return result.FastMarks, result.JoinedTags, true
	}
	return nil, "", false
}

func (dm *DomainMapper) GetRunBit() uint8 {
	return dm.runBit
}

func applyMatchResult(qCtx *query_context.Context, res *MatchResult) {
	for _, mark := range res.FastMarks {
		qCtx.SetFastFlag(mark)
	}
	for _, mark := range res.CtxMarks {
		qCtx.SetMark(mark)
	}
	if res.JoinedTags != "" {
		qCtx.StoreValue(query_context.KeyDomainSet, res.JoinedTags)
	}
	if res.JoinedSources != "" {
		qCtx.StoreValue(query_context.KeyMatchedRuleSource, res.JoinedSources)
	}
}

func (dm *DomainMapper) Exec(ctx context.Context, qCtx *query_context.Context) error {
	if qCtx.HasFastFlag(dm.runBit) {
		return nil
	}

	q := qCtx.Q()
	if q == nil || len(q.Question) == 0 {
		return nil
	}

	name := q.Question[0].Name
	if val, ok := dm.hotMap.Load(name); ok {
		applyMatchResult(qCtx, val.(*MatchResult))
		return nil
	}

	matcher := dm.matcher.Load().(*domain.MixMatcher[*MatchResult])

	result, ok := matcher.Match(name)
	if ok && result != nil {
		applyMatchResult(qCtx, result)
	} else {
		if dm.defaultMark != 0 {
			qCtx.SetFastFlag(dm.defaultMark)
		}
		if dm.defaultCtxMark != 0 {
			qCtx.SetMark(dm.defaultCtxMark)
		}
		if dm.defaultTag != "" {
			qCtx.StoreValue(query_context.KeyDomainSet, dm.defaultTag)
		}
	}
	return nil
}

func (dm *DomainMapper) GetFastExec() func(ctx context.Context, qCtx *query_context.Context) error {
	defMark := dm.defaultMark
	defCtxMark := dm.defaultCtxMark
	defTag := dm.defaultTag
	rBit := dm.runBit
	return func(ctx context.Context, qCtx *query_context.Context) error {
		if qCtx.HasFastFlag(rBit) {
			return nil
		}

		q := qCtx.Q()
		if q == nil || len(q.Question) == 0 {
			return nil
		}

		name := q.Question[0].Name
		if val, ok := dm.hotMap.Load(name); ok {
			applyMatchResult(qCtx, val.(*MatchResult))
			return nil
		}

		matcher := dm.matcher.Load().(*domain.MixMatcher[*MatchResult])
		result, ok := matcher.Match(name)
		if ok && result != nil {
			applyMatchResult(qCtx, result)
		} else {
			if defMark != 0 {
				qCtx.SetFastFlag(defMark)
			}
			if defCtxMark != 0 {
				qCtx.SetMark(defCtxMark)
			}
			if defTag != "" {
				qCtx.StoreValue(query_context.KeyDomainSet, defTag)
			}
		}
		return nil
	}
}
