package coremain

import (
	"archive/zip"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"go.uber.org/zap"
)

const (
	githubOwner          = "yyysuo"
	githubRepo           = "mosdns"
	releaseTag           = "v5-ph-srs"
	githubReleaseAPI     = "https://api.github.com/repos/%s/%s/releases/tags/%s"
	githubLatestAPI      = "https://api.github.com/repos/%s/%s/releases/latest"
	githubReleasePage    = "https://github.com/%s/%s/releases/tag/%s"
	githubExpandedAssets = "https://github.com/%s/%s/releases/expanded_assets/%s"
	defaultCacheTTL      = 15 * time.Minute
	httpTimeout          = 120 * time.Second
	userAgent            = "mosdns-update-client"
	stateFileName        = ".mosdns-update-state.json"
	postUpgradeEndpoint  = "http://127.0.0.1:9099/plugins/my_fakeiplist/restartall"
)

var (
	ErrNoUpdateAvailable = errors.New("当前已是最新版本")
	GlobalUpdateManager  = NewUpdateManager()

	assetLinkRegex    = regexp.MustCompile(fmt.Sprintf(`href="(/%s/%s/releases/download/[^" ]+/([^"?]+))"`, githubOwner, githubRepo))
	tagFromURLRegex   = regexp.MustCompile(`/releases/tag/([^"'<>\s]+)`)
	assetHashRegex    = regexp.MustCompile(`sha256:([a-fA-F0-9]{64})`)
	relativeTimeRegex = regexp.MustCompile(`<relative-time[^>]+datetime="([^\"]+)"`)
)

type UpdateStatus struct {
	CurrentVersion   string     `json:"current_version"`
	LatestVersion    string     `json:"latest_version"`
	ReleaseURL       string     `json:"release_url"`
	Architecture     string     `json:"architecture"`
	AssetName        string     `json:"asset_name,omitempty"`
	DownloadURL      string     `json:"download_url,omitempty"`
	AssetSignature   string     `json:"asset_signature,omitempty"`
	CurrentSignature string     `json:"current_signature,omitempty"`
	PublishedAt      *time.Time `json:"published_at,omitempty"`
	CheckedAt        time.Time  `json:"checked_at"`
	CacheExpiresAt   time.Time  `json:"cache_expires_at"`
	UpdateAvailable  bool       `json:"update_available"`
	Cached           bool       `json:"cached"`
	Message          string     `json:"message,omitempty"`
	PendingRestart   bool       `json:"pending_restart,omitempty"`
}

type UpdateActionResponse struct {
	Status          UpdateStatus `json:"status"`
	Installed       bool         `json:"installed"`
	RestartRequired bool         `json:"restart_required"`
	Notes           string       `json:"notes,omitempty"`
}

type updateState struct {
	AssetSignature string    `json:"asset_signature"`
	UpdatedAt      time.Time `json:"updated_at"`
}

type UpdateManager struct {
	mu                    sync.Mutex
	httpClient            *http.Client
	cacheTTL              time.Duration
	lastStatus            *UpdateStatus
	lastChecked           time.Time
	currentVersion        string
	currentAssetSignature string
	pendingSignature      string
	statePath             string
	// 控制“固定 tag”回退行为：默认启用；也可切到 warn-only 或完全禁用
	fixedTagMode fixedTagFallbackMode
}

// 固定 tag 回退模式
type fixedTagFallbackMode int

const (
	fixedTagFallbackEnabled fixedTagFallbackMode = iota
	fixedTagFallbackWarnOnly
	fixedTagFallbackDisabled
)

func (m *UpdateManager) fixedTagModeString() string {
	switch m.fixedTagMode {
	case fixedTagFallbackWarnOnly:
		return "warn-only"
	case fixedTagFallbackDisabled:
		return "disabled"
	default:
		return "enabled"
	}
}

type githubAsset struct {
	Name               string     `json:"name"`
	BrowserDownloadURL string     `json:"browser_download_url"`
	UpdatedAt          *time.Time `json:"updated_at"`
	Sha256             string
}

type releaseInfo struct {
	tagName     string
	publishedAt *time.Time
	assets      []githubAsset
}

