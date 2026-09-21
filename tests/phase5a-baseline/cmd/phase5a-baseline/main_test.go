package main

import (
	"encoding/json"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	"github.com/miekg/dns"
)

func TestReadWorkloadExpansionAndFiltering(t *testing.T) {
	path := t.TempDir() + "/workload.jsonl"
	content := `{"case_id":"a","scenario":"w1","transport":"udp","qname":"a.test.","qtype":"A","expected_rcode":0,"expected_answer_class":"A","expected_answer":"198.51.100.10","expected_route_class":"forward","request_deadline_ms":500,"weight":2}
{"case_id":"b","scenario":"w1","transport":"tcp","qname":"b.test.","qtype":"A","expected_rcode":3,"expected_answer_class":"NXDOMAIN","expected_route_class":"forward","weight":1}
`
	if err := writeFile(path, content); err != nil {
		t.Fatal(err)
	}
	cases, err := readWorkload(path, "w1", "udp")
	if err != nil {
		t.Fatal(err)
	}
	if len(cases) != 2 || cases[0].CaseID != "a" || cases[1].CaseID != "a" {
		t.Fatalf("unexpected expanded cases: %#v", cases)
	}
}

func TestResponseMatchesExpectedPositiveAndNegative(t *testing.T) {
	query := new(dns.Msg)
	query.SetQuestion("ok.test.", dns.TypeA)
	query.Id = 7
	positive := new(dns.Msg)
	positive.SetReply(query)
	positive.Answer = []dns.RR{&dns.A{Hdr: dns.RR_Header{Name: "ok.test.", Rrtype: dns.TypeA, Class: dns.ClassINET}, A: net.ParseIP("198.51.100.10").To4()}}
	positiveCase := workloadCase{ExpectedRCode: 0, ExpectedAnswerClass: "A", ExpectedAnswer: "198.51.100.10"}
	if !responseMatches(positive, query, positiveCase) {
		t.Fatal("positive answer should be correct")
	}
	negative := new(dns.Msg)
	negative.SetReply(query)
	negative.Rcode = dns.RcodeNameError
	negativeCase := workloadCase{ExpectedRCode: dns.RcodeNameError, ExpectedAnswerClass: "NXDOMAIN"}
	if !responseMatches(negative, query, negativeCase) {
		t.Fatal("expected negative answer should be correct")
	}
	positive.Answer[0].(*dns.A).A = net.ParseIP("198.51.100.99").To4()
	if responseMatches(positive, query, positiveCase) {
		t.Fatal("wrong answer must not be counted as correct")
	}
}

func TestFixtureAnswerTable(t *testing.T) {
	tests := []struct {
		id, qname, ip string
		rcode         int
	}{
		{"forward", "ok.forward.test.", "198.51.100.10", 0},
		{"forward", "negative.forward.test.", "", dns.RcodeNameError},
		{"route-b", "ip-hit.test.", "192.0.2.10", 0},
		{"route-b", "ip-miss.test.", "192.0.2.30", 0},
	}
	for _, tt := range tests {
		got := fixtureAnswer(tt.id, tt.qname, dns.TypeA)
		if got.ip != tt.ip || got.rcode != tt.rcode {
			t.Fatalf("fixtureAnswer(%q,%q) = %#v, want ip=%q rcode=%d", tt.id, tt.qname, got, tt.ip, tt.rcode)
		}
	}
}

func TestPercentileUsesSortedSamples(t *testing.T) {
	values := []int64{1, 2, 3, 4, 5}
	if got := percentile(values, .50); got != 3 {
		t.Fatalf("p50=%d", got)
	}
	if got := percentile(values, .99); got != 5 {
		t.Fatalf("p99=%d", got)
	}
}

func TestStageCountersClassificationNamesAreStable(t *testing.T) {
	var got stageCounters
	got.CorrectOnTime = 1
	got.ExpectedNegativeOnTime = 1
	got.CorrectLate = 2
	if !reflect.DeepEqual(got, stageCounters{CorrectOnTime: 1, ExpectedNegativeOnTime: 1, CorrectLate: 2}) {
		t.Fatal("counter fields changed unexpectedly")
	}
	_ = time.Second
}

func TestStageFailureGateRejectsNonCorrectOutcomes(t *testing.T) {
	if !hasStageFailure(stageCounters{WrongResponse: 1}) {
		t.Fatal("wrong response must fail smoke gate")
	}
	if !hasStageFailure(stageCounters{Timeout: 1}) {
		t.Fatal("timeout must fail smoke gate")
	}
	if hasStageFailure(stageCounters{CorrectOnTime: 1, ExpectedNegativeOnTime: 1}) {
		t.Fatal("correct expected negative must not fail smoke gate")
	}
}

func TestVerifyRoutingCountersRejectsMismatch(t *testing.T) {
	dir := t.TempDir()
	routeAPath := filepath.Join(dir, "route-a.json")
	routeBPath := filepath.Join(dir, "route-b.json")
	routeCPath := filepath.Join(dir, "route-c.json")
	writeCounterTestFile(t, routeAPath, "route-a", map[string]int64{
		"domain-hit.test.|A": 1,
		"ip-hit.test.|A":     1,
	})
	writeCounterTestFile(t, routeBPath, "route-b", map[string]int64{})
	writeCounterTestFile(t, routeCPath, "route-c", map[string]int64{})
	cases := []workloadCase{
		{CaseID: "domain-hit", QName: "domain-hit.test.", QType: "A", ExpectedRouteClass: "DOMAIN_HIT"},
		{CaseID: "ip-rule-hit", QName: "ip-hit.test.", QType: "A", ExpectedRouteClass: "IP_RULE_HIT"},
	}
	if err := verifyRoutingCounters(cases, routeAPath, routeBPath, routeCPath); err == nil {
		t.Fatal("route mismatch must fail counter verification")
	}
}

func writeCounterTestFile(t *testing.T, path, upstream string, counts map[string]int64) {
	t.Helper()
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.NewEncoder(f).Encode(counterFile{Upstream: upstream, Counts: counts}); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
}

func writeFile(path, content string) error {
	return os.WriteFile(path, []byte(content), 0644)
}
