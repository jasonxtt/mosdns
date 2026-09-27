package main

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/miekg/dns"
)

func TestReliabilitySlotAccountingIsRecomputableFromControlRecords(t *testing.T) {
	records := []reliabilitySlotRecord{
		{SlotID: 0, Terminal: reliabilityTerminalCorrectOnTime, DNSSent: true, FinishOffsetUS: ptrInt64(10), PlannedOffsetUS: 0, DispatchOffsetUS: ptrInt64(1), WriteStartOffsetUS: ptrInt64(2)},
		{SlotID: 1, Terminal: reliabilityTerminalFailedBeforeDNSSend, DNSSent: false, PlannedOffsetUS: 100, DispatchOffsetUS: ptrInt64(101), FinishOffsetUS: ptrInt64(110)},
		{SlotID: 2, Terminal: reliabilityTerminalHarnessSkipped, DNSSent: false, PlannedOffsetUS: 200},
		{SlotID: 3, Terminal: reliabilityTerminalHarnessRejected, DNSSent: false, PlannedOffsetUS: 300},
		{SlotID: 4, Terminal: reliabilityTerminalCorrectLate, DNSSent: true, FinishOffsetUS: ptrInt64(420), PlannedOffsetUS: 400, DispatchOffsetUS: ptrInt64(401), WriteStartOffsetUS: ptrInt64(402)},
	}
	accounting, err := recomputeReliabilityAccounting(records)
	if err != nil {
		t.Fatal(err)
	}
	if accounting.Planned != 5 || accounting.Started != 3 || accounting.HarnessSkipped != 1 || accounting.HarnessRejected != 1 || accounting.FailedBeforeDNSSend != 1 || accounting.DNSSent != 2 {
		t.Fatalf("unexpected accounting: %+v", accounting)
	}
	if accounting.PostSendTerminals[reliabilityTerminalCorrectOnTime] != 1 || accounting.PostSendTerminals[reliabilityTerminalCorrectLate] != 1 {
		t.Fatalf("unexpected post-send terminals: %+v", accounting.PostSendTerminals)
	}
	if err := validateReliabilityAccounting(accounting); err != nil {
		t.Fatal(err)
	}
}

func TestReliabilitySlotAccountingRejectsDuplicateTerminalOrDNSMismatch(t *testing.T) {
	duplicate := []reliabilitySlotRecord{{SlotID: 1, Terminal: reliabilityTerminalCorrectOnTime, DNSSent: true, FinishOffsetUS: ptrInt64(1), PlannedOffsetUS: 0, DuplicatePackets: 1}}
	accounting, err := recomputeReliabilityAccounting(duplicate)
	if err != nil || accounting.DuplicatePackets != 1 {
		t.Fatalf("duplicate packets must remain a diagnostic counter: accounting=%+v err=%v", accounting, err)
	}
	wrongDNS := []reliabilitySlotRecord{{SlotID: 1, Terminal: reliabilityTerminalCorrectOnTime, DNSSent: false, FinishOffsetUS: ptrInt64(1), PlannedOffsetUS: 0}}
	if _, err := recomputeReliabilityAccounting(wrongDNS); err == nil || !strings.Contains(err.Error(), "dns_sent") {
		t.Fatalf("post-send terminal without dns_sent must fail: %v", err)
	}
}

func TestReliabilityAbsoluteDeadlineUsesOneMonotonicBudget(t *testing.T) {
	deadlines := makeReliabilityDeadlines(20*time.Millisecond, 50*time.Millisecond, 10*time.Millisecond)
	if deadlines.Service != 70*time.Millisecond || deadlines.Collection != 80*time.Millisecond {
		t.Fatalf("unexpected deadlines: %+v", deadlines)
	}
	if got := reliabilityRemainingBudget(deadlines.Service, 40*time.Millisecond); got != 30*time.Millisecond {
		t.Fatalf("queue/connect must share remaining service budget, got %s", got)
	}
	if got := reliabilityRemainingBudget(deadlines.Service, 80*time.Millisecond); got != 0 {
		t.Fatalf("expired service deadline must not produce a new budget, got %s", got)
	}
	if !reliabilityIsLate(deadlines, 75*time.Millisecond) || reliabilityIsLate(deadlines, 70*time.Millisecond) {
		t.Fatal("late classification must use the service deadline boundary")
	}
}

func TestReliabilityWallClockJumpsDoNotChangeMonotonicClassification(t *testing.T) {
	clock := newFakeReliabilityClock()
	deadlines := makeReliabilityDeadlines(0, 50*time.Millisecond, 10*time.Millisecond)
	clock.AdvanceWall(72 * time.Hour)
	clock.Advance(40 * time.Millisecond)
	remainingBeforeJump := reliabilityRemainingBudget(deadlines.Service, clock.Now())
	onTime := classifyReliabilityExchange(reliabilityExchangeResult{BytesWritten: 1, FrameBytes: 1, ResponseAt: clock.Now(), ResponseOK: true}, &reliabilityExpected{ServiceDeadline: deadlines.Service, CollectionDeadline: deadlines.Collection})
	clock.AdvanceWall(-144 * time.Hour)
	remainingAfterJump := reliabilityRemainingBudget(deadlines.Service, clock.Now())
	late := classifyReliabilityExchange(reliabilityExchangeResult{BytesWritten: 1, FrameBytes: 1, ResponseAt: 55 * time.Millisecond, ResponseOK: true}, &reliabilityExpected{ServiceDeadline: deadlines.Service, CollectionDeadline: deadlines.Collection})
	if remainingBeforeJump != 10*time.Millisecond || remainingAfterJump != remainingBeforeJump || onTime.Terminal != reliabilityTerminalCorrectOnTime || late.Terminal != reliabilityTerminalCorrectLate {
		t.Fatalf("wall-clock jumps must not change monotonic deadlines or classification: before=%s after=%s ontime=%+v late=%+v", remainingBeforeJump, remainingAfterJump, onTime, late)
	}
}

