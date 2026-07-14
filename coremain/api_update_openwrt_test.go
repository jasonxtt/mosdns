package coremain

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestHandleApplyUpdateRejectsOpenWrtMode(t *testing.T) {
	t.Setenv(openWrtModeEnv, "1")

	req := httptest.NewRequest(http.MethodPost, "/api/v1/update/apply", strings.NewReader(`{}`))
	rec := httptest.NewRecorder()

	handleApplyUpdate(rec, req)

	if rec.Code != http.StatusConflict {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusConflict)
	}
	if body := rec.Body.String(); !strings.Contains(body, openWrtUpdateConflictReason) {
		t.Fatalf("response body %q does not contain %q", body, openWrtUpdateConflictReason)
	}
}
