// Command phase5a-baseline contains the deliberately narrow helper used by
// the Phase 5A Go-only baseline. It is not a general DNS server or load
// testing framework; its fixture behavior is fixed by the committed corpus.
package main

import (
	"bufio"
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"math"
	"net"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/miekg/dns"
)

const (
	defaultLateDrain = 100 * time.Millisecond
	defaultDeadline  = 500 * time.Millisecond
)

type workloadCase struct {
	CaseID              string `json:"case_id"`
	Scenario            string `json:"scenario"`
	Transport           string `json:"transport"`
	QName               string `json:"qname"`
	QType               string `json:"qtype"`
	ExpectedRCode       int    `json:"expected_rcode"`
	ExpectedAnswerClass string `json:"expected_answer_class"`
	ExpectedAnswer      string `json:"expected_answer,omitempty"`
	ExpectedRouteClass  string `json:"expected_route_class"`
	RequestDeadlineMS   int    `json:"request_deadline_ms"`
	Weight              int    `json:"weight"`
}

type stageCounters struct {
	Scheduled              int64 `json:"scheduled"`
	Sent                   int64 `json:"sent"`
	Received               int64 `json:"received"`
	CorrectOnTime          int64 `json:"correct_on_time"`
	CorrectLate            int64 `json:"correct_late"`
	ExpectedNegativeOnTime int64 `json:"expected_negative_on_time"`
	WrongResponse          int64 `json:"wrong_response"`
	ProtocolError          int64 `json:"protocol_error"`
	TransportError         int64 `json:"transport_error"`
	Timeout                int64 `json:"timeout"`
	SenderShortfall        int64 `json:"sender_shortfall"`
}

type stageResult struct {
	Stage                string           `json:"stage"`
	RunID                string           `json:"run_id"`
	FixtureSessionID     string           `json:"fixture_session_id"`
	Scenario             string           `json:"scenario"`
	Transport            string           `json:"transport"`
	TargetQPS            float64          `json:"target_qps"`
	DurationMS           int64            `json:"duration_ms"`
	RequestDeadlineMS    int              `json:"request_deadline_ms"`
	LateDrainMS          int              `json:"late_drain_ms"`
	Counters             stageCounters    `json:"counters"`
	CaseScheduled        map[string]int64 `json:"case_scheduled,omitempty"`
	LatencySamplesUS     []int64          `json:"latency_samples_us"`
	P50US                int64            `json:"p50_us"`
	P95US                int64            `json:"p95_us"`
	P99US                int64            `json:"p99_us"`
	EffectiveThroughput  float64          `json:"effective_throughput_qps"`
	SenderLagMaxUS       int64            `json:"sender_lag_max_us"`
	StartedAt            time.Time        `json:"started_at"`
	FinishedAt           time.Time        `json:"finished_at"`
	ResourceSampleCount  int              `json:"resource_sample_count"`
	ResourceSampleCounts map[string]int   `json:"resource_sample_counts"`
	SUTPID               int              `json:"sut_pid"`
	SUTStartIdentity     string           `json:"sut_start_identity"`
	SUTCPUSet            string           `json:"sut_cpu_set"`
	HarnessPID           int              `json:"harness_pid"`
	HarnessCPUSet        string           `json:"harness_cpu_set"`
	RequestLedgerPath    string           `json:"request_ledger_path"`
	EventJournalPath     string           `json:"event_journal_path,omitempty"`
	FixtureSeqStart      uint64           `json:"fixture_seq_start"`
	FixtureSeqEnd        uint64           `json:"fixture_seq_end"`
	RequestSeqStart      uint64           `json:"request_seq_start"`
	RequestSeqEnd        uint64           `json:"request_seq_end"`
}

type resourceSample struct {
	RunID               string    `json:"run_id"`
	StageID             string    `json:"stage_id"`
	Role                string    `json:"role"`
	Timestamp           time.Time `json:"timestamp"`
	PID                 int       `json:"pid"`
	UserTicks           uint64    `json:"user_ticks"`
	SystemTicks         uint64    `json:"system_ticks"`
	ClockTicksPerSecond int64     `json:"clock_ticks_per_second"`
	UserSeconds         float64   `json:"user_seconds"`
	SystemSeconds       float64   `json:"system_seconds"`
	RSSKiB              int64     `json:"rss_kib"`
	FDCount             int       `json:"fd_count"`
}

type resourceTarget struct {
	Role string
	PID  int
}

type runStats struct {
	mu          sync.Mutex
	counters    stageCounters
	latenciesUS []int64
	maxLagUS    int64
	ledgerErr   error
}

