package coremain

import (
	"encoding/json"
	"errors"
	"io"
	"net/http"

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
	status, err := GlobalUpdateManager.CheckForUpdate(r.Context(), false)
	if err != nil {
		writeError(w, http.StatusBadGateway, err)
		return
	}
	writeJSON(w, http.StatusOK, status)
}

func handleForceUpdateStatus(w http.ResponseWriter, r *http.Request) {
	status, err := GlobalUpdateManager.CheckForUpdate(r.Context(), true)
	if err != nil {
		writeError(w, http.StatusBadGateway, err)
		return
	}
	writeJSON(w, http.StatusOK, status)
}

func handleApplyUpdate(w http.ResponseWriter, r *http.Request) {
    force := false
    preferV3 := false
    if r.Body != nil && r.Body != http.NoBody {
        var req struct {
            Force bool `json:"force"`
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
