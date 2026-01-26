package sd_set

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"context"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/go-chi/chi/v5"
	scdomain "github.com/sagernet/sing/common/domain"
	"github.com/sagernet/sing/common/varbin"
	"golang.org/x/net/proxy"
)

const (
	PluginType      = "sd_set"
	downloadTimeout = 60 * time.Second
)

func init() {
	coremain.RegNewPluginFunc(PluginType, newSdSet, func() any { return new(Args) })
}

type Args struct {
	Socks5      string `yaml:"socks5,omitempty"`
	LocalConfig string `yaml:"local_config"`
}

type RuleSource struct {
	Name                string    `json:"name"`
	Type                string    `json:"type"`
	Files               string    `json:"files"`
	URL                 string    `json:"url"`
	Enabled             bool      `json:"enabled"`
	EnableRegexp        bool      `json:"enable_regexp,omitempty"` // Added: 默认为 false
	AutoUpdate          bool      `json:"auto_update"`
	UpdateIntervalHours int       `json:"update_interval_hours"`
	RuleCount           int       `json:"rule_count"`
	LastUpdated         time.Time `json:"last_updated"`
}

type SdSet struct {
	matcher atomic.Value // 使用 atomic.Value 来安全地读写 matcher 指针

	mu      sync.RWMutex // mu 保护 sources map 和相关的配置文件写入
	sources map[string]*RuleSource

	localConfigFile string
	httpClient      *http.Client
	ctx             context.Context
	cancel          context.CancelFunc

	// 新增：订阅者
	subscribers []func()
	subsMu      sync.RWMutex
}

var _ data_provider.DomainMatcherProvider = (*SdSet)(nil)
var _ io.Closer = (*SdSet)(nil)
// 确保实现了 RuleExporter 接口
var _ data_provider.RuleExporter = (*SdSet)(nil)

// RuleReceiver 接口用于解耦 SRS 解析和具体的 Matcher
type RuleReceiver interface {
	Add(string, struct{}) error
}

// ruleCollector 用于 GetRules 时收集规则
type ruleCollector struct {
	rules []string
}

func (c *ruleCollector) Add(s string, _ struct{}) error {
	c.rules = append(c.rules, s)
	return nil
}

// Subscribe 实现 RuleExporter
func (p *SdSet) Subscribe(cb func()) {
	p.subsMu.Lock()
	defer p.subsMu.Unlock()
	p.subscribers = append(p.subscribers, cb)
}

// GetRules 实现 RuleExporter
// 注意：这会读取所有启用的本地文件并重新解析规则，以获取字符串形式的规则列表
func (p *SdSet) GetRules() ([]string, error) {
	p.mu.RLock()
	sourcesSnapshot := make([]*RuleSource, 0, len(p.sources))
	for _, src := range p.sources {
		if src.Enabled {
			sourcesSnapshot = append(sourcesSnapshot, src)
		}
	}
	p.mu.RUnlock()

	collector := &ruleCollector{rules: make([]string, 0)}

	for _, src := range sourcesSnapshot {
		if src.Files == "" {
			continue
		}
		b, err := os.ReadFile(src.Files)
		if err != nil {
			// 在导出模式下，如果文件不可读，记录日志但继续处理其他文件
			log.Printf("[%s] GetRules: WARN: cannot read file %s: %v", PluginType, src.Files, err)
			continue
		}
		// 使用通用的 tryLoadSRS，传入 collector
		tryLoadSRS(b, collector, src.EnableRegexp)
	}
	return collector.rules, nil
}

func (p *SdSet) notifySubscribers() {
	p.subsMu.RLock()
	subs := make([]func(), len(p.subscribers))
	copy(subs, p.subscribers)
	p.subsMu.RUnlock()

	for _, cb := range subs {
		go cb()
	}
}

