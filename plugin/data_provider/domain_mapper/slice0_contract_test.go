package domain_mapper

import (
	"context"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/miekg/dns"
)

type slice0DetailedExporter struct {
	mu      sync.RWMutex
	entries []data_provider.RuleEntry
	subs    []func()
}

func (e *slice0DetailedExporter) GetRules() ([]string, error) {
	e.mu.RLock()
	defer e.mu.RUnlock()
	rules := make([]string, 0, len(e.entries))
	for _, entry := range e.entries {
		rules = append(rules, entry.Rule)
	}
	return rules, nil
}

func (e *slice0DetailedExporter) Subscribe(cb func()) {
	e.mu.Lock()
	e.subs = append(e.subs, cb)
	e.mu.Unlock()
}

func (e *slice0DetailedExporter) GetRuleEntries() ([]data_provider.RuleEntry, error) {
	e.mu.RLock()
	defer e.mu.RUnlock()
	return append([]data_provider.RuleEntry(nil), e.entries...), nil
}

func (e *slice0DetailedExporter) replace(entries []data_provider.RuleEntry) {
	e.mu.Lock()
	e.entries = append([]data_provider.RuleEntry(nil), entries...)
	subs := append([]func(){}, e.subs...)
	e.mu.Unlock()
	for _, cb := range subs {
		cb()
	}
}

func newSlice0DomainMapper(t *testing.T, exporters map[string]*slice0DetailedExporter) *DomainMapper {
	t.Helper()
	plugins := make(map[string]any, len(exporters))
	for tag, exporter := range exporters {
		plugins[tag] = exporter
	}
	m := coremain.NewTestMosdnsWithPlugins(plugins)
	rules := make([]RuleConfig, 0, len(exporters))
	rules = append(rules,
		RuleConfig{Tag: "base", Mark: 3, CtxMark: 30, OutputTag: "base"},
		RuleConfig{Tag: "alias", Mark: 3, CtxMark: 40, OutputTag: "base|alias"},
		RuleConfig{Tag: "keyword", Mark: 7, CtxMark: 70, OutputTag: "kw"},
		RuleConfig{Tag: "regex", Mark: 9, CtxMark: 90, OutputTag: "rx"},
	)
	value, err := NewMapper(coremain.NewBP("mapper", m), &Args{
		Rules:          rules,
		DefaultMark:    11,
		DefaultCtxMark: 110,
		DefaultTag:     "default",
	})
	if err != nil {
		t.Fatal(err)
	}
	return value.(*DomainMapper)
}

func TestSlice0DomainMapperInheritanceOverlapMetadataAndDefaults(t *testing.T) {
	exporters := map[string]*slice0DetailedExporter{
		"base": {entries: []data_provider.RuleEntry{
			{Rule: "domain:example.com", SourceName: "base-source", SourceType: "local"},
			{Rule: "full:child.example.com", SourceName: "exact-source", SourceType: "local"},
		}},
		"alias": {entries: []data_provider.RuleEntry{
			{Rule: "domain:example.com", SourceName: "base-source", SourceType: "alias"},
			{Rule: "full:child.example.com", SourceName: "alias-source", SourceType: "alias"},
		}},
		"keyword": {entries: []data_provider.RuleEntry{
			{Rule: "keyword:child", SourceName: "keyword-source", SourceType: "keyword"},
		}},
		"regex": {entries: []data_provider.RuleEntry{
			{Rule: `regexp:^child\.example\.com$`, SourceName: "regex-source", SourceType: "regex"},
		}},
	}
	dm := newSlice0DomainMapper(t, exporters)

	marks, tags, ok := dm.FastMatch("child.example.com.")
	if !ok {
		t.Fatal("expected child.example to match")
	}
	if !sameSlice0Uint8Set(marks, []uint8{3, 7, 9}) {
		t.Fatalf("FastMatch marks = %v, want the set [3 7 9]", marks)
	}
	if !sameJoinedSlice0Values(tags, "base|alias|kw|rx") {
		t.Fatalf("FastMatch tags = %q", tags)
	}

	q := new(dns.Msg)
	q.SetQuestion("child.example.com.", dns.TypeA)
	ctx := query_context.NewContext(q)
	if err := dm.Exec(context.Background(), ctx); err != nil {
		t.Fatal(err)
	}
	for _, mark := range []uint8{3, 7, 9} {
		if !ctx.HasFastFlag(mark) {
			t.Errorf("matched context is missing fast mark %d", mark)
		}
	}
	for _, mark := range []uint32{30, 40, 70, 90} {
		if !ctx.HasMark(mark) {
			t.Errorf("matched context is missing ctx mark %d", mark)
		}
	}
	if value, ok := ctx.GetValue(query_context.KeyDomainSet); !ok || valueStringSet(value, "base|alias|kw|rx") == false {
		t.Fatalf("domain tag value = %#v, want %q", value, "base|alias|kw|rx")
	}
	if value, ok := ctx.GetValue(query_context.KeyMatchedRuleSource); !ok || valueStringSet(value, "exact-source|alias-source|base-source|keyword-source|regex-source") == false {
		t.Fatalf("source value = %#v", value)
	}

	q = new(dns.Msg)
	q.SetQuestion("unrelated.example.", dns.TypeA)
	ctx = query_context.NewContext(q)
	if err := dm.GetFastExec()(context.Background(), ctx); err != nil {
		t.Fatal(err)
	}
	if !ctx.HasFastFlag(11) || !ctx.HasMark(110) {
		t.Fatal("default result did not apply default marks")
	}
	if value, ok := ctx.GetValue(query_context.KeyDomainSet); !ok || value != "default" {
		t.Fatalf("default tag = %#v", value)
	}

	entries, err := getRuleEntriesFromProvider(RuleConfig{Tag: "base"}, exporters["base"])
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 2 || entries[0].SourceName != "base-source" || entries[0].SourceType != "local" {
		t.Fatalf("detailed exporter entries = %+v", entries)
	}
}

