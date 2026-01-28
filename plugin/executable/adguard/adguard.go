package adguard_rule

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/go-chi/chi/v5"
	"github.com/google/uuid"
	"golang.org/x/net/proxy"
)

const (
	PluginType        = "adguard_rule"
	configFile        = "config.json"
	downloadTimeout   = 30 * time.Second
	reloadDebounceDur = 500 * time.Millisecond // 防抖延迟
)

// 注册插件
func init() {
	coremain.RegNewPluginFunc(PluginType, newAdguardRule, func() any { return new(Args) })
}

// Args 是插件的配置参数
type Args struct {
	Dir    string `yaml:"dir"`
	Socks5 string `yaml:"socks5,omitempty"` // 可选: SOCKS5 代理地址 (e.g., "127.0.0.1:1080")
}

// OnlineRule 定义了一个在线规则源的结构
type OnlineRule struct {
	ID                  string    `json:"id"`
	Name                string    `json:"name"`
	URL                 string    `json:"url"`
	Enabled             bool      `json:"enabled"`
	AutoUpdate          bool      `json:"auto_update"`
	UpdateIntervalHours int       `json:"update_interval_hours"` // in hours
	RuleCount           int       `json:"rule_count"`
	LastUpdated         time.Time `json:"last_updated"`

	localPath string `json:"-"`
}

// onlineRuleAlias 是为了在 MarshalJSON 中避免无限递归而定义的别名
type onlineRuleAlias OnlineRule

// MarshalJSON 自定义 OnlineRule 的 JSON 序列化行为
func (rule *OnlineRule) MarshalJSON() ([]byte, error) {
	if rule.LastUpdated.IsZero() {
		return json.Marshal(&struct {
			*onlineRuleAlias
			LastUpdated *time.Time `json:"last_updated,omitempty"`
		}{
			onlineRuleAlias: (*onlineRuleAlias)(rule),
			LastUpdated:     nil,
		})
	}
	return json.Marshal((*onlineRuleAlias)(rule))
}

// AdguardRule 是插件的主结构体
type AdguardRule struct {
	mu           sync.RWMutex
	reloadMu     sync.Mutex
	dir          string
	configFile   string
	onlineRules  map[string]*OnlineRule
	allowMatcher *domain.MixMatcher[struct{}]
	denyMatcher  *domain.MixMatcher[struct{}]
	httpClient   *http.Client
	reloadID     atomic.Uint64

	// 用于优雅关闭
	ctx    context.Context
	cancel context.CancelFunc

	// 新增：订阅者支持
	subscribers []func()
	subsMu      sync.RWMutex
}

// 确保实现了必要的接口
var _ data_provider.RuleExporter = (*AdguardRule)(nil)        // 新增接口
var _ data_provider.DomainMatcherProvider = (*AdguardRule)(nil) // 原有接口

// RuleReceiver 接口用于解耦解析和存储逻辑，使 parseRules 既能用于构建 Matcher 也能用于导出
type RuleReceiver interface {
	Add(string, struct{}) error
}

// ruleCollector 用于收集规则列表 (供 GetRules 使用)
type ruleCollector struct {
	rules []string
}

func (c *ruleCollector) Add(s string, _ struct{}) error {
	c.rules = append(c.rules, s)
	return nil
}

// noOpCollector 用于丢弃不需要的规则 (供 GetRules 忽略白名单使用)
type noOpCollector struct{}

func (c *noOpCollector) Add(_ string, _ struct{}) error { return nil }

// Subscribe 实现 RuleExporter，允许 external plugin 监听变更
func (p *AdguardRule) Subscribe(cb func()) {
	p.subsMu.Lock()
	defer p.subsMu.Unlock()
	p.subscribers = append(p.subscribers, cb)
}

