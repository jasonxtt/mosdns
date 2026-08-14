//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

package matcher_adapter

import "testing"

const (
	valuedRuleBatchVersion = 1
	valuedResultVersion    = 1
)

func TestValuedAdapterEncodingAndResultValidation(t *testing.T) {
	encoded, err := encodeValuedRules([]ValuedRule{{
		Rule:          "domain:example.com",
		FastMarks:     1 << 2,
		CtxMarks:      []uint32{30, 70},
		JoinedTags:    "base|light",
		JoinedSources: "base-source",
	}})
	if err != nil {
		t.Fatal(err)
	}
	if len(encoded) == 0 || encoded[0] != valuedRuleBatchVersion {
		t.Fatalf("encoded valued rule batch = %v", encoded)
	}

	result, err := decodeValuedResult([]byte{
		valuedResultVersion,
		0x04, 0, 0, 0, 0, 0, 0, 0, // fast mark 3
		0x02, 0, 0, 0,
		30, 0, 0, 0,
		70, 0, 0, 0,
		0x04, 0, 0, 0, 't', 'a', 'g', 's',
		0x06, 0, 0, 0, 's', 'o', 'u', 'r', 'c', 'e',
	})
	if err != nil {
		t.Fatal(err)
	}
	if !result.Matched || len(result.FastMarks) != 1 || result.FastMarks[0] != 3 || result.JoinedTags != "tags" || result.JoinedSources != "source" {
		t.Fatalf("decoded valued result = %+v", result)
	}
	if _, err := decodeValuedResult([]byte{valuedResultVersion, 0}); err == nil {
		t.Fatal("truncated valued result unexpectedly decoded")
	}
	if _, err := decodeValuedResult([]byte{
		valuedResultVersion,
		0, 0, 0, 0, 0, 0, 0, 128,
		0, 0, 0, 0,
		0, 0, 0, 0,
		0, 0, 0, 0,
	}); err == nil {
		t.Fatal("unsupported fast mark unexpectedly decoded")
	}
}
