/*
 * Copyright (C) 2020-2022, IrineSistiana
 * Copyright (C) 2024, a user of mosdns (modified for si_set)
 *
 * This file is part of mosdns.
 *
 * mosdns is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * mosdns is distributed in the hope that it is useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

package si_set

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
	"net/netip"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/go-chi/chi/v5"
	"github.com/sagernet/sing/common/varbin"
	"go4.org/netipx"
	"golang.org/x/net/proxy"
)

const (
	PluginType      = "si_set"
	downloadTimeout = 60 * time.Second
)

func init() {
	coremain.RegNewPluginFunc(PluginType, newSiSet, func() any { return new(Args) })
}

// Args holds the configuration for si_set plugin
type Args struct {
	Socks5      string `yaml:"socks5,omitempty"`
	LocalConfig string `yaml:"local_config"`
}

// RuleSource defines the structure for an online SRS rule source.
type RuleSource struct {
	Name                string    `json:"name"`
	Type                string    `json:"type"` // For informational purposes, e.g., "geoipcn"
	Files               string    `json:"files"`
	URL                 string    `json:"url"`
	Enabled             bool      `json:"enabled"`
	AutoUpdate          bool      `json:"auto_update"`
	UpdateIntervalHours int       `json:"update_interval_hours"`
	RuleCount           int       `json:"rule_count"`
	LastUpdated         time.Time `json:"last_updated"`
}

// [新增] 接口定义，用于解耦。参考 sd_set_light
type IPReceiver interface {
	Add(netip.Prefix)
}

// [新增] 规则列表接收器 (用于 reloadAllRules)
type listCollector struct {
	l *netlist.List
}

func (c *listCollector) Add(p netip.Prefix) {
	c.l.Append(p)
}

// [新增] 计数收集器 (用于校验，不存数据，省内存)
type counterCollector struct {
	count int
}

func (c *counterCollector) Add(_ netip.Prefix) {
	c.count++
}

// SiSet implements IPMatcherProvider and holds the state for the plugin.
type SiSet struct {
	matcher atomic.Value // Stores a netlist.Matcher for concurrent-safe access.

	mu      sync.RWMutex // Protects the sources map and related file I/O.
	sources map[string]*RuleSource

	localConfigFile string
	httpClient      *http.Client
	ctx             context.Context
	cancel          context.CancelFunc
}

// Ensure SiSet implements required interfaces.
var _ data_provider.IPMatcherProvider = (*SiSet)(nil)
var _ io.Closer = (*SiSet)(nil)

var _ netlist.Matcher = (*SiSet)(nil)

// newSiSet initializes the si_set plugin.
func newSiSet(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)
	if cfg.LocalConfig == "" {
		return nil, fmt.Errorf("%s: 'local_config' must be specified", PluginType)
	}

	// Configure HTTP client with optional SOCKS5 proxy
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

	p := &SiSet{
		sources:         make(map[string]*RuleSource),
		localConfigFile: cfg.LocalConfig,
		httpClient:      httpClient,
		ctx:             ctx,
		cancel:          cancel,
	}
	p.matcher.Store(netlist.NewList()) // Initialize with an empty list

	if err := p.loadConfig(); err != nil {
		log.Printf("[%s] failed to load config file '%s': %v. Starting with empty config.", PluginType, p.localConfigFile, err)
	}

	if err := p.reloadAllRules(); err != nil {
		log.Printf("[%s] failed to perform initial rule load: %v", PluginType, err)
	}

	bp.RegAPI(p.api())
	go p.backgroundUpdater()

	return p, nil
}

// GetIPMatcher provides the currently active IP matcher.
// [MODIFIED] Return the plugin instance itself instead of the internal list snapshot.
// This allows external callers to always use the proxy method `Match`, which delegates
// to the latest internal list.
func (p *SiSet) GetIPMatcher() netlist.Matcher {
	return p
}

// [NEW] Match implements netlist.Matcher.
// It atomically loads the latest internal matcher to perform the check.
// This ensures any update via API or auto-update is immediately effective
// without restarting the service.
func (p *SiSet) Match(addr netip.Addr) bool {
	// Atomic Load: Extremely fast (nanosecond scale) and thread-safe.
	m, ok := p.matcher.Load().(netlist.Matcher)
	if !ok || m == nil {
		return false
	}
	return m.Match(addr)
}

// Close gracefully shuts down the plugin.
func (p *SiSet) Close() error {
	log.Printf("[%s] closing...", PluginType)
	p.cancel()
	return nil
}

// loadConfig reads the rule source configuration from the local JSON file.
func (p *SiSet) loadConfig() error {
	p.mu.Lock()
	defer p.mu.Unlock()

	data, err := os.ReadFile(p.localConfigFile)
	if err != nil {
		if os.IsNotExist(err) {
			log.Printf("[%s] config file not found at %s, a new one will be created upon modification.", PluginType, p.localConfigFile)
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

	p.sources = make(map[string]*RuleSource, len(sources))
	for _, src := range sources {
		if src.Name == "" {
			log.Printf("[%s] WARN: found a rule source with an empty name, skipping.", PluginType)
			continue
		}
		p.sources[src.Name] = src
	}
	log.Printf("[%s] loaded %d rule sources from %s", PluginType, len(p.sources), p.localConfigFile)
	return nil
}

// saveConfig writes the current rule source configuration to the local JSON file.
// **MODIFIED**: Uses a write lock to ensure atomicity and prevent race conditions.
func (p *SiSet) saveConfig() error {
	p.mu.Lock()
	defer p.mu.Unlock()

	// Create a snapshot of sources to work on.
	sourcesSnapshot := make([]*RuleSource, 0, len(p.sources))
	for _, src := range p.sources {
		s := *src // Create a copy for safety
		sourcesSnapshot = append(sourcesSnapshot, &s)
	}

	// Sort the snapshot for consistent file output.
	sort.Slice(sourcesSnapshot, func(i, j int) bool {
		return sourcesSnapshot[i].Name < sourcesSnapshot[j].Name
	})

	data, err := json.MarshalIndent(sourcesSnapshot, "", "  ")
	if err != nil {
		return fmt.Errorf("failed to marshal config to json: %w", err)
	}

	// Atomic write: write to a temporary file then rename.
	tmpFile := p.localConfigFile + ".tmp"
	if err := os.WriteFile(tmpFile, data, 0644); err != nil {
		return fmt.Errorf("failed to write to temporary config file: %w", err)
	}
	if err := os.Rename(tmpFile, p.localConfigFile); err != nil {
		return fmt.Errorf("failed to rename temporary config to final: %w", err)
	}
	return nil
}


// reloadAllRules re-parses all enabled local SRS files into a new matcher.
func (p *SiSet) reloadAllRules() error {
	log.Printf("[%s] starting to reload all rules (optimized memory mode)...", PluginType)

	p.mu.RLock()
	// Create a snapshot of enabled sources to process.
	enabledSources := make([]*RuleSource, 0, len(p.sources))
	for _, src := range p.sources {
		if src.Enabled {
			enabledSources = append(enabledSources, src)
		}
	}
	p.mu.RUnlock()

	newList := netlist.NewList()
	collector := &listCollector{l: newList} // [优化] 使用收集器
	totalRules := 0
	configChanged := false

	for _, src := range enabledSources {
		if src.Files == "" {
			log.Printf("[%s] WARN: skipping enabled source '%s', local file path is empty.", PluginType, src.Name)
			continue
		}

		before := newList.Len()
		// [优化] 改为流式加载，不再一次性 ReadFile
		err := loadFromFile(src.Files, collector)
		after := newList.Len()
		count := after - before

		if err != nil {
			log.Printf("[%s] ERROR: failed to load rules for source '%s' from file %s: %v", PluginType, src.Name, src.Files, err)
			// If file is unloadable, set its count to 0.
			if src.RuleCount != 0 {
				p.mu.Lock()
				if s, ok := p.sources[src.Name]; ok {
					s.RuleCount = 0
					configChanged = true
				}
				p.mu.Unlock()
			}
			continue
		}

		totalRules += count
		log.Printf("[%s] loaded %d rules from source '%s'", PluginType, count, src.Name)

		// Update RuleCount in the config if it has changed.
		if src.RuleCount != count {
			p.mu.Lock()
			if s, ok := p.sources[src.Name]; ok {
				s.RuleCount = count
				configChanged = true
			}
			p.mu.Unlock()
		}
	}

	newList.Sort()
	p.matcher.Store(newList) // Atomically swap the matcher
	log.Printf("[%s] finished reloading. Total active rules: %d", PluginType, totalRules)

	if configChanged {
		log.Printf("[%s] Rule counts have changed, saving configuration...", PluginType)
		if err := p.saveConfig(); err != nil {
			log.Printf("[%s] ERROR: failed to save config after reloading rules: %v", PluginType, err)
		}
	}

	// [优化] 异步清理扫描期间产生的临时对象
	go func() {
		time.Sleep(1 * time.Second)
		coremain.ManualGC()
	}()

	return nil
}

// downloadAndUpdateLocalFile handles the download, validation, and file writing.
func (p *SiSet) downloadAndUpdateLocalFile(ctx context.Context, sourceName string) error {
	p.mu.RLock()
	source, ok := p.sources[sourceName]
	if !ok {
		p.mu.RUnlock()
		return fmt.Errorf("source '%s' not found", sourceName)
	}
	// Make copies of fields to use outside the lock
	sourceURL := source.URL
	localFile := source.Files
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

	// CRITICAL STEP: Validate the downloaded data in memory before writing to disk.
	// [优化] 使用 counterCollector，仅计数校验，不产生额外的 IP 对象内存开销
	counter := &counterCollector{}
	ok, count, _ := tryLoadSRS(bytes.NewReader(srsData), counter)
	if !ok {
		return fmt.Errorf("downloaded file for '%s' is not a valid SRS file or is corrupted", sourceName)
	}
	log.Printf("[%s] downloaded file for '%s' validated successfully with %d rules.", PluginType, sourceName, count)

	// Ensure the target directory exists.
	if err := os.MkdirAll(filepath.Dir(localFile), 0755); err != nil {
		return fmt.Errorf("failed to create directory for '%s': %w", localFile, err)
	}
	if err := os.WriteFile(localFile, srsData, 0644); err != nil {
		return fmt.Errorf("failed to write srs file for '%s': %w", sourceName, err)
	}

	// Update metadata. This part needs a lock.
	p.mu.Lock()
	if source, ok := p.sources[sourceName]; ok {
		source.RuleCount = count
		source.LastUpdated = time.Now()
	}
	p.mu.Unlock()

	// The subsequent saveConfig call will handle its own locking.
	if err := p.saveConfig(); err != nil {
		log.Printf("[%s] ERROR: failed to save config after updating '%s': %v", PluginType, sourceName, err)
	}

	return nil
}

// backgroundUpdater periodically checks for and updates rules.
func (p *SiSet) backgroundUpdater() {
	// Initial delay to allow system to stabilize.
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
			if err := p.reloadAllRules(); err != nil {
				log.Printf("[%s] ERROR: failed to reload rules after auto-update: %v", PluginType, err)
			}
			coremain.ManualGC()
		case <-p.ctx.Done():
			log.Printf("[%s] background updater is shutting down.", PluginType)
			return
		}
	}
}

// api sets up the HTTP API endpoints.
func (p *SiSet) api() *chi.Mux {
	r := chi.NewRouter()

	// GET /config: Retrieve the list of all rule sources.
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

	// POST /update/{name}: Manually trigger an update for a specific rule source.
	r.Post("/update/{name}", func(w http.ResponseWriter, r *http.Request) {
		name := chi.URLParam(r, "name")
		p.mu.RLock()
		_, ok := p.sources[name]
		p.mu.RUnlock()
		if !ok {
			jsonError(w, fmt.Sprintf("source '%s' not found", name), http.StatusNotFound)
			return
		}

		// Run update in a separate goroutine to avoid blocking the API response.
		go func() {
			log.Printf("[%s] manual update triggered for source '%s'.", PluginType, name)
			updateCtx, cancel := context.WithTimeout(p.ctx, downloadTimeout*2) // Give more time for manual updates
			defer cancel()
			if err := p.downloadAndUpdateLocalFile(updateCtx, name); err != nil {
				log.Printf("[%s] ERROR: failed to manually update source '%s': %v", PluginType, name, err)
				return // Don't reload if download fails
			}
			log.Printf("[%s] manual update for '%s' successful, triggering reload.", PluginType, name)
			if err := p.reloadAllRules(); err != nil {
				log.Printf("[%s] ERROR: failed to reload rules after manual update: %v", PluginType, err)
			}
			coremain.ManualGC()
		}()

		jsonResponse(w, map[string]string{"message": fmt.Sprintf("update process for '%s' started in the background", name)}, http.StatusAccepted)
	})

	// PUT /config/{name}: Add a new rule source or update an existing one.
	r.Put("/config/{name}", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		name := chi.URLParam(r, "name")
		var reqData RuleSource
		if err := json.NewDecoder(r.Body).Decode(&reqData); err != nil {
			jsonError(w, "invalid request body", http.StatusBadRequest)
			return
		}

		reqData.Name = name // Ensure name from URL is used.
		var updatedSource *RuleSource
		var statusCode int

		p.mu.Lock()
		existing, isUpdate := p.sources[name]
		if isUpdate {
			// Update existing source
			existing.Type = reqData.Type
			existing.Files = reqData.Files
			existing.URL = reqData.URL
			existing.Enabled = reqData.Enabled
			existing.AutoUpdate = reqData.AutoUpdate
			existing.UpdateIntervalHours = reqData.UpdateIntervalHours
			updatedSource = existing
			statusCode = http.StatusOK
		} else {
			// Add new source
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

		// Trigger a reload in the background to apply changes.
		go p.reloadAllRules()

		jsonResponse(w, updatedSource, statusCode)
	}))

	// DELETE /config/{name}: Delete a rule source and its local file.
	r.Delete("/config/{name}", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
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

		// Attempt to delete the associated local file.
		if srcToDelete.Files != "" {
			if err := os.Remove(srcToDelete.Files); err != nil && !os.IsNotExist(err) {
				log.Printf("[%s] WARN: failed to delete srs file %s: %v", PluginType, srcToDelete.Files, err)
			}
		}

		if err := p.saveConfig(); err != nil {
			jsonError(w, "failed to save config", http.StatusInternalServerError)
			return
		}

		// Trigger a reload to remove the rules from the matcher.
		go p.reloadAllRules()

		w.WriteHeader(http.StatusNoContent)
	}))

	return r
}

// --- Helper Functions for API and SRS Parsing (mostly from ip_set and sd_set) ---

func jsonResponse(w http.ResponseWriter, data any, statusCode int) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(statusCode)
	json.NewEncoder(w).Encode(data)
}

func jsonError(w http.ResponseWriter, message string, code int) {
	jsonResponse(w, map[string]string{"error": message}, code)
}

// [修改] 改为接收 IPReceiver 接口，并支持 io.Reader 流式解析
func loadFromFile(path string, m IPReceiver) error {
	if path == "" {
		return nil
	}

	// [优化] 流式读取文件，不再 ReadFile 到内存
	f, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			// This is not a fatal error during reload, just means the file isn't there yet.
			log.Printf("[%s] file not found, skipping: %s", PluginType, path)
			return nil
		}
		return err
	}
	defer f.Close()

	// Only try .srs binary format as per requirement.
	// [优化] tryLoadSRS 参数改为 io.Reader
	ok, _, _ := tryLoadSRS(f, m)
	if !ok {
		return fmt.Errorf("file %s is not a valid or supported SRS file", path)
	}

	return nil
}

// --- SRS Parsing Logic (Optimized for low memory overhead) ---
var (
	srsMagic            = [3]byte{'S', 'R', 'S'}
	ruleItemIPCIDR      = uint8(6)
	ruleItemFinal       = uint8(0xFF)
	maxSupportedVersion = uint8(3)
)

// [修改] 参数 data []byte 改为 r io.Reader，收集器改为 IPReceiver 接口
func tryLoadSRS(r io.Reader, m IPReceiver) (bool, int, string) {
	var mb [3]byte
	if _, err := io.ReadFull(r, mb[:]); err != nil || mb != srsMagic {
		return false, 0, ""
	}
	var version uint8
	if err := binary.Read(r, binary.BigEndian, &version); err != nil || version > maxSupportedVersion {
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
	count := 0
	var last string
	for i := uint64(0); i < length; i++ {
		c, lr := readRule(br, m)
		if lr != "" {
			last = lr
		}
		count += c
	}
	return true, count, last
}

// [修改] 类型改为 IPReceiver 接口
func readRule(r *bufio.Reader, m IPReceiver) (int, string) {
	ct := 0
	var last string
	mode, err := r.ReadByte()
	if err != nil {
		return 0, ""
	}
	switch mode {
	case 0:
		c, lr := readDefault(r, m)
		ct += c
		if lr != "" {
			last = lr
		}
	case 1:
		_, _ = r.ReadByte()
		n, _ := binary.ReadUvarint(r)
		for j := uint64(0); j < n; j++ {
			c, lr := readRule(r, m)
			ct += c
			if lr != "" {
				last = lr
			}
		}
		_, _ = r.ReadByte()
	}
	return ct, last
}

// [修改] 类型改为 IPReceiver 接口
func readDefault(r *bufio.Reader, m IPReceiver) (int, string) {
	count := 0
	var last string
	for {
		item, err := r.ReadByte()
		if err != nil {
			break
		}
		switch item {
		case ruleItemIPCIDR:
			// [核心优化点] parseIPSet 修改为逐个读取模式，直接注入 receiver，不再分配大数据切片
			err := streamParseIPSet(r, m, &count, &last)
			if err != nil {
				return count, last
			}
		case ruleItemFinal:
			return count, last
		default:
			// Unrecognized item, stop parsing this rule section.
			return count, last
		}
	}
	return count, last
}

// [新增/优化] 原 parseIPSet 会分配巨大的 ranges 切片。
// 现在改为 streamParseIPSet，在解析过程中逐个处理 Range，彻底消除内存尖峰。
func streamParseIPSet(r *bufio.Reader, m IPReceiver, count *int, last *string) error {
	ver, err := r.ReadByte()
	if err != nil {
		return err
	}
	if ver != 1 {
		return fmt.Errorf("unsupported ipset version: %d", ver)
	}
	var length uint64
	if err := binary.Read(r, binary.BigEndian, &length); err != nil {
		return err
	}

	// [优化] 核心逻辑：逐个循环解析 varbin 数据，不再分配巨大的 ranges 切片
	for i := uint64(0); i < length; i++ {
		// 读取单条 Range 数据
		fromData, err := varbin.ReadValue[[]byte](r, binary.BigEndian)
		if err != nil {
			return err
		}
		toData, err := varbin.ReadValue[[]byte](r, binary.BigEndian)
		if err != nil {
			return err
		}

		from, ok1 := netip.AddrFromSlice(fromData)
		to, ok2 := netip.AddrFromSlice(toData)
		if !ok1 || !ok2 {
			continue
		}

		// 使用 netipx 处理该 Range。
		// 虽然这步会产生少量对象，但它是随用随抛的，不会堆积。
		var builder netipx.IPSetBuilder
		builder.AddRange(netipx.IPRangeFrom(from, to))
		ipset, _ := builder.IPSet()
		for _, pfx := range ipset.Prefixes() {
			m.Add(pfx) // 注入最终的 netlist 或计数器
			*count++
			*last = pfx.String()
		}
	}
	return nil
}
