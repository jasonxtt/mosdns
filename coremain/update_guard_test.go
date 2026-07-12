package coremain

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
)

func TestWaitForCandidateHealth(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ready":true,"version":"v0.7.0","applied_config_schema":3}`))
	}))
	defer server.Close()
	tx := updateTransaction{
		TargetVersion:        "v0.7.0",
		RequiredConfigSchema: 3,
		HealthURL:            server.URL,
	}
	if err := waitForCandidateHealth(&tx, make(chan error)); err != nil {
		t.Fatalf("healthy candidate rejected: %v", err)
	}
}

func TestWaitForCandidateHealthReturnsCandidateExit(t *testing.T) {
	exitCh := make(chan error, 1)
	exitCh <- errors.New("candidate exited")
	tx := updateTransaction{HealthURL: "http://127.0.0.1:1"}
	if err := waitForCandidateHealth(&tx, exitCh); err == nil || err.Error() != "candidate exited" {
		t.Fatalf("unexpected exit error: %v", err)
	}
}

func TestRestoreConfigAfterFailedCandidate(t *testing.T) {
	baseDir := t.TempDir()
	target := filepath.Join(baseDir, "config_custom.yaml")
	if err := os.WriteFile(target, []byte("new config"), 0o644); err != nil {
		t.Fatal(err)
	}

	backupDir := filepath.Join(baseDir, "backup", "config-update-test")
	backupFile := filepath.Join(backupDir, "files", "config_custom.yaml")
	if err := os.MkdirAll(filepath.Dir(backupFile), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(backupFile, []byte("old config"), 0o644); err != nil {
		t.Fatal(err)
	}
	previous := ConfigUpdateState{
		Format:        configUpdateFormat,
		AppliedSchema: 3,
		Status:        "success",
		PackageID:     "main-config-schema-3",
	}
	backup := configBackupManifest{
		Format:        configUpdateFormat,
		BaseDir:       baseDir,
		PreviousState: previous,
		Entries: []configBackupEntry{
			{Path: "config_custom.yaml", Existed: true},
		},
	}
	if err := writeJSONFileAtomic(filepath.Join(backupDir, "backup_manifest.json"), backup, 0o600); err != nil {
		t.Fatal(err)
	}
	state := ConfigUpdateState{
		Format:         configUpdateFormat,
		AppliedSchema:  4,
		RequiredSchema: 4,
		Status:         "success",
		PackageID:      "main-config-schema-4",
		BackupDir:      backupDir,
	}
	if err := writeConfigUpdateState(configUpdateStatePath(baseDir), state); err != nil {
		t.Fatal(err)
	}

	tx := updateTransaction{
		ConfigPackagePath: "staged-config.zip",
		ConfigPackageID:   "main-config-schema-4",
		ConfigBaseDir:     baseDir,
	}
	if err := restoreConfigAfterFailedCandidate(&tx); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "old config" {
		t.Fatalf("config was not restored: %q", data)
	}
	restoredState := loadConfigUpdateState(baseDir)
	if restoredState.AppliedSchema != previous.AppliedSchema || restoredState.PackageID != previous.PackageID {
		t.Fatalf("config state was not restored: %+v", restoredState)
	}
}