func NewUpdateManager() *UpdateManager {
	client := &http.Client{Timeout: httpTimeout}
	mgr := &UpdateManager{
		httpClient:     client,
		cacheTTL:       defaultCacheTTL,
		currentVersion: GetBuildVersion(),
	}
	// 读取环境变量以控制固定 tag 回退逻辑（默认启用）
	// MOSDNS_UPDATE_FIXED_TAG_MODE=enabled|warn-only|disabled
	switch strings.ToLower(os.Getenv("MOSDNS_UPDATE_FIXED_TAG_MODE")) {
	case "warn-only", "warnonly", "warn":
		mgr.fixedTagMode = fixedTagFallbackWarnOnly
	case "disabled", "disable", "off", "none":
		mgr.fixedTagMode = fixedTagFallbackDisabled
	default:
		mgr.fixedTagMode = fixedTagFallbackEnabled
	}
	mgr.initState()
	return mgr
}

func (m *UpdateManager) initState() {
	exe, err := os.Executable()
	if err != nil {
		return
	}
	dir := filepath.Dir(exe)
	m.statePath = filepath.Join(dir, stateFileName)
	m.loadState()
}

func (m *UpdateManager) loadState() {
	m.mu.Lock()
	path := m.statePath
	m.mu.Unlock()
	if path == "" {
		return
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return
	}
	var st updateState
	if err := json.Unmarshal(data, &st); err != nil {
		return
	}
	if st.AssetSignature != "" {
		m.mu.Lock()
		m.currentAssetSignature = st.AssetSignature
		m.mu.Unlock()
	}
}

func (m *UpdateManager) saveState(signature string) {
	if signature == "" {
		return
	}
	m.mu.Lock()
	path := m.statePath
	m.mu.Unlock()
	if path == "" {
		return
	}
	payload := updateState{AssetSignature: signature, UpdatedAt: time.Now()}
	data, err := json.Marshal(payload)
	if err != nil {
		m.logWarn("save update state marshal failed", err)
		return
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		m.logWarn("save update state failed", err)
	}
}

func (m *UpdateManager) SetCurrentVersion(version string) {
	if version == "" {
		return
	}
	m.mu.Lock()
	m.currentVersion = version
	m.pendingSignature = ""
	if m.lastStatus != nil {
		m.lastStatus.CurrentVersion = version
		m.lastStatus.UpdateAvailable = m.updateAvailableLocked(m.lastStatus.LatestVersion, m.lastStatus.AssetSignature)
	}
	m.mu.Unlock()
}

func (m *UpdateManager) logger() *zap.Logger {
	if lg := mlog.L(); lg != nil {
		return lg
	}
	return nil
}

func (m *UpdateManager) logWarn(msg string, err error, fields ...zap.Field) {
	if lg := m.logger(); lg != nil {
		lg.Warn(msg, append(fields, zap.Error(err))...)
	}
}

func (m *UpdateManager) CheckForUpdate(ctx context.Context, force bool) (UpdateStatus, error) {
	now := time.Now()

	m.mu.Lock()
	if !force && m.lastStatus != nil && now.Sub(m.lastChecked) < m.cacheTTL {
		cached := *m.lastStatus
		cached.CheckedAt = now
		cached.CacheExpiresAt = m.lastChecked.Add(m.cacheTTL)
		cached.Cached = true
		m.mu.Unlock()
		return cached, nil
	}
	m.mu.Unlock()

	rel, err := m.fetchReleaseInfo(ctx)
	if err != nil {
		return UpdateStatus{}, err
	}

	tag := rel.tagName
	if tag == "" {
		tag = releaseTag
	}
	status := UpdateStatus{
		CurrentVersion:   m.currentVersion,
		LatestVersion:    tag,
		ReleaseURL:       fmt.Sprintf(githubReleasePage, githubOwner, githubRepo, tag),
		Architecture:     fmt.Sprintf("%s/%s", runtime.GOOS, runtime.GOARCH),
		PublishedAt:      rel.publishedAt,
		CheckedAt:        now,
		CacheExpiresAt:   now.Add(m.cacheTTL),
		Cached:           false,
		CurrentSignature: m.currentAssetSignature,
	}

	if asset := selectAsset(rel.assets); asset != nil {
		status.AssetName = asset.Name
		status.DownloadURL = asset.BrowserDownloadURL
		status.AssetSignature = buildAssetSignature(*asset)
	} else {
		status.Message = fmt.Sprintf("未找到适用于 %s/%s 的安装包", runtime.GOOS, runtime.GOARCH)
	}

	status.UpdateAvailable = m.isUpdateNeeded(status.LatestVersion, status.AssetSignature)

	m.mu.Lock()
	m.lastStatus = &status
	m.lastChecked = now
	m.mu.Unlock()

	return status, nil
}

