/*
 * Copyright (C) 2020-2026, IrineSistiana
 *
 * Slice 0 golden fixtures for the Go domain matcher. These vectors freeze the
 * exact Go behavior that the Rust domain core (Slice 2) must reproduce, and
 * are the reference for the Go/Rust parity harness. Do not weaken them to make
 * an implementation pass; update them only if a compatibility bug is confirmed
 * against the real contract.
 */

package domain

import (
	"reflect"
	"testing"
)

func TestGoldenNormalizeDomain(t *testing.T) {
	tests := []struct {
		in   string
		want string
	}{
		{"google.com", "google.com"},
		{"google.com.", "google.com"},
		{"GOOGLE.com.", "google.com"},
		{"Google.COM", "google.com"},
		{"a.b.C.", "a.b.c"},
		{"例.EXAMPLE.", "例.example"},
		{"Ä.EXAMPLE.", "ä.example"},
		{"İ.EXAMPLE.", "i.example"},
		{"", ""},
		{".", ""},
		{"..", "."}, // TrimDot removes exactly one trailing dot
		{"EXAMPLE.", "example"},
		{"EXAMPLE..", "example."},
	}
	for _, tt := range tests {
		if got := NormalizeDomain(tt.in); got != tt.want {
			t.Errorf("NormalizeDomain(%q) = %q, want %q", tt.in, got, tt.want)
		}
	}
}

type addRule struct {
	pattern string
	v       any
}

type expectQuery struct {
	q    string
	want bool
	val  any
}

type mixVector struct {
	name       string
	defaultTyp string
	adds       []addRule
	queries    []expectQuery
}

// buildMix creates a MixMatcher[any], applies the optional default type and the
// ordered rules, and returns it. addErr is non-nil when an Add must fail.
func buildMix(v mixVector) (m *MixMatcher[any], addErr error) {
	m = NewMixMatcher[any]()
	if v.defaultTyp != "" {
		m.SetDefaultMatcher(v.defaultTyp)
	}
	for _, r := range v.adds {
		if err := m.Add(r.pattern, r.v); err != nil {
			return nil, err
		}
	}
	return m, nil
}

