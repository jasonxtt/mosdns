package coremain

import (
	"archive/zip"
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
)

var switchStatePathPattern = regexp.MustCompile(`^rule/switch[0-9]+\.txt$`)

const (
	configUpdateManifestName = "manifest.json"
	configUpdateStateName    = "config_update_state.json"
	configUpdateFormat       = 1
	configUpdateChannel      = "main"
	maxConfigPackageSize     = 64 << 20
)

// These values describe the external config package required by this binary.
// Keep them unchanged for binary-only releases. Bump both when config structure changes.
var (
	requiredConfigSchema    = "3"
	requiredConfigPackageID = "main-config-schema-3"
)

type configUpdateManifest struct {
	Format          int               `json:"format"`
	Channel         string            `json:"channel"`
	PackageID       string            `json:"package_id"`
	ConfigSchema    int               `json:"config_schema"`
	ManagedFiles    []string          `json:"managed_files"`
	CreateIfMissing map[string]string `json:"create_if_missing"`
	DeleteFiles     []string          `json:"delete_files"`
	SHA256          map[string]string `json:"sha256"`
}

type ConfigUpdateState struct {
	Format         int       `json:"format"`
	AppliedSchema  int       `json:"applied_schema"`
	RequiredSchema int       `json:"required_schema"`
	Status         string    `json:"status"`
	PackageID      string    `json:"package_id,omitempty"`
	PackageSHA256  string    `json:"package_sha256,omitempty"`
	BinaryVersion  string    `json:"binary_version,omitempty"`
	FilesUpdated   int       `json:"files_updated,omitempty"`
	BackupDir      string    `json:"backup_dir,omitempty"`
	Message        string    `json:"message,omitempty"`
	LastError      string    `json:"last_error,omitempty"`
	StartedAt      time.Time `json:"started_at,omitempty"`
	UpdatedAt      time.Time `json:"updated_at,omitempty"`
}

type configBackupEntry struct {
	Path    string `json:"path"`
	Existed bool   `json:"existed"`
}

type configBackupManifest struct {
	Format        int                 `json:"format"`
	BaseDir       string              `json:"base_dir"`
	PreviousState ConfigUpdateState   `json:"previous_state"`
	Entries       []configBackupEntry `json:"entries"`
}

type configUpdatePackage struct {
	manifest   configUpdateManifest
	files      map[string][]byte
	packageSHA string
}

type configUpdateTransaction struct {
	baseDir      string
	statePath    string
	backupDir    string
	manifest     configUpdateManifest
	packageSHA   string
	previous     ConfigUpdateState
	filesUpdated int
}

func requiredConfigSchemaValue() (int, error) {
	v := strings.TrimSpace(requiredConfigSchema)
	if v == "" {
		return 0, nil
	}
	schema, err := strconv.Atoi(v)
	if err != nil || schema < 0 {
		return 0, fmt.Errorf("invalid required config schema %q", requiredConfigSchema)
	}
	return schema, nil
}

func configUpdateStatePath(baseDir string) string {
	return filepath.Join(configBaseDirOrDot(baseDir), managedWebInfoDirName, configUpdateStateName)
}

func loadConfigUpdateState(baseDir string) ConfigUpdateState {
	path := configUpdateStatePath(baseDir)
	data, err := os.ReadFile(path)
	if err != nil {
		return ConfigUpdateState{Format: configUpdateFormat}
	}
	var state ConfigUpdateState
	if json.Unmarshal(data, &state) != nil {
		return ConfigUpdateState{Format: configUpdateFormat}
	}
	if state.Format == 0 {
		state.Format = configUpdateFormat
	}
	return state
}

func writeConfigUpdateState(path string, state ConfigUpdateState) error {
	state.Format = configUpdateFormat
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".config-update-state-*")
	if err != nil {
		return err
	}
	tmpPath := tmp.Name()
	defer os.Remove(tmpPath)
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmpPath, path)
}

