package coremain

import (
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"go.uber.org/zap"
)

const managedStateDirName = "state"

var managedStateFileMu sync.Mutex

func InitializeManagedStateFiles() {
	for _, filename := range []string{
		appearanceSettingsFilename,
		appearanceTextSettingsFile,
		appearanceButtonSettingsFile,
		auditSettingsFilename,
		webUIPortSettingsFilename,
	} {
		_ = managedStateFilePath(filename)
	}
}

func configBaseDirOrDot(baseDir string) string {
	base := strings.TrimSpace(baseDir)
	if base == "" {
		return "."
	}
	return base
}

func managedStateFilePath(filename string) string {
	return managedStateFilePathInDir(MainConfigBaseDir, filename)
}

func managedStateFilePathInDir(baseDir, filename string) string {
	base := configBaseDirOrDot(baseDir)
	stateDir := filepath.Join(base, managedStateDirName)
	statePath := filepath.Join(stateDir, filename)
	legacyPath := filepath.Join(base, filename)

	managedStateFileMu.Lock()
	defer managedStateFileMu.Unlock()

	if err := os.MkdirAll(stateDir, 0o755); err != nil {
		mlog.L().Warn("failed to create managed state directory, using legacy state file path",
			zap.String("dir", stateDir),
			zap.String("legacy_path", legacyPath),
			zap.Error(err))
		return legacyPath
	}

	if info, err := os.Stat(statePath); err == nil {
		if info.IsDir() {
			mlog.L().Warn("managed state file path is a directory, using legacy state file path",
				zap.String("path", statePath),
				zap.String("legacy_path", legacyPath))
			return legacyPath
		}
		return statePath
	} else if err != nil && !os.IsNotExist(err) {
		mlog.L().Warn("failed to inspect managed state file, using legacy state file path",
			zap.String("path", statePath),
			zap.String("legacy_path", legacyPath),
			zap.Error(err))
		return legacyPath
	}

	info, err := os.Stat(legacyPath)
	if err != nil {
		if !os.IsNotExist(err) {
			mlog.L().Warn("failed to inspect legacy state file",
				zap.String("path", legacyPath),
				zap.Error(err))
		}
		return statePath
	}
	if info.IsDir() {
		mlog.L().Warn("legacy state file path is a directory, using managed state file path",
			zap.String("legacy_path", legacyPath),
			zap.String("state_path", statePath))
		return statePath
	}

	if err := os.Rename(legacyPath, statePath); err == nil {
		mlog.L().Info("migrated managed state file",
			zap.String("from", legacyPath),
			zap.String("to", statePath))
		return statePath
	} else {
		mlog.L().Warn("failed to move legacy state file, trying copy fallback",
			zap.String("from", legacyPath),
			zap.String("to", statePath),
			zap.Error(err))
	}

	if err := copyStateFile(legacyPath, statePath, info.Mode().Perm()); err != nil {
		mlog.L().Warn("failed to copy legacy state file, using legacy state file path",
			zap.String("from", legacyPath),
			zap.String("to", statePath),
			zap.Error(err))
		return legacyPath
	}

	if err := os.Remove(legacyPath); err != nil {
		mlog.L().Warn("copied legacy state file but failed to remove old file",
			zap.String("legacy_path", legacyPath),
			zap.String("state_path", statePath),
			zap.Error(err))
	} else {
		mlog.L().Info("copied and removed legacy state file",
			zap.String("from", legacyPath),
			zap.String("to", statePath))
	}
	return statePath
}

func copyStateFile(from, to string, mode os.FileMode) error {
	data, err := os.ReadFile(from)
	if err != nil {
		return err
	}
	if mode == 0 {
		mode = 0o644
	}
	return os.WriteFile(to, data, mode)
}
