package domain_mapper

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
	"github.com/miekg/dns"
)

type slice4ValuedSnapshot struct {
	results  map[string]matcher_adapter.ValuedResult
	closeErr error
	matchErr error
	entered  chan struct{}
	release  chan struct{}
	blockOne sync.Once
	closed   atomic.Bool
	matches  atomic.Int64
	closeN   atomic.Int64
}

func (s *slice4ValuedSnapshot) Match(name string) (matcher_adapter.ValuedResult, error) {
	s.matches.Add(1)
	if s.entered != nil && s.release != nil {
		s.blockOne.Do(func() { close(s.entered) })
		<-s.release
	}
	if s.matchErr != nil {
		return matcher_adapter.ValuedResult{}, s.matchErr
	}
	result, ok := s.results[name]
	if !ok {
		return matcher_adapter.ValuedResult{}, nil
	}
	return result, nil
}

func (s *slice4ValuedSnapshot) Len() (uint64, error) { return uint64(len(s.results)), nil }

func (s *slice4ValuedSnapshot) Close() error {
	s.closeN.Add(1)
	s.closed.Store(true)
	return s.closeErr
}

type slice4ValuedBuilder struct {
	mu       sync.Mutex
	builds   [][]matcher_adapter.ValuedRule
	snapshot *slice4ValuedSnapshot
	err      error
}

func (b *slice4ValuedBuilder) build(rules []matcher_adapter.ValuedRule) (matcher_adapter.ValuedSnapshot, error) {
	b.mu.Lock()
	b.builds = append(b.builds, append([]matcher_adapter.ValuedRule(nil), rules...))
	snapshot, err := b.snapshot, b.err
	b.mu.Unlock()
	if err != nil {
		return nil, err
	}
	return snapshot, nil
}

func newSlice4Mapper(t *testing.T, exporters map[string]*slice0DetailedExporter, rules []RuleConfig) *DomainMapper {
	t.Helper()
	plugins := make(map[string]any, len(exporters))
	for tag, exporter := range exporters {
		plugins[tag] = exporter
	}
	m := coremain.NewTestMosdnsWithPlugins(plugins)
	value, err := NewMapper(coremain.NewBP("mapper", m), &Args{
		Rules:          rules,
		DefaultMark:    61,
		DefaultCtxMark: 610,
		DefaultTag:     "slice4-default",
	})
	if err != nil {
		t.Fatal(err)
	}
	return value.(*DomainMapper)
}

