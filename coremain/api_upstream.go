package coremain

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/go-chi/chi/v5"
	"go.uber.org/zap"
)

const upstreamOverridesFilename = "upstream_overrides.json"

// UpstreamOverrideConfig 定义 UI/API 交互的完整数据结构
type UpstreamOverrideConfig struct {
	Tag      string `json:"tag"`      // 上游名称 (Upstream Name)
	Enabled  bool   `json:"enabled"`  // 是否启用
	Protocol string `json:"protocol"` // UI类型: aliapi, udp, tcp, dot, doh...

	// 通用字段
	Addr                 string `json:"addr,omitempty"`
	DialAddr             string `json:"dial_addr,omitempty"`
	IdleTimeout          int    `json:"idle_timeout,omitempty"`
	UpstreamQueryTimeout int    `json:"upstream_query_timeout,omitempty"`

	// DNS (DoT/DoH/TCP/UDP) 专用
	EnablePipeline     bool   `json:"enable_pipeline,omitempty"`
	EnableHTTP3        bool   `json:"enable_http3,omitempty"`
	InsecureSkipVerify bool   `json:"insecure_skip_verify,omitempty"`
	Socks5             string `json:"socks5,omitempty"`
	SoMark             int    `json:"so_mark,omitempty"`
	BindToDevice       string `json:"bind_to_device,omitempty"`
	Bootstrap          string `json:"bootstrap,omitempty"`
	BootstrapVer       int    `json:"bootstrap_version,omitempty"`

	// AliAPI 专用
	AccountID       string `json:"account_id,omitempty"`
	AccessKeyID     string `json:"access_key_id,omitempty"`
	AccessKeySecret string `json:"access_key_secret,omitempty"`
	ServerAddr      string `json:"server_addr,omitempty"`
	EcsClientIP     string `json:"ecs_client_ip,omitempty"`
	EcsClientMask   uint8  `json:"ecs_client_mask,omitempty"`
}

// GlobalUpstreamOverrides 映射关系: 插件Tag -> 上游配置列表
type GlobalUpstreamOverrides map[string][]UpstreamOverrideConfig

type upstreamReloader interface {
	ReloadFromOverrides() error
}

type cacheFlusher interface {
	Flush() error
}

type upstreamStateInspector interface {
	CurrentUpstreamTargets() []string
}

var (
	upstreamOverridesLock sync.RWMutex
	upstreamOverrides     GlobalUpstreamOverrides
	upstreamAPIHost       *Mosdns
)

// RegisterUpstreamAPI 注册路由
func RegisterUpstreamAPI(router *chi.Mux, m *Mosdns) {
	upstreamAPIHost = m
	router.Route("/api/v1/upstream", func(r chi.Router) {
		r.Get("/tags", handleGetAliAPITags)
		r.Get("/config", handleGetUpstreamConfig)
		r.Get("/runtime/{tag}", handleGetUpstreamRuntimeState)
		r.Post("/config", handleSetUpstreamConfig)
	})
}

// GetUpstreamOverrides 供 aliapi 插件初始化调用
func GetUpstreamOverrides(pluginTag string) []UpstreamOverrideConfig {
	upstreamOverridesLock.RLock()
	defer upstreamOverridesLock.RUnlock()

	if upstreamOverrides == nil {
		// 释放读锁，获取写锁加载 (简化处理)
		upstreamOverridesLock.RUnlock()
		_ = loadUpstreamOverrides()
		upstreamOverridesLock.RLock()
	}

	entries, ok := upstreamOverrides[pluginTag]
	if !ok || len(entries) == 0 {
		return nil
	}
	return entries
}

// loadUpstreamOverrides 内部加载函数
func loadUpstreamOverrides() error {
	upstreamOverridesLock.Lock()
	defer upstreamOverridesLock.Unlock()

	dir := MainConfigBaseDir
	if dir == "" {
		dir = "."
	}
	// 获取绝对路径用于 Debug
	absDir, _ := filepath.Abs(dir)
	path := filepath.Join(dir, upstreamOverridesFilename)

	mlog.L().Info("[Debug UpstreamAPI] Loading overrides",
		zap.String("MainConfigBaseDir", dir),
		zap.String("AbsoluteDir", absDir),
		zap.String("File", path))

	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			mlog.L().Info("[Debug UpstreamAPI] File not found, creating new map", zap.String("path", path))
			upstreamOverrides = make(GlobalUpstreamOverrides)
			return nil
		}
		mlog.L().Error("[Debug UpstreamAPI] Failed to read file", zap.Error(err))
		return err
	}

	var cfg GlobalUpstreamOverrides
	if err := json.Unmarshal(data, &cfg); err != nil {
		mlog.L().Error("[Debug UpstreamAPI] JSON parse error", zap.Error(err))
		return err
	}

	// Count items for debug
	count := 0
	for _, v := range cfg {
		count += len(v)
	}
	mlog.L().Info("[Debug UpstreamAPI] Loaded success", zap.Int("groups", len(cfg)), zap.Int("total_items", count))

	upstreamOverrides = cfg
	return nil
}

