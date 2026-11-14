package coremain

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestHandleConfigExportPreservesNativeBackup(t *testing.T) {
	t.Setenv(containerModeEnv, "")
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "config_custom.yaml"), []byte("plugins: []\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	payload, err := json.Marshal(ConfigManagerRequest{Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequest(http.MethodPost, "/api/v1/config/export", bytes.NewReader(payload))
	rec := httptest.NewRecorder()
	handleConfigExport(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}
	zr, err := zip.NewReader(bytes.NewReader(rec.Body.Bytes()), int64(rec.Body.Len()))
	if err != nil {
		t.Fatalf("invalid native backup: %v", err)
	}
	if len(zr.File) != 1 || zr.File[0].Name != "config_custom.yaml" {
		t.Fatalf("unexpected backup contents: %+v", zr.File)
	}
}

func TestHandleConfigUpdateFromURLValidatesNativeRequest(t *testing.T) {
	t.Setenv(containerModeEnv, "")
	req := httptest.NewRequest(http.MethodPost, "/api/v1/config/update_from_url", strings.NewReader(`{}`))
	rec := httptest.NewRecorder()
	handleConfigUpdateFromURL(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want native request validation (%d)", rec.Code, http.StatusBadRequest)
	}
}

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
