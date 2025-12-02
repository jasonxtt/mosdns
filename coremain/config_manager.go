package coremain

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"time"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/go-chi/chi/v5"
	"go.uber.org/zap"
	"golang.org/x/net/proxy"
)

// ConfigManagerRequest 定义前端传入的参数
type ConfigManagerRequest struct {
	URL string `json:"url"` // 在线更新用的下载地址
	Dir string `json:"dir"` // 本地配置所在的目录
}

// RegisterConfigManagerAPI 注册配置管理相关的 API
func RegisterConfigManagerAPI(router *chi.Mux) {
	router.Post("/api/v1/config/export", handleConfigExport)
	router.Post("/api/v1/config/update_from_url", handleConfigUpdateFromURL)
}

// handleConfigExport 对应需求：把本地目录打包下载
func handleConfigExport(w http.ResponseWriter, r *http.Request) {
	var req ConfigManagerRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid request body", http.StatusBadRequest)
		return
	}
	if req.Dir == "" {
		http.Error(w, "dir is required", http.StatusBadRequest)
		return
	}

	// 设置响应头，告诉浏览器这是一个附件下载
	w.Header().Set("Content-Type", "application/zip")
	filename := fmt.Sprintf("mosdns_backup_%d.zip", time.Now().Unix())
	w.Header().Set("Content-Disposition", fmt.Sprintf("attachment; filename=\"%s\"", filename))

	zipWriter := zip.NewWriter(w)
	defer zipWriter.Close()

	// 遍历目录并打包
	err := filepath.Walk(req.Dir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		
		// 排除 backup 文件夹自身，避免递归备份或下载无用数据
		if info.IsDir() && info.Name() == "backup" {
			return filepath.SkipDir
		}
		if info.IsDir() {
			return nil
		}

		// 获取相对于根目录的路径，作为 zip 内的文件名
		relPath, err := filepath.Rel(req.Dir, path)
		if err != nil {
			return err
		}

		// 写入 Zip
		zipFile, err := zipWriter.Create(relPath)
		if err != nil {
			return err
		}
		
		fsFile, err := os.Open(path)
		if err != nil {
			return err
		}
		defer fsFile.Close()
		
		_, err = io.Copy(zipFile, fsFile)
		return err
	})

	if err != nil {
		mlog.L().Error("export config failed", zap.String("dir", req.Dir), zap.Error(err))
	}
}

// handleConfigUpdateFromURL 对应需求：下载 -> 备份 -> 覆盖 -> 重启
func handleConfigUpdateFromURL(w http.ResponseWriter, r *http.Request) {
	var req ConfigManagerRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid request body", http.StatusBadRequest)
		return
	}
	if req.URL == "" || req.Dir == "" {
		http.Error(w, "url and dir are required", http.StatusBadRequest)
		return
	}

	lg := mlog.L()

	// --- 1. 下载文件 (包含代理检测和降级逻辑) ---
	zipData, err := downloadWithFallback(req.URL)
	if err != nil {
		lg.Error("download config failed", zap.Error(err))
		http.Error(w, "Download failed: "+err.Error(), http.StatusInternalServerError)
		return
	}

	// --- 2. 执行备份 (先清空后备份，失败则熔断) ---
	backupDir := filepath.Join(req.Dir, "backup")
	if err := performLocalBackup(req.Dir, backupDir); err != nil {
		lg.Error("local backup failed, aborting update", zap.Error(err))
		http.Error(w, "Backup failed (update aborted): "+err.Error(), http.StatusInternalServerError)
		return
	}

	// --- 3. 解压并覆盖 (包含目录创建逻辑) ---
	updatedCount, err := extractAndOverwrite(zipData, req.Dir)
	if err != nil {
		lg.Error("extract and overwrite failed", zap.Error(err))
		http.Error(w, "Update files failed: "+err.Error(), http.StatusInternalServerError)
		return
	}

	// --- 4. 成功响应并触发重启 ---
	lg.Info("config update successful", zap.Int("files_updated", updatedCount))
	
	w.Header().Set("Content-Type", "application/json")
	fmt.Fprintf(w, `{"message": "Update successful. %d files updated. Restarting...", "status": "success"}`, updatedCount)

	// 异步触发重启，给前端一点时间处理响应
	go func() {
		time.Sleep(500 * time.Millisecond)
		triggerRestart()
	}()
}

// downloadWithFallback 尝试使用配置的 Socks5 下载，失败则直连
func downloadWithFallback(url string) ([]byte, error) {
	// 1. 尝试获取代理配置
	var proxyAddr string
	overridesPath := filepath.Join(MainConfigBaseDir, overridesFilename)
	data, err := os.ReadFile(overridesPath)
	if err == nil {
		var temp struct {
			Socks5 string `json:"socks5"`
		}
		if json.Unmarshal(data, &temp) == nil {
			proxyAddr = temp.Socks5
		}
	}

	// 2. 如果有代理，先尝试代理下载
	if proxyAddr != "" {
		mlog.L().Info("attempting download via proxy", zap.String("proxy", proxyAddr))
		data, err := doDownload(url, proxyAddr)
		if err == nil {
			return data, nil
		}
		mlog.L().Warn("download via proxy failed, falling back to direct", zap.Error(err))
	}

	// 3. 直连下载 (Fallback)
	mlog.L().Info("attempting direct download")
	return doDownload(url, "")
}

