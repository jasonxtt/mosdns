package coremain

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"syscall"
	"time"

	"github.com/go-chi/chi/v5"
	"go.uber.org/zap"
)

// RegisterSystemAPI 提供系统级操作，如自重启。
func RegisterSystemAPI(router *chi.Mux) {
	router.Route("/api/v1/system", func(r chi.Router) {
		r.Post("/restart", handleSelfRestart)
	})
}

func handleSelfRestart(w http.ResponseWriter, r *http.Request) {
	type reqBody struct {
		DelayMs int `json:"delay_ms"`
	}
	var body reqBody
	if r.Body != nil && r.Body != http.NoBody {
		_ = json.NewDecoder(r.Body).Decode(&body)
	}
	if body.DelayMs <= 0 {
		body.DelayMs = 300
	}

	if isWindows() {
		writeJSON(w, http.StatusNotImplemented, map[string]any{
			"error": "self-restart is not supported on Windows",
		})
		return
	}

	// 获取当前 API 的 Host (例如 127.0.0.1:9099)
	apiHost := r.Host
	if apiHost == "" {
		apiHost = "127.0.0.1:9099" // 兜底
	}

	writeJSON(w, http.StatusOK, map[string]any{"status": "scheduled", "delay_ms": body.DelayMs})

	go func(delay int) {
		time.Sleep(time.Duration(delay) * time.Millisecond)
		exe, err := os.Executable()
		if err != nil {
			if lg := GlobalUpdateManager.logger(); lg != nil {
				lg.Warn("self-restart get executable failed", zap.Error(err))
			}
			return
		}

		// --- [核心逻辑 1]：解析配置并保存数据 ---
		
		configPath := getConfigPathFromArgs()
		if configPath != "" {
			if !filepath.IsAbs(configPath) {
				if wd, err := os.Getwd(); err == nil {
					configPath = filepath.Join(wd, configPath)
				}
			}
			
			// 递归扫描
			fmt.Printf("\n[SAVE] Scanning configuration for plugins (Main: %s)...\n", configPath)
			saveUrls := recursiveScanPlugins(configPath, apiHost)
			
			// 执行保存
			if len(saveUrls) > 0 {
				performDataSave(saveUrls)
			} else {
				fmt.Println("[SAVE] No cache or domain_output plugins found needing save.")
			}
		}

		// --- [核心逻辑 2]：执行重启 ---
		
	  pid := os.Getpid()
		args := os.Args[1:]

		argsStr := ""
		for _, arg := range args {
			safeArg := strings.ReplaceAll(arg, "\"", "\\\"")
			argsStr += fmt.Sprintf(" \"%s\"", safeArg)
		}

		// 构造重启命令: kill -15 触发优雅关闭 -> sleep 1 等待 -> 启动新进程
// 将 kill 去掉，保留 sleep
shellCmd := fmt.Sprintf("kill -15 %d && sleep 0.1 && \"%s\"%s >/dev/null 2>&1",
   pid, exe, argsStr)

		fmt.Printf("\n[RESTART] Attempting graceful restart sequence...\n[RESTART] CMD: %s\n\n", shellCmd)

		if lg := GlobalUpdateManager.logger(); lg != nil {
			lg.Info("executing graceful restart script", zap.String("cmd", shellCmd))
			_ = lg.Sync()
		}

		cmd := exec.Command("/bin/sh", "-c", shellCmd)
		// 使用 Setsid 脱离父进程，替代 nohup
		setProcessGroup(cmd)

		if err := cmd.Start(); err != nil {
			if lg := GlobalUpdateManager.logger(); lg != nil {
				lg.Warn("failed to start restart script", zap.Error(err))
			}
		} else {
			time.Sleep(2 * time.Second)
		}

		if lg := GlobalUpdateManager.logger(); lg != nil {
			lg.Warn("graceful restart did not terminate process, falling back to syscall.Exec")
		}

		rawArgs := append([]string{exe}, os.Args[1:]...)
		env := os.Environ()
		_ = syscall.Exec(exe, rawArgs, env)
	}(body.DelayMs)
}