func prepareRequiredConfigUpdate(baseDir, configPath string) (*configUpdateTransaction, error) {
	requiredSchema, err := requiredConfigSchemaValue()
	if err != nil {
		return nil, err
	}
	if requiredSchema == 0 {
		return nil, nil
	}

	statePath := configUpdateStatePath(baseDir)
	state := loadConfigUpdateState(baseDir)
	if state.Status == "in_progress" && state.BackupDir != "" {
		if err := recoverInterruptedConfigUpdate(baseDir, statePath, state); err != nil {
			return nil, fmt.Errorf("recover interrupted config update: %w", err)
		}
		state = loadConfigUpdateState(baseDir)
	}
	if state.AppliedSchema >= requiredSchema {
		return nil, nil
	}

	packageData, err := loadOfficialConfigPackage()
	if err != nil {
		recordConfigUpdateFailure(statePath, state, requiredSchema, "", "", err)
		return nil, err
	}
	pkg, err := parseConfigUpdatePackage(packageData, requiredSchema, requiredConfigPackageID)
	if err != nil {
		recordConfigUpdateFailure(statePath, state, requiredSchema, "", "", err)
		return nil, err
	}

	tx, err := beginConfigUpdate(baseDir, statePath, state, pkg)
	if err != nil {
		recordConfigUpdateFailure(statePath, state, requiredSchema, pkg.manifest.PackageID, "", err)
		return nil, err
	}
	if err := tx.apply(pkg); err != nil {
		return nil, tx.rollbackWithError(err)
	}
	if err := validateConfigTree(configPath); err != nil {
		return nil, tx.rollbackWithError(fmt.Errorf("config validation failed: %w", err))
	}
	return tx, nil
}

func commitConfigUpdate(tx *configUpdateTransaction) error {
	if tx == nil {
		return nil
	}
	now := time.Now()
	state := ConfigUpdateState{
		Format:         configUpdateFormat,
		AppliedSchema:  tx.manifest.ConfigSchema,
		RequiredSchema: tx.manifest.ConfigSchema,
		Status:         "success",
		PackageID:      tx.manifest.PackageID,
		PackageSHA256:  tx.packageSHA,
		BinaryVersion:  GetBuildVersion(),
		FilesUpdated:   tx.filesUpdated,
		BackupDir:      tx.backupDir,
		Message:        fmt.Sprintf("配置已升级到 schema %d", tx.manifest.ConfigSchema),
		StartedAt:      tx.previous.StartedAt,
		UpdatedAt:      now,
	}
	if state.StartedAt.IsZero() {
		state.StartedAt = now
	}
	if err := writeConfigUpdateState(tx.statePath, state); err != nil {
		return tx.rollbackWithError(fmt.Errorf("save config update state: %w", err))
	}
	ConfigAutoUpdatedCount = tx.filesUpdated
	return nil
}

func rollbackConfigUpdate(tx *configUpdateTransaction, cause error) error {
	if tx == nil {
		return cause
	}
	return tx.rollbackWithError(cause)
}

func recordConfigUpdateFailure(statePath string, previous ConfigUpdateState, requiredSchema int, packageID, backupDir string, cause error) {
	now := time.Now()
	state := previous
	state.Format = configUpdateFormat
	state.RequiredSchema = requiredSchema
	state.Status = "failed"
	state.PackageID = packageID
	state.PackageSHA256 = ""
	state.BinaryVersion = GetBuildVersion()
	state.FilesUpdated = 0
	state.BackupDir = backupDir
	state.Message = "配置自动升级失败，已保留原配置"
	state.LastError = cause.Error()
	state.StartedAt = now
	state.UpdatedAt = now
	_ = writeConfigUpdateState(statePath, state)
}

