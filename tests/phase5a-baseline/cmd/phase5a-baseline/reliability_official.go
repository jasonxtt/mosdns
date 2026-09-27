package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

// reliabilityStageOptions is the narrow adapter between the new reliability
// ledger and the historical stage/counter consumers that still validate
// fixture, TTL, routing, and paired-run evidence. The raw reliability bundle
// remains authoritative; these files are compatibility projections only.
type reliabilityStageOptions struct {
	stageName        string
	runID            string
	stageDurationMS  int64
	fixtureSessionID string
	rawResultDir     string
	stageOutputDir   string
	sutPID           int
	fixtureTargets   []resourceTarget
	requestLedger    string
	eventJournal     string
	fixtureSeqStart  uint64
	fixtureSeqEnd    uint64
	sutStartIdentity string
	sutCPUSet        string
	harnessPID       int
	harnessCPUSet    string
	harnessHost      string
	harnessGoProfile map[string]string
	clockTicks       int64
}

func prepareReliabilityStageOptions(stageName string, stageDurationMS int64, fixtureSessionID string, sutPID int, requestLedgerPath, eventJournalPath string, fixturePIDArgs []string, rawResultDir, stageOutputDir string) (*reliabilityStageOptions, error) {
	if stageName == "" {
		if stageDurationMS != 0 || fixtureSessionID != "" || sutPID != 0 || requestLedgerPath != "" || eventJournalPath != "" || len(fixturePIDArgs) != 0 || stageOutputDir != "" {
			return nil, errors.New("official stage metadata requires --stage")
		}
		return nil, nil
	}
	if stageDurationMS <= 0 || sutPID <= 0 {
		return nil, errors.New("official reliability stage requires --stage-duration-ms and a positive --sut-pid")
	}
	if stageOutputDir == "" {
		stageOutputDir = rawResultDir
	}
	if requestLedgerPath == "" {
		requestLedgerPath = filepath.Join(stageOutputDir, "requests.jsonl")
	}
	if fixtureSessionID == "" {
		fixtureSessionID = "reliability-fixture-session"
	}
	targets := make([]resourceTarget, 0, len(fixturePIDArgs))
	seen := make(map[int]struct{}, len(fixturePIDArgs))
	for index, value := range fixturePIDArgs {
		pid, err := strconv.Atoi(value)
		if err != nil || pid <= 0 {
			return nil, fmt.Errorf("invalid fixture PID %q", value)
		}
		if _, exists := seen[pid]; exists {
			return nil, fmt.Errorf("duplicate fixture PID %d", pid)
		}
		seen[pid] = struct{}{}
		targets = append(targets, resourceTarget{Role: fmt.Sprintf("fixture-%d", index+1), PID: pid})
	}
	return &reliabilityStageOptions{
		stageName:        stageName,
		stageDurationMS:  stageDurationMS,
		fixtureSessionID: fixtureSessionID,
		rawResultDir:     rawResultDir,
		stageOutputDir:   stageOutputDir,
		sutPID:           sutPID,
		fixtureTargets:   targets,
		requestLedger:    requestLedgerPath,
		eventJournal:     eventJournalPath,
		harnessPID:       os.Getpid(),
		harnessGoProfile: map[string]string{
			"GOMAXPROCS": os.Getenv("GOMAXPROCS"),
			"GOGC":       os.Getenv("GOGC"),
			"GODEBUG":    os.Getenv("GODEBUG"),
			"GOMEMLIMIT": os.Getenv("GOMEMLIMIT"),
		},
	}, nil
}

