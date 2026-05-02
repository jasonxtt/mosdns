package coremain

import (
	"container/heap"
	"container/list"
	"encoding/json"
	"math"
	"net"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/miekg/dns"
	"go.uber.org/zap"
)

// --- Optimized String Interning with Constant Fast-Path ---
const lruCacheSize = 16384

type lruEntry struct {
	key   string
	value string
}

type lruCache struct {
	mu       sync.Mutex
	capacity int
	cache    map[string]*list.Element
	ll       *list.List
}

func newLRUCache(capacity int) *lruCache {
	if capacity <= 0 {
		capacity = lruCacheSize
	}
	return &lruCache{
		capacity: capacity,
		cache:    make(map[string]*list.Element, capacity),
		ll:       list.New(),
	}
}

func (l *lruCache) Get(key string) (value string, ok bool) {
	l.mu.Lock()
	defer l.mu.Unlock()

	if elem, hit := l.cache[key]; hit {
		l.ll.MoveToFront(elem)
		return elem.Value.(*lruEntry).value, true
	}
	return "", false
}

func (l *lruCache) Put(key, value string) {
	l.mu.Lock()
	defer l.mu.Unlock()

	if elem, hit := l.cache[key]; hit {
		l.ll.MoveToFront(elem)
		elem.Value.(*lruEntry).value = value
		return
	}

	if l.ll.Len() >= l.capacity {
		oldest := l.ll.Back()
		if oldest != nil {
			l.ll.Remove(oldest)
			delete(l.cache, oldest.Value.(*lruEntry).key)
		}
	}

	elem := l.ll.PushFront(&lruEntry{key: key, value: value})
	l.cache[key] = elem
}

var globalStringLRU = newLRUCache(lruCacheSize)

func internString(s string) string {
	// OPTIMIZATION: Fast-path for common DNS constants to bypass LRU lock entirely.
	switch s {
	case "A", "AAAA", "CNAME", "TXT", "NS", "MX", "PTR", "SOA", "SRV", "HTTPS", "SVCB",
		"NOERROR", "FORMERR", "SERVFAIL", "NXDOMAIN", "NOTIMP", "REFUSED", "IN",
		"NO_RESPONSE", "unmatched_rule":
		return s
	}

	if val, ok := globalStringLRU.Get(s); ok {
		return val
	}
	globalStringLRU.Put(s, s)
	return s
}

type auditContext struct {
	Ctx                *query_context.Context
	ProcessingDuration time.Duration
}

// Pool for auditContext to minimize GC overhead during high-load periods.
var auditCtxPool = sync.Pool{
	New: func() any { return new(auditContext) },
}

type AnswerDetail struct {
	Type string `json:"type"`
	TTL  uint32 `json:"ttl"`
	Data string `json:"data"`
}

type AuditLog struct {
	ClientIP          string         `json:"client_ip"`
	QueryType         string         `json:"query_type"`
	QueryName         string         `json:"query_name"`
	QueryClass        string         `json:"query_class"`
	QueryTime         time.Time      `json:"query_time"`
	DurationMs        float64        `json:"duration_ms"`
	TraceID           string         `json:"trace_id"`
	ResponseCode      string         `json:"response_code"`
	ResponseFlags     ResponseFlags  `json:"response_flags"`
	Answers           []AnswerDetail `json:"answers"`
	DomainSet         string         `json:"domain_set,omitempty"`
	EffectiveTag      string         `json:"effective_tag,omitempty"`
	MatchedGroup      string         `json:"matched_group,omitempty"`
	FinalSequence     string         `json:"final_sequence,omitempty"`
	FinalUpstream     string         `json:"final_upstream,omitempty"`
	UpstreamTargets   string         `json:"upstream_targets,omitempty"`
	SelectedUpstream  string         `json:"selected_upstream,omitempty"`
	MatchedRuleSource string         `json:"matched_rule_source,omitempty"`
}

type ResponseFlags struct {
	AA bool `json:"aa"`
	TC bool `json:"tc"`
	RA bool `json:"ra"`
}

const (
	defaultAuditCapacity   = 100000
	maxAuditCapacity       = 400000
	slowestQueriesCapacity = 300
	auditChannelCapacity   = 10240
	auditSettingsFilename  = "audit_settings.json"
)

type AuditSettings struct {
	Capacity int `json:"capacity"`
}