func TestReliabilityExchangeRequiresCompleteDNSFrame(t *testing.T) {
	result := classifyReliabilityExchange(reliabilityExchangeResult{BytesWritten: 3, FrameBytes: 4, ResponseAt: 5 * time.Millisecond}, nil)
	if result.DNSSent || result.Terminal != reliabilityTerminalFailedBeforeDNSSend {
		t.Fatalf("partial write must not be dns_sent: %+v", result)
	}
	result = classifyReliabilityExchange(reliabilityExchangeResult{BytesWritten: 4, FrameBytes: 4, ResponseAt: 60 * time.Millisecond, ResponseOK: true}, &reliabilityExpected{ServiceDeadline: 50 * time.Millisecond, CollectionDeadline: 100 * time.Millisecond})
	if !result.DNSSent || result.Terminal != reliabilityTerminalCorrectLate {
		t.Fatalf("complete send after service deadline must be late only when response is collectable: %+v", result)
	}
}

func TestReliabilityTransportWriteStopsAtAbsoluteDeadlineAfterPartialWrite(t *testing.T) {
	clock := newFakeReliabilityClock()
	conn := &partialReliabilityConn{clock: clock, advance: 6 * time.Millisecond}
	written, err := writeReliabilityFrame(conn, []byte("1234"), 5*time.Millisecond, clock)
	if written != 2 || err == nil || !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("partial write must stop at the original deadline: written=%d err=%v", written, err)
	}
	classified := classifyReliabilityExchange(reliabilityExchangeResult{BytesWritten: written, FrameBytes: 4}, &reliabilityExpected{ServiceDeadline: 5 * time.Millisecond, CollectionDeadline: 10 * time.Millisecond})
	if classified.DNSSent || classified.Terminal != reliabilityTerminalFailedBeforeDNSSend {
		t.Fatalf("partial framed write must not become dns_sent: %+v", classified)
	}
}

func TestReliabilityWriteOffsetsCaptureCompleteWriteDeadlineRace(t *testing.T) {
	clock := newFakeReliabilityClock()
	conn := &advancingReliabilityConn{clock: clock, advance: 6 * time.Millisecond}
	writeResult := writeReliabilityFrameDetailed(conn, []byte("1234"), 5*time.Millisecond, clock)
	if writeResult.Err != nil || writeResult.Written != 4 || !writeResult.HasStart || !writeResult.HasComplete || !writeResult.DeadlineRace {
		t.Fatalf("complete write crossing the service deadline must remain observable: %+v", writeResult)
	}
	if writeResult.Start != 0 || writeResult.Complete != 6*time.Millisecond {
		t.Fatalf("write offsets must use the monotonic run clock: %+v", writeResult)
	}
	classified := classifyReliabilityExchange(reliabilityExchangeResult{
		BytesWritten:      writeResult.Written,
		FrameBytes:        4,
		WriteDeadlineRace: writeResult.DeadlineRace,
		ResponseAt:        4 * time.Millisecond,
		ResponseOK:        true,
	}, &reliabilityExpected{ServiceDeadline: 5 * time.Millisecond, CollectionDeadline: 10 * time.Millisecond})
	if classified.Terminal != reliabilityTerminalCorrectLate || classified.ResponseAt != 4*time.Millisecond {
		t.Fatalf("write deadline race must never become on-time: %+v", classified)
	}
}

func TestReliabilityNetworkTransportRejectsWriteAfterSlowConnect(t *testing.T) {
	clock := newFakeReliabilityClock()
	client, peer := net.Pipe()
	defer client.Close()
	defer peer.Close()
	exchange := &netReliabilityExchange{
		address:   "127.0.0.1:1",
		transport: "tcp",
		clock:     clock,
		dial: func(context.Context, string, string) (net.Conn, error) {
			clock.Advance(6 * time.Millisecond)
			return client, nil
		},
	}
	result := exchange.Exchange(context.Background(), reliabilityRequest{FrameBytes: 1, Payload: []byte{1}}, makeReliabilityDeadlines(0, 5*time.Millisecond, time.Millisecond))
	if result.BytesWritten != 0 || result.HasWriteStart || !result.TimedOut || !errors.Is(result.Err, context.DeadlineExceeded) {
		t.Fatalf("slow connect must not receive a renewed write budget: %+v", result)
	}
}

func TestReliabilityNetworkTransportRecordsTCPFrameAndWriteRace(t *testing.T) {
	clock := newFakeReliabilityClock()
	query := new(dns.Msg)
	query.SetQuestion("case.test.", dns.TypeA)
	payload, err := query.Pack()
	if err != nil {
		t.Fatal(err)
	}
	conn := &partialNetReliabilityConn{clock: clock, advance: 3 * time.Millisecond, chunk: 2}
	exchange := &netReliabilityExchange{
		address:   "127.0.0.1:1",
		transport: "tcp",
		clock:     clock,
		dial: func(context.Context, string, string) (net.Conn, error) {
			return conn, nil
		},
	}
	result := exchange.Exchange(context.Background(), reliabilityRequest{FrameBytes: len(payload), Payload: payload}, makeReliabilityDeadlines(0, 5*time.Millisecond, time.Millisecond))
	if result.FrameBytes != len(payload)+2 || result.BytesWritten != len(payload)+2 || !result.DNSSent || !result.WriteDeadlineRace || !result.HasWriteStart || !result.HasWriteComplete {
		t.Fatalf("TCP must account for and preserve the complete raced frame: %+v", result)
	}
}