func TestSlice0DomainMapperQuickAddAndConcurrentRebuildLookup(t *testing.T) {
	exporters := map[string]*slice0DetailedExporter{
		"base": {entries: []data_provider.RuleEntry{{
			Rule: "domain:example.com", SourceName: "base-source",
		}}},
		"alias": {entries: []data_provider.RuleEntry{{
			Rule: "full:child.example.com", SourceName: "exact-source",
		}}},
		"keyword": {entries: []data_provider.RuleEntry{{
			Rule: "keyword:child", SourceName: "keyword-source",
		}}},
		"regex": {entries: []data_provider.RuleEntry{{
			Rule: `regexp:^child\.example\.com$`, SourceName: "regex-source",
		}}},
	}
	dm := newSlice0DomainMapper(t, exporters)
	dm.QuickAdd("child.example.com", []uint8{13}, "quick")

	const workers = 8
	const iterations = 100
	var wg sync.WaitGroup
	for i := 0; i < workers; i++ {
		worker := i
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < iterations; j++ {
				dm.QuickAdd("child.example.com", []uint8{uint8(20 + worker)}, "q")
				_, _, _ = dm.FastMatch("child.example.com.")
				_, _ = dm.lookupMatchResult("child.example.com.")
			}
		}()
	}
	exporters["base"].replace([]data_provider.RuleEntry{{
		Rule: "full:new.example", SourceName: "new-source",
	}})
	for i := 0; i < workers; i++ {
		worker := i
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < iterations; j++ {
				if j%2 == 0 {
					dm.QuickAdd("child.example.com", []uint8{uint8(30 + worker)}, "q2")
				}
				_, _, _ = dm.FastMatch("new.example.")
			}
		}()
	}
	wg.Wait()

	deadline := time.Now().Add(3 * time.Second)
	for {
		if _, _, ok := dm.FastMatch("new.example."); ok {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("provider notification did not publish the rebuilt mapper snapshot")
		}
		time.Sleep(10 * time.Millisecond)
	}
	if _, _, ok := dm.FastMatch("old.example."); ok {
		t.Fatal("rebuilt mapper retained an old-only rule")
	}
}

func sameSlice0Uint8Set(a, b []uint8) bool {
	if len(a) != len(b) {
		return false
	}
	want := make(map[uint8]struct{}, len(b))
	for _, value := range b {
		want[value] = struct{}{}
	}
	for i := range a {
		if _, ok := want[a[i]]; !ok {
			return false
		}
	}
	return true
}

func sameJoinedSlice0Values(got, want string) bool {
	return valueStringSet(got, want)
}

func valueStringSet(value any, want string) bool {
	gotString, ok := value.(string)
	if !ok {
		return false
	}
	got := splitSlice0Values(gotString)
	wantValues := splitSlice0Values(want)
	if len(got) != len(wantValues) {
		return false
	}
	for part := range wantValues {
		if !got[part] {
			return false
		}
	}
	return true
}

func splitSlice0Values(value string) map[string]bool {
	parts := make(map[string]bool)
	for _, part := range strings.Split(value, "|") {
		parts[part] = true
	}
	return parts
}