func main() {
	if len(os.Args) < 2 {
		usage()
		os.Exit(2)
	}
	var err error
	switch os.Args[1] {
	case "fixture":
		err = runFixture(os.Args[2:])
	case "run":
		err = runStage(os.Args[2:])
	case "verify-counters":
		err = verifyCounters(os.Args[2:])
	case "verify-routing-events":
		err = verifyRoutingEventsCommand(os.Args[2:])
	case "verify-warm-ttl":
		err = verifyWarmTTLCommand(os.Args[2:])
	case "verify-manifest":
		err = verifyManifestCommand(os.Args[2:])
	case "verify-affinity":
		err = verifyAffinityCommand(os.Args[2:])
	case "verify-cpu-sets":
		err = verifyCPUSetsCommand(os.Args[2:])
	case "verify-continuous":
		err = verifyContinuousCommand(os.Args[2:])
	case "verify-stage":
		err = verifyStageCommand(os.Args[2:])
	case "verify-session-counters":
		err = verifySessionCountersCommand(os.Args[2:])
	case "verify-event-journal":
		err = verifyEventJournalCommand(os.Args[2:])
	case "verify-samples":
		err = verifySamplesCommand(os.Args[2:])
	case "verify-sender":
		err = verifySenderCommand(os.Args[2:])
	case "aggregate-pairs":
		err = aggregatePairedStagesCommand(os.Args[2:])
	case "validate-binary":
		err = validateBinary(os.Args[2:])
	case "version":
		fmt.Println(helperVersion)
	default:
		usage()
		err = fmt.Errorf("unknown command %q", os.Args[1])
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func usage() {
	fmt.Fprintln(os.Stderr, "usage: phase5a-baseline-helper {fixture|run|verify-counters|verify-routing-events|verify-warm-ttl|verify-manifest|verify-affinity|verify-cpu-sets|verify-continuous|verify-stage|verify-session-counters|verify-event-journal|verify-samples|verify-sender|aggregate-pairs|validate-binary|version}")
}

const (
	helperVersion          = "phase5a-baseline-helper/v9"
	fixtureEventSchema     = "fixture-event-v3-occurrence-time"
	recoveryAssessmentMode = "indeterminate-no-overload-evidence"
)

var fixedCorpusInputPaths = []string{
	"tests/phase5a-baseline/configs/cache.yaml",
	"tests/phase5a-baseline/configs/forward-tcp.yaml",
	"tests/phase5a-baseline/configs/forward-udp.yaml",
	"tests/phase5a-baseline/configs/routing.yaml",
	"tests/phase5a-baseline/workloads/cache.jsonl",
	"tests/phase5a-baseline/workloads/forward.jsonl",
	"tests/phase5a-baseline/workloads/routing.jsonl",
}

var continuousStageSequence = []string{
	"normal-reference", "common-load", "near-saturation", "overload", "recovery",
}

type manifestInput struct {
	Path   string `json:"path"`
	SHA256 string `json:"sha256"`
}

type manifestArtifact struct {
	Path   string `json:"path"`
	SHA256 string `json:"sha256"`
}

type manifestHelper struct {
	SourcePath   string `json:"source_path"`
	SourceSHA256 string `json:"source_sha256"`
	BinaryPath   string `json:"binary_path"`
	BinarySHA256 string `json:"binary_sha256"`
	Version      string `json:"version"`
}

type manifestCandidate struct {
	SourceCommit string `json:"source_commit"`
	BinaryPath   string `json:"binary_path"`
	BinarySHA256 string `json:"binary_sha256"`
}

type recoveryAssessment struct {
	Status string `json:"status"`
	Mode   string `json:"mode"`
	Reason string `json:"reason"`
}

type manifestPair struct {
	Repetition int      `json:"repetition"`
	Order      []string `json:"order"`
}

type pairedStageObservation struct {
	Scenario           string        `json:"scenario"`
	Stage              string        `json:"stage"`
	Candidate          string        `json:"candidate"`
	RecoveryAssessment string        `json:"recovery_assessment,omitempty"`
	ManifestSHA256     string        `json:"manifest_sha256"`
	Repetition         int           `json:"repetition"`
	PairPosition       int           `json:"pair_position"`
	TargetQPS          float64       `json:"target_qps"`
	DurationMS         int64         `json:"duration_ms"`
	Counters           stageCounters `json:"counters"`
	P50US              int64         `json:"p50_us"`
	P95US              int64         `json:"p95_us"`
	P99US              int64         `json:"p99_us"`
	LatencySampleCount int           `json:"latency_sample_count"`
	InvalidReason      string        `json:"invalid_reason,omitempty"`
}

type pairedMetricRange struct {
	N      int     `json:"n"`
	Min    float64 `json:"min"`
	Median float64 `json:"median"`
	Max    float64 `json:"max"`
}

type pairedCandidateSummary struct {
	P50US               pairedMetricRange `json:"p50_us"`
	P95US               pairedMetricRange `json:"p95_us"`
	P99US               pairedMetricRange `json:"p99_us"`
	EffectiveThroughput pairedMetricRange `json:"effective_throughput_qps"`
	LatencySampleCount  pairedMetricRange `json:"latency_sample_count"`
}

type pairedInvalidRecord struct {
	Repetition int    `json:"repetition"`
	Candidate  string `json:"candidate"`
	Reason     string `json:"reason"`
}

type pairedStageAggregate struct {
	Scenario           string                 `json:"scenario"`
	Stage              string                 `json:"stage"`
	TargetQPS          float64                `json:"target_qps"`
	RecoveryAssessment string                 `json:"recovery_assessment,omitempty"`
	CompletePairs      int                    `json:"complete_pairs"`
	ValidPairs         int                    `json:"valid_pairs"`
	Go                 pairedCandidateSummary `json:"go"`
	Rust               pairedCandidateSummary `json:"rust"`
	RustMinusGoP95US   pairedMetricRange      `json:"rust_minus_go_p95_us"`
	InvalidPairs       []pairedInvalidRecord  `json:"invalid_pairs,omitempty"`
}

type pairedAggregationReport struct {
	ManifestSHA256 string                 `json:"manifest_sha256"`
	Groups         []pairedStageAggregate `json:"groups"`
}

type manifestScenario struct {
	StageDurationMS        int64   `json:"stage_duration_ms"`
	NormalReferenceQPS     float64 `json:"normal_reference_qps"`
	CommonLoadQPS          float64 `json:"common_load_qps"`
	NearSaturationQPS      float64 `json:"near_saturation_qps"`
	OverloadQPS            float64 `json:"overload_qps"`
	RequestDeadlineMS      int     `json:"request_deadline_ms"`
	LateDrainMS            int     `json:"late_drain_ms"`
	TCPPolicy              string  `json:"tcp_policy"`
	W2CacheTTLMS           int64   `json:"w2_cache_ttl_ms"`
	W2TTLSafetyMarginMS    int64   `json:"w2_ttl_safety_margin_ms"`
	W2WarmLifecycle        string  `json:"w2_warm_lifecycle,omitempty"`
	HarnessCPUSet          string  `json:"harness_cpu_set"`
	SUTCPUSet              string  `json:"sut_cpu_set"`
	RecoveryMinimumSamples int     `json:"recovery_minimum_samples"`
	RecoveryP95CeilingUS   int64   `json:"recovery_p95_ceiling_us"`
	RecoveryP99CeilingUS   int64   `json:"recovery_p99_ceiling_us"`
}

type manifestEnvironment struct {
	HostAlias     string `json:"host_alias"`
	GOOS          string `json:"goos"`
	GOARCH        string `json:"goarch"`
	OnlineCPUs    int    `json:"online_cpus"`
	KernelRelease string `json:"kernel_release"`
	GoToolchain   string `json:"go_toolchain"`
	RustToolchain string `json:"rust_toolchain"`
}

type officialManifest struct {
	SchemaVersion          int                          `json:"schema_version"`
	OfficialFrozen         bool                         `json:"official_frozen"`
	RecoveryAssessmentMode string                       `json:"recovery_assessment_mode"`
	Inputs                 []manifestInput              `json:"inputs"`
	Runner                 manifestArtifact             `json:"runner"`
	Helper                 manifestHelper               `json:"helper"`
	Candidates             map[string]manifestCandidate `json:"candidates"`
	StageSequence          []string                     `json:"stage_sequence"`
	W3EventSchema          string                       `json:"w3_event_schema"`
	PairSchedule           []manifestPair               `json:"pair_schedule"`
	Scenarios              map[string]manifestScenario  `json:"scenarios"`
	Environment            manifestEnvironment          `json:"environment"`
}

type manifestValidationOptions struct {
	ManifestPath           string
	ExpectedSHA256         string
	RepoRoot               string
	HelperPath             string
	RunnerPath             string
	SUTPath                string
	Candidate              string
	Scenario               string
	Repetition             int
	Position               int
	StageDurationMS        int64
	NormalReferenceQPS     float64
	CommonLoadQPS          float64
	NearSaturationQPS      float64
	OverloadQPS            float64
	RequestDeadlineMS      int
	LateDrainMS            int
	W2CacheTTLMS           int64
	W2TTLSafetyMarginMS    int64
	W2WarmLifecycle        string
	RecoveryMinimumSamples int
	RecoveryP95CeilingUS   int64
	RecoveryP99CeilingUS   int64
	HarnessCPUSet          string
	SUTCPUSet              string
	HostAlias              string
	GOOS                   string
	GOARCH                 string
	OnlineCPUs             int
	KernelRelease          string
	GoToolchain            string
	RustToolchain          string
}

type recoveryCriteria struct {
	MinimumSamples int
	P95CeilingUS   int64
	P99CeilingUS   int64
}

func validateBinary(args []string) error {
	fs := flag.NewFlagSet("validate-binary", flag.ContinueOnError)
	path := fs.String("path", "", "executable path")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *path == "" {
		return errors.New("--path is required")
	}
	info, err := os.Stat(*path)
	if err != nil {
		return fmt.Errorf("stat binary: %w", err)
	}
	if info.IsDir() || info.Mode()&0111 == 0 {
		return fmt.Errorf("binary is not executable: %s", *path)
	}
	hash, err := sha256File(*path)
	if err != nil {
		return err
	}
	resolved, err := filepath.Abs(*path)
	if err != nil {
		return err
	}
	return writeJSON(os.Stdout, map[string]any{"path": resolved, "sha256": hash})
}

func processCPUSet(pid int) (string, error) {
	if pid <= 0 {
		return "", errors.New("PID must be positive")
	}
	if runtime.GOOS != "linux" {
		return "", fmt.Errorf("process affinity inspection requires Linux; current OS is %s", runtime.GOOS)
	}
	data, err := os.ReadFile(filepath.Join("/proc", strconv.Itoa(pid), "status"))
	if err != nil {
		return "", fmt.Errorf("read process status for PID %d: %w", pid, err)
	}
	return cpuSetFromStatus(string(data))
}

func cpuSetFromStatus(status string) (string, error) {
	for _, line := range strings.Split(status, "\n") {
		if strings.HasPrefix(line, "Cpus_allowed_list:") {
			value := strings.TrimSpace(strings.TrimPrefix(line, "Cpus_allowed_list:"))
			cpus, err := parseCPUList(value)
			if err != nil {
				return "", fmt.Errorf("invalid Cpus_allowed_list %q: %w", value, err)
			}
			return formatCPUList(cpus), nil
		}
	}
	return "", errors.New("process status has no Cpus_allowed_list")
}

func processStartIdentity(pid int) (string, error) {
	if pid <= 0 {
		return "", errors.New("PID must be positive")
	}
	if runtime.GOOS != "linux" {
		return "", fmt.Errorf("process start identity inspection requires Linux; current OS is %s", runtime.GOOS)
	}
	data, err := os.ReadFile(filepath.Join("/proc", strconv.Itoa(pid), "stat"))
	if err != nil {
		return "", fmt.Errorf("read process stat for PID %d: %w", pid, err)
	}
	closeParen := bytes.LastIndexByte(data, ')')
	if closeParen < 0 || closeParen+1 >= len(data) {
		return "", errors.New("malformed /proc process stat")
	}
	fields := strings.Fields(string(data[closeParen+1:]))
	// The substring starts at field 3 (state), so field 22 (starttime) is index 19.
	if len(fields) <= 19 || fields[19] == "" {
		return "", errors.New("/proc process stat is missing starttime")
	}
	bootID, err := os.ReadFile("/proc/sys/kernel/random/boot_id")
	if err != nil {
		return "", fmt.Errorf("read kernel boot ID: %w", err)
	}
	return strings.TrimSpace(string(bootID)) + ":" + fields[19], nil
}

func processCPUSetOrEmpty(pid int) string {
	if value, err := processCPUSet(pid); err == nil {
		return value
	}
	return ""
}

func processStartIdentityOrEmpty(pid int) string {
	if value, err := processStartIdentity(pid); err == nil {
		return value
	}
	return ""
}

func parseCPUList(value string) ([]int, error) {
	if strings.TrimSpace(value) == "" {
		return nil, errors.New("CPU list is empty")
	}
	seen := make(map[int]struct{})
	for _, part := range strings.Split(value, ",") {
		part = strings.TrimSpace(part)
		if part == "" {
			return nil, errors.New("CPU list contains an empty element")
		}
		bounds := strings.Split(part, "-")
		if len(bounds) > 2 {
			return nil, fmt.Errorf("invalid CPU range %q", part)
		}
		start, err := strconv.Atoi(bounds[0])
		if err != nil || start < 0 || start > 1_000_000 {
			return nil, fmt.Errorf("invalid CPU number %q", bounds[0])
		}
		end := start
		if len(bounds) == 2 {
			end, err = strconv.Atoi(bounds[1])
			if err != nil || end < start || end > 1_000_000 {
				return nil, fmt.Errorf("invalid CPU range %q", part)
			}
		}
		if end-start > 100_000 {
			return nil, fmt.Errorf("CPU range %q is unreasonably large", part)
		}
		for cpu := start; cpu <= end; cpu++ {
			if _, exists := seen[cpu]; exists {
				return nil, fmt.Errorf("CPU %d occurs more than once", cpu)
			}
			seen[cpu] = struct{}{}
		}
	}
	cpus := make([]int, 0, len(seen))
	for cpu := range seen {
		cpus = append(cpus, cpu)
	}
	sort.Ints(cpus)
	return cpus, nil
}

func formatCPUList(cpus []int) string {
	if len(cpus) == 0 {
		return ""
	}
	var parts []string
	start, previous := cpus[0], cpus[0]
	for _, cpu := range cpus[1:] {
		if cpu == previous+1 {
			previous = cpu
			continue
		}
		parts = appendCPUInterval(parts, start, previous)
		start, previous = cpu, cpu
	}
	parts = appendCPUInterval(parts, start, previous)
	return strings.Join(parts, ",")
}

func appendCPUInterval(parts []string, start, end int) []string {
	if start == end {
		return append(parts, strconv.Itoa(start))
	}
	return append(parts, strconv.Itoa(start)+"-"+strconv.Itoa(end))
}

func verifyDisjointCPULists(left, right string) error {
	leftCPUs, err := parseCPUList(left)
	if err != nil {
		return fmt.Errorf("invalid left CPU set: %w", err)
	}
	rightCPUs, err := parseCPUList(right)
	if err != nil {
		return fmt.Errorf("invalid right CPU set: %w", err)
	}
	leftSet := make(map[int]struct{}, len(leftCPUs))
	for _, cpu := range leftCPUs {
		leftSet[cpu] = struct{}{}
	}
	for _, cpu := range rightCPUs {
		if _, overlaps := leftSet[cpu]; overlaps {
			return fmt.Errorf("CPU %d is shared by the harness and SUT sets", cpu)
		}
	}
	return nil
}

func verifySingleCoreDisjointSets(harness, sut string) error {
	harnessCPUs, err := parseCPUList(harness)
	if err != nil {
		return fmt.Errorf("invalid harness CPU set: %w", err)
	}
	sutCPUs, err := parseCPUList(sut)
	if err != nil {
		return fmt.Errorf("invalid SUT CPU set: %w", err)
	}
	if len(harnessCPUs) != 1 || len(sutCPUs) != 1 {
		return fmt.Errorf("single-core comparison requires one CPU each: harness=%s SUT=%s", formatCPUList(harnessCPUs), formatCPUList(sutCPUs))
	}
	return verifyDisjointCPULists(harness, sut)
}

func verifyProcessAffinity(pid int, expected string) error {
	actual, err := processCPUSet(pid)
	if err != nil {
		return err
	}
	expectedCPUs, err := parseCPUList(expected)
	if err != nil {
		return fmt.Errorf("invalid expected CPU set: %w", err)
	}
	if actual != formatCPUList(expectedCPUs) {
		return fmt.Errorf("PID %d allowed CPUs differ: expected=%s actual=%s", pid, formatCPUList(expectedCPUs), actual)
	}
	return nil
}

func verifyAffinityCommand(args []string) error {
	fs := flag.NewFlagSet("verify-affinity", flag.ContinueOnError)
	pid := fs.Int("pid", 0, "process PID")
	expected := fs.String("expected", "", "exact expected Linux CPU list")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *pid <= 0 || *expected == "" {
		return errors.New("verify-affinity requires --pid and --expected")
	}
	return verifyProcessAffinity(*pid, *expected)
}

func verifyCPUSetsCommand(args []string) error {
	fs := flag.NewFlagSet("verify-cpu-sets", flag.ContinueOnError)
	left := fs.String("harness", "", "harness CPU list")
	right := fs.String("sut", "", "SUT CPU list")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *left == "" || *right == "" {
		return errors.New("verify-cpu-sets requires --harness and --sut")
	}
	return verifySingleCoreDisjointSets(*left, *right)
}

func verifyManifestCommand(args []string) error {
	fs := flag.NewFlagSet("verify-manifest", flag.ContinueOnError)
	opts := manifestValidationOptions{}
	fs.StringVar(&opts.ManifestPath, "manifest", "", "frozen task manifest JSON")
	fs.StringVar(&opts.ExpectedSHA256, "sha256", "", "expected manifest SHA-256")
	fs.StringVar(&opts.RepoRoot, "repo-root", "", "repository root for fixed inputs")
	fs.StringVar(&opts.HelperPath, "helper", "", "running helper binary path")
	fs.StringVar(&opts.RunnerPath, "runner", "", "running benchmark runner path")
	fs.StringVar(&opts.SUTPath, "sut", "", "candidate binary path")
	fs.StringVar(&opts.Candidate, "candidate", "", "go or rust candidate label")
	fs.StringVar(&opts.Scenario, "scenario", "", "scenario selected for this invocation")
	fs.IntVar(&opts.Repetition, "repetition", 0, "one-based paired repetition")
	fs.IntVar(&opts.Position, "position", 0, "one-based candidate position in the paired schedule")
	fs.Int64Var(&opts.StageDurationMS, "stage-duration-ms", 0, "stage duration used by runner")
	fs.Float64Var(&opts.NormalReferenceQPS, "normal-reference-qps", 0, "normal reference offered rate")
	fs.Float64Var(&opts.CommonLoadQPS, "common-load-qps", 0, "common offered rate")
	fs.Float64Var(&opts.NearSaturationQPS, "near-saturation-qps", 0, "near-saturation offered rate")
	fs.Float64Var(&opts.OverloadQPS, "overload-qps", 0, "overload offered rate")
	fs.IntVar(&opts.RequestDeadlineMS, "deadline-ms", 0, "per request deadline")
	fs.IntVar(&opts.LateDrainMS, "late-drain-ms", -1, "late response drain")
	fs.Int64Var(&opts.W2CacheTTLMS, "w2-cache-ttl-ms", 0, "frozen W2 fixture TTL")
	fs.Int64Var(&opts.W2TTLSafetyMarginMS, "w2-ttl-safety-margin-ms", -1, "frozen W2 TTL safety margin")
	fs.StringVar(&opts.W2WarmLifecycle, "w2-warm-lifecycle", "not-applicable", "same-process or independent-prefilled for W2")
	fs.IntVar(&opts.RecoveryMinimumSamples, "recovery-minimum-samples", 0, "frozen reference/recovery sample minimum")
	fs.Int64Var(&opts.RecoveryP95CeilingUS, "recovery-p95-ceiling-us", 0, "frozen recovery p95 ceiling")
	fs.Int64Var(&opts.RecoveryP99CeilingUS, "recovery-p99-ceiling-us", 0, "frozen recovery p99 ceiling")
	fs.StringVar(&opts.HarnessCPUSet, "harness-cpu-set", "", "runner/helper/fixture affinity")
	fs.StringVar(&opts.SUTCPUSet, "sut-cpu-set", "", "candidate affinity")
	fs.StringVar(&opts.HostAlias, "host-alias", "", "frozen SSH host alias")
	fs.StringVar(&opts.RustToolchain, "rust-toolchain", "", "frozen Rust compiler version")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if opts.ManifestPath == "" || opts.ExpectedSHA256 == "" || opts.RepoRoot == "" || opts.HelperPath == "" || opts.RunnerPath == "" || opts.SUTPath == "" || opts.Candidate == "" || opts.Scenario == "" || opts.HarnessCPUSet == "" || opts.SUTCPUSet == "" || opts.HostAlias == "" || opts.RustToolchain == "" {
		return errors.New("verify-manifest requires manifest and SHA, artifact paths, candidate/scenario, CPU masks, host alias, and Rust toolchain")
	}
	if opts.Repetition <= 0 || opts.Position <= 0 || opts.StageDurationMS <= 0 || opts.NormalReferenceQPS <= 0 || opts.CommonLoadQPS <= 0 || opts.NearSaturationQPS <= 0 || opts.OverloadQPS <= 0 || opts.RequestDeadlineMS <= 0 || opts.LateDrainMS < 0 || opts.W2CacheTTLMS <= 0 || opts.W2TTLSafetyMarginMS < 0 || opts.RecoveryMinimumSamples <= 0 || opts.RecoveryP95CeilingUS <= 0 || opts.RecoveryP99CeilingUS < opts.RecoveryP95CeilingUS {
		return errors.New("verify-manifest requires positive repetition, position, stage, QPS and recovery values, plus non-negative drain/margin")
	}
	return verifyOfficialManifest(opts)
}

func verifyOfficialManifest(opts manifestValidationOptions) error {
	manifestDigest, err := sha256File(opts.ManifestPath)
	if err != nil {
		return fmt.Errorf("hash manifest: %w", err)
	}
	if !validSHA256(opts.ExpectedSHA256) || manifestDigest != strings.ToLower(opts.ExpectedSHA256) {
		return fmt.Errorf("manifest SHA-256 mismatch: expected=%s actual=%s", opts.ExpectedSHA256, manifestDigest)
	}
	file, err := os.Open(opts.ManifestPath)
	if err != nil {
		return err
	}
	var manifest officialManifest
	decodeErr := json.NewDecoder(file).Decode(&manifest)
	closeErr := file.Close()
	if decodeErr != nil {
		return fmt.Errorf("decode manifest: %w", decodeErr)
	}
	if closeErr != nil {
		return closeErr
	}
	if manifest.SchemaVersion != 1 || !manifest.OfficialFrozen {
		return errors.New("official mode requires schema_version=1 and official_frozen=true")
	}
	if manifest.RecoveryAssessmentMode != recoveryAssessmentMode {
		return fmt.Errorf("official recovery assessment mode must be %q, got %q", recoveryAssessmentMode, manifest.RecoveryAssessmentMode)
	}
	if manifest.Environment.HostAlias != opts.HostAlias || manifest.Environment.GOOS != runtime.GOOS || manifest.Environment.GOARCH != runtime.GOARCH || manifest.Environment.OnlineCPUs != currentOnlineCPUCount() || manifest.Environment.KernelRelease != currentKernelRelease() || manifest.Environment.GoToolchain != runtime.Version() || manifest.Environment.RustToolchain != opts.RustToolchain {
		return fmt.Errorf("official environment differs from manifest: host=%s go=%s/%s online_cpus=%d kernel=%s go_toolchain=%s rust_toolchain=%s", opts.HostAlias, runtime.GOOS, runtime.GOARCH, currentOnlineCPUCount(), currentKernelRelease(), runtime.Version(), opts.RustToolchain)
	}
	root, err := filepath.Abs(opts.RepoRoot)
	if err != nil {
		return err
	}
	if len(manifest.Inputs) != len(fixedCorpusInputPaths) {
		return fmt.Errorf("manifest must contain exactly %d fixed corpus inputs; got %d", len(fixedCorpusInputPaths), len(manifest.Inputs))
	}
	inputByPath := make(map[string]string, len(manifest.Inputs))
	for _, input := range manifest.Inputs {
		clean := filepath.Clean(input.Path)
		if filepath.IsAbs(input.Path) || clean == "." || clean == ".." || strings.HasPrefix(clean, ".."+string(filepath.Separator)) {
			return fmt.Errorf("manifest input path must stay within repository root: %q", input.Path)
		}
		if _, exists := inputByPath[clean]; exists {
			return fmt.Errorf("manifest repeats input path %q", clean)
		}
		inputByPath[clean] = input.SHA256
	}
	for _, rel := range fixedCorpusInputPaths {
		want, ok := inputByPath[filepath.Clean(rel)]
		if !ok {
			return fmt.Errorf("manifest is missing fixed input %q", rel)
		}
		if !validSHA256(want) {
			return fmt.Errorf("manifest has invalid SHA-256 for %s", rel)
		}
		actual, err := sha256File(filepath.Join(root, rel))
		if err != nil {
			return fmt.Errorf("hash fixed input %s: %w", rel, err)
		}
		if actual != strings.ToLower(want) {
			return fmt.Errorf("fixed input hash mismatch for %s: expected=%s actual=%s", rel, want, actual)
		}
	}
	if err := verifyManifestArtifact(root, manifest.Runner, opts.RunnerPath, "runner"); err != nil {
		return err
	}
	helperSource := manifestArtifact{Path: manifest.Helper.SourcePath, SHA256: manifest.Helper.SourceSHA256}
	if err := verifyManifestArtifact(root, helperSource, filepath.Join(root, "tests/phase5a-baseline/cmd/phase5a-baseline/main.go"), "helper source"); err != nil {
		return err
	}
	if manifest.Helper.Version != helperVersion {
		return fmt.Errorf("helper version mismatch: manifest=%q current=%q", manifest.Helper.Version, helperVersion)
	}
	if err := verifyManifestPathAndHash(root, manifest.Helper.BinaryPath, manifest.Helper.BinarySHA256, opts.HelperPath, "helper binary"); err != nil {
		return err
	}
	if opts.Candidate != "go" && opts.Candidate != "rust" {
		return fmt.Errorf("unsupported candidate %q", opts.Candidate)
	}
	candidate, ok := manifest.Candidates[opts.Candidate]
	if !ok || candidate.SourceCommit == "" {
		return fmt.Errorf("manifest is missing source identity for candidate %q", opts.Candidate)
	}
	if err := verifyManifestPathAndHash(root, candidate.BinaryPath, candidate.BinarySHA256, opts.SUTPath, opts.Candidate+" SUT"); err != nil {
		return err
	}
	if !equalStrings(manifest.StageSequence, continuousStageSequence) {
		return fmt.Errorf("manifest stage sequence mismatch: got=%v want=%v", manifest.StageSequence, continuousStageSequence)
	}
	if manifest.W3EventSchema != fixtureEventSchema {
		return fmt.Errorf("manifest W3 event schema mismatch: got=%q want=%q", manifest.W3EventSchema, fixtureEventSchema)
	}
	if err := verifyManifestPairSchedule(manifest.PairSchedule, opts.Repetition, opts.Position, opts.Candidate); err != nil {
		return err
	}
	planned, ok := manifest.Scenarios[opts.Scenario]
	if !ok {
		return fmt.Errorf("manifest has no scenario plan for %q", opts.Scenario)
	}
	if err := validateManifestScenario(planned); err != nil {
		return fmt.Errorf("invalid scenario plan for %s: %w", opts.Scenario, err)
	}
	if err := verifySingleCoreDisjointSets(opts.HarnessCPUSet, opts.SUTCPUSet); err != nil {
		return err
	}
	if canonicalCPUSet(opts.HarnessCPUSet) != canonicalCPUSet(planned.HarnessCPUSet) || canonicalCPUSet(opts.SUTCPUSet) != canonicalCPUSet(planned.SUTCPUSet) {
		return fmt.Errorf("CPU placement differs from manifest: harness=%s/%s sut=%s/%s", planned.HarnessCPUSet, opts.HarnessCPUSet, planned.SUTCPUSet, opts.SUTCPUSet)
	}
	if opts.Scenario == "w2" {
		if opts.W2WarmLifecycle != planned.W2WarmLifecycle {
			return fmt.Errorf("official W2 warm lifecycle differs from manifest: expected=%s actual=%s", planned.W2WarmLifecycle, opts.W2WarmLifecycle)
		}
		if planned.W2WarmLifecycle != "same-process" && planned.W2WarmLifecycle != "independent-prefilled" {
			return fmt.Errorf("manifest W2 warm lifecycle is invalid: %q", planned.W2WarmLifecycle)
		}
	}
	checks := []struct {
		name string
		want float64
		got  float64
	}{
		{"stage duration ms", float64(planned.StageDurationMS), float64(opts.StageDurationMS)},
		{"normal reference QPS", planned.NormalReferenceQPS, opts.NormalReferenceQPS},
		{"common load QPS", planned.CommonLoadQPS, opts.CommonLoadQPS},
		{"near-saturation QPS", planned.NearSaturationQPS, opts.NearSaturationQPS},
		{"overload QPS", planned.OverloadQPS, opts.OverloadQPS},
		{"deadline ms", float64(planned.RequestDeadlineMS), float64(opts.RequestDeadlineMS)},
		{"late drain ms", float64(planned.LateDrainMS), float64(opts.LateDrainMS)},
		{"W2 cache TTL ms", float64(planned.W2CacheTTLMS), float64(opts.W2CacheTTLMS)},
		{"W2 TTL safety margin ms", float64(planned.W2TTLSafetyMarginMS), float64(opts.W2TTLSafetyMarginMS)},
		{"recovery minimum samples", float64(planned.RecoveryMinimumSamples), float64(opts.RecoveryMinimumSamples)},
		{"recovery p95 ceiling us", float64(planned.RecoveryP95CeilingUS), float64(opts.RecoveryP95CeilingUS)},
		{"recovery p99 ceiling us", float64(planned.RecoveryP99CeilingUS), float64(opts.RecoveryP99CeilingUS)},
	}
	for _, check := range checks {
		if math.Abs(check.want-check.got) > 1e-9 {
			return fmt.Errorf("official %s differs from frozen manifest: expected=%v actual=%v", check.name, check.want, check.got)
		}
	}
	return nil
}

func currentKernelRelease() string {
	data, err := os.ReadFile("/proc/sys/kernel/osrelease")
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(data))
}

func currentOnlineCPUCount() int {
	data, err := os.ReadFile("/sys/devices/system/cpu/online")
	if err == nil {
		if cpus, parseErr := parseCPUList(strings.TrimSpace(string(data))); parseErr == nil {
			return len(cpus)
		}
	}
	return runtime.NumCPU()
}

func canonicalCPUSet(value string) string {
	cpus, err := parseCPUList(value)
	if err != nil {
		return ""
	}
	return formatCPUList(cpus)
}

func verifyManifestArtifact(root string, artifact manifestArtifact, actualPath, label string) error {
	return verifyManifestPathAndHash(root, artifact.Path, artifact.SHA256, actualPath, label)
}

func verifyManifestPathAndHash(root, manifestPath, expectedHash, actualPath, label string) error {
	if manifestPath == "" || !validSHA256(expectedHash) {
		return fmt.Errorf("manifest has incomplete %s path/hash", label)
	}
	manifestResolved := manifestPath
	if !filepath.IsAbs(manifestResolved) {
		manifestResolved = filepath.Join(root, manifestResolved)
	}
	if !sameExistingPath(manifestResolved, actualPath) {
		return fmt.Errorf("%s path mismatch: manifest=%q actual=%q", label, manifestPath, actualPath)
	}
	actualHash, err := sha256File(actualPath)
	if err != nil {
		return fmt.Errorf("hash %s: %w", label, err)
	}
	if actualHash != strings.ToLower(expectedHash) {
		return fmt.Errorf("%s SHA-256 mismatch: expected=%s actual=%s", label, expectedHash, actualHash)
	}
	return nil
}

func sameExistingPath(left, right string) bool {
	leftAbs, leftErr := filepath.Abs(left)
	rightAbs, rightErr := filepath.Abs(right)
	if leftErr != nil || rightErr != nil {
		return false
	}
	leftResolved, leftEvalErr := filepath.EvalSymlinks(leftAbs)
	rightResolved, rightEvalErr := filepath.EvalSymlinks(rightAbs)
	if leftEvalErr == nil && rightEvalErr == nil {
		return filepath.Clean(leftResolved) == filepath.Clean(rightResolved)
	}
	return filepath.Clean(leftAbs) == filepath.Clean(rightAbs)
}

func validSHA256(value string) bool {
	if len(value) != sha256.Size*2 || strings.ToLower(value) != value {
		return false
	}
	_, err := hex.DecodeString(value)
	return err == nil
}

func validateManifestScenario(s manifestScenario) error {
	if s.StageDurationMS <= 0 || s.NormalReferenceQPS <= 0 || s.CommonLoadQPS <= 0 || s.NearSaturationQPS <= 0 || s.OverloadQPS <= 0 {
		return errors.New("stage duration and offered rates must be positive")
	}
	if !(s.NormalReferenceQPS < s.CommonLoadQPS && s.CommonLoadQPS < s.NearSaturationQPS && s.NearSaturationQPS < s.OverloadQPS) {
		return errors.New("offered rates must strictly increase from normal reference through overload")
	}
	if s.RequestDeadlineMS <= 0 || s.LateDrainMS < 0 || s.TCPPolicy != "fresh-connection-per-request" {
		return errors.New("deadline, late drain, or TCP connection policy is invalid")
	}
	if s.W2CacheTTLMS <= 0 || s.W2TTLSafetyMarginMS < 0 || s.W2TTLSafetyMarginMS >= s.W2CacheTTLMS {
		return errors.New("W2 TTL and safety margin are invalid")
	}
	if s.W2WarmLifecycle != "" && s.W2WarmLifecycle != "same-process" && s.W2WarmLifecycle != "independent-prefilled" {
		return errors.New("W2 warm lifecycle must be same-process or independent-prefilled")
	}
	if err := verifySingleCoreDisjointSets(s.HarnessCPUSet, s.SUTCPUSet); err != nil {
		return fmt.Errorf("CPU placement: %w", err)
	}
	if s.RecoveryMinimumSamples <= 0 || s.RecoveryP95CeilingUS <= 0 || s.RecoveryP99CeilingUS < s.RecoveryP95CeilingUS {
		return errors.New("recovery sample minimum or p95/p99 ceilings are invalid")
	}
	return nil
}

func verifyManifestPairSchedule(schedule []manifestPair, repetition, position int, candidate string) error {
	if len(schedule) < 3 {
		return errors.New("frozen pair schedule must contain at least three repetitions")
	}
	if repetition <= 0 || repetition > len(schedule) || position < 1 || position > 2 {
		return fmt.Errorf("repetition/position is not present in the frozen pair schedule: repetition=%d position=%d", repetition, position)
	}
	for index, pair := range schedule {
		if pair.Repetition != index+1 || len(pair.Order) != 2 || pair.Order[0] == pair.Order[1] {
			return fmt.Errorf("invalid pair schedule row %d", index+1)
		}
		if !containsString(pair.Order, "go") || !containsString(pair.Order, "rust") {
			return fmt.Errorf("pair schedule row %d must contain exactly go and rust", index+1)
		}
		if index > 0 && pair.Order[0] == schedule[index-1].Order[0] {
			return fmt.Errorf("candidate order does not alternate at repetition %d", index+1)
		}
	}
	if schedule[repetition-1].Order[position-1] != candidate {
		return fmt.Errorf("candidate order mismatch: repetition=%d position=%d expects=%s got=%s", repetition, position, schedule[repetition-1].Order[position-1], candidate)
	}
	return nil
}

func buildPairSchedule(repetitions int) ([]manifestPair, error) {
	if repetitions < 3 {
		return nil, errors.New("paired performance schedule requires at least three repetitions")
	}
	schedule := make([]manifestPair, repetitions)
	for i := range schedule {
		if i%2 == 0 {
			schedule[i] = manifestPair{Repetition: i + 1, Order: []string{"go", "rust"}}
		} else {
			schedule[i] = manifestPair{Repetition: i + 1, Order: []string{"rust", "go"}}
		}
	}
	return schedule, nil
}

func aggregatePairedStages(observations []pairedStageObservation, schedule []manifestPair, expectedManifestSHA string) ([]pairedStageAggregate, error) {
	if !validSHA256(expectedManifestSHA) {
		return nil, errors.New("expected manifest SHA-256 must be a 64-character hexadecimal digest")
	}
	if len(schedule) < 3 {
		return nil, errors.New("paired aggregation requires at least three scheduled repetitions")
	}
	if len(observations) == 0 {
		return nil, errors.New("paired aggregation has no observations")
	}

	type groupKey struct {
		scenario string
		stage    string
		qps      float64
	}
	groups := make(map[groupKey]map[int]map[string]pairedStageObservation)
	for _, observation := range observations {
		if observation.Scenario == "" || observation.Stage == "" {
			return nil, errors.New("paired observation is missing scenario or stage")
		}
		if observation.ManifestSHA256 != expectedManifestSHA {
			return nil, fmt.Errorf("manifest SHA mismatch for %s/%s repetition %d candidate %s", observation.Scenario, observation.Stage, observation.Repetition, observation.Candidate)
		}
		if err := verifyManifestPairSchedule(schedule, observation.Repetition, observation.PairPosition, observation.Candidate); err != nil {
			return nil, err
		}
		if observation.TargetQPS <= 0 || observation.DurationMS <= 0 {
			return nil, fmt.Errorf("invalid rate or duration for %s/%s repetition %d candidate %s", observation.Scenario, observation.Stage, observation.Repetition, observation.Candidate)
		}
		if observation.LatencySampleCount < 0 || (observation.Counters.Scheduled <= 0 && observation.InvalidReason == "") {
			return nil, fmt.Errorf("invalid sample counters for %s/%s repetition %d candidate %s", observation.Scenario, observation.Stage, observation.Repetition, observation.Candidate)
		}
		if observation.P50US < 0 || observation.P95US < observation.P50US || observation.P99US < observation.P95US {
			return nil, fmt.Errorf("invalid latency quantiles for %s/%s repetition %d candidate %s", observation.Scenario, observation.Stage, observation.Repetition, observation.Candidate)
		}

		key := groupKey{scenario: observation.Scenario, stage: observation.Stage, qps: observation.TargetQPS}
		if groups[key] == nil {
			groups[key] = make(map[int]map[string]pairedStageObservation)
		}
		if groups[key][observation.Repetition] == nil {
			groups[key][observation.Repetition] = make(map[string]pairedStageObservation, 2)
		}
		byCandidate := groups[key][observation.Repetition]
		if _, duplicate := byCandidate[observation.Candidate]; duplicate {
			return nil, fmt.Errorf("duplicate %s result for %s/%s repetition %d at %.3f QPS", observation.Candidate, observation.Scenario, observation.Stage, observation.Repetition, observation.TargetQPS)
		}
		byCandidate[observation.Candidate] = observation
	}

	keys := make([]groupKey, 0, len(groups))
	for key := range groups {
		keys = append(keys, key)
	}
	sort.Slice(keys, func(i, j int) bool {
		if keys[i].scenario != keys[j].scenario {
			return keys[i].scenario < keys[j].scenario
		}
		if keys[i].stage != keys[j].stage {
			return keys[i].stage < keys[j].stage
		}
		return keys[i].qps < keys[j].qps
	})

	result := make([]pairedStageAggregate, 0, len(keys))
	for _, key := range keys {
		byRepetition := groups[key]
		summary := pairedStageAggregate{Scenario: key.scenario, Stage: key.stage, TargetQPS: key.qps, CompletePairs: len(schedule)}
		var goP50, goP95, goP99, goQPS, goSamples []float64
		var rustP50, rustP95, rustP99, rustQPS, rustSamples []float64
		var p95Deltas []float64
		recoveryAssessmentValue := ""
		recoveryAssessmentsConsistent := true
		for repetition := 1; repetition <= len(schedule); repetition++ {
			pair, ok := byRepetition[repetition]
			if !ok {
				return nil, fmt.Errorf("missing paired results for %s/%s at %.3f QPS repetition %d", key.scenario, key.stage, key.qps, repetition)
			}
			goObservation, hasGo := pair["go"]
			rustObservation, hasRust := pair["rust"]
			if !hasGo || !hasRust {
				missing := "go"
				if hasGo {
					missing = "rust"
				}
				return nil, fmt.Errorf("incomplete paired results for %s/%s at %.3f QPS repetition %d: missing %s", key.scenario, key.stage, key.qps, repetition, missing)
			}
			if key.stage == "recovery" {
				prefix := "status=indeterminate; mode=" + recoveryAssessmentMode + "; reason="
				goAssessmentValid := strings.HasPrefix(goObservation.RecoveryAssessment, prefix)
				rustAssessmentValid := strings.HasPrefix(rustObservation.RecoveryAssessment, prefix)
				if !goAssessmentValid {
					goObservation.InvalidReason = appendInvalidReason(goObservation.InvalidReason, "missing or invalid service recovery assessment evidence")
					recoveryAssessmentsConsistent = false
				}
				if !rustAssessmentValid {
					rustObservation.InvalidReason = appendInvalidReason(rustObservation.InvalidReason, "missing or invalid service recovery assessment evidence")
					recoveryAssessmentsConsistent = false
				}
				if goAssessmentValid && rustAssessmentValid && goObservation.RecoveryAssessment != rustObservation.RecoveryAssessment {
					if goObservation.InvalidReason == "" && rustObservation.InvalidReason == "" {
						goObservation.InvalidReason = appendInvalidReason(goObservation.InvalidReason, "paired recovery assessment differs")
						rustObservation.InvalidReason = appendInvalidReason(rustObservation.InvalidReason, "paired recovery assessment differs")
					}
					recoveryAssessmentsConsistent = false
				} else if goAssessmentValid && rustAssessmentValid {
					if recoveryAssessmentValue == "" {
						recoveryAssessmentValue = goObservation.RecoveryAssessment
					} else if recoveryAssessmentValue != goObservation.RecoveryAssessment {
						recoveryAssessmentsConsistent = false
					}
				}
			}

			goReason := pairedObservationInvalidReason(goObservation)
			rustReason := pairedObservationInvalidReason(rustObservation)
			if goReason != "" {
				summary.InvalidPairs = append(summary.InvalidPairs, pairedInvalidRecord{Repetition: repetition, Candidate: "go", Reason: goReason})
			}
			if rustReason != "" {
				summary.InvalidPairs = append(summary.InvalidPairs, pairedInvalidRecord{Repetition: repetition, Candidate: "rust", Reason: rustReason})
			}
			if goReason != "" || rustReason != "" {
				continue
			}

			summary.ValidPairs++
			goP50 = append(goP50, float64(goObservation.P50US))
			goP95 = append(goP95, float64(goObservation.P95US))
			goP99 = append(goP99, float64(goObservation.P99US))
			goQPS = append(goQPS, effectiveQPS(goObservation))
			goSamples = append(goSamples, float64(goObservation.LatencySampleCount))
			rustP50 = append(rustP50, float64(rustObservation.P50US))
			rustP95 = append(rustP95, float64(rustObservation.P95US))
			rustP99 = append(rustP99, float64(rustObservation.P99US))
			rustQPS = append(rustQPS, effectiveQPS(rustObservation))
			rustSamples = append(rustSamples, float64(rustObservation.LatencySampleCount))
			p95Deltas = append(p95Deltas, float64(rustObservation.P95US-goObservation.P95US))
		}
		if key.stage == "recovery" {
			if recoveryAssessmentsConsistent && recoveryAssessmentValue != "" {
				summary.RecoveryAssessment = recoveryAssessmentValue
			} else {
				summary.RecoveryAssessment = "status=indeterminate; mode=" + recoveryAssessmentMode + "; reason=service recovery assessment evidence is incomplete or inconsistent"
			}
		}
		summary.Go = summarizePairedCandidate(goP50, goP95, goP99, goQPS, goSamples)
		summary.Rust = summarizePairedCandidate(rustP50, rustP95, rustP99, rustQPS, rustSamples)
		summary.RustMinusGoP95US = summarizePairedValues(p95Deltas)
		result = append(result, summary)
	}
	return result, nil
}

func pairedObservationInvalidReason(observation pairedStageObservation) string {
	if observation.InvalidReason != "" {
		return observation.InvalidReason
	}
	c := observation.Counters
	switch {
	case c.SenderShortfall != 0 || c.Sent != c.Scheduled:
		return fmt.Sprintf("sender shortfall or unsent requests (scheduled=%d sent=%d shortfall=%d)", c.Scheduled, c.Sent, c.SenderShortfall)
	case c.CorrectOnTime != c.Scheduled:
		return fmt.Sprintf("not every scheduled request was correct on time (scheduled=%d correct_on_time=%d)", c.Scheduled, c.CorrectOnTime)
	case c.Received != c.Scheduled:
		return fmt.Sprintf("received count differs from scheduled count (scheduled=%d received=%d)", c.Scheduled, c.Received)
	case c.CorrectLate != 0 || c.WrongResponse != 0 || c.ProtocolError != 0 || c.TransportError != 0 || c.Timeout != 0:
		return fmt.Sprintf("stage contains late or failed responses (late=%d wrong=%d protocol=%d transport=%d timeout=%d)", c.CorrectLate, c.WrongResponse, c.ProtocolError, c.TransportError, c.Timeout)
	case observation.LatencySampleCount != int(c.CorrectOnTime):
		return fmt.Sprintf("latency sample count differs from correct-on-time count (samples=%d correct_on_time=%d)", observation.LatencySampleCount, c.CorrectOnTime)
	default:
		return ""
	}
}

func effectiveQPS(observation pairedStageObservation) float64 {
	return float64(observation.Counters.CorrectOnTime) * 1000 / float64(observation.DurationMS)
}

func summarizePairedCandidate(p50, p95, p99, qps, samples []float64) pairedCandidateSummary {
	return pairedCandidateSummary{
		P50US:               summarizePairedValues(p50),
		P95US:               summarizePairedValues(p95),
		P99US:               summarizePairedValues(p99),
		EffectiveThroughput: summarizePairedValues(qps),
		LatencySampleCount:  summarizePairedValues(samples),
	}
}

func summarizePairedValues(values []float64) pairedMetricRange {
	if len(values) == 0 {
		return pairedMetricRange{}
	}
	sorted := append([]float64(nil), values...)
	sort.Float64s(sorted)
	median := sorted[len(sorted)/2]
	if len(sorted)%2 == 0 {
		median = (sorted[len(sorted)/2-1] + sorted[len(sorted)/2]) / 2
	}
	return pairedMetricRange{N: len(sorted), Min: sorted[0], Median: median, Max: sorted[len(sorted)-1]}
}

type pairedPlannedStage struct {
	name       string
	qps        float64
	resultPath string
}

func collectPairedStageObservations(resultsRoot string, manifest officialManifest, manifestSHA string) ([]pairedStageObservation, error) {
	if resultsRoot == "" {
		return nil, errors.New("results root is required")
	}
	if !manifest.OfficialFrozen {
		return nil, errors.New("paired result collection requires an official-frozen manifest")
	}
	if manifest.RecoveryAssessmentMode != recoveryAssessmentMode {
		return nil, fmt.Errorf("unsupported recovery assessment mode %q", manifest.RecoveryAssessmentMode)
	}
	if len(manifest.PairSchedule) < 3 || len(manifest.Scenarios) == 0 {
		return nil, errors.New("frozen manifest lacks paired repetitions or scenarios")
	}
	scenarios := make([]string, 0, len(manifest.Scenarios))
	for scenario := range manifest.Scenarios {
		scenarios = append(scenarios, scenario)
	}
	sort.Strings(scenarios)
	observations := make([]pairedStageObservation, 0)
	for _, scenario := range scenarios {
		plan := manifest.Scenarios[scenario]
		plannedStages, err := phase5aPlannedStages(scenario, plan)
		if err != nil {
			return nil, err
		}
		for _, pair := range manifest.PairSchedule {
			for position, candidate := range pair.Order {
				runDir := filepath.Join(resultsRoot, scenario, fmt.Sprintf("repetition-%d", pair.Repetition), candidate)
				metadata, metadataErr := readRunMetadata(filepath.Join(runDir, "run-metadata.txt"))
				if metadataErr != nil && !errors.Is(metadataErr, os.ErrNotExist) {
					return nil, metadataErr
				}
				if got := metadata["manifest_sha256"]; got != "" && got != manifestSHA {
					return nil, fmt.Errorf("result manifest SHA mismatch in %s: got %s want %s", runDir, got, manifestSHA)
				}
				if got := metadata["candidate"]; got != "" && got != candidate {
					return nil, fmt.Errorf("result candidate mismatch in %s: got %s want %s", runDir, got, candidate)
				}
				if got := metadata["scenario"]; got != "" && got != scenario {
					return nil, fmt.Errorf("result scenario mismatch in %s: got %s want %s", runDir, got, scenario)
				}
				if got := metadata["run_mode"]; got != "" && got != "official" {
					return nil, fmt.Errorf("result in %s is not official mode: %s", runDir, got)
				}
				if got := metadata["repetition"]; got != "" && got != strconv.Itoa(pair.Repetition) {
					return nil, fmt.Errorf("result repetition mismatch in %s: got %s want %d", runDir, got, pair.Repetition)
				}
				if got := metadata["pair_position"]; got != "" && got != strconv.Itoa(position+1) {
					return nil, fmt.Errorf("result pair position mismatch in %s: got %s want %d", runDir, got, position+1)
				}

				invalidReasons, err := readInvalidStageReasons(filepath.Join(runDir, "invalid-stages.tsv"))
				if err != nil {
					return nil, err
				}
				stageReasons, err := expandInvalidStageReasons(scenario, plannedStages, invalidReasons)
				if err != nil {
					return nil, fmt.Errorf("invalid stage reasons in %s: %w", runDir, err)
				}
				assessment, assessmentErr := readServiceRecoveryAssessment(filepath.Join(runDir, "service-recovery-assessment.txt"), manifest.RecoveryAssessmentMode)
				if assessmentErr != nil {
					stageReasons["recovery"] = appendInvalidReason(stageReasons["recovery"], "service recovery assessment: "+assessmentErr.Error())
				}
				if scenario == "w2" {
					lifecyclePath := filepath.Join(runDir, "w2-warm", "recovery-status.txt")
					if plan.W2WarmLifecycle == "independent-prefilled" {
						lifecyclePath = filepath.Join(runDir, "w2-warm-independent", "recovery-status.txt")
					}
					lifecycleStatus, statusErr := readSmallTextFile(lifecyclePath)
					lifecycleValid := statusErr == nil && ((plan.W2WarmLifecycle == "same-process" && lifecycleStatus == "TTL-eligible") || (plan.W2WarmLifecycle == "independent-prefilled" && strings.HasPrefix(lifecycleStatus, "indeterminate:")))
					if !lifecycleValid {
						reason := "missing or invalid W2 warm lifecycle status"
						if statusErr != nil && !errors.Is(statusErr, os.ErrNotExist) {
							return nil, statusErr
						}
						if statusErr == nil {
							reason += ": " + lifecycleStatus
						}
						for _, stage := range continuousStageSequence {
							stageReasons[stage] = appendInvalidReason(stageReasons[stage], reason)
						}
					}
				}
				runLevelInvalid := ""
				if metadataErr != nil {
					runLevelInvalid = "missing official run metadata"
				}
				if _, err := os.Stat(runDir); errors.Is(err, os.ErrNotExist) {
					runLevelInvalid = "missing candidate run directory"
				}
				for _, key := range []string{"scenario", "run_mode", "candidate", "repetition", "pair_position", "manifest_sha256", "recovery_assessment_mode"} {
					if metadata[key] == "" {
						runLevelInvalid = appendInvalidReason(runLevelInvalid, "missing run metadata field "+key)
					}
				}
				if metadata["recovery_assessment_mode"] != manifest.RecoveryAssessmentMode {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, "run recovery assessment mode differs from frozen manifest")
				}
				if scenario == "w2" && metadata["w2_warm_lifecycle"] != plan.W2WarmLifecycle {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, "run W2 warm lifecycle differs from frozen manifest")
				}
				if metadata["manifest_sha256"] == "" {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, "missing manifest SHA evidence")
				}
				if metadata["run_mode"] != "official" {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, "run mode is not official")
				}
				helperVersion, err := readSmallTextFile(filepath.Join(runDir, "helper-version.txt"))
				if errors.Is(err, os.ErrNotExist) {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, "missing helper version evidence")
				} else if err != nil {
					return nil, err
				} else if helperVersion != manifest.Helper.Version {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, fmt.Sprintf("helper version mismatch: got %s want %s", helperVersion, manifest.Helper.Version))
				}
				manifestFileSHA, err := readManifestSHAFile(filepath.Join(runDir, "manifest.sha256"))
				if errors.Is(err, os.ErrNotExist) {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, "missing manifest.sha256 evidence")
				} else if err != nil {
					return nil, err
				} else if manifestFileSHA != manifestSHA {
					return nil, fmt.Errorf("manifest.sha256 mismatch in %s: got %s want %s", runDir, manifestFileSHA, manifestSHA)
				}
				if hashReason, err := verifyRunInputHashes(filepath.Join(runDir, "input-hashes.sha256"), manifest, candidate); err != nil {
					return nil, err
				} else if hashReason != "" {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, hashReason)
				}
				status, err := readAttemptExitStatus(filepath.Join(runDir, "attempt-exit-status.txt"))
				if err != nil {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, "missing attempt exit status")
				} else if status != 0 && !hasInvalidReason(invalidReasons) {
					runLevelInvalid = appendInvalidReason(runLevelInvalid, fmt.Sprintf("runner exited with status %d and no stage failure detail", status))
				}

				stageRows := make(map[string]stageResult)
				loadedResultPaths := make(map[string]struct{})
				for _, planned := range plannedStages {
					relative := planned.resultPath
					if _, loaded := loadedResultPaths[relative]; loaded {
						continue
					}
					loadedResultPaths[relative] = struct{}{}
					path := filepath.Join(runDir, relative)
					rows, err := readStageResultRows(path)
					if errors.Is(err, os.ErrNotExist) {
						continue
					}
					if err != nil {
						return nil, fmt.Errorf("read stage results %s: %w", path, err)
					}
					for _, row := range rows {
						if _, duplicate := stageRows[row.Stage]; duplicate {
							return nil, fmt.Errorf("duplicate stage %q in %s", row.Stage, runDir)
						}
						stageRows[row.Stage] = row
					}
				}

				plannedNames := make(map[string]struct{}, len(plannedStages))
				for _, planned := range plannedStages {
					plannedNames[planned.name] = struct{}{}
					row, found := stageRows[planned.name]
					reason := runLevelInvalid
					if stageReason := stageReasons[planned.name]; stageReason != "" {
						reason = appendInvalidReason(reason, stageReason)
					}
					observation := pairedStageObservation{
						Scenario: scenario, Stage: planned.name, Candidate: candidate,
						ManifestSHA256: manifestSHA, Repetition: pair.Repetition, PairPosition: position + 1,
						TargetQPS: planned.qps, DurationMS: plan.StageDurationMS, InvalidReason: reason,
					}
					if planned.name == "recovery" {
						if assessmentErr == nil {
							observation.RecoveryAssessment = formatRecoveryAssessment(assessment)
						} else {
							observation.RecoveryAssessment = formatRecoveryAssessment(recoveryAssessment{
								Status: "indeterminate", Mode: manifest.RecoveryAssessmentMode,
								Reason: "missing or invalid assessment evidence",
							})
						}
					}
					if !found {
						observation.InvalidReason = appendInvalidReason(observation.InvalidReason, "missing stage result")
					} else {
						if row.DurationMS != plan.StageDurationMS || math.Abs(row.TargetQPS-planned.qps) > 1e-9 {
							observation.InvalidReason = appendInvalidReason(observation.InvalidReason, fmt.Sprintf("stage settings mismatch: duration=%d/%d qps=%.6f/%.6f", row.DurationMS, plan.StageDurationMS, row.TargetQPS, planned.qps))
						}
						expectedScenario, expectedTransport, err := expectedWorkloadPath(scenario)
						if err != nil {
							return nil, err
						}
						if row.Scenario != expectedScenario || row.Transport != expectedTransport {
							observation.InvalidReason = appendInvalidReason(observation.InvalidReason, fmt.Sprintf("stage scenario/transport mismatch: got %s/%s want %s/%s", row.Scenario, row.Transport, expectedScenario, expectedTransport))
						}
						if row.RequestDeadlineMS != plan.RequestDeadlineMS || row.LateDrainMS != plan.LateDrainMS {
							observation.InvalidReason = appendInvalidReason(observation.InvalidReason, fmt.Sprintf("stage deadline/drain mismatch: got %d/%d want %d/%d", row.RequestDeadlineMS, row.LateDrainMS, plan.RequestDeadlineMS, plan.LateDrainMS))
						}
						observation.DurationMS = row.DurationMS
						observation.Counters = row.Counters
						observation.P50US = row.P50US
						observation.P95US = row.P95US
						observation.P99US = row.P99US
						observation.LatencySampleCount = len(row.LatencySamplesUS)
					}
					observations = append(observations, observation)
				}
				for stage := range stageRows {
					if _, planned := plannedNames[stage]; !planned {
						return nil, fmt.Errorf("unplanned stage %q in %s", stage, runDir)
					}
				}
			}
		}
	}
	return observations, nil
}

