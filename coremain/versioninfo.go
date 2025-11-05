package coremain

import "sync"

var (
	buildVersionMu sync.RWMutex
	buildVersion   = "dev"
)

// SetBuildVersion allows the main package to inject the actual build version string.
func SetBuildVersion(version string) {
	if version == "" {
		return
	}
	buildVersionMu.Lock()
	buildVersion = version
	buildVersionMu.Unlock()
	if GlobalUpdateManager != nil {
		GlobalUpdateManager.SetCurrentVersion(version)
	}
}

// GetBuildVersion returns the version string embedded in the running binary.
func GetBuildVersion() string {
	buildVersionMu.RLock()
	defer buildVersionMu.RUnlock()
	return buildVersion
}