func loadOfficialConfigPackage() ([]byte, error) {
	source := strings.TrimSpace(os.Getenv("MOSDNS_CONFIG_PACKAGE_URL"))
	if source == "" {
		source = configPackageURL
	}
	if strings.HasPrefix(source, "file://") {
		return os.ReadFile(strings.TrimPrefix(source, "file://"))
	}
	if !strings.Contains(source, "://") {
		return os.ReadFile(source)
	}

	req, err := http.NewRequest(http.MethodGet, source, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", userAgent)
	req.Header.Set("Cache-Control", "no-cache")
	req.Header.Set("Pragma", "no-cache")
	resp, err := GlobalUpdateManager.doRequestWithFallback(req)
	if err != nil {
		return nil, fmt.Errorf("download config package: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("download config package: HTTP %s", resp.Status)
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxConfigPackageSize+1))
	if err != nil {
		return nil, err
	}
	if len(data) > maxConfigPackageSize {
		return nil, fmt.Errorf("config package exceeds %d bytes", maxConfigPackageSize)
	}
	return data, nil
}

func parseConfigUpdatePackage(data []byte, requiredSchema int, requiredPackageID string) (configUpdatePackage, error) {
	if len(data) == 0 {
		return configUpdatePackage{}, errors.New("empty config package")
	}
	zr, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return configUpdatePackage{}, fmt.Errorf("invalid config zip: %w", err)
	}
	prefix := detectSingleRootPrefix(zr.File)
	files := make(map[string][]byte, len(zr.File))
	for _, f := range zr.File {
		if f.FileInfo().IsDir() {
			continue
		}
		if !f.Mode().IsRegular() {
			return configUpdatePackage{}, fmt.Errorf("unsupported zip entry %q", f.Name)
		}
		name, err := cleanConfigRelativePath(strings.TrimPrefix(normalizeZipPath(f.Name), prefix))
		if err != nil {
			return configUpdatePackage{}, err
		}
		if _, exists := files[name]; exists {
			return configUpdatePackage{}, fmt.Errorf("duplicate zip entry %q", name)
		}
		rc, err := f.Open()
		if err != nil {
			return configUpdatePackage{}, err
		}
		content, readErr := io.ReadAll(io.LimitReader(rc, maxConfigPackageSize+1))
		closeErr := rc.Close()
		if readErr != nil {
			return configUpdatePackage{}, readErr
		}
		if closeErr != nil {
			return configUpdatePackage{}, closeErr
		}
		if len(content) > maxConfigPackageSize {
			return configUpdatePackage{}, fmt.Errorf("zip entry %q is too large", name)
		}
		files[name] = content
	}

	manifestData, ok := files[configUpdateManifestName]
	if !ok {
		return configUpdatePackage{}, errors.New("config package has no manifest.json")
	}
	var manifest configUpdateManifest
	dec := json.NewDecoder(bytes.NewReader(manifestData))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&manifest); err != nil {
		return configUpdatePackage{}, fmt.Errorf("invalid config manifest: %w", err)
	}
	if err := validateConfigUpdateManifest(manifest, files, requiredSchema, requiredPackageID); err != nil {
		return configUpdatePackage{}, err
	}
	sum := sha256.Sum256(data)
	return configUpdatePackage{
		manifest:   manifest,
		files:      files,
		packageSHA: hex.EncodeToString(sum[:]),
	}, nil
}

func validateConfigUpdateManifest(manifest configUpdateManifest, files map[string][]byte, requiredSchema int, requiredPackageID string) error {
	if manifest.Format != configUpdateFormat {
		return fmt.Errorf("unsupported config manifest format %d", manifest.Format)
	}
	if manifest.Channel != configUpdateChannel {
		return fmt.Errorf("config package channel %q does not match %q", manifest.Channel, configUpdateChannel)
	}
	if manifest.ConfigSchema != requiredSchema {
		return fmt.Errorf("config package schema %d does not match required schema %d", manifest.ConfigSchema, requiredSchema)
	}
	if manifest.PackageID != requiredPackageID {
		return fmt.Errorf("config package id %q does not match required id %q", manifest.PackageID, requiredPackageID)
	}
	if len(manifest.ManagedFiles) == 0 {
		return errors.New("config manifest has no managed files")
	}

	managed := make(map[string]struct{}, len(manifest.ManagedFiles))
	for _, rawPath := range manifest.ManagedFiles {
		p, err := cleanConfigRelativePath(rawPath)
		if err != nil {
			return err
		}
		if !isAllowedManagedConfigPath(p) {
			return fmt.Errorf("managed file %q is outside the managed config set", p)
		}
		if _, exists := managed[p]; exists {
			return fmt.Errorf("duplicate managed file %q", p)
		}
		content, exists := files[p]
		if !exists {
			return fmt.Errorf("managed file %q is missing from package", p)
		}
		wantHash := strings.ToLower(strings.TrimSpace(manifest.SHA256[p]))
		if len(wantHash) != sha256.Size*2 {
			return fmt.Errorf("managed file %q has no valid sha256", p)
		}
		sum := sha256.Sum256(content)
		if hex.EncodeToString(sum[:]) != wantHash {
			return fmt.Errorf("sha256 mismatch for %q", p)
		}
		managed[p] = struct{}{}
	}

	for rawPath := range manifest.CreateIfMissing {
		p, err := cleanConfigRelativePath(rawPath)
		if err != nil {
			return err
		}
		if !switchStatePathPattern.MatchString(p) {
			return fmt.Errorf("create-if-missing file %q is not an allowed switch state file", p)
		}
		if _, exists := managed[p]; exists {
			return fmt.Errorf("file %q cannot be both managed and create-if-missing", p)
		}
		value := strings.TrimSpace(manifest.CreateIfMissing[rawPath])
		if value != "A" && value != "B" {
			return fmt.Errorf("create-if-missing switch %q must default to A or B", p)
		}
	}
	deleted := make(map[string]struct{}, len(manifest.DeleteFiles))
	for _, rawPath := range manifest.DeleteFiles {
		p, err := cleanConfigRelativePath(rawPath)
		if err != nil {
			return err
		}
		if !strings.HasPrefix(p, "sub_config/") || !strings.HasSuffix(p, ".yaml") {
			return fmt.Errorf("delete file %q is outside sub_config", p)
		}
		if _, exists := managed[p]; exists {
			return fmt.Errorf("file %q cannot be both managed and deleted", p)
		}
		if _, exists := deleted[p]; exists {
			return fmt.Errorf("duplicate delete file %q", p)
		}
		deleted[p] = struct{}{}
	}
	for name := range files {
		if name == configUpdateManifestName {
			continue
		}
		if _, ok := managed[name]; !ok {
			return fmt.Errorf("package contains unlisted payload file %q", name)
		}
	}
	return nil
}