func TestSlice4ValuedMapperUsesOneGenerationAndMergesQuickAdd(t *testing.T) {
	oldBuilder := valuedSnapshotBuilder
	t.Cleanup(func() { valuedSnapshotBuilder = oldBuilder })

	builder := &slice4ValuedBuilder{snapshot: &slice4ValuedSnapshot{results: map[string]matcher_adapter.ValuedResult{
		"child.example.com.": {
			Matched:       true,
			FastMarks:     []uint8{3, 7, 9},
			CtxMarks:      []uint32{30, 70, 90},
			JoinedTags:    "light|base",
			JoinedSources: "light-source|base-source",
		},
	}}}
	valuedSnapshotBuilder = builder.build

	exporters := map[string]*slice0DetailedExporter{
		"base": {entries: []data_provider.RuleEntry{{
			Rule: "domain:example.com", SourceName: "base-source",
		}}},
		"domain_set_light": {entries: []data_provider.RuleEntry{{
			Rule: "full:child.example.com", SourceName: "light-source",
		}}},
	}
	dm := newSlice4Mapper(t, exporters, []RuleConfig{
		{Tag: "base", Mark: 3, CtxMark: 30, OutputTag: "base"},
		{Tag: "domain_set_light", Mark: 7, CtxMark: 70, OutputTag: "light"},
	})

	builder.mu.Lock()
	if len(builder.builds) != 1 || len(builder.builds[0]) != 2 {
		t.Fatalf("valued builder rules = %+v, want both static and light entries", builder.builds)
	}
	builder.mu.Unlock()

	marks, tags, ok := dm.FastMatch("child.example.com.")
	if !ok || !sameSlice0Uint8Set(marks, []uint8{3, 7, 9}) || !sameJoinedSlice0Values(tags, "base|light") {
		t.Fatalf("Rust generation FastMatch = %v, %q, %v", marks, tags, ok)
	}

	q := new(dns.Msg)
	q.SetQuestion("child.example.com.", dns.TypeA)
	qctx := query_context.NewContext(q)
	if err := dm.Exec(context.Background(), qctx); err != nil {
		t.Fatal(err)
	}
	for _, mark := range []uint8{3, 7, 9} {
		if !qctx.HasFastFlag(mark) {
			t.Errorf("missing fast mark %d", mark)
		}
	}
	for _, mark := range []uint32{30, 70, 90} {
		if !qctx.HasMark(mark) {
			t.Errorf("missing context mark %d", mark)
		}
	}
	if got, _ := qctx.GetValue(query_context.KeyMatchedRuleSource); got != "light-source|base-source" {
		t.Fatalf("sources = %#v", got)
	}
	if got, _ := qctx.GetValue(query_context.KeyDomainSet); got != "light|base" {
		t.Fatalf("domain tags = %#v", got)
	}

	q = new(dns.Msg)
	q.SetQuestion("child.example.com.", dns.TypeA)
	qctx = query_context.NewContext(q)
	if err := dm.GetFastExec()(context.Background(), qctx); err != nil {
		t.Fatal(err)
	}
	if !qctx.HasFastFlag(3) || !qctx.HasFastFlag(7) || !qctx.HasFastFlag(9) {
		t.Fatal("GetFastExec did not apply the valued Rust result")
	}

	q = new(dns.Msg)
	q.SetQuestion("unrelated.example.", dns.TypeA)
	qctx = query_context.NewContext(q)
	if err := dm.Exec(context.Background(), qctx); err != nil {
		t.Fatal(err)
	}
	if !qctx.HasFastFlag(61) || !qctx.HasMark(610) {
		t.Fatal("default result did not apply after a valued Rust miss")
	}
	if got, _ := qctx.GetValue(query_context.KeyDomainSet); got != "slice4-default" {
		t.Fatalf("default tag = %#v", got)
	}

	dm.QuickAdd("child.example.com", []uint8{13}, "quick")
	marks, tags, ok = dm.FastMatch("child.example.com.")
	if !ok || !sameSlice0Uint8Set(marks, []uint8{3, 7, 9, 13}) || !sameJoinedSlice0Values(tags, "base|light|quick") {
		t.Fatalf("Rust/static+QuickAdd FastMatch = %v, %q, %v", marks, tags, ok)
	}

	if err := dm.Close(); err != nil {
		t.Fatal(err)
	}
	if err := dm.Close(); err != nil {
		t.Fatal(err)
	}
	if !builder.snapshot.closed.Load() || builder.snapshot.closeN.Load() != 1 {
		t.Fatalf("snapshot lifecycle = closed=%v close_count=%d", builder.snapshot.closed.Load(), builder.snapshot.closeN.Load())
	}
}

func TestSlice4RebuildFailurePublishesCurrentGoGeneration(t *testing.T) {
	oldBuilder := valuedSnapshotBuilder
	t.Cleanup(func() { valuedSnapshotBuilder = oldBuilder })

	first := &slice4ValuedSnapshot{results: map[string]matcher_adapter.ValuedResult{
		"old.example.": {Matched: true, FastMarks: []uint8{3}},
	}}
	builder := &slice4ValuedBuilder{snapshot: first}
	valuedSnapshotBuilder = func(rules []matcher_adapter.ValuedRule) (matcher_adapter.ValuedSnapshot, error) {
		if len(rules) > 0 && rules[0].Rule == "full:new.example" {
			return nil, errors.New("injected valued build failure")
		}
		return builder.build(rules)
	}

	exporter := &slice0DetailedExporter{entries: []data_provider.RuleEntry{{Rule: "full:old.example", SourceName: "old"}}}
	dm := newSlice4Mapper(t, map[string]*slice0DetailedExporter{"base": exporter}, []RuleConfig{{Tag: "base", Mark: 3}})
	if _, _, ok := dm.FastMatch("old.example."); !ok {
		t.Fatal("initial Rust generation did not match")
	}

	exporter.replace([]data_provider.RuleEntry{{Rule: "full:new.example", SourceName: "new"}})
	deadline := time.Now().Add(3 * time.Second)
	for {
		if _, _, ok := dm.FastMatch("new.example."); ok {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("rebuild failure did not publish Go candidate")
		}
		time.Sleep(10 * time.Millisecond)
	}
	if _, _, ok := dm.FastMatch("old.example."); ok {
		t.Fatal("stale Rust generation remained visible after failed rebuild")
	}
	dm.generationMu.RLock()
	if dm.rustGeneration != nil {
		dm.generationMu.RUnlock()
		t.Fatal("failed rebuild retained a Rust generation")
	}
	dm.generationMu.RUnlock()
	if first.closed.Load() == false {
		t.Fatal("retired Rust generation was not closed")
	}
	_ = dm.Close()
}

