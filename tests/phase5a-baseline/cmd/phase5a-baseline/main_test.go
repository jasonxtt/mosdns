package main

import (
	"encoding/json"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/miekg/dns"
)

func TestSampleProcessGroupRecordsRSSAndFDCountsByStageAndRole(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("/proc resource samples are Linux-specific")
	}
	path := filepath.Join(t.TempDir(), "resource-samples.jsonl")
	stop := make(chan struct{})
	close(stop)
	counts := sampleProcessGroup([]resourceTarget{{Role: "load-generator", PID: os.Getpid()}}, "run-1", "normal-reference", path, stop, 100)
	if counts["load-generator"] != 1 {
		t.Fatalf("sample count = %d, want 1", counts["load-generator"])
	}
	f, err := os.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	var sample resourceSample
	if err := json.NewDecoder(f).Decode(&sample); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	if sample.RunID != "run-1" || sample.StageID != "normal-reference" || sample.Role != "load-generator" || sample.PID != os.Getpid() || sample.FDCount <= 0 || sample.RSSKiB <= 0 {
		t.Fatalf("resource sample is missing stage, role, RSS, or FD evidence: %+v", sample)
	}
}

func TestResourceSamplingDoesNotSpawnClockQueries(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("/proc resource samples are Linux-specific")
	}
	dir := t.TempDir()
	marker := filepath.Join(dir, "spawned")
	script := "#!/bin/sh\nprintf called >> '" + marker + "'\nprintf '100\\n'\n"
	if err := os.WriteFile(filepath.Join(dir, "getconf"), []byte(script), 0755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir)
	stop := make(chan struct{})
	close(stop)
	counts := sampleProcessGroup([]resourceTarget{{Role: "sut", PID: os.Getpid()}, {Role: "load-generator", PID: os.Getpid()}}, "clock-test", "normal-reference", filepath.Join(dir, "samples.jsonl"), stop, 100)
	if counts["sut"] != 1 || counts["load-generator"] != 1 {
		t.Fatalf("missing samples: %v", counts)
	}
	if _, err := os.Stat(marker); !os.IsNotExist(err) {
		t.Fatalf("resource sampling spawned getconf during the measured stage: %v", err)
	}
}

func TestResourceClockRejectsInvalidOrMissingHostConstant(t *testing.T) {
	for _, output := range []string{"0", "-1", "invalid", "100"} {
		t.Run(output, func(t *testing.T) {
			dir := t.TempDir()
			if err := os.WriteFile(filepath.Join(dir, "getconf"), []byte("#!/bin/sh\nprintf '%s\\n' '"+output+"'\n"), 0755); err != nil {
				t.Fatal(err)
			}
			t.Setenv("PATH", dir)
			hz, err := resourceClockTicksPerSecond()
			if output == "100" {
				if err != nil || hz != 100 {
					t.Fatalf("clock = %d, %v", hz, err)
				}
			} else if err == nil {
				t.Fatalf("invalid clock %q was accepted", output)
			}
		})
	}
	t.Setenv("PATH", t.TempDir())
	if _, err := resourceClockTicksPerSecond(); err == nil {
		t.Fatal("missing getconf must fail before measurement")
	}
}

func TestM2RunnerRequiresAndRecordsFixedRuntimeProfile(t *testing.T) {
	runner := os.Getenv("PHASE5A_RUNNER_UNDER_TEST")
	if runner == "" {
		var err error
		runner, err = filepath.Abs("../../../../scripts/run-phase5a-baseline.sh")
		if err != nil {
			t.Fatal(err)
		}
	}
	for _, value := range []string{"", "2", "1"} {
		t.Run("gomaxprocs-"+value, func(t *testing.T) {
			cmd := exec.Command("bash", runner)
			for _, item := range os.Environ() {
				key := strings.SplitN(item, "=", 2)[0]
				if key != "GOMAXPROCS" && key != "PHASE5A_MEASUREMENT_PROFILE" && key != "RUN_MODE" && key != "CANDIDATE" && key != "HARNESS_CPU_SET" && key != "MOSDNS_BINARY" && key != "SCENARIO" {
					cmd.Env = append(cmd.Env, item)
				}
			}
			cmd.Env = append(cmd.Env, "PHASE5A_MEASUREMENT_PROFILE=m2", "RUN_MODE=pilot", "CANDIDATE=rust", "GOMAXPROCS="+value)
			out, err := cmd.CombinedOutput()
			if err == nil {
				t.Fatal("test must stop before any SUT launch")
			}
			want := "m2 measurement requires GOMAXPROCS=1"
			if value == "1" {
				want = "MOSDNS_BINARY and SCENARIO are required"
			}
			if !strings.Contains(string(out), want) {
				t.Fatalf("expected %q, got %s", want, out)
			}
		})
	}
	content, err := os.ReadFile(runner)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(content), "gomaxprocs_environment=%s") || !strings.Contains(string(content), "measurement_profile=%s") {
		t.Fatal("standard environment evidence omits the measurement runtime profile")
	}
}

func TestVerifySamplesRequiresEveryExpectedProcessRole(t *testing.T) {
	path := filepath.Join(t.TempDir(), "stages.jsonl")
	stage := stageResult{Stage: "normal-reference", ResourceSampleCounts: map[string]int{"sut": 2, "load-generator": 2, "fixture-1": 2}}
	writeJSONLines(t, path, []stageResult{stage})
	if err := verifySamplesCommand([]string{"--stage-result", path, "--stage", "normal-reference", "--expected-fixtures", "1"}); err != nil {
		t.Fatalf("complete process-role samples should pass: %v", err)
	}
	stage.ResourceSampleCounts["fixture-1"] = 0
	writeJSONLines(t, path, []stageResult{stage})
	if err := verifySamplesCommand([]string{"--stage-result", path, "--stage", "normal-reference", "--expected-fixtures", "1"}); err == nil {
		t.Fatal("missing fixture process samples must fail")
	}
}

func TestVerifySenderStageRejectsShortfallAndUnsentQueries(t *testing.T) {
	stage := stageResult{Stage: "common-load", Counters: stageCounters{Scheduled: 20, Sent: 20}}
	if err := verifySenderStage(stage); err != nil {
		t.Fatalf("fully sent stage should pass sender validation: %v", err)
	}
	stage.Counters.SenderShortfall = 1
	if err := verifySenderStage(stage); err == nil {
		t.Fatal("dropped open-loop schedule slots must invalidate the stage")
	}
	stage.Counters.SenderShortfall = 0
	stage.Counters.Sent = 19
	if err := verifySenderStage(stage); err == nil {
		t.Fatal("scheduled queries that were not sent must invalidate the stage")
	}
}

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

func TestResponseMatchesRejectsInvalidDNSResponseContract(t *testing.T) {
	tests := []struct {
		name   string
		mutate func(*dns.Msg)
	}{
		{name: "missing response bit", mutate: func(msg *dns.Msg) { msg.Response = false }},
		{name: "wrong id", mutate: func(msg *dns.Msg) { msg.Id++ }},
		{name: "wrong opcode", mutate: func(msg *dns.Msg) { msg.Opcode = dns.OpcodeNotify }},
		{name: "wrong question name", mutate: func(msg *dns.Msg) { msg.Question[0].Name = "other.test." }},
		{name: "wrong question type", mutate: func(msg *dns.Msg) { msg.Question[0].Qtype = dns.TypeAAAA }},
		{name: "wrong question class", mutate: func(msg *dns.Msg) { msg.Question[0].Qclass = dns.ClassCHAOS }},
		{name: "unexpected response code", mutate: func(msg *dns.Msg) { msg.Rcode = dns.RcodeNameError }},
		{name: "truncated response", mutate: func(msg *dns.Msg) { msg.Truncated = true }},
		{name: "missing expected answer", mutate: func(msg *dns.Msg) { msg.Answer = nil }},
		{name: "extra answer", mutate: func(msg *dns.Msg) {
			msg.Answer = append(msg.Answer, &dns.A{
				Hdr: dns.RR_Header{Name: "ok.test.", Rrtype: dns.TypeA, Class: dns.ClassINET},
				A:   net.ParseIP("198.51.100.11").To4(),
			})
		}},
		{name: "wrong answer owner", mutate: func(msg *dns.Msg) { msg.Answer[0].Header().Name = "other.test." }},
		{name: "wrong answer class", mutate: func(msg *dns.Msg) { msg.Answer[0].Header().Class = dns.ClassCHAOS }},
		{name: "wrong answer type", mutate: func(msg *dns.Msg) {
			msg.Answer[0] = &dns.AAAA{
				Hdr:  dns.RR_Header{Name: "ok.test.", Rrtype: dns.TypeAAAA, Class: dns.ClassINET},
				AAAA: net.ParseIP("2001:db8::1"),
			}
		}},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			query := new(dns.Msg)
			query.SetQuestion("ok.test.", dns.TypeA)
			query.Id = 7
			response := new(dns.Msg)
			response.SetReply(query)
			response.Answer = []dns.RR{&dns.A{
				Hdr: dns.RR_Header{Name: "ok.test.", Rrtype: dns.TypeA, Class: dns.ClassINET},
				A:   net.ParseIP("198.51.100.10").To4(),
			}}
			tt.mutate(response)

			c := workloadCase{ExpectedRCode: dns.RcodeSuccess, ExpectedAnswerClass: "A", ExpectedAnswer: "198.51.100.10"}
			if responseMatches(response, query, c) {
				t.Fatal("invalid DNS response must not pass the correctness oracle")
			}
		})
	}
}

func TestResponseMatchesRejectsAnswerOnExpectedNXDOMAIN(t *testing.T) {
	query := new(dns.Msg)
	query.SetQuestion("missing.test.", dns.TypeA)
	query.Id = 8
	response := new(dns.Msg)
	response.SetReply(query)
	response.Rcode = dns.RcodeNameError
	response.Answer = []dns.RR{&dns.A{
		Hdr: dns.RR_Header{Name: "missing.test.", Rrtype: dns.TypeA, Class: dns.ClassINET},
		A:   net.ParseIP("198.51.100.10").To4(),
	}}

	c := workloadCase{ExpectedRCode: dns.RcodeNameError, ExpectedAnswerClass: "NXDOMAIN"}
	if responseMatches(response, query, c) {
		t.Fatal("NXDOMAIN with an answer must fail the correctness oracle")
	}
}

