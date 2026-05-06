package coremain

import (
	"os"
	"path/filepath"
	"testing"
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
	legacyData, err := os.ReadFile(legacyPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(legacyData) != `{"capacity":1}` {
		t.Fatalf("legacy data changed: %q", string(legacyData))
	}
}