type slowestQueryHeap []AuditLog

func (h slowestQueryHeap) Len() int           { return len(h) }
func (h slowestQueryHeap) Less(i, j int) bool { return h[i].DurationMs < h[j].DurationMs }
func (h slowestQueryHeap) Swap(i, j int)      { h[i], h[j] = h[j], h[i] }

func (h *slowestQueryHeap) Push(x any) {
	*h = append(*h, x.(AuditLog))
}

func (h *slowestQueryHeap) Pop() any {
	old := *h
	n := len(old)
	x := old[n-1]
	*h = old[0 : n-1]
	return x
}

type AuditCollector struct {
	mu                 sync.RWMutex
	capturing          bool
	capacity           int
	logs               []AuditLog
	head               int
	slowestQueries     slowestQueryHeap
	domainCounts       map[string]int
	clientCounts       map[string]int
	domainSetCounts    map[string]int
	effectiveTagCounts map[string]int
	totalQueryCount    uint64
	totalQueryDuration float64
	ctxChan            chan *auditContext
	workerDone         chan struct{}

	// Lazy Sync Control
	lastSyncTime time.Time

	// Global Statistics for monitoring without mutex pressure
	totalQueryCountGlobal    atomic.Uint64
	totalQueryDurationGlobal atomic.Uint64 // Stored in microseconds
}

var GlobalAuditCollector = NewAuditCollector(defaultAuditCapacity)

func InitializeAuditCollector(configBaseDir string) {
	initialCapacity := defaultAuditCapacity
	settingsPath := filepath.Join(configBaseDir, auditSettingsFilename)
	settings := &AuditSettings{}
	data, err := os.ReadFile(settingsPath)

	if err == nil {
		if json.Unmarshal(data, settings) == nil {
			initialCapacity = settings.Capacity
			if initialCapacity < 0 {
				initialCapacity = 0
			}
			if initialCapacity > maxAuditCapacity {
				initialCapacity = maxAuditCapacity
			}
			mlog.S().Infof("Loaded audit log capacity: %d", initialCapacity)
		}
	}

	if initialCapacity != defaultAuditCapacity {
		GlobalAuditCollector = NewAuditCollector(initialCapacity)
	}
}

func NewAuditCollector(capacity int) *AuditCollector {
	c := &AuditCollector{
		capturing:          true,
		capacity:           capacity,
		logs:               make([]AuditLog, 0, capacity),
		slowestQueries:     make(slowestQueryHeap, 0, slowestQueriesCapacity),
		domainCounts:       make(map[string]int),
		clientCounts:       make(map[string]int),
		domainSetCounts:    make(map[string]int),
		effectiveTagCounts: make(map[string]int),
		totalQueryCount:    0,
		totalQueryDuration: 0.0,
		ctxChan:            make(chan *auditContext, auditChannelCapacity),
		workerDone:         make(chan struct{}),
	}
	heap.Init(&c.slowestQueries)
	return c
}

func (c *AuditCollector) StartWorker() {
	go c.worker()
}

func (c *AuditCollector) StopWorker() {
	close(c.ctxChan)
	<-c.workerDone
}

func (c *AuditCollector) worker() {
	defer close(c.workerDone)

	// Batch processing slice to reduce lock contention frequency
	batch := make([]*auditContext, 0, 256)

	for {
		batch = batch[:0]

		wrappedCtx, ok := <-c.ctxChan
		if !ok {
			return
		}
		batch = append(batch, wrappedCtx)

		// Non-blocking drain to fill the batch
	drainLoop:
		for len(batch) < cap(batch) {
			select {
			case nextItem, ok := <-c.ctxChan:
				if !ok {
					break drainLoop
				}
				batch = append(batch, nextItem)
			default:
				break drainLoop
			}
		}

		c.processBatch(batch)

		for _, item := range batch {
			auditCtxPool.Put(item)
		}
	}
}