func phase5aPlannedStages(scenario string, plan manifestScenario) ([]pairedPlannedStage, error) {
	if plan.StageDurationMS <= 0 {
		return nil, fmt.Errorf("manifest scenario %s has invalid stage duration", scenario)
	}
	stages := make([]pairedPlannedStage, 0, len(continuousStageSequence)+1)
	if scenario == "w2" {
		stages = append(stages, pairedPlannedStage{name: "official-w2-cold", qps: plan.NormalReferenceQPS, resultPath: filepath.Join("w2-cold", "stages.jsonl")})
	}
	qpsByStage := map[string]float64{
		"normal-reference": plan.NormalReferenceQPS,
		"common-load":      plan.CommonLoadQPS,
		"near-saturation":  plan.NearSaturationQPS,
		"overload":         plan.OverloadQPS,
		"recovery":         plan.NormalReferenceQPS,
	}
	for _, name := range continuousStageSequence {
		qps := qpsByStage[name]
		if qps <= 0 {
			return nil, fmt.Errorf("manifest scenario %s has invalid QPS for %s", scenario, name)
		}
		resultPath := "stages.jsonl"
		if scenario == "w2" {
			if plan.W2WarmLifecycle == "independent-prefilled" {
				resultPath = filepath.Join("w2-warm-independent", name, "stages.jsonl")
			} else {
				resultPath = filepath.Join("w2-warm", "stages.jsonl")
			}
		}
		stages = append(stages, pairedPlannedStage{name: name, qps: qps, resultPath: resultPath})
	}
	if scenario == "w2" && plan.NormalReferenceQPS <= 0 {
		return nil, errors.New("manifest scenario w2 has invalid cold QPS")
	}
	return stages, nil
}