func isAllowedManagedConfigPath(p string) bool {
	return p == "config_custom.yaml" ||
		(strings.HasPrefix(p, "sub_config/") && strings.HasSuffix(p, ".yaml"))
}

func cleanConfigRelativePath(p string) (string, error) {
	p = normalizeZipPath(strings.TrimSpace(p))
	if p == "" || p == "." || filepath.IsAbs(p) || p == ".." || strings.HasPrefix(p, "../") {
		return "", fmt.Errorf("invalid config package path %q", p)
	}
	return filepath.ToSlash(filepath.Clean(filepath.FromSlash(p))), nil
}

func beginConfigUpdate(baseDir, statePath string, previous ConfigUpdateState, pkg configUpdatePackage) (*configUpdateTransaction, error) {
	paths := make(map[string]struct{})
	for _, p := range pkg.manifest.ManagedFiles {
		paths[p] = struct{}{}
	}
	for p := range pkg.manifest.CreateIfMissing {
		paths[p] = struct{}{}
	}
	for _, p := range pkg.manifest.DeleteFiles {
		paths[p] = struct{}{}
	}
	sortedPaths := make([]string, 0, len(paths))
	for p := range paths {
		sortedPaths = append(sortedPaths, p)
	}
	sort.Strings(sortedPaths)

	backupDir, err := createUniqueConfigBackupDir(baseDir, previous.AppliedSchema, pkg.manifest.ConfigSchema)
	if err != nil {
		return nil, err
	}
	backup := configBackupManifest{
		Format:        configUpdateFormat,
		BaseDir:       filepath.Clean(baseDir),
		PreviousState: previous,
		Entries:       make([]configBackupEntry, 0, len(sortedPaths)),
	}
	for _, rel := range sortedPaths {
		target, err := configTargetPath(baseDir, rel)
		if err != nil {
			return nil, err
		}
		entry := configBackupEntry{Path: rel}
		info, statErr := os.Stat(target)
		switch {
		case statErr == nil && info.IsDir():
			return nil, fmt.Errorf("config update target %q is a directory", rel)
		case statErr == nil:
			entry.Existed = true
			backupPath := filepath.Join(backupDir, "files", filepath.FromSlash(rel))
			if err := copyConfigFile(target, backupPath, info.Mode().Perm()); err != nil {
				return nil, fmt.Errorf("backup %q: %w", rel, err)
			}
		case os.IsNotExist(statErr):
		default:
			return nil, fmt.Errorf("inspect %q: %w", rel, statErr)
		}
		backup.Entries = append(backup.Entries, entry)
	}
	if err := writeJSONFileAtomic(filepath.Join(backupDir, "backup_manifest.json"), backup, 0o600); err != nil {
		return nil, err
	}

	now := time.Now()
	inProgress := previous
	inProgress.Format = configUpdateFormat
	inProgress.RequiredSchema = pkg.manifest.ConfigSchema
	inProgress.Status = "in_progress"
	inProgress.PackageID = pkg.manifest.PackageID
	inProgress.PackageSHA256 = pkg.packageSHA
	inProgress.BinaryVersion = GetBuildVersion()
	inProgress.BackupDir = backupDir
	inProgress.Message = "正在升级配置"
	inProgress.LastError = ""
	inProgress.StartedAt = now
	inProgress.UpdatedAt = now
	if err := writeConfigUpdateState(statePath, inProgress); err != nil {
		return nil, err
	}
	previous.StartedAt = now
	return &configUpdateTransaction{
		baseDir:    baseDir,
		statePath:  statePath,
		backupDir:  backupDir,
		manifest:   pkg.manifest,
		packageSHA: pkg.packageSHA,
		previous:   previous,
	}, nil
}

