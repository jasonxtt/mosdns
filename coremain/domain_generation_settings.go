package coremain

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"sync"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"go.uber.org/zap"
)

const domainGenerationSettingsFilename = "domain_generation_settings.json"

type DomainGenerationSettings struct {
	Enabled        bool `json:"enabled"`
	RememberDirect bool `json:"remember_direct"`
	RememberProxy  bool `json:"remember_proxy"`
	NoV4           bool `json:"no_v4"`
	NoV6           bool `json:"no_v6"`
}

type domainGenerationSettingsPatch struct {
	Enabled        *bool `json:"enabled"`
	RememberDirect *bool `json:"remember_direct"`
	RememberProxy  *bool `json:"remember_proxy"`
	NoV4           *bool `json:"no_v4"`
	NoV6           *bool `json:"no_v6"`
}

var (
	domainGenerationSettingsMu    sync.RWMutex
	domainGenerationSettingsCache = defaultDomainGenerationSettings()
)

var totalControlledDomainGenerationTags = []string{
	"top_domains",
	"my_sv4list",
	"my_realiplist",
	"my_fakeiplist",
	"my_nov4list",
	"my_nov6list",
	"my_nodenov4list",
	"my_nodenov6list",
}

func defaultDomainGenerationSettings() DomainGenerationSettings {
	return DomainGenerationSettings{
		Enabled:        true,
		RememberDirect: true,
		RememberProxy:  true,
		NoV4:           true,
		NoV6:           true,
	}
}

func InitializeDomainGenerationSettings() {
	settings, err := loadDomainGenerationSettingsFromDisk()
	if err != nil {
		mlog.L().Warn("failed to load domain generation settings, using defaults",
			zap.Error(err))
		settings = defaultDomainGenerationSettings()
		if saveErr := saveDomainGenerationSettingsToDisk(settings); saveErr != nil {
			mlog.L().Warn("failed to persist default domain generation settings",
				zap.Error(saveErr))
		}
	}

	domainGenerationSettingsMu.Lock()
	domainGenerationSettingsCache = settings
	domainGenerationSettingsMu.Unlock()
}

func domainGenerationSettingsFilePath() string {
	return domainGenerationSettingsFilePathInDir(MainConfigBaseDir)
}

func domainGenerationSettingsFilePathInDir(baseDir string) string {
	return managedMigratingFilePathInDir(
		baseDir,
		managedWebInfoDirName,
		domainGenerationSettingsFilename,
		domainGenerationSettingsFilename,
		filepath.Join(managedStateDirName, domainGenerationSettingsFilename),
	)
}

func GetDomainGenerationSettings() DomainGenerationSettings {
	domainGenerationSettingsMu.RLock()
	defer domainGenerationSettingsMu.RUnlock()
	return domainGenerationSettingsCache
}

func UpdateDomainGenerationSettings(patch domainGenerationSettingsPatch) (before DomainGenerationSettings, after DomainGenerationSettings, err error) {
	domainGenerationSettingsMu.Lock()
	defer domainGenerationSettingsMu.Unlock()

	before = domainGenerationSettingsCache
	after = before

	if patch.Enabled != nil {
		after.Enabled = *patch.Enabled
	}
	if patch.RememberDirect != nil {
		after.RememberDirect = *patch.RememberDirect
	}
	if patch.RememberProxy != nil {
		after.RememberProxy = *patch.RememberProxy
	}
	if patch.NoV4 != nil {
		after.NoV4 = *patch.NoV4
	}
	if patch.NoV6 != nil {
		after.NoV6 = *patch.NoV6
	}

	if err := saveDomainGenerationSettingsToDisk(after); err != nil {
		return before, before, err
	}

	domainGenerationSettingsCache = after
	return before, after, nil
}

func DomainGenerationEnabledForTag(tag string) bool {
	settings := GetDomainGenerationSettings()
	if !settings.Enabled {
		switch tag {
		case "top_domains", "my_sv4list", "my_realiplist", "my_fakeiplist", "my_nov4list", "my_nov6list", "my_nodenov4list", "my_nodenov6list":
			return false
		}
	}

	switch tag {
	case "my_realiplist":
		return settings.RememberDirect
	case "my_fakeiplist":
		return settings.RememberProxy
	case "my_nov4list":
		return settings.NoV4
	case "my_nov6list":
		return settings.NoV6
	default:
		return true
	}
}

func DomainGenerationFlushTargets(before, after DomainGenerationSettings) []string {
	targets := make(map[string]struct{})

	if before.Enabled && !after.Enabled {
		for _, tag := range totalControlledDomainGenerationTags {
			targets[tag] = struct{}{}
		}
	}
	if before.RememberDirect && !after.RememberDirect {
		targets["my_realiplist"] = struct{}{}
	}
	if before.RememberProxy && !after.RememberProxy {
		targets["my_fakeiplist"] = struct{}{}
	}
	if before.NoV4 && !after.NoV4 {
		targets["my_nov4list"] = struct{}{}
	}
	if before.NoV6 && !after.NoV6 {
		targets["my_nov6list"] = struct{}{}
	}

	out := make([]string, 0, len(targets))
	for tag := range targets {
		out = append(out, tag)
	}
	return out
}

func loadDomainGenerationSettingsFromDisk() (DomainGenerationSettings, error) {
	path := domainGenerationSettingsFilePath()
	data, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			settings := defaultDomainGenerationSettings()
			if saveErr := saveDomainGenerationSettingsToDisk(settings); saveErr != nil {
				return settings, saveErr
			}
			return settings, nil
		}
		return DomainGenerationSettings{}, err
	}

	if len(data) == 0 {
		settings := defaultDomainGenerationSettings()
		if saveErr := saveDomainGenerationSettingsToDisk(settings); saveErr != nil {
			return settings, saveErr
		}
		return settings, nil
	}

	var patch domainGenerationSettingsPatch
	if err := json.Unmarshal(data, &patch); err != nil {
		return DomainGenerationSettings{}, err
	}

	settings := defaultDomainGenerationSettings()
	if patch.Enabled != nil {
		settings.Enabled = *patch.Enabled
	}
	if patch.RememberDirect != nil {
		settings.RememberDirect = *patch.RememberDirect
	}
	if patch.RememberProxy != nil {
		settings.RememberProxy = *patch.RememberProxy
	}
	if patch.NoV4 != nil {
		settings.NoV4 = *patch.NoV4
	}
	if patch.NoV6 != nil {
		settings.NoV6 = *patch.NoV6
	}

	return settings, nil
}

func saveDomainGenerationSettingsToDisk(settings DomainGenerationSettings) error {
	return writeJSONFileAtomic(domainGenerationSettingsFilePath(), settings, 0o644)
}
