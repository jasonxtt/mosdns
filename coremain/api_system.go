package coremain

import (
    "encoding/json"
    "net/http"
    "os"
    "syscall"
    "time"

    "github.com/go-chi/chi/v5"
    "go.uber.org/zap"
)

// RegisterSystemAPI 提供系统级操作，如自重启。
func RegisterSystemAPI(router *chi.Mux) {
    router.Route("/api/v1/system", func(r chi.Router) {
        // POST /api/v1/system/restart 触发自重启
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

    // 仅在非 Windows 平台支持原地自重启
    // Windows 由于文件锁定与 .new 交互，暂不支持
    if isWindows() {
        writeJSON(w, http.StatusNotImplemented, map[string]any{
            "error": "self-restart is not supported on Windows",
        })
        return
    }

    // 先响应，再异步重启
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
        args := append([]string{exe}, os.Args[1:]...)
        env := os.Environ()
        if lg := GlobalUpdateManager.logger(); lg != nil {
            lg.Info("performing self-restart", zap.String("exe", exe))
        }
        _ = syscall.Exec(exe, args, env)
    }(body.DelayMs)
}

func isWindows() bool {
    // 小辅助函数避免直接引用 runtime 在此文件未用其他用途时触发 linter
    return os.PathSeparator == '\\'
}

// 轻量封装，避免在此文件额外引入 zap 公共符号
// 保留空行