func newSdSet(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)
	if cfg.LocalConfig == "" {
		return nil, fmt.Errorf("%s: 'local_config' must be specified", PluginType)
	}

	transport := &http.Transport{
		Proxy: http.ProxyFromEnvironment,
		DialContext: (&net.Dialer{
			Timeout:   30 * time.Second,
			KeepAlive: 30 * time.Second,
		}).DialContext,
		TLSHandshakeTimeout: 10 * time.Second,
	}
	if cfg.Socks5 != "" {
		log.Printf("[%s] using SOCKS5 proxy: %s", PluginType, cfg.Socks5)
		dialer, err := proxy.SOCKS5("tcp", cfg.Socks5, nil, proxy.Direct)
		if err != nil {
			return nil, fmt.Errorf("%s: failed to create SOCKS5 dialer: %w", PluginType, err)
		}
		contextDialer, ok := dialer.(proxy.ContextDialer)
		if !ok {
			return nil, fmt.Errorf("%s: created dialer does not support context", PluginType)
		}
		transport.DialContext = contextDialer.DialContext
		transport.Proxy = nil
	}
	httpClient := &http.Client{
		Timeout:   downloadTimeout,
		Transport: transport,
	}

	ctx, cancel := context.WithCancel(context.Background())

	p := &SdSet{
		sources:         make(map[string]*RuleSource),
		localConfigFile: cfg.LocalConfig,
		httpClient:      httpClient,
		ctx:             ctx,
		cancel:          cancel,
		subscribers:     make([]func(), 0),
	}
	p.matcher.Store(domain.NewDomainMixMatcher()) // 初始化为一个空的 matcher

	if err := p.loadConfig(); err != nil {
		log.Printf("[%s] failed to load config file: %v. Starting with empty config.", PluginType, err)
	}

	if err := p.reloadAllRules(); err != nil {
		log.Printf("[%s] failed to perform initial rule load: %v", PluginType, err)
	}

	bp.RegAPI(p.api())
	go p.backgroundUpdater()

	return p, nil
}

func (p *SdSet) Close() error {
	log.Printf("[%s] closing...", PluginType)
	p.cancel()
	return nil
}

func (p *SdSet) GetDomainMatcher() domain.Matcher[struct{}] {
	return p
}

func (p *SdSet) Match(domainStr string) (value struct{}, ok bool) {
	m := p.matcher.Load().(*domain.MixMatcher[struct{}])
	return m.Match(domainStr)
}

func (p *SdSet) loadConfig() error {
	p.mu.Lock()
	defer p.mu.Unlock()

	data, err := os.ReadFile(p.localConfigFile)
	if err != nil {
		if os.IsNotExist(err) {
			log.Printf("[%s] config file not found at %s, will create a new one.", PluginType, p.localConfigFile)
			return nil
		}
		return err
	}
	if len(data) == 0 {
		p.sources = make(map[string]*RuleSource)
		return nil
	}

	var sources []*RuleSource
	if err := json.Unmarshal(data, &sources); err != nil {
		return fmt.Errorf("failed to parse config json: %w", err)
	}

	if len(sources) == 0 {
		log.Printf("[%s] WARN: config file %s is not empty, but parsed 0 rules. Treating as empty config.", PluginType, p.localConfigFile)
	}

	p.sources = make(map[string]*RuleSource, len(sources))
	for _, src := range sources {
		if src.Name == "" {
			log.Printf("[%s] WARN: found a rule source with empty name, skipping.", PluginType)
			continue
		}
		p.sources[src.Name] = src
	}
	log.Printf("[%s] loaded %d rule sources from %s", PluginType, len(p.sources), p.localConfigFile)
	return nil
}

// **MODIFIED**: This function now uses a write lock to ensure the entire file-saving
// process is atomic, preventing race conditions from concurrent calls.
func (p *SdSet) saveConfig() error {
	p.mu.Lock()
	defer p.mu.Unlock()

	// Create a snapshot of the sources map.
	sourcesSnapshot := make([]*RuleSource, 0, len(p.sources))
	for _, src := range p.sources {
		s := *src // Create a copy for safety
		sourcesSnapshot = append(sourcesSnapshot, &s)
	}

	// Perform slow operations on the snapshot.
	sort.Slice(sourcesSnapshot, func(i, j int) bool {
		return sourcesSnapshot[i].Name < sourcesSnapshot[j].Name
	})

	data, err := json.MarshalIndent(sourcesSnapshot, "", "  ")
	if err != nil {
		return fmt.Errorf("failed to marshal config to json: %w", err)
	}

	// Atomically write the file.
	tmpFile := p.localConfigFile + ".tmp"
	if err := os.WriteFile(tmpFile, data, 0644); err != nil {
		return fmt.Errorf("failed to write to temporary config file: %w", err)
	}
	if err := os.Rename(tmpFile, p.localConfigFile); err != nil {
		return fmt.Errorf("failed to rename temporary config to final: %w", err)
	}
	return nil
}

