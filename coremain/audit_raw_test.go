package coremain

import (
	"testing"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/miekg/dns"
)

func TestAuditCollectorDecodesRawResponseOnDemand(t *testing.T) {
	q := new(dns.Msg)
	q.SetQuestion("raw-audit.example.", dns.TypeA)
	r := new(dns.Msg)
	r.SetReply(q)
	r.Rcode = dns.RcodeSuccess
	r.RecursionAvailable = true
	rr, err := dns.NewRR("raw-audit.example. 41 IN A 192.0.2.8")
	if err != nil {
		t.Fatal(err)
	}
	r.Answer = []dns.RR{rr}
	wire, err := r.Pack()
	if err != nil {
		t.Fatal(err)
	}

	qCtx := query_context.NewContext(q)
	qCtx.SetRawResponse(wire)
	collector := NewAuditCollector(1)
	collector.processBatch([]*auditContext{{Ctx: qCtx}})

	logs := collector.GetLogs()
	if len(logs) != 1 {
		t.Fatalf("logs=%d, want 1", len(logs))
	}
	if logs[0].ResponseCode != "NOERROR" || len(logs[0].Answers) != 1 {
		t.Fatalf("raw response audit mismatch: %#v", logs[0])
	}
	if logs[0].Answers[0].TTL != 41 || logs[0].Answers[0].Data != "192.0.2.8" {
		t.Fatalf("raw response answer mismatch: %#v", logs[0].Answers[0])
	}
}
