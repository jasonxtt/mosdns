package coremain

import (
	"archive/zip"
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestParseConfigUpdatePackage(t *testing.T) {
	files := map[string][]byte{
		"config_custom.yaml": []byte("log:\n  level: warn\n"),
	}
	manifest := testConfigUpdateManifest(files)
	data := buildTestConfigPackage(t, manifest, files)

	pkg, err := parseConfigUpdatePackage(data, 1, "main-config-schema-1")
	if err != nil {
		t.Fatalf("parseConfigUpdatePackage() error = %v", err)
	}
	if pkg.manifest.ConfigSchema != 1 {
		t.Fatalf("schema = %d, want 1", pkg.manifest.ConfigSchema)
	}
	if _, err := parseConfigUpdatePackage(data, 2, "main-config-schema-2"); err == nil {
		t.Fatal("parseConfigUpdatePackage() accepted a package for another binary schema")
	}

	manifest.SHA256["config_custom.yaml"] = strings.Repeat("0", 64)
	badHash := buildTestConfigPackage(t, manifest, files)
	if _, err := parseConfigUpdatePackage(badHash, 1, "main-config-schema-1"); err == nil {
		t.Fatal("parseConfigUpdatePackage() accepted a bad file hash")
	}
}

func TestConfigUpdateTransactionRollback(t *testing.T) {
	baseDir := t.TempDir()
	mustWriteTestFile(t, filepath.Join(baseDir, "config_custom.yaml"), "old\n")
	mustWriteTestFile(t, filepath.Join(baseDir, "sub_config", "obsolete.yaml"), "old obsolete\n")

	files := map[string][]byte{
		"config_custom.yaml": []byte("new\n"),
	}
	manifest := testConfigUpdateManifest(files)
	manifest.CreateIfMissing = map[string]string{"rule/switch16.txt": "B"}
	manifest.DeleteFiles = []string{"sub_config/obsolete.yaml"}
	pkg := configUpdatePackage{
		manifest:   manifest,
		files:      files,
		packageSHA: strings.Repeat("a", 64),
	}
	statePath := configUpdateStatePath(baseDir)
	tx, err := beginConfigUpdate(baseDir, statePath, ConfigUpdateState{Format: 1}, pkg)
	if err != nil {
		t.Fatalf("beginConfigUpdate() error = %v", err)
	}
	if err := tx.apply(pkg); err != nil {
		t.Fatalf("apply() error = %v", err)
	}
	assertTestFile(t, filepath.Join(baseDir, "config_custom.yaml"), "new\n")
	assertTestFile(t, filepath.Join(baseDir, "rule", "switch16.txt"), "B")
	if _, err := os.Stat(filepath.Join(baseDir, "sub_config", "obsolete.yaml")); !os.IsNotExist(err) {
		t.Fatalf("obsolete file still exists or stat failed: %v", err)
	}

	wantErr := "forced validation failure"
	if err := tx.rollbackWithError(errors.New(wantErr)); err == nil {
		t.Fatal("rollbackWithError() returned nil")
	}
	assertTestFile(t, filepath.Join(baseDir, "config_custom.yaml"), "old\n")
	assertTestFile(t, filepath.Join(baseDir, "sub_config", "obsolete.yaml"), "old obsolete\n")
	if _, err := os.Stat(filepath.Join(baseDir, "rule", "switch16.txt")); !os.IsNotExist(err) {
		t.Fatalf("created switch file was not removed: %v", err)
	}
	state := loadConfigUpdateState(baseDir)
	if state.Status != "failed" || !strings.Contains(state.LastError, wantErr) {
		t.Fatalf("failure state = %+v", state)
	}
}

func TestRecoverInterruptedConfigUpdate(t *testing.T) {
	baseDir := t.TempDir()
	configPath := filepath.Join(baseDir, "config_custom.yaml")
	mustWriteTestFile(t, configPath, "old\n")

	files := map[string][]byte{
		"config_custom.yaml": []byte("new\n"),
	}
	pkg := configUpdatePackage{
		manifest:   testConfigUpdateManifest(files),
		files:      files,
		packageSHA: strings.Repeat("b", 64),
	}
	statePath := configUpdateStatePath(baseDir)
	tx, err := beginConfigUpdate(baseDir, statePath, ConfigUpdateState{Format: 1}, pkg)
	if err != nil {
		t.Fatalf("beginConfigUpdate() error = %v", err)
	}
	if err := tx.apply(pkg); err != nil {
		t.Fatalf("apply() error = %v", err)
	}
	assertTestFile(t, configPath, "new\n")

	state := loadConfigUpdateState(baseDir)
	if state.Status != "in_progress" {
		t.Fatalf("state status = %q, want in_progress", state.Status)
	}
	if err := recoverInterruptedConfigUpdate(baseDir, statePath, state); err != nil {
		t.Fatalf("recoverInterruptedConfigUpdate() error = %v", err)
	}
	assertTestFile(t, configPath, "old\n")
	state = loadConfigUpdateState(baseDir)
	if state.Status != "failed" || !strings.Contains(state.LastError, "未完成") {
		t.Fatalf("recovered state = %+v", state)
	}
}

func TestPrepareAndCommitRequiredConfigUpdate(t *testing.T) {
	baseDir := t.TempDir()
	configPath := filepath.Join(baseDir, "config_custom.yaml")
	mustWriteTestFile(t, configPath, "log:\n  level: warn\n")

	files := map[string][]byte{
		"config_custom.yaml": []byte("log:\n  level: error\n"),
	}
	manifest := testConfigUpdateManifest(files)
	packagePath := filepath.Join(t.TempDir(), "config_up.zip")
	if err := os.WriteFile(packagePath, buildTestConfigPackage(t, manifest, files), 0o644); err != nil {
		t.Fatal(err)
	}

	oldSchema := requiredConfigSchema
	oldPackageID := requiredConfigPackageID
	requiredConfigSchema = "1"
	requiredConfigPackageID = "main-config-schema-1"
	t.Cleanup(func() {
		requiredConfigSchema = oldSchema
		requiredConfigPackageID = oldPackageID
	})
	t.Setenv("MOSDNS_CONFIG_PACKAGE_URL", packagePath)

	tx, err := prepareRequiredConfigUpdate(baseDir, configPath)
	if err != nil {
		t.Fatalf("prepareRequiredConfigUpdate() error = %v", err)
	}
	if tx == nil {
		t.Fatal("prepareRequiredConfigUpdate() returned nil transaction")
	}
	if err := commitConfigUpdate(tx); err != nil {
		t.Fatalf("commitConfigUpdate() error = %v", err)
	}
	assertTestFile(t, configPath, "log:\n  level: error\n")
	state := loadConfigUpdateState(baseDir)
	if state.Status != "success" || state.AppliedSchema != 1 {
		t.Fatalf("committed state = %+v", state)
	}

	t.Setenv("MOSDNS_CONFIG_PACKAGE_URL", filepath.Join(t.TempDir(), "missing.zip"))
	tx, err = prepareRequiredConfigUpdate(baseDir, configPath)
	if err != nil {
		t.Fatalf("second prepareRequiredConfigUpdate() error = %v", err)
	}
	if tx != nil {
		t.Fatal("already applied schema triggered another update")
	}
}

func TestManifestPathPolicies(t *testing.T) {
	files := map[string][]byte{
		"rule/greylist.txt": []byte("example.com\n"),
	}
	manifest := testConfigUpdateManifest(files)
	manifest.ManagedFiles = []string{"rule/greylist.txt"}
	manifest.SHA256 = testFileHashes(files)
	data := buildTestConfigPackage(t, manifest, files)
	if _, err := parseConfigUpdatePackage(data, 1, "main-config-schema-1"); err == nil {
		t.Fatal("manifest was allowed to manage user rule data")
	}

	files = map[string][]byte{"config_custom.yaml": []byte("plugins: []\n")}
	manifest = testConfigUpdateManifest(files)
	manifest.CreateIfMissing = map[string]string{"webinfo/config_overrides.json": "{}"}
	data = buildTestConfigPackage(t, manifest, files)
	if _, err := parseConfigUpdatePackage(data, 1, "main-config-schema-1"); err == nil {
		t.Fatal("manifest was allowed to create protected webinfo state")
	}
}

func testConfigUpdateManifest(files map[string][]byte) configUpdateManifest {
	managed := make([]string, 0, len(files))
	for name := range files {
		managed = append(managed, name)
	}
	return configUpdateManifest{
		Format:          1,
		Channel:         "main",
		PackageID:       "main-config-schema-1",
		ConfigSchema:    1,
		ManagedFiles:    managed,
		CreateIfMissing: map[string]string{},
		DeleteFiles:     []string{},
		SHA256:          testFileHashes(files),
	}
}

func testFileHashes(files map[string][]byte) map[string]string {
	hashes := make(map[string]string, len(files))
	for name, data := range files {
		sum := sha256.Sum256(data)
		hashes[name] = hex.EncodeToString(sum[:])
	}
	return hashes
}

func buildTestConfigPackage(t *testing.T, manifest configUpdateManifest, files map[string][]byte) []byte {
	t.Helper()
	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	manifestData, err := json.Marshal(manifest)
	if err != nil {
		t.Fatal(err)
	}
	allFiles := make(map[string][]byte, len(files)+1)
	allFiles[configUpdateManifestName] = manifestData
	for name, data := range files {
		allFiles[name] = data
	}
	for name, data := range allFiles {
		w, err := zw.Create(name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := w.Write(data); err != nil {
			t.Fatal(err)
		}
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

func mustWriteTestFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func assertTestFile(t *testing.T, path, want string) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	if string(data) != want {
		t.Fatalf("%s = %q, want %q", path, data, want)
	}
}
