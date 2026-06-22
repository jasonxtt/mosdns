package coremain

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestFirstFreeSpecialSlotReusesLowestGap(t *testing.T) {
	groups := []SpecialGroup{
		{Slot: 50, Name: "a"},
		{Slot: 52, Name: "b"},
		{Slot: 53, Name: "c"},
	}

	if got := firstFreeSpecialSlot(groups); got != 51 {
		t.Fatalf("firstFreeSpecialSlot() = %d, want 51", got)
	}
}

func TestRenderSpecialGroupsConfigValid(t *testing.T) {
	t.Run("empty", func(t *testing.T) {
		raw := renderSpecialGroupsConfig(nil)
		if err := validateSpecialGroupsConfig(raw); err != nil {
			t.Fatalf("validateSpecialGroupsConfig(empty) error = %v", err)
		}
		text := string(raw)
		if !strings.Contains(text, "rules: []") {
			t.Fatalf("expected empty rules list, got:\n%s", text)
		}
		for _, want := range []string{
			"tag: sequence_special_v4",
			"tag: sequence_special_v6",
			"tag: sequence_special_ot",
			"args: []",
		} {
			if !strings.Contains(text, want) {
				t.Fatalf("expected empty config to contain %q, got:\n%s", want, text)
			}
		}
	})

	t.Run("populated", func(t *testing.T) {
		raw := renderSpecialGroupsConfig([]SpecialGroup{
			{Slot: 50, Name: "cmcc", ListenPort: 6053, CustomPortOnly: true},
			{Slot: 53, Name: "hk"},
		})
		if err := validateSpecialGroupsConfig(raw); err != nil {
			t.Fatalf("validateSpecialGroupsConfig(populated) error = %v", err)
		}
		text := string(raw)
		for _, want := range []string{
			"special_route_50",
			"special_manual_50",
			"special_upstream_53",
			"cache/cache_special_53.dump",
			"special_udp_server_50",
			"special_tcp_server_50",
			"enable_audit: true",
			`listen: ":6053"`,
			"mark 53",
		} {
			if !strings.Contains(text, want) {
				t.Fatalf("expected generated config to contain %q, got:\n%s", want, text)
			}
		}
		if strings.Contains(text, "mark 50") {
			t.Fatalf("custom-port-only group should not be reachable from 53 chain:\n%s", text)
		}
		if strings.Contains(text, "special_udp_server_53") || strings.Contains(text, "special_tcp_server_53") {
			t.Fatalf("unexpected listeners rendered for group without listen_port:\n%s", text)
		}
	})
}

func TestSyncSpecialGroupsConfigWritesRuntimeFile(t *testing.T) {
	dir := t.TempDir()
	webinfoDir := filepath.Join(dir, managedWebInfoDirName)
	if err := os.MkdirAll(webinfoDir, 0o755); err != nil {
		t.Fatalf("mkdir webinfo: %v", err)
	}

	jsonPath := filepath.Join(webinfoDir, specialGroupsFilename)
	if err := os.WriteFile(jsonPath, []byte(`[
  {"slot": 50, "name": "cmcc", "listen_port": 6053, "custom_port_only": true},
  {"slot": 52, "name": "hk"}
]`), 0o644); err != nil {
		t.Fatalf("write json: %v", err)
	}

	if err := SyncSpecialGroupsConfig(dir); err != nil {
		t.Fatalf("SyncSpecialGroupsConfig() error = %v", err)
	}

	raw, err := os.ReadFile(filepath.Join(dir, specialGroupsConfigRelativePath))
	if err != nil {
		t.Fatalf("read generated yaml: %v", err)
	}
	text := string(raw)
	if strings.Contains(text, "special_route_51") {
		t.Fatalf("unexpected unused slot rendered:\n%s", text)
	}
	for _, want := range []string{
		"special_route_50",
		"special_route_52",
		"mark 52",
		`listen: ":6053"`,
	} {
		if !strings.Contains(text, want) {
			t.Fatalf("expected generated config to contain %q, got:\n%s", want, text)
		}
	}
	if strings.Contains(text, "mark 50") {
		t.Fatalf("custom-port-only group should not be rendered into 53 chain:\n%s", text)
	}
}

