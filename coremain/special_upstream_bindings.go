package coremain

import (
	"encoding/json"
	"fmt"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"

	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
)

const (
	specialSourceKindGroup    = "group"
	specialSourceKindUpstream = "upstream"
)

type configuredAliAPIArgs struct {
	Upstreams       []configuredAliAPIUpstream `yaml:"upstreams"`
	Concurrent      int                        `yaml:"concurrent"`
	AccountID       string                     `yaml:"account_id"`
	AccessKeyID     string                     `yaml:"access_key_id"`
	AccessKeySecret string                     `yaml:"access_key_secret"`
	ServerAddr      string                     `yaml:"server_addr"`
	EcsClientIP     string                     `yaml:"ecs_client_ip"`
	EcsClientMask   uint8                      `yaml:"ecs_client_mask"`
	Socks5          string                     `yaml:"socks5"`
	SoMark          int                        `yaml:"so_mark"`
	BindToDevice    string                     `yaml:"bind_to_device"`
	Bootstrap       string                     `yaml:"bootstrap"`
	BootstrapVer    int                        `yaml:"bootstrap_version"`
}

type configuredAliAPIUpstream struct {
	Tag                  string `yaml:"tag"`
	Addr                 string `yaml:"addr"`
	DialAddr             string `yaml:"dial_addr"`
	IdleTimeout          int    `yaml:"idle_timeout"`
	UpstreamQueryTimeout int    `yaml:"upstream_query_timeout"`
	Type                 string `yaml:"type"`
	MaxConns             int    `yaml:"max_conns"`
	EnablePipeline       bool   `yaml:"enable_pipeline"`
	EnableHTTP3          bool   `yaml:"enable_http3"`
	InsecureSkipVerify   bool   `yaml:"insecure_skip_verify"`
	Socks5               string `yaml:"socks5"`
	SoMark               int    `yaml:"so_mark"`
	BindToDevice         string `yaml:"bind_to_device"`
	Bootstrap            string `yaml:"bootstrap"`
	BootstrapVer         int    `yaml:"bootstrap_version"`
}

type configuredUpstreamGroup struct {
	PluginTag string
	Args      configuredAliAPIArgs
	Upstreams []UpstreamOverrideConfig
}

type specialGroupResolution struct {
	Effective []UpstreamOverrideConfig
	Warnings  []string
}

type UpstreamSourceCatalogGroup struct {
	PluginTag string                       `json:"plugin_tag"`
	Upstreams []UpstreamSourceCatalogEntry `json:"upstreams"`
}

type UpstreamSourceCatalogEntry struct {
	Tag      string `json:"tag"`
	Protocol string `json:"protocol"`
	Address  string `json:"address"`
	Enabled  bool   `json:"enabled"`
}

var configuredUpstreamGroupsState = struct {
	sync.RWMutex
	groups map[string]configuredUpstreamGroup
}{
	groups: make(map[string]configuredUpstreamGroup),
}

func prepareConfiguredUpstreamGroups(cfg *Config, overrides *GlobalOverrides) error {
	groups := make(map[string]configuredUpstreamGroup)
	visited := make(map[string]struct{})
	if err := collectConfiguredUpstreamGroups(cfg, overrides, groups, visited); err != nil {
		return err
	}

	configuredUpstreamGroupsState.Lock()
	configuredUpstreamGroupsState.groups = groups
	configuredUpstreamGroupsState.Unlock()
	return nil
}

