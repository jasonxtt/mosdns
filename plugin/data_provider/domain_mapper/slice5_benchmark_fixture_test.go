package domain_mapper

import (
	"fmt"
	"strings"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
)

type slice5MapperBenchmarkFixture struct {
	rules        []matcher_adapter.ValuedRule
	queries      []string
	fixtureBytes int
	resultBytes  int
}

func newSlice5MapperBenchmarkFixture() slice5MapperBenchmarkFixture {
	rules := make([]matcher_adapter.ValuedRule, 0, 512)
	ruleText := make([]string, 0, 512)
	for i := 0; i < 128; i++ {
		entries := []matcher_adapter.ValuedRule{
			{
				Rule:          fmt.Sprintf("full:full-%04d.bench.example", i),
				FastMarks:     1 << uint(i%8),
				CtxMarks:      []uint32{uint32(i + 1)},
				JoinedTags:    "full",
				JoinedSources: "fixture-full",
			},
			{
				Rule:          fmt.Sprintf("domain:suffix-%04d.bench.example", i),
				FastMarks:     1 << uint((i+1)%8),
				CtxMarks:      []uint32{uint32(i + 101)},
				JoinedTags:    "suffix",
				JoinedSources: "fixture-suffix",
			},
			{
				Rule:          fmt.Sprintf(`regexp:^regex-%04d\.bench\.example$`, i),
				FastMarks:     1 << uint((i+2)%8),
				CtxMarks:      []uint32{uint32(i + 201)},
				JoinedTags:    "regex",
				JoinedSources: "fixture-regex",
			},
			{
				Rule:          fmt.Sprintf("keyword:keyword-%04d", i),
				FastMarks:     1 << uint((i+3)%8),
				CtxMarks:      []uint32{uint32(i + 301)},
				JoinedTags:    "keyword",
				JoinedSources: "fixture-keyword",
			},
		}
		rules = append(rules, entries...)
		for _, entry := range entries {
			ruleText = append(ruleText, entry.Rule)
		}
	}

	return slice5MapperBenchmarkFixture{
		rules:        rules,
		queries:      []string{"suffix-0042.bench.example.", "regex-0042.bench.example.", "keyword-0042.bench.example.", "suffix-0043.bench.example."},
		fixtureBytes: len(strings.Join(ruleText, "\n")),
		resultBytes:  len("full") + len("fixture-full") + 1 + 4,
	}
}

func slice5MapperBenchmarkAggregation(fixture slice5MapperBenchmarkFixture) ruleAggregation {
	agg := ruleAggregation{
		fastMarkMap: make(map[string]uint64, len(fixture.rules)),
		ctxMarkMap:  make(map[string]map[uint32]struct{}, len(fixture.rules)),
		tagMap:      make(map[string]string, len(fixture.rules)),
		sourceMap:   make(map[string]string, len(fixture.rules)),
		ruleOrder:   make([]string, 0, len(fixture.rules)),
		valuedRules: append([]matcher_adapter.ValuedRule(nil), fixture.rules...),
		totalRules:  len(fixture.rules),
	}
	for _, rule := range fixture.rules {
		agg.ruleOrder = append(agg.ruleOrder, rule.Rule)
		agg.fastMarkMap[rule.Rule] = rule.FastMarks
		agg.tagMap[rule.Rule] = rule.JoinedTags
		agg.sourceMap[rule.Rule] = rule.JoinedSources
		if len(rule.CtxMarks) > 0 {
			agg.ctxMarkMap[rule.Rule] = make(map[uint32]struct{}, len(rule.CtxMarks))
			for _, mark := range rule.CtxMarks {
				agg.ctxMarkMap[rule.Rule][mark] = struct{}{}
			}
		}
	}
	return agg
}

func TestSlice5MapperBenchmarkFixture(t *testing.T) {
	fixture := newSlice5MapperBenchmarkFixture()
	if len(fixture.rules) != 512 {
		t.Fatalf("fixture rules = %d, want 512", len(fixture.rules))
	}
	if len(fixture.queries) != 4 {
		t.Fatalf("fixture queries = %d, want 4", len(fixture.queries))
	}
	if fixture.fixtureBytes != 15231 || fixture.resultBytes != 21 {
		t.Fatalf("fixture sizes = input:%d result:%d, want input:15231 result:21", fixture.fixtureBytes, fixture.resultBytes)
	}
}