func (m *UpdateManager) PerformUpdate(ctx context.Context, force bool) (UpdateActionResponse, error) {
	status, err := m.CheckForUpdate(ctx, force)
	if err != nil {
		return UpdateActionResponse{}, err
	}

	if !status.UpdateAvailable && !force {
		return UpdateActionResponse{Status: status}, ErrNoUpdateAvailable
	}

	if status.DownloadURL == "" {
		note := status.Message
		if note == "" {
			note = "无法定位下载地址"
		}
		status.Message = note
		return UpdateActionResponse{Status: status}, errors.New(note)
	}

	assetFile, err := m.downloadAsset(ctx, status.DownloadURL)
	if err != nil {
		status.Message = fmt.Sprintf("下载失败: %v", err)
		return UpdateActionResponse{Status: status}, err
	}
	defer os.Remove(assetFile)

	if status.AssetSignature == "" {
		if sig, hashErr := fileSHA256(assetFile); hashErr == nil {
			status.AssetSignature = fmt.Sprintf("%s:%s", status.AssetName, sig)
		}
	}

	extractedBinary, mode, err := extractBinaryFromZip(assetFile)
	if err != nil {
		status.Message = fmt.Sprintf("解压失败: %v", err)
		return UpdateActionResponse{Status: status}, err
	}
	defer os.Remove(extractedBinary)

	action := UpdateActionResponse{Status: status}
	exePath, err := os.Executable()
	if err != nil {
		action.Notes = fmt.Sprintf("获取当前可执行文件失败: %v", err)
		return action, err
	}

	if runtime.GOOS == "windows" {
		target := exePath + ".new"
		if err := copyFile(extractedBinary, target, mode); err != nil {
			action.Notes = fmt.Sprintf("写入新文件失败: %v", err)
			return action, err
		}
		action.Notes = fmt.Sprintf("已将新版写入 %s，请手动替换 mosdns.exe", target)
		action.RestartRequired = true
		status.PendingRestart = true
		m.mu.Lock()
		m.pendingSignature = status.AssetSignature
		m.mu.Unlock()
		action.Status = status
		return action, nil
	}

	if err := installBinary(exePath, extractedBinary, mode); err != nil {
		action.Notes = err.Error()
		return action, err
	}

	action.Installed = true
	action.RestartRequired = true
	action.Notes = fmt.Sprintf("新版本已写入 %s，重启 Mosdns 后生效。", exePath)

	status.PendingRestart = true
	status.Message = action.Notes
	action.Status = status

	m.recordInstalled(status.AssetSignature)
	if err := m.triggerPostUpgradeHook(ctx); err != nil {
		m.logWarn("post-upgrade restart hook failed", err, zap.String("endpoint", postUpgradeEndpoint))
	} else {
		action.Notes += " 已请求刷新 my_fakeiplist。"
		status.Message = action.Notes
	}

	return action, nil
}

func (m *UpdateManager) recordInstalled(signature string) {
	if signature == "" {
		signature = fmt.Sprintf("manual-%d", time.Now().Unix())
	}
	m.mu.Lock()
	m.currentAssetSignature = signature
	m.pendingSignature = ""
	m.mu.Unlock()
	m.saveState(signature)
}

func (m *UpdateManager) triggerPostUpgradeHook(ctx context.Context) error {
	endpoint := postUpgradeEndpoint
	if endpoint == "" {
		return nil
	}
	if ctx == nil {
		ctx = context.Background()
	}
	requestCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(requestCtx, http.MethodGet, endpoint, nil)
	if err != nil {
		return err
	}
	if host, _, err := net.SplitHostPort(req.URL.Host); err == nil && (host == "localhost" || host == "127.0.0.1") {
		req.Host = req.URL.Host
	}
	resp, err := m.httpClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	io.Copy(io.Discard, resp.Body)
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("hook returned %s", resp.Status)
	}
	return nil
}

func (m *UpdateManager) isUpdateNeeded(latest, signature string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.updateAvailableLocked(latest, signature)
}

func (m *UpdateManager) updateAvailableLocked(latest, signature string) bool {
	if signature != "" {
		if signature == m.currentAssetSignature {
			return false
		}
		if signature == m.pendingSignature {
			return false
		}
		return true
	}

	latest = strings.TrimSpace(latest)
	if latest == "" {
		return false
	}
	current := strings.TrimSpace(m.currentVersion)
	if current == "" {
		return true
	}
	return latest != current
}

