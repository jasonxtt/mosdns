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
		if !strings.Contains(text, "args: []") {
			t.Fatalf("expected empty sequence args, got:\n%s", text)
		}
	})

	t.Run("populated", func(t *testing.T) {
		raw := renderSpecialGroupsConfig([]SpecialGroup{
			{Slot: 50, Name: "cmcc"},
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
			"mark 53",
			"cache/cache_special_53.dump",
		} {
			if !strings.Contains(text, want) {
				t.Fatalf("expected generated config to contain %q, got:\n%s", want, text)
			}
		}
	})
}

func TestSyncSpecialGroupsConfigWritesRuntimeFile(t *testing.T) {
	dir := t.TempDir()
	jsonPath := filepath.Join(dir, specialGroupsFilename)
	if err := os.WriteFile(jsonPath, []byte(`[
  {"slot": 50, "name": "cmcc"},
  {"slot": 52, "name": "hk"}
]`), 0644); err != nil {
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
		"mark 50",
		"mark 52",
	} {
		if !strings.Contains(text, want) {
			t.Fatalf("expected generated config to contain %q, got:\n%s", want, text)
		}
	}
}

func TestCleanupOrphanSpecialCacheDumps(t *testing.T) {
	dir := t.TempDir()
	cacheDir := filepath.Join(dir, "cache")
	if err := os.MkdirAll(cacheDir, 0755); err != nil {
		t.Fatalf("mkdir cache: %v", err)
	}

	keep := filepath.Join(cacheDir, "cache_special_50.dump")
	remove := filepath.Join(cacheDir, "cache_special_51.dump")
	if err := os.WriteFile(keep, []byte("keep"), 0644); err != nil {
		t.Fatalf("write keep: %v", err)
	}
	if err := os.WriteFile(remove, []byte("remove"), 0644); err != nil {
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