func TestSlice4RuntimeFailureFallsBackToSameGenerationGo(t *testing.T) {
	oldBuilder := valuedSnapshotBuilder
	t.Cleanup(func() { valuedSnapshotBuilder = oldBuilder })

	failing := &slice4ValuedSnapshot{matchErr: errors.New("runtime failure")}
	valuedSnapshotBuilder = func([]matcher_adapter.ValuedRule) (matcher_adapter.ValuedSnapshot, error) {
		return failing, nil
	}
	exporter := &slice0DetailedExporter{entries: []data_provider.RuleEntry{{
		Rule: "full:runtime.example", SourceName: "runtime-source",
	}}}
	dm := newSlice4Mapper(t, map[string]*slice0DetailedExporter{"base": exporter}, []RuleConfig{{Tag: "base", Mark: 3}})
	marks, _, ok := dm.FastMatch("runtime.example.")
	if !ok || !sameSlice0Uint8Set(marks, []uint8{3}) {
		t.Fatalf("same-generation Go fallback = %v, %v", marks, ok)
	}
	if failing.matches.Load() == 0 {
		t.Fatal("Rust generation was not attempted")
	}
	_ = dm.Close()
}

func TestSlice4EmptyRulesetPublishesValidGeneration(t *testing.T) {
	oldBuilder := valuedSnapshotBuilder
	t.Cleanup(func() { valuedSnapshotBuilder = oldBuilder })
	empty := &slice4ValuedSnapshot{results: map[string]matcher_adapter.ValuedResult{}}
	valuedSnapshotBuilder = func(rules []matcher_adapter.ValuedRule) (matcher_adapter.ValuedSnapshot, error) {
		if len(rules) != 0 {
			t.Fatalf("empty mapper sent valued rules: %+v", rules)
		}
		return empty, nil
	}
	dm := newSlice4Mapper(t, map[string]*slice0DetailedExporter{"base": {}}, []RuleConfig{{Tag: "base", Mark: 3}})
	if _, _, ok := dm.FastMatch("empty.example."); ok {
		t.Fatal("empty mapper unexpectedly matched")
	}
	dm.QuickAdd("empty.example", []uint8{13}, "quick")
	if marks, tags, ok := dm.FastMatch("empty.example."); !ok || !sameSlice0Uint8Set(marks, []uint8{13}) || tags != "quick" {
		t.Fatalf("empty mapper QuickAdd = %v, %q, %v", marks, tags, ok)
	}
	if err := dm.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestSlice4GenerationLockKeepsLookupAndReplacementSeparate(t *testing.T) {
	oldBuilder := valuedSnapshotBuilder
	t.Cleanup(func() { valuedSnapshotBuilder = oldBuilder })

	oldSnapshot := &slice4ValuedSnapshot{
		results: map[string]matcher_adapter.ValuedResult{
			"old.example.": {Matched: true, FastMarks: []uint8{3}},
		},
		entered: make(chan struct{}),
		release: make(chan struct{}),
	}
	newSnapshot := &slice4ValuedSnapshot{results: map[string]matcher_adapter.ValuedResult{
		"new.example.": {Matched: true, FastMarks: []uint8{5}},
	}}
	var buildN atomic.Int64
	valuedSnapshotBuilder = func([]matcher_adapter.ValuedRule) (matcher_adapter.ValuedSnapshot, error) {
		if buildN.Add(1) == 1 {
			return oldSnapshot, nil
		}
		return newSnapshot, nil
	}

	exporter := &slice0DetailedExporter{entries: []data_provider.RuleEntry{{Rule: "full:old.example"}}}
	dm := newSlice4Mapper(t, map[string]*slice0DetailedExporter{"base": exporter}, []RuleConfig{{Tag: "base", Mark: 3}})
	lookupDone := make(chan struct{})
	go func() {
		_, _, _ = dm.FastMatch("old.example.")
		close(lookupDone)
	}()
	<-oldSnapshot.entered

	exporter.replace([]data_provider.RuleEntry{{Rule: "full:new.example"}})
	time.Sleep(1200 * time.Millisecond)
	if oldSnapshot.closeN.Load() != 0 {
		t.Fatal("retired generation closed while a Rust lookup was still active")
	}
	close(oldSnapshot.release)
	select {
	case <-lookupDone:
	case <-time.After(2 * time.Second):
		t.Fatal("blocked generation lookup did not finish")
	}

	deadline := time.Now().Add(2 * time.Second)
	for {
		if _, _, ok := dm.FastMatch("new.example."); ok {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("replacement generation was not published")
		}
		time.Sleep(10 * time.Millisecond)
	}
	if oldSnapshot.closeN.Load() != 1 || newSnapshot.closeN.Load() != 0 {
		t.Fatalf("generation close counts = old:%d new:%d", oldSnapshot.closeN.Load(), newSnapshot.closeN.Load())
	}
	if err := dm.Close(); err != nil {
		t.Fatal(err)
	}
	if newSnapshot.closeN.Load() != 1 {
		t.Fatalf("replacement generation close count = %d", newSnapshot.closeN.Load())
	}
}