func TestVerifyWarmTTLUsesPerKeyPrefillTimeAndSafetyMargin(t *testing.T) {
	base := time.Date(2026, time.September, 23, 12, 0, 0, 0, time.UTC)
	cases := []workloadCase{
		{CaseID: "a", QName: "a.test.", QType: "A"},
		{CaseID: "b", QName: "b.test.", QType: "A"},
	}
	prefill := []requestRecord{
		{RunID: "run-1", StageID: "warm-prefill", CaseID: "a", QName: "a.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base, FinishedAt: base.Add(10 * time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "warm-prefill", CaseID: "b", QName: "b.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(20 * time.Second), FinishedAt: base.Add(20*time.Second + 10*time.Millisecond), Outcome: "correct_on_time"},
	}
	warm := []requestRecord{
		{RunID: "run-1", StageID: "w2-warm", CaseID: "a", QName: "a.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(28 * time.Second), FinishedAt: base.Add(28*time.Second + 10*time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "w2-warm", CaseID: "b", QName: "b.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(48 * time.Second), FinishedAt: base.Add(48*time.Second + 10*time.Millisecond), Outcome: "correct_on_time"},
	}
	if err := verifyWarmTTL(cases, prefill, warm, 30*time.Second, time.Second); err != nil {
		t.Fatalf("per-key prefill times within the TTL safety window should pass: %v", err)
	}

	tests := []struct {
		name   string
		mutate func([]requestRecord, []requestRecord) ([]requestRecord, []requestRecord)
	}{
		{
			name: "reject exact safety-margin boundary",
			mutate: func(prefill, warm []requestRecord) ([]requestRecord, []requestRecord) {
				warm[0].FinishedAt = prefill[0].SentAt.Add(29 * time.Second)
				return prefill, warm
			},
		},
		{
			name: "reject expired response",
			mutate: func(prefill, warm []requestRecord) ([]requestRecord, []requestRecord) {
				warm[0].FinishedAt = prefill[0].SentAt.Add(30 * time.Second)
				return prefill, warm
			},
		},
		{
			name: "reject missing prefill key",
			mutate: func(prefill, warm []requestRecord) ([]requestRecord, []requestRecord) {
				return prefill[:1], warm
			},
		},
		{
			name: "reject missing measured key",
			mutate: func(prefill, warm []requestRecord) ([]requestRecord, []requestRecord) {
				return prefill, warm[:1]
			},
		},
		{
			name: "reject incorrect warm response",
			mutate: func(prefill, warm []requestRecord) ([]requestRecord, []requestRecord) {
				warm[0].Outcome = "wrong_response"
				return prefill, warm
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			prefillCopy := append([]requestRecord(nil), prefill...)
			warmCopy := append([]requestRecord(nil), warm...)
			prefillCopy, warmCopy = tt.mutate(prefillCopy, warmCopy)
			if err := verifyWarmTTL(cases, prefillCopy, warmCopy, 30*time.Second, time.Second); err == nil {
				t.Fatal("invalid warm-cache timing must fail verification")
			}
		})
	}
}

