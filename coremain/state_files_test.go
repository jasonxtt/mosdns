package coremain

import (
	"os"
	"path/filepath"
	"testing"
)

func TestManagedStateFilePathUsesWebinfoDir(t *testing.T) {
	baseDir := t.TempDir()
	filename := "appearance_settings.json"
	statePath := filepath.Join(baseDir, managedStateDirName, filename)

	got := managedStateFilePathInDir(baseDir, filename)
	if got != statePath {
		t.Fatalf("managedStateFilePathInDir() = %q, want %q", got, statePath)
	}

	info, err := os.Stat(filepath.Join(baseDir, managedStateDirName))
	if err != nil {
		t.Fatal(err)
	}
	if !info.IsDir() {
		t.Fatalf("managed state dir is not a directory: %s", info.Mode())
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
	legacyData, err := os.ReadFile(legacyPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(legacyData) != `{"capacity":1}` {
		t.Fatalf("legacy data changed: %q", string(legacyData))
	}
}

func TestManagedStateFilePathMigratesLegacyFile(t *testing.T) {
	baseDir := t.TempDir()
	filename := "special_upstream_groups.json"
	legacyPath := filepath.Join(baseDir, filename)
	statePath := filepath.Join(baseDir, managedStateDirName, filename)
	payload := []byte(`[{"slot":50,"name":"cmcc"}]`)

	if err := os.WriteFile(legacyPath, payload, 0o644); err != nil {
		t.Fatal(err)
	}

	got := managedStateFilePathInDir(baseDir, filename)
	if got != statePath {
		t.Fatalf("managedStateFilePathInDir() = %q, want %q", got, statePath)
	}

	stateData, err := os.ReadFile(statePath)
	if err != nil {
		t.Fatal(err)
	}
	if string(stateData) != string(payload) {
		t.Fatalf("managed state data = %q, want %q", string(stateData), string(payload))
	}
	if _, err := os.Stat(legacyPath); !os.IsNotExist(err) {
		t.Fatalf("expected legacy file removed after migration, stat err = %v", err)
	}
}

func TestOverridesPathMigratesLegacyFile(t *testing.T) {
	baseDir := t.TempDir()
	oldBaseDir := MainConfigBaseDir
	MainConfigBaseDir = baseDir
	defer func() {
		MainConfigBaseDir = oldBaseDir
	}()

	legacyPath := filepath.Join(baseDir, overridesFilename)
	statePath := filepath.Join(baseDir, managedStateDirName, overridesFilename)
	payload := []byte(`{"socks5":"127.0.0.1:1080"}`)

	if err := os.WriteFile(legacyPath, payload, 0o644); err != nil {
		t.Fatal(err)
	}

	got := overridesPath()
	if got != statePath {
		t.Fatalf("overridesPath() = %q, want %q", got, statePath)
	}

	stateData, err := os.ReadFile(statePath)
	if err != nil {
		t.Fatal(err)
	}
	if string(stateData) != string(payload) {
		t.Fatalf("managed overrides data = %q, want %q", string(stateData), string(payload))
	}
	if _, err := os.Stat(legacyPath); !os.IsNotExist(err) {
		t.Fatalf("expected legacy overrides file removed after migration, stat err = %v", err)
	}
}