func (o *reliabilityStageOptions) start(stop *chan struct{}, wg *sync.WaitGroup, counts *map[string]int) error {
	if runtime.GOOS != "linux" {
		return errors.New("official reliability stage requires Linux process sampling")
	}
	var err error
	o.sutStartIdentity, err = processStartIdentity(o.sutPID)
	if err != nil {
		return fmt.Errorf("capture SUT process start identity: %w", err)
	}
	o.sutCPUSet, err = processCPUSet(o.sutPID)
	if err != nil {
		return fmt.Errorf("capture SUT CPU affinity: %w", err)
	}
	o.harnessCPUSet = processCPUSetOrEmpty(o.harnessPID)
	if o.harnessCPUSet == "" {
		return errors.New("could not capture helper process CPU affinity")
	}
	o.harnessHost, err = os.Hostname()
	if err != nil || o.harnessHost == "" {
		return errors.New("could not capture helper host identity")
	}
	o.clockTicks, err = resourceClockTicksPerSecond()
	if err != nil {
		return fmt.Errorf("resolve resource clock before stage: %w", err)
	}
	if o.eventJournal != "" {
		o.fixtureSeqStart, err = newFixtureEventJournal(o.eventJournal).lastSequence()
		if err != nil {
			return fmt.Errorf("read fixture event barrier before stage: %w", err)
		}
	}
	targets := []resourceTarget{{Role: "sut", PID: o.sutPID}, {Role: "load-generator", PID: o.harnessPID}}
	targets = append(targets, o.fixtureTargets...)
	*stop = make(chan struct{})
	wg.Add(1)
	go func() {
		defer wg.Done()
		*counts = sampleProcessGroup(targets, o.runID, o.stageName, filepath.Join(o.rawResultDir, "resource-samples.jsonl"), *stop, o.clockTicks)
	}()
	return nil
}

func (o *reliabilityStageOptions) finish() error {
	if o.eventJournal == "" {
		return nil
	}
	var err error
	o.fixtureSeqEnd, err = newFixtureEventJournal(o.eventJournal).lastSequence()
	if err != nil {
		return fmt.Errorf("read fixture event barrier after stage: %w", err)
	}
	return nil
}

