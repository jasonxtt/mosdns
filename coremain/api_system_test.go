package coremain

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

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
