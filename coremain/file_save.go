package coremain

import (
	"fmt"
	"os"
	"path/filepath"
)

type fileApplyFunc func() error

type fileValidateFunc func([]byte) error

func writeManagedFile(path string, data []byte, validate fileValidateFunc, apply fileApplyFunc, rollback fileApplyFunc) error {
	if validate != nil {
		if err := validate(data); err != nil {
			return err
		}
	}

	dir := filepath.Dir(path)
	if dir != "." && dir != "" {
		if err := os.MkdirAll(dir, 0755); err != nil {
			return err
		}
	}

	tmpPath := path + ".tmp"
	bakPath := path + ".bak"
	_ = os.Remove(tmpPath)
	_ = os.Remove(bakPath)

	if err := os.WriteFile(tmpPath, data, 0644); err != nil {
		return err
	}

	hasOriginal := false
	if _, err := os.Stat(path); err == nil {
		hasOriginal = true
		if err := os.Rename(path, bakPath); err != nil {
			_ = os.Remove(tmpPath)
			return err
		}
	} else if !os.IsNotExist(err) {
		_ = os.Remove(tmpPath)
		return err
	}

	if err := os.Rename(tmpPath, path); err != nil {
		if hasOriginal {
			_ = os.Rename(bakPath, path)
		}
		_ = os.Remove(tmpPath)
		return err
	}

	if apply != nil {
		if err := apply(); err != nil {
			_ = os.Remove(path)
			if hasOriginal {
				if restoreErr := os.Rename(bakPath, path); restoreErr != nil {
					return fmt.Errorf("apply failed: %w; restore file failed: %v", err, restoreErr)
				}
			}
			if rollback != nil {
				if rollbackErr := rollback(); rollbackErr != nil {
					return fmt.Errorf("apply failed: %w; rollback failed: %v", err, rollbackErr)
				}
			}
			return err
		}
	}

	_ = os.Remove(bakPath)
	return nil
}