func TestReliabilityRunnerDoesNotUseResponseCompletionToAdvancePlanner(t *testing.T) {
	clock := newFakeReliabilityClock()
	var started []int64
	transport := reliabilityExchangeFunc(func(ctx context.Context, request reliabilityRequest, deadlines reliabilityDeadlines) reliabilityExchangeResult {
		started = append(started, request.SlotID)
		clock.Advance(20 * time.Millisecond)
		return reliabilityExchangeResult{BytesWritten: request.FrameBytes, FrameBytes: request.FrameBytes, ResponseAt: clock.Now(), ResponseOK: true}
	})
	config := reliabilityRunConfig{RunID: "open-loop", Scenario: "w1", Transport: "udp", Slots: 3, TargetQPS: 1000, RequestDeadline: time.Second, LateDrain: time.Millisecond, Limits: reliabilityLimits{Workers: 1, InFlight: 1, DispatchQueue: 1, EvidenceQueue: 8, RecordBytes: 64 * 1024}}
	workload := []workloadCase{{CaseID: "case", Scenario: "w1", Transport: "udp", QName: "case.test.", QType: "A", ExpectedRCode: 0, ExpectedAnswerClass: "A", ExpectedAnswer: "192.0.2.1", RequestDeadlineMS: 1000, Weight: 1}}
	result, err := runReliability(context.Background(), config, workload, transport, clock, newMemoryReliabilitySink())
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(started, []int64{0}) {
		t.Fatalf("in-flight saturation should reject later open-loop slots, started=%v", started)
	}
	if result.Accounting.HarnessRejected+result.Accounting.Started != result.Accounting.Planned {
		t.Fatalf("planner accounting is not conserved: %+v", result.Accounting)
	}
}

func TestReliabilityRunnerRecordsInFlightRejectionAndSchedulerPause(t *testing.T) {
	clock := newFakeReliabilityClock()
	started := make(chan struct{})
	release := make(chan struct{})
	transport := reliabilityExchangeFunc(func(ctx context.Context, request reliabilityRequest, deadlines reliabilityDeadlines) reliabilityExchangeResult {
		if request.SlotID == 0 {
			close(started)
			select {
			case <-release:
			case <-ctx.Done():
			}
		}
		return reliabilityExchangeResult{BytesWritten: request.FrameBytes, FrameBytes: request.FrameBytes, ResponseAt: clock.Now(), ResponseOK: true}
	})
	config := reliabilityRunConfig{RunID: "bounded", Scenario: "w1", Transport: "udp", Slots: 3, TargetQPS: 1000, RequestDeadline: time.Second, LateDrain: time.Millisecond, CleanupTimeout: time.Second, Limits: reliabilityLimits{Workers: 1, InFlight: 1, DispatchQueue: 1, EvidenceQueue: 8, RecordBytes: 64 * 1024}}
	workload := []workloadCase{{CaseID: "case", Scenario: "w1", Transport: "udp", QName: "case.test.", QType: "A", ExpectedRCode: 0, ExpectedAnswerClass: "A", ExpectedAnswer: "192.0.2.1", RequestDeadlineMS: 1000, Weight: 1}}
	resultCh := make(chan reliabilityRunResult, 1)
	errCh := make(chan error, 1)
	go func() {
		result, err := runReliability(context.Background(), config, workload, transport, clock, newMemoryReliabilitySink())
		resultCh <- result
		errCh <- err
	}()
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatal("first request did not start")
	}
	close(release)
	select {
	case err := <-errCh:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(time.Second):
		t.Fatal("bounded run did not finish")
	}
	result := <-resultCh
	if result.Accounting.HarnessRejected == 0 {
		t.Fatalf("in-flight saturation must be explicit: %+v", result.Accounting)
	}

	clock = newFakeReliabilityClock()
	clock.Advance(2 * time.Second)
	lagging := reliabilityExchangeFunc(func(context.Context, reliabilityRequest, reliabilityDeadlines) reliabilityExchangeResult {
		return reliabilityExchangeResult{BytesWritten: 1, FrameBytes: 1, ResponseAt: clock.Now(), ResponseOK: true}
	})
	config.RunID = "lagging"
	config.Slots = 2
	config.TargetQPS = 100
	config.RequestDeadline = 10 * time.Millisecond
	result, err := runReliability(context.Background(), config, workload, lagging, clock, newMemoryReliabilitySink())
	if err != nil {
		t.Fatal(err)
	}
	if result.Accounting.HarnessSkipped == 0 {
		t.Fatalf("scheduler pause must produce explicit skipped slots: %+v", result.Accounting)
	}
}

func TestReliabilityWriterErrorInvalidatesEvidenceWithoutLosingControl(t *testing.T) {
	clock := newFakeReliabilityClock()
	config := reliabilityRunConfig{RunID: "writer-error", Scenario: "w1", Transport: "udp", Slots: 1, TargetQPS: 1, RequestDeadline: time.Second, LateDrain: time.Millisecond, CleanupTimeout: time.Second, Limits: reliabilityLimits{Workers: 1, InFlight: 1, DispatchQueue: 1, EvidenceQueue: 1, RecordBytes: 64 * 1024}}
	workload := []workloadCase{{CaseID: "case", Scenario: "w1", Transport: "udp", QName: "case.test.", QType: "A", ExpectedRCode: 0, ExpectedAnswerClass: "A", ExpectedAnswer: "192.0.2.1", RequestDeadlineMS: 1000, Weight: 1}}
	exchange := reliabilityExchangeFunc(func(context.Context, reliabilityRequest, reliabilityDeadlines) reliabilityExchangeResult {
		return reliabilityExchangeResult{BytesWritten: 1, FrameBytes: 1, ResponseAt: clock.Now(), ResponseOK: true}
	})
	result, err := runReliability(context.Background(), config, workload, exchange, clock, failingReliabilitySink{})
	if err != nil {
		t.Fatal(err)
	}
	if result.Raw.EvidenceValid || len(result.Raw.ControlRecords) != 1 || result.Accounting.Planned != 1 {
		t.Fatalf("writer error must invalidate evidence but preserve control accounting: %+v", result.Raw)
	}
}

