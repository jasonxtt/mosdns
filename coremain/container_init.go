package coremain

import (
	"archive/zip"
	"bytes"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"go.uber.org/zap"
)

const containerConfigInitTimeout = 60 * time.Second

func ensureContainerConfigInitialized(baseDir, configPath string) error {
	if !containerModeEnabled() || !containerAutoInitEnabled() {
		return nil
	}

	baseDir = strings.TrimSpace(baseDir)
	if baseDir == "" {
		return nil
	}

	if configPath == "" {
		configPath = filepath.Join(baseDir, "config_custom.yaml")
	}

	if _, err := os.Stat(configPath); err == nil {
		return nil
	} else if !os.IsNotExist(err) {
		return fmt.Errorf("failed to stat config file %s: %w", configPath, err)
	}

	if err := os.MkdirAll(baseDir, 0o755); err != nil {
		return fmt.Errorf("failed to prepare container config dir %s: %w", baseDir, err)
	}

	empty, err := dirIsEmpty(baseDir)
	if err != nil {
		return err
	}
	if !empty {
		return fmt.Errorf("container auto-init refused: %s is not empty but %s is missing", baseDir, configPath)
	}

	initURLs := containerConfigInitURLs()
	mlog.L().Info("starting container config auto-init",
		zap.String("dir", baseDir),
		zap.String("config", configPath),
		zap.Strings("urls", initURLs))

	if err := downloadAndExtractConfigZip(initURLs, baseDir); err != nil {
		return err
	}

	if _, err := os.Stat(configPath); err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("container auto-init completed but %s is still missing", configPath)
		}
		return fmt.Errorf("failed to verify auto-initialized config %s: %w", configPath, err)
	}

	mlog.L().Info("container config auto-init completed",
		zap.String("dir", baseDir),
		zap.String("config", configPath))
	return nil
}

func dirIsEmpty(dir string) (bool, error) {
	f, err := os.Open(dir)
	if err != nil {
		return false, fmt.Errorf("failed to open dir %s: %w", dir, err)
	}
	defer f.Close()

	names, err := f.Readdirnames(1)
	if err == io.EOF {
		return true, nil
	}
	if err != nil {
		return false, fmt.Errorf("failed to inspect dir %s: %w", dir, err)
	}
	if len(names) > 0 {
		return false, nil
	}
	return true, nil
}

func downloadAndExtractConfigZip(urls []string, destDir string) error {
	var errs []string
	for _, url := range urls {
		url = strings.TrimSpace(url)
		if url == "" {
			continue
		}
		if err := downloadAndExtractConfigZipOnce(url, destDir); err == nil {
			return nil
		} else {
			errs = append(errs, fmt.Sprintf("%s: %v", url, err))
			mlog.L().Warn("container config auto-init download failed, trying next url if available",
				zap.String("url", url),
				zap.Error(err))
		}
	}
	if len(errs) == 0 {
		return fmt.Errorf("failed to download container init config package: no valid init url configured")
	}
	return fmt.Errorf("failed to download container init config package from all urls: %s", strings.Join(errs, "; "))
}

func downloadAndExtractConfigZipOnce(url, destDir string) error {
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		return fmt.Errorf("failed to build config init request: %w", err)
	}

	client := &http.Client{Timeout: containerConfigInitTimeout}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("failed to download container init config package: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return fmt.Errorf("container init config package request failed: HTTP %s: %s", resp.Status, strings.TrimSpace(string(body)))
	}

	payload, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("failed to read container init config package: %w", err)
	}

	zr, err := zip.NewReader(bytes.NewReader(payload), int64(len(payload)))
	if err != nil {
		return fmt.Errorf("failed to parse container init config package: %w", err)
	}

	for _, file := range zr.File {
		if err := extractZipEntry(file, destDir); err != nil {
			return err
		}
	}

	return nil
}

func extractZipEntry(file *zip.File, destDir string) error {
	name := strings.TrimSpace(file.Name)
	if name == "" {
		return nil
	}

	targetPath := filepath.Join(destDir, filepath.FromSlash(name))
	cleanDestDir, err := filepath.Abs(destDir)
	if err != nil {
		return fmt.Errorf("failed to resolve container init dir %s: %w", destDir, err)
	}
	cleanTargetPath, err := filepath.Abs(targetPath)
	if err != nil {
		return fmt.Errorf("failed to resolve container init target %s: %w", targetPath, err)
	}
	prefix := cleanDestDir + string(os.PathSeparator)
	if cleanTargetPath != cleanDestDir && !strings.HasPrefix(cleanTargetPath, prefix) {
		return fmt.Errorf("container init zip contains invalid path %q", file.Name)
	}

	mode := file.Mode()
	if file.FileInfo().IsDir() {
		if err := os.MkdirAll(cleanTargetPath, mode.Perm()); err != nil {
			return fmt.Errorf("failed to create dir %s: %w", cleanTargetPath, err)
		}
		return nil
	}

	if err := os.MkdirAll(filepath.Dir(cleanTargetPath), 0o755); err != nil {
		return fmt.Errorf("failed to create parent dir for %s: %w", cleanTargetPath, err)
	}

	rc, err := file.Open()
	if err != nil {
		return fmt.Errorf("failed to open zip entry %s: %w", file.Name, err)
	}
	defer rc.Close()

	out, err := os.OpenFile(cleanTargetPath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, mode.Perm())
	if err != nil {
		return fmt.Errorf("failed to create file %s: %w", cleanTargetPath, err)
	}
	defer out.Close()

	if _, err := io.Copy(out, rc); err != nil {
		return fmt.Errorf("failed to extract %s: %w", cleanTargetPath, err)
	}
	return nil
}