func collectConfiguredUpstreamGroups(cfg *Config, overrides *GlobalOverrides, groups map[string]configuredUpstreamGroup, visited map[string]struct{}) error {
	if cfg == nil {
		return nil
	}

	for _, includePath := range cfg.Include {
		resolvedPath := includePath
		if cfg.baseDir != "" && !filepath.IsAbs(includePath) {
			resolvedPath = filepath.Join(cfg.baseDir, includePath)
		}
		absPath, err := filepath.Abs(resolvedPath)
		if err == nil {
			resolvedPath = absPath
		}
		if _, ok := visited[resolvedPath]; ok {
			continue
		}
		visited[resolvedPath] = struct{}{}

		subCfg, _, err := loadConfig(resolvedPath)
		if err != nil {
			return fmt.Errorf("failed to load upstream source config %s: %w", resolvedPath, err)
		}
		if err := collectConfiguredUpstreamGroups(subCfg, overrides, groups, visited); err != nil {
			return err
		}
	}

	for _, plugin := range cfg.Plugins {
		pluginTag := strings.TrimSpace(plugin.Tag)
		if plugin.Type != "aliapi" || pluginTag == "" {
			continue
		}

		argsValue, err := cloneConfigValue(plugin.Args)
		if err != nil {
			return fmt.Errorf("failed to clone upstream source %s args: %w", plugin.Tag, err)
		}
		pluginCopy := plugin
		pluginCopy.Tag = pluginTag
		pluginCopy.Args = argsValue
		if overrides != nil {
			ApplyOverrides(pluginCopy.Tag, &pluginCopy, overrides)
		}

		var args configuredAliAPIArgs
		if err := utils.WeakDecode(pluginCopy.Args, &args); err != nil {
			return fmt.Errorf("failed to decode upstream source %s: %w", plugin.Tag, err)
		}

		entries := make([]UpstreamOverrideConfig, 0, len(args.Upstreams))
		for _, item := range args.Upstreams {
			entry := configuredUpstreamEntry(pluginTag, args, item)
			entries = append(entries, entry)
		}
		groups[pluginTag] = configuredUpstreamGroup{
			PluginTag: pluginTag,
			Args:      args,
			Upstreams: entries,
		}
	}
	return nil
}

func cloneConfigValue(value any) (any, error) {
	if value == nil {
		return nil, nil
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return nil, err
	}
	var cloned any
	if err := json.Unmarshal(raw, &cloned); err != nil {
		return nil, err
	}
	return cloned, nil
}

func configuredUpstreamEntry(pluginTag string, args configuredAliAPIArgs, item configuredAliAPIUpstream) UpstreamOverrideConfig {
	protocol := normalizeUpstreamProtocol(item.Type)
	if protocol == "" || protocol == "dns" {
		protocol = inferConfiguredUpstreamProtocol(item.Addr)
	}
	entry := UpstreamOverrideConfig{
		Tag:                  strings.TrimSpace(item.Tag),
		Enabled:              true,
		Protocol:             protocol,
		Addr:                 strings.TrimSpace(item.Addr),
		DialAddr:             strings.TrimSpace(item.DialAddr),
		IdleTimeout:          item.IdleTimeout,
		UpstreamQueryTimeout: item.UpstreamQueryTimeout,
		EnablePipeline:       item.EnablePipeline,
		EnableHTTP3:          item.EnableHTTP3,
		InsecureSkipVerify:   item.InsecureSkipVerify,
		Socks5:               strings.TrimSpace(item.Socks5),
		SoMark:               item.SoMark,
		BindToDevice:         strings.TrimSpace(item.BindToDevice),
		Bootstrap:            strings.TrimSpace(item.Bootstrap),
		BootstrapVer:         item.BootstrapVer,
	}
	if entry.Socks5 == "" {
		entry.Socks5 = strings.TrimSpace(args.Socks5)
	}
	if entry.SoMark == 0 {
		entry.SoMark = args.SoMark
	}
	if entry.BindToDevice == "" {
		entry.BindToDevice = strings.TrimSpace(args.BindToDevice)
	}
	if entry.Bootstrap == "" {
		entry.Bootstrap = strings.TrimSpace(args.Bootstrap)
	}
	if entry.BootstrapVer == 0 {
		entry.BootstrapVer = args.BootstrapVer
	}
	if protocol == "aliapi" {
		entry.AccountID = strings.TrimSpace(args.AccountID)
		entry.AccessKeyID = strings.TrimSpace(args.AccessKeyID)
		entry.AccessKeySecret = strings.TrimSpace(args.AccessKeySecret)
		entry.ServerAddr = strings.TrimSpace(args.ServerAddr)
		entry.EcsClientIP = strings.TrimSpace(args.EcsClientIP)
		entry.EcsClientMask = args.EcsClientMask
	}
	entry.UseSocksProxy = boolPtr(inferUseSocksProxy(pluginTag, entry))
	return entry
}