func TestReliabilityRunStopsDispatchAfterEvidenceFailure(t *testing.T) {
	clock := newRealReliabilityClock()
	config := reliabilityRunConfig{RunID: "writer-stop", Scenario: "w1", Transport: "udp", Slots: 3, TargetQPS: 1, RequestDeadline: time.Second, LateDrain: time.Millisecond, CleanupTimeout: time.Second, Limits: reliabilityLimits{Workers: 1, InFlight: 1, DispatchQueue: 1, EvidenceQueue: 1, RecordBytes: 64 * 1024}}
	workload := []workloadCase{{CaseID: "case", Scenario: "w1", Transport: "udp", QName: "case.test.", QType: "A", ExpectedRCode: 0, ExpectedAnswerClass: "A", ExpectedAnswer: "192.0.2.1", RequestDeadlineMS: 1000, Weight: 1}}
	calls := 0
	exchange := reliabilityExchangeFunc(func(context.Context, reliabilityRequest, reliabilityDeadlines) reliabilityExchangeResult {
		calls++
		return reliabilityExchangeResult{BytesWritten: 1, FrameBytes: 1, ResponseAt: clock.Now(), ResponseOK: true}
	})
	result, err := runReliability(context.Background(), config, workload, exchange, clock, failingReliabilitySink{})
	if err != nil {
		t.Fatal(err)
	}
	if calls != 1 || result.Accounting.Planned != 3 || result.Accounting.HarnessRejected+result.Accounting.HarnessSkipped != 2 || result.Raw.EvidenceValid || result.Raw.LoadValid {
		t.Fatalf("evidence failure must stop later exchanges and close remaining slots: calls=%d raw=%+v", calls, result.Raw)
	}
}

func TestReliabilityRunEnforcesRecordAndControlBudgets(t *testing.T) {
	clock := newFakeReliabilityClock()
	config := reliabilityRunConfig{RunID: "record-budget", Scenario: "w1", Transport: "udp", Slots: 1, TargetQPS: 1, RequestDeadline: time.Second, LateDrain: time.Millisecond, CleanupTimeout: time.Second, Limits: reliabilityLimits{Workers: 1, InFlight: 1, DispatchQueue: 1, EvidenceQueue: 1, RecordBytes: 256}}
	longName := "a."
	for len(longName) < 220 {
		longName += "label."
	}
	workload := []workloadCase{{CaseID: strings.Repeat("case", 40), Scenario: "w1", Transport: "udp", QName: longName, QType: "A", ExpectedRCode: 0, ExpectedAnswerClass: "A", ExpectedAnswer: "192.0.2.1", RequestDeadlineMS: 1000, Weight: 1}}
	exchange := reliabilityExchangeFunc(func(context.Context, reliabilityRequest, reliabilityDeadlines) reliabilityExchangeResult {
		return reliabilityExchangeResult{BytesWritten: 1, FrameBytes: 1, ResponseAt: clock.Now(), ResponseOK: true}
	})
	result, err := runReliability(context.Background(), config, workload, exchange, clock, newMemoryReliabilitySink())
	if err != nil {
		t.Fatal(err)
	}
	if result.Accounting.HarnessRejected != 0 || result.Accounting.Started != 1 || !result.Raw.ControlRecords[0].DNSSent || result.Raw.ControlRecords[0].Terminal == reliabilityTerminalHarnessRejected || result.Raw.EvidenceValid || result.Raw.LoadValid {
		t.Fatalf("record-size overflow must preserve the executed terminal while invalidating evidence: %+v", result.Raw)
	}
	if err := validateReliabilityRunConfig(reliabilityRunConfig{RunID: "too-large", Slots: 1025, TargetQPS: 1, RequestDeadline: time.Second, CleanupTimeout: time.Second, Limits: reliabilityLimits{Workers: 1, InFlight: 1, DispatchQueue: 1, EvidenceQueue: 1, RecordBytes: 64 * 1024}}); err == nil || !strings.Contains(err.Error(), "control budget") {
		t.Fatalf("unbounded control budget must be rejected: %v", err)
	}
}

func TestReliabilityRecoveryStateRequiresSameProcessAndHealthyReturnWindows(t *testing.T) {
	windowIDs := []string{"reference", "overload-1", "overload-2", "recovery-1", "recovery-2", "recovery-3"}
	terminals := []reliabilityTerminal{reliabilityTerminalCorrectOnTime, reliabilityTerminalCorrectLate, reliabilityTerminalCorrectLate, reliabilityTerminalCorrectOnTime, reliabilityTerminalCorrectOnTime, reliabilityTerminalCorrectOnTime}
	finish := []int64{1000, 300000, 400000, 301000, 401000, 501000}
	records := make([]reliabilitySlotRecord, len(windowIDs))
	evidence := make([]reliabilitySlotRecord, len(windowIDs))
	windows := make([]reliabilityWindow, len(windowIDs))
	for i, windowID := range windowIDs {
		planned := int64(i) * 100000
		records[i] = reliabilitySlotRecord{SlotID: int64(i), WindowID: windowID, PlannedOffsetUS: planned, FinishOffsetUS: ptrInt64(finish[i]), DNSSent: true, Terminal: terminals[i]}
		evidence[i] = records[i]
		phase := "reference"
		offeredQPS := 1.0
		if i >= 1 && i <= 2 {
			phase = "overload"
			offeredQPS = 10
		} else if i >= 3 {
			phase = "recovery"
			offeredQPS = 1
		}
		windows[i] = reliabilityWindow{WindowID: windowID, SlotIDs: []int64{int64(i)}, StartOffsetUS: planned, EndOffsetUS: planned + 100000, Phase: phase, OfferedQPS: offeredQPS, SUTPID: 7, SUTStartIdentity: "start-1", ResourceWithinBudget: ptrBool(true)}
	}
	raw := reliabilityRawBundle{
		SchemaVersion:   reliabilitySchemaVersion,
		RunID:           "recovery",
		Config:          reliabilityRunConfig{RequestDeadline: 100 * time.Millisecond, TargetQPS: 1},
		ControlRecords:  records,
		EvidenceRecords: evidence,
		Windows:         windows,
	}
	criteria := reliabilityCriteria{ServiceP95CeilingUS: 50000, TimeoutRateCeiling: 0, OnTimeRateFloor: 0.9, ReferenceQPS: 1, OverloadWindows: 2, RecoveryWindows: 3}
	assessment, err := assessReliability(raw, criteria)
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "recovered" || len(assessment.OverloadWindowIDs) < 1 {
		t.Fatalf("expected same-process recovery: %+v", assessment)
	}
	raw.Windows[3].ResourceWithinBudget = ptrBool(false)
	assessment, err = assessReliability(raw, criteria)
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "overload-without-recovery" {
		t.Fatalf("resource pressure must prevent recovery: %+v", assessment)
	}
	raw.Windows[3].ResourceWithinBudget = ptrBool(true)
	raw.Windows[3].SUTStartIdentity = "start-2"
	assessment, err = assessReliability(raw, criteria)
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "non-recovery-restart" {
		t.Fatalf("restart must not count as recovery: %+v", assessment)
	}
}