// GetRules 实现 RuleExporter
// 注意：只导出 Deny (黑名单) 规则，忽略 Allow (白名单) 规则。
func (p *AdguardRule) GetRules() ([]string, error) {
	p.mu.RLock()
	// 获取所有“已启用”的规则
	enabledRules := make([]*OnlineRule, 0)
	for _, rule := range p.onlineRules {
		if rule.Enabled {
			enabledRules = append(enabledRules, rule)
		}
	}
	p.mu.RUnlock()

	// 收集黑名单规则
	denyCollector := &ruleCollector{rules: make([]string, 0)}
	// 忽略白名单规则 (我们不希望把白名单导出给 domain_mapper 进行打标拦截)
	allowCollector := &noOpCollector{}

	for _, rule := range enabledRules {
		file, err := os.Open(rule.localPath)
		if err != nil {
			// 仅记录日志，不中断其他文件的导出
			log.Printf("[adguard_rule] GetRules: WARN: skipping enabled rule '%s', cannot open file: %v", rule.Name, err)
			continue
		}
		// 复用 parseRules 逻辑
		parseRules(file, allowCollector, denyCollector)
		file.Close()
	}

	return denyCollector.rules, nil
}

// notifySubscribers 通知所有订阅者（domain_mapper）进行更新
func (p *AdguardRule) notifySubscribers() {
	p.subsMu.RLock()
	subs := make([]func(), len(p.subscribers))
	copy(subs, p.subscribers)
	p.subsMu.RUnlock()

	for _, cb := range subs {
		go cb()
	}

    // 新增：给订阅者一点时间完成它们的重载工作，然后统一回收
    if len(subs) > 0 {
        time.AfterFunc(time.Second*5, func() {
            log.Println("[adguard_rule] post-notification GC triggered")
            coremain.ManualGC()
        })
    }
}

// newAdguardRule 是插件的初始化函数
func newAdguardRule(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)
	if cfg.Dir == "" {
		return nil, errors.New("adguard_rule: 'dir' must be specified")
	}

	if err := os.MkdirAll(cfg.Dir, 0755); err != nil {
		return nil, fmt.Errorf("adguard_rule: failed to create directory %s: %w", cfg.Dir, err)
	}
	log.Printf("[adguard_rule] working directory is: %s", cfg.Dir)

	// 创建带 SOCKS5 支持的 HTTP Client
	transport := &http.Transport{
		Proxy: http.ProxyFromEnvironment,
		DialContext: (&net.Dialer{
			Timeout:   30 * time.Second,
			KeepAlive: 30 * time.Second,
		}).DialContext,
		TLSHandshakeTimeout: 10 * time.Second,
	}
	if cfg.Socks5 != "" {
		log.Printf("[adguard_rule] using SOCKS5 proxy: %s", cfg.Socks5)
		dialer, err := proxy.SOCKS5("tcp", cfg.Socks5, nil, proxy.Direct)
		if err != nil {
			return nil, fmt.Errorf("adguard_rule: failed to create SOCKS5 dialer: %w", err)
		}
		contextDialer, ok := dialer.(proxy.ContextDialer)
		if !ok {
			return nil, errors.New("adguard_rule: created dialer does not support context")
		}
		transport.DialContext = contextDialer.DialContext
		transport.Proxy = nil
	}
	httpClient := &http.Client{
		Timeout:   downloadTimeout,
		Transport: transport,
	}

	// 创建可取消的上下文，用于优雅关闭
	ctx, cancel := context.WithCancel(context.Background())

	p := &AdguardRule{
		dir:          cfg.Dir,
		configFile:   filepath.Join(cfg.Dir, configFile),
		onlineRules:  make(map[string]*OnlineRule),
		allowMatcher: domain.NewDomainMixMatcher(),
		denyMatcher:  domain.NewDomainMixMatcher(),
		httpClient:   httpClient,
		ctx:          ctx,
		cancel:       cancel,
		subscribers:  make([]func(), 0),
	}

	if err := p.loadConfig(); err != nil {
		log.Printf("[adguard_rule] failed to load config file: %v. Starting with empty config.", err)
	}

	p.reloadAllRules(context.Background(), true)

	bp.RegAPI(p.api())

	go p.backgroundUpdater()

	return p, nil
}

// Close 实现了 io.Closer 接口，用于 mosdns 关闭时回收资源
func (p *AdguardRule) Close() error {
	log.Println("[adguard_rule] closing...")
	p.cancel() // 发出取消信号，终止后台 goroutine
	return nil
}

