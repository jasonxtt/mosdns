package coremain

import (
	"archive/zip"
	"bytes"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
)

func TestEnsureContainerConfigInitializedDownloadsIntoEmptyDir(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerAutoInitEnv, "1")

	payload := buildTestConfigZip(t, map[string]string{
		"config_custom.yaml":             "api:\n  http: 127.0.0.1:9099\n",
		"sub_config/special_groups.yaml": "special_groups: []\n",
	})

	var hits atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		w.Header().Set("Content-Type", "application/zip")
		_, _ = w.Write(payload)
	}))
	defer server.Close()

	t.Setenv(containerConfigInitURLEnv, server.URL+"/config_all.zip")

	dir := t.TempDir()
	configPath := filepath.Join(dir, "config_custom.yaml")
	if err := ensureContainerConfigInitialized(dir, configPath); err != nil {
		t.Fatalf("ensureContainerConfigInitialized() error = %v", err)
	}

	if hits.Load() != 1 {
		t.Fatalf("download hits = %d, want 1", hits.Load())
	}
	assertTestFileContains(t, configPath, "127.0.0.1:9099")
	assertTestFileContains(t, filepath.Join(dir, "sub_config", "special_groups.yaml"), "special_groups")
}

func TestEnsureContainerConfigInitializedSkipsExistingConfig(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerAutoInitEnv, "1")

	var hits atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		http.Error(w, "unexpected download", http.StatusInternalServerError)
	}))
	defer server.Close()

	t.Setenv(containerConfigInitURLEnv, server.URL+"/config_all.zip")

	dir := t.TempDir()
	configPath := filepath.Join(dir, "config_custom.yaml")
	if err := os.WriteFile(configPath, []byte("plugins: []\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := ensureContainerConfigInitialized(dir, configPath); err != nil {
		t.Fatalf("ensureContainerConfigInitialized() error = %v", err)
	}

	if hits.Load() != 0 {
		t.Fatalf("download hits = %d, want 0", hits.Load())
	}
	assertTestFileContains(t, configPath, "plugins: []")
}

func TestEnsureContainerConfigInitializedSkipsDisabledModes(t *testing.T) {
	for _, mode := range []struct{ name, container, autoInit string }{
		{"native", "", "1"},
		{"manual-container", "1", ""},
	} {
		t.Run(mode.name, func(t *testing.T) {
			t.Setenv(containerModeEnv, mode.container)
			t.Setenv(containerAutoInitEnv, mode.autoInit)
			t.Setenv(containerConfigInitURLEnv, "http://127.0.0.1:1/unexpected")
			dir := t.TempDir()
			if err := ensureContainerConfigInitialized(dir, filepath.Join(dir, "config_custom.yaml")); err != nil {
				t.Fatal(err)
			}
			entries, err := os.ReadDir(dir)
			if err != nil || len(entries) != 0 {
				t.Fatalf("disabled auto-init modified directory: %v, %v", entries, err)
			}
		})
	}
}

func TestExtractContainerConfigRejectsPathTraversal(t *testing.T) {
	dir := t.TempDir()
	payload := buildTestConfigZip(t, map[string]string{"../operator.txt": "overwrite"})
	zr, err := zip.NewReader(bytes.NewReader(payload), int64(len(payload)))
	if err != nil {
		t.Fatal(err)
	}
	if err := extractZipEntry(zr.File[0], filepath.Join(dir, "config")); err == nil {
		t.Fatal("accepted a config archive entry outside the data directory")
	}
	if _, err := os.Stat(filepath.Join(dir, "operator.txt")); !os.IsNotExist(err) {
		t.Fatalf("archive wrote outside its destination: %v", err)
	}
}

func TestEnsureContainerConfigInitializedRejectsNonEmptyDirWithoutConfig(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerAutoInitEnv, "1")
	t.Setenv(containerConfigInitURLEnv, "https://example.com/config_all.zip")

	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "note.txt"), []byte("keep"), 0o644); err != nil {
		t.Fatal(err)
	}

	err := ensureContainerConfigInitialized(dir, filepath.Join(dir, "config_custom.yaml"))
	if err == nil {
		t.Fatal("ensureContainerConfigInitialized() error = nil, want refusal for non-empty dir")
	}
	if !strings.Contains(err.Error(), "is not empty") {
		t.Fatalf("ensureContainerConfigInitialized() error = %v, want non-empty hint", err)
	}
}

func TestDownloadAndExtractConfigZipFallsBackToNextURL(t *testing.T) {
	payload := buildTestConfigZip(t, map[string]string{
		"config_custom.yaml": "api:\n  http: 127.0.0.1:9099\n",
	})

	var badHits atomic.Int32
	badServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		badHits.Add(1)
		http.Error(w, "upstream blocked", http.StatusBadGateway)
	}))
	defer badServer.Close()

	var goodHits atomic.Int32
	goodServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		goodHits.Add(1)
		w.Header().Set("Content-Type", "application/zip")
		_, _ = w.Write(payload)
	}))
	defer goodServer.Close()

	dir := t.TempDir()
	if err := downloadAndExtractConfigZip([]string{
		badServer.URL + "/config_all.zip",
		goodServer.URL + "/config_all.zip",
	}, dir); err != nil {
		t.Fatalf("downloadAndExtractConfigZip() error = %v", err)
	}

	if badHits.Load() != 1 {
		t.Fatalf("bad server hits = %d, want 1", badHits.Load())
	}
	if goodHits.Load() != 1 {
		t.Fatalf("good server hits = %d, want 1", goodHits.Load())
	}
	assertTestFileContains(t, filepath.Join(dir, "config_custom.yaml"), "127.0.0.1:9099")
}

func buildTestConfigZip(t *testing.T, files map[string]string) []byte {
	t.Helper()

	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	for name, content := range files {
		w, err := zw.Create(name)
		if err != nil {
			t.Fatalf("Create(%q) error = %v", name, err)
		}
		if _, err := w.Write([]byte(content)); err != nil {
			t.Fatalf("Write(%q) error = %v", name, err)
		}
	}
	if err := zw.Close(); err != nil {
		t.Fatalf("zip close error = %v", err)
	}
	return buf.Bytes()
}

func assertTestFileContains(t *testing.T, path, substr string) {
	t.Helper()

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ReadFile(%q) error = %v", path, err)
	}
	if !strings.Contains(string(data), substr) {
		t.Fatalf("file %q does not contain %q", path, substr)
	}
}