func TestReliabilityAssessmentRequiresReferencePhaseBeforeOverload(t *testing.T) {
	records := []reliabilitySlotRecord{
		{SlotID: 0, PlannedOffsetUS: 0, FinishOffsetUS: ptrInt64(200000), DNSSent: true, Terminal: reliabilityTerminalCorrectLate},
		{SlotID: 1, PlannedOffsetUS: 100000, FinishOffsetUS: ptrInt64(300000), DNSSent: true, Terminal: reliabilityTerminalCorrectLate},
	}
	raw := reliabilityRawBundle{
		SchemaVersion:   reliabilitySchemaVersion,
		RunID:           "missing-reference",
		Config:          reliabilityRunConfig{RequestDeadline: 100 * time.Millisecond, TargetQPS: 1},
		ControlRecords:  records,
		EvidenceRecords: append([]reliabilitySlotRecord(nil), records...),
		Windows: []reliabilityWindow{
			{WindowID: "overload-1", SlotIDs: []int64{0}, StartOffsetUS: 0, EndOffsetUS: 100000, Phase: "overload", OfferedQPS: 10, SUTPID: 1, SUTStartIdentity: "start-1", ResourceWithinBudget: ptrBool(true)},
			{WindowID: "overload-2", SlotIDs: []int64{1}, StartOffsetUS: 100000, EndOffsetUS: 200000, Phase: "overload", OfferedQPS: 10, SUTPID: 1, SUTStartIdentity: "start-1", ResourceWithinBudget: ptrBool(true)},
		},
	}
	criteria := reliabilityCriteria{ServiceP95CeilingUS: 50000, TimeoutRateCeiling: 0, OnTimeRateFloor: 0.9, ReferenceQPS: 1, OverloadWindows: 2, RecoveryWindows: 3}
	assessment, err := assessReliability(raw, criteria)
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "indeterminate-missing-evidence" || !strings.Contains(assessment.Reason, "begin with a reference phase") {
		t.Fatalf("missing reference phase must fail closed: %+v", assessment)
	}
	raw.Windows[0].Phase = "reference"
	raw.Windows[0].OfferedQPS = 2
	assessment, err = assessReliability(raw, criteria)
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "indeterminate-missing-evidence" || !strings.Contains(assessment.Reason, "reference qps") {
		t.Fatalf("reference phase at the wrong qps must fail closed: %+v", assessment)
	}
}

func TestReliabilityAssessmentDoesNotCountHealthyOverloadAsRecovery(t *testing.T) {
	windowIDs := []string{"reference", "overload-1", "overload-2", "overload-healthy-1", "overload-healthy-2", "overload-healthy-3"}
	records := make([]reliabilitySlotRecord, len(windowIDs))
	windows := make([]reliabilityWindow, len(windowIDs))
	for i, windowID := range windowIDs {
		planned := int64(i) * 100000
		terminal := reliabilityTerminalCorrectOnTime
		finish := planned + 1000
		phase := "reference"
		offeredQPS := 1.0
		if i >= 1 {
			phase = "overload"
			offeredQPS = 10
		}
		if i == 1 || i == 2 {
			terminal = reliabilityTerminalCorrectLate
			finish = planned + 200000
		}
		records[i] = reliabilitySlotRecord{SlotID: int64(i), WindowID: windowID, PlannedOffsetUS: planned, FinishOffsetUS: ptrInt64(finish), DNSSent: true, Terminal: terminal}
		windows[i] = reliabilityWindow{WindowID: windowID, SlotIDs: []int64{int64(i)}, StartOffsetUS: planned, EndOffsetUS: planned + 100000, Phase: phase, OfferedQPS: offeredQPS, SUTPID: 7, SUTStartIdentity: "start-1", ResourceWithinBudget: ptrBool(true)}
	}
	raw := reliabilityRawBundle{
		SchemaVersion:   reliabilitySchemaVersion,
		RunID:           "healthy-overload-not-recovery",
		Config:          reliabilityRunConfig{RequestDeadline: 100 * time.Millisecond, TargetQPS: 1},
		ControlRecords:  records,
		EvidenceRecords: append([]reliabilitySlotRecord(nil), records...),
		Windows:         windows,
	}
	assessment, err := assessReliability(raw, reliabilityCriteria{ServiceP95CeilingUS: 50000, TimeoutRateCeiling: 0, OnTimeRateFloor: 0.9, ReferenceQPS: 1, OverloadWindows: 2, RecoveryWindows: 3})
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "overload-without-recovery" {
		t.Fatalf("healthy overload windows must not count as recovery: %+v", assessment)
	}
}

