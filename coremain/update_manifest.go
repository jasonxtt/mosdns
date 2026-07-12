package coremain

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

const (
	updateManifestAssetName = "mosdns-update-manifest.json"
	updateManifestFormat    = 1
)

type releaseUpdateManifest struct {
	Format               int                       `json:"format"`
	Channel              string                    `json:"channel"`
	Version              string                    `json:"version"`
	RequiredConfigSchema int                       `json:"required_config_schema"`
	ConfigPackageID      string                    `json:"config_package_id"`
	Artifacts            map[string]updateArtifact `json:"artifacts"`
	Config               *updateConfigArtifact     `json:"config,omitempty"`
}

type updateArtifact struct {
	SHA256 string `json:"sha256"`
}

type updateConfigArtifact struct {
	URL    string `json:"url"`
	SHA256 string `json:"sha256"`
}

func (m *UpdateManager) loadReleaseUpdateManifest(ctx context.Context, status UpdateStatus, assets []githubAsset) (releaseUpdateManifest, error) {
	var manifestAsset *githubAsset
	for i := range assets {
		if assets[i].Name == updateManifestAssetName {
			manifestAsset = &assets[i]
			break
		}
	}
	if manifestAsset == nil {
		return releaseUpdateManifest{}, fmt.Errorf("版本 %s 缺少 %s，拒绝执行非事务更新", status.LatestVersion, updateManifestAssetName)
	}
	data, err := m.downloadBytes(ctx, manifestAsset.BrowserDownloadURL, 1<<20)
	if err != nil {
		return releaseUpdateManifest{}, fmt.Errorf("下载更新清单失败: %w", err)
	}
	var manifest releaseUpdateManifest
	dec := json.NewDecoder(strings.NewReader(string(data)))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&manifest); err != nil {
		return releaseUpdateManifest{}, fmt.Errorf("解析更新清单失败: %w", err)
	}
	if err := validateReleaseUpdateManifest(manifest, status); err != nil {
		return releaseUpdateManifest{}, err
	}
	return manifest, nil
}

func validateReleaseUpdateManifest(manifest releaseUpdateManifest, status UpdateStatus) error {
	if manifest.Format != updateManifestFormat {
		return fmt.Errorf("不支持的更新清单格式 %d", manifest.Format)
	}
	if manifest.Channel != defaultUpdateChannel && manifest.Channel != "lite" {
		return fmt.Errorf("无效的更新通道 %q", manifest.Channel)
	}
	if manifest.Version != status.LatestVersion {
		return fmt.Errorf("更新清单版本 %q 与目标版本 %q 不一致", manifest.Version, status.LatestVersion)
	}
	if manifest.RequiredConfigSchema < 0 {
		return errors.New("更新清单配置 schema 无效")
	}
	if manifest.RequiredConfigSchema > 0 && strings.TrimSpace(manifest.ConfigPackageID) == "" {
		return errors.New("更新清单缺少配置包 ID")
	}
	artifact, ok := manifest.Artifacts[status.AssetName]
	if !ok {
		return fmt.Errorf("更新清单未包含二进制资产 %q", status.AssetName)
	}
	if !validSHA256(artifact.SHA256) {
		return fmt.Errorf("二进制资产 %q 的 SHA-256 无效", status.AssetName)
	}
	if manifest.Config != nil {
		if strings.TrimSpace(manifest.Config.URL) == "" || !validSHA256(manifest.Config.SHA256) {
			return errors.New("更新清单中的配置资产无效")
		}
	}
	return nil
}

func (m *UpdateManager) downloadBytes(ctx context.Context, url string, limit int64) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", userAgent)
	resp, err := m.doRequestWithFallback(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("HTTP %s", resp.Status)
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, limit+1))
	if err != nil {
		return nil, err
	}
	if int64(len(data)) > limit {
		return nil, fmt.Errorf("下载内容超过 %d 字节", limit)
	}
	return data, nil
}

func verifyFileSHA256(path, expected string) error {
	actual, err := fileSHA256(path)
	if err != nil {
		return err
	}
	if !strings.EqualFold(actual, strings.TrimSpace(expected)) {
		return fmt.Errorf("SHA-256 不匹配: 期望 %s，实际 %s", expected, actual)
	}
	return nil
}

func verifyBytesSHA256(data []byte, expected string) error {
	sum := sha256.Sum256(data)
	actual := hex.EncodeToString(sum[:])
	if !strings.EqualFold(actual, strings.TrimSpace(expected)) {
		return fmt.Errorf("SHA-256 不匹配: 期望 %s，实际 %s", expected, actual)
	}
	return nil
}

func validSHA256(value string) bool {
	value = strings.TrimSpace(value)
	if len(value) != sha256.Size*2 {
		return false
	}
	_, err := hex.DecodeString(value)
	return err == nil
}

func writeBytesAtomic(path string, data []byte, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".update-download-*")
	if err != nil {
		return err
	}
	tmpPath := tmp.Name()
	defer os.Remove(tmpPath)
	if err := tmp.Chmod(mode); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmpPath, path)
}