func expectedWorkloadPath(scenario string) (string, string, error) {
	switch scenario {
	case "w1-udp":
		return "w1", "udp", nil
	case "w1-tcp":
		return "w1", "tcp", nil
	case "w2":
		return "w2", "udp", nil
	case "w3":
		return "w3", "udp", nil
	default:
		return "", "", fmt.Errorf("unsupported manifest scenario %q", scenario)
	}
}

func readRunMetadata(path string) (map[string]string, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	metadata := make(map[string]string)
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		key, value, ok := strings.Cut(scanner.Text(), "=")
		if !ok || key == "" {
			return nil, fmt.Errorf("invalid run metadata row in %s", path)
		}
		if _, duplicate := metadata[key]; duplicate {
			return nil, fmt.Errorf("duplicate run metadata key %q in %s", key, path)
		}
		metadata[key] = value
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	return metadata, nil
}

func readInvalidStageReasons(path string) (map[string]string, error) {
	f, err := os.Open(path)
	if errors.Is(err, os.ErrNotExist) {
		return map[string]string{}, nil
	}
	if err != nil {
		return nil, err
	}
	defer f.Close()
	reasons := make(map[string]string)
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		stage, reason, ok := strings.Cut(scanner.Text(), "\t")
		if !ok || stage == "" || reason == "" {
			return nil, fmt.Errorf("invalid invalid-stage row in %s", path)
		}
		if previous := reasons[stage]; previous != "" {
			reasons[stage] = appendInvalidReason(previous, reason)
		} else {
			reasons[stage] = reason
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	return reasons, nil
}

func readStageResultRows(path string) ([]stageResult, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	rows := make([]stageResult, 0)
	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 64*1024), 16*1024*1024)
	lineNumber := 0
	for scanner.Scan() {
		lineNumber++
		if strings.TrimSpace(scanner.Text()) == "" {
			continue
		}
		var row stageResult
		if err := json.Unmarshal(scanner.Bytes(), &row); err != nil {
			return nil, fmt.Errorf("decode %s line %d: %w", path, lineNumber, err)
		}
		if row.Stage == "" {
			return nil, fmt.Errorf("stage result row %d in %s has no stage name", lineNumber, path)
		}
		rows = append(rows, row)
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	return rows, nil
}

func readAttemptExitStatus(path string) (int, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return 0, err
	}
	value := strings.TrimSpace(string(data))
	value = strings.TrimPrefix(value, "exit=")
	status, err := strconv.Atoi(value)
	if err != nil {
		return 0, fmt.Errorf("invalid attempt exit status in %s: %w", path, err)
	}
	return status, nil
}

func readManifestSHAFile(path string) (string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	fields := strings.Fields(string(data))
	if len(fields) < 2 || !validSHA256(fields[0]) {
		return "", fmt.Errorf("invalid manifest SHA record in %s", path)
	}
	return fields[0], nil
}

func readSmallTextFile(path string) (string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(data)), nil
}

func readServiceRecoveryAssessment(path, expectedMode string) (recoveryAssessment, error) {
	fields, err := readRunMetadata(path)
	if err != nil {
		return recoveryAssessment{}, err
	}
	assessment := recoveryAssessment{Status: fields["status"], Mode: fields["mode"], Reason: fields["reason"]}
	if assessment.Status != "indeterminate" || assessment.Mode != expectedMode || assessment.Reason == "" {
		return recoveryAssessment{}, fmt.Errorf("unexpected status/mode/reason: status=%q mode=%q reason=%q", assessment.Status, assessment.Mode, assessment.Reason)
	}
	return assessment, nil
}

func formatRecoveryAssessment(assessment recoveryAssessment) string {
	return fmt.Sprintf("status=%s; mode=%s; reason=%s", assessment.Status, assessment.Mode, assessment.Reason)
}

func verifyRunInputHashes(path string, manifest officialManifest, candidateName string) (string, error) {
	f, err := os.Open(path)
	if errors.Is(err, os.ErrNotExist) {
		return "missing per-run frozen input hash list", nil
	}
	if err != nil {
		return "", err
	}
	defer f.Close()
	candidate, ok := manifest.Candidates[candidateName]
	if !ok {
		return "", fmt.Errorf("frozen manifest has no candidate %q", candidateName)
	}
	expected := make(map[string]struct{}, len(manifest.Inputs)+4)
	for _, input := range manifest.Inputs {
		expected[input.SHA256] = struct{}{}
	}
	expected[manifest.Runner.SHA256] = struct{}{}
	expected[manifest.Helper.SourceSHA256] = struct{}{}
	expected[manifest.Helper.BinarySHA256] = struct{}{}
	expected[candidate.BinarySHA256] = struct{}{}
	for digest := range expected {
		if !validSHA256(digest) {
			return "", fmt.Errorf("frozen manifest contains invalid hash %q", digest)
		}
	}

	seen := make(map[string]struct{}, len(expected))
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		fields := strings.Fields(scanner.Text())
		if len(fields) < 2 || !validSHA256(fields[0]) {
			return "", fmt.Errorf("invalid per-run hash row in %s", path)
		}
		seen[fields[0]] = struct{}{}
	}
	if err := scanner.Err(); err != nil {
		return "", err
	}
	for digest := range expected {
		if _, ok := seen[digest]; !ok {
			return "per-run hashes do not contain every frozen input, tool, and candidate binary", nil
		}
	}
	for digest := range seen {
		if _, ok := expected[digest]; !ok {
			return "per-run hashes contain an artifact absent from the frozen manifest", nil
		}
	}
	return "", nil
}

func hasInvalidReason(reasons map[string]string) bool {
	for _, reason := range reasons {
		if reason != "" {
			return true
		}
	}
	return false
}

func expandInvalidStageReasons(scenario string, planned []pairedPlannedStage, input map[string]string) (map[string]string, error) {
	plannedNames := make(map[string]struct{}, len(planned))
	for _, stage := range planned {
		plannedNames[stage.name] = struct{}{}
	}
	result := make(map[string]string)
	for name, reason := range input {
		if _, ok := plannedNames[name]; ok {
			result[name] = appendInvalidReason(result[name], reason)
			continue
		}
		var targets []string
		switch {
		case scenario == "w2" && name == "warm-prefill":
			for _, stage := range continuousStageSequence {
				targets = append(targets, stage)
			}
		case scenario == "w2" && strings.HasSuffix(name, "-prefill"):
			stage := strings.TrimSuffix(name, "-prefill")
			if !containsString(continuousStageSequence, stage) {
				return nil, fmt.Errorf("unrecognized W2 prefill stage %q", name)
			}
			targets = append(targets, stage)
		case scenario == "w2" && name == "w2-warm":
			for _, stage := range continuousStageSequence {
				targets = append(targets, stage)
			}
		case scenario == "w2" && name == "w2-cold":
			targets = append(targets, "official-w2-cold")
		case (strings.HasPrefix(scenario, "w1-") || scenario == "w3") && (name == "counters" || (scenario == "w3" && name == "events")):
			targets = append(targets, continuousStageSequence...)
		default:
			return nil, fmt.Errorf("unrecognized invalid stage %q for scenario %s", name, scenario)
		}
		for _, stage := range targets {
			result[stage] = appendInvalidReason(result[stage], name+": "+reason)
		}
	}
	return result, nil
}

func appendInvalidReason(existing, next string) string {
	if next == "" {
		return existing
	}
	if existing == "" {
		return next
	}
	return existing + "; " + next
}

func aggregatePairedStagesCommand(args []string) error {
	fs := flag.NewFlagSet("aggregate-pairs", flag.ContinueOnError)
	manifestPath := fs.String("manifest", "", "frozen official manifest JSON")
	manifestSHA := fs.String("manifest-sha256", "", "expected SHA-256 of the frozen manifest")
	resultsRoot := fs.String("results-root", "", "task-owned official results directory")
	outputPath := fs.String("output", "-", "aggregate JSON output path, or - for stdout")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *manifestPath == "" || *manifestSHA == "" || *resultsRoot == "" || *outputPath == "" {
		return errors.New("aggregate-pairs requires --manifest, --manifest-sha256, --results-root, and --output")
	}
	actualSHA, err := sha256File(*manifestPath)
	if err != nil {
		return err
	}
	if actualSHA != *manifestSHA {
		return fmt.Errorf("manifest SHA mismatch: expected=%s actual=%s", *manifestSHA, actualSHA)
	}
	manifestBytes, err := os.ReadFile(*manifestPath)
	if err != nil {
		return fmt.Errorf("read frozen manifest: %w", err)
	}
	var manifest officialManifest
	if err := json.Unmarshal(manifestBytes, &manifest); err != nil {
		return fmt.Errorf("decode frozen manifest: %w", err)
	}
	if !manifest.OfficialFrozen {
		return errors.New("aggregate-pairs requires an official-frozen manifest")
	}

	observations, err := collectPairedStageObservations(*resultsRoot, manifest, *manifestSHA)
	if err != nil {
		return err
	}
	groups, err := aggregatePairedStages(observations, manifest.PairSchedule, *manifestSHA)
	if err != nil {
		return err
	}

	var output io.Writer = os.Stdout
	var outputFile *os.File
	if *outputPath != "-" {
		outputFile, err = os.OpenFile(*outputPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0644)
		if err != nil {
			return fmt.Errorf("create paired aggregate output without overwrite: %w", err)
		}
		defer outputFile.Close()
		output = outputFile
	}
	return writeJSON(output, pairedAggregationReport{ManifestSHA256: *manifestSHA, Groups: groups})
}

func containsString(values []string, target string) bool {
	for _, value := range values {
		if value == target {
			return true
		}
	}
	return false
}

func equalStrings(left, right []string) bool {
	if len(left) != len(right) {
		return false
	}
	for i := range left {
		if left[i] != right[i] {
			return false
		}
	}
	return true
}

type fixtureOptions struct {
	network     string
	addr        string
	upstreamID  string
	counterPath string
	eventPath   string
	delayMS     int
}

func runFixture(args []string) error {
	fs := flag.NewFlagSet("fixture", flag.ContinueOnError)
	opts := fixtureOptions{}
	fs.StringVar(&opts.network, "network", "udp", "udp or tcp")
	fs.StringVar(&opts.addr, "addr", "", "listen address")
	fs.StringVar(&opts.upstreamID, "upstream-id", "forward", "committed fixture identity")
	fs.StringVar(&opts.counterPath, "counter", "", "counter JSON path")
	fs.StringVar(&opts.eventPath, "event-journal", "", "shared JSONL route-event journal")
	fs.IntVar(&opts.delayMS, "delay-ms", 0, "fixed response delay")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if opts.addr == "" || opts.counterPath == "" {
		return errors.New("fixture requires --addr and --counter")
	}
	if opts.network != "udp" && opts.network != "tcp" {
		return fmt.Errorf("unsupported fixture network %q", opts.network)
	}
	if opts.delayMS < 0 {
		return errors.New("--delay-ms must be non-negative")
	}

	counts := &counterStore{upstream: opts.upstreamID, path: opts.counterPath, values: make(map[string]int64)}
	if err := counts.write(); err != nil {
		return err
	}
	var events *fixtureEventJournal
	if opts.eventPath != "" {
		events = newFixtureEventJournal(opts.eventPath)
	}
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGINT, syscall.SIGTERM)
	defer signal.Stop(stop)

	var wg sync.WaitGroup
	var connectionWG sync.WaitGroup
	var closeFixture func()
	if opts.network == "udp" {
		conn, err := net.ListenUDP("udp", mustResolveUDP(opts.addr))
		if err != nil {
			return fmt.Errorf("listen UDP fixture: %w", err)
		}
		closeFixture = func() { _ = conn.Close() }
		wg.Add(1)
		go func() {
			defer wg.Done()
			serveUDPFixture(conn, opts, counts, events)
		}()
	} else {
		listener, err := net.Listen("tcp", opts.addr)
		if err != nil {
			return fmt.Errorf("listen TCP fixture: %w", err)
		}
		closeFixture = func() { _ = listener.Close() }
		wg.Add(1)
		go func() {
			defer wg.Done()
			serveTCPFixture(listener, opts, counts, events, &connectionWG)
		}()
	}

	writerStop := make(chan struct{})
	var writerWG sync.WaitGroup
	writerWG.Add(1)
	go func() {
		defer writerWG.Done()
		ticker := time.NewTicker(100 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-writerStop:
				return
			case <-ticker.C:
				_ = counts.write()
			}
		}
	}()
	<-stop
	closeFixture()
	wg.Wait()
	connectionWG.Wait()
	close(writerStop)
	writerWG.Wait()
	return counts.write()
}

func mustResolveUDP(addr string) *net.UDPAddr {
	a, err := net.ResolveUDPAddr("udp", addr)
	if err != nil {
		panic(err)
	}
	return a
}

type counterStore struct {
	mu       sync.Mutex
	writeMu  sync.Mutex
	upstream string
	path     string
	values   map[string]int64
}

func (c *counterStore) add(qname string, qtype uint16) {
	c.mu.Lock()
	c.values[strings.ToLower(dns.Fqdn(qname))+"|"+dns.TypeToString[qtype]]++
	c.mu.Unlock()
}

func (c *counterStore) write() error {
	c.writeMu.Lock()
	defer c.writeMu.Unlock()
	c.mu.Lock()
	values := make(map[string]int64, len(c.values))
	for k, v := range c.values {
		values[k] = v
	}
	c.mu.Unlock()
	payload := map[string]any{"upstream": c.upstream, "counts": values, "updated_at": time.Now().UTC()}
	tmp := c.path + ".tmp"
	f, err := os.Create(tmp)
	if err != nil {
		return err
	}
	encErr := json.NewEncoder(f).Encode(payload)
	closeErr := f.Close()
	if encErr != nil {
		return encErr
	}
	if closeErr != nil {
		return closeErr
	}
	return os.Rename(tmp, c.path)
}

func serveUDPFixture(conn *net.UDPConn, opts fixtureOptions, counts *counterStore, events *fixtureEventJournal) {
	buf := make([]byte, dns.MaxMsgSize)
	for {
		n, remote, err := conn.ReadFromUDP(buf)
		if err != nil {
			return
		}
		resp, ok := fixtureResponse(buf[:n], opts, counts, events)
		if !ok {
			continue
		}
		if opts.delayMS > 0 {
			time.Sleep(time.Duration(opts.delayMS) * time.Millisecond)
		}
		_, _ = conn.WriteToUDP(resp, remote)
	}
}