func TestReliabilityLatencyPercentilesExcludeFailedResponsesButKeepDenominator(t *testing.T) {
	records := []reliabilitySlotRecord{
		{SlotID: 0, PlannedOffsetUS: 0, DispatchOffsetUS: ptrInt64(1), WriteStartOffsetUS: ptrInt64(2), FinishOffsetUS: ptrInt64(10), DNSSent: true, Terminal: reliabilityTerminalCorrectOnTime},
		{SlotID: 1, PlannedOffsetUS: 100, DispatchOffsetUS: ptrInt64(101), FinishOffsetUS: ptrInt64(120), DNSSent: true, Terminal: reliabilityTerminalTimeout},
		{SlotID: 2, PlannedOffsetUS: 200, DispatchOffsetUS: ptrInt64(201), FinishOffsetUS: ptrInt64(230), DNSSent: true, Terminal: reliabilityTerminalTransportError},
		{SlotID: 3, PlannedOffsetUS: 300, FinishOffsetUS: ptrInt64(310), Terminal: reliabilityTerminalFailedBeforeDNSSend},
		{SlotID: 4, PlannedOffsetUS: 400, DispatchOffsetUS: ptrInt64(401), WriteStartOffsetUS: ptrInt64(402), FinishOffsetUS: ptrInt64(550), DNSSent: true, Terminal: reliabilityTerminalCorrectLate},
	}
	views := reliabilityLatencyViews(records)
	for _, view := range views {
		if view.Samples != 2 || view.EligibleSamples != 2 || view.FailureDenominator != 5 || view.IneligibleSamples != 3 {
			t.Fatalf("failed outcomes must stay in the denominator without entering percentiles: %+v", views)
		}
	}
}

func TestReliabilityAssessmentIgnoresForgedDerivedWindowSummary(t *testing.T) {
	records := []reliabilitySlotRecord{
		{SlotID: 0, WindowID: "reference", PlannedOffsetUS: 0, FinishOffsetUS: ptrInt64(1000), DNSSent: true, Terminal: reliabilityTerminalCorrectOnTime},
		{SlotID: 1, WindowID: "overload-2", PlannedOffsetUS: 100000, FinishOffsetUS: ptrInt64(300000), DNSSent: true, Terminal: reliabilityTerminalCorrectLate},
	}
	raw := reliabilityRawBundle{
		SchemaVersion:   reliabilitySchemaVersion,
		RunID:           "tamper-window",
		Config:          reliabilityRunConfig{RequestDeadline: 100 * time.Millisecond, TargetQPS: 1},
		ControlRecords:  records,
		EvidenceRecords: append([]reliabilitySlotRecord(nil), records...),
		Windows: []reliabilityWindow{
			{WindowID: "reference", SlotIDs: []int64{0}, StartOffsetUS: 0, EndOffsetUS: 100000, Phase: "reference", OfferedQPS: 1, SUTPID: 9, SUTStartIdentity: "start-1", ResourceWithinBudget: ptrBool(true)},
			{WindowID: "overload-2", SlotIDs: []int64{1}, StartOffsetUS: 100000, EndOffsetUS: 200000, Phase: "overload", OfferedQPS: 10, SUTPID: 9, SUTStartIdentity: "start-1", ResourceWithinBudget: ptrBool(true)},
		},
	}
	data, err := json.Marshal(raw)
	if err != nil {
		t.Fatal(err)
	}
	var forged map[string]any
	if err := json.Unmarshal(data, &forged); err != nil {
		t.Fatal(err)
	}
	forged["windows"] = []any{
		map[string]any{"window_id": "reference", "slot_ids": []int64{0}, "start_offset_us": 0, "end_offset_us": 100000, "phase": "reference", "offered_qps": 1, "sut_pid": 9, "sut_start_identity": "start-1", "resource_within_budget": true, "p95_us": 1, "timeout_rate": 0, "evidence_valid": true, "load_valid": true, "correctness_valid": true, "healthy": true},
		map[string]any{"window_id": "overload-2", "slot_ids": []int64{1}, "start_offset_us": 100000, "end_offset_us": 200000, "phase": "overload", "offered_qps": 10, "sut_pid": 9, "sut_start_identity": "start-1", "resource_within_budget": true, "p95_us": 1, "timeout_rate": 0, "evidence_valid": true, "load_valid": true, "correctness_valid": true, "healthy": true},
	}
	data, err = json.Marshal(forged)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		t.Fatal(err)
	}
	assessment, err := assessReliability(raw, reliabilityCriteria{ServiceP95CeilingUS: 50000, TimeoutRateCeiling: 0, OnTimeRateFloor: 0.9, ReferenceQPS: 1, OverloadWindows: 1, RecoveryWindows: 3})
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "overload-without-recovery" || assessment.Verdict != reliabilityVerdictDegraded {
		t.Fatalf("forged derived window summary must not create recovery: %+v", assessment)
	}
}

func TestReliabilityAssessmentRequiresFrozenWindowCriteriaAndResourceFact(t *testing.T) {
	record := reliabilitySlotRecord{SlotID: 0, PlannedOffsetUS: 0, FinishOffsetUS: ptrInt64(1), DNSSent: true, Terminal: reliabilityTerminalCorrectOnTime}
	raw := reliabilityRawBundle{
		SchemaVersion:   reliabilitySchemaVersion,
		RunID:           "criteria",
		Config:          reliabilityRunConfig{RequestDeadline: time.Second, TargetQPS: 1},
		ControlRecords:  []reliabilitySlotRecord{record},
		EvidenceRecords: []reliabilitySlotRecord{record},
		Windows:         []reliabilityWindow{{WindowID: "reference", SlotIDs: []int64{0}, StartOffsetUS: 0, EndOffsetUS: 1000000, Phase: "reference", OfferedQPS: 1, SUTPID: 1, SUTStartIdentity: "start-1", ResourceWithinBudget: ptrBool(true)}},
	}
	assessment, err := assessReliability(raw, reliabilityCriteria{})
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "indeterminate-criteria-not-frozen" {
		t.Fatalf("window thresholds must be explicit: %+v", assessment)
	}
	raw.Windows[0].ResourceWithinBudget = nil
	assessment, err = assessReliability(raw, reliabilityCriteria{ServiceP95CeilingUS: 1000, TimeoutRateCeiling: 0, OnTimeRateFloor: 0.9, ReferenceQPS: 1, OverloadWindows: 2, RecoveryWindows: 3})
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "indeterminate-missing-evidence" || !strings.Contains(assessment.Reason, "resource budget") {
		t.Fatalf("missing resource budget fact must fail closed: %+v", assessment)
	}
}

