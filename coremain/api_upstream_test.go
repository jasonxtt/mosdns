package coremain

import (
	"os"
	"path/filepath"
	"testing"
)

func TestGetUpstreamOverridesForeignSocksFallback(t *testing.T) {
	tempDir := t.TempDir()
	overridesFile := filepath.Join(tempDir, overridesFilename)
	if err := os.WriteFile(overridesFile, []byte(`{"socks5":"127.0.0.1:1080"}`), 0o644); err != nil {
		t.Fatalf("write overrides file: %v", err)
	}

	oldBaseDir := MainConfigBaseDir
	MainConfigBaseDir = tempDir
	defer func() {
		MainConfigBaseDir = oldBaseDir
	}()

	upstreamOverridesLock.Lock()
	oldOverrides := upstreamOverrides
	upstreamOverrides = GlobalUpstreamOverrides{
		"foreign": {
			{Tag: "f1", Protocol: "doh", Addr: "https://dns.google/dns-query"},
			{Tag: "f2", Protocol: "doh", Addr: "https://dns.alidns.com/dns-query", Socks5: "127.0.0.1:2080"},
		},
	}
	upstreamOverridesLock.Unlock()
	defer func() {
		upstreamOverridesLock.Lock()
		upstreamOverrides = oldOverrides
		upstreamOverridesLock.Unlock()
	}()

	got := GetUpstreamOverrides("foreign")
	if len(got) != 2 {
		t.Fatalf("expected 2 entries, got %d", len(got))
	}
	if got[0].Socks5 != "127.0.0.1:1080" {
		t.Fatalf("expected fallback socks5 for entry 0, got %q", got[0].Socks5)
	}
	if got[1].Socks5 != "127.0.0.1:2080" {
		t.Fatalf("expected entry 1 custom socks5 to be preserved, got %q", got[1].Socks5)
	}

	upstreamOverridesLock.RLock()
	stored := upstreamOverrides["foreign"]
	upstreamOverridesLock.RUnlock()
	if len(stored) != 2 {
		t.Fatalf("expected stored entries to remain unchanged, got %d", len(stored))
	}
	if stored[0].Socks5 != "" {
		t.Fatalf("expected stored entry 0 socks5 to remain empty, got %q", stored[0].Socks5)
	}
}

func TestGetUpstreamOverridesNoFallbackForNonForeign(t *testing.T) {
	tempDir := t.TempDir()
	overridesFile := filepath.Join(tempDir, overridesFilename)
	if err := os.WriteFile(overridesFile, []byte(`{"socks5":"127.0.0.1:1080"}`), 0o644); err != nil {
		t.Fatalf("write overrides file: %v", err)
	}

	oldBaseDir := MainConfigBaseDir
	MainConfigBaseDir = tempDir
	defer func() {
		MainConfigBaseDir = oldBaseDir
	}()

	upstreamOverridesLock.Lock()
	oldOverrides := upstreamOverrides
	upstreamOverrides = GlobalUpstreamOverrides{
		"domestic": {
			{Tag: "d1", Protocol: "doh", Addr: "https://dns.alidns.com/dns-query"},
		},
	}
	upstreamOverridesLock.Unlock()
	defer func() {
		upstreamOverridesLock.Lock()
		upstreamOverrides = oldOverrides
		upstreamOverridesLock.Unlock()
	}()

	got := GetUpstreamOverrides("domestic")
	if len(got) != 1 {
		t.Fatalf("expected 1 entry, got %d", len(got))
	}
	if got[0].Socks5 != "" {
		t.Fatalf("expected no fallback for non-foreign tag, got %q", got[0].Socks5)
	}
}