func inferConfiguredUpstreamProtocol(addr string) string {
	raw := strings.TrimSpace(addr)
	if idx := strings.Index(raw, "://"); idx > 0 {
		return normalizeUpstreamProtocol(raw[:idx])
	}
	return "udp"
}

func snapshotConfiguredUpstreamGroups() map[string]configuredUpstreamGroup {
	configuredUpstreamGroupsState.RLock()
	defer configuredUpstreamGroupsState.RUnlock()

	groups := make(map[string]configuredUpstreamGroup, len(configuredUpstreamGroupsState.groups))
	for tag, group := range configuredUpstreamGroupsState.groups {
		groups[tag] = configuredUpstreamGroup{
			PluginTag: group.PluginTag,
			Args:      group.Args,
			Upstreams: cloneUpstreamEntries(group.Upstreams),
		}
	}
	return groups
}

func rawUpstreamOverridesSnapshot() GlobalUpstreamOverrides {
	if upstreamOverrides == nil {
		_ = loadUpstreamOverrides()
	}
	upstreamOverridesLock.RLock()
	defer upstreamOverridesLock.RUnlock()

	return cloneGlobalUpstreamOverrides(upstreamOverrides)
}

func cloneGlobalUpstreamOverrides(source GlobalUpstreamOverrides) GlobalUpstreamOverrides {
	cloned := make(GlobalUpstreamOverrides, len(source))
	for tag, entries := range source {
		cloned[tag] = cloneUpstreamEntries(entries)
	}
	return cloned
}

func cloneUpstreamEntries(entries []UpstreamOverrideConfig) []UpstreamOverrideConfig {
	if entries == nil {
		return nil
	}
	cloned := make([]UpstreamOverrideConfig, len(entries))
	copy(cloned, entries)
	for i := range cloned {
		if entries[i].UseSocksProxy != nil {
			value := *entries[i].UseSocksProxy
			cloned[i].UseSocksProxy = &value
		}
	}
	return cloned
}

func effectiveConfiguredUpstreamGroup(pluginTag string, rawOverrides GlobalUpstreamOverrides, configured map[string]configuredUpstreamGroup) ([]UpstreamOverrideConfig, bool) {
	pluginTag = strings.TrimSpace(pluginTag)
	group, configuredExists := configured[pluginTag]
	if !configuredExists {
		return nil, false
	}
	stored, hasStored := rawOverrides[pluginTag]
	if hasStored && hasEnabledUpstream(stored) {
		return normalizeSourceEntries(pluginTag, stored), true
	}
	return cloneUpstreamEntries(group.Upstreams), true
}

func hasEnabledUpstream(entries []UpstreamOverrideConfig) bool {
	for _, entry := range entries {
		if entry.Enabled {
			return true
		}
	}
	return false
}

func normalizeSourceEntries(pluginTag string, entries []UpstreamOverrideConfig) []UpstreamOverrideConfig {
	cloned := cloneUpstreamEntries(entries)
	for i := range cloned {
		cloned[i].Protocol = normalizeUpstreamProtocol(cloned[i].Protocol)
		if cloned[i].Protocol == "" {
			cloned[i].Protocol = inferConfiguredUpstreamProtocol(cloned[i].Addr)
		}
		cloned[i].Tag = strings.TrimSpace(cloned[i].Tag)
		cloned[i].Addr = strings.TrimSpace(cloned[i].Addr)
		cloned[i].UseSocksProxy = boolPtr(inferUseSocksProxy(pluginTag, cloned[i]))
	}
	return cloned
}

