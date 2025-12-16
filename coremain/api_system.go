package coremain

import (
	"encoding/json"
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
	})
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

		// 1. 立即响应给客户端，复用 api_update.go 中的 writeJSON
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
			// 我们只关闭 Cache 和 DomainOutput，绝对不要关闭负责网络监听的插件
			logger.Info("saving data for targeted plugins...")
			
			for tag, p := range m.plugins {
				// 获取插件实例的类型名称，例如 "*cache.Cache"
				typeName := reflect.TypeOf(p).String()

				// 这里的判断逻辑基于你提供的 cache.go 和 domain_output.go 的结构体命名
				isCache := strings.Contains(typeName, "cache.Cache")
				isDomainOutput := strings.Contains(typeName, "domain_output")

				if isCache || isDomainOutput {
					// 动态检查是否实现了 io.Closer
					if closer, ok := p.(io.Closer); ok {
						logger.Info("closing plugin to save data", 
							zap.String("tag", tag), 
							zap.String("type", typeName))
						
						if err := closer.Close(); err != nil {
							logger.Warn("failed to close plugin", zap.String("tag", tag), zap.Error(err))
						}
					}
				} else {
					// 调试日志：记录被跳过的插件，确保 server/entry 被跳过
					// logger.Debug("skipping plugin shutdown", zap.String("tag", tag), zap.String("type", typeName))
				}
			}
			logger.Info("targeted data save completed")

			// 4. 强制刷写操作系统文件缓冲区 (双重保险)
			fmt.Println("[SYSTEM] Syncing OS buffers...")
			syscall.Sync()

			// 5. 执行重启 (进程替换)
			// 此时 UDP/TCP 监听器依然开着，但进程马上就会被 syscall.Exec 替换
			// 这种方式不会触发 Go Runtime 的 "use of closed network connection" 致命错误
			logger.Info("executing syscall.Exec", zap.String("exe", exe))
			_ = logger.Sync()

			rawArgs := append([]string{exe}, os.Args[1:]...)
			env := os.Environ()

			err = syscall.Exec(exe, rawArgs, env)
			
			// 如果代码能走到这里，说明 Exec 失败了
			if err != nil {
				fmt.Printf("[FATAL] syscall.Exec failed: %v\n", err)
				// 此时处于不确定状态，建议直接退出
				os.Exit(1)
			}
		}(body.DelayMs)
	}
}

func isWindows() bool {
	return os.PathSeparator == '\\'
}