func TestReliabilityAssessmentRejectsNonFrozenWindowSequence(t *testing.T) {
	records := []reliabilitySlotRecord{
		{SlotID: 0, PlannedOffsetUS: 0, FinishOffsetUS: ptrInt64(1000), DNSSent: true, Terminal: reliabilityTerminalCorrectOnTime},
		{SlotID: 1, PlannedOffsetUS: 100000, FinishOffsetUS: ptrInt64(101000), DNSSent: true, Terminal: reliabilityTerminalCorrectOnTime},
	}
	raw := reliabilityRawBundle{
		SchemaVersion:   reliabilitySchemaVersion,
		RunID:           "window-sequence",
		Config:          reliabilityRunConfig{RequestDeadline: time.Second, TargetQPS: 1},
		ControlRecords:  records,
		EvidenceRecords: append([]reliabilitySlotRecord(nil), records...),
		Windows: []reliabilityWindow{
			{WindowID: "reference", SlotIDs: []int64{0}, StartOffsetUS: 0, EndOffsetUS: 100000, Phase: "reference", OfferedQPS: 1, SUTPID: 1, SUTStartIdentity: "start-1", ResourceWithinBudget: ptrBool(true)},
			{WindowID: "recovery", SlotIDs: []int64{1}, StartOffsetUS: 100000, EndOffsetUS: 200000, Phase: "recovery", OfferedQPS: 2, SUTPID: 1, SUTStartIdentity: "start-1", ResourceWithinBudget: ptrBool(true)},
		},
	}
	criteria := reliabilityCriteria{ServiceP95CeilingUS: 50000, TimeoutRateCeiling: 0, OnTimeRateFloor: 0.9, ReferenceQPS: 1, OverloadWindows: 1, RecoveryWindows: 1}
	assessment, err := assessReliability(raw, criteria)
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "indeterminate-missing-evidence" || !strings.Contains(assessment.Reason, "reference qps") {
		t.Fatalf("recovery at the wrong offered rate must fail closed: %+v", assessment)
	}
	raw.Windows[1].OfferedQPS = 1
	raw.Windows[1].EndOffsetUS = 250000
	assessment, err = assessReliability(raw, criteria)
	if err != nil {
		t.Fatal(err)
	}
	if assessment.RecoveryState != "indeterminate-missing-evidence" || !strings.Contains(assessment.Reason, "equal length") {
		t.Fatalf("unequal windows must fail closed: %+v", assessment)
	}
}