func (p *SdSet) reloadAllRules() error {
	log.Printf("[%s] starting to reload all rules...", PluginType)

	p.mu.RLock()
	sourcesSnapshot := make([]*RuleSource, 0, len(p.sources))
	for _, src := range p.sources {
		if src.Enabled {
			sourcesSnapshot = append(sourcesSnapshot, src)
		}
	}
	p.mu.RUnlock()

	newMatcher := domain.NewDomainMixMatcher()
	totalRules := 0
	rulesCountUpdated := false

	for _, src := range sourcesSnapshot {
		if src.Files == "" {
			log.Printf("[%s] WARN: skipping enabled source '%s', local file path is empty.", PluginType, src.Name)
			continue
		}

		b, err := os.ReadFile(src.Files)
		if err != nil {
			log.Printf("[%s] WARN: skipping source '%s', cannot read file %s: %v", PluginType, src.Name, src.Files, err)
			p.mu.Lock()
			if s, ok := p.sources[src.Name]; ok && s.RuleCount != 0 {
				s.RuleCount = 0
				rulesCountUpdated = true
			}
			p.mu.Unlock()
			continue
		}

		// Modified: pass src.EnableRegexp
		ok, count, lastRule := tryLoadSRS(b, newMatcher, src.EnableRegexp)
		if !ok {
			log.Printf("[%s] ERROR: failed to load SRS file for source '%s' from %s", PluginType, src.Name, src.Files)
			continue
		}
		totalRules += count
		log.Printf("[%s] loaded %d rules from source '%s' (file: %s, last rule: %s)", PluginType, count, src.Name, src.Files, lastRule)

		p.mu.Lock()
		if s, ok := p.sources[src.Name]; ok && s.RuleCount != count {
			s.RuleCount = count
			rulesCountUpdated = true
		}
		p.mu.Unlock()
	}

	p.matcher.Store(newMatcher)
	log.Printf("[%s] finished reloading. Total active rules: %d", PluginType, totalRules)

	if rulesCountUpdated {
		log.Printf("[%s] Rule counts have changed, saving configuration...", PluginType)
		if err := p.saveConfig(); err != nil {
			log.Printf("[%s] ERROR: failed to save config after reloading rules: %v", PluginType, err)
		}
	}

	// 规则更新完毕（无论是手动、API还是定时器），通知订阅者
	p.notifySubscribers()

	return nil
}

func (p *SdSet) downloadAndUpdateLocalFile(ctx context.Context, sourceName string) error {
	p.mu.RLock()
	source, ok := p.sources[sourceName]
	if !ok {
		p.mu.RUnlock()
		return fmt.Errorf("source '%s' not found", sourceName)
	}
	sourceURL := source.URL
	localFile := source.Files
	enableRegexp := source.EnableRegexp // Added: retrieve config
	p.mu.RUnlock()

	if sourceURL == "" {
		return fmt.Errorf("source '%s' has no URL configured", sourceName)
	}
	if localFile == "" {
		return fmt.Errorf("source '%s' has no local file path configured", sourceName)
	}

	log.Printf("[%s] downloading rule for '%s' from %s", PluginType, sourceName, sourceURL)
	req, err := http.NewRequestWithContext(ctx, "GET", sourceURL, nil)
	if err != nil {
		return fmt.Errorf("failed to create request for '%s': %w", sourceName, err)
	}

	resp, err := p.httpClient.Do(req)
	if err != nil {
		return fmt.Errorf("http request failed for '%s': %w", sourceName, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("bad status code for '%s': %d", sourceName, resp.StatusCode)
	}

	srsData, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("failed to read response body for '%s': %w", sourceName, err)
	}

	tempMatcher := domain.NewDomainMixMatcher()
	// Modified: pass enableRegexp to validation
	ok, count, _ := tryLoadSRS(srsData, tempMatcher, enableRegexp)
	if !ok {
		return fmt.Errorf("downloaded file for '%s' is not a valid SRS file or is corrupted", sourceName)
	}
	log.Printf("[%s] downloaded file for '%s' validated successfully with %d rules.", PluginType, sourceName, count)

	if err := os.MkdirAll(filepath.Dir(localFile), 0755); err != nil {
		return fmt.Errorf("failed to create directory for '%s': %w", localFile, err)
	}
	if err := os.WriteFile(localFile, srsData, 0644); err != nil {
		return fmt.Errorf("failed to write srs file for '%s': %w", sourceName, err)
	}

	p.mu.Lock()
	if source, ok := p.sources[sourceName]; ok {
		source.RuleCount = count
		source.LastUpdated = time.Now()
	}
	p.mu.Unlock()

	if err := p.saveConfig(); err != nil {
		log.Printf("[%s] ERROR: failed to save config after updating '%s': %v", PluginType, sourceName, err)
	}

	return nil
}