func TestGoldenMixMatcher(t *testing.T) {
	vectors := []mixVector{
		{
			name:       "precedence full over domain",
			defaultTyp: MatcherDomain,
			adds: []addRule{
				{"full:exact.example", "full"},
				{"domain:example", "domain"},
			},
			queries: []expectQuery{
				{"exact.example", true, "full"},
				{"exact.example.", true, "full"}, // fqdn normalizes
				{"EXACT.EXAMPLE", true, "full"},  // case-insensitive
				{"sub.exact.example", true, "domain"},
				{"a.example", true, "domain"},
				{"example", true, "domain"},
				{"example.com", false, nil},
				{"xexample", false, nil},
			},
		},
		{
			name:       "domain wins over regex and keyword",
			defaultTyp: MatcherDomain,
			adds: []addRule{
				{"regexp:^exact\\.example$", "regex"},
				{"keyword:example", "keyword"},
				{"domain:example", "domain"},
			},
			queries: []expectQuery{
				{"exact.example", true, "domain"}, // domain matches suffix; wins over regex/keyword
				{"exact.example.", true, "domain"},
				{"myexample.net", true, "keyword"}, // keyword "example" is a substring
				{"exact.example.com", true, "keyword"},
				{"other", false, nil},
			},
		},
		{
			name:       "regex before keyword",
			defaultTyp: MatcherDomain,
			adds: []addRule{
				{"regexp:^re\\.", "regex"},
				{"keyword:re", "keyword"},
			},
			queries: []expectQuery{
				{"re.test", true, "regex"},
				{"xre.y", true, "keyword"},
				{"other", false, nil},
			},
		},
		{
			name:       "default domain adds suffix matcher",
			defaultTyp: MatcherDomain,
			adds:       []addRule{{"example.com", "domain"}},
			queries: []expectQuery{
				{"example.com", true, "domain"},
				{"a.example.com", true, "domain"},
				{"b.example.com.", true, "domain"},
				{"example.com.evil", false, nil},
				{"example", false, nil},
			},
		},
		{
			name:       "default full adds exact matcher",
			defaultTyp: MatcherFull,
			adds:       []addRule{{"example.com", "full"}},
			queries: []expectQuery{
				{"example.com", true, "full"},
				{"example.com.", true, "full"},
				{"a.example.com", false, nil},
				{"xexample.com", false, nil},
			},
		},
		{
			name:       "keyword is substring not boundary aware",
			defaultTyp: MatcherDomain,
			adds:       []addRule{{"keyword:example", "kw"}},
			queries: []expectQuery{
				{"example.com", true, "kw"},
				{"sub.example.org", true, "kw"},
				{"notexample.com", true, "kw"},
				{"examplex.com", true, "kw"},
				{"examp.com", false, nil},
			},
		},
		{
			name:       "root rule matches every domain",
			defaultTyp: MatcherDomain,
			adds:       []addRule{{"domain:.", "root"}},
			queries: []expectQuery{
				{"anything.example", true, "root"},
				{"a", true, "root"},
			},
		},
		{
			name:       "longer domain rule shadows shorter below its terminal node",
			defaultTyp: MatcherDomain,
			adds: []addRule{
				{"domain:example", "base"},
				{"domain:a.example", "specific"},
			},
			queries: []expectQuery{
				{"a.example", true, "specific"},
				{"b.a.example", true, "specific"},
				{"b.example", true, "base"},
			},
		},
		{
			name:       "duplicate full rule replaces value and keeps length",
			defaultTyp: MatcherDomain,
			adds: []addRule{
				{"full:exact.example", 1},
				{"full:exact.example", 2},
			},
			queries: []expectQuery{
				{"exact.example", true, 2},
			},
		},
		{
			name:       "duplicate domain rule replaces terminal value",
			defaultTyp: MatcherDomain,
			adds: []addRule{
				{"domain:example", 1},
				{"domain:example", 2},
			},
			queries: []expectQuery{
				{"sub.example", true, 2},
			},
		},
	}

	for _, v := range vectors {
		t.Run(v.name, func(t *testing.T) {
			m, err := buildMix(v)
			if err != nil {
				t.Fatalf("Add failed: %v", err)
			}
			for _, q := range v.queries {
				got, ok := m.Match(q.q)
				if ok != q.want {
					t.Errorf("Match(%q) ok=%v, want %v", q.q, ok, q.want)
					continue
				}
				if ok && !reflect.DeepEqual(got, q.val) {
					t.Errorf("Match(%q) val=%v, want %v", q.q, got, q.val)
				}
			}
		})
	}
}

func TestGoldenMixMatcherInvalidRules(t *testing.T) {
	t.Run("bad regexp returns error", func(t *testing.T) {
		m := NewMixMatcher[any]()
		m.SetDefaultMatcher(MatcherDomain)
		if err := m.Add("regexp:[", nil); err == nil {
			t.Fatal("Add(regexp:[) must fail to compile")
		}
	})
	t.Run("unsupported type returns error", func(t *testing.T) {
		m := NewMixMatcher[any]()
		m.SetDefaultMatcher(MatcherDomain)
		if err := m.Add("foo:bar", nil); err == nil {
			t.Fatal("Add(foo:bar) must return unsupported type error")
		}
	})
	t.Run("missing default type returns error for bare rule", func(t *testing.T) {
		m := NewMixMatcher[any]()
		if err := m.Add("bare", nil); err == nil {
			t.Fatal("Add(bare) with no default type must return ErrNodefaultMatcher")
		}
	})
	t.Run("typed rule works without default type", func(t *testing.T) {
		m := NewMixMatcher[any]()
		if err := m.Add("full:exact.example", nil); err != nil {
			t.Fatalf("Add(full:exact.example) must succeed: %v", err)
		}
		if _, ok := m.Match("exact.example"); !ok {
			t.Fatal("typed rule did not match")
		}
	})
	t.Run("keyword empty string matches everything", func(t *testing.T) {
		m := NewMixMatcher[any]()
		m.SetDefaultMatcher(MatcherDomain)
		if err := m.Add("keyword:", nil); err != nil {
			t.Fatalf("Add(keyword:) failed: %v", err)
		}
		if _, ok := m.Match("anything"); !ok {
			t.Fatal("empty keyword must match every input")
		}
	})
}