// saveUpstreamOverrides 内部保存函数
func saveUpstreamOverrides() error {
	dir := MainConfigBaseDir
	if dir == "" {
		dir = "."
	}

	path := filepath.Join(dir, upstreamOverridesFilename)
	absPath, _ := filepath.Abs(path)

	data, err := json.MarshalIndent(upstreamOverrides, "", "  ")
	if err != nil {
		mlog.L().Error("[Debug UpstreamAPI] JSON marshal failed", zap.Error(err))
		return err
	}

	mlog.L().Info("[Debug UpstreamAPI] Writing to file",
		zap.String("path", path),
		zap.String("abs_path", absPath),
		zap.Int("bytes", len(data)))

	err = writeManagedFile(path, data, func(raw []byte) error {
		var cfg GlobalUpstreamOverrides
		return json.Unmarshal(raw, &cfg)
	}, nil, nil)
	if err != nil {
		mlog.L().Error("[Debug UpstreamAPI] WriteFile FAILED", zap.Error(err))
	} else {
		mlog.L().Info("[Debug UpstreamAPI] WriteFile SUCCESS")
	}
	return err
}

// handleGetAliAPITags 获取扫描到的插件 Tag
func handleGetAliAPITags(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	tags := discoveredAliAPITags
	if tags == nil {
		tags = []string{}
	}
	// DEBUG
	mlog.L().Info("[Debug UpstreamAPI] API Request: Get Tags", zap.Strings("returning", tags))
	json.NewEncoder(w).Encode(tags)
}