func resolveSpecialGroup(g SpecialGroup) specialGroupResolution {
	return resolveSpecialGroupWithState(g, rawUpstreamOverridesSnapshot(), snapshotConfiguredUpstreamGroups())
}

func resolveSpecialGroupWithState(g SpecialGroup, rawOverrides GlobalUpstreamOverrides, configured map[string]configuredUpstreamGroup) specialGroupResolution {
	resolution := specialGroupResolution{}
	usedRuntimeTags := make(map[string]struct{})
	seenSourceEntries := make(map[string]struct{})
	groupReferences := make(map[string]struct{})

	for _, source := range g.UpstreamSources {
		if source.Kind == specialSourceKindGroup {
			groupReferences[source.PluginTag] = struct{}{}
		}
	}

	for _, owned := range g.OwnedUpstreams {
		if !owned.Enabled {
			continue
		}
		entry := owned
		entry.Tag = strings.TrimSpace(entry.Tag)
		if entry.Tag == "" {
			resolution.Warnings = append(resolution.Warnings, "专属自有上游缺少上游标识")
			continue
		}
		if _, exists := usedRuntimeTags[entry.Tag]; exists {
			resolution.Warnings = append(resolution.Warnings, fmt.Sprintf("专属自有上游标识重复：%s", entry.Tag))
			continue
		}
		usedRuntimeTags[entry.Tag] = struct{}{}
		resolution.Effective = append(resolution.Effective, entry)
	}

	for sourceIndex, source := range g.UpstreamSources {
		pluginTag := strings.TrimSpace(source.PluginTag)
		if pluginTag == "" {
			continue
		}
		entries, exists := effectiveConfiguredUpstreamGroup(pluginTag, rawOverrides, configured)
		if !exists {
			resolution.Warnings = append(resolution.Warnings, fmt.Sprintf("源上游组不存在：%s", pluginTag))
			continue
		}

		if source.Kind == specialSourceKindGroup {
			activeCount := 0
			for entryIndex, entry := range entries {
				if !entry.Enabled {
					continue
				}
				key := fmt.Sprintf("%s|%s|%d", pluginTag, entry.Tag, entryIndex)
				if entry.Tag != "" {
					key = pluginTag + "|" + entry.Tag
				}
				if _, seen := seenSourceEntries[key]; seen {
					continue
				}
				seenSourceEntries[key] = struct{}{}
				entry.Tag = specialRuntimeUpstreamTag(entry.Tag, pluginTag, sourceIndex+entryIndex, usedRuntimeTags)
				usedRuntimeTags[entry.Tag] = struct{}{}
				resolution.Effective = append(resolution.Effective, entry)
				activeCount++
			}
			if activeCount == 0 {
				resolution.Warnings = append(resolution.Warnings, fmt.Sprintf("源上游组没有启用中的上游：%s", pluginTag))
			}
			continue
		}

		if source.Kind != specialSourceKindUpstream {
			resolution.Warnings = append(resolution.Warnings, fmt.Sprintf("未知的专属上游引用类型：%s", source.Kind))
			continue
		}
		if _, covered := groupReferences[pluginTag]; covered {
			continue
		}

		upstreamTag := strings.TrimSpace(source.UpstreamTag)
		found := false
		for entryIndex, entry := range entries {
			if entry.Tag != upstreamTag {
				continue
			}
			found = true
			if !entry.Enabled {
				resolution.Warnings = append(resolution.Warnings, fmt.Sprintf("源上游已禁用：%s / %s", pluginTag, upstreamTag))
				break
			}
			key := pluginTag + "|" + upstreamTag
			if _, seen := seenSourceEntries[key]; seen {
				break
			}
			seenSourceEntries[key] = struct{}{}
			entry.Tag = specialRuntimeUpstreamTag(entry.Tag, pluginTag, sourceIndex+entryIndex, usedRuntimeTags)
			usedRuntimeTags[entry.Tag] = struct{}{}
			resolution.Effective = append(resolution.Effective, entry)
			break
		}
		if !found {
			resolution.Warnings = append(resolution.Warnings, fmt.Sprintf("源上游不存在：%s / %s", pluginTag, upstreamTag))
		}
	}
	if len(resolution.Effective) == 0 {
		resolution.Warnings = append(resolution.Warnings, "没有可用的有效上游，本专属分流组当前不参与分流")
	}

	return resolution
}

