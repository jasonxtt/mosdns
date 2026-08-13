package server_handler

import (
	"context"
	"encoding/binary"
	"strings"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/pool"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/pkg/server"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
)

func rawResponse(t *testing.T, rcode int, answers []dns.RR) []byte {
	t.Helper()
	q := new(dns.Msg)
	q.SetQuestion("raw.example.org.", dns.TypeA)
	r := new(dns.Msg)
	r.SetReply(q)
	r.Id = 0x1111
	r.Rcode = rcode
	r.Answer = answers
	wire, err := r.Pack()
	if err != nil {
		t.Fatal(err)
	}
	return wire
}

func rawEntry(payload []byte) sequence.Executable {
	return sequence.ExecutableFunc(func(_ context.Context, qCtx *query_context.Context) error {
		qCtx.SetRawResponse(payload)
		return nil
	})
}

func rawRequest(edns bool) *dns.Msg {
	q := new(dns.Msg)
	q.SetQuestion("raw.example.org.", dns.TypeA)
	q.Id = 0xcafe
	if edns {
		q.SetEdns0(1232, true)
	}
	return q
}

func TestRawResponseFastPathPreservesUDPHTTPAndTCPFraming(t *testing.T) {
	rr, err := dns.NewRR("raw.example.org. 42 IN A 192.0.2.1")
	if err != nil {
		t.Fatal(err)
	}
	wire := rawResponse(t, dns.RcodeSuccess, []dns.RR{rr})
	tests := []struct {
		name      string
		meta      server.QueryMeta
		packer    func(*dns.Msg) (*[]byte, error)
		tcpFramed bool
	}{
		{name: "udp", meta: server.QueryMeta{FromUDP: true}, packer: pool.PackBuffer},
		{name: "http", meta: server.QueryMeta{UrlPath: "/dns-query"}, packer: pool.PackBuffer},
		{name: "tcp", meta: server.QueryMeta{}, packer: pool.PackTCPBuffer, tcpFramed: true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			h := NewEntryHandler(EntryHandlerOpts{Entry: rawEntry(wire)})
			q := rawRequest(false)
			payload := h.Handle(context.Background(), q, tt.meta, tt.packer)
			if payload == nil {
				t.Fatal("nil payload")
			}
			defer pool.ReleaseBuf(payload)
			body := *payload
			if tt.tcpFramed {
				if len(body) < 2 || int(binary.BigEndian.Uint16(body[:2])) != len(body)-2 {
					t.Fatalf("invalid TCP framing: %x", body)
				}
				body = body[2:]
			}
			r := new(dns.Msg)
			if err := r.Unpack(body); err != nil {
				t.Fatal(err)
			}
			if r.Id != q.Id || !r.RecursionAvailable || r.Rcode != dns.RcodeSuccess {
				t.Fatalf("header mismatch: id=%x ra=%v rcode=%d", r.Id, r.RecursionAvailable, r.Rcode)
			}
		})
	}
}

func TestRawResponseEDNSAndUDPTruncationUseMessagePath(t *testing.T) {
	t.Run("edns", func(t *testing.T) {
		wire := rawResponse(t, dns.RcodeNameError, nil)
		h := NewEntryHandler(EntryHandlerOpts{Entry: rawEntry(wire)})
		q := rawRequest(true)
		payload := h.Handle(context.Background(), q, server.QueryMeta{FromUDP: true}, pool.PackBuffer)
		if payload == nil {
			t.Fatal("nil payload")
		}
		defer pool.ReleaseBuf(payload)
		r := new(dns.Msg)
		if err := r.Unpack(*payload); err != nil {
			t.Fatal(err)
		}
		if opt := r.IsEdns0(); opt == nil || !opt.Do() {
			t.Fatalf("response OPT did not preserve DO bit: %#v", opt)
		}
		if r.Rcode != dns.RcodeNameError {
			t.Fatalf("rcode=%d, want NXDOMAIN", r.Rcode)
		}
	})

	t.Run("truncate", func(t *testing.T) {
		answers := make([]dns.RR, 0, 8)
		for i := 0; i < 8; i++ {
			rr, err := dns.NewRR("raw.example.org. 60 IN TXT \"" + strings.Repeat("x", 120) + "\"")
			if err != nil {
				t.Fatal(err)
			}
			answers = append(answers, rr)
		}
		wire := rawResponse(t, dns.RcodeSuccess, answers)
		if len(wire) <= dns.MinMsgSize {
			t.Fatalf("fixture is not oversized: %d", len(wire))
		}
		h := NewEntryHandler(EntryHandlerOpts{Entry: rawEntry(wire)})
		payload := h.Handle(context.Background(), rawRequest(false), server.QueryMeta{FromUDP: true}, pool.PackBuffer)
		if payload == nil {
			t.Fatal("nil payload")
		}
		defer pool.ReleaseBuf(payload)
		if len(*payload) > dns.MinMsgSize {
			t.Fatalf("UDP response was not truncated: %d", len(*payload))
		}
		r := new(dns.Msg)
		if err := r.Unpack(*payload); err != nil {
			t.Fatal(err)
		}
		if !r.Truncated {
			t.Fatal("truncated response is missing TC flag")
		}
	})
}

func TestRawResponseSupportsNegativeAndEmptyResponses(t *testing.T) {
	for _, rcode := range []int{dns.RcodeNameError, dns.RcodeServerFailure, dns.RcodeSuccess} {
		t.Run(dns.RcodeToString[rcode], func(t *testing.T) {
			h := NewEntryHandler(EntryHandlerOpts{Entry: rawEntry(rawResponse(t, rcode, nil))})
			payload := h.Handle(context.Background(), rawRequest(false), server.QueryMeta{FromUDP: true}, pool.PackBuffer)
			if payload == nil {
				t.Fatal("nil payload")
			}
			defer pool.ReleaseBuf(payload)
			r := new(dns.Msg)
			if err := r.Unpack(*payload); err != nil {
				t.Fatal(err)
			}
			if r.Rcode != rcode || len(r.Answer) != 0 {
				t.Fatalf("response mismatch: rcode=%d answers=%d", r.Rcode, len(r.Answer))
			}
		})
	}
}

func TestMalformedRawResponseReturnsServfail(t *testing.T) {
	h := NewEntryHandler(EntryHandlerOpts{Entry: rawEntry([]byte{1, 2, 3})})
	q := rawRequest(false)
	payload := h.Handle(context.Background(), q, server.QueryMeta{FromUDP: true}, pool.PackBuffer)
	if payload == nil {
		t.Fatal("nil payload")
	}
	defer pool.ReleaseBuf(payload)
	r := new(dns.Msg)
	if err := r.Unpack(*payload); err != nil {
		t.Fatal(err)
	}
	if r.Rcode != dns.RcodeServerFailure {
		t.Fatalf("rcode=%d, want SERVFAIL", r.Rcode)
	}
}
