package domain_mapper

import (
	"regexp"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
)

func result(marks ...uint8) *MatchResult {
	return &MatchResult{FastMarks: marks}
}

func marksSet(res *MatchResult) map[uint8]struct{} {
	out := make(map[uint8]struct{}, len(res.FastMarks))
	for _, mark := range res.FastMarks {
		out[mark] = struct{}{}
	}
	return out
}

func requireMarks(t *testing.T, res *MatchResult, want ...uint8) {
	t.Helper()
	if res == nil {
		t.Fatal("expected match result, got nil")
	}
	got := marksSet(res)
	for _, mark := range want {
		if _, ok := got[mark]; !ok {
			t.Fatalf("expected fast_mark %d in %+v", mark, res.FastMarks)
		}
	}
	if len(got) != len(want) {
		t.Fatalf("expected %d unique marks, got %+v", len(want), res.FastMarks)
	}
}

func TestCompiledMatcherMergesKeywordAndDomainMatches(t *testing.T) {
	base := domain.NewMixMatcher[*MatchResult]()
	if err := base.Add("domain:browserleaks.com", result(14, 16)); err != nil {
		t.Fatalf("add domain rule: %v", err)
	}

	cm := &compiledMatcher{
		domainRules: base,
		overlapRules: []overlapRule{
			{keyword: "browserleak", res: result(7)},
		},
	}

	res, ok := cm.match("browserleaks.com.")
	if !ok {
		t.Fatal("expected browserleaks.com to match")
	}
	requireMarks(t, res, 7, 14, 16)
}

func TestCompiledMatcherMergesRegexWithoutBaseDomainHit(t *testing.T) {
	cm := &compiledMatcher{
		domainRules: domain.NewMixMatcher[*MatchResult](),
		overlapRules: []overlapRule{
			{regex: regexp.MustCompile(`^browserleaks\.`), res: result(1)},
		},
	}

	res, ok := cm.match("browserleaks.com.")
	if !ok {
		t.Fatal("expected regex overlap match")
	}
	requireMarks(t, res, 1)
}

func TestLookupMatchResultMergesHotMapAndOverlapRules(t *testing.T) {
	base := domain.NewMixMatcher[*MatchResult]()
	if err := base.Add("domain:browserleaks.com", result(14, 16)); err != nil {
		t.Fatalf("add domain rule: %v", err)
	}

	dm := &DomainMapper{}
	dm.matcher.Store(&compiledMatcher{
		domainRules: base,
		overlapRules: []overlapRule{
			{keyword: "browserleak", res: result(7)},
		},
	})
	dm.hotMap.Store("browserleaks.com.", result(11))

	res, ok := dm.lookupMatchResult("browserleaks.com.")
	if !ok {
		t.Fatal("expected lookup match")
	}
	requireMarks(t, res, 7, 11, 14, 16)
}