// fetchReleaseInfo tries to get the latest release info first (releases/latest),
// then falls back to the legacy fixed-tag mode (v5-ph-srs) for backward compatibility.
func (m *UpdateManager) fetchReleaseInfo(ctx context.Context) (releaseInfo, error) {
	if info, err := m.fetchLatestReleaseInfo(ctx); err == nil {
		return info, nil
	} else {
		// 记录 latest 获取失败
		m.logWarn("latest release fetch failed", err, zap.String("fallback_mode", m.fixedTagModeString()))

		// 根据回退模式处理
		switch m.fixedTagMode {
		case fixedTagFallbackEnabled:
			m.logWarn("fallback to fixed tag", errors.New("using fixed tag fallback"), zap.String("fallback_mode", m.fixedTagModeString()))
			if info2, err2 := m.fetchReleaseInfoAPI(ctx); err2 == nil {
				return info2, nil
			}
			return m.fetchReleaseInfoHTML(ctx)
		case fixedTagFallbackWarnOnly:
			// 仅记录日志，不再实际回退，便于观测是否仍有老客户端依赖固定 tag
			m.logWarn("fixed tag fallback suppressed (warn-only)", errors.New("suppressed fixed tag fallback"), zap.String("fallback_mode", m.fixedTagModeString()))
			return releaseInfo{}, fmt.Errorf("获取最新版本失败（fixed-tag 回退 warn-only 已生效）: %v", err)
		case fixedTagFallbackDisabled:
			m.logWarn("fixed tag fallback disabled", errors.New("disabled fixed tag fallback"), zap.String("fallback_mode", m.fixedTagModeString()))
			return releaseInfo{}, fmt.Errorf("获取最新版本失败（fixed-tag 回退已禁用）: %v", err)
		default:
			return releaseInfo{}, err
		}
	}
}

func (m *UpdateManager) fetchLatestReleaseInfo(ctx context.Context) (releaseInfo, error) {
	if info, err := m.fetchLatestReleaseInfoAPI(ctx); err == nil {
		return info, nil
	}
	return m.fetchLatestReleaseInfoHTML(ctx)
}

func (m *UpdateManager) fetchReleaseInfoAPI(ctx context.Context) (releaseInfo, error) {
	url := fmt.Sprintf(githubReleaseAPI, githubOwner, githubRepo, releaseTag)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return releaseInfo{}, err
	}
	req.Header.Set("Accept", "application/vnd.github+json")
	req.Header.Set("User-Agent", userAgent)
	if token := os.Getenv("MOSDNS_GITHUB_TOKEN"); token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	} else if token := os.Getenv("GITHUB_TOKEN"); token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}

	resp, err := m.httpClient.Do(req)
	if err != nil {
		return releaseInfo{}, err
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusForbidden {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4<<10))
		return releaseInfo{}, fmt.Errorf("GitHub API 访问受限: %s (%s)", resp.Status, strings.TrimSpace(string(body)))
	}
	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4<<10))
		return releaseInfo{}, fmt.Errorf("GitHub API 请求失败: %s (%s)", resp.Status, strings.TrimSpace(string(body)))
	}

	var payload struct {
		TagName     string        `json:"tag_name"`
		PublishedAt *time.Time    `json:"published_at"`
		Assets      []githubAsset `json:"assets"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&payload); err != nil {
		return releaseInfo{}, err
	}

	tag := payload.TagName
	if tag == "" {
		tag = releaseTag
	}
	return releaseInfo{tagName: tag, publishedAt: payload.PublishedAt, assets: payload.Assets}, nil
}

func (m *UpdateManager) fetchLatestReleaseInfoAPI(ctx context.Context) (releaseInfo, error) {
	url := fmt.Sprintf(githubLatestAPI, githubOwner, githubRepo)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return releaseInfo{}, err
	}
	req.Header.Set("Accept", "application/vnd.github+json")
	req.Header.Set("User-Agent", userAgent)
	if token := os.Getenv("MOSDNS_GITHUB_TOKEN"); token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	} else if token := os.Getenv("GITHUB_TOKEN"); token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}

	resp, err := m.httpClient.Do(req)
	if err != nil {
		return releaseInfo{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusForbidden {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4<<10))
		return releaseInfo{}, fmt.Errorf("GitHub API 访问受限: %s (%s)", resp.Status, strings.TrimSpace(string(body)))
	}
	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4<<10))
		return releaseInfo{}, fmt.Errorf("GitHub API 请求失败: %s (%s)", resp.Status, strings.TrimSpace(string(body)))
	}

	var payload struct {
		TagName     string        `json:"tag_name"`
		PublishedAt *time.Time    `json:"published_at"`
		Assets      []githubAsset `json:"assets"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&payload); err != nil {
		return releaseInfo{}, err
	}
	if payload.TagName == "" {
		return releaseInfo{}, errors.New("API 未返回 tag 名称")
	}
	return releaseInfo{publishedAt: payload.PublishedAt, assets: payload.Assets}, nil
}

