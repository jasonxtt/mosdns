package coremain

import "testing"

func TestDomainGenerationEnabledForTag(t *testing.T) {
	domainGenerationSettingsMu.Lock()
	original := domainGenerationSettingsCache
	domainGenerationSettingsCache = DomainGenerationSettings{
		Enabled:        true,
		RememberDirect: false,
		RememberProxy:  true,
		NoV4:           false,
		NoV6:           true,
	}
	domainGenerationSettingsMu.Unlock()
	defer func() {
		domainGenerationSettingsMu.Lock()
		domainGenerationSettingsCache = original
		domainGenerationSettingsMu.Unlock()
	}()

	cases := []struct {
		tag  string
		want bool
	}{
		{tag: "top_domains", want: true},
		{tag: "my_sv4list", want: true},
		{tag: "my_realiplist", want: false},
		{tag: "my_fakeiplist", want: true},
		{tag: "my_nov4list", want: false},
		{tag: "my_nov6list", want: true},
		{tag: "my_nodenov4list", want: true},
	}

	for _, tc := range cases {
		if got := DomainGenerationEnabledForTag(tc.tag); got != tc.want {
			t.Fatalf("DomainGenerationEnabledForTag(%q) = %v, want %v", tc.tag, got, tc.want)
		}
	}
}

func TestDomainGenerationFlushTargets(t *testing.T) {
	before := DomainGenerationSettings{
		Enabled:        true,
		RememberDirect: true,
		RememberProxy:  true,
		NoV4:           true,
		NoV6:           true,
	}
	after := DomainGenerationSettings{
		Enabled:        false,
		RememberDirect: false,
		RememberProxy:  true,
		NoV4:           true,
		NoV6:           false,
	}

	got := DomainGenerationFlushTargets(before, after)
	want := map[string]struct{}{
		"top_domains":     {},
		"my_sv4list":      {},
		"my_realiplist":   {},
		"my_fakeiplist":   {},
		"my_nov4list":     {},
		"my_nov6list":     {},
		"my_nodenov4list": {},
		"my_nodenov6list": {},
	}

	if len(got) != len(want) {
		t.Fatalf("flush target count = %d, want %d (%v)", len(got), len(want), got)
	}
	for _, tag := range got {
		if _, ok := want[tag]; !ok {
			t.Fatalf("unexpected flush target %q in %v", tag, got)
		}
	}
}