func TestNormalizeSpecialGroupListenPort(t *testing.T) {
	tests := []struct {
		name    string
		port    int
		want    int
		wantErr bool
	}{
		{name: "disabled", port: 0, want: 0},
		{name: "valid", port: 6053, want: 6053},
		{name: "negative", port: -1, wantErr: true},
		{name: "too large", port: 70000, wantErr: true},
		{name: "reserved", port: 53, wantErr: true},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got, err := normalizeSpecialGroupListenPort(tc.port)
			if tc.wantErr {
				if err == nil {
					t.Fatalf("normalizeSpecialGroupListenPort(%d) expected error", tc.port)
				}
				return
			}
			if err != nil {
				t.Fatalf("normalizeSpecialGroupListenPort(%d) error = %v", tc.port, err)
			}
			if got != tc.want {
				t.Fatalf("normalizeSpecialGroupListenPort(%d) = %d, want %d", tc.port, got, tc.want)
			}
		})
	}
}

func TestNormalizeSpecialGroupsDedupListenPorts(t *testing.T) {
	groups := normalizeSpecialGroups([]SpecialGroup{
		{Slot: 50, Name: "cmcc", ListenPort: 6053, CustomPortOnly: true},
		{Slot: 51, Name: "hk", ListenPort: 6053, CustomPortOnly: true},
		{Slot: 52, Name: "bad", ListenPort: 53, CustomPortOnly: true},
	})

	if len(groups) != 3 {
		t.Fatalf("normalizeSpecialGroups() len = %d, want 3", len(groups))
	}
	if groups[0].ListenPort != 6053 {
		t.Fatalf("first listen_port = %d, want 6053", groups[0].ListenPort)
	}
	if !groups[0].CustomPortOnly {
		t.Fatalf("first custom_port_only = false, want true")
	}
	if groups[1].ListenPort != 0 {
		t.Fatalf("duplicate listen_port should be cleared, got %d", groups[1].ListenPort)
	}
	if groups[1].CustomPortOnly {
		t.Fatalf("duplicate listen_port should also clear custom_port_only")
	}
	if groups[2].ListenPort != 0 {
		t.Fatalf("invalid listen_port should be cleared, got %d", groups[2].ListenPort)
	}
	if groups[2].CustomPortOnly {
		t.Fatalf("invalid listen_port should also clear custom_port_only")
	}
}

func TestCleanupOrphanSpecialCacheDumps(t *testing.T) {
	dir := t.TempDir()
	cacheDir := filepath.Join(dir, "cache")
	if err := os.MkdirAll(cacheDir, 0o755); err != nil {
		t.Fatalf("mkdir cache: %v", err)
	}

	keep := filepath.Join(cacheDir, "cache_special_50.dump")
	remove := filepath.Join(cacheDir, "cache_special_51.dump")
	if err := os.WriteFile(keep, []byte("keep"), 0o644); err != nil {
		t.Fatalf("write keep: %v", err)
	}
	if err := os.WriteFile(remove, []byte("remove"), 0o644); err != nil {
		t.Fatalf("write remove: %v", err)
	}

	if err := cleanupOrphanSpecialCacheDumps(dir, []SpecialGroup{{Slot: 50, Name: "cmcc"}}); err != nil {
		t.Fatalf("cleanupOrphanSpecialCacheDumps() error = %v", err)
	}

	if _, err := os.Stat(keep); err != nil {
		t.Fatalf("expected keep file to remain, stat error = %v", err)
	}
	if _, err := os.Stat(remove); !os.IsNotExist(err) {
		t.Fatalf("expected orphan file removed, stat err = %v", err)
	}
}

func TestBuildSpecialGroupViewMarksBridgePortMappingRequirement(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerNetworkModeEnv, containerNetworkModeBridge)

	view := buildSpecialGroupView(SpecialGroup{
		Slot:       50,
		Name:       "cmcc",
		ListenPort: 6053,
	})

	if !view.PortMappingRequired {
		t.Fatal("PortMappingRequired = false, want true in bridge container mode")
	}
	if !strings.Contains(view.Message, "6053/tcp") || !strings.Contains(view.Message, "6053/udp") {
		t.Fatalf("Message = %q, want port mapping hint", view.Message)
	}
}

func TestBuildSpecialGroupViewSkipsPortMappingRequirementInHostMode(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerNetworkModeEnv, containerNetworkModeHost)

	view := buildSpecialGroupView(SpecialGroup{
		Slot:       50,
		Name:       "cmcc",
		ListenPort: 6053,
	})

	if view.PortMappingRequired {
		t.Fatal("PortMappingRequired = true, want false in host container mode")
	}
	if view.Message != "" {
		t.Fatalf("Message = %q, want empty in host container mode", view.Message)
	}
}
