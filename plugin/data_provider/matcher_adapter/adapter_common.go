package matcher_adapter

import (
	"errors"
	"fmt"
)

const BackendEnv = "MOSDNS_MATCHER_BACKEND"

// RustDomainInputSupported reports whether the Rust matcher can apply its
// ASCII-only normalization contract to one domain query without changing Go
// semantics. Callers should use the paired Go matcher when it returns false.
func RustDomainInputSupported(value string) bool {
	for i := 0; i < len(value); i++ {
		if value[i] >= 0x80 {
			return false
		}
	}
	return true
}

// RustDomainRulesSupported is the cheap Go-side defense-in-depth check before
// sending a complete domain rule batch through cgo.
func RustDomainRulesSupported(rules []string) bool {
	for _, rule := range rules {
		if !RustDomainInputSupported(rule) {
			return false
		}
	}
	return true
}

// RustValuedRulesSupported is the equivalent preflight for valued domain rules.
func RustValuedRulesSupported(rules []ValuedRule) bool {
	for _, rule := range rules {
		if !RustDomainInputSupported(rule.Rule) {
			return false
		}
	}
	return true
}

type ErrorClass uint8

const (
	ErrorClassInvalidArgument ErrorClass = iota + 1
	ErrorClassClosed
	ErrorClassRuntime
	ErrorClassCircuitBroken
)

// Error is a classified failure from the experimental matcher runtime.
type Error struct {
	Operation string
	Code      uint32
	Class     ErrorClass
}

func (e *Error) Error() string {
	return fmt.Sprintf("%s failed: status=%d", e.Operation, e.Code)
}

// IsCircuitBreakerError reports failures that disable a snapshot's Rust path.
func IsCircuitBreakerError(err error) bool {
	var runtimeErr *Error
	if !errors.As(err, &runtimeErr) {
		return false
	}
	return runtimeErr.Class == ErrorClassRuntime || runtimeErr.Class == ErrorClassCircuitBroken
}

type DomainSnapshot interface {
	Match(string) (bool, error)
	Len() (uint64, error)
	Close() error
}

type IPSnapshot interface {
	Match(string) (bool, error)
	Len() (uint64, error)
	Close() error
}

// ValuedRule is one ordered domain rule and the metadata that should be
// returned when it participates in a match. The adapter owns the wire
// encoding; callers only construct typed records.
type ValuedRule struct {
	Rule          string
	FastMarks     uint64
	CtxMarks      []uint32
	JoinedTags    string
	JoinedSources string
}

// ValuedResult is the decoded result of a valued-domain snapshot lookup.
type ValuedResult struct {
	Matched       bool
	FastMarks     []uint8
	CtxMarks      []uint32
	JoinedTags    string
	JoinedSources string
}

// ValuedSnapshot is an immutable valued-domain matcher generation.
type ValuedSnapshot interface {
	Match(string) (ValuedResult, error)
	Len() (uint64, error)
	Close() error
}