func (c *AuditCollector) processBatch(batch []*auditContext) {
	c.mu.Lock()
	defer c.mu.Unlock()

	for _, wrappedCtx := range batch {
		if wrappedCtx == nil || wrappedCtx.Ctx == nil {
			continue
		}

		qCtx := wrappedCtx.Ctx
		qQuestion := qCtx.QQuestion()
		duration := wrappedCtx.ProcessingDuration
		durationMs := float64(duration.Microseconds()) / 1000.0

		// Instant update of global atomic statistics
		c.totalQueryCountGlobal.Add(1)
		c.totalQueryDurationGlobal.Add(uint64(duration.Microseconds()))

		if !c.capturing || c.capacity == 0 {
			continue
		}

		// Optimized IP parsing: Strip port before interning to maximize LRU cache utility
		clientAddr := qCtx.ServerMeta.ClientAddr.String()
		if host, _, err := net.SplitHostPort(clientAddr); err == nil {
			clientAddr = host
		}

		// Optimized domain trim without allocating new string unless necessary
		qName := qQuestion.Name
		if len(qName) > 1 && qName[len(qName)-1] == '.' {
			qName = qName[:len(qName)-1]
		}

		log := AuditLog{
			ClientIP:   internString(clientAddr),
			QueryType:  internString(dns.TypeToString[qQuestion.Qtype]),
			QueryName:  internString(qName),
			QueryClass: internString(dns.ClassToString[qQuestion.Qclass]),
			QueryTime:  qCtx.StartTime(),
			DurationMs: durationMs,
			TraceID:    qCtx.TraceID, // OPTIMIZATION: Do not intern unique TraceIDs.
		}

		if val, ok := qCtx.GetValue(query_context.KeyDomainSet); ok {
			if name, isString := val.(string); isString {
				log.DomainSet = name
			}
		}
		if val, ok := qCtx.GetValue(query_context.KeyMatchedGroup); ok {
			if name, isString := val.(string); isString {
				log.MatchedGroup = name
			}
		}
		if val, ok := qCtx.GetValue(query_context.KeyFinalSequence); ok {
			if name, isString := val.(string); isString {
				log.FinalSequence = name
			}
		}
		if val, ok := qCtx.GetValue(query_context.KeyFinalUpstream); ok {
			if name, isString := val.(string); isString {
				log.FinalUpstream = name
			}
		}
		if val, ok := qCtx.GetValue(query_context.KeyFinalUpstreamTargets); ok {
			if name, isString := val.(string); isString {
				log.UpstreamTargets = name
			}
		}
		if val, ok := qCtx.GetValue(query_context.KeySelectedUpstream); ok {
			if name, isString := val.(string); isString {
				log.SelectedUpstream = name
			}
		}
		if val, ok := qCtx.GetValue(query_context.KeyMatchedRuleSource); ok {
			if name, isString := val.(string); isString {
				log.MatchedRuleSource = name
			}
		}

		if log.DomainSet == "" {
			log.DomainSet = "unmatched_rule"
		}
		log.EffectiveTag = internString(computeEffectiveTag(log.DomainSet, log.FinalUpstream, log.MatchedGroup, log.FinalSequence))

		if resp := qCtx.R(); resp != nil {
			log.ResponseCode = internString(dns.RcodeToString[resp.Rcode])
			log.ResponseFlags = ResponseFlags{
				AA: resp.Authoritative,
				TC: resp.Truncated,
				RA: resp.RecursionAvailable,
			}

			if len(resp.Answer) > 0 {
				log.Answers = make([]AnswerDetail, 0, len(resp.Answer))
				for _, ans := range resp.Answer {
					header := ans.Header()
					detail := AnswerDetail{
						Type: internString(dns.TypeToString[header.Rrtype]),
						TTL:  header.Ttl,
					}
					switch record := ans.(type) {
					case *dns.A:
						detail.Data = internString(record.A.String())
					case *dns.AAAA:
						detail.Data = internString(record.AAAA.String())
					case *dns.CNAME:
						detail.Data = internString(record.Target)
					case *dns.PTR:
						detail.Data = internString(record.Ptr)
					case *dns.NS:
						detail.Data = internString(record.Ns)
					case *dns.MX:
						detail.Data = internString(record.Mx)
					case *dns.TXT:
						detail.Data = internString(strings.Join(record.Txt, " "))
					default:
						detail.Data = internString(ans.String())
					}
					log.Answers = append(log.Answers, detail)
				}
			}
		} else {
			log.ResponseCode = "NO_RESPONSE"
		}

		// Circular array logic for fixed-memory logging
		if len(c.logs) < c.capacity {
			c.logs = append(c.logs, log)
		} else {
			c.logs[c.head] = log
			c.head = (c.head + 1) % c.capacity
		}

		// Update slowest queries heap (Priority Queue)
		if c.slowestQueries.Len() < slowestQueriesCapacity {
			heap.Push(&c.slowestQueries, log)
		} else if log.DurationMs > c.slowestQueries[0].DurationMs {
			c.slowestQueries[0] = log
			heap.Fix(&c.slowestQueries, 0)
		}
	}
}