func specialRuntimeUpstreamTag(originalTag, pluginTag string, index int, used map[string]struct{}) string {
	originalTag = strings.TrimSpace(originalTag)
	if originalTag != "" {
		if _, exists := used[originalTag]; !exists {
			return originalTag
		}
	}

	base := "ref_" + sanitizeRuntimeTagPart(pluginTag)
	if originalTag != "" {
		base += "_" + sanitizeRuntimeTagPart(originalTag)
	} else {
		base += fmt.Sprintf("_%d", index)
	}
	candidate := base
	for suffix := 2; ; suffix++ {
		if _, exists := used[candidate]; !exists {
			return candidate
		}
		candidate = fmt.Sprintf("%s_%d", base, suffix)
	}
}

func sanitizeRuntimeTagPart(value string) string {
	var b strings.Builder
	for _, r := range value {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9':
			b.WriteRune(r)
		default:
			b.WriteByte('_')
		}
	}
	result := strings.Trim(b.String(), "_")
	if result == "" {
		return "source"
	}
	return result
}

func normalizeSpecialUpstreamSources(sources []SpecialUpstreamSource) ([]SpecialUpstreamSource, error) {
	if sources == nil {
		return []SpecialUpstreamSource{}, nil
	}
	seen := make(map[string]struct{}, len(sources))
	normalized := make([]SpecialUpstreamSource, 0, len(sources))
	for _, source := range sources {
		source.Kind = strings.ToLower(strings.TrimSpace(source.Kind))
		source.PluginTag = strings.TrimSpace(source.PluginTag)
		source.UpstreamTag = strings.TrimSpace(source.UpstreamTag)
		if source.Kind != specialSourceKindGroup && source.Kind != specialSourceKindUpstream {
			return nil, fmt.Errorf("未知的专属上游引用类型：%s", source.Kind)
		}
		if source.PluginTag == "" {
			return nil, fmt.Errorf("专属上游引用的源组不能为空")
		}
		if isSpecialGroupPluginTag(source.PluginTag) {
			return nil, fmt.Errorf("专属分流组不能引用其他专属分流组")
		}
		if source.Kind == specialSourceKindUpstream && source.UpstreamTag == "" {
			return nil, fmt.Errorf("单个专属上游引用必须包含上游标识")
		}
		key := source.Kind + "|" + source.PluginTag + "|" + source.UpstreamTag
		if _, exists := seen[key]; exists {
			continue
		}
		seen[key] = struct{}{}
		normalized = append(normalized, source)
	}
	return normalized, nil
}

func isSpecialGroupPluginTag(pluginTag string) bool {
	pluginTag = strings.TrimSpace(pluginTag)
	if isSpecialUpstreamTag(pluginTag) {
		return true
	}
	for _, prefix := range []string{"special_route_", "special_manual_"} {
		if !strings.HasPrefix(pluginTag, prefix) {
			continue
		}
		slot, err := strconv.Atoi(strings.TrimPrefix(pluginTag, prefix))
		return err == nil && isValidSpecialSlot(slot)
	}
	return pluginTag == "special_upstream_matcher" || pluginTag == "sequence_special"
}

