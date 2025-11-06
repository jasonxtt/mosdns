package coremain

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"runtime"
	"time"

	"github.com/go-chi/chi/v5"
)

// RegisterUpdateAPI 暴露版本检查与在线更新接口。
func RegisterUpdateAPI(router *chi.Mux) {
	router.Route("/api/v1/update", func(r chi.Router) {
		r.Get("/status", handleUpdateStatus)
		r.Post("/check", handleForceUpdateStatus)
		r.Post("/apply", handleApplyUpdate)
	})
}

func handleUpdateStatus(w http.ResponseWriter, r *http.Request) {
	// 限时查询，避免前端长时间转圈（正常 3 秒内返回）
	ctx, cancel := context.WithTimeout(r.Context(), 3*time.Second)
	defer cancel()

	status, err := GlobalUpdateManager.CheckForUpdate(ctx, false)
	if err != nil {
		// 失败时也返回 200 与降级信息，前端不再卡在“检测中…”。
		fallback := UpdateStatus{
			CurrentVersion:  GetBuildVersion(),
			LatestVersion:   "",
			Architecture:    runtime.GOOS + "/" + runtime.GOARCH,
			CheckedAt:       time.Now(),
			CacheExpiresAt:  time.Now(),
			UpdateAvailable: false,
			Cached:          false,
			Message:         "检查更新失败：" + err.Error(),
		}
		writeJSON(w, http.StatusOK, fallback)
		return
	}
	writeJSON(w, http.StatusOK, status)
}

func handleForceUpdateStatus(w http.ResponseWriter, r *http.Request) {
	// 强制检查允许更长的时间窗口，但也设置上限以保证可用性
	ctx, cancel := context.WithTimeout(r.Context(), 8*time.Second)
	defer cancel()

	status, err := GlobalUpdateManager.CheckForUpdate(ctx, true)
	if err != nil {
		fallback := UpdateStatus{
			CurrentVersion:  GetBuildVersion(),
			LatestVersion:   "",
			Architecture:    runtime.GOOS + "/" + runtime.GOARCH,
			CheckedAt:       time.Now(),
			CacheExpiresAt:  time.Now(),
			UpdateAvailable: false,
			Cached:          false,
			Message:         "检查更新失败：" + err.Error(),
		}
		writeJSON(w, http.StatusOK, fallback)
		return
	}
	writeJSON(w, http.StatusOK, status)
}

func handleApplyUpdate(w http.ResponseWriter, r *http.Request) {
	force := false
	preferV3 := false
	if r.Body != nil && r.Body != http.NoBody {
		var req struct {
			Force    bool `json:"force"`
			PreferV3 bool `json:"prefer_v3"`
		}
		dec := json.NewDecoder(r.Body)
		if err := dec.Decode(&req); err != nil && !errors.Is(err, io.EOF) {
			writeError(w, http.StatusBadRequest, err)
			return
		}
		force = req.Force
		preferV3 = req.PreferV3
	}

	result, err := GlobalUpdateManager.PerformUpdate(r.Context(), force, preferV3)
	if err != nil {
		if errors.Is(err, ErrNoUpdateAvailable) {
			writeJSON(w, http.StatusOK, result)
			return
		}
		writeError(w, http.StatusBadGateway, err)
		return
	}
	writeJSON(w, http.StatusOK, result)
}

func writeJSON(w http.ResponseWriter, status int, payload any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(payload)
}

func writeError(w http.ResponseWriter, status int, err error) {
	writeJSON(w, status, map[string]string{"error": err.Error()})
}
