//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package base_domain

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"os"
	"path/filepath"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
	scdomain "github.com/sagernet/sing/common/domain"
)

func testDomainBQ() sequence.BQ {
	m := coremain.NewTestMosdnsWithPlugins(nil)
	return sequence.NewBQ(m, m.Logger())
}

func testDomainContext(name string) *query_context.Context {
	q := new(dns.Msg)
	q.SetQuestion(name, dns.TypeA)
	return query_context.NewContext(q)
}

func matchQuestion(qCtx *query_context.Context, m domain.Matcher[struct{}]) (bool, error) {
	_, ok := m.Match(qCtx.QQuestion().Name)
	return ok, nil
}

func writeDomainSRS(t *testing.T, path string) {
	t.Helper()
	var compressed bytes.Buffer
	zw := zlib.NewWriter(&compressed)
	bw := bufio.NewWriter(zw)
	var count [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(count[:], 1)
	if _, err := bw.Write(count[:n]); err != nil {
		t.Fatal(err)
	}
	if err := bw.WriteByte(0); err != nil { // default rule
		t.Fatal(err)
	}
	if err := bw.WriteByte(domain_setRuleItemDomain); err != nil {
		t.Fatal(err)
	}
	matcher := scdomain.NewMatcher(nil, []string{"srs.example"}, true)
	if err := matcher.Write(bw); err != nil {
		t.Fatal(err)
	}
	if err := bw.WriteByte(domain_setRuleItemFinal); err != nil {
		t.Fatal(err)
	}
	if err := bw.Flush(); err != nil {
		t.Fatal(err)
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	out := append([]byte{'S', 'R', 'S', 3}, compressed.Bytes()...)
	if err := os.WriteFile(path, out, 0o644); err != nil {
		t.Fatal(err)
	}
}

const (
	domain_setRuleItemDomain = 2
	domain_setRuleItemFinal  = 0xff
)

func TestRustBaseDomainTextFileHitCloseAndGoFallback(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	path := filepath.Join(t.TempDir(), "rules.txt")
	if err := os.WriteFile(path, []byte("full:file.example\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	m, err := NewMatcher(testDomainBQ(), &Args{Files: []string{path}}, matchQuestion)
	if err != nil {
		t.Fatal(err)
	}
	if m.rustBackend == nil {
		t.Fatal("base_domain Files must initialize the Rust backend")
	}
	if _, matched := m.rustBackend.Match("file.example."); !matched {
		t.Fatalf("direct Rust wrapper hit = false, want true")
	}
	if _, matched := m.rustBackend.Match("other.example."); matched {
		t.Fatalf("direct Rust wrapper miss = true, want false")
	}
	if matched, err := m.rustBackend.backend.Match("file.example."); err != nil || !matched {
		t.Fatalf("direct Rust backend hit = (%v, %v), want (true, nil)", matched, err)
	}
	ctx := testDomainContext("file.example.")
	matched, err := m.Match(nil, ctx)
	if err != nil || !matched {
		t.Fatalf("Rust-backed text file match = (%v, %v), want (true, nil)", matched, err)
	}
	if err := m.Close(); err != nil {
		t.Fatal(err)
	}
	if err := m.Close(); err != nil {
		t.Fatal(err)
	}
	matched, err = m.Match(nil, ctx)
	if err != nil || !matched {
		t.Fatalf("Go fallback after Close = (%v, %v), want (true, nil)", matched, err)
	}
}

func TestRustBaseDomainSRSFileAndCNAMEMatchFunc(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	path := filepath.Join(t.TempDir(), "rules.srs")
	writeDomainSRS(t, path)

	m, err := NewMatcher(testDomainBQ(), &Args{Files: []string{path}}, func(qCtx *query_context.Context, matcher domain.Matcher[struct{}]) (bool, error) {
		for _, rr := range qCtx.R().Answer {
			if cname, ok := rr.(*dns.CNAME); ok {
				if _, ok := matcher.Match(cname.Target); ok {
					return true, nil
				}
			}
		}
		return false, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()
	if m.rustBackend == nil {
		t.Fatal("base_domain SRS must initialize the Rust backend")
	}
	if matched, err := m.rustBackend.backend.Match("edge.srs.example."); err != nil || !matched {
		t.Fatalf("direct Rust SRS hit = (%v, %v), want (true, nil)", matched, err)
	}
	if matched, err := m.rustBackend.backend.Match("other.example."); err != nil || matched {
		t.Fatalf("direct Rust SRS miss = (%v, %v), want (false, nil)", matched, err)
	}

	ctx := testDomainContext("query.example.")
	ctx.SetResponse(&dns.Msg{Answer: []dns.RR{&dns.CNAME{
		Hdr:    dns.RR_Header{Name: "query.example.", Rrtype: dns.TypeCNAME, Class: dns.ClassINET, Ttl: 60},
		Target: "edge.srs.example.",
	}}})
	matched, err := m.Match(nil, ctx)
	if err != nil || !matched {
		t.Fatalf("Rust-backed CNAME/SRS match = (%v, %v), want (true, nil)", matched, err)
	}
}