func splitDomainTags(value string) []string {
	raw := strings.Split(value, "|")
	out := make([]string, 0, len(raw))
	seen := make(map[string]struct{}, len(raw))
	for _, item := range raw {
		tag := strings.TrimSpace(item)
		if tag == "" {
			continue
		}
		if _, ok := seen[tag]; ok {
			continue
		}
		seen[tag] = struct{}{}
		out = append(out, tag)
	}
	return out
}

func joinDomainTags(tags []string) string {
	if len(tags) == 0 {
		return ""
	}
	seen := make(map[string]struct{}, len(tags))
	out := make([]string, 0, len(tags))
	for _, tag := range tags {
		v := strings.TrimSpace(tag)
		if v == "" {
			continue
		}
		if _, ok := seen[v]; ok {
			continue
		}
		seen[v] = struct{}{}
		out = append(out, v)
	}
	return strings.Join(out, "|")
}

func hasDomainTag(tags []string, target string) bool {
	for _, tag := range tags {
		if tag == target {
			return true
		}
	}
	return false
}

func firstMatchTag(tags []string, candidates []string) string {
	for _, candidate := range candidates {
		if hasDomainTag(tags, candidate) {
			return candidate
		}
	}
	return ""
}

func firstMatchTagInOrder(tags []string, allowed map[string]struct{}) string {
	for _, tag := range tags {
		if _, ok := allowed[tag]; ok {
			return tag
		}
	}
	return ""
}

func normalizeSpecialTagFromGroup(matchedGroup string) string {
	if !strings.HasPrefix(matchedGroup, "special_") {
		return ""
	}
	slot := strings.TrimPrefix(matchedGroup, "special_")
	if slot == "" {
		return ""
	}
	for _, r := range slot {
		if r < '0' || r > '9' {
			return ""
		}
	}
	return "特殊上游" + slot
}

func normalizeSpecialTagFromUpstream(finalUpstream string) string {
	if !strings.HasPrefix(finalUpstream, "special_upstream_") {
		return ""
	}
	slot := strings.TrimPrefix(finalUpstream, "special_upstream_")
	if slot == "" {
		return ""
	}
	for _, r := range slot {
		if r < '0' || r > '9' {
			return ""
		}
	}
	return "特殊上游" + slot
}

func classifyRouteKind(finalUpstream string) string {
	switch strings.TrimSpace(finalUpstream) {
	case "domestic", "cnfake":
		return "direct"
	case "foreign", "foreignecs", "nocnfake":
		return "proxy"
	default:
		return ""
	}
}

func joinEffectiveTag(noVTags []string, core string) string {
	out := make([]string, 0, len(noVTags)+1)
	out = append(out, noVTags...)
	if core != "" {
		out = append(out, core)
	}
	return joinDomainTags(out)
}