// triggerReload 使用防抖机制来调用 reloadAllRules
func (p *AdguardRule) triggerReload(ctx context.Context) {
	currentReloadID := p.reloadID.Add(1)
	time.AfterFunc(reloadDebounceDur, func() {
		// 检查插件是否已经关闭
		if p.ctx.Err() != nil {
			log.Println("[adguard_rule] reload skipped because plugin is closing.")
			return
		}
		if p.reloadID.Load() == currentReloadID {
			log.Println("[adguard_rule] Debounced reload triggered.")
			p.reloadAllRules(ctx, false)
		} else {
			log.Println("[adguard_rule] Debounced reload skipped (superseded by a newer request).")
		}
	})
}

// GetDomainMatcher 实现了 data_provider.DomainMatcherProvider 接口
func (p *AdguardRule) GetDomainMatcher() domain.Matcher[struct{}] {
	return p
}

// Match 实现了 domain.Matcher 接口
func (p *AdguardRule) Match(domainStr string) (value struct{}, ok bool) {
	p.mu.RLock()
	defer p.mu.RUnlock()

	if _, matched := p.allowMatcher.Match(domainStr); matched {
		return struct{}{}, false
	}

	if _, matched := p.denyMatcher.Match(domainStr); matched {
		return struct{}{}, true
	}

	return struct{}{}, false
}

// loadConfig 从 config.json 加载规则列表配置
func (p *AdguardRule) loadConfig() error {
	p.mu.Lock()
	defer p.mu.Unlock()

	data, err := os.ReadFile(p.configFile)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}

	var rules []*OnlineRule
	if err := json.Unmarshal(data, &rules); err != nil {
		return fmt.Errorf("failed to parse config json: %w", err)
	}

	p.onlineRules = make(map[string]*OnlineRule, len(rules))
	for _, rule := range rules {
		rule.localPath = filepath.Join(p.dir, rule.ID+".rules")
		p.onlineRules[rule.ID] = rule
	}
	log.Printf("[adguard_rule] loaded %d rule configurations from %s", len(p.onlineRules), p.configFile)
	return nil
}

// saveConfig 将当前规则列表配置保存到 config.json (原子写入)
func (p *AdguardRule) saveConfig() error {
	p.mu.RLock()
	defer p.mu.RUnlock()

	rules := make([]*OnlineRule, 0, len(p.onlineRules))
	for _, rule := range p.onlineRules {
		rules = append(rules, rule)
	}

	sort.Slice(rules, func(i, j int) bool {
		return rules[i].ID < rules[j].ID
	})

	data, err := json.MarshalIndent(rules, "", "  ")
	if err != nil {
		return fmt.Errorf("failed to marshal config to json: %w", err)
	}

	// 原子写入：先写入临时文件，再重命名
	tmpFile := p.configFile + ".tmp"
	if err := os.WriteFile(tmpFile, data, 0644); err != nil {
		return fmt.Errorf("failed to write to temporary config file: %w", err)
	}
	if err := os.Rename(tmpFile, p.configFile); err != nil {
		return fmt.Errorf("failed to rename temporary config to final: %w", err)
	}

	return nil
}

// reloadAllRules 重新加载所有启用的规则到内存中的匹配器
func (p *AdguardRule) reloadAllRules(ctx context.Context, initialLoad bool) {
	p.reloadMu.Lock()
	defer p.reloadMu.Unlock()

	log.Println("[adguard_rule] starting to reload all rules...")

	p.mu.RLock()
	allRulesSnapshot := make([]*OnlineRule, 0, len(p.onlineRules))
	enabledRules := make([]*OnlineRule, 0)
	for _, rule := range p.onlineRules {
		allRulesSnapshot = append(allRulesSnapshot, rule)
		if rule.Enabled {
			enabledRules = append(enabledRules, rule)
		}
	}
	p.mu.RUnlock()

	if initialLoad {
		var wg sync.WaitGroup
		for _, rule := range enabledRules {
			if _, err := os.Stat(rule.localPath); os.IsNotExist(err) {
				wg.Add(1)
				go func(ruleID string) {
					defer wg.Done()
					downloadCtx, cancel := context.WithTimeout(ctx, downloadTimeout)
					defer cancel()
					if err := p.downloadRule(downloadCtx, ruleID); err != nil {
						log.Printf("[adguard_rule] ERROR: failed to download rule on initial load: %v", err)
					}
				}(rule.ID)
			}
		}
		wg.Wait()
	}

	p.updateAllRuleCounts()

	// 构建新的 Matcher (用于本插件的原有功能)
	newAllowMatcher := domain.NewDomainMixMatcher()
	newDenyMatcher := domain.NewDomainMixMatcher()
	totalRuleCount := 0

	for _, rule := range enabledRules {
		file, err := os.Open(rule.localPath)
		if err != nil {
			log.Printf("[adguard_rule] WARN: skipping enabled rule '%s', cannot open local file %s: %v", rule.Name, rule.localPath, err)
			continue
		}

		// 这里传入 Matcher，保持原功能不变
		count, err := parseRules(file, newAllowMatcher, newDenyMatcher)
		file.Close() // 确保文件句柄被关闭

		if err != nil {
			log.Printf("[adguard_rule] ERROR: failed to parse rule file for '%s' (%s): %v", rule.Name, rule.localPath, err)
		}
		totalRuleCount += count
	}

	p.mu.Lock()
	p.allowMatcher = newAllowMatcher
	p.denyMatcher = newDenyMatcher
	p.mu.Unlock()

	log.Printf("[adguard_rule] finished reloading. Total active rules from enabled lists: %d", totalRuleCount)
	
        newAllowMatcher = nil
        newDenyMatcher = nil

	// [关键]: 更新完成后，通知所有订阅者 (如 domain_mapper)
	// 因为 Add/Delete/Enable/Disable/Update 最终都会走到这里，所以都能触发通知
	p.notifySubscribers()
        coremain.ManualGC()
}