func validateOwnedSpecialUpstreams(entries []UpstreamOverrideConfig) error {
	if err := validateUpstreamOverrideEntries(specialUpstreamPluginTag(specialSlotMin), entries); err != nil {
		return err
	}
	seen := make(map[string]struct{}, len(entries))
	for i, entry := range entries {
		tag := strings.TrimSpace(entry.Tag)
		if tag == "" {
			return fmt.Errorf("专属自有上游 #%d 的上游标识不能为空", i+1)
		}
		if _, exists := seen[tag]; exists {
			return fmt.Errorf("专属自有上游标识重复：%s", tag)
		}
		seen[tag] = struct{}{}
	}
	return nil
}

func normalizeOwnedSpecialUpstreams(entries []UpstreamOverrideConfig) []UpstreamOverrideConfig {
	normalized := cloneUpstreamEntries(entries)
	for i := range normalized {
		normalized[i].Tag = strings.TrimSpace(normalized[i].Tag)
		normalized[i].Protocol = normalizeUpstreamProtocol(normalized[i].Protocol)
		if normalized[i].UseSocksProxy == nil {
			normalized[i].UseSocksProxy = boolPtr(inferUseSocksProxy(specialUpstreamPluginTag(specialSlotMin), normalized[i]))
		}
	}
	return normalized
}

func specialGroupRuntimeEnabled(g SpecialGroup) bool {
	return len(resolveSpecialGroup(g).Effective) > 0
}

func refreshSpecialGroupRuntimeState(groups []SpecialGroup) (map[int]specialGroupResolution, error) {
	rawOverrides := rawUpstreamOverridesSnapshot()
	configured := snapshotConfiguredUpstreamGroups()
	resolutions := make(map[int]specialGroupResolution, len(groups))
	newOverrides := cloneGlobalUpstreamOverrides(rawOverrides)
	for _, group := range groups {
		resolution := resolveSpecialGroupWithState(group, rawOverrides, configured)
		resolutions[group.Slot] = resolution
		newOverrides[specialUpstreamPluginTag(group.Slot)] = cloneUpstreamEntries(resolution.Effective)
	}

	upstreamOverridesLock.Lock()
	oldOverrides := upstreamOverrides
	upstreamOverrides = newOverrides
	err := saveUpstreamOverrides()
	if err != nil {
		upstreamOverrides = oldOverrides
	}
	upstreamOverridesLock.Unlock()
	if err != nil {
		return nil, err
	}
	return resolutions, nil
}

func specialGroupDependsOnPlugin(group SpecialGroup, pluginTag string) bool {
	pluginTag = strings.TrimSpace(pluginTag)
	if pluginTag == "" {
		return false
	}
	for _, source := range group.UpstreamSources {
		if strings.TrimSpace(source.PluginTag) == pluginTag {
			return true
		}
	}
	return false
}

func specialGroupActiveMap(groups []SpecialGroup, resolutions map[int]specialGroupResolution) map[int]bool {
	active := make(map[int]bool, len(groups))
	for _, group := range groups {
		active[group.Slot] = len(resolutions[group.Slot].Effective) > 0
	}
	return active
}

func snapshotSpecialGroups() []SpecialGroup {
	specialGroupsLock.RLock()
	loaded := specialGroups != nil
	specialGroupsLock.RUnlock()
	if !loaded {
		_ = loadSpecialGroups()
	}
	specialGroupsLock.RLock()
	defer specialGroupsLock.RUnlock()
	return cloneSpecialGroups(specialGroups)
}

func snapshotSpecialGroupActiveState() map[int]bool {
	groups := snapshotSpecialGroups()
	if len(groups) == 0 {
		return map[int]bool{}
	}
	rawOverrides := rawUpstreamOverridesSnapshot()
	configured := snapshotConfiguredUpstreamGroups()
	resolutions := make(map[int]specialGroupResolution, len(groups))
	for _, group := range groups {
		resolutions[group.Slot] = resolveSpecialGroupWithState(group, rawOverrides, configured)
	}
	return specialGroupActiveMap(groups, resolutions)
}