func computeEffectiveTag(domainSet, finalUpstream, matchedGroup, finalSequence string) string {
	domainSet = strings.TrimSpace(domainSet)
	if domainSet == "" || domainSet == "unmatched_rule" {
		return "unmatched_rule"
	}

	if special := normalizeSpecialTagFromGroup(strings.TrimSpace(matchedGroup)); special != "" {
		return special
	}
	if special := normalizeSpecialTagFromUpstream(strings.TrimSpace(finalUpstream)); special != "" {
		return special
	}

	tags := splitDomainTags(domainSet)
	if len(tags) == 0 {
		return "unmatched_rule"
	}

	for _, tag := range tags {
		if strings.HasPrefix(tag, "特殊上游") {
			return tag
		}
	}

	singleDecisionPriority := []string{
		"重定向", "指定客户端直连",
		"黑名单", "广告屏蔽",
		"BANAAAA", "BANSOA", "BANPTR", "BANHTTPS",
		"DDNS域名",
		"stash国内", "stash国外",
		"clashmi国内", "clashmi国外",
		"sing-box国内", "sing-box国外",
	}
	if one := firstMatchTag(tags, singleDecisionPriority); one != "" {
		return one
	}

	noVTags := make([]string, 0, 2)
	if hasDomainTag(tags, "记忆无V4") {
		noVTags = append(noVTags, "记忆无V4")
	}
	if hasDomainTag(tags, "记忆无V6") {
		noVTags = append(noVTags, "记忆无V6")
	}

	hasMemoryDirect := hasDomainTag(tags, "记忆直连")
	hasMemoryProxy := hasDomainTag(tags, "记忆代理")
	routeKind := classifyRouteKind(strings.TrimSpace(finalUpstream))

	if hasMemoryDirect || hasMemoryProxy {
		memoryLabel := ""
		switch {
		case hasMemoryDirect && hasMemoryProxy:
			if routeKind == "proxy" {
				memoryLabel = "记忆代理"
			} else {
				memoryLabel = "记忆直连"
			}
		case hasMemoryDirect:
			if routeKind == "proxy" {
				memoryLabel = "记忆直连转代理"
			} else {
				memoryLabel = "记忆直连"
			}
		case hasMemoryProxy:
			if routeKind == "direct" {
				memoryLabel = "记忆代理转直连"
			} else {
				memoryLabel = "记忆代理"
			}
		}
		if joined := joinEffectiveTag(noVTags, memoryLabel); joined != "" {
			return joined
		}
	}

	directCandidates := []string{"白名单", "订阅直连补充", "订阅直连", "CN fakeip filter", "!CN fakeip filter"}
	proxyCandidates := []string{"灰名单", "订阅代理补充", "订阅代理"}
	directPromotionCandidates := []string{"白名单", "订阅直连补充", "订阅直连", "CN fakeip filter"}

	if routeKind == "proxy" && strings.TrimSpace(finalSequence) == "sequence_fakeip_addlist" {
		if core := firstMatchTag(tags, directPromotionCandidates); core != "" {
			if joined := joinEffectiveTag(noVTags, "直连候选转代理"); joined != "" {
				return joined
			}
		}
	}

	if routeKind == "direct" {
		if core := firstMatchTag(tags, directCandidates); core != "" {
			if joined := joinEffectiveTag(noVTags, core); joined != "" {
				return joined
			}
		}
	}
	if routeKind == "proxy" {
		if core := firstMatchTag(tags, proxyCandidates); core != "" {
			if joined := joinEffectiveTag(noVTags, core); joined != "" {
				return joined
			}
		}
	}

	allowed := map[string]struct{}{
		"白名单": {}, "订阅直连补充": {}, "订阅直连": {},
		"灰名单": {}, "订阅代理补充": {}, "订阅代理": {},
		"CN fakeip filter": {}, "!CN fakeip filter": {},
	}
	if core := firstMatchTagInOrder(tags, allowed); core != "" {
		if joined := joinEffectiveTag(noVTags, core); joined != "" {
			return joined
		}
	}

	return domainSet
}

// syncStatsLocked updates statistical maps when API requests are made.
func (c *AuditCollector) syncStatsLocked() {
	if c.capacity == 0 || len(c.logs) == 0 {
		return
	}

	now := time.Now()
	// Throttle synchronization to avoid UI-triggered CPU spikes
	if now.Sub(c.lastSyncTime) < time.Second {
		return
	}

	// Pre-size maps to reduce re-allocation overhead during heavy processing
	c.domainCounts = make(map[string]int, 1024)
	c.clientCounts = make(map[string]int, 64)
	c.domainSetCounts = make(map[string]int, 16)
	c.effectiveTagCounts = make(map[string]int, 16)
	c.totalQueryCount = uint64(len(c.logs))
	c.totalQueryDuration = 0.0

	for _, l := range c.logs {
		c.domainCounts[l.QueryName]++
		c.clientCounts[l.ClientIP]++
		c.domainSetCounts[l.DomainSet]++
		effectiveTag := l.EffectiveTag
		if effectiveTag == "" {
			effectiveTag = l.DomainSet
		}
		c.effectiveTagCounts[effectiveTag]++
		c.totalQueryDuration += l.DurationMs
	}

	c.lastSyncTime = now
}

