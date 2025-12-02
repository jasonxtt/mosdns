package coremain

import (
	"path/filepath"
	"strings"
	"sync/atomic"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"go.uber.org/zap"
)

const overridesFilename = "config_overrides.json"

// ReplacementRule defines a single replacement rule.
type ReplacementRule struct {
	Original string `json:"original"`
	New      string `json:"new"`
	Comment  string `json:"comment"`

	// appliedCount is an in-memory counter for successful replacements.
	// It is not exported to JSON file, but used for API response.
	appliedCount int64
}

// GlobalOverrides defines the structure of the config_overrides.json file.
type GlobalOverrides struct {
	// Original fields (Kept for backward compatibility and specific logic)
	Socks5 string `json:"socks5,omitempty"`
	ECS    string `json:"ecs,omitempty"`

	// New generic replacements
	Replacements []*ReplacementRule `json:"replacements,omitempty"`

	// lookupMap is used for fast lookup during application.
	// Key is the "Original" string.
	lookupMap map[string]*ReplacementRule
}

var (
	// These variables cache the settings discovered from the original YAML config.
	discoveredSocks5 string
	discoveredECS    string
)

// Prepare builds the lookup map for efficient execution.
// It ignores rules where Original or New is empty.
func (g *GlobalOverrides) Prepare() {
	g.lookupMap = make(map[string]*ReplacementRule)
	if g.Replacements != nil {
		for _, r := range g.Replacements {
			if r.Original == "" || r.New == "" {
				continue
			}
			g.lookupMap[r.Original] = r
			// Reset count on prepare (startup)
			r.appliedCount = 0
		}
	}
}

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
// Modified signature to include 'tag' for logging purposes.
func ApplyOverrides(tag string, pluginConf *PluginConfig, overrides *GlobalOverrides) {
	pluginConf.Args = applyRecursive(tag, pluginConf.Args, overrides)
}

// applyRecursive is a generic function that traverses and modifies the config data structure.
func applyRecursive(tag string, data any, overrides *GlobalOverrides) any {
	if data == nil {
		return nil
	}

	switch v := data.(type) {
	case map[string]any:
		// Priority 1: Original Socks5 logic
		// We modify the map in place, effectively creating a "new" value for the specific key.
		if overrides.Socks5 != "" {
			if _, ok := v["socks5"]; ok {
				v["socks5"] = overrides.Socks5
			}
		}
		// Recurse to handle nested values (and potentially apply replacements on the modified socks5 value)
		for key, val := range v {
			v[key] = applyRecursive(tag, val, overrides)
		}
		return v
	case []any:
		for i, item := range v {
			v[i] = applyRecursive(tag, item, overrides)
		}
		return v
	case string:
		// Use a variable to track the value as it passes through the logic chain.
		currentVal := v

		// Priority 1: Original ECS logic
		// If ECS override is active, update currentVal. 
		// DO NOT RETURN yet.
		if overrides.ECS != "" && strings.HasPrefix(currentVal, "ecs ") {
			currentVal = "ecs " + overrides.ECS
		}

		// Priority 2: Generic Replacement logic
		// Always execute match using currentVal (which might be original or ECS-overridden).
		if overrides.lookupMap != nil {
			if rule, ok := overrides.lookupMap[currentVal]; ok {
				atomic.AddInt64(&rule.appliedCount, 1)
				mlog.L().Info("config replacement applied",
					zap.String("plugin_tag", tag),
					zap.String("original", rule.Original),
					zap.String("new", rule.New),
					zap.String("comment", rule.Comment))
				return rule.New
			}
		}

		// Return the value (which might be original, or modified by ECS logic)
		return currentVal
	default:
		return data
	}
}

// GetCount returns the integer count of replacements.
func (r *ReplacementRule) GetCount() int64 {
	return atomic.LoadInt64(&r.appliedCount)
}