func (tx *configUpdateTransaction) apply(pkg configUpdatePackage) error {
	for _, rel := range tx.manifest.ManagedFiles {
		target, err := configTargetPath(tx.baseDir, rel)
		if err != nil {
			return err
		}
		if err := writeConfigFileAtomic(target, pkg.files[rel], 0o644); err != nil {
			return fmt.Errorf("write managed file %q: %w", rel, err)
		}
		tx.filesUpdated++
	}

	createPaths := make([]string, 0, len(tx.manifest.CreateIfMissing))
	for rel := range tx.manifest.CreateIfMissing {
		createPaths = append(createPaths, rel)
	}
	sort.Strings(createPaths)
	for _, rel := range createPaths {
		target, err := configTargetPath(tx.baseDir, rel)
		if err != nil {
			return err
		}
		if _, err := os.Stat(target); err == nil {
			continue
		} else if !os.IsNotExist(err) {
			return fmt.Errorf("inspect create-if-missing file %q: %w", rel, err)
		}
		if err := writeConfigFileAtomic(target, []byte(tx.manifest.CreateIfMissing[rel]), 0o644); err != nil {
			return fmt.Errorf("create missing file %q: %w", rel, err)
		}
		tx.filesUpdated++
	}
	for _, rel := range tx.manifest.DeleteFiles {
		target, err := configTargetPath(tx.baseDir, rel)
		if err != nil {
			return err
		}
		removeErr := os.Remove(target)
		if removeErr != nil && !os.IsNotExist(removeErr) {
			return fmt.Errorf("delete obsolete file %q: %w", rel, removeErr)
		}
		if removeErr == nil {
			tx.filesUpdated++
		}
	}
	return nil
}

func (tx *configUpdateTransaction) rollbackWithError(cause error) error {
	rollbackErr := restoreConfigBackup(tx.baseDir, tx.backupDir)
	requiredSchema := tx.manifest.ConfigSchema
	if rollbackErr != nil {
		cause = fmt.Errorf("%v; rollback failed: %w", cause, rollbackErr)
	}
	recordConfigUpdateFailure(tx.statePath, tx.previous, requiredSchema, tx.manifest.PackageID, tx.backupDir, cause)
	return cause
}

func recoverInterruptedConfigUpdate(baseDir, statePath string, state ConfigUpdateState) error {
	if err := restoreConfigBackup(baseDir, state.BackupDir); err != nil {
		return err
	}
	err := errors.New("检测到上次配置升级未完成，已自动回滚")
	recordConfigUpdateFailure(statePath, loadBackupPreviousState(state.BackupDir), state.RequiredSchema, state.PackageID, state.BackupDir, err)
	return nil
}

func loadBackupPreviousState(backupDir string) ConfigUpdateState {
	var manifest configBackupManifest
	data, err := os.ReadFile(filepath.Join(backupDir, "backup_manifest.json"))
	if err == nil && json.Unmarshal(data, &manifest) == nil {
		return manifest.PreviousState
	}
	return ConfigUpdateState{Format: configUpdateFormat}
}

func restoreConfigBackup(baseDir, backupDir string) error {
	if err := validateBackupDir(baseDir, backupDir); err != nil {
		return err
	}
	data, err := os.ReadFile(filepath.Join(backupDir, "backup_manifest.json"))
	if err != nil {
		return err
	}
	var manifest configBackupManifest
	if err := json.Unmarshal(data, &manifest); err != nil {
		return err
	}
	for _, entry := range manifest.Entries {
		target, err := configTargetPath(baseDir, entry.Path)
		if err != nil {
			return err
		}
		if !entry.Existed {
			if err := os.Remove(target); err != nil && !os.IsNotExist(err) {
				return err
			}
			continue
		}
		source := filepath.Join(backupDir, "files", filepath.FromSlash(entry.Path))
		info, err := os.Stat(source)
		if err != nil {
			return err
		}
		if err := copyConfigFileAtomic(source, target, info.Mode().Perm()); err != nil {
			return err
		}
	}
	return nil
}

