package coremain

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestManagedStateFilePathMigratesLegacyFile(t *testing.T) {
	baseDir := t.TempDir()
	filename := "appearance_settings.json"
	legacyPath := filepath.Join(baseDir, filename)
	statePath := filepath.Join(baseDir, managedStateDirName, filename)

	if err := os.WriteFile(legacyPath, []byte(`{"mode":"color"}`), 0o644); err != nil {
		t.Fatal(err)
	}

	got := managedStateFilePathInDir(baseDir, filename)
	if got != statePath {
		t.Fatalf("managedStateFilePathInDir() = %q, want %q", got, statePath)
	}
	if _, err := os.Stat(legacyPath); !os.IsNotExist(err) {
		t.Fatalf("legacy file still exists or stat failed: %v", err)
	}
	data, err := os.ReadFile(statePath)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != `{"mode":"color"}` {
		t.Fatalf("migrated data = %q", string(data))
	}
}

func TestManagedStateFilePathPrefersExistingStateFile(t *testing.T) {
	baseDir := t.TempDir()
	filename := "audit_settings.json"
	legacyPath := filepath.Join(baseDir, filename)
	stateDir := filepath.Join(baseDir, managedStateDirName)
	statePath := filepath.Join(stateDir, filename)

	if err := os.MkdirAll(stateDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(legacyPath, []byte(`{"capacity":1}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(statePath, []byte(`{"capacity":2}`), 0o644); err != nil {
		t.Fatal(err)
	}

	got := managedStateFilePathInDir(baseDir, filename)
	if got != statePath {
		t.Fatalf("managedStateFilePathInDir() = %q, want %q", got, statePath)
	}
	if _, err := os.Stat(legacyPath); !os.IsNotExist(err) {
		t.Fatalf("legacy file still exists or stat failed: %v", err)
	}
}

func TestManagedWebInfoFilePathMigratesFromStateDir(t *testing.T) {
	baseDir := t.TempDir()
	filename := "appearance_settings.json"
	stateDir := filepath.Join(baseDir, managedStateDirName)
	legacyPath := filepath.Join(stateDir, filename)
	webinfoPath := filepath.Join(baseDir, managedWebInfoDirName, filename)

	if err := os.MkdirAll(stateDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(legacyPath, []byte(`{"mode":"upload"}`), 0o644); err != nil {
		t.Fatal(err)
	}

	got := managedWebInfoFilePathInDir(baseDir, filename)
	if got != webinfoPath {
		t.Fatalf("managedWebInfoFilePathInDir() = %q, want %q", got, webinfoPath)
	}
	if _, err := os.Stat(legacyPath); !os.IsNotExist(err) {
		t.Fatalf("legacy state file still exists or stat failed: %v", err)
	}
	if _, err := os.Stat(stateDir); !os.IsNotExist(err) {
		t.Fatalf("legacy state dir still exists or stat failed: %v", err)
	}
	data, err := os.ReadFile(webinfoPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != `{"mode":"upload"}` {
		t.Fatalf("migrated data = %q", string(data))
	}
}

func TestManagedWebInfoFilePathPrefersExistingWebInfoFile(t *testing.T) {
	baseDir := t.TempDir()
	filename := "audit_settings.json"
	stateDir := filepath.Join(baseDir, managedStateDirName)
	webinfoDir := filepath.Join(baseDir, managedWebInfoDirName)
	legacyPath := filepath.Join(stateDir, filename)
	webinfoPath := filepath.Join(webinfoDir, filename)

	if err := os.MkdirAll(stateDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(webinfoDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(legacyPath, []byte(`{"capacity":1}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(webinfoPath, []byte(`{"capacity":2}`), 0o644); err != nil {
		t.Fatal(err)
	}

	got := managedWebInfoFilePathInDir(baseDir, filename)
	if got != webinfoPath {
		t.Fatalf("managedWebInfoFilePathInDir() = %q, want %q", got, webinfoPath)
	}
	if _, err := os.Stat(legacyPath); !os.IsNotExist(err) {
		t.Fatalf("legacy state file still exists or stat failed: %v", err)
	}
	if _, err := os.Stat(stateDir); !os.IsNotExist(err) {
		t.Fatalf("legacy state dir still exists or stat failed: %v", err)
	}
}

func TestOverridesFilePathMigratesRootFileToWebInfo(t *testing.T) {
	baseDir := t.TempDir()
	legacyPath := filepath.Join(baseDir, overridesFilename)
	webinfoPath := filepath.Join(baseDir, managedWebInfoDirName, overridesFilename)

	if err := os.WriteFile(legacyPath, []byte(`{"socks5":"127.0.0.1:1080"}`), 0o644); err != nil {
		t.Fatal(err)
	}

	got := overridesFilePathInDir(baseDir)
	if got != webinfoPath {
		t.Fatalf("overridesFilePathInDir() = %q, want %q", got, webinfoPath)
	}
	if _, err := os.Stat(legacyPath); !os.IsNotExist(err) {
		t.Fatalf("legacy overrides file still exists or stat failed: %v", err)
	}
	data, err := os.ReadFile(webinfoPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != `{"socks5":"127.0.0.1:1080"}` {
		t.Fatalf("migrated overrides = %q", string(data))
	}
}

func TestUpstreamOverridesFilePathMigratesRootFileToWebInfo(t *testing.T) {
	baseDir := t.TempDir()
	legacyPath := filepath.Join(baseDir, upstreamOverridesFilename)
	webinfoPath := filepath.Join(baseDir, managedWebInfoDirName, upstreamOverridesFilename)

	if err := os.WriteFile(legacyPath, []byte(`{"foreign":[{"tag":"a","protocol":"udp","addr":"1.1.1.1"}]}`), 0o644); err != nil {
		t.Fatal(err)
	}

	got := upstreamOverridesFilePathInDir(baseDir)
	if got != webinfoPath {
		t.Fatalf("upstreamOverridesFilePathInDir() = %q, want %q", got, webinfoPath)
	}
	if _, err := os.Stat(legacyPath); !os.IsNotExist(err) {
		t.Fatalf("legacy upstream overrides file still exists or stat failed: %v", err)
	}
	data, err := os.ReadFile(webinfoPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != `{"foreign":[{"tag":"a","protocol":"udp","addr":"1.1.1.1"}]}` {
		t.Fatalf("migrated upstream overrides = %q", string(data))
	}
}

func TestManagedWebInfoFilePathKeepsNewerLegacyFile(t *testing.T) {
	baseDir := t.TempDir()
	filename := "audit_settings.json"
	stateDir := filepath.Join(baseDir, managedStateDirName)
	webinfoDir := filepath.Join(baseDir, managedWebInfoDirName)
	legacyPath := filepath.Join(stateDir, filename)
	webinfoPath := filepath.Join(webinfoDir, filename)

	if err := os.MkdirAll(stateDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(webinfoDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(webinfoPath, []byte(`{"capacity":1}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(legacyPath, []byte(`{"capacity":2}`), 0o644); err != nil {
		t.Fatal(err)
	}

	legacyInfo := mustStatFile(t, legacyPath)
	newerTime := legacyInfo.ModTime().Add(2 * time.Second)
	if err := os.Chtimes(legacyPath, newerTime, newerTime); err != nil {
		t.Fatal(err)
	}

	got := managedWebInfoFilePathInDir(baseDir, filename)
	if got != webinfoPath {
		t.Fatalf("managedWebInfoFilePathInDir() = %q, want %q", got, webinfoPath)
	}
	webinfoData, err := os.ReadFile(webinfoPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(webinfoData) != `{"capacity":2}` {
		t.Fatalf("managed file not replaced by newer legacy data: %q", string(webinfoData))
	}
	if _, err := os.Stat(legacyPath); !os.IsNotExist(err) {
		t.Fatalf("newer legacy file still exists or stat failed: %v", err)
	}
	if _, err := os.Stat(stateDir); !os.IsNotExist(err) {
		t.Fatalf("legacy state dir still exists or stat failed: %v", err)
	}
}

func mustStatFile(t *testing.T, path string) os.FileInfo {
	t.Helper()
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	return info
}
