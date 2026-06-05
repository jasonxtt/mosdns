package coremain

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"go.uber.org/zap"
)

const managedStateDirName = "state"
const managedWebInfoDirName = "webinfo"

var managedStateFileMu sync.Mutex

func InitializeManagedStateFiles() {
	for _, filename := range []string{
		appearanceSettingsFilename,
		appearanceTextSettingsFile,
		appearanceButtonSettingsFile,
		auditSettingsFilename,
		webUIPortSettingsFilename,
	} {
		_ = managedWebInfoFilePath(filename)
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
	return managedMigratingFilePathInDir(
		baseDir,
		managedStateDirName,
		filename,
		filename,
	)
}

func managedWebInfoFilePath(filename string) string {
	return managedWebInfoFilePathInDir(MainConfigBaseDir, filename)
}

func managedWebInfoFilePathInDir(baseDir, filename string) string {
	return managedMigratingFilePathInDir(
		baseDir,
		managedWebInfoDirName,
		filename,
		filepath.Join(managedStateDirName, filename),
		filename,
	)
}

func managedMigratingFilePathInDir(baseDir, targetDirName, filename string, legacyRelativePaths ...string) string {
	base := configBaseDirOrDot(baseDir)
	targetDir := filepath.Join(base, targetDirName)
	targetPath := filepath.Join(targetDir, filename)

	managedStateFileMu.Lock()
	defer managedStateFileMu.Unlock()

	if err := os.MkdirAll(targetDir, 0o755); err != nil {
		fallbackPath := firstExistingLegacyPath(base, legacyRelativePaths...)
		if fallbackPath == "" {
			fallbackPath = filepath.Join(base, filename)
		}
		mlog.L().Warn("failed to create managed file directory, using fallback path",
			zap.String("dir", targetDir),
			zap.String("fallback_path", fallbackPath),
			zap.Error(err))
		return fallbackPath
	}

	if info, err := os.Stat(targetPath); err == nil {
		if info.IsDir() {
			fallbackPath := firstExistingLegacyPath(base, legacyRelativePaths...)
			if fallbackPath == "" {
				fallbackPath = filepath.Join(base, filename)
			}
			mlog.L().Warn("managed file path is a directory, using fallback path",
				zap.String("path", targetPath),
				zap.String("fallback_path", fallbackPath))
			return fallbackPath
		}
		cleanupLegacyManagedFiles(base, targetPath, info, legacyRelativePaths...)
		return targetPath
	} else if err != nil && !os.IsNotExist(err) {
		fallbackPath := firstExistingLegacyPath(base, legacyRelativePaths...)
		if fallbackPath == "" {
			fallbackPath = filepath.Join(base, filename)
		}
		mlog.L().Warn("failed to inspect managed file, using fallback path",
			zap.String("path", targetPath),
			zap.String("fallback_path", fallbackPath),
			zap.Error(err))
		return fallbackPath
	}

	for _, legacyRelPath := range legacyRelativePaths {
		legacyPath := filepath.Join(base, legacyRelPath)
		info, err := os.Stat(legacyPath)
		if err != nil {
			if !os.IsNotExist(err) {
				mlog.L().Warn("failed to inspect legacy managed file",
					zap.String("path", legacyPath),
					zap.Error(err))
			}
			continue
		}

		if info.IsDir() {
			mlog.L().Warn("legacy managed file path is a directory, skipping path",
				zap.String("legacy_path", legacyPath),
				zap.String("managed_path", targetPath))
			continue
		}

		if err := os.Rename(legacyPath, targetPath); err == nil {
			mlog.L().Info("migrated managed file",
				zap.String("from", legacyPath),
				zap.String("to", targetPath))
			if targetInfo, statErr := os.Stat(targetPath); statErr == nil && !targetInfo.IsDir() {
				cleanupLegacyManagedFiles(base, targetPath, targetInfo, legacyRelativePaths...)
			}
			return targetPath
		} else {
			mlog.L().Warn("failed to move legacy managed file, trying copy fallback",
				zap.String("from", legacyPath),
				zap.String("to", targetPath),
				zap.Error(err))
		}

		if err := copyStateFile(legacyPath, targetPath, info.Mode().Perm()); err != nil {
			mlog.L().Warn("failed to copy legacy managed file, using legacy path",
				zap.String("from", legacyPath),
				zap.String("to", targetPath),
				zap.Error(err))
			return legacyPath
		}

		if err := os.Remove(legacyPath); err != nil {
			mlog.L().Warn("copied legacy managed file but failed to remove old file",
				zap.String("legacy_path", legacyPath),
				zap.String("managed_path", targetPath),
				zap.Error(err))
		} else {
			mlog.L().Info("copied and removed legacy managed file",
				zap.String("from", legacyPath),
				zap.String("to", targetPath))
		}
		if targetInfo, statErr := os.Stat(targetPath); statErr == nil && !targetInfo.IsDir() {
			cleanupLegacyManagedFiles(base, targetPath, targetInfo, legacyRelativePaths...)
		}
		return targetPath
	}

	return targetPath
}

func cleanupLegacyManagedFiles(base, targetPath string, targetInfo os.FileInfo, legacyRelativePaths ...string) {
	for _, legacyRelPath := range legacyRelativePaths {
		legacyPath := filepath.Join(base, legacyRelPath)
		if legacyPath == targetPath {
			continue
		}

		legacyInfo, err := os.Stat(legacyPath)
		if err != nil {
			continue
		}
		if legacyInfo.IsDir() {
			continue
		}

		action, reason := classifyLegacyManagedFile(targetPath, targetInfo, legacyPath, legacyInfo)
		if action == "replace_target" {
			if err := replaceManagedFileWithLegacy(legacyPath, targetPath, legacyInfo.Mode().Perm()); err != nil {
				mlog.L().Warn("failed to replace managed file with newer legacy file",
					zap.String("legacy_path", legacyPath),
					zap.String("managed_path", targetPath),
					zap.String("reason", reason),
					zap.Error(err))
				continue
			}
			if err := os.Remove(legacyPath); err != nil {
				mlog.L().Warn("replaced managed file but failed to remove newer legacy file",
					zap.String("legacy_path", legacyPath),
					zap.String("managed_path", targetPath),
					zap.String("reason", reason),
					zap.Error(err))
			} else {
				mlog.L().Info("replaced managed file with newer legacy file",
					zap.String("legacy_path", legacyPath),
					zap.String("managed_path", targetPath),
					zap.String("reason", reason))
			}
			if refreshedInfo, statErr := os.Stat(targetPath); statErr == nil && !refreshedInfo.IsDir() {
				targetInfo = refreshedInfo
			}
			continue
		}

		if err := os.Remove(legacyPath); err != nil {
			mlog.L().Warn("failed to remove stale legacy managed file",
				zap.String("legacy_path", legacyPath),
				zap.String("managed_path", targetPath),
				zap.String("reason", reason),
				zap.Error(err))
			continue
		}

		mlog.L().Info("removed stale legacy managed file",
			zap.String("legacy_path", legacyPath),
			zap.String("managed_path", targetPath),
			zap.String("reason", reason))
	}

	tryRemoveEmptyLegacyStateDir(base)
}

func classifyLegacyManagedFile(targetPath string, targetInfo os.FileInfo, legacyPath string, legacyInfo os.FileInfo) (action string, reason string) {
	sameContent, err := filesHaveSameContent(targetPath, legacyPath)
	if err == nil && sameContent {
		return "remove_legacy", "same_content"
	}

	if targetInfo.ModTime().After(legacyInfo.ModTime()) || targetInfo.ModTime().Equal(legacyInfo.ModTime()) {
		return "remove_legacy", "target_not_older"
	}

	return "replace_target", "legacy_newer"
}

func filesHaveSameContent(a, b string) (bool, error) {
	aData, err := os.ReadFile(a)
	if err != nil {
		return false, err
	}
	bData, err := os.ReadFile(b)
	if err != nil {
		return false, err
	}
	return bytes.Equal(aData, bData), nil
}

func tryRemoveEmptyLegacyStateDir(base string) {
	stateDir := filepath.Join(base, managedStateDirName)
	entries, err := os.ReadDir(stateDir)
	if err != nil {
		return
	}
	if len(entries) > 0 {
		return
	}
	if err := os.Remove(stateDir); err != nil {
		mlog.L().Warn("failed to remove empty legacy state directory",
			zap.String("dir", stateDir),
			zap.Error(err))
		return
	}
	mlog.L().Info("removed empty legacy state directory", zap.String("dir", stateDir))
}

func firstExistingLegacyPath(base string, legacyRelativePaths ...string) string {
	for _, legacyRelPath := range legacyRelativePaths {
		legacyPath := filepath.Join(base, legacyRelPath)
		info, err := os.Stat(legacyPath)
		if err == nil && !info.IsDir() {
			return legacyPath
		}
	}
	return ""
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

func replaceManagedFileWithLegacy(from, to string, mode os.FileMode) error {
	if err := copyStateFile(from, to, mode); err != nil {
		return err
	}
	return nil
}
