package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

func runReliabilityCommand(args []string) error {
	fs := flag.NewFlagSet("reliability-run", flag.ContinueOnError)
	workloadPath := fs.String("workload", "", "fixed workload JSONL")
	scenario := fs.String("scenario", "", "fixed workload scenario")
	transport := fs.String("transport", "udp", "loopback transport: udp or tcp")
	address := fs.String("addr", "127.0.0.1:15353", "loopback DNS address")
	resultDir := fs.String("result", "", "fresh result directory")
	runID := fs.String("run-id", "", "run identity")
	slots := fs.Int("slots", 1, "planned open-loop slots")
	targetQPS := fs.Float64("target-qps", 1, "planned open-loop rate")
	deadlineMS := fs.Int("request-deadline-ms", 500, "absolute service deadline")
	lateDrainMS := fs.Int("late-drain-ms", 100, "post-service collection window")
	workers := fs.Int("workers", reliabilityDefaultWorkers, "fixed worker count")
	inFlight := fs.Int("in-flight", reliabilityDefaultInFlight, "maximum in-flight requests")
	dispatchQueue := fs.Int("dispatch-queue", reliabilityDefaultQueue, "bounded dispatch queue")
	evidenceQueue := fs.Int("evidence-queue", reliabilityDefaultQueue, "bounded evidence queue")
	recordBytes := fs.Int("record-bytes", reliabilityDefaultRecordMax, "maximum raw record size")
	cleanupMS := fs.Int("cleanup-timeout-ms", 1000, "bounded evidence cleanup timeout")
	stageName := fs.String("stage", "", "official stage name; enables stage compatibility artifacts")
	stageDurationMS := fs.Int64("stage-duration-ms", 0, "frozen stage duration for official stage metadata")
	stageOutputDir := fs.String("stage-output", "", "compatibility stage-output directory; defaults to --result")
	fixtureSessionID := fs.String("fixture-session-id", "", "shared fixture session identity")
	sutPID := fs.Int("sut-pid", 0, "SUT PID for official resource sampling")
	requestLedgerPath := fs.String("request-ledger", "", "official per-request ledger path")
	eventJournalPath := fs.String("event-journal", "", "official fixture event journal path")
	var fixturePIDArgs stringSliceFlag
	fs.Var(&fixturePIDArgs, "fixture-pid", "fixture PID to sample; may be repeated")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *workloadPath == "" || *scenario == "" || *resultDir == "" {
		return errors.New("--workload, --scenario, and --result are required")
	}
	if *transport != "udp" && *transport != "tcp" {
		return fmt.Errorf("unsupported reliability transport %q", *transport)
	}
	if err := validateReliabilityAddress(*address); err != nil {
		return err
	}
	if err := ensureFreshReliabilityOutput(*resultDir); err != nil {
		return err
	}
	stageOptions, err := prepareReliabilityStageOptions(*stageName, *stageDurationMS, *fixtureSessionID, *sutPID, *requestLedgerPath, *eventJournalPath, fixturePIDArgs, *resultDir, *stageOutputDir)
	if err != nil {
		return err
	}
	workload, err := readWorkload(*workloadPath, *scenario, *transport)
	if err != nil {
		return err
	}
	if *runID == "" {
		*runID = fmt.Sprintf("reliability-%s-%d", *scenario, time.Now().UnixNano())
	}
	if stageOptions != nil {
		stageOptions.runID = *runID
		if *fixtureSessionID == "" {
			stageOptions.fixtureSessionID = *runID
		}
	}
	config := reliabilityRunConfig{
		RunID:           *runID,
		Scenario:        *scenario,
		Transport:       *transport,
		Address:         *address,
		Slots:           *slots,
		TargetQPS:       *targetQPS,
		RequestDeadline: time.Duration(*deadlineMS) * time.Millisecond,
		LateDrain:       time.Duration(*lateDrainMS) * time.Millisecond,
		CleanupTimeout:  time.Duration(*cleanupMS) * time.Millisecond,
		Limits: reliabilityLimits{
			Workers:       *workers,
			InFlight:      *inFlight,
			DispatchQueue: *dispatchQueue,
			EvidenceQueue: *evidenceQueue,
			RecordBytes:   *recordBytes,
		},
	}
	evidencePath := filepath.Join(*resultDir, "reliability-evidence.jsonl")
	sink, err := newJSONLReliabilitySink(evidencePath)
	if err != nil {
		return err
	}
	clock := newRealReliabilityClock()
	var samplerStop chan struct{}
	var samplerWG sync.WaitGroup
	resourceCounts := map[string]int{}
	if stageOptions != nil {
		if err := stageOptions.start(&samplerStop, &samplerWG, &resourceCounts); err != nil {
			_ = sink.Close()
			return err
		}
	}
	result, runErr := runReliability(context.Background(), config, workload, newNetReliabilityExchange(*address, *transport, clock), clock, sink)
	if samplerStop != nil {
		close(samplerStop)
		samplerWG.Wait()
	}
	if stageOptions != nil {
		if err := stageOptions.finish(); err != nil {
			return err
		}
	}
	closeErr := sink.Close()
	if runErr != nil {
		return runErr
	}
	if closeErr != nil {
		result.Raw.CleanupFailure = closeErr.Error()
		result.Raw.EvidenceValid = false
		result.Raw.LoadValid = false
	}
	data, err := json.MarshalIndent(result.Raw, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	if err := os.WriteFile(filepath.Join(*resultDir, "reliability-raw.json"), data, 0644); err != nil {
		return err
	}
	if stageOptions != nil {
		if err := stageOptions.writeArtifacts(result.Raw, workload, resourceCounts); err != nil {
			return err
		}
	}
	if closeErr != nil {
		return closeErr
	}
	fmt.Printf("%s\n", filepath.Join(*resultDir, "reliability-raw.json"))
	return nil
}

func assessReliabilityCommand(args []string) error {
	fs := flag.NewFlagSet("reliability-assess", flag.ContinueOnError)
	rawPath := fs.String("raw", "", "raw reliability bundle")
	outputDir := fs.String("output", "", "fresh derived output directory")
	maxP95US := fs.Int64("max-p95-us", 0, "service p95 ceiling")
	timeoutRate := fs.Float64("timeout-rate", -1, "timeout-rate ceiling")
	onTimeRate := fs.Float64("on-time-rate-floor", -1, "minimum on-time rate")
	referenceQPS := fs.Float64("reference-qps", -1, "frozen reference offered rate")
	overloadWindows := fs.Int("overload-windows", 0, "consecutive violating windows")
	recoveryWindows := fs.Int("recovery-windows", 0, "healthy windows required for recovery")
	repoRoot := fs.String("repo-root", "", "explicit repository root for archive identity")
	sourceCommit := fs.String("source-commit", "", "specified historical Git object")
	sourcePath := fs.String("source-path", "", "repo-relative path in the historical object")
	sourceSHA256 := fs.String("source-sha256", "", "expected SHA-256 of the historical blob")
	requireEvidence := fs.Bool("require-evidence-valid", false, "return an error when raw evidence is incomplete")
	requireLoad := fs.Bool("require-load-valid", false, "return an error when the sender/load contract is invalid")
	requireCorrectness := fs.Bool("require-correctness-valid", false, "return an error when strict DNS correctness fails")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *rawPath == "" || *outputDir == "" {
		return errors.New("--raw and --output are required")
	}
	if err := ensureFreshReliabilityOutput(*outputDir); err != nil {
		return err
	}
	if *repoRoot != "" || *sourceCommit != "" || *sourcePath != "" || *sourceSHA256 != "" {
		if err := validateReliabilityArchive(*repoRoot, *sourceCommit, *sourcePath, *sourceSHA256); err != nil {
			return err
		}
	}
	raw, err := loadReliabilityRaw(*rawPath)
	if err != nil {
		return err
	}
	criteria := reliabilityCriteria{ServiceP95CeilingUS: *maxP95US, TimeoutRateCeiling: *timeoutRate, OnTimeRateFloor: *onTimeRate, ReferenceQPS: *referenceQPS, OverloadWindows: *overloadWindows, RecoveryWindows: *recoveryWindows}
	assessment, err := assessReliability(raw, criteria)
	if err != nil {
		return err
	}
	if err := writeReliabilityAssessment(*outputDir, assessment); err != nil {
		return err
	}
	if *requireEvidence && !assessment.EvidenceValid {
		return errors.New("reliability evidence is invalid")
	}
	if *requireLoad && !assessment.LoadValid {
		return errors.New("reliability load is invalid")
	}
	if *requireCorrectness && !assessment.CorrectnessValid {
		return errors.New("reliability correctness is invalid")
	}
	fmt.Printf("%s\n", filepath.Join(*outputDir, "assessment.json"))
	return nil
}

func validateReliabilityArchive(repoRoot, commit, repoPath, expectedSHA string) error {
	if repoRoot == "" || commit == "" || repoPath == "" || expectedSHA == "" {
		return errors.New("archive identity requires --repo-root, --source-commit, --source-path, and --source-sha256")
	}
	if filepath.IsAbs(repoPath) || strings.Contains(repoPath, "..") {
		return errors.New("source path must be a repo-relative path without parent traversal")
	}
	root, err := filepath.Abs(repoRoot)
	if err != nil {
		return err
	}
	marker, err := exec.Command("git", "-C", root, "rev-parse", "--show-toplevel").Output()
	if err != nil || strings.TrimSpace(string(marker)) != root {
		return fmt.Errorf("repo root is not an explicit Git root: %s", root)
	}
	object := commit + ":" + repoPath
	if err := exec.Command("git", "-C", root, "cat-file", "-e", object).Run(); err != nil {
		return fmt.Errorf("historical Git object is missing: %s", object)
	}
	content, err := exec.Command("git", "-C", root, "show", object).Output()
	if err != nil {
		return err
	}
	hash := sha256.Sum256(content)
	got := hex.EncodeToString(hash[:])
	if !strings.EqualFold(got, expectedSHA) {
		return fmt.Errorf("historical source hash mismatch: got %s want %s", got, expectedSHA)
	}
	return nil
}