type counterCollector struct {
	count int
}
func (c *counterCollector) Add(_ string, _ struct{}) error {
	c.count++
	return nil
}

// updateAllRuleCounts 遍历所有已知规则，并更新它们的 RuleCount 字段
func (p *AdguardRule) updateAllRuleCounts() {
	p.mu.Lock()
	defer p.mu.Unlock()

	var changed bool
	for _, rule := range p.onlineRules {
		file, err := os.Open(rule.localPath)
		if err != nil {
			if rule.RuleCount != 0 {
				rule.RuleCount = 0
				changed = true
			}
			continue
		}
		
		// --- 关键修改开始 ---
		// 使用计数器代替真实的 Matcher。
		// 这样 parseRules 在运行过程中不会构建复杂的 Trie 树和正则对象
		counter := &counterCollector{}
		count, _ := parseRules(file, counter, counter) 
		file.Close()
		// --- 关键修改结束 ---

		if rule.RuleCount != count {
			rule.RuleCount = count
			changed = true
		}
	}

	if changed {
		go func() {
			if err := p.saveConfig(); err != nil {
				log.Printf("[adguard_rule] ERROR: failed to save config after updating rule counts: %v", err)
			}
		}()
	}
}

// downloadRule 通过 ruleID 安全地下载指定的在线规则并保存到本地
func (p *AdguardRule) downloadRule(ctx context.Context, ruleID string) error {
	p.mu.RLock()
	rule, ok := p.onlineRules[ruleID]
	if !ok {
		p.mu.RUnlock()
		return fmt.Errorf("rule with ID %s not found during download", ruleID)
	}
	ruleName := rule.Name
	ruleURL := rule.URL
	localPath := rule.localPath
	p.mu.RUnlock()

	log.Printf("[adguard_rule] downloading rule '%s' from %s", ruleName, ruleURL)

	req, err := http.NewRequestWithContext(ctx, "GET", ruleURL, nil)
	if err != nil {
		return err
	}

	resp, err := p.httpClient.Do(req)
	if err != nil {
		return fmt.Errorf("http request failed for rule '%s': %w", ruleName, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("bad status code for rule '%s': %d", ruleName, resp.StatusCode)
	}

	// 原子写入
	tmpFile, err := os.CreateTemp(p.dir, "download-*.tmp")
	if err != nil {
		return err
	}
	defer os.Remove(tmpFile.Name())

	_, err = io.Copy(tmpFile, resp.Body)
	tmpFile.Close() // 确保在重命名前关闭
	if err != nil {
		return fmt.Errorf("failed to write to temp file for rule '%s': %w", ruleName, err)
	}

	if err := os.Rename(tmpFile.Name(), localPath); err != nil {
		return fmt.Errorf("failed to move temp file for rule '%s': %w", ruleName, err)
	}

	p.mu.Lock()
	if rule, ok := p.onlineRules[ruleID]; ok {
		rule.LastUpdated = time.Now()
	}
	p.mu.Unlock()

	log.Printf("[adguard_rule] successfully downloaded and saved rule '%s'", ruleName)
	return p.saveConfig()
}

// --- Adguard 规则解析逻辑 ---

var (
	blockRuleRegex = regexp.MustCompile(`^\|\|([\w\.\-\*]+)\^$`)
	allowRuleRegex = regexp.MustCompile(`^@@\|\|([\w\.\-\*]+)\^$`)
	regexRuleRegex = regexp.MustCompile(`^\/(.*)\/$`)
	fullMatchRegex = regexp.MustCompile(`^([\w\.\-]+)$`)
)

// parseRules 解析规则文件内容并填充到匹配器中
// 修改点：参数 allowM 和 denyM 类型改为 RuleReceiver 接口，使其既支持 MixMatcher 也支持 ruleCollector
func parseRules(reader io.Reader, allowM, denyM RuleReceiver) (int, error) {
	scanner := bufio.NewScanner(reader)
	count := 0
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "!") || strings.HasPrefix(line, "#") {
			continue
		}
		// 忽略 IP 规则
		if strings.ContainsAny(line, "0123456789") && (strings.Contains(line, "127.0.0.1") || strings.Contains(line, "0.0.0.0") || strings.Contains(line, "::")) {
			parts := strings.Fields(line)
			if len(parts) > 1 {
				continue
			}
		}
		// 忽略元素隐藏规则
		if strings.Contains(line, "#?#") || strings.Contains(line, "##") || strings.Contains(line, "$$") {
			continue
		}
		var mosdnsRule string
		parsed := false

		// 解析逻辑
		if matches := allowRuleRegex.FindStringSubmatch(line); len(matches) > 1 {
			domainStr := cleanDomain(matches[1])
			mosdnsRule = convertToMosdnsRule(domainStr)
			if strings.HasPrefix(mosdnsRule, "regexp:") {
				if _, err := regexp.Compile(strings.TrimPrefix(mosdnsRule, "regexp:")); err != nil {
					log.Printf("[adguard_rule] WARN: skipping invalid wildcard rule (compiles to bad regex) '%s'", line)
					continue
				}
			}
			if err := allowM.Add(mosdnsRule, struct{}{}); err == nil {
				parsed = true
			}
		} else if matches := blockRuleRegex.FindStringSubmatch(line); len(matches) > 1 {
			domainStr := cleanDomain(matches[1])
			mosdnsRule = convertToMosdnsRule(domainStr)
			if strings.HasPrefix(mosdnsRule, "regexp:") {
				if _, err := regexp.Compile(strings.TrimPrefix(mosdnsRule, "regexp:")); err != nil {
					log.Printf("[adguard_rule] WARN: skipping invalid wildcard rule (compiles to bad regex) '%s'", line)
					continue
				}
			}
			if err := denyM.Add(mosdnsRule, struct{}{}); err == nil {
				parsed = true
			}
		} else if matches := regexRuleRegex.FindStringSubmatch(line); len(matches) > 1 {
			regexPattern := matches[1]
			if _, err := regexp.Compile(regexPattern); err != nil {
				log.Printf("[adguard_rule] WARN: skipping invalid regex rule '%s': %v", line, err)
				continue
			}
			mosdnsRule = "regexp:" + regexPattern
			if err := denyM.Add(mosdnsRule, struct{}{}); err == nil {
				parsed = true
			}
		} else if matches := fullMatchRegex.FindStringSubmatch(line); len(matches) > 0 {
			domainStr := matches[1]
			if strings.Contains(domainStr, ".") && !strings.HasPrefix(domainStr, "*") && !strings.HasSuffix(domainStr, "*") {
				mosdnsRule = "full:" + domainStr
				if err := denyM.Add(mosdnsRule, struct{}{}); err == nil {
					parsed = true
				}
			}
		}
		if parsed {
			count++
		}
	}
	return count, scanner.Err()
}

