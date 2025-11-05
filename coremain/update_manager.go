package coremain

import (
	"archive/zip"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"
)

const (
	githubOwner             = "yyysuo"
	githubRepo              = "mosdns"
	githubLatestReleasePath = "https://api.github.com/repos/%s/%s/releases/latest"
	githubReleasePage       = "https://github.com/%s/%s/releases/tag/%s"
	defaultCacheTTL         = 15 * time.Minute
	httpTimeout             = 120 * time.Second
	userAgent               = "mosdns-update-client"
)

var (
	// ErrNoUpdateAvailable 表示当前已是最新版本。
	ErrNoUpdateAvailable = errors.New("当前已是最新版本")

	// GlobalUpdateManager 为整个程序共享的更新管理器实例。
	GlobalUpdateManager = NewUpdateManager()
)

type UpdateStatus struct {
	CurrentVersion  string     `json:"current_version"`
	LatestVersion   string     `json:"latest_version"`
	ReleaseURL      string     `json:"release_url"`
	Architecture    string     `json:"architecture"`
	AssetName       string     `json:"asset_name,omitempty"`
	DownloadURL     string     `json:"download_url,omitempty"`
	PublishedAt     *time.Time `json:"published_at,omitempty"`
	CheckedAt       time.Time  `json:"checked_at"`
	CacheExpiresAt  time.Time  `json:"cache_expires_at"`
	UpdateAvailable bool       `json:"update_available"`
	Cached          bool       `json:"cached"`
	Message         string     `json:"message,omitempty"`
	PendingRestart  bool       `json:"pending_restart,omitempty"`
}

type UpdateActionResponse struct {
	Status          UpdateStatus `json:"status"`
	Installed       bool         `json:"installed"`
	RestartRequired bool         `json:"restart_required"`
	BackupPath      string       `json:"backup_path,omitempty"`
	Notes           string       `json:"notes,omitempty"`
}

type UpdateManager struct {
	mu             sync.Mutex
	httpClient     *http.Client
	cacheTTL       time.Duration
	lastStatus     *UpdateStatus
	lastChecked    time.Time
	currentVersion string
	pendingVersion string
}

type githubRelease struct {
	TagName     string        `json:"tag_name"`
	Name        string        `json:"name"`
	PublishedAt *time.Time    `json:"published_at"`
	Assets      []githubAsset `json:"assets"`
}

type githubAsset struct {
	Name               string `json:"name"`
	BrowserDownloadURL string `json:"browser_download_url"`
}

func NewUpdateManager() *UpdateManager {
	client := &http.Client{Timeout: httpTimeout}
	return &UpdateManager{
		httpClient:     client,
		cacheTTL:       defaultCacheTTL,
		currentVersion: GetBuildVersion(),
	}
}

func (m *UpdateManager) SetCurrentVersion(version string) {
	if version == "" {
		return
	}
	m.mu.Lock()
	m.currentVersion = version
	m.pendingVersion = ""
	if m.lastStatus != nil {
		m.lastStatus.CurrentVersion = version
		m.lastStatus.UpdateAvailable = m.updateAvailableLocked(m.lastStatus.LatestVersion)
	}
	m.mu.Unlock()
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

	status, err := m.fetchLatest(ctx)
	if err != nil {
		return UpdateStatus{}, err
	}

	status.CheckedAt = now
	status.CacheExpiresAt = now.Add(m.cacheTTL)
	status.Cached = false

	m.mu.Lock()
	status.CurrentVersion = m.currentVersion
	status.UpdateAvailable = m.updateAvailableLocked(status.LatestVersion)
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
			note = fmt.Sprintf("未找到 %s/%s 对应的安装包", runtime.GOOS, runtime.GOARCH)
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
		action.Notes = fmt.Sprintf("已将新版写入 %s，请停止服务后手动替换 mosdns.exe", target)
		action.RestartRequired = true
		status.PendingRestart = true
		status.UpdateAvailable = false
		status.Message = action.Notes
		action.Status = status
		m.mu.Lock()
		m.pendingVersion = status.LatestVersion
		m.mu.Unlock()
		return action, nil
	}

	backupPath, err := replaceBinary(exePath, extractedBinary, mode)
	if err != nil {
		action.Notes = err.Error()
		return action, err
	}

	action.BackupPath = backupPath
	action.Installed = true
	action.RestartRequired = true
	action.Notes = fmt.Sprintf("新版本已写入 %s，重启 Mosdns 后生效。备份位于 %s", exePath, backupPath)

	// 成功安装后，更新缓存状态，提示需要重启。
	status.Message = action.Notes
	status.PendingRestart = true
	status.UpdateAvailable = false
	m.mu.Lock()
	m.pendingVersion = status.LatestVersion
	status.Cached = false
	status.CheckedAt = time.Now()
	status.CacheExpiresAt = status.CheckedAt.Add(m.cacheTTL)
	m.lastStatus = &status
	m.lastChecked = status.CheckedAt
	m.mu.Unlock()

	action.Status = status
	return action, nil
}

