package matcher_adapter

import (
	"regexp"
	"testing"
)

func TestRustDomainInputSupportedIsASCIIOnly(t *testing.T) {
	tests := []struct {
		name string
		in   string
		want bool
	}{
		{name: "ascii", in: "Example.COM.", want: true},
		{name: "non_ascii", in: "例.example.", want: false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := RustDomainInputSupported(tt.in); got != tt.want {
				t.Fatalf("RustDomainInputSupported(%q) = %v, want %v", tt.in, got, tt.want)
			}
		})
	}
}

func TestRustDomainRulePreflightIsASCIIOnly(t *testing.T) {
	if !RustDomainRulesSupported([]string{"full:example.com", `regexp:^ads\\.example$`}) {
		t.Fatal("ASCII domain rules were rejected")
	}
	if RustDomainRulesSupported([]string{"full:例.example"}) {
		t.Fatal("non-ASCII domain rule was accepted")
	}
	if RustValuedRulesSupported([]ValuedRule{{Rule: "domain:example.com"}}) == false {
		t.Fatal("ASCII valued rule was rejected")
	}
	if RustValuedRulesSupported([]ValuedRule{{Rule: "domain:例.example"}}) {
		t.Fatal("non-ASCII valued rule was accepted")
	}
}

func TestGoRegexpOracleCompileableShorthandIsASCII(t *testing.T) {
	re := regexp.MustCompile(`^\w+$`)
	if re.MatchString("例") {
		t.Fatal(`Go regexp \\w unexpectedly matched a non-ASCII label`)
	}
	if !re.MatchString("abc123") {
		t.Fatal(`Go regexp \\w failed to match an ASCII label`)
	}
}

func TestGoRegexpOracleTreatsClassOperatorsAsLiterals(t *testing.T) {
	re := regexp.MustCompile(`^[ab&&b]+$`)
	if !re.MatchString("a") {
		t.Fatal(`Go regexp class operators should be ordinary class characters`)
	}
	if !regexp.MustCompile(`[]&&]`).MatchString("]") {
		t.Fatal(`Go regexp should treat a class-leading ] as a literal class character`)
	}
}