func serveTCPFixture(listener net.Listener, opts fixtureOptions, counts *counterStore, events *fixtureEventJournal, connectionWG *sync.WaitGroup) {
	for {
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		connectionWG.Add(1)
		go func() {
			defer connectionWG.Done()
			defer conn.Close()
			var length uint16
			if err := binary.Read(conn, binary.BigEndian, &length); err != nil || length == 0 || length > dns.MaxMsgSize {
				return
			}
			query := make([]byte, length)
			if _, err := io.ReadFull(conn, query); err != nil {
				return
			}
			resp, ok := fixtureResponse(query, opts, counts, events)
			if !ok {
				return
			}
			if opts.delayMS > 0 {
				time.Sleep(time.Duration(opts.delayMS) * time.Millisecond)
			}
			if len(resp) > int(^uint16(0)) {
				return
			}
			_ = binary.Write(conn, binary.BigEndian, uint16(len(resp)))
			_, _ = conn.Write(resp)
		}()
	}
}

func fixtureResponse(wire []byte, opts fixtureOptions, counts *counterStore, events *fixtureEventJournal) ([]byte, bool) {
	query := new(dns.Msg)
	if err := query.Unpack(wire); err != nil || len(query.Question) != 1 {
		return nil, false
	}
	question := query.Question[0]
	if events != nil {
		_, err := events.append(fixtureEvent{
			DNSID: query.Id, QName: strings.ToLower(dns.Fqdn(question.Name)),
			QType: question.Qtype, QClass: question.Qclass, Upstream: opts.upstreamID,
		})
		if err != nil {
			fmt.Fprintf(os.Stderr, "write fixture event: %v\n", err)
			return nil, false
		}
	}
	counts.add(question.Name, question.Qtype)
	response := new(dns.Msg)
	response.SetReply(query)
	response.RecursionAvailable = true
	qname := strings.ToLower(dns.Fqdn(question.Name))
	answer := fixtureAnswer(opts.upstreamID, qname, question.Qtype)
	if answer.rcode != dns.RcodeSuccess {
		response.Rcode = answer.rcode
	} else if question.Qtype == dns.TypeA && answer.ip != "" {
		ip := net.ParseIP(answer.ip).To4()
		if ip == nil {
			response.Rcode = dns.RcodeServerFailure
		} else {
			response.Answer = append(response.Answer, &dns.A{Hdr: dns.RR_Header{Name: question.Name, Rrtype: dns.TypeA, Class: dns.ClassINET, Ttl: 30}, A: ip})
		}
	} else {
		response.Rcode = dns.RcodeNameError
	}
	packed, err := response.Pack()
	return packed, err == nil
}

type fixtureAnswerValue struct {
	rcode int
	ip    string
}

func fixtureAnswer(upstreamID, qname string, qtype uint16) fixtureAnswerValue {
	if qtype != dns.TypeA {
		return fixtureAnswerValue{rcode: dns.RcodeNameError}
	}
	switch upstreamID {
	case "forward":
		if strings.HasPrefix(qname, "negative.") {
			return fixtureAnswerValue{rcode: dns.RcodeNameError}
		}
		return fixtureAnswerValue{ip: "198.51.100.10"}
	case "cache":
		switch qname {
		case "cache-a.test.":
			return fixtureAnswerValue{ip: "198.51.100.20"}
		case "cache-b.test.":
			return fixtureAnswerValue{ip: "198.51.100.21"}
		}
	case "route-a":
		return fixtureAnswerValue{ip: "192.0.2.11"}
	case "route-b":
		if qname == "ip-hit.test." {
			return fixtureAnswerValue{ip: "192.0.2.10"}
		}
		if qname == "ip-miss.test." {
			return fixtureAnswerValue{ip: "192.0.2.30"}
		}
	case "route-c":
		return fixtureAnswerValue{ip: "192.0.2.12"}
	}
	return fixtureAnswerValue{rcode: dns.RcodeNameError}
}

func runStage(args []string) error {
	fs := flag.NewFlagSet("run", flag.ContinueOnError)
	workloadPath := fs.String("workload", "", "JSONL workload")
	addr := fs.String("addr", "", "SUT address")
	transport := fs.String("transport", "udp", "udp or tcp")
	scenario := fs.String("scenario", "", "scenario name")
	stageName := fs.String("stage", "smoke", "stage name")
	qps := fs.Float64("qps", 10, "fixed offered QPS")
	duration := fs.Duration("duration", time.Second, "stage duration")
	deadline := fs.Duration("deadline", defaultDeadline, "per-request deadline")
	lateDrain := fs.Duration("late-drain", defaultLateDrain, "bounded late response drain")
	resultDir := fs.String("result", "", "result directory")
	sutPID := fs.Int("sut-pid", 0, "SUT PID for Linux resource sampling")
	runID := fs.String("run-id", "", "shared identity for stages in one SUT session")
	fixtureSessionID := fs.String("fixture-session-id", "", "shared identity for fixture processes in one run")
	ledgerPath := fs.String("request-ledger", "", "JSONL path for per-request observations")
	eventJournalPath := fs.String("event-journal", "", "JSONL route-event journal for stage barriers")
	var fixturePIDArgs stringSliceFlag
	fs.Var(&fixturePIDArgs, "fixture-pid", "fixture PID to sample; may be repeated")
	failOnError := fs.Bool("fail-on-error", false, "return non-zero when a stage has a correctness or sender failure")
	onePass := fs.Bool("one-pass", false, "send exactly one deterministic pass over the filtered workload")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *workloadPath == "" || *addr == "" || *resultDir == "" {
		return errors.New("run requires --workload, --addr, and --result")
	}
	if *transport != "udp" && *transport != "tcp" {
		return fmt.Errorf("unsupported transport %q", *transport)
	}
	if *qps <= 0 || *duration <= 0 || *deadline <= 0 || *lateDrain < 0 {
		return errors.New("qps, duration, and deadline must be positive; late-drain cannot be negative")
	}
	if err := os.MkdirAll(*resultDir, 0755); err != nil {
		return err
	}
	cases, err := readWorkload(*workloadPath, *scenario, *transport)
	if err != nil {
		return err
	}
	if len(cases) == 0 {
		return errors.New("workload has no matching cases")
	}
	if *ledgerPath == "" {
		*ledgerPath = filepath.Join(*resultDir, "request-ledger.jsonl")
	}
	if *runID == "" {
		*runID = fmt.Sprintf("%s-%d", *stageName, time.Now().UnixNano())
	}
	if *fixtureSessionID == "" {
		*fixtureSessionID = *runID
	}
	fixtureTargets := make([]resourceTarget, 0, len(fixturePIDArgs))
	seenFixturePIDs := make(map[int]struct{}, len(fixturePIDArgs))
	for i, value := range fixturePIDArgs {
		pid, err := strconv.Atoi(value)
		if err != nil || pid <= 0 {
			return fmt.Errorf("invalid fixture PID %q", value)
		}
		if _, exists := seenFixturePIDs[pid]; exists {
			return fmt.Errorf("duplicate fixture PID %d", pid)
		}
		seenFixturePIDs[pid] = struct{}{}
		fixtureTargets = append(fixtureTargets, resourceTarget{Role: fmt.Sprintf("fixture-%d", i+1), PID: pid})
	}
	return executeStage(stageOptions{
		workload:         cases,
		addr:             *addr,
		transport:        *transport,
		scenario:         *scenario,
		stage:            *stageName,
		qps:              *qps,
		duration:         *duration,
		deadline:         *deadline,
		lateDrain:        *lateDrain,
		resultDir:        *resultDir,
		sutPID:           *sutPID,
		runID:            *runID,
		fixtureSessionID: *fixtureSessionID,
		ledgerPath:       *ledgerPath,
		eventJournalPath: *eventJournalPath,
		fixtureTargets:   fixtureTargets,
		failOnError:      *failOnError,
		onePass:          *onePass,
	})
}

func readWorkload(path, scenario, transport string) ([]workloadCase, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	var expanded []workloadCase
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		var c workloadCase
		if err := json.Unmarshal([]byte(line), &c); err != nil {
			return nil, fmt.Errorf("decode workload line: %w", err)
		}
		if scenario != "" && c.Scenario != scenario {
			continue
		}
		if transport != "" && c.Transport != transport {
			continue
		}
		if c.CaseID == "" || c.QName == "" || c.QType == "" || c.Weight <= 0 {
			return nil, fmt.Errorf("invalid workload case %q", c.CaseID)
		}
		if c.RequestDeadlineMS <= 0 {
			c.RequestDeadlineMS = int(defaultDeadline / time.Millisecond)
		}
		for i := 0; i < c.Weight; i++ {
			expanded = append(expanded, c)
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	return expanded, nil
}

type counterFile struct {
	Upstream string           `json:"upstream"`
	Counts   map[string]int64 `json:"counts"`
}

func verifyCounters(args []string) error {
	fs := flag.NewFlagSet("verify-counters", flag.ContinueOnError)
	scenario := fs.String("scenario", "", "w1, w2, or w3")
	workload := fs.String("workload", "", "fixed workload JSONL")
	counter := fs.String("counter", "", "single fixture counter JSON")
	eventJournalPath := fs.String("event-journal", "", "complete W3 fixture event journal JSONL")
	baseline := fs.String("baseline", "", "prefill counter JSON for warm-cache equality")
	expectDelta := fs.Bool("expect-delta", false, "require the exact cold-miss or forwarding delta for the measured stage")
	stageResultPath := fs.String("stage-result", "", "stage JSONL used to derive expected per-case request counts")
	stageName := fs.String("stage", "", "stage name within --stage-result")
	routeA := fs.String("route-a", "", "route-a counter JSON")
	routeB := fs.String("route-b", "", "route-b counter JSON")
	routeC := fs.String("route-c", "", "route-c counter JSON")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *scenario == "" || *workload == "" {
		return errors.New("verify-counters requires --scenario and --workload")
	}
	cases, err := readWorkload(*workload, *scenario, "")
	if err != nil {
		return err
	}
	if len(cases) == 0 {
		return errors.New("verify-counters workload has no cases")
	}
	if *scenario == "w3" {
		if *routeA == "" || *routeB == "" || *routeC == "" || *eventJournalPath == "" {
			return errors.New("w3 counter verification requires route-a, route-b, route-c, and event-journal")
		}
		events, err := readFixtureEvents(*eventJournalPath)
		if err != nil {
			return err
		}
		return verifyRoutingCountersWithEvents(cases, events, *routeA, *routeB, *routeC)
	}
	if *counter == "" {
		return errors.New("counter path is required for w1/w2")
	}
	if *expectDelta && *baseline == "" {
		return errors.New("--expect-delta requires --baseline")
	}
	actual, err := readCounterFile(*counter)
	if err != nil {
		return err
	}
	expected := expectedCounterDeltas(cases, *scenario)
	if *stageResultPath != "" {
		measuredStage, err := readStageResult(*stageResultPath, *stageName)
		if err != nil {
			return err
		}
		expected = expectedCounterDeltasFromStage(cases, measuredStage, *scenario)
	}
	if *baseline != "" {
		before, err := readCounterFile(*baseline)
		if err != nil {
			return err
		}
		keys := make(map[string]struct{}, len(expected)+len(actual.Counts)+len(before.Counts))
		for key := range expected {
			keys[key] = struct{}{}
		}
		for key := range actual.Counts {
			keys[key] = struct{}{}
		}
		for key := range before.Counts {
			keys[key] = struct{}{}
		}
		for key := range keys {
			want := expected[key]
			delta := actual.Counts[key] - before.Counts[key]
			if *expectDelta {
				if delta != want {
					return fmt.Errorf("counter delta mismatch for %s: expected=%d actual=%d", key, want, delta)
				}
			} else if *scenario == "w2" && delta != 0 {
				return fmt.Errorf("warm cache miss for %s: prefill=%d final=%d", key, before.Counts[key], actual.Counts[key])
			}
		}
		return nil
	}
	for key := range expected {
		if actual.Counts[key] <= 0 {
			return fmt.Errorf("fixture counter missing %s", key)
		}
	}
	return nil
}

func verifyRoutingEventsCommand(args []string) error {
	fs := flag.NewFlagSet("verify-routing-events", flag.ContinueOnError)
	workload := fs.String("workload", "", "frozen W3 workload JSONL")
	requestLedgerPath := fs.String("request-ledger", "", "session request-ledger JSONL")
	eventJournalPath := fs.String("event-journal", "", "session fixture-event JSONL")
	stageResultPath := fs.String("stage-result", "", "stage-results JSONL")
	stageName := fs.String("stage", "", "stage name to validate")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *workload == "" || *requestLedgerPath == "" || *eventJournalPath == "" || *stageResultPath == "" || *stageName == "" {
		return errors.New("verify-routing-events requires --workload, --request-ledger, --event-journal, --stage-result, and --stage")
	}
	cases, err := readWorkload(*workload, "w3", "")
	if err != nil {
		return err
	}
	requests, err := readRequestLedger(*requestLedgerPath)
	if err != nil {
		return err
	}
	events, err := readFixtureEvents(*eventJournalPath)
	if err != nil {
		return err
	}
	stage, err := readStageResult(*stageResultPath, *stageName)
	if err != nil {
		return err
	}
	return verifyRoutingEvents(cases, requests, events, stage)
}

type stringSliceFlag []string

func (s *stringSliceFlag) String() string {
	return strings.Join(*s, ",")
}

func (s *stringSliceFlag) Set(value string) error {
	if value == "" {
		return errors.New("value cannot be empty")
	}
	*s = append(*s, value)
	return nil
}

func verifyWarmTTLCommand(args []string) error {
	fs := flag.NewFlagSet("verify-warm-ttl", flag.ContinueOnError)
	workload := fs.String("workload", "", "frozen W2 workload JSONL")
	requestLedgerPath := fs.String("request-ledger", "", "session request-ledger JSONL")
	prefillStage := fs.String("prefill-stage", "warm-prefill", "stage name that prefills the cache")
	ttl := fs.Duration("ttl", 30*time.Second, "upstream answer TTL")
	safetyMargin := fs.Duration("safety-margin", 500*time.Millisecond, "frozen TTL safety margin")
	var warmStages stringSliceFlag
	fs.Var(&warmStages, "warm-stage", "measured warm stage to validate; may be repeated")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *workload == "" || *requestLedgerPath == "" || len(warmStages) == 0 {
		return errors.New("verify-warm-ttl requires --workload, --request-ledger, and at least one --warm-stage")
	}
	cases, err := readWorkload(*workload, "w2", "")
	if err != nil {
		return err
	}
	records, err := readRequestLedger(*requestLedgerPath)
	if err != nil {
		return err
	}
	var prefill []requestRecord
	for _, record := range records {
		if record.StageID == *prefillStage {
			prefill = append(prefill, record)
		}
	}
	if len(prefill) == 0 {
		return fmt.Errorf("no prefill records found for stage %q", *prefillStage)
	}
	for _, stageName := range warmStages {
		var warm []requestRecord
		for _, record := range records {
			if record.StageID == stageName {
				warm = append(warm, record)
			}
		}
		if len(warm) == 0 {
			return fmt.Errorf("no warm records found for stage %q", stageName)
		}
		if err := verifyWarmTTL(cases, prefill, warm, *ttl, *safetyMargin); err != nil {
			return fmt.Errorf("warm stage %s: %w", stageName, err)
		}
	}
	fmt.Fprintf(os.Stdout, "W2 warm TTL verified for %d stage(s) with ttl=%s safety_margin=%s\n", len(warmStages), *ttl, *safetyMargin)
	return nil
}

func verifyContinuousCommand(args []string) error {
	fs := flag.NewFlagSet("verify-continuous", flag.ContinueOnError)
	stagePath := fs.String("stage-result", "", "JSONL stage results for one process session")
	runID := fs.String("run-id", "", "run identity shared by the staged session")
	minimumSamples := fs.Int("minimum-samples", 0, "minimum correct latency samples in reference and recovery")
	p95Ceiling := fs.Int64("p95-ceiling-us", 0, "frozen recovery p95 ceiling")
	p99Ceiling := fs.Int64("p99-ceiling-us", 0, "frozen recovery p99 ceiling")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *stagePath == "" || *runID == "" || *minimumSamples <= 0 || *p95Ceiling <= 0 || *p99Ceiling < *p95Ceiling {
		return errors.New("verify-continuous requires stage-result, run-id, minimum-samples, and valid p95/p99 ceilings")
	}
	stages, err := readStageResults(*stagePath, *runID)
	if err != nil {
		return err
	}
	criteria := recoveryCriteria{MinimumSamples: *minimumSamples, P95CeilingUS: *p95Ceiling, P99CeilingUS: *p99Ceiling}
	assessment, err := assessServiceRecovery(stages, criteria)
	fmt.Fprintf(os.Stdout, "status=%s\nmode=%s\nreason=%s\n", assessment.Status, assessment.Mode, assessment.Reason)
	return err
}

func assessServiceRecovery(stages []stageResult, criteria recoveryCriteria) (recoveryAssessment, error) {
	if err := verifyContinuousStages(stages, criteria); err != nil {
		return recoveryAssessment{
			Status: "indeterminate",
			Mode:   recoveryAssessmentMode,
			Reason: "terminal health check failed; service recovery remains indeterminate: " + err.Error(),
		}, err
	}
	return recoveryAssessment{
		Status: "indeterminate",
		Mode:   recoveryAssessmentMode,
		Reason: "the frozen ladder has no objective overload-evidence criterion; the final same-rate stage is a post-sequence health check, not proof of recovery",
	}, nil
}

func readStageResults(path, runID string) ([]stageResult, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, fmt.Errorf("open stage results %s: %w", path, err)
	}
	defer f.Close()
	var stages []stageResult
	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 4096), 4*1024*1024)
	for scanner.Scan() {
		if strings.TrimSpace(scanner.Text()) == "" {
			continue
		}
		var stage stageResult
		if err := json.Unmarshal(scanner.Bytes(), &stage); err != nil {
			return nil, fmt.Errorf("decode stage result: %w", err)
		}
		if stage.RunID == runID {
			stages = append(stages, stage)
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("read stage results: %w", err)
	}
	return stages, nil
}

