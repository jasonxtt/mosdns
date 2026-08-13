//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package base_ip

import (
	"net/netip"
	"os"
	"path/filepath"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
)

func testIPBQ() sequence.BQ {
	m := coremain.NewTestMosdnsWithPlugins(nil)
	return sequence.NewBQ(m, m.Logger())
}

func testIPContext(addr netip.Addr) *query_context.Context {
	q := new(dns.Msg)
	q.SetQuestion("example.", dns.TypeA)
	ctx := query_context.NewContext(q)
	ctx.ServerMeta.ClientAddr = addr
	return ctx
}

func matchClientIP(qCtx *query_context.Context, matcher netlist.Matcher) (bool, error) {
	return matcher.Match(qCtx.ServerMeta.ClientAddr), nil
}

func TestRustBaseIPIPsAndFilesHitCloseAndGoFallback(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	path := filepath.Join(t.TempDir(), "ips.txt")
	if err := os.WriteFile(path, []byte("2001:db8::/32\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	m, err := NewMatcher(testIPBQ(), &Args{
		IPs:   []string{"10.0.0.0/8"},
		Files: []string{path},
	}, matchClientIP)
	if err != nil {
		t.Fatal(err)
	}
	if m.rustBackend == nil {
		t.Fatal("base_ip IPs/Files must initialize the Rust backend")
	}
	if matched := m.rustBackend.Match(netip.MustParseAddr("2001:db8::1")); !matched {
		t.Fatalf("direct Rust wrapper file hit = false, want true")
	}
	if matched := m.rustBackend.Match(netip.MustParseAddr("2001:db9::1")); matched {
		t.Fatal("direct Rust wrapper file miss must be false")
	}
	if matched, err := m.rustBackend.backend.Match("10.1.2.3"); err != nil || !matched {
		t.Fatalf("direct Rust inline IP hit = (%v, %v), want (true, nil)", matched, err)
	}
	if matched, err := m.rustBackend.backend.Match("11.1.2.3"); err != nil || matched {
		t.Fatalf("direct Rust inline IP miss = (%v, %v), want (false, nil)", matched, err)
	}
	ctx := testIPContext(netip.MustParseAddr("2001:db8::1"))
	matched, err := m.Match(nil, ctx)
	if err != nil || !matched {
		t.Fatalf("Rust-backed file IP match = (%v, %v), want (true, nil)", matched, err)
	}
	ctx.ServerMeta.ClientAddr = netip.MustParseAddr("10.1.2.3")
	matched, err = m.Match(nil, ctx)
	if err != nil || !matched {
		t.Fatalf("Rust-backed inline IP match = (%v, %v), want (true, nil)", matched, err)
	}

	if err := m.Close(); err != nil {
		t.Fatal(err)
	}
	if err := m.Close(); err != nil {
		t.Fatal(err)
	}
	ctx.ServerMeta.ClientAddr = netip.MustParseAddr("2001:db8::1")
	matched, err = m.Match(nil, ctx)
	if err != nil || !matched {
		t.Fatalf("Go fallback after Close = (%v, %v), want (true, nil)", matched, err)
	}
}
