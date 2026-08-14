//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package domain_mapper

import (
	"context"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/miekg/dns"
)

func TestSlice4DomainMapperUsesRealRustGeneration(t *testing.T) {
	t.Setenv("MOSDNS_MATCHER_BACKEND", "rust")
	exporters := map[string]*slice0DetailedExporter{
		"base": {entries: []data_provider.RuleEntry{{
			Rule: "domain:example.com", SourceName: "base-source",
		}}},
		"domain_set_light": {entries: []data_provider.RuleEntry{{
			Rule: "full:child.example.com", SourceName: "light-source",
		}}},
		"sd_set_light": {entries: []data_provider.RuleEntry{{
			Rule: "keyword:child", SourceName: "sd-light-source",
		}}},
		"regex": {entries: []data_provider.RuleEntry{{
			Rule: `regexp:^child\.example\.com$`, SourceName: "regex-source",
		}}},
	}
	dm := newSlice4Mapper(t, exporters, []RuleConfig{
		{Tag: "base", Mark: 3, CtxMark: 30, OutputTag: "base"},
		{Tag: "domain_set_light", Mark: 7, CtxMark: 70, OutputTag: "light"},
		{Tag: "sd_set_light", Mark: 9, CtxMark: 90, OutputTag: "sd-light"},
		{Tag: "regex", Mark: 11, CtxMark: 110, OutputTag: "regex"},
	})
	t.Cleanup(func() { _ = dm.Close() })

	dm.generationMu.RLock()
	generation := dm.rustGeneration
	dm.generationMu.RUnlock()
	if generation == nil || generation.snapshot == nil {
		t.Fatal("domain_mapper did not publish a Rust valued generation")
	}
	if length, err := generation.snapshot.Len(); err != nil || length != 4 {
		t.Fatalf("Rust valued generation Len = %d, %v; want 4, nil", length, err)
	}

	marks, tags, ok := dm.FastMatch("child.example.com.")
	if !ok || !sameSlice0Uint8Set(marks, []uint8{3, 7, 9, 11}) || !sameJoinedSlice0Values(tags, "light|base|sd-light|regex") {
		t.Fatalf("real Rust FastMatch = %v, %q, %v", marks, tags, ok)
	}
	q := new(dns.Msg)
	q.SetQuestion("child.example.com.", dns.TypeA)
	qctx := query_context.NewContext(q)
	if err := dm.Exec(context.Background(), qctx); err != nil {
		t.Fatal(err)
	}
	for _, mark := range []uint8{3, 7, 9, 11} {
		if !qctx.HasFastFlag(mark) {
			t.Errorf("real Rust query context is missing fast mark %d", mark)
		}
	}
	for _, mark := range []uint32{30, 70, 90, 110} {
		if !qctx.HasMark(mark) {
			t.Errorf("real Rust query context is missing context mark %d", mark)
		}
	}
}