func isWindows() bool {
	return os.PathSeparator == '\\'
}

// getConfigPathFromArgs 从启动参数中提取配置文件路径
func getConfigPathFromArgs() string {
	args := os.Args
	for i, arg := range args {
		if (arg == "-c" || arg == "--config") && i+1 < len(args) {
			return args[i+1]
		}
	}
	return ""
}

// executeSaveRequest 发送 HTTP POST 请求
func executeSaveRequest(url string) {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, "POST", url, nil)
	if err != nil {
		fmt.Printf("[SAVE] Failed to create request for %s: %v\n", url, err)
		return
	}

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		fmt.Printf("[SAVE] Request failed: %s | Error: %v\n", url, err)
		return
	}
	defer resp.Body.Close()
	fmt.Printf("[SAVE] Triggered: %s | Status: %s\n", url, resp.Status)
}

// performDataSave 批量执行保存任务
func performDataSave(urls []string) {
	fmt.Println("[SAVE] Starting data persistence sequence...")
	for _, url := range urls {
		// 调用前增加 5ms 延迟，减轻压力
		time.Sleep(5 * time.Millisecond)
		executeSaveRequest(url)
	}
	fmt.Println("[SAVE] Data persistence sequence completed.")
}

var (
	reInclude    = regexp.MustCompile(`^\s*-\s*["']?([^"']+\.yaml)["']?`)
	rePluginTag  = regexp.MustCompile(`^\s*-\s*tag:\s*(\S+)`)
	rePluginType = regexp.MustCompile(`^\s*type:\s*(\S+)`)
)

// recursiveScanPlugins 递归扫描配置文件
func recursiveScanPlugins(path string, apiHost string) []string {
	var urls []string
	
	file, err := os.Open(path)
	if err != nil {
		fmt.Printf("[WARN] Failed to open config file: %s, error: %v\n", path, err)
		return urls
	}
	defer file.Close()

	baseDir := filepath.Dir(path)
	scanner := bufio.NewScanner(file)
	
	inPluginsBlock := false
	inIncludeBlock := false
	
	var currentTag string
	var currentType string

	flushPlugin := func() {
		if currentTag != "" && currentType != "" {
			// [需求变更] cache 和 domain_output 统一使用 /plugins/<tag>/save 接口
			if currentType == "cache" || currentType == "domain_output" {
				url := fmt.Sprintf("http://%s/plugins/%s/save", apiHost, currentTag)
				urls = append(urls, url)
			}
		}
		currentTag = ""
		currentType = ""
	}

	for scanner.Scan() {
		line := scanner.Text()
		trimmed := strings.TrimSpace(line)
		
		if strings.HasPrefix(trimmed, "#") {
			continue
		}

		if strings.HasPrefix(line, "plugins:") {
			inPluginsBlock = true
			inIncludeBlock = false
			continue
		} else if strings.HasPrefix(line, "include:") {
			inIncludeBlock = true
			inPluginsBlock = false
			flushPlugin()
			continue
		}

		if inIncludeBlock {
			if matches := reInclude.FindStringSubmatch(line); len(matches) > 1 {
				subFile := matches[1]
				if !filepath.IsAbs(subFile) {
					subFile = filepath.Join(baseDir, subFile)
				}
				urls = append(urls, recursiveScanPlugins(subFile, apiHost)...)
			}
		}

		if inPluginsBlock {
			if matches := rePluginTag.FindStringSubmatch(line); len(matches) > 1 {
				flushPlugin()
				currentTag = matches[1]
				continue
			}

			if matches := rePluginType.FindStringSubmatch(line); len(matches) > 1 {
				currentType = matches[1]
			}
		}
	}
	flushPlugin()

	return urls
}