// convertToMosdnsRule 是一个辅助函数
func convertToMosdnsRule(domainStr string) string {
	if strings.Contains(domainStr, "*") {
		regexStr := strings.ReplaceAll(domainStr, ".", `\.`)
		regexStr = strings.ReplaceAll(regexStr, "*", ".*")
		return "regexp:" + regexStr
	}
	return "domain:" + domainStr
}

// cleanDomain 移除Adguard规则中可能存在的前导/尾随通配符和点
func cleanDomain(domain string) string {
	domain = strings.TrimPrefix(domain, "*.")
	domain = strings.TrimPrefix(domain, ".")
	return domain
}

// --- 后台自动更新功能 ---
func (p *AdguardRule) backgroundUpdater() {
	// 初始延迟，避免启动时立即执行
	select {
	case <-time.After(1 * time.Minute):
	case <-p.ctx.Done():
		return
	}

	ticker := time.NewTicker(1 * time.Minute)
	defer ticker.Stop()

	for {
		select {
		case <-ticker.C:
			// 执行更新检查
			rulesToUpdate := p.getRulesForAutoUpdate()
			if len(rulesToUpdate) == 0 {
				continue
			}

			log.Printf("[adguard_rule] auto-update: found %d rule(s) that need updating.", len(rulesToUpdate))

			var wg sync.WaitGroup
			for _, rule := range rulesToUpdate {
				wg.Add(1)
				go func(ruleID string) {
					defer wg.Done()
					downloadCtx, cancel := context.WithTimeout(p.ctx, downloadTimeout)
					defer cancel()
					if err := p.downloadRule(downloadCtx, ruleID); err != nil {
						log.Printf("[adguard_rule] ERROR: failed to auto-update rule: %v", err)
					}
				}(rule.ID)
			}
			wg.Wait()

			log.Println("[adguard_rule] auto-update: downloads finished, triggering reload.")
			p.triggerReload(p.ctx)
			coremain.ManualGC()

		case <-p.ctx.Done():
			// 接收到关闭信号，退出循环
			log.Println("[adguard_rule] background updater is shutting down.")
			return
		}
	}
}