func (o *reliabilityStageOptions) writeArtifacts(raw reliabilityRawBundle, workload []workloadCase, resourceCounts map[string]int) error {
	if err := os.MkdirAll(o.stageOutputDir, 0755); err != nil {
		return err
	}
	records := append([]reliabilitySlotRecord(nil), raw.ControlRecords...)
	sort.Slice(records, func(i, j int) bool { return records[i].SlotID < records[j].SlotID })
	caseByID := make(map[string]workloadCase, len(workload))
	for _, c := range workload {
		caseByID[c.CaseID] = c
	}
	requestHistory, lastRequestSeq, err := loadRequestHistory(o.requestLedger, raw.RunID)
	if err != nil {
		return fmt.Errorf("load reliability request history: %w", err)
	}
	_ = requestHistory
	ledger, err := openRequestLedger(o.requestLedger)
	if err != nil {
		return fmt.Errorf("open reliability request ledger: %w", err)
	}
	ledgerClosed := false
	defer func() {
		if !ledgerClosed {
			_ = ledger.close()
		}
	}()
	caseScheduled := make(map[string]int64)
	converted := make([]requestRecord, 0, len(records))
	counters := stageCounters{}
	latencies := make([]int64, 0, len(records))
	for index, record := range records {
		requestSeq := lastRequestSeq + uint64(index) + 1
		if record.CaseID != "" {
			caseScheduled[record.CaseID]++
		}
		counters.Scheduled++
		if record.DNSSent {
			counters.Sent++
		} else {
			counters.SenderShortfall++
		}
		outcome := reliabilityRequestOutcome(record.Terminal)
		sentAt := raw.StartedAt.Add(time.Duration(record.PlannedOffsetUS) * time.Microsecond)
		if record.WriteCompleteOffsetUS != nil {
			sentAt = raw.StartedAt.Add(time.Duration(*record.WriteCompleteOffsetUS) * time.Microsecond)
		}
		finishedAt := sentAt
		if record.FinishOffsetUS != nil {
			finishedAt = raw.StartedAt.Add(time.Duration(*record.FinishOffsetUS) * time.Microsecond)
		}
		if finishedAt.Before(sentAt) {
			finishedAt = sentAt
		}
		request := requestRecord{
			RunID: raw.RunID, StageID: o.stageName, RequestSeq: requestSeq,
			DNSID: uint16((record.SlotID % 65535) + 1), CaseID: record.CaseID,
			QName: dnsFqdnOrEmpty(record.QName), QType: strings.ToUpper(record.QType), QClass: 1,
			Sent: record.DNSSent, SentAt: sentAt.UTC(), FinishedAt: finishedAt.UTC(), Outcome: outcome,
		}
		if err := ledger.write(request); err != nil {
			return fmt.Errorf("write reliability request ledger: %w", err)
		}
		converted = append(converted, request)
		switch record.Terminal {
		case reliabilityTerminalCorrectOnTime:
			counters.CorrectOnTime++
			counters.Received++
			if c, ok := caseByID[record.CaseID]; ok && c.ExpectedRCode != 0 {
				counters.ExpectedNegativeOnTime++
			}
		case reliabilityTerminalCorrectLate:
			counters.CorrectLate++
			counters.Received++
		case reliabilityTerminalWrongResponse:
			counters.WrongResponse++
			counters.Received++
		case reliabilityTerminalProtocolError:
			counters.ProtocolError++
			counters.Received++
		case reliabilityTerminalTransportError:
			counters.TransportError++
		case reliabilityTerminalTimeout:
			counters.Timeout++
		case reliabilityTerminalFailedBeforeDNSSend:
			counters.TransportError++
		case reliabilityTerminalHarnessSkipped, reliabilityTerminalHarnessRejected:
			// SenderShortfall already accounts for this harness failure.
		default:
			return fmt.Errorf("unknown reliability terminal %q", record.Terminal)
		}
		if isReliabilitySuccessfulResponse(record.Terminal) && record.FinishOffsetUS != nil {
			if value := *record.FinishOffsetUS - record.PlannedOffsetUS; value >= 0 {
				latencies = append(latencies, value)
			}
		}
	}
	if err := ledger.close(); err != nil {
		return err
	}
	ledgerClosed = true
	sort.Slice(latencies, func(i, j int) bool { return latencies[i] < latencies[j] })
	durationMS := o.stageDurationMS
	if durationMS <= 0 {
		durationMS = raw.FinishedAt.Sub(raw.StartedAt).Milliseconds()
	}
	result := stageResult{
		Stage: o.stageName, RunID: raw.RunID, FixtureSessionID: o.fixtureSessionID,
		Scenario: raw.Scenario, Transport: raw.Transport, TargetQPS: raw.Config.TargetQPS,
		DurationMS: durationMS, RequestDeadlineMS: int(raw.Config.RequestDeadline / time.Millisecond),
		LateDrainMS: int(raw.Config.LateDrain / time.Millisecond), Counters: counters,
		CaseScheduled: caseScheduled, LatencySamplesUS: latencies,
		P50US: percentile(latencies, .50), P95US: percentile(latencies, .95), P99US: percentile(latencies, .99),
		EffectiveThroughput: effectiveReliabilityThroughput(counters.CorrectOnTime, durationMS),
		SenderLagMaxUS:      reliabilitySenderLag(records, raw.Config), StartedAt: raw.StartedAt, FinishedAt: raw.FinishedAt,
		ResourceSampleCount: resourceCounts["sut"], ResourceSampleCounts: resourceCounts,
		SUTPID: o.sutPID, SUTStartIdentity: o.sutStartIdentity, SUTCPUSet: o.sutCPUSet,
		HarnessPID: o.harnessPID, HarnessCPUSet: o.harnessCPUSet, HarnessHost: o.harnessHost,
		HarnessGoProfile: o.harnessGoProfile, RequestLedgerPath: o.requestLedger, EventJournalPath: o.eventJournal,
		FixtureSeqStart: o.fixtureSeqStart, FixtureSeqEnd: o.fixtureSeqEnd,
	}
	if len(converted) > 0 {
		result.RequestSeqStart = converted[0].RequestSeq
		result.RequestSeqEnd = converted[len(converted)-1].RequestSeq
	}
	stageFile, err := os.OpenFile(filepath.Join(o.stageOutputDir, "stages.jsonl"), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return err
	}
	encodeErr := json.NewEncoder(stageFile).Encode(result)
	closeErr := stageFile.Close()
	if encodeErr != nil {
		return encodeErr
	}
	return closeErr
}

func reliabilityRequestOutcome(terminal reliabilityTerminal) string {
	switch terminal {
	case reliabilityTerminalCorrectOnTime:
		return "correct_on_time"
	case reliabilityTerminalCorrectLate:
		return "correct_late"
	case reliabilityTerminalTimeout:
		return "timeout"
	case reliabilityTerminalTransportError, reliabilityTerminalFailedBeforeDNSSend:
		return "transport_error"
	case reliabilityTerminalWrongResponse:
		return "wrong_response"
	case reliabilityTerminalProtocolError:
		return "protocol_error"
	case reliabilityTerminalHarnessSkipped, reliabilityTerminalHarnessRejected:
		return "harness_rejected"
	default:
		return "protocol_error"
	}
}

