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
    "syscall"
    "strings"
    "sync"
    "time"
    stdlog "log"

    "github.com/IrineSistiana/mosdns/v5/mlog"
    "go.uber.org/zap"
    "runtime/debug"
    xcpu "golang.org/x/sys/cpu"
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
    // 默认的重启端点；可由环境变量 MOSDNS_RESTART_ENDPOINT 覆盖。
    postUpgradeEndpoint  = "http://127.0.0.1:9099/api/v1/system/restart"
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
    // 新增：在 amd64 上提示可手动切到 v3 优化构建
    AMD64V3Capable   bool       `json:"amd64_v3_capable,omitempty"`
    CurrentIsV3      bool       `json:"current_is_v3,omitempty"`
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
        m.logWarn("fetch latest release failed", err)
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

    // 提供 v3 能力提示用的标记（仅在 amd64 + linux/windows 下有意义）
    if runtime.GOARCH == "amd64" && (runtime.GOOS == "linux" || runtime.GOOS == "windows") {
        status.AMD64V3Capable = cpuSupportsAMD64V3()
        status.CurrentIsV3 = binaryIsAMD64V3Plus()
    }

    // 记录一次探测信息，便于排查未显示 v3 提示的情况
    if lg := m.logger(); lg != nil {
        goamd64 := readGOAMD64()
        cpuModel := cpuModelName()
        lg.Info("update status",
            zap.String("arch", status.Architecture),
            zap.String("current", status.CurrentVersion),
            zap.String("latest", status.LatestVersion),
            zap.Bool("update_available", status.UpdateAvailable),
            zap.Bool("amd64_v3_capable", status.AMD64V3Capable),
            zap.Bool("current_is_v3", status.CurrentIsV3),
            zap.String("goamd64", goamd64),
            zap.String("cpu_model", cpuModel),
            zap.Bool("cpu_avx2", runtime.GOARCH == "amd64" && xcpu.X86.HasAVX2),
            zap.Bool("cpu_bmi1", runtime.GOARCH == "amd64" && xcpu.X86.HasBMI1),
            zap.Bool("cpu_bmi2", runtime.GOARCH == "amd64" && xcpu.X86.HasBMI2),
            zap.Bool("cpu_fma", runtime.GOARCH == "amd64" && xcpu.X86.HasFMA),
        )
        // 兼容 stdout/stderr 场景的简洁日志，便于快速排查（与 requery 插件日志一致风格）
        stdlog.Printf("[update] arch=%s current=%s latest=%s update=%t goamd64=%s v3_capable=%t current_is_v3=%t cpu='%s' avx2=%t bmi1=%t bmi2=%t fma=%t",
            status.Architecture, status.CurrentVersion, status.LatestVersion, status.UpdateAvailable, goamd64,
            status.AMD64V3Capable, status.CurrentIsV3, cpuModel,
            runtime.GOARCH == "amd64" && xcpu.X86.HasAVX2,
            runtime.GOARCH == "amd64" && xcpu.X86.HasBMI1,
            runtime.GOARCH == "amd64" && xcpu.X86.HasBMI2,
            runtime.GOARCH == "amd64" && xcpu.X86.HasFMA,
        )
        // 中文直观概览
        stdlog.Printf("[update] 概览：当前版本=%s 最新版本=%s 架构=%s CPU=%s CPU支持v3=%s 当前为v3构建=%s GOAMD64=%s 需要更新=%s",
            status.CurrentVersion,
            status.LatestVersion,
            status.Architecture,
            cpuModel,
            yesNoCN(status.AMD64V3Capable),
            yesNoCN(status.CurrentIsV3),
            nonEmpty(goamd64, "未知"),
            yesNoCN(status.UpdateAvailable),
        )
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

func (m *UpdateManager) PerformUpdate(ctx context.Context, force bool, preferV3 bool) (UpdateActionResponse, error) {
    status, err := m.CheckForUpdate(ctx, force)
    if err != nil {
        return UpdateActionResponse{}, err
    }

    if !status.UpdateAvailable && !force && !preferV3 {
        return UpdateActionResponse{Status: status}, ErrNoUpdateAvailable
    }

    // 若用户显式请求 v3，并且平台/CPU 支持，尝试切换到 v3 资产
    if preferV3 && runtime.GOARCH == "amd64" && (runtime.GOOS == "linux" || runtime.GOOS == "windows") && cpuSupportsAMD64V3() {
        if lg := m.logger(); lg != nil { lg.Info("prefer v3 requested; trying to switch asset") }
        stdlog.Printf("[update] 已收到手动切换为 v3 的请求：如果存在 v3 资产将优先选择该包进行更新（不改变版本号，仅切换构建）。")
        if rel, err := m.fetchReleaseInfo(ctx); err == nil {
            if v3 := findV3Asset(rel.assets); v3 != nil {
                status.AssetName = v3.Name
                status.DownloadURL = v3.BrowserDownloadURL
                status.AssetSignature = buildAssetSignature(*v3)
                status.UpdateAvailable = m.isUpdateNeeded(status.LatestVersion, status.AssetSignature)
                if !status.UpdateAvailable {
                    // 即使版本相同，只要资产不同也视为需要更新（切换构建）
                    status.UpdateAvailable = status.AssetSignature != m.currentAssetSignature
                }
            } else {
                status.Message = "未找到 v3 优化构建包"
                return UpdateActionResponse{Status: status}, errors.New(status.Message)
            }
        }
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
		// 短文案：避免在 UI 中出现过长路径
		action.Notes = "更新已下载，已生成 mosdns.exe.new，请手动替换并重启。"
		action.RestartRequired = true
		status.PendingRestart = true
		m.mu.Lock()
		m.pendingSignature = status.AssetSignature
		m.mu.Unlock()
		status.Message = action.Notes
		action.Status = status
		return action, nil
	}

	if err := installBinary(exePath, extractedBinary, mode); err != nil {
		action.Notes = err.Error()
		return action, err
	}

	action.Installed = true
	action.RestartRequired = true
	// 短文案：由前端呈现“自重启中”状态
	action.Notes = "更新已安装，正在自重启…"

	status.PendingRestart = true
	status.Message = action.Notes
	action.Status = status

	m.recordInstalled(status.AssetSignature)
	if err := m.triggerPostUpgradeHook(ctx); err != nil {
		m.logWarn("post-upgrade restart hook failed", err, zap.String("endpoint", postUpgradeEndpoint))
		// 简短降级提示
		action.Notes = "更新已安装，请手动重启。"
		status.Message = action.Notes
	} else {
		// 保持简短
		action.Notes = "更新已安装，正在自重启…"
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
    // 优先使用环境变量覆盖（便于自定义 API 端口/路径）
    endpoint := strings.TrimSpace(os.Getenv("MOSDNS_RESTART_ENDPOINT"))
    if endpoint == "" {
        endpoint = postUpgradeEndpoint
    }

    // 先尝试通过 HTTP 端点触发重启
    if endpoint != "" {
        if ctx == nil {
            ctx = context.Background()
        }
        requestCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
        defer cancel()
        payload := strings.NewReader(`{"delay_ms":500}`)
        req, err := http.NewRequestWithContext(requestCtx, http.MethodPost, endpoint, payload)
        if err == nil {
            req.Header.Set("Content-Type", "application/json")
            if host, _, err := net.SplitHostPort(req.URL.Host); err == nil && (host == "localhost" || host == "127.0.0.1") {
                req.Host = req.URL.Host
            }
            if resp, err := m.httpClient.Do(req); err == nil {
                defer resp.Body.Close()
                io.Copy(io.Discard, resp.Body)
                if resp.StatusCode >= 200 && resp.StatusCode < 300 {
                    return nil
                }
                // 记录非 2xx 的响应，继续走本地回退
                m.logWarn("self-restart hook returned non-2xx", fmt.Errorf("HTTP %s", resp.Status), zap.String("endpoint", endpoint))
            } else {
                // 记录 HTTP 失败，继续走本地回退
                m.logWarn("self-restart hook request failed", err, zap.String("endpoint", endpoint))
            }
        } else {
            // 构造请求失败，继续走本地回退
            m.logWarn("self-restart hook request build failed", err, zap.String("endpoint", endpoint))
        }
    }

    // 本地回退：直接在当前进程内发起自重启（仅非 Windows）
    if runtime.GOOS != "windows" {
        exe, err := os.Executable()
        if err != nil {
            return err
        }
        args := append([]string{exe}, os.Args[1:]...)
        env := os.Environ()
        go func() {
            // 给上层 UI 一点时间处理 pending 状态
            time.Sleep(500 * time.Millisecond)
            _ = syscall.Exec(exe, args, env)
        }()
        return nil
    }
    return errors.New("self-restart is not supported on Windows")
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

func (m *UpdateManager) fetchReleaseInfo(ctx context.Context) (releaseInfo, error) {
    // 固定 tag 回退路径已移除：仅使用 releases/latest
    info, err := m.fetchLatestReleaseInfo(ctx)
    if err != nil {
        return releaseInfo{}, fmt.Errorf("获取最新版本失败: %v", err)
    }
    return info, nil
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
    // 关键修复：确保通过 API 路径返回真实 tag，用于 UI 展示与 ReleaseURL 构造
    return releaseInfo{tagName: payload.TagName, publishedAt: payload.PublishedAt, assets: payload.Assets}, nil
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
            if binaryIsAMD64V3Plus() {
                candidates = []string{"mosdns-linux-amd64-v3.zip", "mosdns-linux-amd64.zip"}
            } else {
                candidates = []string{"mosdns-linux-amd64.zip"}
            }
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
            if binaryIsAMD64V3Plus() {
                candidates = []string{"mosdns-windows-amd64-v3.zip", "mosdns-windows-amd64.zip"}
            } else {
                candidates = []string{"mosdns-windows-amd64.zip"}
            }
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

// binaryIsAMD64V3Plus 通过读取当前二进制的构建信息，判断是否为
// amd64 v3（或更高，如 v4）构建。读取失败则返回 false（保守选择传统包）。
func binaryIsAMD64V3Plus() bool {
    if runtime.GOARCH != "amd64" {
        return false
    }
    if bi, ok := debug.ReadBuildInfo(); ok {
        for _, s := range bi.Settings {
            if s.Key == "GOAMD64" {
                v := strings.ToLower(strings.TrimSpace(s.Value))
                return v == "v3" || v == "v4"
            }
        }
    }
    // 未能读到 GOAMD64，保守处理：按非 v3 处理
    return false
}

// cpuSupportsAMD64V3 检测 CPU 是否支持 v3 关键指令（AVX2、BMI1、BMI2、FMA）。
// 注意：这不是当前二进制的构建档位，而是硬件能力探测。
func cpuSupportsAMD64V3() bool {
    if runtime.GOARCH != "amd64" {
        return false
    }
    // 依据 Go 官方 GOAMD64 v3 的近似集合进行判断（保守取交集）。
    return xcpu.X86.HasAVX2 && xcpu.X86.HasBMI1 && xcpu.X86.HasBMI2 && xcpu.X86.HasFMA
}

// readGOAMD64 返回当前二进制的 GOAMD64 构建档位（v1/v2/v3/v4），未知则返回空串。
func readGOAMD64() string {
    if bi, ok := debug.ReadBuildInfo(); ok {
        for _, s := range bi.Settings {
            if s.Key == "GOAMD64" {
                return strings.ToLower(strings.TrimSpace(s.Value))
            }
        }
    }
    return ""
}

// cpuModelName 在 Linux 上尝试读取 /proc/cpuinfo 的 model name；其他平台返回空串。
func cpuModelName() string {
    if runtime.GOOS != "linux" {
        return ""
    }
    data, err := os.ReadFile("/proc/cpuinfo")
    if err != nil {
        return ""
    }
    lines := strings.Split(string(data), "\n")
    for _, ln := range lines {
        if strings.HasPrefix(strings.ToLower(ln), "model name") {
            if idx := strings.Index(ln, ":"); idx != -1 {
                return strings.TrimSpace(ln[idx+1:])
            }
        }
    }
    return ""
}

// yesNoCN 将布尔值转换为“是/否”。
func yesNoCN(b bool) string {
    if b {
        return "是"
    }
    return "否"
}

// nonEmpty 返回 s，否则返回 fallback。
func nonEmpty(s, fallback string) string {
    if strings.TrimSpace(s) == "" {
        return fallback
    }
    return s
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

// findV3Asset 在支持 v3 的 amd64 平台上，返回 v3 优化包。
func findV3Asset(assets []githubAsset) *githubAsset {
    if runtime.GOARCH != "amd64" {
        return nil
    }
    want := ""
    switch runtime.GOOS {
    case "linux":
        want = "mosdns-linux-amd64-v3.zip"
    case "windows":
        want = "mosdns-windows-amd64-v3.zip"
    default:
        return nil
    }
    for i := range assets {
        if assets[i].Name == want {
            return &assets[i]
        }
    }
    return nil
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
