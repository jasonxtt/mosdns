package coremain

import (
	"encoding/json"
	"net/http"
	"sort"

	"github.com/go-chi/chi/v5"
)

func RegisterDomainGenerationAPI(router *chi.Mux, m *Mosdns) {
	router.Route("/api/v1/domain-generation", func(r chi.Router) {
		r.Get("/", handleGetDomainGenerationSettings)
		r.Post("/", handleSetDomainGenerationSettings(m))
	})
}

func handleGetDomainGenerationSettings(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, GetDomainGenerationSettings())
}

func handleSetDomainGenerationSettings(m *Mosdns) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		var patch domainGenerationSettingsPatch
		if err := json.NewDecoder(r.Body).Decode(&patch); err != nil {
			writeError(w, http.StatusBadRequest, err)
			return
		}

		before, after, err := UpdateDomainGenerationSettings(patch)
		if err != nil {
			writeError(w, http.StatusInternalServerError, err)
			return
		}

		flushTargets := DomainGenerationFlushTargets(before, after)
		sort.Strings(flushTargets)
		flushDomainGenerationPlugins(m, flushTargets)

		writeJSON(w, http.StatusOK, map[string]any{
			"settings":      after,
			"flush_targets": flushTargets,
		})
	}
}

func flushDomainGenerationPlugins(m *Mosdns, tags []string) {
	if m == nil || len(tags) == 0 {
		return
	}

	for _, tag := range tags {
		p := m.GetPlugin(tag)
		flusher, ok := p.(interface{ FlushGeneratedData() })
		if !ok || flusher == nil {
			continue
		}
		flusher.FlushGeneratedData()
	}
}