func verifyContinuousStages(stages []stageResult, criteria recoveryCriteria) error {
	if len(stages) != len(continuousStageSequence) {
		return fmt.Errorf("continuous session has %d stages, want %d", len(stages), len(continuousStageSequence))
	}
	if criteria.MinimumSamples <= 0 || criteria.P95CeilingUS <= 0 || criteria.P99CeilingUS < criteria.P95CeilingUS {
		return errors.New("recovery criteria are invalid")
	}
	first := stages[0]
	if first.RunID == "" || first.FixtureSessionID == "" || first.SUTPID <= 0 || first.SUTStartIdentity == "" || first.SUTCPUSet == "" || first.HarnessPID <= 0 || first.HarnessCPUSet == "" {
		return errors.New("continuous session lacks run, fixture, process-start, or observed CPU-affinity identity")
	}
	for i, stage := range stages {
		if err := verifySenderStage(stage); err != nil {
			return fmt.Errorf("continuous stage %s: %w", stage.Stage, err)
		}
		if stage.Stage != continuousStageSequence[i] {
			return fmt.Errorf("continuous stage order mismatch at %d: got=%q want=%q", i, stage.Stage, continuousStageSequence[i])
		}
		if stage.RunID != first.RunID || stage.FixtureSessionID != first.FixtureSessionID || stage.SUTPID != first.SUTPID || stage.SUTStartIdentity != first.SUTStartIdentity || stage.SUTCPUSet != first.SUTCPUSet || stage.Scenario != first.Scenario || stage.Transport != first.Transport {
			return fmt.Errorf("stage %s does not belong to the same SUT/fixture process session", stage.Stage)
		}
		if stage.HarnessCPUSet != first.HarnessCPUSet || stage.HarnessCPUSet == "" {
			return fmt.Errorf("stage %s has missing or changed harness CPU affinity", stage.Stage)
		}
		if stage.StartedAt.IsZero() || stage.FinishedAt.IsZero() || !stage.FinishedAt.After(stage.StartedAt) {
			return fmt.Errorf("stage %s has invalid start/finish timestamps", stage.Stage)
		}
		if i > 0 {
			previous := stages[i-1]
			if stage.StartedAt.Before(previous.FinishedAt) {
				return fmt.Errorf("stage %s overlaps previous stage %s", stage.Stage, previous.Stage)
			}
			if stage.RequestSeqStart != previous.RequestSeqEnd+1 {
				return fmt.Errorf("request sequence discontinuity before %s: previous_end=%d next_start=%d", stage.Stage, previous.RequestSeqEnd, stage.RequestSeqStart)
			}
			if stage.FixtureSeqStart != previous.FixtureSeqEnd {
				return fmt.Errorf("fixture sequence discontinuity before %s: previous_end=%d next_start=%d", stage.Stage, previous.FixtureSeqEnd, stage.FixtureSeqStart)
			}
		}
		if stage.RequestSeqStart == 0 || stage.RequestSeqEnd < stage.RequestSeqStart-1 {
			return fmt.Errorf("stage %s has invalid request sequence range", stage.Stage)
		}
		if stage.TargetQPS <= 0 || stage.DurationMS <= 0 {
			return fmt.Errorf("stage %s has invalid rate/duration", stage.Stage)
		}
	}
	if !(stages[0].TargetQPS < stages[1].TargetQPS && stages[1].TargetQPS < stages[2].TargetQPS && stages[2].TargetQPS < stages[3].TargetQPS) {
		return errors.New("normal/common/near/overload offered rates must strictly increase")
	}
	if math.Abs(stages[4].TargetQPS-stages[0].TargetQPS) > 1e-9 {
		return errors.New("recovery offered rate differs from normal-reference rate")
	}
	for _, index := range []int{0, 4} {
		stage := stages[index]
		if hasStageFailure(stage.Counters) || stage.Counters.CorrectOnTime != stage.Counters.Scheduled || stage.Counters.CorrectOnTime != stage.Counters.Sent || stage.Counters.CorrectOnTime != stage.Counters.Received {
			return fmt.Errorf("stage %s is not fully correct-on-time: %+v", stage.Stage, stage.Counters)
		}
		if len(stage.LatencySamplesUS) < criteria.MinimumSamples {
			return fmt.Errorf("stage %s has %d latency samples, minimum is %d", stage.Stage, len(stage.LatencySamplesUS), criteria.MinimumSamples)
		}
		samples := append([]int64(nil), stage.LatencySamplesUS...)
		sort.Slice(samples, func(i, j int) bool { return samples[i] < samples[j] })
		p95, p99 := percentile(samples, .95), percentile(samples, .99)
		if stage.P95US != p95 || stage.P99US != p99 {
			return fmt.Errorf("stage %s percentile summaries do not match latency samples", stage.Stage)
		}
		if p95 > criteria.P95CeilingUS || p99 > criteria.P99CeilingUS {
			return fmt.Errorf("stage %s exceeds frozen recovery latency band: p95=%d/%d p99=%d/%d", stage.Stage, p95, criteria.P95CeilingUS, p99, criteria.P99CeilingUS)
		}
	}
	return nil
}

func verifyStageCommand(args []string) error {
	fs := flag.NewFlagSet("verify-stage", flag.ContinueOnError)
	stagePath := fs.String("stage-result", "", "JSONL stage results")
	stageName := fs.String("stage", "", "stage name to verify")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *stagePath == "" || *stageName == "" {
		return errors.New("verify-stage requires --stage-result and --stage")
	}
	stage, err := readStageResult(*stagePath, *stageName)
	if err != nil {
		return err
	}
	return verifyCleanStage(stage)
}

func verifyCleanStage(stage stageResult) error {
	if stage.Counters.CorrectOnTime == 0 || stage.Counters.CorrectOnTime != stage.Counters.Scheduled || stage.Counters.CorrectOnTime != stage.Counters.Sent || stage.Counters.CorrectOnTime != stage.Counters.Received || hasStageFailure(stage.Counters) {
		return fmt.Errorf("stage %s is not fully correct-on-time: %+v", stage.Stage, stage.Counters)
	}
	return nil
}

func verifySenderStage(stage stageResult) error {
	if stage.Counters.Scheduled <= 0 {
		return fmt.Errorf("stage %s has no scheduled queries", stage.Stage)
	}
	if stage.Counters.SenderShortfall != 0 {
		return fmt.Errorf("stage %s dropped %d open-loop schedule slot(s), max_lag_us=%d", stage.Stage, stage.Counters.SenderShortfall, stage.SenderLagMaxUS)
	}
	if stage.Counters.Sent != stage.Counters.Scheduled {
		return fmt.Errorf("stage %s did not send every scheduled query: scheduled=%d sent=%d", stage.Stage, stage.Counters.Scheduled, stage.Counters.Sent)
	}
	return nil
}

func verifySenderCommand(args []string) error {
	fs := flag.NewFlagSet("verify-sender", flag.ContinueOnError)
	stagePath := fs.String("stage-result", "", "stage JSONL to check for sender shortfall")
	stageName := fs.String("stage", "", "stage name to validate")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *stagePath == "" || *stageName == "" {
		return errors.New("verify-sender requires stage-result and stage")
	}
	stage, err := readStageResult(*stagePath, *stageName)
	if err != nil {
		return err
	}
	if err := verifySenderStage(stage); err != nil {
		return err
	}
	fmt.Fprintf(os.Stdout, "sender schedule verified: stage=%s scheduled=%d sent=%d\n", stage.Stage, stage.Counters.Scheduled, stage.Counters.Sent)
	return nil
}

func verifySessionCountersCommand(args []string) error {
	fs := flag.NewFlagSet("verify-session-counters", flag.ContinueOnError)
	scenario := fs.String("scenario", "", "w1 or w2")
	workloadPath := fs.String("workload", "", "frozen workload JSONL")
	counterPath := fs.String("counter", "", "final fixture counter JSON")
	stagePath := fs.String("stage-result", "", "session stage results JSONL")
	runID := fs.String("run-id", "", "session run identity")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *scenario == "" || *workloadPath == "" || *counterPath == "" || *stagePath == "" || *runID == "" {
		return errors.New("verify-session-counters requires scenario, workload, counter, stage-result, and run-id")
	}
	if *scenario != "w1" && *scenario != "w2" {
		return fmt.Errorf("unsupported session counter scenario %q", *scenario)
	}
	cases, err := readWorkload(*workloadPath, *scenario, "")
	if err != nil {
		return err
	}
	stages, err := readStageResults(*stagePath, *runID)
	if err != nil {
		return err
	}
	if len(stages) == 0 {
		return fmt.Errorf("no stage results for run_id %q", *runID)
	}
	return verifySessionCounters(cases, stages, *scenario, *counterPath)
}

func verifySessionCounters(cases []workloadCase, stages []stageResult, scenario, counterPath string) error {
	if len(cases) == 0 || len(stages) == 0 {
		return errors.New("session counter verification requires workload cases and stage results")
	}
	expected := make(map[string]int64)
	caseByID := make(map[string]workloadCase, len(cases))
	for _, c := range cases {
		caseByID[c.CaseID] = c
	}
	for _, stage := range stages {
		if stage.Scenario != scenario {
			return fmt.Errorf("stage %s scenario mismatch: expected=%s actual=%s", stage.Stage, scenario, stage.Scenario)
		}
		for caseID, count := range stage.CaseScheduled {
			c, ok := caseByID[caseID]
			if !ok || count < 0 {
				return fmt.Errorf("stage %s has unknown case or negative schedule count: %s=%d", stage.Stage, caseID, count)
			}
			key := counterKey(c)
			if scenario == "w2" {
				expected[key] = 1
			} else {
				expected[key] += count
			}
		}
	}
	actual, err := readCounterFile(counterPath)
	if err != nil {
		return err
	}
	wantUpstream := map[string]string{"w1": "forward", "w2": "cache"}[scenario]
	if actual.Upstream != wantUpstream {
		return fmt.Errorf("fixture identity mismatch: expected=%s actual=%s", wantUpstream, actual.Upstream)
	}
	keys := make(map[string]struct{}, len(expected)+len(actual.Counts))
	for key := range expected {
		keys[key] = struct{}{}
	}
	for key := range actual.Counts {
		keys[key] = struct{}{}
	}
	for key := range keys {
		if actual.Counts[key] != expected[key] {
			return fmt.Errorf("session counter mismatch for %s: expected=%d actual=%d", key, expected[key], actual.Counts[key])
		}
	}
	return nil
}

func verifySamplesCommand(args []string) error {
	fs := flag.NewFlagSet("verify-samples", flag.ContinueOnError)
	stagePath := fs.String("stage-result", "", "stage JSONL containing per-role resource sample counts")
	stageName := fs.String("stage", "", "stage name to verify")
	expectedFixtures := fs.Int("expected-fixtures", 0, "number of live fixture processes")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *stagePath == "" || *stageName == "" || *expectedFixtures <= 0 {
		return errors.New("verify-samples requires stage-result, stage, and a positive expected-fixtures count")
	}
	stage, err := readStageResult(*stagePath, *stageName)
	if err != nil {
		return err
	}
	for _, role := range []string{"sut", "load-generator"} {
		if stage.ResourceSampleCounts[role] <= 0 {
			return fmt.Errorf("stage %s has no resource samples for role %q", stage.Stage, role)
		}
	}
	for i := 1; i <= *expectedFixtures; i++ {
		role := fmt.Sprintf("fixture-%d", i)
		if stage.ResourceSampleCounts[role] <= 0 {
			return fmt.Errorf("stage %s has no resource samples for role %q", stage.Stage, role)
		}
	}
	fmt.Fprintf(os.Stdout, "resource sampling verified: stage=%s samples=%v\n", stage.Stage, stage.ResourceSampleCounts)
	return nil
}

func verifyEventJournalCommand(args []string) error {
	fs := flag.NewFlagSet("verify-event-journal", flag.ContinueOnError)
	eventPath := fs.String("event-journal", "", "complete fixture event journal JSONL")
	stagePath := fs.String("stage-result", "", "session stage results JSONL")
	lastStage := fs.String("last-stage", "", "final stage used for the sequence barrier")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *eventPath == "" || *stagePath == "" || *lastStage == "" {
		return errors.New("verify-event-journal requires event-journal, stage-result, and last-stage")
	}
	stage, err := readStageResult(*stagePath, *lastStage)
	if err != nil {
		return err
	}
	events, err := readFixtureEvents(*eventPath)
	if err != nil {
		return err
	}
	return verifyCompleteFixtureJournal(events, stage.FixtureSeqEnd)
}

func verifyCompleteFixtureJournal(events []fixtureEvent, expectedLastSequence uint64) error {
	if uint64(len(events)) != expectedLastSequence {
		return fmt.Errorf("fixture journal tail differs from final barrier: events=%d final_barrier=%d", len(events), expectedLastSequence)
	}
	sorted := append([]fixtureEvent(nil), events...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].FixtureSeq < sorted[j].FixtureSeq })
	for i, event := range sorted {
		if event.FixtureSeq != uint64(i+1) {
			return fmt.Errorf("fixture journal has sequence gap or duplicate at %d: got=%d", i+1, event.FixtureSeq)
		}
	}
	return nil
}

func readRequestLedger(path string) ([]requestRecord, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, fmt.Errorf("open request ledger %s: %w", path, err)
	}
	defer f.Close()
	var records []requestRecord
	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 4096), 1024*1024)
	for scanner.Scan() {
		if strings.TrimSpace(scanner.Text()) == "" {
			continue
		}
		var record requestRecord
		if err := json.Unmarshal(scanner.Bytes(), &record); err != nil {
			return nil, fmt.Errorf("decode request ledger %s: %w", path, err)
		}
		records = append(records, record)
	}
	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("read request ledger %s: %w", path, err)
	}
	return records, nil
}

func loadRequestHistory(path, runID string) ([]requestRecord, uint64, error) {
	records, err := readRequestLedger(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, 0, nil
	}
	if err != nil {
		return nil, 0, err
	}
	var lastSeq uint64
	for _, record := range records {
		if record.RunID != runID {
			return nil, 0, fmt.Errorf("request ledger contains a different run_id: expected=%q actual=%q", runID, record.RunID)
		}
		if record.RequestSeq > lastSeq {
			lastSeq = record.RequestSeq
		}
	}
	return records, lastSeq, nil
}

func expectedCounterDeltas(cases []workloadCase, scenario string) map[string]int64 {
	expected := make(map[string]int64, len(cases))
	for _, c := range cases {
		key := counterKey(c)
		if scenario == "w2" {
			expected[key] = 1
		} else {
			expected[key]++
		}
	}
	return expected
}

func expectedCounterDeltasFromStage(cases []workloadCase, result stageResult, scenario string) map[string]int64 {
	expected := make(map[string]int64, len(result.CaseScheduled))
	seen := make(map[string]struct{}, len(result.CaseScheduled))
	for _, c := range cases {
		if _, ok := seen[c.CaseID]; ok {
			continue
		}
		seen[c.CaseID] = struct{}{}
		if count := result.CaseScheduled[c.CaseID]; count > 0 {
			key := counterKey(c)
			if scenario == "w2" {
				expected[key] = 1
			} else {
				expected[key] += count
			}
		}
	}
	return expected
}

func readStageResult(path, stageName string) (stageResult, error) {
	f, err := os.Open(path)
	if err != nil {
		return stageResult{}, fmt.Errorf("open stage result %s: %w", path, err)
	}
	defer f.Close()
	var found *stageResult
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		var result stageResult
		if err := json.Unmarshal(scanner.Bytes(), &result); err != nil {
			return stageResult{}, fmt.Errorf("decode stage result %s: %w", path, err)
		}
		if stageName == "" || result.Stage == stageName {
			copy := result
			found = &copy
		}
	}
	if err := scanner.Err(); err != nil {
		return stageResult{}, fmt.Errorf("read stage result %s: %w", path, err)
	}
	if found == nil {
		return stageResult{}, fmt.Errorf("stage %q not found in %s", stageName, path)
	}
	return *found, nil
}

func counterKey(c workloadCase) string {
	return strings.ToLower(dns.Fqdn(c.QName)) + "|" + strings.ToUpper(c.QType)
}

func verifyRoutingCounters(cases []workloadCase, routeAPath, routeBPath, routeCPath string) error {
	routeA, err := readCounterFile(routeAPath)
	if err != nil {
		return err
	}
	routeB, err := readCounterFile(routeBPath)
	if err != nil {
		return err
	}
	routeC, err := readCounterFile(routeCPath)
	if err != nil {
		return err
	}
	for _, c := range cases {
		key := counterKey(c)
		counts := map[string]int64{"route-a": routeA.Counts[key], "route-b": routeB.Counts[key], "route-c": routeC.Counts[key]}
		var required []string
		var forbidden []string
		switch c.ExpectedRouteClass {
		case "DOMAIN_HIT":
			required, forbidden = []string{"route-a"}, []string{"route-b", "route-c"}
		case "IP_RULE_HIT":
			required, forbidden = []string{"route-b", "route-a"}, []string{"route-c"}
		case "IP_RULE_MISS":
			required, forbidden = []string{"route-b", "route-c"}, []string{"route-a"}
		default:
			return fmt.Errorf("unknown expected route class %q for %s", c.ExpectedRouteClass, c.CaseID)
		}
		for _, route := range required {
			if counts[route] <= 0 {
				return fmt.Errorf("route counter missing %s for %s (%s)", route, c.CaseID, c.ExpectedRouteClass)
			}
		}
		for _, route := range forbidden {
			if counts[route] != 0 {
				return fmt.Errorf("unexpected route %s for %s (%s): %d", route, c.CaseID, c.ExpectedRouteClass, counts[route])
			}
		}
	}
	return nil
}

func verifyRoutingCountersWithEvents(cases []workloadCase, events []fixtureEvent, routeAPath, routeBPath, routeCPath string) error {
	if err := verifyRoutingCounters(cases, routeAPath, routeBPath, routeCPath); err != nil {
		return err
	}
	actualByUpstream := make(map[string]counterFile, 3)
	for _, path := range []string{routeAPath, routeBPath, routeCPath} {
		counter, err := readCounterFile(path)
		if err != nil {
			return err
		}
		if _, exists := actualByUpstream[counter.Upstream]; exists {
			return fmt.Errorf("duplicate W3 fixture counter identity %q", counter.Upstream)
		}
		actualByUpstream[counter.Upstream] = counter
	}
	expectedByUpstream := map[string]map[string]int64{"route-a": {}, "route-b": {}, "route-c": {}}
	for _, event := range events {
		counts, ok := expectedByUpstream[event.Upstream]
		if !ok {
			return fmt.Errorf("unknown upstream in W3 event journal: %q", event.Upstream)
		}
		key := fmt.Sprintf("%s|%s", strings.ToLower(dns.Fqdn(event.QName)), strings.ToUpper(dns.TypeToString[event.QType]))
		counts[key]++
	}
	for upstream, expected := range expectedByUpstream {
		actual, ok := actualByUpstream[upstream]
		if !ok {
			return fmt.Errorf("missing W3 fixture counter identity %q", upstream)
		}
		keys := make(map[string]struct{}, len(expected)+len(actual.Counts))
		for key := range expected {
			keys[key] = struct{}{}
		}
		for key := range actual.Counts {
			keys[key] = struct{}{}
		}
		for key := range keys {
			if actual.Counts[key] != expected[key] {
				return fmt.Errorf("W3 %s counter mismatch for %s: events=%d counter=%d", upstream, key, expected[key], actual.Counts[key])
			}
		}
	}
	return nil
}

func readCounterFile(path string) (counterFile, error) {
	f, err := os.Open(path)
	if err != nil {
		return counterFile{}, fmt.Errorf("open counter %s: %w", path, err)
	}
	defer f.Close()
	var counter counterFile
	if err := json.NewDecoder(f).Decode(&counter); err != nil {
		return counterFile{}, fmt.Errorf("decode counter %s: %w", path, err)
	}
	if counter.Counts == nil {
		counter.Counts = make(map[string]int64)
	}
	return counter, nil
}

type stageOptions struct {
	workload         []workloadCase
	addr             string
	transport        string
	scenario         string
	stage            string
	qps              float64
	duration         time.Duration
	deadline         time.Duration
	lateDrain        time.Duration
	resultDir        string
	sutPID           int
	runID            string
	fixtureSessionID string
	ledgerPath       string
	eventJournalPath string
	fixtureTargets   []resourceTarget
	idAllocator      *requestIDAllocator
	failOnError      bool
	onePass          bool
}

type requestLedgerWriter struct {
	mu sync.Mutex
	f  *os.File
}

type requestIDAllocator struct {
	mu          sync.Mutex
	next        uint16
	active      map[string]struct{}
	quarantined map[uint16]struct{}
}

func newRequestIDAllocator(history []requestRecord) *requestIDAllocator {
	a := &requestIDAllocator{
		next:        uint16(time.Now().UnixNano()),
		active:      make(map[string]struct{}),
		quarantined: make(map[uint16]struct{}),
	}
	for _, record := range history {
		if record.Sent && record.Outcome != "correct_on_time" {
			a.quarantined[record.DNSID] = struct{}{}
		}
	}
	return a
}