func (p *SdSet) backgroundUpdater() {
	select {
	case <-time.After(1 * time.Minute):
	case <-p.ctx.Done():
		return
	}
	ticker := time.NewTicker(10 * time.Minute)
	defer ticker.Stop()
	for {
		select {
		case <-ticker.C:
			p.mu.RLock()
			var sourcesToUpdate []string
			for name, src := range p.sources {
				if src.Enabled && src.AutoUpdate && src.UpdateIntervalHours > 0 {
					if time.Since(src.LastUpdated).Hours() >= float64(src.UpdateIntervalHours) {
						sourcesToUpdate = append(sourcesToUpdate, name)
					}
				}
			}
			p.mu.RUnlock()
			if len(sourcesToUpdate) == 0 {
				continue
			}
			log.Printf("[%s] auto-update: found %d source(s) that need updating.", PluginType, len(sourcesToUpdate))
			var wg sync.WaitGroup
			for _, name := range sourcesToUpdate {
				wg.Add(1)
				go func(sourceName string) {
					defer wg.Done()
					updateCtx, cancel := context.WithTimeout(p.ctx, downloadTimeout)
					defer cancel()
					if err := p.downloadAndUpdateLocalFile(updateCtx, sourceName); err != nil {
						log.Printf("[%s] ERROR: failed to auto-update source '%s': %v", PluginType, sourceName, err)
					}
				}(name)
			}
			wg.Wait()
			log.Printf("[%s] auto-update: downloads finished, triggering reload.", PluginType)
			p.reloadAllRules()
		case <-p.ctx.Done():
			log.Printf("[%s] background updater is shutting down.", PluginType)
			return
		}
	}
}

func (p *SdSet) api() *chi.Mux {
	r := chi.NewRouter()
	r.Get("/config", func(w http.ResponseWriter, r *http.Request) {
		p.mu.RLock()
		defer p.mu.RUnlock()
		sources := make([]*RuleSource, 0, len(p.sources))
		for _, src := range p.sources {
			sources = append(sources, src)
		}
		sort.Slice(sources, func(i, j int) bool { return sources[i].Name < sources[j].Name })
		jsonResponse(w, sources, http.StatusOK)
	})
	r.Post("/update/{name}", func(w http.ResponseWriter, r *http.Request) {
		name := chi.URLParam(r, "name")
		p.mu.RLock()
		_, ok := p.sources[name]
		p.mu.RUnlock()
		if !ok {
			jsonError(w, fmt.Sprintf("source '%s' not found", name), http.StatusNotFound)
			return
		}
		go func() {
			log.Printf("[%s] manual update triggered for source '%s'.", PluginType, name)
			updateCtx, cancel := context.WithTimeout(p.ctx, downloadTimeout)
			defer cancel()
			if err := p.downloadAndUpdateLocalFile(updateCtx, name); err != nil {
				log.Printf("[%s] ERROR: failed to manually update source '%s': %v", PluginType, name, err)
				return
			}
			log.Printf("[%s] manual update for '%s' successful, triggering reload.", PluginType, name)
			p.reloadAllRules()
		}()
		jsonResponse(w, map[string]string{"message": fmt.Sprintf("update process for '%s' started in the background", name)}, http.StatusAccepted)
	})
	r.Put("/config/{name}", func(w http.ResponseWriter, r *http.Request) {
		name := chi.URLParam(r, "name")
		var reqData RuleSource
		if err := json.NewDecoder(r.Body).Decode(&reqData); err != nil {
			jsonError(w, "invalid request body", http.StatusBadRequest)
			return
		}
		if strings.TrimSpace(reqData.Files) == "" || strings.TrimSpace(reqData.URL) == "" {
			jsonError(w, "'files' and 'url' fields are required", http.StatusBadRequest)
			return
		}
		reqData.Name = name
		var updatedSource *RuleSource
		var statusCode int
		p.mu.Lock()
		existing, isUpdate := p.sources[name]
		if isUpdate {
			existing.Type = reqData.Type
			existing.Files = reqData.Files
			existing.URL = reqData.URL
			existing.Enabled = reqData.Enabled
			existing.EnableRegexp = reqData.EnableRegexp // Added: update config
			existing.AutoUpdate = reqData.AutoUpdate
			existing.UpdateIntervalHours = reqData.UpdateIntervalHours
			updatedSource = existing
			statusCode = http.StatusOK
		} else {
			reqData.RuleCount = 0
			reqData.LastUpdated = time.Time{}
			p.sources[name] = &reqData
			updatedSource = &reqData
			statusCode = http.StatusCreated
		}
		p.mu.Unlock()
		if err := p.saveConfig(); err != nil {
			jsonError(w, "failed to save config", http.StatusInternalServerError)
			return
		}
		go p.reloadAllRules()
		jsonResponse(w, updatedSource, statusCode)
	})
	r.Delete("/config/{name}", func(w http.ResponseWriter, r *http.Request) {
		name := chi.URLParam(r, "name")
		var srcToDelete *RuleSource
		p.mu.Lock()
		src, ok := p.sources[name]
		if ok {
			srcToDelete = src
			delete(p.sources, name)
		}
		p.mu.Unlock()
		if !ok {
			jsonError(w, "source not found", http.StatusNotFound)
			return
		}
		if srcToDelete.Files != "" {
			if err := os.Remove(srcToDelete.Files); err != nil && !os.IsNotExist(err) {
				log.Printf("[%s] WARN: failed to delete srs file %s: %v", PluginType, srcToDelete.Files, err)
			}
		}
		if err := p.saveConfig(); err != nil {
			jsonError(w, "failed to save config", http.StatusInternalServerError)
			return
		}
		go p.reloadAllRules()
		w.WriteHeader(http.StatusNoContent)
	})
	return r
}

