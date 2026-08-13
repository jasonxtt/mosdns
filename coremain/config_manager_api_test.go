package coremain

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/go-chi/chi/v5"
)

func TestRegisterConfigManagerAPIIfEnabled(t *testing.T) {
	tests := []struct {
		name       string
		enabled    bool
		wantStatus int
	}{
		{name: "enabled", enabled: true, wantStatus: http.StatusBadRequest},
		{name: "disabled", enabled: false, wantStatus: http.StatusNotFound},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			router := chi.NewRouter()
			registerConfigManagerAPIIfEnabled(router, tt.enabled)

			response := httptest.NewRecorder()
			router.ServeHTTP(response, httptest.NewRequest(http.MethodPost, "/api/v1/config/export", nil))
			if response.Code != tt.wantStatus {
				t.Fatalf("status = %d, want %d", response.Code, tt.wantStatus)
			}
		})
	}
}