func (a *requestIDAllocator) allocate(qname, qtype string, qclass uint16) (uint16, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	for attempts := 0; attempts <= int(^uint16(0)); attempts++ {
		id := a.next
		a.next++
		if _, ok := a.quarantined[id]; ok {
			continue
		}
		key := fmt.Sprintf("%d|%s", id, requestKey(qname, qtype, qclass))
		if _, ok := a.active[key]; ok {
			continue
		}
		a.active[key] = struct{}{}
		return id, nil
	}
	return 0, errors.New("no DNS transaction ID available for question")
}

func (a *requestIDAllocator) release(id uint16, qname, qtype string, qclass uint16, outcome string) {
	a.mu.Lock()
	defer a.mu.Unlock()
	key := fmt.Sprintf("%d|%s", id, requestKey(qname, qtype, qclass))
	delete(a.active, key)
	if outcome != "correct_on_time" {
		a.quarantined[id] = struct{}{}
	}
}

func openRequestLedger(path string) (*requestLedgerWriter, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return nil, err
	}
	return &requestLedgerWriter{f: f}, nil
}

func (w *requestLedgerWriter) write(record requestRecord) error {
	data, err := json.Marshal(record)
	if err != nil {
		return err
	}
	data = append(data, '\n')
	w.mu.Lock()
	defer w.mu.Unlock()
	_, err = w.f.Write(data)
	return err
}

func (w *requestLedgerWriter) close() error {
	w.mu.Lock()
	defer w.mu.Unlock()
	if err := w.f.Sync(); err != nil {
		_ = w.f.Close()
		return err
	}
	return w.f.Close()
}

func executeStage(opts stageOptions) error {
	var eventJournal *fixtureEventJournal
	var fixtureSeqStart uint64
	if opts.eventJournalPath != "" {
		eventJournal = newFixtureEventJournal(opts.eventJournalPath)
		var err error
		fixtureSeqStart, err = eventJournal.lastSequence()
		if err != nil {
			return fmt.Errorf("read fixture event barrier before stage: %w", err)
		}
	}
	history, lastRequestSeq, err := loadRequestHistory(opts.ledgerPath, opts.runID)
	if err != nil {
		return fmt.Errorf("load request history: %w", err)
	}
	if opts.idAllocator == nil {
		opts.idAllocator = newRequestIDAllocator(history)
	}
	sutStartIdentity := ""
	sutCPUSet := ""
	if opts.sutPID > 0 && runtime.GOOS == "linux" {
		sutStartIdentity, err = processStartIdentity(opts.sutPID)
		if err != nil {
			return fmt.Errorf("capture SUT process start identity: %w", err)
		}
		sutCPUSet, err = processCPUSet(opts.sutPID)
		if err != nil {
			return fmt.Errorf("capture SUT CPU affinity: %w", err)
		}
	}
	harnessPID := os.Getpid()
	harnessCPUSet := processCPUSetOrEmpty(harnessPID)
	if runtime.GOOS == "linux" && harnessCPUSet == "" {
		return errors.New("could not capture helper process CPU affinity")
	}
	// Resolve the host constant before any measured-stage work. Sampling must
	// never fork a clock query alongside the load generator and fixtures.
	clockTicks := int64(100)
	if opts.sutPID > 0 && runtime.GOOS == "linux" {
		clockTicks, err = resourceClockTicksPerSecond()
		if err != nil {
			return fmt.Errorf("resolve resource clock before stage: %w", err)
		}
	}
	started := time.Now().UTC()
	ledger, err := openRequestLedger(opts.ledgerPath)
	if err != nil {
		return fmt.Errorf("open request ledger: %w", err)
	}
	ledgerClosed := false
	defer func() {
		if !ledgerClosed {
			_ = ledger.close()
		}
	}()
	stats := new(runStats)
	var requestWG sync.WaitGroup
	caseScheduled := make(map[string]int64)
	resourcePath := filepath.Join(opts.resultDir, "resource-samples.jsonl")
	resourceCount := 0
	resourceCounts := make(map[string]int)
	stopSamples := make(chan struct{})
	var sampleWG sync.WaitGroup
	if opts.sutPID > 0 {
		sampleWG.Add(1)
		go func() {
			defer sampleWG.Done()
			targets := []resourceTarget{{Role: "sut", PID: opts.sutPID}, {Role: "load-generator", PID: os.Getpid()}}
			targets = append(targets, opts.fixtureTargets...)
			resourceCounts = sampleProcessGroup(targets, opts.runID, opts.stage, resourcePath, stopSamples, clockTicks)
			resourceCount = resourceCounts["sut"]
		}()
	}

	interval := time.Duration(float64(time.Second) / opts.qps)
	if interval <= 0 {
		interval = time.Nanosecond
	}
	end := time.Now().Add(opts.duration)
	next := time.Now()
	requestSeq := lastRequestSeq
	inFlightQuestions := make(map[string]chan struct{})
	for index := 0; (opts.onePass && index < len(opts.workload)) || (!opts.onePass && next.Before(end)); index++ {
		if sleep := time.Until(next); sleep > 0 {
			time.Sleep(sleep)
		}
		lag := time.Since(next)
		if lag > 2*interval {
			stats.mu.Lock()
			stats.counters.SenderShortfall++
			if lag.Microseconds() > stats.maxLagUS {
				stats.maxLagUS = lag.Microseconds()
			}
			stats.mu.Unlock()
			next = next.Add(interval)
			continue
		}
		caseValue := opts.workload[index%len(opts.workload)]
		var questionDone chan struct{}
		if opts.scenario == "w3" {
			question := requestKey(caseValue.QName, caseValue.QType, dns.ClassINET)
			if previousDone := inFlightQuestions[question]; previousDone != nil {
				<-previousDone
			}
			questionDone = make(chan struct{})
			inFlightQuestions[question] = questionDone
		}
		requestSeq++
		stats.mu.Lock()
		stats.counters.Scheduled++
		caseScheduled[caseValue.CaseID]++
		stats.mu.Unlock()
		requestWG.Add(1)
		go func(seq uint64, c workloadCase, done chan struct{}) {
			defer requestWG.Done()
			if done != nil {
				defer close(done)
			}
			executeRequest(opts, c, seq, stats, ledger)
		}(requestSeq, caseValue, questionDone)
		next = next.Add(interval)
	}

	// executeRequest owns its own bounded deadline. The late drain gives the
	// response association logic a deterministic window before timeout.
	grace := opts.deadline + opts.lateDrain + 50*time.Millisecond
	time.Sleep(grace)
	requestWG.Wait()
	close(stopSamples)
	sampleWG.Wait()
	var fixtureSeqEnd uint64
	if eventJournal != nil {
		var err error
		fixtureSeqEnd, err = eventJournal.lastSequence()
		if err != nil {
			return fmt.Errorf("read fixture event barrier after stage: %w", err)
		}
	}
	ledgerCloseErr := ledger.close()
	ledgerClosed = true
	if ledgerCloseErr != nil {
		return fmt.Errorf("close request ledger: %w", ledgerCloseErr)
	}
	finished := time.Now().UTC()
	stats.mu.Lock()
	result := stageResult{
		Stage: opts.stage, RunID: opts.runID, FixtureSessionID: opts.fixtureSessionID, Scenario: opts.scenario, Transport: opts.transport,
		TargetQPS: opts.qps, DurationMS: opts.duration.Milliseconds(),
		RequestDeadlineMS: int(opts.deadline / time.Millisecond),
		LateDrainMS:       int(opts.lateDrain / time.Millisecond), Counters: stats.counters,
		CaseScheduled:    caseScheduled,
		LatencySamplesUS: append([]int64(nil), stats.latenciesUS...),
		SenderLagMaxUS:   stats.maxLagUS, StartedAt: started, FinishedAt: finished,
		ResourceSampleCount: resourceCount, ResourceSampleCounts: resourceCounts,
		SUTPID: opts.sutPID, SUTStartIdentity: sutStartIdentity,
		SUTCPUSet: sutCPUSet, HarnessPID: harnessPID,
		HarnessCPUSet: harnessCPUSet, RequestLedgerPath: opts.ledgerPath,
		EventJournalPath: opts.eventJournalPath, FixtureSeqStart: fixtureSeqStart, FixtureSeqEnd: fixtureSeqEnd,
		RequestSeqStart: lastRequestSeq + 1, RequestSeqEnd: requestSeq,
	}
	ledgerErr := stats.ledgerErr
	stats.mu.Unlock()
	if ledgerErr != nil {
		return fmt.Errorf("write request ledger: %w", ledgerErr)
	}
	sort.Slice(result.LatencySamplesUS, func(i, j int) bool { return result.LatencySamplesUS[i] < result.LatencySamplesUS[j] })
	result.P50US = percentile(result.LatencySamplesUS, 0.50)
	result.P95US = percentile(result.LatencySamplesUS, 0.95)
	result.P99US = percentile(result.LatencySamplesUS, 0.99)
	if result.DurationMS > 0 {
		result.EffectiveThroughput = float64(result.Counters.CorrectOnTime) / (float64(result.DurationMS) / 1000)
	}
	stageFile := filepath.Join(opts.resultDir, "stages.jsonl")
	f, err := os.OpenFile(stageFile, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return err
	}
	err = json.NewEncoder(f).Encode(result)
	closeErr := f.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	if opts.failOnError && hasStageFailure(result.Counters) {
		return fmt.Errorf("stage %s has correctness failures: %+v", opts.stage, result.Counters)
	}
	return nil
}

func executeRequest(opts stageOptions, c workloadCase, requestSeq uint64, stats *runStats, ledger *requestLedgerWriter) {
	request := requestRecord{
		RunID: opts.runID, StageID: opts.stage, RequestSeq: requestSeq,
		CaseID: c.CaseID, QName: dns.Fqdn(c.QName), QType: strings.ToUpper(c.QType),
		Outcome: "protocol_error",
	}
	idAllocated := false
	finish := func(outcome string) {
		request.Outcome = outcome
		request.FinishedAt = time.Now().UTC()
		if idAllocated {
			opts.idAllocator.release(request.DNSID, request.QName, request.QType, request.QClass, outcome)
		}
		if err := ledger.write(request); err != nil {
			stats.mu.Lock()
			if stats.ledgerErr == nil {
				stats.ledgerErr = err
			}
			stats.mu.Unlock()
		}
	}
	qtype, ok := dns.StringToType[strings.ToUpper(c.QType)]
	if !ok {
		recordTerminal(stats, "protocol")
		finish("protocol_error")
		return
	}
	query := new(dns.Msg)
	query.SetQuestion(dns.Fqdn(c.QName), qtype)
	if opts.idAllocator == nil {
		recordTerminal(stats, "protocol")
		finish("protocol_error")
		return
	}
	id, err := opts.idAllocator.allocate(query.Question[0].Name, dns.TypeToString[qtype], query.Question[0].Qclass)
	if err != nil {
		recordTerminal(stats, "protocol")
		finish("protocol_error")
		return
	}
	query.Id = id
	request.DNSID = query.Id
	request.QClass = query.Question[0].Qclass
	idAllocated = true
	payload, err := query.Pack()
	if err != nil {
		recordTerminal(stats, "protocol")
		finish("protocol_error")
		return
	}
	sendAt := time.Now()
	request.SentAt = sendAt.UTC()
	var wire []byte
	var sent bool
	if opts.transport == "udp" {
		wire, sent, err = exchangeUDP(opts.addr, payload, opts.deadline+opts.lateDrain)
	} else {
		wire, sent, err = exchangeTCP(opts.addr, payload, opts.deadline+opts.lateDrain)
	}
	request.Sent = sent
	if sent {
		stats.mu.Lock()
		stats.counters.Sent++
		stats.mu.Unlock()
	}
	if err != nil {
		if errors.Is(err, os.ErrDeadlineExceeded) || errors.Is(err, errTimeout) {
			recordTerminal(stats, "timeout")
			finish("timeout")
		} else if ne, ok := err.(net.Error); ok && ne.Timeout() {
			recordTerminal(stats, "timeout")
			finish("timeout")
		} else {
			recordTerminal(stats, "transport")
			finish("transport_error")
		}
		return
	}
	latencyUS := time.Since(sendAt).Microseconds()
	stats.mu.Lock()
	stats.counters.Received++
	stats.mu.Unlock()
	response := new(dns.Msg)
	if err := response.Unpack(wire); err != nil {
		recordTerminal(stats, "protocol")
		finish("protocol_error")
		return
	}
	if !responseMatches(response, query, c) {
		recordTerminal(stats, "wrong")
		finish("wrong_response")
		return
	}
	stats.mu.Lock()
	stats.latenciesUS = append(stats.latenciesUS, latencyUS)
	if latencyUS <= int64(c.RequestDeadlineMS)*1000 {
		stats.counters.CorrectOnTime++
		if c.ExpectedRCode != dns.RcodeSuccess {
			stats.counters.ExpectedNegativeOnTime++
		}
	} else {
		stats.counters.CorrectLate++
	}
	stats.mu.Unlock()
	if latencyUS <= int64(c.RequestDeadlineMS)*1000 {
		finish("correct_on_time")
	} else {
		finish("correct_late")
	}
}

var errTimeout = errors.New("deadline exceeded")

func recordTerminal(stats *runStats, kind string) {
	stats.mu.Lock()
	defer stats.mu.Unlock()
	switch kind {
	case "wrong":
		stats.counters.WrongResponse++
	case "protocol":
		stats.counters.ProtocolError++
	case "transport":
		stats.counters.TransportError++
	case "timeout":
		stats.counters.Timeout++
	}
}

type requestRecord struct {
	RunID      string    `json:"run_id"`
	StageID    string    `json:"stage_id"`
	RequestSeq uint64    `json:"request_seq"`
	DNSID      uint16    `json:"dns_id"`
	CaseID     string    `json:"case_id"`
	QName      string    `json:"qname"`
	QType      string    `json:"qtype"`
	QClass     uint16    `json:"qclass"`
	Sent       bool      `json:"sent"`
	SentAt     time.Time `json:"sent_at"`
	FinishedAt time.Time `json:"finished_at"`
	Outcome    string    `json:"outcome"`
}

type fixtureEvent struct {
	FixtureSeq uint64    `json:"fixture_seq"`
	OccurredAt time.Time `json:"occurred_at"`
	DNSID      uint16    `json:"dns_id"`
	QName      string    `json:"qname"`
	QType      uint16    `json:"qtype"`
	QClass     uint16    `json:"qclass"`
	Upstream   string    `json:"upstream"`
}

type fixtureEventJournal struct {
	path string
	mu   *sync.Mutex
}

var fixtureEventJournalLocks sync.Map

func newFixtureEventJournal(path string) *fixtureEventJournal {
	resolved, err := filepath.Abs(path)
	if err == nil {
		path = resolved
	}
	mu, _ := fixtureEventJournalLocks.LoadOrStore(path, new(sync.Mutex))
	return &fixtureEventJournal{path: path, mu: mu.(*sync.Mutex)}
}