func (m *UpdateManager) fetchLatestReleaseInfoHTML(ctx context.Context) (releaseInfo, error) {
	latestURL := fmt.Sprintf("https://github.com/%s/%s/releases/latest", githubOwner, githubRepo)
	body, err := m.fetchHTML(ctx, latestURL)
	if err != nil {
		return releaseInfo{}, err
	}
	tag := ""
	if match := tagFromURLRegex.FindStringSubmatch(body); len(match) == 2 {
		tag = match[1]
	}
	if tag == "" {
		return releaseInfo{}, errors.New("无法从 latest 页面解析 tag")
	}

	// 发布时间（可选）
	var publishedAt *time.Time
	if match := relativeTimeRegex.FindStringSubmatch(body); len(match) == 2 {
		if t, err := time.Parse(time.RFC3339, match[1]); err == nil {
			publishedAt = &t
		}
	}

	assetsHTML, err := m.fetchHTML(ctx, fmt.Sprintf(githubExpandedAssets, githubOwner, githubRepo, tag))
	if err != nil {
		return releaseInfo{}, err
	}
	assets := parseAssetsFromHTML(assetsHTML)
	if len(assets) == 0 {
		return releaseInfo{}, errors.New("未在最新发布页面解析到资产")
	}
	return releaseInfo{tagName: tag, publishedAt: publishedAt, assets: assets}, nil
}

func (m *UpdateManager) fetchReleaseInfoHTML(ctx context.Context) (releaseInfo, error) {
	assetsURL := fmt.Sprintf(githubExpandedAssets, githubOwner, githubRepo, releaseTag)
	body, err := m.fetchHTML(ctx, assetsURL)
	if err != nil {
		return releaseInfo{}, err
	}

	assets := parseAssetsFromHTML(body)
	if len(assets) == 0 {
		return releaseInfo{}, errors.New("未在发布页面解析到资产")
	}

	var publishedAt *time.Time
	if match := relativeTimeRegex.FindStringSubmatch(body); len(match) == 2 {
		if t, err := time.Parse(time.RFC3339, match[1]); err == nil {
			publishedAt = &t
		}
	}

	return releaseInfo{publishedAt: publishedAt, assets: assets}, nil
}

func (m *UpdateManager) fetchHTML(ctx context.Context, url string) (string, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("User-Agent", userAgent)
	resp, err := m.httpClient.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4<<10))
		return "", fmt.Errorf("请求 %s 失败: %s (%s)", url, resp.Status, strings.TrimSpace(string(body)))
	}
	bodyBytes, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", err
	}
	return string(bodyBytes), nil
}

func selectAsset(assets []githubAsset) *githubAsset {
	var candidates []string
	switch runtime.GOOS {
	case "linux":
		switch runtime.GOARCH {
		case "amd64":
			candidates = []string{"mosdns-linux-amd64-v3.zip", "mosdns-linux-amd64.zip"}
		case "arm64":
			candidates = []string{"mosdns-linux-arm64.zip"}
		case "arm":
			candidates = []string{"mosdns-linux-arm-7.zip", "mosdns-linux-arm-6.zip", "mosdns-linux-arm-5.zip"}
		case "mips", "mips64", "mips64le", "mipsle":
			candidates = append(candidates, fmt.Sprintf("mosdns-linux-%s.zip", runtime.GOARCH))
		}
	case "darwin":
		candidates = append(candidates, fmt.Sprintf("mosdns-darwin-%s.zip", runtime.GOARCH))
	case "windows":
		if runtime.GOARCH == "amd64" {
			candidates = []string{"mosdns-windows-amd64-v3.zip", "mosdns-windows-amd64.zip"}
		} else if runtime.GOARCH == "arm64" {
			candidates = []string{"mosdns-windows-arm64.zip"}
		}
	}

	for _, name := range candidates {
		for idx := range assets {
			if assets[idx].Name == name {
				return &assets[idx]
			}
		}
	}
	if len(assets) > 0 {
		return &assets[0]
	}
	return nil
}

