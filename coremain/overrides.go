package coremain

import (
	"path/filepath"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"go.uber.org/zap"
)

const overridesFilename = "config_overrides.json"

// GlobalOverrides defines the structure of the config_overrides.json file.
type GlobalOverrides struct {
	Socks5 string `json:"socks5,omitempty"`
	ECS    string `json:"ecs,omitempty"`
}

var (
	// These variables cache the settings discovered from the original YAML config.
	discoveredSocks5 string
	discoveredECS    string
)

// DiscoverAndCacheSettings iterates through the entire config object, including includes,
// to find the first occurrence of socks5 and ecs settings.
func DiscoverAndCacheSettings(cfg *Config) {
	var socks5Found, ecsFound bool
	// Reset global vars before discovery
	discoveredSocks5 = ""
	discoveredECS = ""

	// Create a recursive function to traverse the config tree.
	var discover func(c *Config)
	discover = func(c *Config) {
		// Discover in current level's plugins
		for _, pluginConf := range c.Plugins {
			if socks5Found && ecsFound {
				return
			}
			discoverRecursive(pluginConf.Args, &socks5Found, &ecsFound)
		}
		// Recurse into included configs
		for _, includePath := range c.Include {
			if socks5Found && ecsFound {
				return
			}
			resolvedPath := includePath
			if len(c.baseDir) > 0 && !filepath.IsAbs(includePath) {
				resolvedPath = filepath.Join(c.baseDir, includePath)
			}
			// We have to re-read the sub-configs here. It's a bit inefficient but necessary.
			subCfg, _, err := loadConfig(resolvedPath)
			if err == nil {
				discover(subCfg)
			}
		}
	}

	discover(cfg)

	mlog.L().Info("discovered original settings from all config files",
		zap.String("socks5", discoveredSocks5),
		zap.String("ecs", discoveredECS))
}

// discoverRecursive correctly handles nested map[string]any and []any.
func discoverRecursive(data any, socks5Found, ecsFound *bool) {
	if data == nil || (*socks5Found && *ecsFound) {
		return
	}

	switch v := data.(type) {
	case map[string]any:
		if !*socks5Found {
			if sockVal, ok := v["socks5"]; ok {
				if socks5Str, isString := sockVal.(string); isString && socks5Str != "" {
					discoveredSocks5 = socks5Str
					*socks5Found = true
				}
			}
		}
		for _, val := range v {
			if *socks5Found && *ecsFound {
				return
			}
			discoverRecursive(val, socks5Found, ecsFound)
		}
	case []any:
		for _, item := range v {
			if *socks5Found && *ecsFound {
				return
			}
			discoverRecursive(item, socks5Found, ecsFound)
		}
	case string:
		if !*ecsFound && strings.HasPrefix(v, "ecs ") {
			parts := strings.SplitN(v, " ", 2)
			if len(parts) == 2 && parts[1] != "" {
				discoveredECS = parts[1]
				*ecsFound = true
			}
		}
	}
}

// ApplyOverrides modifies a single PluginConfig based on the loaded overrides.
func ApplyOverrides(pluginConf *PluginConfig, overrides *GlobalOverrides) {
	pluginConf.Args = applyRecursive(pluginConf.Args, overrides)
}

// applyRecursive is a generic function that traverses and modifies the config data structure.
func applyRecursive(data any, overrides *GlobalOverrides) any {
	if data == nil {
		return nil
	}

	switch v := data.(type) {
	case map[string]any:
		if overrides.Socks5 != "" {
			if _, ok := v["socks5"]; ok {
				v["socks5"] = overrides.Socks5
			}
		}
		for key, val := range v {
			v[key] = applyRecursive(val, overrides)
		}
		return v
	case []any:
		for i, item := range v {
			v[i] = applyRecursive(item, overrides)
		}
		return v
	case string:
		if overrides.ECS != "" && strings.HasPrefix(v, "ecs ") {
			return "ecs " + overrides.ECS
		}
		return v
	default:
		return data
	}
}