func (c *AuditCollector) Collect(qCtx *query_context.Context) {
	if !c.IsCapturing() {
		return
	}

	duration := time.Since(qCtx.StartTime())

	// Retrieve object from pool to reduce heap pressure
	wrappedCtx := auditCtxPool.Get().(*auditContext)
	wrappedCtx.Ctx = qCtx
	wrappedCtx.ProcessingDuration = duration

	select {
	case c.ctxChan <- wrappedCtx:
	default:
		// Non-blocking drop during system overload
		auditCtxPool.Put(wrappedCtx)
	}
}

func (c *AuditCollector) Start() { c.mu.Lock(); c.capturing = true; c.mu.Unlock() }
func (c *AuditCollector) Stop()  { c.mu.Lock(); c.capturing = false; c.mu.Unlock() }
func (c *AuditCollector) IsCapturing() bool {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.capturing
}

func (c *AuditCollector) GetLogs() []AuditLog {
	c.mu.RLock()
	defer c.mu.RUnlock()

	if c.capacity == 0 || len(c.logs) == 0 {
		return []AuditLog{}
	}

	if len(c.logs) < c.capacity {
		logsCopy := make([]AuditLog, len(c.logs))
		copy(logsCopy, c.logs)
		return logsCopy
	}

	logsCopy := make([]AuditLog, c.capacity)
	copy(logsCopy, c.logs[c.head:])
	copy(logsCopy[c.capacity-c.head:], c.logs[:c.head])
	return logsCopy
}

func (c *AuditCollector) ClearLogs() {
	c.mu.Lock()
	defer c.mu.Unlock()

	if c.logs != nil {
		c.logs = c.logs[:0]
	}

	c.head = 0
	c.slowestQueries = make(slowestQueryHeap, 0, slowestQueriesCapacity)
	heap.Init(&c.slowestQueries)
	c.domainCounts = make(map[string]int)
	c.clientCounts = make(map[string]int)
	c.domainSetCounts = make(map[string]int)
	c.effectiveTagCounts = make(map[string]int)
	c.totalQueryCount = 0
	c.totalQueryDuration = 0.0
}

func (c *AuditCollector) GetCapacity() int {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.capacity
}

func (c *AuditCollector) SetCapacity(newCapacity int, configBaseDir string) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if newCapacity < 0 {
		newCapacity = 0
	}
	if newCapacity > maxAuditCapacity {
		newCapacity = maxAuditCapacity
	}

	c.saveSettings(newCapacity, configBaseDir)

	c.capacity = newCapacity
	c.logs = make([]AuditLog, 0, newCapacity)
	c.head = 0
	c.slowestQueries = make(slowestQueryHeap, 0, slowestQueriesCapacity)
	heap.Init(&c.slowestQueries)
	c.domainCounts = make(map[string]int)
	c.clientCounts = make(map[string]int)
	c.domainSetCounts = make(map[string]int)
	c.effectiveTagCounts = make(map[string]int)
	c.totalQueryCount = 0
	c.totalQueryDuration = 0.0
}

func (c *AuditCollector) saveSettings(capacityToSave int, configBaseDir string) {
	settings := AuditSettings{Capacity: capacityToSave}
	data, err := json.MarshalIndent(settings, "", "  ")
	if err != nil {
		mlog.L().Error("failed to marshal audit settings", zap.Error(err))
		return
	}
	settingsPath := filepath.Join(configBaseDir, auditSettingsFilename)
	if err := os.WriteFile(settingsPath, data, 0644); err != nil {
		mlog.L().Error("failed to write audit settings file", zap.String("path", settingsPath), zap.Error(err))
	} else {
		mlog.L().Info("successfully saved audit settings", zap.String("path", settingsPath), zap.Int("capacity", capacityToSave))
	}
}

type V2GetLogsParams struct {
	Page        int
	Limit       int
	Domain      string
	AnswerIP    string
	AnswerCNAME string
	ClientIP    string
	Q           string
	Exact       bool
}

func (c *AuditCollector) getLogsSnapshot() []AuditLog {
	c.mu.RLock()
	defer c.mu.RUnlock()

	if c.capacity == 0 || len(c.logs) == 0 {
		return []AuditLog{}
	}

	snapshot := make([]AuditLog, len(c.logs))
	if len(c.logs) < c.capacity {
		copy(snapshot, c.logs)
	} else {
		copy(snapshot, c.logs[c.head:])
		copy(snapshot[c.capacity-c.head:], c.logs[:c.head])
	}

	for i, j := 0, len(snapshot)-1; i < j; i, j = i+1, j-1 {
		snapshot[i], snapshot[j] = snapshot[j], snapshot[i]
	}
	return snapshot
}