func jsonResponse(w http.ResponseWriter, data any, statusCode int) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(statusCode)
	json.NewEncoder(w).Encode(data)
}

func jsonError(w http.ResponseWriter, message string, code int) {
	jsonResponse(w, map[string]string{"error": message}, code)
}

var (
	magicBytes            = [3]byte{0x53, 0x52, 0x53}
	ruleItemDomain        = uint8(2)
	ruleItemDomainKeyword = uint8(3)
	ruleItemDomainRegex   = uint8(4)
	ruleItemFinal         = uint8(0xFF)
)

const ruleSetVersionCurrent = 3

// Modified: added enableRegexp parameter and RuleReceiver interface
func tryLoadSRS(b []byte, m RuleReceiver, enableRegexp bool) (ok bool, count int, lastRule string) {
	r := bytes.NewReader(b)
	var mb [3]byte
	if _, err := io.ReadFull(r, mb[:]); err != nil || mb != magicBytes {
		return false, 0, ""
	}
	var version uint8
	if err := binary.Read(r, binary.BigEndian, &version); err != nil || version > ruleSetVersionCurrent {
		return false, 0, ""
	}
	zr, err := zlib.NewReader(r)
	if err != nil {
		return false, 0, ""
	}
	defer zr.Close()
	br := bufio.NewReader(zr)
	length, err := binary.ReadUvarint(br)
	if err != nil {
		return false, 0, ""
	}
	for i := uint64(0); i < length; i++ {
		count += readRuleCompat(br, m, &lastRule, enableRegexp)
	}
	return true, count, lastRule
}

// Modified: added enableRegexp parameter and RuleReceiver interface
func readRuleCompat(r *bufio.Reader, m RuleReceiver, last *string, enableRegexp bool) int {
	ct := 0
	mode, err := r.ReadByte()
	if err != nil {
		return 0
	}
	switch mode {
	case 0:
		ct += readDefaultRuleCompat(r, m, last, enableRegexp)
	case 1:
		r.ReadByte()
		n, _ := binary.ReadUvarint(r)
		for i := uint64(0); i < n; i++ {
			ct += readRuleCompat(r, m, last, enableRegexp)
		}
		r.ReadByte()
	}
	return ct
}

// Modified: added enableRegexp parameter and RuleReceiver interface
func readDefaultRuleCompat(r *bufio.Reader, m RuleReceiver, last *string, enableRegexp bool) int {
	count := 0
	for {
		item, err := r.ReadByte()
		if err != nil {
			break
		}
		switch item {
		case ruleItemDomain:
			matcher, err := scdomain.ReadMatcher(r)
			if err != nil {
				return count
			}
			doms, suffix := matcher.Dump()
			for _, d := range doms {
				*last = "full:" + d
				if m.Add(*last, struct{}{}) == nil {
					count++
				}
			}
			for _, d := range suffix {
				*last = "domain:" + d
				if m.Add(*last, struct{}{}) == nil {
					count++
				}
			}
		case ruleItemDomainKeyword:
			sl, _ := varbin.ReadValue[[]string](r, binary.BigEndian)
			for _, d := range sl {
				*last = "keyword:" + d
				if m.Add(*last, struct{}{}) == nil {
					count++
				}
			}
		case ruleItemDomainRegex:
			sl, _ := varbin.ReadValue[[]string](r, binary.BigEndian)
			// Modified: check enableRegexp before adding
			if enableRegexp {
				for _, d := range sl {
					*last = "regexp:" + d
					if m.Add(*last, struct{}{}) == nil {
						count++
					}
				}
			}
		case ruleItemFinal:
			return count
		default:
			return count
		}
	}
	return count
}