func (m *UpdateManager) fetchLatest(ctx context.Context) (UpdateStatus, error) {
	url := fmt.Sprintf(githubLatestReleasePath, githubOwner, githubRepo)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return UpdateStatus{}, err
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
		return UpdateStatus{}, err
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusForbidden {
		return UpdateStatus{}, fmt.Errorf("GitHub API 访问受限: %s", resp.Status)
	}
	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4<<10))
		return UpdateStatus{}, fmt.Errorf("GitHub API 请求失败: %s (%s)", resp.Status, strings.TrimSpace(string(body)))
	}

	var rel githubRelease
	if err := json.NewDecoder(resp.Body).Decode(&rel); err != nil {
		return UpdateStatus{}, err
	}

	status := UpdateStatus{
		LatestVersion: rel.TagName,
		ReleaseURL:    fmt.Sprintf(githubReleasePage, githubOwner, githubRepo, rel.TagName),
		Architecture:  fmt.Sprintf("%s/%s", runtime.GOOS, runtime.GOARCH),
	}
	if rel.PublishedAt != nil {
		status.PublishedAt = rel.PublishedAt
	}

	asset, message := selectAsset(rel.Assets)
	if asset != nil {
		status.AssetName = asset.Name
		status.DownloadURL = asset.BrowserDownloadURL
	}
	if message != "" {
		status.Message = message
	}

	return status, nil
}

func (m *UpdateManager) downloadAsset(ctx context.Context, url string) (string, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("User-Agent", userAgent)
	if token := os.Getenv("MOSDNS_GITHUB_TOKEN"); token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	} else if token := os.Getenv("GITHUB_TOKEN"); token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}

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

func replaceBinary(exePath, newBinary string, mode os.FileMode) (string, error) {
	dir := filepath.Dir(exePath)
	backupPath := fmt.Sprintf("%s.bak.%s", exePath, time.Now().Format("20060102-150405"))

	tempDest, err := os.CreateTemp(dir, "mosdns-new-*")
	if err != nil {
		return "", err
	}
	tempDestPath := tempDest.Name()
	tempDest.Close()

	if err := copyFile(newBinary, tempDestPath, mode); err != nil {
		os.Remove(tempDestPath)
		return "", err
	}

	if err := os.Rename(exePath, backupPath); err != nil {
		os.Remove(tempDestPath)
		return "", err
	}

	if err := os.Rename(tempDestPath, exePath); err != nil {
		// 尝试还原旧文件
		os.Rename(backupPath, exePath)
		os.Remove(tempDestPath)
		return "", err
	}

	return backupPath, nil
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

func selectAsset(assets []githubAsset) (*githubAsset, string) {
	candidates := buildAssetCandidates()
	for _, preferred := range candidates {
		for i := range assets {
			if assets[i].Name == preferred {
				return &assets[i], ""
			}
		}
	}
	if len(assets) == 0 {
		return nil, "发布页未提供任何资产文件"
	}
	return nil, fmt.Sprintf("未找到适用于 %s/%s 的安装包", runtime.GOOS, runtime.GOARCH)
}

func buildAssetCandidates() []string {
	var list []string
	goos := runtime.GOOS
	arch := runtime.GOARCH

	switch goos {
	case "linux":
		switch arch {
		case "amd64":
			list = append(list, "mosdns-linux-amd64-v3.zip", "mosdns-linux-amd64.zip")
		case "arm64":
			list = append(list, "mosdns-linux-arm64.zip")
		case "arm":
			list = append(list, "mosdns-linux-arm-7.zip", "mosdns-linux-arm-6.zip", "mosdns-linux-arm-5.zip")
		case "mips":
			list = append(list, "mosdns-linux-mips.zip", "mosdns-linux-mips-softfloat.zip")
		case "mipsle":
			list = append(list, "mosdns-linux-mipsle.zip", "mosdns-linux-mipsle-softfloat.zip")
		case "mips64":
			list = append(list, "mosdns-linux-mips64.zip", "mosdns-linux-mips64-softfloat.zip")
		case "mips64le":
			list = append(list, "mosdns-linux-mips64le.zip", "mosdns-linux-mips64le-softfloat.zip")
		default:
			list = append(list, fmt.Sprintf("mosdns-linux-%s.zip", arch))
		}
	case "darwin":
		switch arch {
		case "amd64":
			list = append(list, "mosdns-darwin-amd64.zip")
		case "arm64":
			list = append(list, "mosdns-darwin-arm64.zip")
		}
	case "windows":
		switch arch {
		case "amd64":
			list = append(list, "mosdns-windows-amd64-v3.zip", "mosdns-windows-amd64.zip")
		case "arm64":
			list = append(list, "mosdns-windows-arm64.zip")
		}
	case "android":
		if arch == "arm64" {
			list = append(list, "mosdns-android-arm64.zip")
		}
	}

	return list
}

func (m *UpdateManager) isUpdateNeeded(latest string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.updateAvailableLocked(latest)
}

func (m *UpdateManager) updateAvailableLocked(latest string) bool {
	latest = strings.TrimSpace(latest)
	current := strings.TrimSpace(m.currentVersion)
	if latest == "" {
		return false
	}
	if m.pendingVersion != "" && latest == m.pendingVersion {
		return false
	}
	if current == "" {
		return true
	}
	return latest != current
}