func validateBackupDir(baseDir, backupDir string) error {
	root := filepath.Clean(filepath.Join(baseDir, "backup"))
	candidate := filepath.Clean(backupDir)
	if candidate == root || !strings.HasPrefix(candidate, root+string(os.PathSeparator)) {
		return fmt.Errorf("invalid config backup directory %q", backupDir)
	}
	return nil
}

func createUniqueConfigBackupDir(baseDir string, fromSchema, toSchema int) (string, error) {
	root := filepath.Join(baseDir, "backup")
	if err := os.MkdirAll(root, 0o755); err != nil {
		return "", err
	}
	prefix := fmt.Sprintf("config-update-%s-schema-%d-to-%d-", time.Now().Format("20060102-150405"), fromSchema, toSchema)
	return os.MkdirTemp(root, prefix)
}

func configTargetPath(baseDir, rel string) (string, error) {
	cleanRel, err := cleanConfigRelativePath(rel)
	if err != nil {
		return "", err
	}
	root := filepath.Clean(baseDir)
	target := filepath.Clean(filepath.Join(root, filepath.FromSlash(cleanRel)))
	if !strings.HasPrefix(target, root+string(os.PathSeparator)) {
		return "", fmt.Errorf("config path escapes target directory: %q", rel)
	}
	return target, nil
}

func writeConfigFileAtomic(path string, data []byte, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".config-update-*")
	if err != nil {
		return err
	}
	tmpPath := tmp.Name()
	defer os.Remove(tmpPath)
	if err := tmp.Chmod(mode); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmpPath, path)
}

func copyConfigFileAtomic(source, target string, mode os.FileMode) error {
	data, err := os.ReadFile(source)
	if err != nil {
		return err
	}
	return writeConfigFileAtomic(target, data, mode)
}

func copyConfigFile(source, target string, mode os.FileMode) error {
	data, err := os.ReadFile(source)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
		return err
	}
	return os.WriteFile(target, data, mode)
}

func writeJSONFileAtomic(path string, value any, mode os.FileMode) error {
	data, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return err
	}
	return writeConfigFileAtomic(path, data, mode)
}

func validateConfigTree(configPath string) error {
	seenFiles := make(map[string]struct{})
	seenTags := make(map[string]string)
	var walk func(string, int) error
	walk = func(path string, depth int) error {
		if depth > 8 {
			return errors.New("maximum include depth reached")
		}
		cfg, fileUsed, err := loadConfig(path)
		if err != nil {
			return err
		}
		abs, err := filepath.Abs(fileUsed)
		if err != nil {
			abs = filepath.Clean(fileUsed)
		}
		if _, exists := seenFiles[abs]; exists {
			return fmt.Errorf("config include cycle or duplicate: %s", fileUsed)
		}
		seenFiles[abs] = struct{}{}
		for _, includePath := range cfg.Include {
			resolved := includePath
			if cfg.baseDir != "" && !filepath.IsAbs(includePath) {
				resolved = filepath.Join(cfg.baseDir, includePath)
			}
			if err := walk(resolved, depth+1); err != nil {
				return fmt.Errorf("include %s: %w", includePath, err)
			}
		}
		for i, plugin := range cfg.Plugins {
			typeInfo, ok := GetPluginType(plugin.Type)
			if !ok {
				return fmt.Errorf("%s plugin #%d uses unknown type %q", fileUsed, i, plugin.Type)
			}
			args := typeInfo.NewArgs()
			if reflect.TypeOf(plugin.Args) != reflect.TypeOf(args) {
				if err := utils.WeakDecode(plugin.Args, args); err != nil {
					return fmt.Errorf("%s plugin #%d %q args: %w", fileUsed, i, plugin.Tag, err)
				}
			}
			if plugin.Tag != "" {
				if previousFile, exists := seenTags[plugin.Tag]; exists {
					return fmt.Errorf("duplicate plugin tag %q in %s and %s", plugin.Tag, previousFile, fileUsed)
				}
				seenTags[plugin.Tag] = fileUsed
			}
		}
		return nil
	}
	return walk(configPath, 0)
}
