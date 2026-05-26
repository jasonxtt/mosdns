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