// handleGetUpstreamConfig 获取当前所有配置
func handleGetUpstreamConfig(w http.ResponseWriter, r *http.Request) {
	if upstreamOverrides == nil {
		_ = loadUpstreamOverrides()
	}
	upstreamOverridesLock.RLock()
	defer upstreamOverridesLock.RUnlock()

	safeData := upstreamOverrides
	if safeData == nil {
		safeData = make(GlobalUpstreamOverrides)
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(safeData)
}

func handleGetUpstreamRuntimeState(w http.ResponseWriter, r *http.Request) {
	tag := chi.URLParam(r, "tag")
	if tag == "" {
		http.Error(w, `{"error": "tag is required"}`, http.StatusBadRequest)
		return
	}
	if upstreamAPIHost == nil {
		http.Error(w, `{"error": "Upstream runtime is not initialized"}`, http.StatusInternalServerError)
		return
	}

	type resp struct {
		Tag            string                   `json:"tag"`
		OverrideConfig []UpstreamOverrideConfig `json:"override_config,omitempty"`
		RuntimeTargets []string                 `json:"runtime_targets,omitempty"`
	}

	out := resp{Tag: tag}
	out.OverrideConfig = GetUpstreamOverrides(tag)

	p := upstreamAPIHost.GetPlugin(tag)
	if p == nil {
		http.Error(w, `{"error": "Target upstream plugin is not loaded"}`, http.StatusNotFound)
		return
	}
	if inspector, ok := p.(upstreamStateInspector); ok {
		out.RuntimeTargets = inspector.CurrentUpstreamTargets()
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(out)
}

// handleSetUpstreamConfig 核心保存逻辑
func handleSetUpstreamConfig(w http.ResponseWriter, r *http.Request) {
	mlog.L().Info("[Debug UpstreamAPI] API Request: Set Config Received") // DEBUG

	var payload struct {
		PluginTag string                   `json:"plugin_tag"`
		Upstreams []UpstreamOverrideConfig `json:"upstreams"`
	}

	if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
		mlog.L().Error("[Debug UpstreamAPI] Invalid request body", zap.Error(err))
		http.Error(w, `{"error": "Invalid request body"}`, http.StatusBadRequest)
		return
	}

	// DEBUG: 打印接收到的数据
	mlog.L().Info("[Debug UpstreamAPI] Payload decoded",
		zap.String("plugin_tag", payload.PluginTag),
		zap.Int("items_count", len(payload.Upstreams)))

	if payload.PluginTag == "" {
		http.Error(w, `{"error": "plugin_tag is required"}`, http.StatusBadRequest)
		return
	}

	for i, u := range payload.Upstreams {
		if u.Tag == "" {
			msg := fmt.Sprintf(`{"error": "Item #%d: tag (name) is required"}`, i+1)
			http.Error(w, msg, http.StatusBadRequest)
			return
		}

		if !u.Enabled {
			continue
		}

		if u.Protocol == "aliapi" {
			if u.AccountID == "" || u.AccessKeyID == "" || u.AccessKeySecret == "" {
				msg := fmt.Sprintf(`{"error": "Item #%d (%s): AliAPI requires account_id, access_key_id, and access_key_secret"}`, i+1, u.Tag)
				http.Error(w, msg, http.StatusBadRequest)
				return
			}
		} else {
			if u.Addr == "" {
				msg := fmt.Sprintf(`{"error": "Item #%d (%s): addr is required for DNS types"}`, i+1, u.Tag)
				http.Error(w, msg, http.StatusBadRequest)
				return
			}
		}
	}

	if upstreamOverrides == nil {
		_ = loadUpstreamOverrides()
	}

	upstreamOverridesLock.Lock()
	if upstreamOverrides == nil {
		upstreamOverrides = make(GlobalUpstreamOverrides)
	}
	oldState := make(GlobalUpstreamOverrides, len(upstreamOverrides))
	rawOldState, _ := json.Marshal(upstreamOverrides)
	_ = json.Unmarshal(rawOldState, &oldState)

	newState := make(GlobalUpstreamOverrides, len(upstreamOverrides))
	rawNewState, _ := json.Marshal(upstreamOverrides)
	_ = json.Unmarshal(rawNewState, &newState)
	newState[payload.PluginTag] = payload.Upstreams

	upstreamOverrides = newState
	saveErr := saveUpstreamOverrides()
	upstreamOverridesLock.Unlock()
	if saveErr != nil {
		upstreamOverridesLock.Lock()
		upstreamOverrides = oldState
		upstreamOverridesLock.Unlock()
		mlog.L().Error("[Debug UpstreamAPI] Save failed", zap.Error(saveErr))
		http.Error(w, `{"error": "Failed to save config file"}`, http.StatusInternalServerError)
		return
	}

	if upstreamAPIHost == nil {
		http.Error(w, `{"error": "Upstream runtime is not initialized"}`, http.StatusInternalServerError)
		return
	}

	p := upstreamAPIHost.GetPlugin(payload.PluginTag)
	if p == nil {
		http.Error(w, `{"error": "Target upstream plugin is not loaded"}`, http.StatusInternalServerError)
		return
	}

	if reloader, ok := p.(upstreamReloader); ok {
		if err := reloader.ReloadFromOverrides(); err != nil {
			upstreamOverridesLock.Lock()
			upstreamOverrides = oldState
			rollbackErr := saveUpstreamOverrides()
			upstreamOverridesLock.Unlock()
			if rollbackErr == nil {
				_ = reloader.ReloadFromOverrides()
			}
			mlog.L().Error("[Debug UpstreamAPI] Live reload failed",
				zap.String("plugin_tag", payload.PluginTag),
				zap.Error(err))
			http.Error(w, `{"error": "Saved, but live reload failed"}`, http.StatusInternalServerError)
			return
		}
		mlog.L().Info("[Debug UpstreamAPI] Live reload success", zap.String("plugin_tag", payload.PluginTag))
	}
	if inspector, ok := p.(upstreamStateInspector); ok {
		mlog.L().Warn("[Debug UpstreamAPI] Active upstream targets after save",
			zap.String("plugin_tag", payload.PluginTag),
			zap.Strings("targets", inspector.CurrentUpstreamTargets()))
	}
	if slot, ok := parseSpecialUpstreamSlot(payload.PluginTag); ok {
		if err := flushDedicatedCaches(slot); err != nil {
			mlog.L().Error("[Debug UpstreamAPI] Cache flush failed after dedicated upstream update",
				zap.String("plugin_tag", payload.PluginTag),
				zap.Int("slot", slot),
				zap.Error(err))
			http.Error(w, `{"error": "Saved and reloaded, but cache flush failed"}`, http.StatusInternalServerError)
			return
		}
		mlog.L().Info("[Debug UpstreamAPI] Dedicated caches flushed after upstream update",
			zap.String("plugin_tag", payload.PluginTag),
			zap.Int("slot", slot))
	}

	w.Header().Set("Content-Type", "application/json")
	fmt.Fprint(w, `{"message": "Upstream configuration saved."}`)
}

func parseSpecialUpstreamSlot(pluginTag string) (int, bool) {
	const prefix = "special_upstream_"
	if !strings.HasPrefix(pluginTag, prefix) {
		return 0, false
	}
	slot, err := strconv.Atoi(strings.TrimPrefix(pluginTag, prefix))
	if err != nil {
		return 0, false
	}
	if slot < 50 || slot > 59 {
		return 0, false
	}
	return slot, true
}

func flushDedicatedCaches(slot int) error {
	cacheTags := []string{
		fmt.Sprintf("cache_special_%d", slot),
		"cache_all",
		"cache_all_noleak",
	}
	for _, tag := range cacheTags {
		p := upstreamAPIHost.GetPlugin(tag)
		if p == nil {
			continue
		}
		flusher, ok := p.(cacheFlusher)
		if !ok {
			return fmt.Errorf("plugin %s does not support cache flush", tag)
		}
		if err := flusher.Flush(); err != nil {
			return fmt.Errorf("flush %s: %w", tag, err)
		}
	}
	return nil
}