func (c *AuditCollector) CalculateV2Stats() V2StatsResponse {
	c.mu.Lock()
	c.syncStatsLocked()
	c.mu.Unlock()

	c.mu.RLock()
	defer c.mu.RUnlock()

	avgDuration := 0.0
	if c.totalQueryCount > 0 {
		avgDuration = c.totalQueryDuration / float64(c.totalQueryCount)
	}

	return V2StatsResponse{
		TotalQueries:      c.totalQueryCount,
		AverageDurationMs: avgDuration,
	}
}

type rankHeap []V2RankItem

func (h rankHeap) Len() int           { return len(h) }
func (h rankHeap) Less(i, j int) bool { return h[i].Count < h[j].Count }
func (h rankHeap) Swap(i, j int)      { h[i], h[j] = h[j], h[i] }
func (h *rankHeap) Push(x any)        { *h = append(*h, x.(V2RankItem)) }
func (h *rankHeap) Pop() any {
	old := *h
	n := len(old)
	x := old[n-1]
	*h = old[0 : n-1]
	return x
}

func (c *AuditCollector) getRankFromMap(sourceMap map[string]int, limit int) []V2RankItem {
	if len(sourceMap) == 0 {
		return []V2RankItem{}
	}

	if len(sourceMap) <= limit {
		res := make([]V2RankItem, 0, len(sourceMap))
		for k, v := range sourceMap {
			res = append(res, V2RankItem{Key: k, Count: v})
		}
		sort.Slice(res, func(i, j int) bool {
			return res[i].Count > res[j].Count
		})
		return res
	}

	h := &rankHeap{}
	heap.Init(h)

	for key, count := range sourceMap {
		if h.Len() < limit {
			heap.Push(h, V2RankItem{Key: key, Count: count})
		} else if count > (*h)[0].Count {
			heap.Pop(h)
			heap.Push(h, V2RankItem{Key: key, Count: count})
		}
	}

	result := make([]V2RankItem, h.Len())
	for i := h.Len() - 1; i >= 0; i-- {
		result[i] = heap.Pop(h).(V2RankItem)
	}

	return result
}

type RankType string

const (
	RankByDomain    RankType = "domain"
	RankByClient    RankType = "client"
	RankByDomainSet RankType = "domain_set"
	RankByEffective RankType = "effective_tag"
)

func (c *AuditCollector) CalculateRank(rankType RankType, limit int) []V2RankItem {
	c.mu.Lock()
	c.syncStatsLocked()
	c.mu.Unlock()

	c.mu.RLock()
	defer c.mu.RUnlock()

	switch rankType {
	case RankByDomain:
		return c.getRankFromMap(c.domainCounts, limit)
	case RankByClient:
		return c.getRankFromMap(c.clientCounts, limit)
	case RankByDomainSet:
		return c.getRankFromMap(c.domainSetCounts, limit)
	case RankByEffective:
		return c.getRankFromMap(c.effectiveTagCounts, limit)
	default:
		return []V2RankItem{}
	}
}

func (c *AuditCollector) GetSlowestQueries(limit int) []AuditLog {
	c.mu.RLock()
	defer c.mu.RUnlock()

	if c.slowestQueries.Len() == 0 {
		return []AuditLog{}
	}

	snapshot := make([]AuditLog, c.slowestQueries.Len())
	copy(snapshot, c.slowestQueries)

	sort.Slice(snapshot, func(i, j int) bool {
		return snapshot[i].DurationMs > snapshot[j].DurationMs
	})

	if len(snapshot) > limit {
		return snapshot[:limit]
	}
	return snapshot
}