func buildAssetSignature(asset githubAsset) string {
	if asset.Sha256 != "" {
		return fmt.Sprintf("%s:%s", asset.Name, strings.ToLower(asset.Sha256))
	}
	if asset.UpdatedAt != nil {
		return fmt.Sprintf("%s:%d", asset.Name, asset.UpdatedAt.Unix())
	}
	if asset.BrowserDownloadURL != "" {
		return asset.BrowserDownloadURL
	}
	return ""
}

func parseAssetsFromHTML(html string) []githubAsset {
	items := strings.Split(html, "<li")
	seen := make(map[string]struct{})
	result := make([]githubAsset, 0, len(items))
	for _, raw := range items {
		chunk := "<li" + raw
		if !strings.Contains(chunk, "/releases/download/") {
			continue
		}
		linkMatch := assetLinkRegex.FindStringSubmatch(chunk)
		if len(linkMatch) != 3 {
			continue
		}
		urlPart := linkMatch[1]
		name := linkMatch[2]
		if _, ok := seen[name]; ok {
			continue
		}
		seen[name] = struct{}{}
		sha := ""
		if hashMatch := assetHashRegex.FindStringSubmatch(chunk); len(hashMatch) == 2 {
			sha = strings.ToLower(hashMatch[1])
		}
		var updatedAt *time.Time
		if tm := relativeTimeRegex.FindStringSubmatch(chunk); len(tm) == 2 {
			if t, err := time.Parse(time.RFC3339, tm[1]); err == nil {
				updatedAt = &t
			}
		}
		result = append(result, githubAsset{
			Name:               name,
			BrowserDownloadURL: "https://github.com" + urlPart,
			Sha256:             sha,
			UpdatedAt:          updatedAt,
		})
	}
	return result
}

func (m *UpdateManager) downloadAsset(ctx context.Context, url string) (string, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("User-Agent", userAgent)
	resp, err := m.httpClient.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("下载失败: %s", resp.Status)
	}

	tmpFile, err := os.CreateTemp("", "mosdns-update-*.zip")
	if err != nil {
		return "", err
	}
	defer tmpFile.Close()

	if _, err := io.Copy(tmpFile, resp.Body); err != nil {
		return "", err
	}

	return tmpFile.Name(), nil
}

func extractBinaryFromZip(zipPath string) (string, os.FileMode, error) {
	file, err := os.Open(zipPath)
	if err != nil {
		return "", 0, err
	}
	defer file.Close()

	info, err := file.Stat()
	if err != nil {
		return "", 0, err
	}

	zr, err := zip.NewReader(file, info.Size())
	if err != nil {
		return "", 0, err
	}

	var target *zip.File
	for _, f := range zr.File {
		base := filepath.Base(f.Name)
		if base == "mosdns" || base == "mosdns.exe" {
			target = f
			break
		}
	}

	if target == nil {
		return "", 0, errors.New("压缩包中未找到 mosdns 可执行文件")
	}

	rc, err := target.Open()
	if err != nil {
		return "", 0, err
	}
	defer rc.Close()

	tmpFile, err := os.CreateTemp("", "mosdns-binary-*")
	if err != nil {
		return "", 0, err
	}

	if _, err := io.Copy(tmpFile, rc); err != nil {
		tmpFile.Close()
		os.Remove(tmpFile.Name())
		return "", 0, err
	}
	tmpFile.Close()

	mode := target.Mode()
	if mode == 0 {
		mode = 0o755
	} else {
		mode |= 0o111
	}
	if err := os.Chmod(tmpFile.Name(), mode); err != nil {
		os.Remove(tmpFile.Name())
		return "", 0, err
	}

	return tmpFile.Name(), mode, nil
}

func installBinary(exePath, newBinary string, mode os.FileMode) error {
	dir := filepath.Dir(exePath)
	tempDest, err := os.CreateTemp(dir, "mosdns-new-*")
	if err != nil {
		return err
	}
	tempDestPath := tempDest.Name()
	tempDest.Close()

	if err := copyFile(newBinary, tempDestPath, mode); err != nil {
		os.Remove(tempDestPath)
		return err
	}

	if err := os.Rename(tempDestPath, exePath); err != nil {
		os.Remove(tempDestPath)
		return err
	}

	return os.Chmod(exePath, mode)
}

func copyFile(src, dst string, mode os.FileMode) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()

	out, err := os.OpenFile(dst, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, mode)
	if err != nil {
		return err
	}

	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return err
	}

	return out.Close()
}

func fileSHA256(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}
