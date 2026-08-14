package domain_mapper

import (
	"context"
	"fmt"
	"io"
	"regexp"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
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

type overlapRule struct {
	keyword string
	regex   *regexp.Regexp
	res     *MatchResult
}

type compiledMatcher struct {
	domainRules  *domain.MixMatcher[*MatchResult]
	overlapRules []overlapRule
}

type ruleAggregation struct {
	fastMarkMap map[string]uint64
	ctxMarkMap  map[string]map[uint32]struct{}
	tagMap      map[string]string
	sourceMap   map[string]string
	ruleOrder   []string
	valuedRules []matcher_adapter.ValuedRule
	totalRules  int
}

type hotEntry struct {
	name string
	res  *MatchResult
}

type goCandidate struct {
	matcher    *compiledMatcher
	hotEntries []hotEntry
	poolSize   int
}

type valuedGeneration struct {
	snapshot matcher_adapter.ValuedSnapshot
	disabled atomic.Bool
}

type DomainMapper struct {
	logger         *zap.Logger
	matcher        atomic.Value
	updateMu       sync.Mutex
	generationMu   sync.RWMutex
	updateTimer    *time.Timer
	closed         atomic.Bool
	ruleConfigs    []RuleConfig
	defaultMark    uint8
	defaultCtxMark uint32
	defaultTag     string
	providers      map[string]data_provider.RuleExporter
	runBit         uint8
	rustGeneration *valuedGeneration

	hotMap sync.Map
}

var _ sequence.Executable = (*DomainMapper)(nil)
var _ io.Closer = (*DomainMapper)(nil)

var valuedSnapshotBuilder = matcher_adapter.BuildValuedDomainSnapshot

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
	dm.matcher.Store(&compiledMatcher{domainRules: domain.NewMixMatcher[*MatchResult]()})

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
		dm.updateMu.Lock()
		defer dm.updateMu.Unlock()

		dm.logger.Info("rebuilding domain_mapper with logic inheritance...")
		start := time.Now()
		agg := dm.aggregateRules()
		candidate := dm.buildGoCandidate(agg)
		var rustSnapshot matcher_adapter.ValuedSnapshot
		var rustErr error
		if valuedSnapshotBuilder != nil {
			rustSnapshot, rustErr = valuedSnapshotBuilder(agg.valuedRules)
		}
		if rustErr != nil {
			if dm.logger != nil {
				dm.logger.Warn("valued Rust domain_mapper build failed; using Go candidate",
					zap.String("plugin", PluginType),
					zap.Error(rustErr),
					zap.Int("rules", len(agg.valuedRules)))
			}
			if rustSnapshot != nil {
				_ = rustSnapshot.Close()
				rustSnapshot = nil
			}
		}
		dm.publishGeneration(candidate, rustSnapshot)
		dm.logger.Info("rebuild finished",
			zap.Int("rules", agg.totalRules),
			zap.Int("pooled_results", candidate.poolSize),
			zap.Int("hot_entries", len(candidate.hotEntries)),
			zap.Duration("duration", time.Since(start)))
		go func() {
			time.Sleep(3 * time.Second)
			coremain.ManualGC()
		}()
	}

	triggerUpdate := func() {
		dm.updateMu.Lock()
		defer dm.updateMu.Unlock()
		if dm.closed.Load() {
			return
		}
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

// Close stops pending rebuilds and retires the selected Rust generation. It is
// safe to call more than once and never closes a replacement generation.
func (dm *DomainMapper) Close() error {
	dm.updateMu.Lock()
	if dm.closed.Load() {
		dm.updateMu.Unlock()
		return nil
	}
	dm.closed.Store(true)
	if dm.updateTimer != nil {
		dm.updateTimer.Stop()
		dm.updateTimer = nil
	}
	dm.updateMu.Unlock()

	dm.generationMu.Lock()
	oldGeneration := dm.rustGeneration
	dm.rustGeneration = nil
	dm.generationMu.Unlock()
	if oldGeneration == nil || oldGeneration.snapshot == nil {
		return nil
	}
	return oldGeneration.snapshot.Close()
}

func (dm *DomainMapper) aggregateRules() ruleAggregation {
	agg := ruleAggregation{
		fastMarkMap: make(map[string]uint64),
		ctxMarkMap:  make(map[string]map[uint32]struct{}),
		tagMap:      make(map[string]string),
		sourceMap:   make(map[string]string),
		ruleOrder:   make([]string, 0),
		valuedRules: make([]matcher_adapter.ValuedRule, 0),
	}
	ruleSeen := make(map[string]struct{})
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
			if _, seen := ruleSeen[ruleStr]; !seen {
				ruleSeen[ruleStr] = struct{}{}
				agg.ruleOrder = append(agg.ruleOrder, ruleStr)
			}
			var fastMask uint64
			if ruleCfg.Mark > 0 && ruleCfg.Mark <= 63 {
				fastMask = 1 << (ruleCfg.Mark - 1)
				agg.fastMarkMap[ruleStr] |= fastMask
			}
			if ruleCfg.CtxMark > 0 {
				if agg.ctxMarkMap[ruleStr] == nil {
					agg.ctxMarkMap[ruleStr] = make(map[uint32]struct{})
				}
				agg.ctxMarkMap[ruleStr][ruleCfg.CtxMark] = struct{}{}
			}
			agg.tagMap[ruleStr] = appendJoinedValue(agg.tagMap[ruleStr], targetTag)
			agg.sourceMap[ruleStr] = appendJoinedValue(agg.sourceMap[ruleStr], sourceName)

			var ctxMarks []uint32
			if ruleCfg.CtxMark > 0 {
				ctxMarks = []uint32{ruleCfg.CtxMark}
			}
			agg.valuedRules = append(agg.valuedRules, matcher_adapter.ValuedRule{
				Rule:          ruleStr,
				FastMarks:     fastMask,
				CtxMarks:      ctxMarks,
				JoinedTags:    targetTag,
				JoinedSources: sourceName,
			})
		}
		agg.totalRules += len(ruleEntries)
	}
	return agg
}