func (c *AuditCollector) GetV2Logs(params V2GetLogsParams) V2PaginatedLogsResponse {
	c.mu.RLock()
	defer c.mu.RUnlock()

	totalLogs := len(c.logs)
	if totalLogs == 0 || c.capacity == 0 {
		return V2PaginatedLogsResponse{
			Pagination: V2PaginationInfo{CurrentPage: params.Page, ItemsPerPage: params.Limit},
			Logs:       []AuditLog{},
		}
	}

	if params.Page < 1 {
		params.Page = 1
	}
	if params.Limit <= 0 {
		params.Limit = 50
	}

	searchTerm := params.Q
	if params.Q != "" && !params.Exact {
		searchTerm = strings.ToLower(searchTerm)
	}

	matchCount := 0
	offset := (params.Page - 1) * params.Limit
	filteredLogs := make([]AuditLog, 0, params.Limit)

	curr := (c.head - 1 + totalLogs) % totalLogs

	for i := 0; i < totalLogs; i++ {
		log := c.logs[curr]
		isMatched := true

		if params.Q != "" {
			foundInQ := false
			matchFunc := strings.Contains
			if params.Exact {
				matchFunc = func(s, substr string) bool { return s == substr }
			}

			// 1. Check QueryName
			haystack := log.QueryName
			if !params.Exact {
				haystack = strings.ToLower(haystack)
			}
			if matchFunc(haystack, searchTerm) {
				foundInQ = true
			}

			// 2. Check ClientIP
			if !foundInQ {
				haystack = log.ClientIP
				if !params.Exact {
					haystack = strings.ToLower(haystack)
				}
				if matchFunc(haystack, searchTerm) {
					foundInQ = true
				}
			}

			// 3. Check TraceID
			if !foundInQ {
				haystack = log.TraceID
				if !params.Exact {
					haystack = strings.ToLower(haystack)
				}
				if matchFunc(haystack, searchTerm) {
					foundInQ = true
				}
			}

			// 4. Check DomainSet
			if !foundInQ && log.DomainSet != "" {
				haystack = log.DomainSet
				if !params.Exact {
					haystack = strings.ToLower(haystack)
				}
				if matchFunc(haystack, searchTerm) {
					foundInQ = true
				}
			}

			// 5. Check EffectiveTag
			if !foundInQ && log.EffectiveTag != "" {
				haystack = log.EffectiveTag
				if !params.Exact {
					haystack = strings.ToLower(haystack)
				}
				if matchFunc(haystack, searchTerm) {
					foundInQ = true
				}
			}

			// 6. Check MatchedRuleSource
			if !foundInQ && log.MatchedRuleSource != "" {
				haystack = log.MatchedRuleSource
				if !params.Exact {
					haystack = strings.ToLower(haystack)
				}
				if matchFunc(haystack, searchTerm) {
					foundInQ = true
				}
			}

			// 7. Check SelectedUpstream
			if !foundInQ && log.SelectedUpstream != "" {
				haystack = log.SelectedUpstream
				if !params.Exact {
					haystack = strings.ToLower(haystack)
				}
				if matchFunc(haystack, searchTerm) {
					foundInQ = true
				}
			}

			// 8. Check Answers
			if !foundInQ {
				for _, answer := range log.Answers {
					haystack = answer.Data
					if !params.Exact {
						haystack = strings.ToLower(haystack)
					}
					if matchFunc(haystack, searchTerm) {
						foundInQ = true
						break
					}
				}
			}
			if !foundInQ {
				isMatched = false
			}
		}

		if isMatched && params.ClientIP != "" && log.ClientIP != params.ClientIP {
			isMatched = false
		}
		if isMatched && params.Domain != "" && !strings.Contains(log.QueryName, params.Domain) {
			isMatched = false
		}
		if isMatched && params.AnswerIP != "" {
			found := false
			for _, answer := range log.Answers {
				if (answer.Type == "A" || answer.Type == "AAAA") && answer.Data == params.AnswerIP {
					found = true
					break
				}
			}
			if !found {
				isMatched = false
			}
		}
		if isMatched && params.AnswerCNAME != "" {
			found := false
			for _, answer := range log.Answers {
				if answer.Type == "CNAME" && strings.Contains(answer.Data, params.AnswerCNAME) {
					found = true
					break
				}
			}
			if !found {
				isMatched = false
			}
		}

		if isMatched {
			if matchCount >= offset && len(filteredLogs) < params.Limit {
				filteredLogs = append(filteredLogs, log)
			}
			matchCount++
		}

		curr = (curr - 1 + totalLogs) % totalLogs
	}

	totalPages := int(math.Ceil(float64(matchCount) / float64(params.Limit)))
	return V2PaginatedLogsResponse{
		Pagination: V2PaginationInfo{
			TotalItems:   matchCount,
			TotalPages:   totalPages,
			CurrentPage:  params.Page,
			ItemsPerPage: params.Limit,
		},
		Logs: filteredLogs,
	}
}