func (j *fixtureEventJournal) withLock(fn func() error) error {
	j.mu.Lock()
	defer j.mu.Unlock()
	if err := os.MkdirAll(filepath.Dir(j.path), 0755); err != nil {
		return err
	}
	lockFile, err := os.OpenFile(j.path+".lock", os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	if err := syscall.Flock(int(lockFile.Fd()), syscall.LOCK_EX); err != nil {
		_ = lockFile.Close()
		return err
	}
	fnErr := fn()
	unlockErr := syscall.Flock(int(lockFile.Fd()), syscall.LOCK_UN)
	closeErr := lockFile.Close()
	if fnErr != nil {
		return fnErr
	}
	if unlockErr != nil {
		return unlockErr
	}
	return closeErr
}

func (j *fixtureEventJournal) append(event fixtureEvent) (fixtureEvent, error) {
	var err error
	err = j.withLock(func() error {
		event.OccurredAt = time.Now().UTC()
		last, err := j.lastSequenceUnlocked()
		if err != nil {
			return err
		}
		event.FixtureSeq = last + 1
		data, err := json.Marshal(event)
		if err != nil {
			return err
		}
		data = append(data, '\n')
		f, err := os.OpenFile(j.path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
		if err != nil {
			return err
		}
		n, writeErr := f.Write(data)
		closeErr := f.Close()
		if writeErr != nil {
			return writeErr
		}
		if n != len(data) {
			return io.ErrShortWrite
		}
		return closeErr
	})
	return event, err
}

func (j *fixtureEventJournal) lastSequence() (uint64, error) {
	var seq uint64
	err := j.withLock(func() error {
		var err error
		seq, err = j.lastSequenceUnlocked()
		return err
	})
	return seq, err
}

func (j *fixtureEventJournal) lastSequenceUnlocked() (uint64, error) {
	f, err := os.Open(j.path)
	if errors.Is(err, os.ErrNotExist) {
		return 0, nil
	}
	if err != nil {
		return 0, err
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil {
		return 0, err
	}
	if info.Size() == 0 {
		return 0, nil
	}
	const maxLineBytes = 4096
	readSize := info.Size()
	if readSize > maxLineBytes {
		readSize = maxLineBytes
	}
	buf := make([]byte, readSize)
	if _, err := f.ReadAt(buf, info.Size()-readSize); err != nil {
		return 0, err
	}
	buf = bytes.TrimRight(buf, "\r\n")
	lineStart := bytes.LastIndexByte(buf, '\n') + 1
	if lineStart == 0 && info.Size() > readSize {
		return 0, errors.New("fixture event record exceeds 4096 bytes")
	}
	var event fixtureEvent
	if err := json.Unmarshal(buf[lineStart:], &event); err != nil {
		return 0, fmt.Errorf("decode last fixture event: %w", err)
	}
	if event.FixtureSeq == 0 {
		return 0, errors.New("last fixture event has zero sequence")
	}
	return event.FixtureSeq, nil
}

func readFixtureEvents(path string) ([]fixtureEvent, error) {
	journal := newFixtureEventJournal(path)
	var events []fixtureEvent
	err := journal.withLock(func() error {
		f, err := os.Open(path)
		if errors.Is(err, os.ErrNotExist) {
			return nil
		}
		if err != nil {
			return err
		}
		defer f.Close()
		scanner := bufio.NewScanner(f)
		scanner.Buffer(make([]byte, 4096), 1024*1024)
		for scanner.Scan() {
			if strings.TrimSpace(scanner.Text()) == "" {
				continue
			}
			var event fixtureEvent
			if err := json.Unmarshal(scanner.Bytes(), &event); err != nil {
				return fmt.Errorf("decode fixture event: %w", err)
			}
			events = append(events, event)
		}
		return scanner.Err()
	})
	return events, err
}

func requestKey(qname, qtype string, qclass uint16) string {
	return fmt.Sprintf("%s|%s|%d", strings.ToLower(dns.Fqdn(qname)), strings.ToUpper(qtype), qclass)
}

func verifyWarmTTL(cases []workloadCase, prefill, warm []requestRecord, ttl, safetyMargin time.Duration) error {
	if ttl <= 0 || safetyMargin < 0 || safetyMargin >= ttl {
		return fmt.Errorf("invalid TTL window: ttl=%s safety_margin=%s", ttl, safetyMargin)
	}
	maxAge := ttl - safetyMargin
	expectedKeys := make(map[string]string, len(cases))
	for _, c := range cases {
		qtype, ok := dns.StringToType[strings.ToUpper(c.QType)]
		if !ok {
			return fmt.Errorf("unknown query type %q for %s", c.QType, c.CaseID)
		}
		key := requestKey(c.QName, dns.TypeToString[qtype], dns.ClassINET)
		expectedKeys[key] = c.CaseID
	}
	if len(expectedKeys) == 0 {
		return errors.New("warm TTL verification requires workload cases")
	}

	sessionID := ""
	prefillAt := make(map[string]time.Time, len(expectedKeys))
	for _, record := range prefill {
		if record.RunID == "" || record.StageID == "" {
			return errors.New("prefill ledger record is missing run_id or stage_id")
		}
		if sessionID == "" {
			sessionID = record.RunID
		} else if sessionID != record.RunID {
			return fmt.Errorf("prefill ledger mixes run_id values: %q and %q", sessionID, record.RunID)
		}
		key := requestKey(record.QName, record.QType, record.QClass)
		if _, ok := expectedKeys[key]; !ok {
			return fmt.Errorf("unexpected prefill key %s for case %s", key, record.CaseID)
		}
		if !record.Sent || record.Outcome != "correct_on_time" || record.SentAt.IsZero() || record.FinishedAt.IsZero() {
			return fmt.Errorf("invalid prefill record for %s: sent=%t outcome=%s sent_at=%s finished_at=%s", record.CaseID, record.Sent, record.Outcome, record.SentAt, record.FinishedAt)
		}
		if record.FinishedAt.Before(record.SentAt) {
			return fmt.Errorf("prefill completion precedes send for %s", record.CaseID)
		}
		if previous, ok := prefillAt[key]; !ok || record.SentAt.Before(previous) {
			prefillAt[key] = record.SentAt
		}
	}
	for key, caseID := range expectedKeys {
		if _, ok := prefillAt[key]; !ok {
			return fmt.Errorf("missing successful prefill for %s (%s)", caseID, key)
		}
	}

	measured := make(map[string]int, len(expectedKeys))
	for _, record := range warm {
		if record.RunID == "" || record.StageID == "" || record.RunID != sessionID {
			return fmt.Errorf("warm ledger session mismatch: expected run_id=%q, got run_id=%q stage_id=%q", sessionID, record.RunID, record.StageID)
		}
		key := requestKey(record.QName, record.QType, record.QClass)
		caseID, ok := expectedKeys[key]
		if !ok {
			return fmt.Errorf("unexpected warm key %s for case %s", key, record.CaseID)
		}
		if !record.Sent || record.Outcome != "correct_on_time" || record.FinishedAt.IsZero() || record.SentAt.IsZero() {
			return fmt.Errorf("invalid warm response for %s: sent=%t outcome=%s sent_at=%s finished_at=%s", record.CaseID, record.Sent, record.Outcome, record.SentAt, record.FinishedAt)
		}
		if record.FinishedAt.Before(record.SentAt) {
			return fmt.Errorf("warm completion precedes send for %s", record.CaseID)
		}
		age := record.FinishedAt.Sub(prefillAt[key])
		if age < 0 || age >= maxAge {
			return fmt.Errorf("warm response for %s exceeds TTL safety window: age=%s maximum=%s", caseID, age, maxAge)
		}
		measured[key]++
	}
	for key, caseID := range expectedKeys {
		if measured[key] == 0 {
			return fmt.Errorf("missing warm measurement for %s (%s)", caseID, key)
		}
	}
	return nil
}

func expectedRoutePath(routeClass string) ([]string, error) {
	switch routeClass {
	case "DOMAIN_HIT":
		return []string{"route-a"}, nil
	case "IP_RULE_HIT":
		return []string{"route-b", "route-a"}, nil
	case "IP_RULE_MISS":
		return []string{"route-b", "route-c"}, nil
	default:
		return nil, fmt.Errorf("unknown expected route class %q", routeClass)
	}
}

func verifyRequestIDUse(records []requestRecord) error {
	bySequence := append([]requestRecord(nil), records...)
	sort.Slice(bySequence, func(i, j int) bool { return bySequence[i].RequestSeq < bySequence[j].RequestSeq })
	for i, record := range bySequence {
		if record.RequestSeq == 0 || (i > 0 && record.RequestSeq != bySequence[i-1].RequestSeq+1) {
			return errors.New("request_seq is missing, duplicated, or not contiguous for the session")
		}
	}
	ordered := append([]requestRecord(nil), records...)
	sort.Slice(ordered, func(i, j int) bool {
		if ordered[i].SentAt.Equal(ordered[j].SentAt) {
			return ordered[i].RequestSeq < ordered[j].RequestSeq
		}
		return ordered[i].SentAt.Before(ordered[j].SentAt)
	})
	activeKeyEnd := make(map[string]time.Time)
	quarantinedAt := make(map[uint16]time.Time)
	for _, record := range ordered {
		if !record.Sent {
			continue
		}
		if record.SentAt.IsZero() || record.FinishedAt.IsZero() || record.FinishedAt.Before(record.SentAt) {
			return fmt.Errorf("invalid request interval for request_seq=%d", record.RequestSeq)
		}
		if failedAt, ok := quarantinedAt[record.DNSID]; ok && !record.SentAt.Before(failedAt) {
			return fmt.Errorf("DNS ID %d was reused after a failed request", record.DNSID)
		}
		key := fmt.Sprintf("%d|%s", record.DNSID, requestKey(record.QName, record.QType, record.QClass))
		if previousEnd, ok := activeKeyEnd[key]; ok {
			if record.SentAt.Before(previousEnd) {
				return fmt.Errorf("DNS ID/question tuple reused while in flight: dns_id=%d question=%s", record.DNSID, key)
			}
			if record.FinishedAt.After(previousEnd) {
				activeKeyEnd[key] = record.FinishedAt
			}
		} else {
			activeKeyEnd[key] = record.FinishedAt
		}
		if record.Outcome != "correct_on_time" {
			if previousFailedAt, ok := quarantinedAt[record.DNSID]; !ok || record.FinishedAt.Before(previousFailedAt) {
				quarantinedAt[record.DNSID] = record.FinishedAt
			}
		}
	}
	return nil
}

func verifyRoutingEvents(cases []workloadCase, requests []requestRecord, events []fixtureEvent, stage stageResult) error {
	if stage.RunID == "" || stage.Stage == "" || stage.FixtureSeqEnd < stage.FixtureSeqStart {
		return errors.New("routing stage is missing run identity, stage identity, or valid fixture barriers")
	}
	if err := verifyRequestIDUse(requests); err != nil {
		return err
	}
	if err := verifyQuestionUse(requests); err != nil {
		return err
	}
	caseByID := make(map[string]workloadCase, len(cases))
	for _, c := range cases {
		if _, exists := caseByID[c.CaseID]; exists {
			return fmt.Errorf("duplicate workload case id %q", c.CaseID)
		}
		caseByID[c.CaseID] = c
	}

	stageRequests := make([]requestRecord, 0, len(requests))
	for _, request := range requests {
		if request.RunID != stage.RunID {
			return fmt.Errorf("request run_id mismatch: expected=%q actual=%q", stage.RunID, request.RunID)
		}
		if request.StageID != stage.Stage {
			continue
		}
		if !request.Sent || request.Outcome != "correct_on_time" {
			return fmt.Errorf("invalid W3 request %d: sent=%t outcome=%s", request.RequestSeq, request.Sent, request.Outcome)
		}
		c, ok := caseByID[request.CaseID]
		if !ok {
			return fmt.Errorf("unknown case_id %q in request ledger", request.CaseID)
		}
		qtype, ok := dns.StringToType[strings.ToUpper(c.QType)]
		if !ok || !strings.EqualFold(dns.Fqdn(c.QName), dns.Fqdn(request.QName)) || qtype != dns.StringToType[strings.ToUpper(request.QType)] || request.QClass != dns.ClassINET {
			return fmt.Errorf("request question mismatch for case %s", c.CaseID)
		}
		stageRequests = append(stageRequests, request)
	}
	if len(stageRequests) == 0 {
		return fmt.Errorf("no client requests found for stage %q", stage.Stage)
	}
	sort.Slice(stageRequests, func(i, j int) bool { return stageRequests[i].RequestSeq < stageRequests[j].RequestSeq })
	for i := range stageRequests {
		if i > 0 && stageRequests[i].RequestSeq <= stageRequests[i-1].RequestSeq {
			return errors.New("request_seq is not strictly increasing within the stage")
		}
	}
	if stageRequests[0].RequestSeq != stage.RequestSeqStart || stageRequests[len(stageRequests)-1].RequestSeq != stage.RequestSeqEnd || uint64(len(stageRequests)) != stage.RequestSeqEnd-stage.RequestSeqStart+1 {
		return fmt.Errorf("request ledger does not match stage sequence range: ledger=%d range=%d..%d", len(stageRequests), stage.RequestSeqStart, stage.RequestSeqEnd)
	}
	if int64(len(stageRequests)) != stage.Counters.Sent || int64(len(stageRequests)) != stage.Counters.CorrectOnTime {
		return fmt.Errorf("stage/client ledger count mismatch: ledger=%d sent=%d correct_on_time=%d", len(stageRequests), stage.Counters.Sent, stage.Counters.CorrectOnTime)
	}

	windowEvents := make([]fixtureEvent, 0, len(events))
	for _, event := range events {
		if event.FixtureSeq > stage.FixtureSeqStart && event.FixtureSeq <= stage.FixtureSeqEnd {
			windowEvents = append(windowEvents, event)
		}
	}
	sort.Slice(windowEvents, func(i, j int) bool { return windowEvents[i].FixtureSeq < windowEvents[j].FixtureSeq })
	if len(windowEvents) == 0 || uint64(len(windowEvents)) != stage.FixtureSeqEnd-stage.FixtureSeqStart {
		return fmt.Errorf("fixture event loss in stage window: seq_start=%d seq_end=%d events=%d", stage.FixtureSeqStart, stage.FixtureSeqEnd, len(windowEvents))
	}
	for i, event := range windowEvents {
		wantSeq := stage.FixtureSeqStart + uint64(i) + 1
		if event.FixtureSeq != wantSeq {
			return fmt.Errorf("fixture sequence gap or duplicate: expected=%d actual=%d", wantSeq, event.FixtureSeq)
		}
	}

	requestsByQuestion := make(map[string][]requestRecord)
	for _, request := range stageRequests {
		key := requestKey(request.QName, request.QType, request.QClass)
		requestsByQuestion[key] = append(requestsByQuestion[key], request)
	}
	eventsByRequest := make(map[uint64][]fixtureEvent, len(stageRequests))
	for _, event := range windowEvents {
		if event.OccurredAt.IsZero() {
			return fmt.Errorf("fixture event %d has no occurrence timestamp", event.FixtureSeq)
		}
		key := requestKey(event.QName, dns.TypeToString[event.QType], event.QClass)
		var owner requestRecord
		matches := 0
		for _, request := range requestsByQuestion[key] {
			if !event.OccurredAt.Before(request.SentAt) && !event.OccurredAt.After(request.FinishedAt) {
				owner = request
				matches++
			}
		}
		if matches == 0 {
			return fmt.Errorf("fixture event %d has no matching client request interval for %s", event.FixtureSeq, key)
		}
		if matches > 1 {
			return fmt.Errorf("fixture event %d ambiguously matches %d client request intervals for %s", event.FixtureSeq, matches, key)
		}
		eventsByRequest[owner.RequestSeq] = append(eventsByRequest[owner.RequestSeq], event)
	}
	for _, request := range stageRequests {
		c := caseByID[request.CaseID]
		path, err := expectedRoutePath(c.ExpectedRouteClass)
		if err != nil {
			return fmt.Errorf("case %s: %w", c.CaseID, err)
		}
		requestEvents := eventsByRequest[request.RequestSeq]
		sort.Slice(requestEvents, func(i, j int) bool { return requestEvents[i].FixtureSeq < requestEvents[j].FixtureSeq })
		if len(requestEvents) != len(path) {
			return fmt.Errorf("route leg count mismatch for request_seq=%d case=%s: expected=%d actual=%d path=%v", request.RequestSeq, c.CaseID, len(path), len(requestEvents), path)
		}
		for i, upstream := range path {
			if requestEvents[i].Upstream != upstream {
				return fmt.Errorf("route path mismatch for request_seq=%d case=%s at leg %d: expected=%s actual=%s", request.RequestSeq, c.CaseID, i, upstream, requestEvents[i].Upstream)
			}
		}
	}
	return nil
}

// verifyQuestionUse proves that one W3 question tuple is never in flight more
// than once. Fixture occurrence timestamps are the client-to-fixture join;
// single-flight prevents adjacent same-question intervals from overlapping.
func verifyQuestionUse(records []requestRecord) error {
	ordered := append([]requestRecord(nil), records...)
	sort.Slice(ordered, func(i, j int) bool {
		if ordered[i].SentAt.Equal(ordered[j].SentAt) {
			return ordered[i].RequestSeq < ordered[j].RequestSeq
		}
		return ordered[i].SentAt.Before(ordered[j].SentAt)
	})
	activeEnd := make(map[string]time.Time)
	for _, record := range ordered {
		if !record.Sent {
			continue
		}
		if record.SentAt.IsZero() || record.FinishedAt.IsZero() || record.FinishedAt.Before(record.SentAt) {
			return fmt.Errorf("invalid request interval for request_seq=%d", record.RequestSeq)
		}
		key := requestKey(record.QName, record.QType, record.QClass)
		if previousEnd, ok := activeEnd[key]; ok && record.SentAt.Before(previousEnd) {
			return fmt.Errorf("question tuple reused while in flight: question=%s request_seq=%d", key, record.RequestSeq)
		}
		activeEnd[key] = record.FinishedAt
	}
	return nil
}

func responseMatches(response, query *dns.Msg, c workloadCase) bool {
	if response == nil || query == nil || response.Id != query.Id || response.Opcode != query.Opcode || response.Response != true || response.Truncated || len(query.Question) != 1 || len(response.Question) != 1 {
		return false
	}
	question := response.Question[0]
	expectedQuestion := query.Question[0]
	if !strings.EqualFold(question.Name, expectedQuestion.Name) || question.Qtype != expectedQuestion.Qtype || question.Qclass != expectedQuestion.Qclass || response.Rcode != c.ExpectedRCode {
		return false
	}
	if c.ExpectedRCode != dns.RcodeSuccess {
		return len(response.Answer) == 0
	}
	if c.ExpectedAnswerClass != "A" || question.Qtype != dns.TypeA || len(response.Answer) != 1 {
		return false
	}
	expectedIP := net.ParseIP(c.ExpectedAnswer).To4()
	answer, ok := response.Answer[0].(*dns.A)
	if !ok || expectedIP == nil || answer.A.To4() == nil {
		return false
	}
	return strings.EqualFold(answer.Hdr.Name, expectedQuestion.Name) && answer.Hdr.Rrtype == expectedQuestion.Qtype && answer.Hdr.Class == expectedQuestion.Qclass && answer.A.To4().Equal(expectedIP)
}

func exchangeUDP(addr string, payload []byte, timeout time.Duration) ([]byte, bool, error) {
	conn, err := net.DialTimeout("udp", addr, timeout)
	if err != nil {
		return nil, false, err
	}
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(timeout))
	if _, err := conn.Write(payload); err != nil {
		return nil, false, err
	}
	buf := make([]byte, dns.MaxMsgSize)
	n, err := conn.Read(buf)
	if err != nil {
		return nil, true, err
	}
	return append([]byte(nil), buf[:n]...), true, nil
}

func exchangeTCP(addr string, payload []byte, timeout time.Duration) ([]byte, bool, error) {
	conn, err := net.DialTimeout("tcp", addr, timeout)
	if err != nil {
		return nil, false, err
	}
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(timeout))
	if len(payload) > int(^uint16(0)) {
		return nil, false, errors.New("DNS query too large")
	}
	frame := make([]byte, 2+len(payload))
	binary.BigEndian.PutUint16(frame, uint16(len(payload)))
	copy(frame[2:], payload)
	if _, err := conn.Write(frame); err != nil {
		return nil, false, err
	}
	var length uint16
	if err := binary.Read(conn, binary.BigEndian, &length); err != nil {
		return nil, true, err
	}
	if length == 0 || length > dns.MaxMsgSize {
		return nil, true, errors.New("invalid DNS TCP response length")
	}
	response := make([]byte, length)
	if _, err := io.ReadFull(conn, response); err != nil {
		return nil, true, err
	}
	return response, true, nil
}

func percentile(values []int64, p float64) int64 {
	if len(values) == 0 {
		return 0
	}
	index := int(math.Ceil(float64(len(values))*p)) - 1
	if index < 0 {
		index = 0
	}
	if index >= len(values) {
		index = len(values) - 1
	}
	return values[index]
}

func hasStageFailure(c stageCounters) bool {
	return c.WrongResponse > 0 || c.ProtocolError > 0 || c.TransportError > 0 || c.Timeout > 0 || c.SenderShortfall > 0
}

func resourceClockTicksPerSecond() (int64, error) {
	out, err := exec.Command("getconf", "CLK_TCK").Output()
	if err != nil {
		return 0, err
	}
	hz, err := strconv.ParseInt(strings.TrimSpace(string(out)), 10, 64)
	if err != nil || hz <= 0 {
		return 0, fmt.Errorf("invalid CLK_TCK %q", strings.TrimSpace(string(out)))
	}
	return hz, nil
}

func readResourceSample(pid int, hz int64) (resourceSample, bool) {
	if runtime.GOOS != "linux" {
		return resourceSample{}, false
	}
	statBytes, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return resourceSample{}, false
	}
	closeParen := strings.LastIndexByte(string(statBytes), ')')
	if closeParen < 0 {
		return resourceSample{}, false
	}
	fields := strings.Fields(string(statBytes[closeParen+1:]))
	if len(fields) < 13 {
		return resourceSample{}, false
	}
	utime, err1 := strconv.ParseUint(fields[11], 10, 64)
	stime, err2 := strconv.ParseUint(fields[12], 10, 64)
	if err1 != nil || err2 != nil {
		return resourceSample{}, false
	}
	status, err := os.ReadFile(fmt.Sprintf("/proc/%d/status", pid))
	if err != nil {
		return resourceSample{}, false
	}
	var rss int64
	fdEntries, err := os.ReadDir(fmt.Sprintf("/proc/%d/fd", pid))
	if err != nil {
		return resourceSample{}, false
	}
	for _, line := range strings.Split(string(status), "\n") {
		if strings.HasPrefix(line, "VmRSS:") {
			parts := strings.Fields(line)
			if len(parts) >= 2 {
				rss, _ = strconv.ParseInt(parts[1], 10, 64)
			}
		}
	}
	return resourceSample{Timestamp: time.Now().UTC(), PID: pid, UserTicks: utime, SystemTicks: stime, ClockTicksPerSecond: hz, UserSeconds: float64(utime) / float64(hz), SystemSeconds: float64(stime) / float64(hz), RSSKiB: rss, FDCount: len(fdEntries)}, true
}

func sampleProcessGroup(targets []resourceTarget, runID, stageID, path string, stop <-chan struct{}, clockTicks int64) map[string]int {
	counts := make(map[string]int, len(targets))
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return counts
	}
	defer f.Close()
	enc := json.NewEncoder(f)
	write := func() bool {
		ok := true
		for _, target := range targets {
			sample, sampleOK := readResourceSample(target.PID, clockTicks)
			if !sampleOK {
				ok = false
				continue
			}
			sample.RunID = runID
			sample.StageID = stageID
			sample.Role = target.Role
			if enc.Encode(sample) != nil {
				ok = false
				continue
			}
			counts[target.Role]++
		}
		return ok
	}
	write()
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-stop:
			return counts
		case <-ticker.C:
			write()
		}
	}
}

func sha256File(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", fmt.Errorf("open %s for sha256: %w", path, err)
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", fmt.Errorf("sha256 %s: %w", path, err)
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

func writeJSON(w io.Writer, value any) error {
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	return enc.Encode(value)
}
