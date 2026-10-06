package coremain

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/go-chi/chi/v5"
)

func TestUnmatchedRouteReturns404AndMethodMismatchKeepsLegacyStatus(t *testing.T) {
	router := chi.NewRouter()
	router.Get("/api/v1/known", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	})
	registerInvalidAPIRequestHandlers(router)

	tests := []struct {
		name   string
		method string
		path   string
		want   int
		body   string
	}{
		{name: "Go capability probe", method: http.MethodGet, path: "/api/v1/capabilities", want: http.StatusNotFound, body: "Invalid request GET /api/v1/capabilities"},
		{name: "native cache catalog enables Go fallback", method: http.MethodGet, path: "/api/v1/cache/inventory", want: http.StatusNotFound, body: "Invalid request GET /api/v1/cache/inventory"},
		{name: "optional plugin route enables Go fallback", method: http.MethodGet, path: "/plugins/requery", want: http.StatusNotFound, body: "Invalid request GET /plugins/requery"},
		{name: "unknown route preserves help body", method: http.MethodGet, path: "/api/v1/unknown", want: http.StatusNotFound, body: "Invalid request GET /api/v1/unknown"},
		{name: "unsupported method preserves help status", method: http.MethodPost, path: "/api/v1/known", want: http.StatusOK, body: "Invalid request POST /api/v1/known"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			req := httptest.NewRequest(tt.method, tt.path, nil)
			res := httptest.NewRecorder()
			router.ServeHTTP(res, req)

			if res.Code != tt.want {
				t.Fatalf("status = %d, want %d", res.Code, tt.want)
			}
			if body := res.Body.String(); !strings.Contains(body, tt.body) {
				t.Fatalf("route help missing from response: %q", body)
			}
			if !strings.Contains(res.Body.String(), "Available api urls:") || !strings.Contains(res.Body.String(), "GET /api/v1/known") {
				t.Fatalf("legacy route help missing from response: %q", res.Body.String())
			}
		})
	}
}
