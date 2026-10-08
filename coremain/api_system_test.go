package coremain

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestHandleGetWebUIPortAllowsNativeSettings(t *testing.T) {
	t.Setenv(containerModeEnv, "")
	originalBaseDir := MainConfigBaseDir
	MainConfigBaseDir = t.TempDir()
	t.Cleanup(func() { MainConfigBaseDir = originalBaseDir })
	if err := saveWebUIPortSettings(9191); err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequest(http.MethodGet, "/api/v1/system/webui-port", nil)
	rec := httptest.NewRecorder()
	handleGetWebUIPort(&Mosdns{apiHTTPAddr: ":9099"})(rec, req)
	var body struct {
		Port            int  `json:"port"`
		ChangeSupported bool `json:"change_supported"`
		PendingRestart  bool `json:"pending_restart"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if rec.Code != http.StatusOK || !body.ChangeSupported || !body.PendingRestart || body.Port != 9191 {
		t.Fatalf("native settings not preserved: status=%d body=%+v", rec.Code, body)
	}
}

func TestHandleSetWebUIPortValidatesHostAndNativeRequests(t *testing.T) {
	for _, mode := range []string{"", "1"} {
		t.Run("container="+mode, func(t *testing.T) {
			t.Setenv(containerModeEnv, mode)
			t.Setenv(containerNetworkModeEnv, containerNetworkModeHost)
			req := httptest.NewRequest(http.MethodPost, "/api/v1/system/webui-port", strings.NewReader(`{"port":0}`))
			rec := httptest.NewRecorder()
			handleSetWebUIPort(&Mosdns{apiHTTPAddr: ":9099"})(rec, req)
			if rec.Code != http.StatusBadRequest {
				t.Fatalf("status = %d, want normal port validation (%d)", rec.Code, http.StatusBadRequest)
			}
		})
	}
}

func TestHandleGetWebUIPortMarksContainerModeUnsupported(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerNetworkModeEnv, containerNetworkModeBridge)
	MainConfigBaseDir = t.TempDir()

	req := httptest.NewRequest(http.MethodGet, "/api/v1/system/webui-port", nil)
	rec := httptest.NewRecorder()

	m := &Mosdns{apiHTTPAddr: ":9099"}
	handleGetWebUIPort(m)(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}
	body := rec.Body.String()
	if !strings.Contains(body, `"change_supported":false`) {
		t.Fatalf("response body %q does not mark change_supported false", body)
	}
	if !strings.Contains(body, containerWebUIPortMessage) {
		t.Fatalf("response body %q does not contain %q", body, containerWebUIPortMessage)
	}
}

func TestHandleSetWebUIPortRejectsContainerMode(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerNetworkModeEnv, containerNetworkModeBridge)

	req := httptest.NewRequest(http.MethodPost, "/api/v1/system/webui-port", strings.NewReader(`{"port":9099}`))
	rec := httptest.NewRecorder()

	m := &Mosdns{apiHTTPAddr: ":9099"}
	handleSetWebUIPort(m)(rec, req)

	if rec.Code != http.StatusConflict {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusConflict)
	}
	if body := rec.Body.String(); !strings.Contains(body, containerWebUIPortMessage) {
		t.Fatalf("response body %q does not contain %q", body, containerWebUIPortMessage)
	}
}

func TestHandleGetWebUIPortAllowsHostContainerMode(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerNetworkModeEnv, containerNetworkModeHost)
	MainConfigBaseDir = t.TempDir()

	req := httptest.NewRequest(http.MethodGet, "/api/v1/system/webui-port", nil)
	rec := httptest.NewRecorder()

	m := &Mosdns{apiHTTPAddr: ":9099"}
	handleGetWebUIPort(m)(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}
	body := rec.Body.String()
	if !strings.Contains(body, `"change_supported":true`) {
		t.Fatalf("response body %q does not mark change_supported true", body)
	}
	if strings.Contains(body, containerWebUIPortMessage) {
		t.Fatalf("response body %q should not contain %q", body, containerWebUIPortMessage)
	}
}

func TestHandleGetWebUIPortIgnoresStoredOverrideInBridgeMode(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerNetworkModeEnv, containerNetworkModeBridge)
	MainConfigBaseDir = t.TempDir()

	webInfoDir := filepath.Join(MainConfigBaseDir, managedWebInfoDirName)
	if err := os.MkdirAll(webInfoDir, 0o755); err != nil {
		t.Fatalf("mkdir webinfo: %v", err)
	}
	if err := os.WriteFile(filepath.Join(webInfoDir, webUIPortSettingsFilename), []byte("{\n  \"port\": 80\n}\n"), 0o644); err != nil {
		t.Fatalf("write settings: %v", err)
	}

	req := httptest.NewRequest(http.MethodGet, "/api/v1/system/webui-port", nil)
	rec := httptest.NewRecorder()

	m := &Mosdns{apiHTTPAddr: ":9099"}
	handleGetWebUIPort(m)(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", rec.Code, http.StatusOK)
	}
	body := rec.Body.String()
	if !strings.Contains(body, `"port":9099`) {
		t.Fatalf("response body %q should report active port in bridge mode", body)
	}
	if strings.Contains(body, `"pending_restart":true`) {
		t.Fatalf("response body %q should not report pending restart in bridge mode", body)
	}
}
