package coremain

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestHandleConfigExportRejectsContainerMode(t *testing.T) {
	t.Setenv(containerModeEnv, "1")

	req := httptest.NewRequest(http.MethodPost, "/api/v1/config/export", strings.NewReader(`{"dir":"/cus/mosdns"}`))
	rec := httptest.NewRecorder()

	handleConfigExport(rec, req)

	if rec.Code != http.StatusConflict {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusConflict)
	}
	if body := rec.Body.String(); !strings.Contains(body, containerConfigManageMessage) {
		t.Fatalf("response body %q does not contain %q", body, containerConfigManageMessage)
	}
}

func TestHandleConfigUpdateFromURLRejectsContainerMode(t *testing.T) {
	t.Setenv(containerModeEnv, "1")

	req := httptest.NewRequest(http.MethodPost, "/api/v1/config/update_from_url", strings.NewReader(`{"url":"https://example.com/config.zip","dir":"/cus/mosdns"}`))
	rec := httptest.NewRecorder()

	handleConfigUpdateFromURL(rec, req)

	if rec.Code != http.StatusConflict {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusConflict)
	}
	if body := rec.Body.String(); !strings.Contains(body, containerConfigManageMessage) {
		t.Fatalf("response body %q does not contain %q", body, containerConfigManageMessage)
	}
}