func (p *AdguardRule) getRulesForAutoUpdate() []*OnlineRule {
	p.mu.RLock()
	defer p.mu.RUnlock()

	var rulesToUpdate []*OnlineRule
	for _, rule := range p.onlineRules {
		if !rule.Enabled || !rule.AutoUpdate || rule.UpdateIntervalHours <= 0 {
			continue
		}
		if time.Since(rule.LastUpdated).Hours() >= float64(rule.UpdateIntervalHours) {
			rulesToUpdate = append(rulesToUpdate, rule)
		}
	}
	return rulesToUpdate
}

// --- API 处理器 ---

// jsonError 是一个辅助函数
func jsonError(w http.ResponseWriter, message string, code int) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(code)
	json.NewEncoder(w).Encode(map[string]string{"error": message})
}

func (p *AdguardRule) api() *chi.Mux {
	r := chi.NewRouter()

	r.Get("/rules", func(w http.ResponseWriter, r *http.Request) {
		p.mu.RLock()
		defer p.mu.RUnlock()
		rules := make([]*OnlineRule, 0, len(p.onlineRules))
		for _, rule := range p.onlineRules {
			rules = append(rules, rule)
		}
		sort.Slice(rules, func(i, j int) bool { return rules[i].Name < rules[j].Name })
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(rules)
	})

	r.Post("/rules", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		var newRule OnlineRule
		if err := json.NewDecoder(r.Body).Decode(&newRule); err != nil {
			jsonError(w, "Invalid request body", http.StatusBadRequest)
			return
		}

		newRule.Name = strings.TrimSpace(newRule.Name)
		newRule.URL = strings.TrimSpace(newRule.URL)
		if newRule.Name == "" || newRule.URL == "" {
			jsonError(w, "Name and URL are required", http.StatusBadRequest)
			return
		}
		if newRule.UpdateIntervalHours < 0 {
			jsonError(w, "UpdateIntervalHours cannot be negative", http.StatusBadRequest)
			return
		}

		newRule.ID = uuid.New().String()
		newRule.localPath = filepath.Join(p.dir, newRule.ID+".rules")
		newRule.LastUpdated = time.Time{}

		p.mu.Lock()
		p.onlineRules[newRule.ID] = &newRule
		p.mu.Unlock()

		if err := p.saveConfig(); err != nil {
			jsonError(w, "Failed to save config", http.StatusInternalServerError)
			return
		}

		// 下载新规则
		go func(ruleID string) {
			if newRule.Enabled {
				downloadCtx, cancel := context.WithTimeout(p.ctx, downloadTimeout)
				defer cancel()
				if err := p.downloadRule(downloadCtx, ruleID); err != nil {
					log.Printf("[adguard_rule] ERROR: failed to download new rule: %v", err)
				}
				// 下载完成后触发 reload (进而触发 notify)
				p.triggerReload(p.ctx)
			}
		}(newRule.ID)

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusCreated)
		json.NewEncoder(w).Encode(newRule)
	}))

	r.Put("/rules/{id}", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		id := chi.URLParam(r, "id")
		var updatedRuleData OnlineRule
		if err := json.NewDecoder(r.Body).Decode(&updatedRuleData); err != nil {
			jsonError(w, "Invalid request body", http.StatusBadRequest)
			return
		}

		updatedRuleData.Name = strings.TrimSpace(updatedRuleData.Name)
		updatedRuleData.URL = strings.TrimSpace(updatedRuleData.URL)
		if updatedRuleData.Name == "" || updatedRuleData.URL == "" {
			jsonError(w, "Name and URL are required", http.StatusBadRequest)
			return
		}
		if updatedRuleData.UpdateIntervalHours < 0 {
			jsonError(w, "UpdateIntervalHours cannot be negative", http.StatusBadRequest)
			return
		}

		p.mu.Lock()
		rule, ok := p.onlineRules[id]
		if !ok {
			p.mu.Unlock()
			jsonError(w, "Rule not found", http.StatusNotFound)
			return
		}

		rule.Name = updatedRuleData.Name
		rule.URL = updatedRuleData.URL
		rule.Enabled = updatedRuleData.Enabled
		rule.AutoUpdate = updatedRuleData.AutoUpdate
		rule.UpdateIntervalHours = updatedRuleData.UpdateIntervalHours
		p.mu.Unlock()

		if err := p.saveConfig(); err != nil {
			jsonError(w, "Failed to save config", http.StatusInternalServerError)
			return
		}

		// 配置变更触发 reload (进而触发 notify)
		p.triggerReload(r.Context())
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(rule)
	}))

	r.Delete("/rules/{id}", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		id := chi.URLParam(r, "id")

		p.mu.Lock()
		rule, ok := p.onlineRules[id]
		if !ok {
			p.mu.Unlock()
			jsonError(w, "Rule not found", http.StatusNotFound)
			return
		}
		localPath := rule.localPath
		delete(p.onlineRules, id)
		p.mu.Unlock()

		if err := os.Remove(localPath); err != nil && !os.IsNotExist(err) {
			log.Printf("[adguard_rule] WARN: failed to delete rule file %s: %v", localPath, err)
		}

		if err := p.saveConfig(); err != nil {
			jsonError(w, "Failed to save config", http.StatusInternalServerError)
			return
		}

		// 删除规则触发 reload (进而触发 notify)
		p.triggerReload(r.Context())
		w.WriteHeader(http.StatusNoContent)
	}))

	r.Post("/update", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		log.Println("[adguard_rule] Manual update triggered for all enabled rules.")

		go func() {
			p.mu.RLock()
			rulesToUpdate := make([]*OnlineRule, 0)
			for _, rule := range p.onlineRules {
				if rule.Enabled {
					rulesToUpdate = append(rulesToUpdate, rule)
				}
			}
			p.mu.RUnlock()

			var wg sync.WaitGroup
			for _, rule := range rulesToUpdate {
				wg.Add(1)
				go func(ruleID string) {
					defer wg.Done()
					downloadCtx, cancel := context.WithTimeout(p.ctx, downloadTimeout)
					defer cancel()
					if err := p.downloadRule(downloadCtx, ruleID); err != nil {
						log.Printf("[adguard_rule] ERROR: failed to update rule during manual update: %v", err)
					}
				}(rule.ID)
			}
			wg.Wait()

			log.Println("[adguard_rule] Manual update process finished.")
			// 更新完成触发 reload (进而触发 notify)
			p.triggerReload(p.ctx)
			coremain.ManualGC() 
		}()

		w.WriteHeader(http.StatusAccepted)
		fmt.Fprintln(w, "Update process for enabled rules has been started in the background.")
	}))

	return r
}
