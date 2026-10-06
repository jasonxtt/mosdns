package coremain

import (
	"strings"
	"testing"
)

func TestResolveSpecialGroupBindings(t *testing.T) {
	raw := GlobalUpstreamOverrides{
		"foreign": {
			{Tag: "dns1", Enabled: true, Protocol: "udp", Addr: "1.1.1.1"},
			{Tag: "disabled", Enabled: false, Protocol: "udp", Addr: "9.9.9.9"},
		},
	}
	configured := map[string]configuredUpstreamGroup{
		"foreign": {
			PluginTag: "foreign",
			Upstreams: []UpstreamOverrideConfig{{Tag: "yaml-only", Enabled: true, Protocol: "udp", Addr: "8.8.8.8"}},
		},
	}

	group := SpecialGroup{
		Slot:           50,
		Name:           "test",
		OwnedUpstreams: []UpstreamOverrideConfig{{Tag: "own", Enabled: true, Protocol: "udp", Addr: "192.0.2.1"}},
		UpstreamSources: []SpecialUpstreamSource{
			{Kind: specialSourceKindGroup, PluginTag: "foreign"},
			{Kind: specialSourceKindUpstream, PluginTag: "foreign", UpstreamTag: "dns1"},
		},
	}

	resolution := resolveSpecialGroupWithState(group, raw, configured)
	if len(resolution.Effective) != 2 {
		t.Fatalf("effective upstream count = %d, want 2", len(resolution.Effective))
	}
	if resolution.Effective[0].Tag != "own" || resolution.Effective[1].Tag != "dns1" {
		t.Fatalf("effective tags = %#v, want own and dns1", resolution.Effective)
	}
	if len(resolution.Warnings) != 0 {
		t.Fatalf("unexpected warnings: %#v", resolution.Warnings)
	}

	individual := SpecialGroup{
		Slot: 51,
		Name: "individual",
		UpstreamSources: []SpecialUpstreamSource{
			{Kind: specialSourceKindUpstream, PluginTag: "foreign", UpstreamTag: "disabled"},
			{Kind: specialSourceKindUpstream, PluginTag: "foreign", UpstreamTag: "missing"},
		},
	}
	resolution = resolveSpecialGroupWithState(individual, raw, configured)
	if len(resolution.Effective) != 0 {
		t.Fatalf("disabled/missing references unexpectedly resolved: %#v", resolution.Effective)
	}
	joinedWarnings := strings.Join(resolution.Warnings, "\n")
	for _, want := range []string{"源上游已禁用：foreign / disabled", "源上游不存在：foreign / missing"} {
		if !strings.Contains(joinedWarnings, want) {
			t.Fatalf("warnings %q do not contain %q", joinedWarnings, want)
		}
	}
}

func TestResolveSpecialGroupWithoutValidUpstreamIsInactive(t *testing.T) {
	group := SpecialGroup{Slot: 50, Name: "empty"}
	resolution := resolveSpecialGroupWithState(group, nil, nil)
	if len(resolution.Effective) != 0 {
		t.Fatalf("effective upstreams = %#v, want empty", resolution.Effective)
	}
	if !strings.Contains(strings.Join(resolution.Warnings, "\n"), "不参与分流") {
		t.Fatalf("expected inactive warning, got %#v", resolution.Warnings)
	}
	if specialGroupRuntimeEnabled(group) {
		t.Fatal("empty special group should not be runtime enabled")
	}
}

func TestDeletedSourceGroupDoesNotResolveFromStaleOverride(t *testing.T) {
	group := SpecialGroup{
		Slot:            50,
		Name:            "deleted-source",
		UpstreamSources: []SpecialUpstreamSource{{Kind: specialSourceKindGroup, PluginTag: "deleted"}},
	}
	resolution := resolveSpecialGroupWithState(group, GlobalUpstreamOverrides{
		"deleted": {{Tag: "stale", Enabled: true, Protocol: "udp", Addr: "192.0.2.60"}},
	}, nil)
	if len(resolution.Effective) != 0 {
		t.Fatalf("stale override unexpectedly resolved: %#v", resolution.Effective)
	}
	if !strings.Contains(strings.Join(resolution.Warnings, "\n"), "源上游组不存在：deleted") {
		t.Fatalf("expected missing source warning, got %#v", resolution.Warnings)
	}
}

func TestNormalizeSpecialUpstreamSourcesRejectsSpecialGroupReference(t *testing.T) {
	for _, pluginTag := range []string{"special_upstream_60", "special_route_60", "special_manual_60"} {
		if _, err := normalizeSpecialUpstreamSources([]SpecialUpstreamSource{{
			Kind:      specialSourceKindGroup,
			PluginTag: pluginTag,
		}}); err == nil {
			t.Fatalf("expected special group reference %q to be rejected", pluginTag)
		}
	}
	if slot, ok := parseSpecialUpstreamSlot("special_upstream_60"); !ok || slot != 60 {
		t.Fatalf("parseSpecialUpstreamSlot() = %d, %v, want 60, true", slot, ok)
	}
}

func TestMigrateLegacySpecialUpstreamToOwnedUpstreams(t *testing.T) {
	upstreamOverridesLock.Lock()
	oldOverrides := upstreamOverrides
	upstreamOverrides = GlobalUpstreamOverrides{
		"special_upstream_50": {{Tag: "legacy", Enabled: true, Protocol: "udp", Addr: "192.0.2.50"}},
	}
	upstreamOverridesLock.Unlock()
	defer func() {
		upstreamOverridesLock.Lock()
		upstreamOverrides = oldOverrides
		upstreamOverridesLock.Unlock()
	}()

	groups := migrateSpecialGroupOwnedUpstreams([]SpecialGroup{{Slot: 50, Name: "legacy"}})
	if len(groups) != 1 || len(groups[0].OwnedUpstreams) != 1 || groups[0].OwnedUpstreams[0].Tag != "legacy" {
		t.Fatalf("legacy migration result = %#v", groups)
	}
}