func TestVerifyWarmTTLCommandReadsSessionLedger(t *testing.T) {
	dir := t.TempDir()
	workloadPath := filepath.Join(dir, "cache.jsonl")
	ledgerPath := filepath.Join(dir, "requests.jsonl")
	workload := "" +
		`{"case_id":"a","scenario":"w2","transport":"udp","qname":"a.test.","qtype":"A","expected_rcode":0,"expected_answer_class":"A","expected_answer":"198.51.100.20","expected_route_class":"cache","request_deadline_ms":500,"weight":1}` + "\n" +
		`{"case_id":"b","scenario":"w2","transport":"udp","qname":"b.test.","qtype":"A","expected_rcode":0,"expected_answer_class":"A","expected_answer":"198.51.100.21","expected_route_class":"cache","request_deadline_ms":500,"weight":1}` + "\n"
	if err := writeFile(workloadPath, workload); err != nil {
		t.Fatal(err)
	}
	base := time.Date(2026, time.September, 23, 12, 0, 0, 0, time.UTC)
	records := []requestRecord{
		{RunID: "run-1", StageID: "warm-prefill", QName: "a.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base, FinishedAt: base.Add(time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "warm-prefill", QName: "b.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(time.Second), FinishedAt: base.Add(time.Second + time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "warm-normal", QName: "a.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(2 * time.Second), FinishedAt: base.Add(2*time.Second + time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "warm-normal", QName: "b.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(3 * time.Second), FinishedAt: base.Add(3*time.Second + time.Millisecond), Outcome: "correct_on_time"},
	}
	f, err := os.Create(ledgerPath)
	if err != nil {
		t.Fatal(err)
	}
	enc := json.NewEncoder(f)
	for _, record := range records {
		if err := enc.Encode(record); err != nil {
			_ = f.Close()
			t.Fatal(err)
		}
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	args := []string{"--workload", workloadPath, "--request-ledger", ledgerPath, "--warm-stage", "warm-normal", "--ttl", "30s", "--safety-margin", "1s"}
	if err := verifyWarmTTLCommand(args); err != nil {
		t.Fatalf("valid session ledger should pass TTL verification: %v", err)
	}
}

func TestVerifyRoutingEventsProvesExactOrderedPathPerRequest(t *testing.T) {
	cases, requests, events, stage := validRouteEvidence()
	if err := verifyRoutingEvents(cases, requests, events, stage); err != nil {
		t.Fatalf("complete ordered route evidence should pass: %v", err)
	}
}

func TestVerifyRoutingEventsCorrelatesRewrittenUpstreamIDsByRequestTimeWindow(t *testing.T) {
	cases, requests, events, stage := validRouteEvidence()
	if err := verifyRoutingEvents(cases, requests, events, stage); err != nil {
		t.Fatalf("upstream-assigned DNS IDs must not break ordered question correlation: %v", err)
	}

	base := requests[0].FinishedAt
	repeated := requests[0]
	repeated.RequestSeq = 4
	repeated.DNSID = 404
	repeated.SentAt = base.Add(time.Millisecond)
	repeated.FinishedAt = repeated.SentAt.Add(time.Millisecond)
	requests = append(requests, repeated)
	extraEvent := events[0]
	extraEvent.FixtureSeq = 106
	extraEvent.DNSID = 7
	extraEvent.OccurredAt = repeated.SentAt.Add(time.Microsecond)
	events = append(events, extraEvent)
	stage.RequestSeqEnd = 4
	stage.FixtureSeqEnd = 106
	stage.Counters.Sent = 4
	stage.Counters.CorrectOnTime = 4
	if err := verifyRoutingEvents(cases, requests, events, stage); err != nil {
		t.Fatalf("sequential repeated questions should correlate by ordered occurrence: %v", err)
	}
}

func TestVerifyRoutingEventsRejectsRedistributedLegsAcrossRepeatedQuestions(t *testing.T) {
	base := time.Date(2026, time.September, 24, 12, 0, 0, 0, time.UTC)
	cases := []workloadCase{{
		CaseID: "domain-hit", Scenario: "w3", Transport: "udp", QName: "domain-hit.test.", QType: "A",
		ExpectedRouteClass: "DOMAIN_HIT", Weight: 1,
	}}
	requests := []requestRecord{
		{RunID: "run-1", StageID: "w3-normal", RequestSeq: 1, DNSID: 101, CaseID: "domain-hit", QName: cases[0].QName, QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base, FinishedAt: base.Add(5 * time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "w3-normal", RequestSeq: 2, DNSID: 102, CaseID: "domain-hit", QName: cases[0].QName, QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(6 * time.Millisecond), FinishedAt: base.Add(10 * time.Millisecond), Outcome: "correct_on_time"},
	}
	events := []fixtureEvent{
		{FixtureSeq: 1, OccurredAt: base.Add(time.Millisecond), DNSID: 0, QName: cases[0].QName, QType: dns.TypeA, QClass: dns.ClassINET, Upstream: "route-a"},
		{FixtureSeq: 2, OccurredAt: base.Add(2 * time.Millisecond), DNSID: 1, QName: cases[0].QName, QType: dns.TypeA, QClass: dns.ClassINET, Upstream: "route-a"},
	}
	stage := stageResult{
		RunID: "run-1", Stage: "w3-normal", FixtureSeqStart: 0, FixtureSeqEnd: 2,
		RequestSeqStart: 1, RequestSeqEnd: 2,
		Counters: stageCounters{Sent: 2, CorrectOnTime: 2},
	}
	if err := verifyRoutingEvents(cases, requests, events, stage); err == nil {
		t.Fatal("both route-a events falling inside the first request must not be split across repeated same-question requests")
	}
}

func TestCollectPairedStageObservationsSupportsW2IndependentPrefillLifecycle(t *testing.T) {
	schedule, err := buildPairSchedule(3)
	if err != nil {
		t.Fatal(err)
	}
	root := t.TempDir()
	manifest := officialManifest{
		OfficialFrozen:         true,
		RecoveryAssessmentMode: recoveryAssessmentMode,
		PairSchedule:           schedule,
		Inputs:                 []manifestInput{{Path: "input", SHA256: strings.Repeat("1", 64)}},
		Runner:                 manifestArtifact{Path: "runner.sh", SHA256: strings.Repeat("2", 64)},
		Helper:                 manifestHelper{SourcePath: "helper.go", SourceSHA256: strings.Repeat("3", 64), BinaryPath: "helper", BinarySHA256: strings.Repeat("4", 64), Version: helperVersion},
		Candidates: map[string]manifestCandidate{
			"go":   {SourceCommit: "go-source", BinaryPath: "mosdns-go", BinarySHA256: strings.Repeat("5", 64)},
			"rust": {SourceCommit: "rust-source", BinaryPath: "mosdns-rust", BinarySHA256: strings.Repeat("6", 64)},
		},
		Scenarios: map[string]manifestScenario{"w2": {
			StageDurationMS: 3000, NormalReferenceQPS: 200, CommonLoadQPS: 400,
			NearSaturationQPS: 800, OverloadQPS: 1000, RequestDeadlineMS: 500,
			LateDrainMS: 100, HarnessCPUSet: "1", SUTCPUSet: "0",
			RecoveryMinimumSamples: 1, RecoveryP95CeilingUS: 1000, RecoveryP99CeilingUS: 2000,
			W2WarmLifecycle: "independent-prefilled", W2CacheTTLMS: 30000, W2TTLSafetyMarginMS: 500,
		}},
	}
	manifestSHA := strings.Repeat("a", 64)
	resultsRoot := filepath.Join(root, "results")
	writeOfficialRunFixtures(t, resultsRoot, "w2", schedule, manifestSHA, manifest, manifest.Scenarios["w2"])
	failedGoRun := filepath.Join(resultsRoot, "w2", "repetition-2", "go")
	if err := os.WriteFile(filepath.Join(failedGoRun, "invalid-stages.tsv"), []byte("common-load-prefill\tper-key prefill failed\n"), 0644); err != nil {
		t.Fatal(err)
	}

	observations, err := collectPairedStageObservations(resultsRoot, manifest, manifestSHA)
	if err != nil {
		t.Fatalf("independent-prefilled W2 rows and stage-scoped prefill failure should be collected: %v", err)
	}
	var recovered []pairedStageObservation
	var commonGo *pairedStageObservation
	for i := range observations {
		observation := observations[i]
		if observation.Stage == "recovery" && observation.Repetition == 1 && observation.Candidate == "go" {
			recovered = append(recovered, observation)
		}
		if observation.Stage == "common-load" && observation.Repetition == 2 && observation.Candidate == "go" {
			commonGo = &observations[i]
		}
	}
	if len(recovered) != 1 || !strings.Contains(recovered[0].RecoveryAssessment, "independent-prefilled") {
		t.Fatalf("independent W2 recovery row must be retained with an indeterminate assessment: %+v", recovered)
	}
	if commonGo == nil || !strings.Contains(commonGo.InvalidReason, "per-key prefill failed") {
		t.Fatalf("stage-specific W2 prefill failure was not propagated: %+v", commonGo)
	}
	aggregates, err := aggregatePairedStages(observations, schedule, manifestSHA)
	if err != nil {
		t.Fatalf("independent W2 rows should aggregate: %v", err)
	}
	for _, aggregate := range aggregates {
		if aggregate.Stage == "recovery" && !strings.Contains(aggregate.RecoveryAssessment, "independent-prefilled") {
			t.Fatalf("aggregate recovery row lost its indeterminate lifecycle assessment: %+v", aggregate)
		}
	}
}

func TestMissingRecoveryAssessmentEvidenceIsRetainedAsInvalidPair(t *testing.T) {
	schedule, err := buildPairSchedule(3)
	if err != nil {
		t.Fatal(err)
	}
	root := t.TempDir()
	manifestPath := filepath.Join(root, "manifest.json")
	manifest := officialManifest{
		OfficialFrozen: true, RecoveryAssessmentMode: recoveryAssessmentMode,
		PairSchedule: schedule,
		Inputs:       []manifestInput{{Path: "input", SHA256: strings.Repeat("1", 64)}},
		Runner:       manifestArtifact{Path: "runner", SHA256: strings.Repeat("2", 64)},
		Helper:       manifestHelper{SourcePath: "helper.go", SourceSHA256: strings.Repeat("3", 64), BinaryPath: "helper", BinarySHA256: strings.Repeat("4", 64), Version: helperVersion},
		Candidates: map[string]manifestCandidate{
			"go":   {SourceCommit: "go-source", BinaryPath: "mosdns-go", BinarySHA256: strings.Repeat("5", 64)},
			"rust": {SourceCommit: "rust-source", BinaryPath: "mosdns-rust", BinarySHA256: strings.Repeat("6", 64)},
		},
		Scenarios: map[string]manifestScenario{"w1-udp": {
			StageDurationMS: 3000, NormalReferenceQPS: 200, CommonLoadQPS: 400,
			NearSaturationQPS: 800, OverloadQPS: 1000, RequestDeadlineMS: 500,
			LateDrainMS: 100, HarnessCPUSet: "1", SUTCPUSet: "0",
			RecoveryMinimumSamples: 1, RecoveryP95CeilingUS: 1000, RecoveryP99CeilingUS: 2000,
		}},
	}
	if err := writeJSONFile(manifestPath, manifest); err != nil {
		t.Fatal(err)
	}
	manifestSHA, err := sha256File(manifestPath)
	if err != nil {
		t.Fatal(err)
	}
	resultsRoot := filepath.Join(root, "results")
	writeOfficialRunFixtures(t, resultsRoot, "w1-udp", schedule, manifestSHA, manifest, manifest.Scenarios["w1-udp"])
	missingRun := filepath.Join(resultsRoot, "w1-udp", "repetition-1", "go")
	if err := os.Remove(filepath.Join(missingRun, "service-recovery-assessment.txt")); err != nil {
		t.Fatal(err)
	}
	observations, err := collectPairedStageObservations(resultsRoot, manifest, manifestSHA)
	if err != nil {
		t.Fatalf("missing recovery evidence should be retained as an invalid stage, not abort collection: %v", err)
	}
	aggregates, err := aggregatePairedStages(observations, schedule, manifestSHA)
	if err != nil {
		t.Fatalf("invalid recovery evidence should remain reportable: %v", err)
	}
	for _, aggregate := range aggregates {
		if aggregate.Stage == "recovery" {
			if aggregate.ValidPairs != 2 || len(aggregate.InvalidPairs) != 1 || !strings.Contains(aggregate.InvalidPairs[0].Reason, "service recovery assessment") {
				t.Fatalf("missing assessment should invalidate only that paired recovery point: %+v", aggregate)
			}
			return
		}
	}
	t.Fatal("missing recovery aggregate")
}

func TestVerifyRoutingCounterTotalsMatchCompleteEventJournal(t *testing.T) {
	cases, _, events, _ := validRouteEvidence()
	dir := t.TempDir()
	routeAPath := filepath.Join(dir, "route-a.json")
	routeBPath := filepath.Join(dir, "route-b.json")
	routeCPath := filepath.Join(dir, "route-c.json")
	writeCounterTestFile(t, routeAPath, "route-a", map[string]int64{"domain-hit.test.|A": 1, "ip-hit.test.|A": 1})
	writeCounterTestFile(t, routeBPath, "route-b", map[string]int64{"ip-hit.test.|A": 1, "ip-miss.test.|A": 1})
	writeCounterTestFile(t, routeCPath, "route-c", map[string]int64{"ip-miss.test.|A": 1})
	if err := verifyRoutingCountersWithEvents(cases, events, routeAPath, routeBPath, routeCPath); err != nil {
		t.Fatalf("counter totals matching the event journal should pass: %v", err)
	}
	writeCounterTestFile(t, routeCPath, "route-c", map[string]int64{"ip-miss.test.|A": 1, "unexpected.test.|A": 1})
	if err := verifyRoutingCountersWithEvents(cases, events, routeAPath, routeBPath, routeCPath); err == nil {
		t.Fatal("an extra aggregate fixture event must fail exact counter cross-check")
	}
}

func TestVerifyCompleteFixtureJournalRejectsTailLossAndSequenceGaps(t *testing.T) {
	events := []fixtureEvent{
		{FixtureSeq: 1}, {FixtureSeq: 2}, {FixtureSeq: 3},
	}
	if err := verifyCompleteFixtureJournal(events, 3); err != nil {
		t.Fatalf("complete journal should pass: %v", err)
	}
	if err := verifyCompleteFixtureJournal(events, 4); err == nil {
		t.Fatal("events appended after the last stage barrier must be detected")
	}
	events[2].FixtureSeq = 4
	if err := verifyCompleteFixtureJournal(events, 4); err == nil {
		t.Fatal("fixture sequence gap must be detected")
	}
}

func TestVerifyRoutingEventsCommandReadsSessionEvidence(t *testing.T) {
	cases, requests, events, stage := validRouteEvidence()
	dir := t.TempDir()
	workloadPath := filepath.Join(dir, "routing.jsonl")
	requestPath := filepath.Join(dir, "requests.jsonl")
	eventPath := filepath.Join(dir, "events.jsonl")
	stagePath := filepath.Join(dir, "stages.jsonl")
	writeJSONLines(t, workloadPath, cases)
	writeJSONLines(t, requestPath, requests)
	writeJSONLines(t, eventPath, events)
	writeJSONLines(t, stagePath, []stageResult{stage})
	args := []string{"--workload", workloadPath, "--request-ledger", requestPath, "--event-journal", eventPath, "--stage-result", stagePath, "--stage", stage.Stage}
	if err := verifyRoutingEventsCommand(args); err != nil {
		t.Fatalf("complete session evidence should pass command verification: %v", err)
	}
}

func TestVerifyRoutingEventsRejectsInvalidEvidence(t *testing.T) {
	tests := []struct {
		name   string
		mutate func([]workloadCase, []requestRecord, []fixtureEvent, *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent)
	}{
		{
			name: "missing leg",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				return c, r, append(e[:2], e[3:]...)
			},
		},
		{
			name: "duplicate leg",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, s *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				extra := e[0]
				extra.FixtureSeq = 106
				s.FixtureSeqEnd = 106
				return c, r, append(e, extra)
			},
		},
		{
			name: "reversed route legs",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				e[1].Upstream, e[2].Upstream = e[2].Upstream, e[1].Upstream
				return c, r, e
			},
		},
		{
			name: "forbidden route leg",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				e[0].Upstream = "route-c"
				return c, r, e
			},
		},
		{
			name: "unmatched event",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				e[0].QName = "unexpected.test."
				return c, r, e
			},
		},
		{
			name: "event outside stage barrier",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				e[0].FixtureSeq = 100
				return c, r, e
			},
		},
		{
			name: "event outside client request interval",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				e[0].OccurredAt = r[0].FinishedAt.Add(time.Nanosecond)
				return c, r, e
			},
		},
		{
			name: "missing fixture occurrence timestamp",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				e[0].OccurredAt = time.Time{}
				return c, r, e
			},
		},
		{
			name: "fixture sequence gap",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				e = append(e[:2], e[3:]...)
				return c, r, e
			},
		},
		{
			name: "duplicate fixture sequence",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				e[1].FixtureSeq = e[0].FixtureSeq
				return c, r, e
			},
		},
		{
			name: "missing request record",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				return c, r[:2], e
			},
		},
		{
			name: "request sequence range mismatch",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, s *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				s.RequestSeqEnd++
				return c, r, e
			},
		},
		{
			name: "overlapping same question with a distinct id",
			mutate: func(c []workloadCase, r []requestRecord, e []fixtureEvent, _ *stageResult) ([]workloadCase, []requestRecord, []fixtureEvent) {
				reused := r[0]
				reused.RequestSeq = 4
				reused.DNSID = 999
				reused.SentAt = reused.SentAt.Add(5 * time.Millisecond)
				reused.FinishedAt = reused.FinishedAt.Add(5 * time.Millisecond)
				r = append(r, reused)
				return c, r, e
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			cases, requests, events, stage := validRouteEvidence()
			cases, requests, events = tt.mutate(cases, requests, events, &stage)
			if err := verifyRoutingEvents(cases, requests, events, stage); err == nil {
				t.Fatal("invalid routing evidence must fail verification")
			}
		})
	}
}

