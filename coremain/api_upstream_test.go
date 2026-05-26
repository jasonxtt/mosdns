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

func TestGetUpstreamOverridesNormalizesRuntimeAddrWithoutMutatingStoredState(t *testing.T) {
	upstreamOverridesLock.Lock()
	oldOverrides := upstreamOverrides
	upstreamOverrides = GlobalUpstreamOverrides{
		"domestic": {
			{Tag: "udp1", Protocol: "udp", Addr: "223.5.5.5"},
			{Tag: "dot1", Protocol: "dot", Addr: "223.5.5.5"},
			{Tag: "doh1", Protocol: "doh", Addr: "223.5.5.5"},
			{Tag: "doq1", Protocol: "doq", Addr: "223.5.5.5"},
			{Tag: "doh2", Protocol: "https", Addr: "dns.alidns.com/dns-query"},
		},
	}
	upstreamOverridesLock.Unlock()
	defer func() {
		upstreamOverridesLock.Lock()
		upstreamOverrides = oldOverrides
		upstreamOverridesLock.Unlock()
	}()

	got := GetUpstreamOverrides("domestic")
	if got[0].Addr != "udp://223.5.5.5" {
		t.Fatalf("expected udp runtime addr to be normalized, got %q", got[0].Addr)
	}
	if got[1].Protocol != "tls" || got[1].Addr != "tls://223.5.5.5" {
		t.Fatalf("expected dot to normalize to tls://, got protocol=%q addr=%q", got[1].Protocol, got[1].Addr)
	}
	if got[2].Protocol != "https" || got[2].Addr != "https://223.5.5.5/dns-query" {
		t.Fatalf("expected doh IP addr to normalize to https://.../dns-query, got protocol=%q addr=%q", got[2].Protocol, got[2].Addr)
	}
	if got[3].Protocol != "quic" || got[3].Addr != "quic://223.5.5.5" {
		t.Fatalf("expected doq to normalize to quic://, got protocol=%q addr=%q", got[3].Protocol, got[3].Addr)
	}
	if got[4].Addr != "https://dns.alidns.com/dns-query" {
		t.Fatalf("expected DoH host/path addr to normalize with https scheme, got %q", got[4].Addr)
	}

	upstreamOverridesLock.RLock()
	stored := upstreamOverrides["domestic"]
	upstreamOverridesLock.RUnlock()
	if stored[1].Protocol != "dot" || stored[1].Addr != "223.5.5.5" {
		t.Fatalf("expected stored dot entry to remain unchanged, got protocol=%q addr=%q", stored[1].Protocol, stored[1].Addr)
	}
	if stored[2].Protocol != "doh" || stored[2].Addr != "223.5.5.5" {
		t.Fatalf("expected stored doh entry to remain unchanged, got protocol=%q addr=%q", stored[2].Protocol, stored[2].Addr)
	}
}

func TestValidateProtocolAddrCompatibility(t *testing.T) {
	cases := []struct {
		name      string
		protocol  string
		addr      string
		wantError bool
	}{
		{name: "udp bare addr", protocol: "udp", addr: "223.5.5.5", wantError: false},
		{name: "dot bare addr", protocol: "dot", addr: "223.5.5.5", wantError: false},
		{name: "doh bare addr", protocol: "doh", addr: "223.5.5.5", wantError: false},
		{name: "dot tls scheme", protocol: "dot", addr: "tls://223.5.5.5", wantError: false},
		{name: "doh https scheme", protocol: "doh", addr: "https://223.5.5.5/dns-query", wantError: false},
		{name: "udp conflicting scheme", protocol: "udp", addr: "tcp://223.5.5.5", wantError: true},
		{name: "doh conflicting scheme", protocol: "doh", addr: "udp://223.5.5.5", wantError: true},
		{name: "doq conflicting scheme", protocol: "doq", addr: "tls://223.5.5.5", wantError: true},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := validateProtocolAddrCompatibility(tc.protocol, tc.addr)
			if tc.wantError && err == nil {
				t.Fatalf("expected error, got nil")
			}
			if !tc.wantError && err != nil {
				t.Fatalf("expected no error, got %v", err)
			}
		})
	}
}

func TestDisabledEntryStillValidatesSchemeCompatibility(t *testing.T) {
	payload := struct {
		PluginTag string                   `json:"plugin_tag"`
		Upstreams []UpstreamOverrideConfig `json:"upstreams"`
	}{
		PluginTag: "domestic",
		Upstreams: []UpstreamOverrideConfig{
			{
				Tag:      "disabled-conflict",
				Enabled:  false,
				Protocol: "doh",
				Addr:     "udp://223.5.5.5",
			},
		},
	}

	for _, u := range payload.Upstreams {
		if err := validateProtocolAddrCompatibility(u.Protocol, u.Addr); err == nil {
			t.Fatalf("expected disabled conflicting entry to still fail validation")
		}
	}
}