func refreshDependentSpecialGroups(pluginTag string, oldActive map[int]bool) error {
	groups := snapshotSpecialGroups()
	dependent := make([]SpecialGroup, 0)
	for _, group := range groups {
		if specialGroupDependsOnPlugin(group, pluginTag) {
			dependent = append(dependent, group)
		}
	}
	if len(dependent) == 0 {
		return nil
	}

	resolutions, err := refreshSpecialGroupRuntimeState(groups)
	if err != nil {
		return err
	}
	if err := writeSpecialGroupsConfig(mainConfigDir(), groups); err != nil {
		return err
	}

	needsRestart := false
	for _, group := range dependent {
		newActive := len(resolutions[group.Slot].Effective) > 0
		if oldActive[group.Slot] != newActive {
			needsRestart = true
			continue
		}
		if !newActive {
			continue
		}
		if upstreamAPIHost == nil {
			needsRestart = true
			continue
		}
		plugin := upstreamAPIHost.GetPlugin(specialUpstreamPluginTag(group.Slot))
		reloader, ok := plugin.(upstreamReloader)
		if !ok {
			needsRestart = true
			continue
		}
		if err := reloader.ReloadFromOverrides(); err != nil {
			return fmt.Errorf("failed to reload special group %d: %w", group.Slot, err)
		}
		if err := flushDedicatedCaches(group.Slot); err != nil {
			return fmt.Errorf("failed to flush special group %d caches: %w", group.Slot, err)
		}
	}
	if needsRestart {
		_ = scheduleSelfRestart(GetCurrentMosdns(), specialGroupRestartDelayMs)
	}
	return nil
}

func sortConfiguredUpstreamGroups(groups map[string]configuredUpstreamGroup) []configuredUpstreamGroup {
	ordered := make([]configuredUpstreamGroup, 0, len(groups))
	for _, group := range groups {
		if isSpecialGroupPluginTag(group.PluginTag) {
			continue
		}
		ordered = append(ordered, configuredUpstreamGroup{
			PluginTag: group.PluginTag,
			Args:      group.Args,
			Upstreams: cloneUpstreamEntries(group.Upstreams),
		})
	}
	sort.Slice(ordered, func(i, j int) bool { return ordered[i].PluginTag < ordered[j].PluginTag })
	return ordered
}

func buildUpstreamSourceCatalog() []UpstreamSourceCatalogGroup {
	configured := snapshotConfiguredUpstreamGroups()
	rawOverrides := rawUpstreamOverridesSnapshot()
	allTags := make(map[string]struct{}, len(configured)+len(rawOverrides))
	for tag := range configured {
		allTags[tag] = struct{}{}
	}
	for tag := range rawOverrides {
		allTags[tag] = struct{}{}
	}

	groups := make(map[string]configuredUpstreamGroup, len(allTags))
	for tag := range allTags {
		if isSpecialGroupPluginTag(tag) {
			continue
		}
		entries, ok := effectiveConfiguredUpstreamGroup(tag, rawOverrides, configured)
		if !ok {
			continue
		}
		groups[tag] = configuredUpstreamGroup{PluginTag: tag, Upstreams: entries}
	}

	ordered := sortConfiguredUpstreamGroups(groups)
	result := make([]UpstreamSourceCatalogGroup, 0, len(ordered))
	for _, group := range ordered {
		view := UpstreamSourceCatalogGroup{PluginTag: group.PluginTag}
		for _, entry := range group.Upstreams {
			address := entry.Addr
			if entry.Protocol == "aliapi" && entry.ServerAddr != "" {
				address = entry.ServerAddr
			}
			view.Upstreams = append(view.Upstreams, UpstreamSourceCatalogEntry{
				Tag:      entry.Tag,
				Protocol: normalizeUpstreamProtocol(entry.Protocol),
				Address:  strings.TrimSpace(address),
				Enabled:  entry.Enabled,
			})
		}
		result = append(result, view)
	}
	return result
}