func TestFixtureEventJournalAssignsSharedMonotonicSequence(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing-events.jsonl")
	const writers = 4
	const eventsPerWriter = 20
	var wg sync.WaitGroup
	seqs := make(chan uint64, writers*eventsPerWriter)
	for writerID := 0; writerID < writers; writerID++ {
		journal := newFixtureEventJournal(path)
		wg.Add(1)
		go func(id int, journal *fixtureEventJournal) {
			defer wg.Done()
			for i := 0; i < eventsPerWriter; i++ {
				event := fixtureEvent{DNSID: uint16(id*eventsPerWriter + i), QName: "event.test.", QType: dns.TypeA, QClass: dns.ClassINET, Upstream: "route-a"}
				written, err := journal.append(event)
				if err != nil {
					t.Errorf("append event: %v", err)
					return
				}
				seqs <- written.FixtureSeq
			}
		}(writerID, journal)
	}
	wg.Wait()
	close(seqs)
	got := make([]uint64, 0, writers*eventsPerWriter)
	for seq := range seqs {
		got = append(got, seq)
	}
	sort.Slice(got, func(i, j int) bool { return got[i] < got[j] })
	for i, seq := range got {
		if want := uint64(i + 1); seq != want {
			t.Fatalf("fixture sequence[%d]=%d, want %d", i, seq, want)
		}
	}
	events, err := readFixtureEvents(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != writers*eventsPerWriter {
		t.Fatalf("journal contains %d events, want %d", len(events), writers*eventsPerWriter)
	}
}

func TestRequestIDAllocatorReservesAndQuarantinesIDs(t *testing.T) {
	allocator := newRequestIDAllocator(nil)
	allocator.next = 7
	first, err := allocator.allocate("same.test.", "A", dns.ClassINET)
	if err != nil {
		t.Fatal(err)
	}
	second, err := allocator.allocate("same.test.", "A", dns.ClassINET)
	if err != nil {
		t.Fatal(err)
	}
	if first == second {
		t.Fatalf("in-flight DNS ID/question tuple was reused: %d", first)
	}
	allocator.release(first, "same.test.", "A", dns.ClassINET, "correct_on_time")
	allocator.next = first
	reused, err := allocator.allocate("same.test.", "A", dns.ClassINET)
	if err != nil {
		t.Fatal(err)
	}
	if reused != first {
		t.Fatalf("completed successful request ID=%d should be reusable, got %d", first, reused)
	}

	allocator.release(reused, "same.test.", "A", dns.ClassINET, "timeout")
	allocator.next = reused
	afterTimeout, err := allocator.allocate("same.test.", "A", dns.ClassINET)
	if err != nil {
		t.Fatal(err)
	}
	if afterTimeout == reused {
		t.Fatalf("timed-out DNS ID=%d must remain quarantined for the session", reused)
	}
}

func TestVerifyRequestIDUseRejectsReuseAfterFailedRequest(t *testing.T) {
	base := time.Date(2026, time.September, 23, 12, 0, 0, 0, time.UTC)
	records := []requestRecord{
		{RunID: "run-1", StageID: "overload", RequestSeq: 1, DNSID: 44, QName: "a.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base, FinishedAt: base.Add(10 * time.Millisecond), Outcome: "timeout"},
		{RunID: "run-1", StageID: "recovery", RequestSeq: 2, DNSID: 44, QName: "b.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(11 * time.Millisecond), FinishedAt: base.Add(12 * time.Millisecond), Outcome: "correct_on_time"},
	}
	if err := verifyRequestIDUse(records); err == nil {
		t.Fatal("DNS ID from a failed request must remain quarantined across stages")
	}
}

func TestVerifyRequestIDUseAllowsSafeReuseAndDistinctConcurrentQuestions(t *testing.T) {
	base := time.Date(2026, time.September, 23, 12, 0, 0, 0, time.UTC)
	records := []requestRecord{
		{RunID: "run-1", StageID: "normal-reference", RequestSeq: 1, DNSID: 44, QName: "a.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base, FinishedAt: base.Add(10 * time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "normal-reference", RequestSeq: 2, DNSID: 44, QName: "b.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(time.Millisecond), FinishedAt: base.Add(11 * time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "recovery", RequestSeq: 3, DNSID: 44, QName: "a.test.", QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(12 * time.Millisecond), FinishedAt: base.Add(13 * time.Millisecond), Outcome: "correct_on_time"},
	}
	if err := verifyRequestIDUse(records); err != nil {
		t.Fatalf("distinct concurrent questions and later successful reuse are unambiguous: %v", err)
	}
}

func TestVerifyContinuousStagesRequiresOneOrderedSessionAndFrozenRecoveryBand(t *testing.T) {
	stages := validContinuousStages()
	criteria := recoveryCriteria{MinimumSamples: 5, P95CeilingUS: 1200, P99CeilingUS: 1500}
	if err := verifyContinuousStages(stages, criteria); err != nil {
		t.Fatalf("valid continuous recovery evidence should pass: %v", err)
	}
	tests := []struct {
		name   string
		mutate func([]stageResult) []stageResult
	}{
		{name: "reordered stages", mutate: func(in []stageResult) []stageResult { in[1], in[2] = in[2], in[1]; return in }},
		{name: "different SUT PID", mutate: func(in []stageResult) []stageResult { in[4].SUTPID++; return in }},
		{name: "different process start identity", mutate: func(in []stageResult) []stageResult { in[4].SUTStartIdentity = "restarted"; return in }},
		{name: "different fixture session", mutate: func(in []stageResult) []stageResult { in[4].FixtureSessionID = "new-fixtures"; return in }},
		{name: "request sequence reset", mutate: func(in []stageResult) []stageResult { in[2].RequestSeqStart = in[1].RequestSeqStart; return in }},
		{name: "recovery rate differs from reference", mutate: func(in []stageResult) []stageResult { in[4].TargetQPS++; return in }},
		{name: "insufficient recovery samples", mutate: func(in []stageResult) []stageResult {
			in[4].Counters.CorrectOnTime = 4
			in[4].LatencySamplesUS = in[4].LatencySamplesUS[:4]
			return in
		}},
		{name: "recovery latency above frozen ceiling", mutate: func(in []stageResult) []stageResult { in[4].P99US = 1501; return in }},
		{name: "recovery has a failed query", mutate: func(in []stageResult) []stageResult { in[4].Counters.Timeout = 1; return in }},
		{name: "overload stage dropped schedule slots", mutate: func(in []stageResult) []stageResult { in[3].Counters.SenderShortfall = 1; return in }},
		{name: "fixture barrier has a gap", mutate: func(in []stageResult) []stageResult { in[3].FixtureSeqStart++; return in }},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			in := append([]stageResult(nil), stages...)
			in = tt.mutate(in)
			if err := verifyContinuousStages(in, criteria); err == nil {
				t.Fatal("invalid continuous recovery evidence must fail")
			}
		})
	}
}

func TestAllHealthyContinuousSequenceDoesNotClaimServiceRecovery(t *testing.T) {
	assessment, err := assessServiceRecovery(validContinuousStages(), recoveryCriteria{
		MinimumSamples: 5, P95CeilingUS: 1200, P99CeilingUS: 1500,
	})
	if err != nil {
		t.Fatalf("healthy continuous stages should pass the same-session health check: %v", err)
	}
	if assessment.Status != "indeterminate" || assessment.Mode != recoveryAssessmentMode || !strings.Contains(assessment.Reason, "no objective overload-evidence criterion") {
		t.Fatalf("healthy five-stage sequence must not be called service recovery: %+v", assessment)
	}
}

func TestFailedContinuousHealthCheckStillReportsIndeterminateServiceRecovery(t *testing.T) {
	stages := validContinuousStages()
	stages[4].LatencySamplesUS[9] = 1201
	stages[4].P95US = 1201
	stages[4].P99US = 1201
	assessment, err := assessServiceRecovery(stages, recoveryCriteria{
		MinimumSamples: 5, P95CeilingUS: 1200, P99CeilingUS: 1500,
	})
	if err == nil {
		t.Fatal("a terminal health check above the frozen latency ceiling must fail its health gate")
	}
	if assessment.Status != "indeterminate" || assessment.Mode != recoveryAssessmentMode || !strings.Contains(assessment.Reason, "health check failed") {
		t.Fatalf("failed terminal health check must still preserve the categorical service-recovery assessment: %+v", assessment)
	}

	stagePath := filepath.Join(t.TempDir(), "stages.jsonl")
	writeJSONLines(t, stagePath, stages)
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	previousStdout := os.Stdout
	os.Stdout = writer
	commandErr := verifyContinuousCommand([]string{
		"--stage-result", stagePath, "--run-id", "run-1", "--minimum-samples", "5",
		"--p95-ceiling-us", "1200", "--p99-ceiling-us", "1500",
	})
	_ = writer.Close()
	os.Stdout = previousStdout
	output, readErr := io.ReadAll(reader)
	_ = reader.Close()
	if readErr != nil {
		t.Fatal(readErr)
	}
	if commandErr == nil || !strings.Contains(string(output), "status=indeterminate") || !strings.Contains(string(output), "terminal health check failed") {
		t.Fatalf("failed health-check command must emit its indeterminate assessment before returning an error: err=%v output=%s", commandErr, output)
	}
}

func TestParseCPUListAndRequireDisjointPlacement(t *testing.T) {
	cpus, err := parseCPUList("0-1,4,6-7")
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(cpus, []int{0, 1, 4, 6, 7}) {
		t.Fatalf("parsed CPUs=%v", cpus)
	}
	for _, invalid := range []string{"", "1-0", "0,,1", "0-1,1", "-1", "a"} {
		if _, err := parseCPUList(invalid); err == nil {
			t.Errorf("parseCPUList(%q) unexpectedly passed", invalid)
		}
	}
	if err := verifyDisjointCPULists("0", "1-2"); err != nil {
		t.Fatalf("disjoint harness and SUT masks should pass: %v", err)
	}
	if err := verifyDisjointCPULists("0-1", "1-2"); err == nil {
		t.Fatal("overlapping harness and SUT masks must fail")
	}
	if err := verifySingleCoreDisjointSets("0", "1"); err != nil {
		t.Fatalf("single CPU masks should be accepted: %v", err)
	}
	if err := verifySingleCoreDisjointSets("0-1", "2"); err == nil {
		t.Fatal("multi-core placement must not be labeled a single-core comparison")
	}
}

func TestOfficialManifestVerifiesEveryFrozenInputAndCandidate(t *testing.T) {
	root := t.TempDir()
	manifest, opts := validManifestFixture(t, root)
	manifestPath := filepath.Join(root, "manifest.json")
	if err := writeJSONFile(manifestPath, manifest); err != nil {
		t.Fatal(err)
	}
	manifestHash, err := sha256File(manifestPath)
	if err != nil {
		t.Fatal(err)
	}
	opts.ManifestPath = manifestPath
	opts.ExpectedSHA256 = manifestHash
	if err := verifyOfficialManifest(opts); err != nil {
		t.Fatalf("valid frozen manifest should pass: %v", err)
	}

	inputPath := filepath.Join(root, fixedCorpusInputPaths[0])
	if err := os.WriteFile(inputPath, []byte("changed after freeze"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := verifyOfficialManifest(opts); err == nil {
		t.Fatal("a changed corpus input must invalidate official execution")
	}
}

func TestOfficialManifestFreezesIndeterminateRecoveryAssessmentMode(t *testing.T) {
	root := t.TempDir()
	manifest, opts := validManifestFixture(t, root)
	manifest.RecoveryAssessmentMode = "claim-recovered-without-overload-proof"
	manifestPath := filepath.Join(root, "manifest.json")
	if err := writeJSONFile(manifestPath, manifest); err != nil {
		t.Fatal(err)
	}
	manifestSHA, err := sha256File(manifestPath)
	if err != nil {
		t.Fatal(err)
	}
	opts.ManifestPath = manifestPath
	opts.ExpectedSHA256 = manifestSHA
	if err := verifyOfficialManifest(opts); err == nil {
		t.Fatal("official manifest must reject a recovery claim without a frozen overload criterion")
	}
}

func TestBuildPairScheduleAlternatesCandidateOrder(t *testing.T) {
	schedule, err := buildPairSchedule(4)
	if err != nil {
		t.Fatal(err)
	}
	want := []manifestPair{
		{Repetition: 1, Order: []string{"go", "rust"}},
		{Repetition: 2, Order: []string{"rust", "go"}},
		{Repetition: 3, Order: []string{"go", "rust"}},
		{Repetition: 4, Order: []string{"rust", "go"}},
	}
	if !reflect.DeepEqual(schedule, want) {
		t.Fatalf("schedule=%+v, want %+v", schedule, want)
	}
	if err := verifyManifestPairSchedule(schedule, 2, 2, "go"); err != nil {
		t.Fatalf("scheduled candidate at position 2 should pass: %v", err)
	}
	if err := verifyManifestPairSchedule(schedule, 2, 1, "go"); err == nil {
		t.Fatal("candidate executed out of frozen order must fail")
	}
}

func TestAggregatePairedStagesKeepsCompletePairsAndExcludesInvalidPairSymmetrically(t *testing.T) {
	schedule, err := buildPairSchedule(3)
	if err != nil {
		t.Fatal(err)
	}
	const manifestSHA = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	var observations []pairedStageObservation
	for repetition := 1; repetition <= 3; repetition++ {
		for position, candidate := range schedule[repetition-1].Order {
			p95 := int64(200 + 10*(repetition-1))
			if candidate == "rust" {
				p95 -= 20
			}
			observation := validPairedStageObservation(manifestSHA, repetition, position+1, candidate, p95)
			if repetition == 2 && candidate == "rust" {
				observation.InvalidReason = "sender shortfall"
				observation.Counters.SenderShortfall = 1
				observation.Counters.Sent--
			}
			observations = append(observations, observation)
		}
	}

	got, err := aggregatePairedStages(observations, schedule, manifestSHA)
	if err != nil {
		t.Fatalf("complete measurements should aggregate: %v", err)
	}
	if len(got) != 1 {
		t.Fatalf("aggregate groups=%d, want 1", len(got))
	}
	summary := got[0]
	if summary.CompletePairs != 3 || summary.ValidPairs != 2 {
		t.Fatalf("pair counts complete=%d valid=%d, want 3 and 2", summary.CompletePairs, summary.ValidPairs)
	}
	if summary.Go.P95US.N != 2 || summary.Go.P95US.Median != 210 {
		t.Fatalf("Go p95 summary=%+v, want two valid values with median 210", summary.Go.P95US)
	}
	if summary.Rust.P95US.N != 2 || summary.Rust.P95US.Median != 190 {
		t.Fatalf("Rust p95 summary=%+v, want two valid values with median 190", summary.Rust.P95US)
	}
	if summary.RustMinusGoP95US.N != 2 || summary.RustMinusGoP95US.Median != -20 {
		t.Fatalf("paired p95 delta=%+v, want two pairs with median -20", summary.RustMinusGoP95US)
	}
	if len(summary.InvalidPairs) != 1 || summary.InvalidPairs[0].Candidate != "rust" || summary.InvalidPairs[0].Reason != "sender shortfall" {
		t.Fatalf("invalid pair evidence=%+v, want the Rust shortfall retained", summary.InvalidPairs)
	}
}

func TestAggregatePairedStagesRejectsMissingPairManifestMismatchAndWrongOrder(t *testing.T) {
	schedule, err := buildPairSchedule(3)
	if err != nil {
		t.Fatal(err)
	}
	const manifestSHA = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	observations := make([]pairedStageObservation, 0, 6)
	for repetition := 1; repetition <= 3; repetition++ {
		for position, candidate := range schedule[repetition-1].Order {
			observations = append(observations, validPairedStageObservation(manifestSHA, repetition, position+1, candidate, int64(100+repetition)))
		}
	}

	t.Run("missing candidate result", func(t *testing.T) {
		if _, err := aggregatePairedStages(observations[:len(observations)-1], schedule, manifestSHA); err == nil {
			t.Fatal("an incomplete final pair must be rejected")
		}
	})
	t.Run("manifest mismatch", func(t *testing.T) {
		mismatched := append([]pairedStageObservation(nil), observations...)
		mismatched[0].ManifestSHA256 = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
		if _, err := aggregatePairedStages(mismatched, schedule, manifestSHA); err == nil {
			t.Fatal("a result from another frozen manifest must be rejected")
		}
	})
	t.Run("wrong candidate order", func(t *testing.T) {
		wrongOrder := append([]pairedStageObservation(nil), observations...)
		wrongOrder[0].PairPosition = 3 - wrongOrder[0].PairPosition
		if _, err := aggregatePairedStages(wrongOrder, schedule, manifestSHA); err == nil {
			t.Fatal("a result that violates the frozen pair schedule must be rejected")
		}
	})
}

func TestAggregatePairedStagesCommandRequiresFrozenManifestAndNeverOverwrites(t *testing.T) {
	schedule, err := buildPairSchedule(3)
	if err != nil {
		t.Fatal(err)
	}
	root := t.TempDir()
	manifestPath := filepath.Join(root, "manifest.json")
	manifest := officialManifest{
		OfficialFrozen:         true,
		RecoveryAssessmentMode: recoveryAssessmentMode,
		PairSchedule:           schedule,
		Inputs: []manifestInput{
			{Path: "input-1", SHA256: strings.Repeat("1", 64)},
			{Path: "input-2", SHA256: strings.Repeat("2", 64)},
			{Path: "input-3", SHA256: strings.Repeat("3", 64)},
			{Path: "input-4", SHA256: strings.Repeat("4", 64)},
			{Path: "input-5", SHA256: strings.Repeat("5", 64)},
			{Path: "input-6", SHA256: strings.Repeat("6", 64)},
			{Path: "input-7", SHA256: strings.Repeat("7", 64)},
		},
		Runner: manifestArtifact{Path: "runner.sh", SHA256: strings.Repeat("8", 64)},
		Helper: manifestHelper{SourcePath: "helper.go", SourceSHA256: strings.Repeat("9", 64), BinaryPath: "helper", BinarySHA256: strings.Repeat("a", 64), Version: helperVersion},
		Candidates: map[string]manifestCandidate{
			"go":   {SourceCommit: "go-source", BinaryPath: "mosdns-go", BinarySHA256: strings.Repeat("b", 64)},
			"rust": {SourceCommit: "rust-source", BinaryPath: "mosdns-rust", BinarySHA256: strings.Repeat("c", 64)},
		},
		Scenarios: map[string]manifestScenario{"w1-udp": {
			StageDurationMS: 3000, NormalReferenceQPS: 200, CommonLoadQPS: 400,
			NearSaturationQPS: 800, OverloadQPS: 1000, RequestDeadlineMS: 500,
			LateDrainMS: 100, HarnessCPUSet: "1", SUTCPUSet: "0",
			RecoveryMinimumSamples: 1, RecoveryP95CeilingUS: 1000, RecoveryP99CeilingUS: 2000,
		}},
	}
	if err := writeJSONFile(manifestPath, manifest); err != nil {
		t.Fatal(err)
	}
	manifestSHA, err := sha256File(manifestPath)
	if err != nil {
		t.Fatal(err)
	}
	resultsRoot := filepath.Join(root, "results")
	writeOfficialRunFixtures(t, resultsRoot, "w1-udp", schedule, manifestSHA, manifest, manifest.Scenarios["w1-udp"])
	outputPath := filepath.Join(root, "aggregate.json")
	args := []string{"--manifest", manifestPath, "--manifest-sha256", manifestSHA, "--results-root", resultsRoot, "--output", outputPath}
	if err := aggregatePairedStagesCommand(args); err != nil {
		t.Fatalf("frozen complete measurements should aggregate: %v", err)
	}
	var report pairedAggregationReport
	data, err := os.ReadFile(outputPath)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &report); err != nil {
		t.Fatal(err)
	}
	if report.ManifestSHA256 != manifestSHA || len(report.Groups) != len(continuousStageSequence) {
		t.Fatalf("unexpected aggregate report: %+v", report)
	}
	for _, group := range report.Groups {
		if group.ValidPairs != 3 {
			t.Fatalf("group %s has %d valid pairs, want 3", group.Stage, group.ValidPairs)
		}
	}
	if err := aggregatePairedStagesCommand(args); err == nil {
		t.Fatal("aggregation must refuse to overwrite an existing report")
	}
	badHashArgs := append([]string(nil), args...)
	badHashArgs[3] = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
	badHashArgs[7] = filepath.Join(root, "bad-hash-output.json")
	if err := aggregatePairedStagesCommand(badHashArgs); err == nil {
		t.Fatal("aggregation must reject an incorrect frozen manifest hash")
	}
}

func TestCollectPairedStageObservationsIncludesW2ColdWarmAndStageInvalidReasons(t *testing.T) {
	schedule, err := buildPairSchedule(3)
	if err != nil {
		t.Fatal(err)
	}
	root := t.TempDir()
	manifestPath := filepath.Join(root, "manifest.json")
	manifest := officialManifest{
		OfficialFrozen:         true,
		RecoveryAssessmentMode: recoveryAssessmentMode,
		PairSchedule:           schedule,
		Inputs:                 []manifestInput{{Path: "input", SHA256: strings.Repeat("1", 64)}},
		Runner:                 manifestArtifact{Path: "runner", SHA256: strings.Repeat("2", 64)},
		Helper:                 manifestHelper{SourcePath: "helper.go", SourceSHA256: strings.Repeat("3", 64), BinaryPath: "helper", BinarySHA256: strings.Repeat("4", 64), Version: helperVersion},
		Candidates: map[string]manifestCandidate{
			"go":   {SourceCommit: "go-source", BinaryPath: "mosdns-go", BinarySHA256: strings.Repeat("5", 64)},
			"rust": {SourceCommit: "rust-source", BinaryPath: "mosdns-rust", BinarySHA256: strings.Repeat("6", 64)},
		},
		Scenarios: map[string]manifestScenario{"w2": {
			StageDurationMS: 3000, NormalReferenceQPS: 200, CommonLoadQPS: 400,
			NearSaturationQPS: 800, OverloadQPS: 1000, RequestDeadlineMS: 500,
			LateDrainMS: 100, HarnessCPUSet: "1", SUTCPUSet: "0",
			RecoveryMinimumSamples: 1, RecoveryP95CeilingUS: 1000, RecoveryP99CeilingUS: 2000,
			W2WarmLifecycle: "same-process", W2CacheTTLMS: 30000, W2TTLSafetyMarginMS: 500,
		}},
	}
	if err := writeJSONFile(manifestPath, manifest); err != nil {
		t.Fatal(err)
	}
	manifestSHA, err := sha256File(manifestPath)
	if err != nil {
		t.Fatal(err)
	}
	resultsRoot := filepath.Join(root, "results")
	writeOfficialRunFixtures(t, resultsRoot, "w2", schedule, manifestSHA, manifest, manifest.Scenarios["w2"])
	failedGoRun := filepath.Join(resultsRoot, "w2", "repetition-2", "go")
	if err := os.WriteFile(filepath.Join(failedGoRun, "invalid-stages.tsv"), []byte("overload\tsender shortfall\n"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(failedGoRun, "attempt-exit-status.txt"), []byte("exit=1\n"), 0644); err != nil {
		t.Fatal(err)
	}

	observations, err := collectPairedStageObservations(resultsRoot, manifest, manifestSHA)
	if err != nil {
		t.Fatalf("complete W2 evidence should be collected: %v", err)
	}
	if len(observations) != len(schedule)*2*(len(continuousStageSequence)+1) {
		t.Fatalf("collected observations=%d, want one cold plus five warm stages per candidate and repetition", len(observations))
	}
	aggregates, err := aggregatePairedStages(observations, schedule, manifestSHA)
	if err != nil {
		t.Fatalf("W2 observations should aggregate: %v", err)
	}
	var cold, overload *pairedStageAggregate
	for i := range aggregates {
		if aggregates[i].Stage == "official-w2-cold" {
			cold = &aggregates[i]
		}
		if aggregates[i].Stage == "overload" {
			overload = &aggregates[i]
		}
	}
	if cold == nil || cold.ValidPairs != 3 {
		t.Fatalf("W2 cold aggregation=%+v, want three valid pairs", cold)
	}
	if overload == nil || overload.ValidPairs != 2 || len(overload.InvalidPairs) != 1 || overload.InvalidPairs[0].Reason != "sender shortfall" {
		t.Fatalf("W2 overload should retain and exclude its shortfall: %+v", overload)
	}
}

func TestExpandInvalidStageReasonsScopesRunLevelOracles(t *testing.T) {
	plan := manifestScenario{StageDurationMS: 3000, NormalReferenceQPS: 200, CommonLoadQPS: 400, NearSaturationQPS: 800, OverloadQPS: 1000}
	w2Stages, err := phase5aPlannedStages("w2", plan)
	if err != nil {
		t.Fatal(err)
	}
	w2Reasons, err := expandInvalidStageReasons("w2", w2Stages, map[string]string{"warm-prefill": "prefill failed", "w2-cold": "cold counter mismatch"})
	if err != nil {
		t.Fatal(err)
	}
	if w2Reasons["official-w2-cold"] != "w2-cold: cold counter mismatch" {
		t.Fatalf("cold lifecycle reason=%q", w2Reasons["official-w2-cold"])
	}
	for _, stage := range continuousStageSequence {
		if !strings.Contains(w2Reasons[stage], "warm-prefill: prefill failed") {
			t.Errorf("W2 stage %s did not inherit prefill failure: %q", stage, w2Reasons[stage])
		}
	}
	independentW2Stages, err := phase5aPlannedStages("w2", manifestScenario{
		StageDurationMS: 3000, NormalReferenceQPS: 200, CommonLoadQPS: 400,
		NearSaturationQPS: 800, OverloadQPS: 1000, W2WarmLifecycle: "independent-prefilled",
	})
	if err != nil {
		t.Fatal(err)
	}
	independentReasons, err := expandInvalidStageReasons("w2", independentW2Stages, map[string]string{"common-load-prefill": "per-key prefill failed"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(independentReasons["common-load"], "per-key prefill failed") {
		t.Fatalf("independent W2 prefill failure was not mapped to its measurement stage: %q", independentReasons["common-load"])
	}
	if independentReasons["normal-reference"] != "" {
		t.Fatalf("a stage-specific prefill failure contaminated another W2 stage: %q", independentReasons["normal-reference"])
	}

	w3Stages, err := phase5aPlannedStages("w3", plan)
	if err != nil {
		t.Fatal(err)
	}
	w3Reasons, err := expandInvalidStageReasons("w3", w3Stages, map[string]string{"events": "tail mismatch"})
	if err != nil {
		t.Fatal(err)
	}
	for _, stage := range continuousStageSequence {
		if !strings.Contains(w3Reasons[stage], "events: tail mismatch") {
			t.Errorf("W3 stage %s did not inherit event journal failure: %q", stage, w3Reasons[stage])
		}
	}
	if _, err := expandInvalidStageReasons("w3", w3Stages, map[string]string{"unexpected": "unknown"}); err == nil {
		t.Fatal("unrecognized invalid-stage scope must be rejected")
	}
}

func writeOfficialRunFixtures(t *testing.T, resultsRoot, scenario string, schedule []manifestPair, manifestSHA string, manifest officialManifest, plan manifestScenario) {
	t.Helper()
	stageQPS := map[string]float64{
		"normal-reference": plan.NormalReferenceQPS,
		"common-load":      plan.CommonLoadQPS,
		"near-saturation":  plan.NearSaturationQPS,
		"overload":         plan.OverloadQPS,
		"recovery":         plan.NormalReferenceQPS,
	}
	for _, pair := range schedule {
		for position, candidate := range pair.Order {
			runDir := filepath.Join(resultsRoot, scenario, "repetition-"+strconv.Itoa(pair.Repetition), candidate)
			if err := os.MkdirAll(runDir, 0755); err != nil {
				t.Fatal(err)
			}
			metadata := strings.Join([]string{
				"scenario=" + scenario,
				"run_mode=official",
				"recovery_assessment_mode=" + recoveryAssessmentMode,
				"candidate=" + candidate,
				"repetition=" + strconv.Itoa(pair.Repetition),
				"pair_position=" + strconv.Itoa(position+1),
				"manifest_sha256=" + manifestSHA,
			}, "\n") + "\n"
			if scenario == "w2" {
				metadata += "w2_warm_lifecycle=" + plan.W2WarmLifecycle + "\n"
			}
			if err := os.WriteFile(filepath.Join(runDir, "run-metadata.txt"), []byte(metadata), 0644); err != nil {
				t.Fatal(err)
			}
			assessmentReason := "no frozen overload-evidence criterion; this is a post-sequence same-rate health check"
			if scenario == "w2" && plan.W2WarmLifecycle == "independent-prefilled" {
				assessmentReason = "independent-prefilled W2 sessions; no same-process recovery is measured"
			}
			assessment := "status=indeterminate\nmode=" + recoveryAssessmentMode + "\nreason=" + assessmentReason + "\n"
			if err := os.WriteFile(filepath.Join(runDir, "service-recovery-assessment.txt"), []byte(assessment), 0644); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(filepath.Join(runDir, "manifest.sha256"), []byte(manifestSHA+"  manifest.json\n"), 0644); err != nil {
				t.Fatal(err)
			}
			var hashes strings.Builder
			for _, input := range manifest.Inputs {
				hashes.WriteString(input.SHA256 + "  " + input.Path + "\n")
			}
			hashes.WriteString(manifest.Runner.SHA256 + "  " + manifest.Runner.Path + "\n")
			hashes.WriteString(manifest.Helper.SourceSHA256 + "  " + manifest.Helper.SourcePath + "\n")
			hashes.WriteString(manifest.Helper.BinarySHA256 + "  " + manifest.Helper.BinaryPath + "\n")
			hashes.WriteString(manifest.Candidates[candidate].BinarySHA256 + "  " + manifest.Candidates[candidate].BinaryPath + "\n")
			if err := os.WriteFile(filepath.Join(runDir, "input-hashes.sha256"), []byte(hashes.String()), 0644); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(filepath.Join(runDir, "attempt-exit-status.txt"), []byte("exit=0\n"), 0644); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(filepath.Join(runDir, "helper-version.txt"), []byte(helperVersion+"\n"), 0644); err != nil {
				t.Fatal(err)
			}
			workloadScenario, transport := "w1", "udp"
			if scenario == "w2" {
				workloadScenario = "w2"
			} else if scenario == "w3" {
				workloadScenario = "w3"
			} else if scenario == "w1-tcp" {
				transport = "tcp"
			}
			makeStage := func(name string, qps float64) stageResult {
				scheduled := int64(float64(plan.StageDurationMS) * qps / 1000)
				return stageResult{
					Stage: name, Scenario: workloadScenario, Transport: transport, TargetQPS: qps,
					DurationMS: plan.StageDurationMS, RequestDeadlineMS: plan.RequestDeadlineMS, LateDrainMS: plan.LateDrainMS,
					Counters:         stageCounters{Scheduled: scheduled, Sent: scheduled, Received: scheduled, CorrectOnTime: scheduled},
					LatencySamplesUS: make([]int64, scheduled), P50US: 100, P95US: 200, P99US: 300,
				}
			}
			if scenario == "w2" {
				coldDir := filepath.Join(runDir, "w2-cold")
				warmDir := filepath.Join(runDir, "w2-warm")
				if err := os.MkdirAll(coldDir, 0755); err != nil {
					t.Fatal(err)
				}
				writeJSONLines(t, filepath.Join(coldDir, "stages.jsonl"), []stageResult{makeStage("official-w2-cold", plan.NormalReferenceQPS)})
				if plan.W2WarmLifecycle == "independent-prefilled" {
					independentDir := filepath.Join(runDir, "w2-warm-independent")
					if err := os.MkdirAll(independentDir, 0755); err != nil {
						t.Fatal(err)
					}
					if err := os.WriteFile(filepath.Join(independentDir, "recovery-status.txt"), []byte("indeterminate: independent-prefilled W2 sessions; no same-process recovery is measured\n"), 0644); err != nil {
						t.Fatal(err)
					}
					for _, name := range continuousStageSequence {
						stageDir := filepath.Join(independentDir, name)
						if err := os.MkdirAll(filepath.Join(stageDir, "prefill"), 0755); err != nil {
							t.Fatal(err)
						}
						writeJSONLines(t, filepath.Join(stageDir, "stages.jsonl"), []stageResult{makeStage(name, stageQPS[name])})
						writeJSONLines(t, filepath.Join(stageDir, "prefill", "stages.jsonl"), []stageResult{makeStage("warm-prefill", plan.NormalReferenceQPS)})
					}
				} else {
					if err := os.MkdirAll(warmDir, 0755); err != nil {
						t.Fatal(err)
					}
					warmStages := make([]stageResult, 0, len(continuousStageSequence))
					for _, name := range continuousStageSequence {
						warmStages = append(warmStages, makeStage(name, stageQPS[name]))
					}
					writeJSONLines(t, filepath.Join(warmDir, "stages.jsonl"), warmStages)
					if err := os.WriteFile(filepath.Join(warmDir, "recovery-status.txt"), []byte("TTL-eligible\n"), 0644); err != nil {
						t.Fatal(err)
					}
				}
			} else {
				stages := make([]stageResult, 0, len(continuousStageSequence))
				for _, name := range continuousStageSequence {
					stages = append(stages, makeStage(name, stageQPS[name]))
				}
				writeJSONLines(t, filepath.Join(runDir, "stages.jsonl"), stages)
			}
		}
	}
}

func validPairedStageObservation(manifestSHA string, repetition, position int, candidate string, p95 int64) pairedStageObservation {
	return pairedStageObservation{
		Scenario:           "w1-udp",
		Stage:              "normal-reference",
		Candidate:          candidate,
		ManifestSHA256:     manifestSHA,
		Repetition:         repetition,
		PairPosition:       position,
		TargetQPS:          200,
		DurationMS:         3000,
		Counters:           stageCounters{Scheduled: 600, Sent: 600, Received: 600, CorrectOnTime: 600},
		P50US:              100,
		P95US:              p95,
		P99US:              p95 + 50,
		LatencySampleCount: 600,
	}
}

func TestOfficialManifestFreezesW2WarmLifecycle(t *testing.T) {
	root := t.TempDir()
	manifest, opts := validManifestFixture(t, root)
	w2Plan := manifest.Scenarios["w1-udp"]
	w2Plan.W2WarmLifecycle = "same-process"
	manifest.Scenarios["w2"] = w2Plan
	opts.Scenario = "w2"
	opts.W2WarmLifecycle = "same-process"
	manifestPath := filepath.Join(root, "manifest.json")
	if err := writeJSONFile(manifestPath, manifest); err != nil {
		t.Fatal(err)
	}
	manifestHash, err := sha256File(manifestPath)
	if err != nil {
		t.Fatal(err)
	}
	opts.ManifestPath = manifestPath
	opts.ExpectedSHA256 = manifestHash
	if err := verifyOfficialManifest(opts); err != nil {
		t.Fatalf("matching W2 lifecycle should pass: %v", err)
	}
	opts.W2WarmLifecycle = "independent-prefilled"
	if err := verifyOfficialManifest(opts); err == nil {
		t.Fatal("changing W2 lifecycle after freeze must fail")
	}
}

func validContinuousStages() []stageResult {
	base := time.Date(2026, time.September, 23, 12, 0, 0, 0, time.UTC)
	names := []string{"normal-reference", "common-load", "near-saturation", "overload", "recovery"}
	qps := []float64{10, 20, 30, 40, 10}
	stages := make([]stageResult, len(names))
	for i, name := range names {
		start := base.Add(time.Duration(i) * 2 * time.Second)
		stages[i] = stageResult{
			Stage: name, RunID: "run-1", FixtureSessionID: "fixtures-1", Scenario: "w1", Transport: "udp",
			TargetQPS: qps[i], DurationMS: 1000, StartedAt: start, FinishedAt: start.Add(time.Second),
			SUTPID: 123, SUTStartIdentity: "boot:456", SUTCPUSet: "0", HarnessPID: 200 + i, HarnessCPUSet: "1", RequestSeqStart: uint64(i*10 + 1), RequestSeqEnd: uint64((i + 1) * 10),
			FixtureSeqStart: uint64(i * 10), FixtureSeqEnd: uint64((i + 1) * 10),
			Counters:         stageCounters{Scheduled: 10, Sent: 10, Received: 10, CorrectOnTime: 10},
			LatencySamplesUS: []int64{800, 850, 900, 950, 1000, 1050, 1080, 1100, 1150, 1200}, P95US: 1200, P99US: 1200,
		}
	}
	return stages
}

func validManifestFixture(t *testing.T, root string) (officialManifest, manifestValidationOptions) {
	t.Helper()
	for _, rel := range fixedCorpusInputPaths {
		path := filepath.Join(root, rel)
		if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte("frozen:"+rel), 0644); err != nil {
			t.Fatal(err)
		}
	}
	for _, rel := range []string{"scripts/run-phase5a-baseline.sh", "tests/phase5a-baseline/cmd/phase5a-baseline/main.go"} {
		path := filepath.Join(root, rel)
		if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte("frozen:"+rel), 0644); err != nil {
			t.Fatal(err)
		}
	}
	helperPath := filepath.Join(root, "helper")
	goPath := filepath.Join(root, "go-mosdns")
	rustPath := filepath.Join(root, "rust-mosdns")
	for _, path := range []string{helperPath, goPath, rustPath} {
		if err := os.WriteFile(path, []byte("binary:"+filepath.Base(path)), 0755); err != nil {
			t.Fatal(err)
		}
	}
	inputs := make([]manifestInput, 0, len(fixedCorpusInputPaths))
	for _, rel := range fixedCorpusInputPaths {
		digest, err := sha256File(filepath.Join(root, rel))
		if err != nil {
			t.Fatal(err)
		}
		inputs = append(inputs, manifestInput{Path: rel, SHA256: digest})
	}
	runnerRel := "scripts/run-phase5a-baseline.sh"
	helperSourceRel := "tests/phase5a-baseline/cmd/phase5a-baseline/main.go"
	runnerHash, err := sha256File(filepath.Join(root, runnerRel))
	if err != nil {
		t.Fatal(err)
	}
	helperSourceHash, err := sha256File(filepath.Join(root, helperSourceRel))
	if err != nil {
		t.Fatal(err)
	}
	helperHash, err := sha256File(helperPath)
	if err != nil {
		t.Fatal(err)
	}
	goHash, err := sha256File(goPath)
	if err != nil {
		t.Fatal(err)
	}
	rustHash, err := sha256File(rustPath)
	if err != nil {
		t.Fatal(err)
	}
	manifest := officialManifest{
		SchemaVersion: 1, OfficialFrozen: true, RecoveryAssessmentMode: recoveryAssessmentMode, Inputs: inputs,
		Runner: manifestArtifact{Path: runnerRel, SHA256: runnerHash},
		Helper: manifestHelper{SourcePath: helperSourceRel, SourceSHA256: helperSourceHash, BinaryPath: helperPath, BinarySHA256: helperHash, Version: helperVersion},
		Candidates: map[string]manifestCandidate{
			"go":   {SourceCommit: "go-source-sha", BinaryPath: goPath, BinarySHA256: goHash},
			"rust": {SourceCommit: "rust-source-sha", BinaryPath: rustPath, BinarySHA256: rustHash},
		},
		StageSequence: []string{"normal-reference", "common-load", "near-saturation", "overload", "recovery"},
		W3EventSchema: fixtureEventSchema,
		PairSchedule: []manifestPair{
			{Repetition: 1, Order: []string{"go", "rust"}},
			{Repetition: 2, Order: []string{"rust", "go"}},
			{Repetition: 3, Order: []string{"go", "rust"}},
		},
		Scenarios: map[string]manifestScenario{"w1-udp": {
			StageDurationMS: 1000, NormalReferenceQPS: 10, CommonLoadQPS: 20, NearSaturationQPS: 30, OverloadQPS: 40,
			RequestDeadlineMS: 500, LateDrainMS: 100, TCPPolicy: "fresh-connection-per-request",
			W2CacheTTLMS: 30000, W2TTLSafetyMarginMS: 500,
			HarnessCPUSet: "0", SUTCPUSet: "1",
			RecoveryMinimumSamples: 5, RecoveryP95CeilingUS: 1200, RecoveryP99CeilingUS: 1500,
		}},
		Environment: manifestEnvironment{HostAlias: "test-vm", GOOS: runtime.GOOS, GOARCH: runtime.GOARCH, OnlineCPUs: currentOnlineCPUCount(), KernelRelease: currentKernelRelease(), GoToolchain: runtime.Version(), RustToolchain: "rustc test-version"},
	}
	opts := manifestValidationOptions{
		RepoRoot: root, HelperPath: helperPath, RunnerPath: filepath.Join(root, runnerRel), SUTPath: goPath,
		Candidate: "go", Scenario: "w1-udp", Repetition: 1, Position: 1, StageDurationMS: 1000,
		NormalReferenceQPS: 10, CommonLoadQPS: 20, NearSaturationQPS: 30, OverloadQPS: 40,
		RequestDeadlineMS: 500, LateDrainMS: 100, W2CacheTTLMS: 30000, W2TTLSafetyMarginMS: 500,
		RecoveryMinimumSamples: 5, RecoveryP95CeilingUS: 1200, RecoveryP99CeilingUS: 1500,
		HarnessCPUSet: "0", SUTCPUSet: "1", HostAlias: "test-vm", RustToolchain: "rustc test-version",
	}
	return manifest, opts
}

func writeJSONFile(path string, value any) error {
	f, err := os.Create(path)
	if err != nil {
		return err
	}
	if err := json.NewEncoder(f).Encode(value); err != nil {
		_ = f.Close()
		return err
	}
	return f.Close()
}

func validRouteEvidence() ([]workloadCase, []requestRecord, []fixtureEvent, stageResult) {
	base := time.Date(2026, time.September, 23, 12, 0, 0, 0, time.UTC)
	cases := []workloadCase{
		{CaseID: "domain-hit", Scenario: "w3", Transport: "udp", QName: "domain-hit.test.", QType: "A", ExpectedRouteClass: "DOMAIN_HIT", Weight: 1},
		{CaseID: "ip-rule-hit", Scenario: "w3", Transport: "udp", QName: "ip-hit.test.", QType: "A", ExpectedRouteClass: "IP_RULE_HIT", Weight: 1},
		{CaseID: "ip-rule-miss", Scenario: "w3", Transport: "udp", QName: "ip-miss.test.", QType: "A", ExpectedRouteClass: "IP_RULE_MISS", Weight: 1},
	}
	requests := []requestRecord{
		{RunID: "run-1", StageID: "w3-normal", RequestSeq: 1, DNSID: 101, CaseID: "domain-hit", QName: cases[0].QName, QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base, FinishedAt: base.Add(10 * time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "w3-normal", RequestSeq: 2, DNSID: 102, CaseID: "ip-rule-hit", QName: cases[1].QName, QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(time.Millisecond), FinishedAt: base.Add(11 * time.Millisecond), Outcome: "correct_on_time"},
		{RunID: "run-1", StageID: "w3-normal", RequestSeq: 3, DNSID: 103, CaseID: "ip-rule-miss", QName: cases[2].QName, QType: "A", QClass: dns.ClassINET, Sent: true, SentAt: base.Add(2 * time.Millisecond), FinishedAt: base.Add(12 * time.Millisecond), Outcome: "correct_on_time"},
	}
	events := []fixtureEvent{
		{FixtureSeq: 101, OccurredAt: base.Add(5 * time.Millisecond), DNSID: 0, QName: cases[0].QName, QType: dns.TypeA, QClass: dns.ClassINET, Upstream: "route-a"},
		{FixtureSeq: 102, OccurredAt: base.Add(3 * time.Millisecond), DNSID: 0, QName: cases[1].QName, QType: dns.TypeA, QClass: dns.ClassINET, Upstream: "route-b"},
		{FixtureSeq: 103, OccurredAt: base.Add(4 * time.Millisecond), DNSID: 1, QName: cases[1].QName, QType: dns.TypeA, QClass: dns.ClassINET, Upstream: "route-a"},
		{FixtureSeq: 104, OccurredAt: base.Add(6 * time.Millisecond), DNSID: 0, QName: cases[2].QName, QType: dns.TypeA, QClass: dns.ClassINET, Upstream: "route-b"},
		{FixtureSeq: 105, OccurredAt: base.Add(7 * time.Millisecond), DNSID: 0, QName: cases[2].QName, QType: dns.TypeA, QClass: dns.ClassINET, Upstream: "route-c"},
	}
	stage := stageResult{RunID: "run-1", Stage: "w3-normal", FixtureSeqStart: 100, FixtureSeqEnd: 105, RequestSeqStart: 1, RequestSeqEnd: 3, Counters: stageCounters{Sent: 3, CorrectOnTime: 3}}
	return cases, requests, events, stage
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

func TestFixtureResponseJournalsRouteEventBeforeReturningResponse(t *testing.T) {
	query := new(dns.Msg)
	query.SetQuestion("IP-HIT.test.", dns.TypeA)
	query.Id = 4242
	wire, err := query.Pack()
	if err != nil {
		t.Fatal(err)
	}
	journalPath := filepath.Join(t.TempDir(), "events.jsonl")
	journal := newFixtureEventJournal(journalPath)
	counts := &counterStore{upstream: "route-b", values: make(map[string]int64)}
	responseWire, ok := fixtureResponse(wire, fixtureOptions{upstreamID: "route-b"}, counts, journal)
	if !ok {
		t.Fatal("fixture response should succeed")
	}
	response := new(dns.Msg)
	if err := response.Unpack(responseWire); err != nil {
		t.Fatal(err)
	}
	if !response.Response || response.Id != query.Id {
		t.Fatalf("unexpected fixture response header: %#v", response.MsgHdr)
	}
	events, err := readFixtureEvents(journalPath)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 1 {
		t.Fatalf("fixture journal contains %d events, want 1", len(events))
	}
	want := fixtureEvent{FixtureSeq: 1, DNSID: query.Id, QName: "ip-hit.test.", QType: dns.TypeA, QClass: dns.ClassINET, Upstream: "route-b"}
	if events[0].FixtureSeq != want.FixtureSeq || events[0].DNSID != want.DNSID || events[0].QName != want.QName || events[0].QType != want.QType || events[0].QClass != want.QClass || events[0].Upstream != want.Upstream || events[0].OccurredAt.IsZero() {
		t.Fatalf("fixture event=%+v, want fields %+v with an occurrence timestamp", events[0], want)
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

func TestVerifyCountersRejectsUnexpectedColdAndWarmDeltas(t *testing.T) {
	dir := t.TempDir()
	workloadPath := filepath.Join(dir, "cache.jsonl")
	workload := "" +
		`{"case_id":"a","scenario":"w2","transport":"udp","qname":"a.test.","qtype":"A","expected_rcode":0,"expected_answer_class":"A","expected_answer":"198.51.100.20","expected_route_class":"cache","request_deadline_ms":500,"weight":1}` + "\n" +
		`{"case_id":"b","scenario":"w2","transport":"udp","qname":"b.test.","qtype":"A","expected_rcode":0,"expected_answer_class":"A","expected_answer":"198.51.100.21","expected_route_class":"cache","request_deadline_ms":500,"weight":1}` + "\n"
	if err := os.WriteFile(workloadPath, []byte(workload), 0644); err != nil {
		t.Fatal(err)
	}
	stagePath := filepath.Join(dir, "stages.jsonl")
	writeJSONLines(t, stagePath, []stageResult{{Stage: "cold", CaseScheduled: map[string]int64{"a": 1, "b": 1}}})
	counterPath := filepath.Join(dir, "counter.json")
	baselinePath := filepath.Join(dir, "baseline.json")
	writeCounterTestFile(t, baselinePath, "cache", map[string]int64{"a.test.|A": 0, "b.test.|A": 0})
	writeCounterTestFile(t, counterPath, "cache", map[string]int64{"a.test.|A": 1, "b.test.|A": 1, "unexpected.test.|A": 1})
	coldArgs := []string{"--scenario", "w2", "--workload", workloadPath, "--counter", counterPath, "--baseline", baselinePath, "--expect-delta", "--stage-result", stagePath, "--stage", "cold"}
	if err := verifyCounters(coldArgs); err == nil {
		t.Fatal("unexpected cold upstream request must fail exact counter verification")
	}

	writeCounterTestFile(t, baselinePath, "cache", map[string]int64{"a.test.|A": 1, "b.test.|A": 1})
	writeCounterTestFile(t, counterPath, "cache", map[string]int64{"a.test.|A": 1, "b.test.|A": 1, "unexpected.test.|A": 1})
	warmArgs := []string{"--scenario", "w2", "--workload", workloadPath, "--counter", counterPath, "--baseline", baselinePath, "--stage-result", stagePath, "--stage", "cold"}
	if err := verifyCounters(warmArgs); err == nil {
		t.Fatal("unexpected warm upstream request must fail zero-delta verification")
	}
}

func TestVerifyCountersExpectsOneColdMissPerCacheKey(t *testing.T) {
	dir := t.TempDir()
	workloadPath := filepath.Join(dir, "cache.jsonl")
	workload := "" +
		`{"case_id":"a","scenario":"w2","transport":"udp","qname":"a.test.","qtype":"A","expected_rcode":0,"expected_answer_class":"A","expected_answer":"198.51.100.20","expected_route_class":"cache","request_deadline_ms":500,"weight":1}` + "\n" +
		`{"case_id":"b","scenario":"w2","transport":"udp","qname":"b.test.","qtype":"A","expected_rcode":0,"expected_answer_class":"A","expected_answer":"198.51.100.21","expected_route_class":"cache","request_deadline_ms":500,"weight":1}` + "\n"
	if err := os.WriteFile(workloadPath, []byte(workload), 0644); err != nil {
		t.Fatal(err)
	}
	stagePath := filepath.Join(dir, "stages.jsonl")
	writeJSONLines(t, stagePath, []stageResult{{Stage: "cold", CaseScheduled: map[string]int64{"a": 30, "b": 30}}})
	counterPath := filepath.Join(dir, "counter.json")
	baselinePath := filepath.Join(dir, "baseline.json")
	writeCounterTestFile(t, baselinePath, "cache", map[string]int64{"a.test.|A": 0, "b.test.|A": 0})
	writeCounterTestFile(t, counterPath, "cache", map[string]int64{"a.test.|A": 1, "b.test.|A": 1})
	args := []string{"--scenario", "w2", "--workload", workloadPath, "--counter", counterPath, "--baseline", baselinePath, "--expect-delta", "--stage-result", stagePath, "--stage", "cold"}
	if err := verifyCounters(args); err != nil {
		t.Fatalf("cold cache should miss once per unique key, not once per request: %v", err)
	}
	writeCounterTestFile(t, counterPath, "cache", map[string]int64{"a.test.|A": 2, "b.test.|A": 1})
	if err := verifyCounters(args); err == nil {
		t.Fatal("more than one cold miss for a cache key must fail exact verification")
	}
}

func TestVerifySessionCountersCountsOneW2MissPerCacheKey(t *testing.T) {
	cases := []workloadCase{
		{CaseID: "a", QName: "a.test.", QType: "A"},
		{CaseID: "b", QName: "b.test.", QType: "A"},
	}
	stages := []stageResult{{
		Stage: "cold", Scenario: "w2", CaseScheduled: map[string]int64{"a": 30, "b": 30},
	}}
	path := filepath.Join(t.TempDir(), "counter.json")
	writeCounterTestFile(t, path, "cache", map[string]int64{"a.test.|A": 1, "b.test.|A": 1})
	if err := verifySessionCounters(cases, stages, "w2", path); err != nil {
		t.Fatalf("W2 cold session should miss once per unique key: %v", err)
	}
	writeCounterTestFile(t, path, "cache", map[string]int64{"a.test.|A": 2, "b.test.|A": 1})
	if err := verifySessionCounters(cases, stages, "w2", path); err == nil {
		t.Fatal("extra W2 cold upstream query must fail session counter verification")
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

func writeJSONLines(t *testing.T, path string, values any) {
	t.Helper()
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	encoder := json.NewEncoder(f)
	switch rows := values.(type) {
	case []workloadCase:
		for _, row := range rows {
			if err := encoder.Encode(row); err != nil {
				t.Fatal(err)
			}
		}
	case []requestRecord:
		for _, row := range rows {
			if err := encoder.Encode(row); err != nil {
				t.Fatal(err)
			}
		}
	case []fixtureEvent:
		for _, row := range rows {
			if err := encoder.Encode(row); err != nil {
				t.Fatal(err)
			}
		}
	case []stageResult:
		for _, row := range rows {
			if err := encoder.Encode(row); err != nil {
				t.Fatal(err)
			}
		}
	case []pairedStageObservation:
		for _, row := range rows {
			if err := encoder.Encode(row); err != nil {
				t.Fatal(err)
			}
		}
	default:
		t.Fatalf("unsupported JSONL row type %T", values)
	}
}

func writeFile(path, content string) error {
	return os.WriteFile(path, []byte(content), 0644)
}
