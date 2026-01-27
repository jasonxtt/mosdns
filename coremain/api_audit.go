// /root/mosdns/coremain/api_audit.go
package coremain

import (
	"encoding/json"
	"fmt"
	"net/http"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/go-chi/chi/v5"
	"go.uber.org/zap"
)

// RegisterAuditAPI registers the audit log APIs to the given router.
func RegisterAuditAPI(router *chi.Mux) {
	router.Route("/api/v1/audit", func(r chi.Router) {
		r.Post("/start", handleAuditStart)
		r.Post("/stop", handleAuditStop)
		r.Get("/status", handleAuditStatus)
		r.Get("/logs", WithAsyncGC(handleGetAuditLogs))
		r.Post("/clear", WithAsyncGC(handleClearAuditLogs))
		// ADDED: New routes for capacity management
		r.Get("/capacity", handleGetAuditCapacity)
		r.Post("/capacity", handleSetAuditCapacity)
	})
}

func handleAuditStart(w http.ResponseWriter, r *http.Request) {
	GlobalAuditCollector.Start()
	w.WriteHeader(http.StatusOK)
	fmt.Fprint(w, "Audit log collection started.")
}

func handleAuditStop(w http.ResponseWriter, r *http.Request) {
	GlobalAuditCollector.Stop()
	w.WriteHeader(http.StatusOK)
	fmt.Fprint(w, "Audit log collection stopped.")
}

func handleAuditStatus(w http.ResponseWriter, r *http.Request) {
	status := struct {
		Capturing bool `json:"capturing"`
	}{
		Capturing: GlobalAuditCollector.IsCapturing(),
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(status)
}

func handleGetAuditLogs(w http.ResponseWriter, r *http.Request) {
	logs := GlobalAuditCollector.GetLogs()
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(logs); err != nil {
		mlog.L().Error("failed to encode audit logs to client", zap.Error(err))
	}
}

func handleClearAuditLogs(w http.ResponseWriter, r *http.Request) {
	GlobalAuditCollector.ClearLogs()
	w.WriteHeader(http.StatusOK)
	fmt.Fprint(w, "In-memory audit logs cleared.")
}

// ADDED: Handler to get current audit log capacity.
func handleGetAuditCapacity(w http.ResponseWriter, r *http.Request) {
	capacity := struct {
		Capacity int `json:"capacity"`
	}{
		Capacity: GlobalAuditCollector.GetCapacity(),
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(capacity)
}

// MODIFIED: Handler to set new audit log capacity.
func handleSetAuditCapacity(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Capacity int `json:"capacity"`
	}

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid request body: "+err.Error(), http.StatusBadRequest)
		return
	}
	
	// <<< MODIFIED: Pass the MainConfigBaseDir to the SetCapacity function >>>
	GlobalAuditCollector.SetCapacity(req.Capacity, MainConfigBaseDir)
	
	w.WriteHeader(http.StatusOK)
	fmt.Fprintf(w, "Audit log capacity set to %d. Existing logs have been cleared.", req.Capacity)
}