func dnsFqdnOrEmpty(value string) string {
	if value == "" {
		return ""
	}
	return strings.TrimSuffix(value, ".") + "."
}

func effectiveReliabilityThroughput(correct int64, durationMS int64) float64 {
	if durationMS <= 0 {
		return 0
	}
	return float64(correct) / (float64(durationMS) / 1000)
}

func reliabilitySenderLag(records []reliabilitySlotRecord, config reliabilityRunConfig) int64 {
	if config.TargetQPS <= 0 {
		return 0
	}
	intervalUS := time.Duration(float64(time.Second) / config.TargetQPS).Microseconds()
	if intervalUS <= 0 {
		intervalUS = 1
	}
	var max int64
	for _, record := range records {
		if record.DispatchOffsetUS == nil {
			continue
		}
		lag := *record.DispatchOffsetUS - record.PlannedOffsetUS
		if lag > max {
			max = lag
		}
	}
	return max
}

func readResourceSamples(path string) ([]resourceSample, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	var samples []resourceSample
	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 4096), 1024*1024)
	for scanner.Scan() {
		if strings.TrimSpace(scanner.Text()) == "" {
			continue
		}
		var sample resourceSample
		if err := json.Unmarshal(scanner.Bytes(), &sample); err != nil {
			return nil, err
		}
		samples = append(samples, sample)
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	return samples, nil
}

func verifyResourceBudgetsCommand(args []string) error {
	fs := flag.NewFlagSet("verify-resource-budgets", flag.ContinueOnError)
	samplesPath := fs.String("samples", "", "resource-samples.jsonl")
	maxRoleRSSKiB := fs.Int64("max-role-rss-kib", 0, "per-role RSS ceiling")
	maxCombinedRSSKiB := fs.Int64("max-combined-harness-fixture-rss-kib", 0, "combined load-generator plus fixture RSS ceiling")
	maxFDs := fs.Int("max-fds", 0, "per-process FD ceiling")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *samplesPath == "" || *maxRoleRSSKiB <= 0 || *maxCombinedRSSKiB <= 0 || *maxFDs <= 0 {
		return errors.New("verify-resource-budgets requires samples and positive RSS/FD ceilings")
	}
	samples, err := readResourceSamples(*samplesPath)
	if err != nil {
		return fmt.Errorf("read resource samples: %w", err)
	}
	if len(samples) == 0 {
		return errors.New("resource sample file is empty")
	}
	maxByRole := make(map[string]int64)
	maxCombined := int64(0)
	combinedByRole := make(map[string]int64)
	for _, sample := range samples {
		if sample.RSSKiB > *maxRoleRSSKiB {
			return fmt.Errorf("resource RSS ceiling exceeded: role=%s rss_kib=%d ceiling=%d", sample.Role, sample.RSSKiB, *maxRoleRSSKiB)
		}
		if sample.FDCount > *maxFDs {
			return fmt.Errorf("resource FD ceiling exceeded: role=%s fds=%d ceiling=%d", sample.Role, sample.FDCount, *maxFDs)
		}
		if sample.RSSKiB > maxByRole[sample.Role] {
			maxByRole[sample.Role] = sample.RSSKiB
		}
		if sample.Role == "load-generator" || strings.HasPrefix(sample.Role, "fixture-") {
			if sample.RSSKiB > combinedByRole[sample.Role] {
				combinedByRole[sample.Role] = sample.RSSKiB
			}
		}
	}
	for _, value := range combinedByRole {
		maxCombined += value
	}
	if maxCombined > *maxCombinedRSSKiB {
		return fmt.Errorf("combined harness/fixture RSS ceiling exceeded: rss_kib=%d ceiling=%d", maxCombined, *maxCombinedRSSKiB)
	}
	fmt.Fprintf(os.Stdout, "resource budgets verified: samples=%d max_by_role=%v combined_harness_fixture_rss_kib=%d\n", len(samples), maxByRole, maxCombined)
	return nil
}
