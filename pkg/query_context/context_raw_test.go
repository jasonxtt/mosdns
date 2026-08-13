package query_context

import (
	"testing"

	"github.com/miekg/dns"
)

func packedResponse(t *testing.T, id uint16) []byte {
	t.Helper()
	q := new(dns.Msg)
	q.SetQuestion("raw.example.org.", dns.TypeA)
	r := new(dns.Msg)
	r.SetReply(q)
	r.Id = id
	r.Answer = []dns.RR{mustRR(t, "raw.example.org. 60 IN A 192.0.2.1")}
	wire, err := r.Pack()
	if err != nil {
		t.Fatal(err)
	}
	return wire
}

func mustRR(t *testing.T, value string) dns.RR {
	t.Helper()
	rr, err := dns.NewRR(value)
	if err != nil {
		t.Fatal(err)
	}
	return rr
}

func TestRawResponseIsDecodedOnlyWhenMessageIsRequested(t *testing.T) {
	q := new(dns.Msg)
	q.SetQuestion("raw.example.org.", dns.TypeA)
	q.Id = 0xabcd
	ctx := NewContext(q)
	wire := packedResponse(t, 0x1234)

	ctx.SetRawResponse(wire)
	if got := ctx.RawResponse(); len(got) == 0 {
		t.Fatal("raw response was not retained")
	}

	r := ctx.R()
	if r == nil || r.Id != q.Id || len(r.Answer) != 1 {
		t.Fatalf("lazy decode mismatch: %#v", r)
	}
	if got := ctx.RawResponse(); got != nil {
		t.Fatalf("raw response should transfer into decoded response, got %d bytes", len(got))
	}
}

func TestRawResponseCopyHasIndependentOwnership(t *testing.T) {
	q := new(dns.Msg)
	q.SetQuestion("raw.example.org.", dns.TypeA)
	ctx := NewContext(q)
	ctx.SetRawResponse(packedResponse(t, 0x2345))

	copyCtx := ctx.Copy()
	ctx.RawResponse()[0] ^= 0xff

	if copyCtx.RawResponse()[0] == ctx.RawResponse()[0] {
		t.Fatal("copied raw response aliases source storage")
	}
}

func TestSetResponseClearsRawResponse(t *testing.T) {
	q := new(dns.Msg)
	q.SetQuestion("raw.example.org.", dns.TypeA)
	ctx := NewContext(q)
	ctx.SetRawResponse(packedResponse(t, 0x3456))
	r := new(dns.Msg)
	r.SetReply(q)

	ctx.SetResponse(r)
	if ctx.RawResponse() != nil {
		t.Fatal("decoded response did not clear raw response")
	}
	if ctx.R() != r {
		t.Fatal("decoded response was not retained")
	}
}

func TestMalformedRawResponseDoesNotBecomeMessage(t *testing.T) {
	q := new(dns.Msg)
	q.SetQuestion("raw.example.org.", dns.TypeA)
	ctx := NewContext(q)
	ctx.SetRawResponse([]byte{1, 2, 3})

	if got := ctx.R(); got != nil {
		t.Fatalf("malformed response decoded unexpectedly: %#v", got)
	}
	if ctx.RawResponse() != nil {
		t.Fatal("malformed raw response should be discarded after decode failure")
	}
}