func TestReliabilityAssessmentFailsClosedOnMissingEvidenceAndRejectsOverwrite(t *testing.T) {
	raw := reliabilityRawBundle{
		SchemaVersion:   reliabilitySchemaVersion,
		RunID:           "assess",
		Config:          reliabilityRunConfig{RequestDeadline: 50 * time.Millisecond, LateDrain: 10 * time.Millisecond},
		ControlRecords:  []reliabilitySlotRecord{{SlotID: 0, PlannedOffsetUS: 0, Terminal: reliabilityTerminalCorrectOnTime, DNSSent: true, FinishOffsetUS: ptrInt64(1)}},
		EvidenceRecords: nil,
	}
	assessment, err := assessReliability(raw, reliabilityCriteria{})
	if err != nil {
		t.Fatal(err)
	}
	if assessment.EvidenceValid || assessment.Verdict != reliabilityVerdictIndeterminate {
		t.Fatalf("missing evidence must fail closed: %+v", assessment)
	}
	output := t.TempDir()
	if err := os.WriteFile(filepath.Join(output, "existing.json"), []byte("x"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := ensureFreshReliabilityOutput(output); err == nil {
		t.Fatal("existing non-empty derived output must not be accepted")
	}
}

func TestReliabilitySinkCancellationDoesNotRequireBlockedSinkToResume(t *testing.T) {
	sink := &blockingReliabilitySink{started: make(chan struct{})}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	writer := newReliabilityEvidenceWriter(sink, 1, 10*time.Millisecond)
	writer.Start(ctx)
	if !writer.Enqueue(reliabilitySlotRecord{SlotID: 0}) {
		t.Fatal("first evidence record should enter bounded queue")
	}
	select {
	case <-sink.started:
	case <-time.After(time.Second):
		t.Fatal("writer did not reach blocked sink")
	}
	writer.Close()
	if err := writer.Wait(); err == nil || !strings.Contains(err.Error(), "cleanup_failure") {
		t.Fatalf("blocked sink must surface cleanup_failure: %v", err)
	}
	select {
	case <-sink.release:
		t.Fatal("test sink was unexpectedly unblocked by cancellation")
	default:
	}
}

func TestReliabilityRunBlockedSinkReturnsCleanupFailureWithoutFurtherExchange(t *testing.T) {
	clock := newFakeReliabilityClock()
	sink := &blockingReliabilitySink{started: make(chan struct{}), release: make(chan struct{})}
	config := reliabilityRunConfig{RunID: "blocked-run", Scenario: "w1", Transport: "udp", Slots: 4, TargetQPS: 1000, RequestDeadline: time.Second, LateDrain: time.Millisecond, CleanupTimeout: 10 * time.Millisecond, Limits: reliabilityLimits{Workers: 1, InFlight: 1, DispatchQueue: 1, EvidenceQueue: 1, RecordBytes: 64 * 1024}}
	workload := []workloadCase{{CaseID: "case", Scenario: "w1", Transport: "udp", QName: "case.test.", QType: "A", ExpectedRCode: 0, ExpectedAnswerClass: "A", ExpectedAnswer: "192.0.2.1", RequestDeadlineMS: 1000, Weight: 1}}
	calls := 0
	exchange := reliabilityExchangeFunc(func(_ context.Context, request reliabilityRequest, _ reliabilityDeadlines) reliabilityExchangeResult {
		calls++
		return reliabilityExchangeResult{BytesWritten: request.FrameBytes, FrameBytes: request.FrameBytes, ResponseAt: clock.Now(), ResponseOK: true}
	})
	resultCh := make(chan reliabilityRunResult, 1)
	errCh := make(chan error, 1)
	go func() {
		result, err := runReliability(context.Background(), config, workload, exchange, clock, sink)
		resultCh <- result
		errCh <- err
	}()
	select {
	case <-sink.started:
	case <-time.After(time.Second):
		t.Fatal("blocked sink did not receive the first record")
	}
	select {
	case err := <-errCh:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(time.Second):
		t.Fatal("blocked reliability run did not honor cleanup budget")
	}
	result := <-resultCh
	if calls >= config.Slots || result.Raw.CleanupFailure == "" || result.Raw.EvidenceValid || result.Raw.LoadValid || result.Accounting.Planned != config.Slots || len(result.Raw.MissingSlotIDs) == 0 {
		t.Fatalf("blocked sink must stop later exchanges and preserve an invalid control ledger: calls=%d raw=%+v", calls, result.Raw)
	}
	missing := make(map[int64]struct{}, len(result.Raw.MissingSlotIDs))
	for _, slotID := range result.Raw.MissingSlotIDs {
		missing[slotID] = struct{}{}
	}
	persisted := make(map[int64]struct{}, len(result.Raw.EvidenceRecords))
	for _, record := range result.Raw.EvidenceRecords {
		persisted[record.SlotID] = struct{}{}
	}
	for _, record := range result.Raw.ControlRecords {
		if _, ok := persisted[record.SlotID]; !ok {
			if _, ok := missing[record.SlotID]; !ok {
				t.Fatalf("non-error queue overflow omitted missing slot %d: raw=%+v", record.SlotID, result.Raw)
			}
		}
	}
	close(sink.release)
}

func TestReliabilityLoopbackTransportUsesStrictLoopbackOnly(t *testing.T) {
	if err := validateReliabilityAddress("8.8.8.8:53"); err == nil {
		t.Fatal("reliability runner must reject non-loopback targets")
	}
	if err := validateReliabilityAddress("127.0.0.1:53"); err != nil {
		t.Fatal(err)
	}
	if err := validateReliabilityAddress("[::1]:53"); err != nil {
		t.Fatal(err)
	}
}

type fakeReliabilityClock struct {
	mu   sync.Mutex
	now  time.Duration
	wall time.Time
}

func newFakeReliabilityClock() *fakeReliabilityClock {
	return &fakeReliabilityClock{wall: time.Unix(0, 0).UTC()}
}

func (c *fakeReliabilityClock) Now() time.Duration {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}

func (c *fakeReliabilityClock) WallNow() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.wall
}

func (c *fakeReliabilityClock) SleepUntil(_ context.Context, target time.Duration) error {
	c.mu.Lock()
	if c.now < target {
		c.now = target
	}
	c.mu.Unlock()
	return nil
}

func (c *fakeReliabilityClock) Advance(delta time.Duration) {
	c.mu.Lock()
	c.now += delta
	c.mu.Unlock()
}

func (c *fakeReliabilityClock) AdvanceWall(delta time.Duration) {
	c.mu.Lock()
	c.wall = c.wall.Add(delta)
	c.mu.Unlock()
}

type blockingReliabilitySink struct {
	started chan struct{}
	release chan struct{}
}

type failingReliabilitySink struct{}

func (failingReliabilitySink) Write(context.Context, reliabilitySlotRecord) error {
	return errors.New("synthetic writer failure")
}

type partialReliabilityConn struct {
	clock     *fakeReliabilityClock
	advance   time.Duration
	deadlines int
}

type advancingReliabilityConn struct {
	clock   *fakeReliabilityClock
	advance time.Duration
}

type partialNetReliabilityConn struct {
	clock   *fakeReliabilityClock
	advance time.Duration
	chunk   int
	calls   int
}

func (c *partialNetReliabilityConn) Read([]byte) (int, error) { return 0, os.ErrDeadlineExceeded }

func (c *partialNetReliabilityConn) Write(data []byte) (int, error) {
	c.calls++
	c.clock.Advance(c.advance)
	if c.calls > 1 || len(data) < c.chunk {
		return len(data), nil
	}
	return c.chunk, nil
}

func (c *partialNetReliabilityConn) Close() error                     { return nil }
func (c *partialNetReliabilityConn) LocalAddr() net.Addr              { return reliabilityTestAddr("local") }
func (c *partialNetReliabilityConn) RemoteAddr() net.Addr             { return reliabilityTestAddr("remote") }
func (c *partialNetReliabilityConn) SetDeadline(time.Time) error      { return nil }
func (c *partialNetReliabilityConn) SetReadDeadline(time.Time) error  { return nil }
func (c *partialNetReliabilityConn) SetWriteDeadline(time.Time) error { return nil }

type reliabilityTestAddr string

func (a reliabilityTestAddr) Network() string { return "test" }
func (a reliabilityTestAddr) String() string  { return string(a) }

func (c *advancingReliabilityConn) SetWriteDeadline(time.Time) error { return nil }

func (c *advancingReliabilityConn) Write(data []byte) (int, error) {
	c.clock.Advance(c.advance)
	return len(data), nil
}

func (c *partialReliabilityConn) SetWriteDeadline(time.Time) error {
	c.deadlines++
	return nil
}

func (c *partialReliabilityConn) Write(data []byte) (int, error) {
	if len(data) == 0 {
		return 0, nil
	}
	c.clock.Advance(c.advance)
	return 2, nil
}

func (s *blockingReliabilitySink) Write(context.Context, reliabilitySlotRecord) error {
	select {
	case <-s.started:
	default:
		close(s.started)
	}
	<-s.release
	return nil
}