func (dm *DomainMapper) buildGoCandidate(agg ruleAggregation) goCandidate {
	for _, ruleStr := range agg.ruleOrder {
		dotPos := strings.Index(ruleStr, ":")
		if dotPos == -1 {
			continue
		}
		originalDName := ruleStr[dotPos+1:]
		dName := originalDName

		inherit := func(ancestorKey string) {
			if aMask, ok := agg.fastMarkMap[ancestorKey]; ok {
				agg.fastMarkMap[ruleStr] |= aMask
			}
			if aMarks, ok := agg.ctxMarkMap[ancestorKey]; ok {
				if agg.ctxMarkMap[ruleStr] == nil {
					agg.ctxMarkMap[ruleStr] = make(map[uint32]struct{})
				}
				for mark := range aMarks {
					agg.ctxMarkMap[ruleStr][mark] = struct{}{}
				}
			}
			agg.tagMap[ruleStr] = appendJoinedValue(agg.tagMap[ruleStr], agg.tagMap[ancestorKey])
			agg.sourceMap[ruleStr] = appendJoinedValue(agg.sourceMap[ruleStr], agg.sourceMap[ancestorKey])
		}

		if strings.HasPrefix(ruleStr, "full:") {
			inherit("domain:" + originalDName)
		}
		for {
			nextDot := strings.Index(dName, ".")
			if nextDot == -1 {
				break
			}
			dName = dName[nextDot+1:]
			inherit("domain:" + dName)
		}
	}

	pool := make(map[string]*MatchResult)
	domainRules := domain.NewMixMatcher[*MatchResult]()
	overlapRules := make([]overlapRule, 0, 64)
	hotEntries := make([]hotEntry, 0)
	for _, ruleStr := range agg.ruleOrder {
		fastMask := agg.fastMarkMap[ruleStr]
		tagsStr := agg.tagMap[ruleStr]
		sourcesStr := agg.sourceMap[ruleStr]
		ctxMarks := agg.ctxMarkMap[ruleStr]
		sig := fmt.Sprintf("%d-%v-%s-%s", fastMask, sortedCtxMarks(ctxMarks), tagsStr, sourcesStr)
		res, exists := pool[sig]
		if !exists {
			res = &MatchResult{JoinedTags: tagsStr, JoinedSources: sourcesStr}
			for i := uint8(0); i < 63; i++ {
				if fastMask&(1<<i) != 0 {
					res.FastMarks = append(res.FastMarks, i+1)
				}
			}
			res.CtxMarks = sortedCtxMarks(ctxMarks)
			pool[sig] = res
		}

		switch {
		case strings.HasPrefix(ruleStr, "full:"):
			name := strings.TrimPrefix(ruleStr, "full:")
			if !strings.HasSuffix(name, ".") {
				name += "."
			}
			hotEntries = append(hotEntries, hotEntry{name: name, res: res})
		case strings.HasPrefix(ruleStr, "keyword:"):
			overlapRules = append(overlapRules, overlapRule{
				keyword: domain.NormalizeDomain(strings.TrimPrefix(ruleStr, "keyword:")),
				res:     res,
			})
		case strings.HasPrefix(ruleStr, "regexp:"):
			pattern := strings.TrimPrefix(ruleStr, "regexp:")
			compiled, err := regexp.Compile(pattern)
			if err != nil {
				if dm.logger != nil {
					dm.logger.Warn("skip invalid regexp rule", zap.String("rule", ruleStr), zap.Error(err))
				}
				continue
			}
			overlapRules = append(overlapRules, overlapRule{regex: compiled, res: res})
		default:
			if err := domainRules.Add(ruleStr, res); err != nil && dm.logger != nil {
				dm.logger.Warn("skip invalid domain rule", zap.String("rule", ruleStr), zap.Error(err))
			}
		}
	}
	return goCandidate{
		matcher:    &compiledMatcher{domainRules: domainRules, overlapRules: overlapRules},
		hotEntries: hotEntries,
		poolSize:   len(pool),
	}
}