func doDownload(url, proxyAddr string) ([]byte, error) {
	client := &http.Client{Timeout: 60 * time.Second}

	if proxyAddr != "" {
		dialer, err := proxy.SOCKS5("tcp", proxyAddr, nil, proxy.Direct)
		if err != nil {
			return nil, err
		}
		client.Transport = &http.Transport{
			DialContext: (dialer.(proxy.ContextDialer)).DialContext,
		}
	}

	resp, err := client.Get(url)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("http status %s", resp.Status)
	}

	return io.ReadAll(resp.Body)
}

// performLocalBackup 将 source 目录备份到 dest，备份前先清空 dest
func performLocalBackup(source, dest string) error {
	// 1. 清空旧备份
	if err := os.RemoveAll(dest); err != nil {
		return fmt.Errorf("clean backup dir failed: %w", err)
	}
	// 2. 创建新备份目录
	if err := os.MkdirAll(dest, 0755); err != nil {
		return fmt.Errorf("create backup dir failed: %w", err)
	}

	// 3. 递归复制
	return filepath.Walk(source, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		
		// 跳过备份目录本身
		if path == dest || strings.HasPrefix(path, dest+string(os.PathSeparator)) {
			return nil
		}

		// 计算相对路径
		relPath, err := filepath.Rel(source, path)
		if err != nil {
			return err
		}
		targetPath := filepath.Join(dest, relPath)

		if info.IsDir() {
			return os.MkdirAll(targetPath, info.Mode())
		}

		// 复制文件内容
		srcFile, err := os.Open(path)
		if err != nil {
			return err
		}
		defer srcFile.Close()

		dstFile, err := os.Create(targetPath)
		if err != nil {
			return err
		}
		defer dstFile.Close()

		_, err = io.Copy(dstFile, srcFile)
		return err
	})
}

// extractAndOverwrite 解压 ZIP 并覆盖本地文件
func extractAndOverwrite(zipData []byte, targetDir string) (int, error) {
	zipReader, err := zip.NewReader(bytes.NewReader(zipData), int64(len(zipData)))
	if err != nil {
		return 0, fmt.Errorf("invalid zip data: %w", err)
	}

	count := 0
	for _, f := range zipReader.File {
		if f.FileInfo().IsDir() {
			continue
		}

		// 构造绝对路径
		fullPath := filepath.Join(targetDir, f.Name)

		// 安全检查：防止 zip slip (../../)
		if !strings.HasPrefix(fullPath, filepath.Clean(targetDir)+string(os.PathSeparator)) {
			continue
		}

		// 确保父目录存在
		if err := os.MkdirAll(filepath.Dir(fullPath), 0755); err != nil {
			return count, fmt.Errorf("create dir failed for %s: %w", f.Name, err)
		}

		// 写入文件 (覆盖模式)
		rc, err := f.Open()
		if err != nil {
			return count, err
		}
		
		dst, err := os.Create(fullPath)
		if err != nil {
			rc.Close()
			return count, fmt.Errorf("create file %s failed: %w", f.Name, err)
		}

		_, err = io.Copy(dst, rc)
		dst.Close()
		rc.Close()
		
		if err != nil {
			return count, fmt.Errorf("write file %s failed: %w", f.Name, err)
		}
		count++
	}
	return count, nil
}

// triggerRestart 尝试重启服务，逻辑对齐 update_manager.go
func triggerRestart() {
	lg := mlog.L()
	
	// 1. 尝试使用 HTTP API 重启 (优先读取环境变量)
	endpoint := strings.TrimSpace(os.Getenv("MOSDNS_RESTART_ENDPOINT"))
	if endpoint == "" {
		endpoint = "http://127.0.0.1:9099/api/v1/system/restart"
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	req, _ := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, strings.NewReader(`{"delay_ms":500}`))
	req.Header.Set("Content-Type", "application/json")
	
	// 针对 localhost/127.0.0.1 强制设置 Host，避免代理干扰
	// (update_manager.go 中有类似的逻辑)
	if u, err := req.URL.Parse(endpoint); err == nil {
		if h, _, _ := net.SplitHostPort(u.Host); h == "localhost" || h == "127.0.0.1" {
			req.Host = u.Host
		}
	}

	client := &http.Client{Timeout: 5 * time.Second}
	resp, err := client.Do(req)
	
	if err == nil {
		defer resp.Body.Close()
		io.Copy(io.Discard, resp.Body)
		if resp.StatusCode >= 200 && resp.StatusCode < 300 {
			lg.Info("http restart request sent successfully", zap.String("endpoint", endpoint))
			return
		}
		lg.Warn("http restart request returned non-2xx", zap.String("status", resp.Status))
	} else {
		lg.Warn("http restart request failed", zap.Error(err))
	}

	// 2. 如果 HTTP 重启失败且不是 Windows，尝试直接 Exec 重启 (Fallback)
	if runtime.GOOS != "windows" {
		exe, err := os.Executable()
		if err != nil {
			lg.Error("failed to get executable path for restart", zap.Error(err))
			return
		}
		lg.Info("falling back to syscall.Exec for restart")
		// 等待一小会儿确保 HTTP 响应已发送
		time.Sleep(100 * time.Millisecond)
		if err := syscall.Exec(exe, os.Args, os.Environ()); err != nil {
			lg.Error("syscall.Exec failed", zap.Error(err))
		}
	} else {
		lg.Warn("automatic restart failed, manual restart required")
	}
}
