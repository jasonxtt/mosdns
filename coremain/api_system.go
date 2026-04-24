package coremain

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"reflect"
	"strings"
	"syscall"
	"time"

	"github.com/go-chi/chi/v5"
	"go.uber.org/zap"
)

// RegisterSystemAPI 提供系统级操作，如自重启。
func RegisterSystemAPI(router *chi.Mux, m *Mosdns) {
	router.Route("/api/v1/system", func(r chi.Router) {
		r.Post("/restart", handleSelfRestart(m))
		r.Get("/webui-port", handleGetWebUIPort(m))
		r.Post("/webui-port", handleSetWebUIPort(m))
	})
}

func handleGetWebUIPort(m *Mosdns) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		activeAddr := strings.TrimSpace(m.apiHTTPAddr)
		activePort, _ := parsePortFromListenAddr(activeAddr)

		configuredPort := activePort
		settings, err := loadWebUIPortSettings()
		if err != nil {
			writeError(w, http.StatusInternalServerError, fmt.Errorf("读取 WebUI 端口设置失败: %w", err))
			return
		}
		if settings.Port > 0 {
			configuredPort = settings.Port
		}

		writeJSON(w, http.StatusOK, map[string]any{
			"port":            configuredPort,
			"active_port":     activePort,
			"active_addr":     activeAddr,
			"pending_restart": configuredPort != activePort,
		})
	}
}

func handleSetWebUIPort(m *Mosdns) http.HandlerFunc {
	type reqBody struct {
		Port int `json:"port"`
	}
	return func(w http.ResponseWriter, r *http.Request) {
		var body reqBody
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			writeError(w, http.StatusBadRequest, fmt.Errorf("无效请求: %w", err))
			return
		}
		port, err := normalizeWebUIPort(body.Port)
		if err != nil {
			writeError(w, http.StatusBadRequest, err)
			return
		}

		activeAddr := strings.TrimSpace(m.apiHTTPAddr)
		if activeAddr == "" {
			writeError(w, http.StatusServiceUnavailable, errors.New("当前未启用 WebUI/API 服务"))
			return
		}
		activePort, _ := parsePortFromListenAddr(activeAddr)
		targetAddr, err := replaceListenAddrPort(activeAddr, port)
		if err != nil {
			writeError(w, http.StatusBadRequest, err)
			return
		}
		if err := checkWebUIPortAvailable(activeAddr, port); err != nil {
			writeError(w, http.StatusConflict, err)
			return
		}
		if err := saveWebUIPortSettings(port); err != nil {
			writeError(w, http.StatusInternalServerError, fmt.Errorf("保存 WebUI 端口设置失败: %w", err))
			return
		}

		writeJSON(w, http.StatusOK, map[string]any{
			"port":            port,
			"active_port":     activePort,
			"active_addr":     activeAddr,
			"target_addr":     targetAddr,
			"pending_restart": port != activePort,
			"message":         "WebUI 端口已保存，重启后生效",
		})
	}
}

func handleSelfRestart(m *Mosdns) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
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
			// 复用 api_update.go 中的 writeJSON
			writeJSON(w, http.StatusNotImplemented, map[string]any{
				"error": "self-restart is not supported on Windows",
			})
			return
		}

		// 1. 立即响应
		writeJSON(w, http.StatusOK, map[string]any{"status": "scheduled", "delay_ms": body.DelayMs})

		go func(delay int) {
			logger := m.Logger()

			// 2. 等待延迟
			time.Sleep(time.Duration(delay) * time.Millisecond)

			exe, err := os.Executable()
			if err != nil {
				logger.Error("self-restart failed: cannot get executable path", zap.Error(err))
				return
			}

			// 3. [核心逻辑] 定向关闭需要保存数据的插件
			logger.Info("saving data for targeted plugins...")

			for tag, p := range m.plugins {
				// 【新增防御与排查】：拦截 nil 插件并打印日志
				if p == nil {
					// 打印出是哪个 tag 的插件变成了 nil
					logger.Warn("⚠️ 发现未初始化的插件 (nil plugin)", zap.String("tag", tag))
					continue
				}
				// 获取类型名称
				typeName := reflect.TypeOf(p).String()

				// 只匹配 cache 和 domain_output
				isCache := strings.Contains(typeName, "cache.Cache")
				isDomainOutput := strings.Contains(typeName, "domain_output")

				if isCache || isDomainOutput {
					if closer, ok := p.(io.Closer); ok {
						logger.Info("closing plugin to save data",
							zap.String("tag", tag),
							zap.String("type", typeName))

						// 这里会阻塞，直到文件写入操作完成 (Go bufio -> OS Cache)
						if err := closer.Close(); err != nil {
							logger.Warn("failed to close plugin", zap.String("tag", tag), zap.Error(err))
						}
					}
				}
			}
			logger.Info("targeted data save completed")

			// 4. [已移除] syscall.Sync()
			// 既然只是进程重启而非系统关机，Close() 将数据写入 OS Cache 已经足够安全且高效。
			// 移除后也解决了 Windows 编译报错问题。

			// 5. 执行重启 (进程替换)
			logger.Info("executing syscall.Exec", zap.String("exe", exe))
			_ = logger.Sync()

			rawArgs := append([]string{exe}, os.Args[1:]...)
			env := os.Environ()

			err = syscall.Exec(exe, rawArgs, env)

			if err != nil {
				fmt.Printf("[FATAL] syscall.Exec failed: %v\n", err)
				os.Exit(1)
			}
		}(body.DelayMs)
	}
}

func isWindows() bool {
	return os.PathSeparator == '\\'
}
