package coremain

import (
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"go.uber.org/zap"
)

const managedStateDirName = "webinfo"

var managedStateFileMu sync.Mutex

func InitializeManagedStateFiles() {
	for _, filename := range []string{
		appearanceSettingsFilename,
		appearanceTextSettingsFile,
		appearanceButtonSettingsFile,
		auditSettingsFilename,
		overridesFilename,
		specialGroupsFilename,
		upstreamOverridesFilename,
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

	if info, err := os.Stat(legacyPath); err == nil {
		if info.IsDir() {
			mlog.L().Warn("legacy state file path is a directory, using legacy state file path",
				zap.String("legacy_path", legacyPath),
				zap.String("managed_path", statePath))
			return legacyPath
		}
		if err := os.Rename(legacyPath, statePath); err != nil {
			mlog.L().Warn("failed to migrate legacy state file, using legacy state file path",
				zap.String("legacy_path", legacyPath),
				zap.String("managed_path", statePath),
				zap.Error(err))
			return legacyPath
		}
		return statePath
	} else if err != nil && !os.IsNotExist(err) {
		mlog.L().Warn("failed to inspect legacy state file, using legacy state file path",
			zap.String("legacy_path", legacyPath),
			zap.String("managed_path", statePath),
			zap.Error(err))
		return legacyPath
	}

	return statePath
}
