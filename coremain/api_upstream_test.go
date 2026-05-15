package coremain

import (
	"os"
	"path/filepath"
	"testing"
)

func TestGetUpstreamOverridesForeignSocks5Fallback(t *testing.T) {
	tmpDir := t.TempDir()
	oldBaseDir := MainConfigBaseDir
	oldOverrides := upstreamOverrides
	defer func() {
		MainConfigBaseDir = oldBaseDir
		upstreamOverrides = oldOverrides
	}()

	MainConfigBaseDir = tmpDir
	overridesPath := overridesPathInDir(tmpDir)
	if err := os.MkdirAll(filepath.Dir(overridesPath), 0o755); err != nil {
		t.Fatalf("mkdir overrides dir: %v", err)
	}
	if err := os.WriteFile(overridesPath, []byte(`{"socks5":"127.0.0.1:7890"}`), 0o644); err != nil {
		t.Fatalf("write overrides file: %v", err)
	}

	upstreamOverrides = GlobalUpstreamOverrides{
		"foreign": {
			{
				Tag:      "f1",
				Protocol: "https",
				Addr:     "https://dns.google/dns-query",
				Socks5:   "",
			},
			{
				Tag:      "f2",
				Protocol: "https",
				Addr:     "https://1.1.1.1/dns-query",
				Socks5:   "10.0.0.2:7891",
			},
		},
	}

	entries := GetUpstreamOverrides("foreign")
	if len(entries) != 2 {
		t.Fatalf("expected 2 entries, got %d", len(entries))
	}
	if entries[0].Socks5 != "127.0.0.1:7890" {
		t.Fatalf("expected fallback socks5 for first entry, got %q", entries[0].Socks5)
	}
	if entries[1].Socks5 != "10.0.0.2:7891" {
		t.Fatalf("expected explicit socks5 to win, got %q", entries[1].Socks5)
	}

	// Ensure original in-memory state is not mutated by fallback resolution.
	if upstreamOverrides["foreign"][0].Socks5 != "" {
		t.Fatalf("expected original override entry unchanged, got %q", upstreamOverrides["foreign"][0].Socks5)
	}
}