func (dm *DomainMapper) publishGeneration(candidate goCandidate, rustSnapshot matcher_adapter.ValuedSnapshot) {
	dm.generationMu.Lock()
	if dm.closed.Load() {
		dm.generationMu.Unlock()
		if rustSnapshot != nil {
			_ = rustSnapshot.Close()
		}
		return
	}
	oldGeneration := dm.rustGeneration
	dm.matcher.Store(candidate.matcher)
	dm.hotMap.Range(func(key, value any) bool {
		dm.hotMap.Delete(key)
		return true
	})
	for _, entry := range candidate.hotEntries {
		dm.hotMap.Store(entry.name, entry.res)
	}
	if rustSnapshot == nil {
		dm.rustGeneration = nil
	} else {
		dm.rustGeneration = &valuedGeneration{snapshot: rustSnapshot}
	}
	dm.generationMu.Unlock()

	if oldGeneration != nil && oldGeneration.snapshot != nil {
		if err := oldGeneration.snapshot.Close(); err != nil && dm.logger != nil {
			dm.logger.Warn("close retired Rust domain_mapper generation failed", zap.Error(err))
		}
	}
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

func cloneMatchResult(res *MatchResult) *MatchResult {
	if res == nil {
		return nil
	}
	cloned := &MatchResult{
		JoinedTags:    res.JoinedTags,
		JoinedSources: res.JoinedSources,
	}
	if len(res.FastMarks) > 0 {
		cloned.FastMarks = append(cloned.FastMarks, res.FastMarks...)
	}
	if len(res.CtxMarks) > 0 {
		cloned.CtxMarks = append(cloned.CtxMarks, res.CtxMarks...)
	}
	return cloned
}

func mergeMatchResult(dst, src *MatchResult) *MatchResult {
	if src == nil {
		return dst
	}
	if dst == nil {
		return cloneMatchResult(src)
	}
	for _, mark := range src.FastMarks {
		if !slices.Contains(dst.FastMarks, mark) {
			dst.FastMarks = append(dst.FastMarks, mark)
		}
	}
	for _, mark := range src.CtxMarks {
		if !slices.Contains(dst.CtxMarks, mark) {
			dst.CtxMarks = append(dst.CtxMarks, mark)
		}
	}
	dst.JoinedTags = appendJoinedValue(dst.JoinedTags, src.JoinedTags)
	dst.JoinedSources = appendJoinedValue(dst.JoinedSources, src.JoinedSources)
	return dst
}

func (cm *compiledMatcher) match(name string) (*MatchResult, bool) {
	if cm == nil {
		return nil, false
	}

	var merged *MatchResult
	if cm.domainRules != nil {
		if res, ok := cm.domainRules.Match(name); ok && res != nil {
			merged = cloneMatchResult(res)
		}
	}

	if len(cm.overlapRules) == 0 {
		return merged, merged != nil
	}

	normalizedName := domain.NormalizeDomain(name)
	for _, rule := range cm.overlapRules {
		switch {
		case rule.keyword != "":
			if strings.Contains(normalizedName, rule.keyword) {
				merged = mergeMatchResult(merged, rule.res)
			}
		case rule.regex != nil:
			if rule.regex.MatchString(normalizedName) {
				merged = mergeMatchResult(merged, rule.res)
			}
		}
	}
	return merged, merged != nil
}

func (dm *DomainMapper) lookupMatchResult(name string) (*MatchResult, bool) {
	dm.generationMu.RLock()
	defer dm.generationMu.RUnlock()
	return dm.lookupMatchResultLocked(name)
}

func (dm *DomainMapper) lookupMatchResultLocked(name string) (*MatchResult, bool) {
	var matcher *compiledMatcher
	if value := dm.matcher.Load(); value != nil {
		matcher, _ = value.(*compiledMatcher)
	}
	var merged *MatchResult
	if val, ok := dm.hotMap.Load(name); ok {
		merged = cloneMatchResult(val.(*MatchResult))
	}

	usedRust := false
	if generation := dm.rustGeneration; generation != nil && generation.snapshot != nil && !generation.disabled.Load() {
		valued, err := generation.snapshot.Match(name)
		if err != nil {
			if generation.disabled.CompareAndSwap(false, true) && dm.logger != nil {
				dm.logger.Warn("disable failed Rust domain_mapper generation; using Go fallback",
					zap.String("plugin", PluginType),
					zap.Error(err))
			}
		} else {
			usedRust = true
			if valued.Matched {
				merged = mergeMatchResult(merged, valuedResultToMatchResult(valued))
			}
		}
	}
	if !usedRust && matcher != nil {
		if res, ok := matcher.match(name); ok {
			merged = mergeMatchResult(merged, res)
		}
	}
	return merged, merged != nil
}

func valuedResultToMatchResult(result matcher_adapter.ValuedResult) *MatchResult {
	return &MatchResult{
		FastMarks:     append([]uint8(nil), result.FastMarks...),
		CtxMarks:      append([]uint32(nil), result.CtxMarks...),
		JoinedTags:    result.JoinedTags,
		JoinedSources: result.JoinedSources,
	}
}

func (dm *DomainMapper) QuickAdd(domainName string, marks []uint8, joinedTags string) {
	dm.generationMu.Lock()
	defer dm.generationMu.Unlock()

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

			if res, matchOk := dm.lookupMatchResultLocked(key); matchOk && res != nil {
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
	if res, ok := dm.lookupMatchResult(qname); ok && res != nil {
		return res.FastMarks, res.JoinedTags, true
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
	result, ok := dm.lookupMatchResult(name)
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
		result, ok := dm.lookupMatchResult(name)
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
