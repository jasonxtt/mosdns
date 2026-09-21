// Command phase5a-baseline contains the deliberately narrow helper used by
// the Phase 5A Go-only baseline. It is not a general DNS server or load
// testing framework; its fixture behavior is fixed by the committed corpus.
package main

import (
	"bufio"
	"encoding/binary"
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
	Stage               string        `json:"stage"`
	Scenario            string        `json:"scenario"`
	Transport           string        `json:"transport"`
	TargetQPS           float64       `json:"target_qps"`
	DurationMS          int64         `json:"duration_ms"`
	RequestDeadlineMS   int           `json:"request_deadline_ms"`
	LateDrainMS         int           `json:"late_drain_ms"`
	Counters            stageCounters `json:"counters"`
	LatencySamplesUS    []int64       `json:"latency_samples_us"`
	P50US               int64         `json:"p50_us"`
	P95US               int64         `json:"p95_us"`
	P99US               int64         `json:"p99_us"`
	EffectiveThroughput float64       `json:"effective_throughput_qps"`
	SenderLagMaxUS      int64         `json:"sender_lag_max_us"`
	StartedAt           time.Time     `json:"started_at"`
	FinishedAt          time.Time     `json:"finished_at"`
	ResourceSampleCount int           `json:"resource_sample_count"`
}

type resourceSample struct {
	Timestamp           time.Time `json:"timestamp"`
	PID                 int       `json:"pid"`
	UserTicks           uint64    `json:"user_ticks"`
	SystemTicks         uint64    `json:"system_ticks"`
	ClockTicksPerSecond int64     `json:"clock_ticks_per_second"`
	UserSeconds         float64   `json:"user_seconds"`
	SystemSeconds       float64   `json:"system_seconds"`
	RSSKiB              int64     `json:"rss_kib"`
}

type runStats struct {
	mu          sync.Mutex
	counters    stageCounters
	latenciesUS []int64
	maxLagUS    int64
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
	case "validate-binary":
		err = validateBinary(os.Args[2:])
	case "version":
		fmt.Println("phase5a-baseline-helper/v1")
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
	fmt.Fprintln(os.Stderr, "usage: phase5a-baseline-helper {fixture|run|validate-binary|version}")
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

type fixtureOptions struct {
	network     string
	addr        string
	upstreamID  string
	counterPath string
	delayMS     int
}

func runFixture(args []string) error {
	fs := flag.NewFlagSet("fixture", flag.ContinueOnError)
	opts := fixtureOptions{}
	fs.StringVar(&opts.network, "network", "udp", "udp or tcp")
	fs.StringVar(&opts.addr, "addr", "", "listen address")
	fs.StringVar(&opts.upstreamID, "upstream-id", "forward", "committed fixture identity")
	fs.StringVar(&opts.counterPath, "counter", "", "counter JSON path")
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
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGINT, syscall.SIGTERM)
	defer signal.Stop(stop)

	var wg sync.WaitGroup
	if opts.network == "udp" {
		conn, err := net.ListenUDP("udp", mustResolveUDP(opts.addr))
		if err != nil {
			return fmt.Errorf("listen UDP fixture: %w", err)
		}
		defer conn.Close()
		wg.Add(1)
		go func() {
			defer wg.Done()
			serveUDPFixture(conn, opts, counts)
		}()
	} else {
		listener, err := net.Listen("tcp", opts.addr)
		if err != nil {
			return fmt.Errorf("listen TCP fixture: %w", err)
		}
		defer listener.Close()
		wg.Add(1)
		go func() {
			defer wg.Done()
			serveTCPFixture(listener, opts, counts)
		}()
	}

	<-stop
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
	upstream string
	path     string
	values   map[string]int64
}

func (c *counterStore) add(qname string, qtype uint16) error {
	c.mu.Lock()
	c.values[strings.ToLower(dns.Fqdn(qname))+"|"+dns.TypeToString[qtype]]++
	c.mu.Unlock()
	return c.write()
}

func (c *counterStore) write() error {
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

func serveUDPFixture(conn *net.UDPConn, opts fixtureOptions, counts *counterStore) {
	buf := make([]byte, dns.MaxMsgSize)
	for {
		n, remote, err := conn.ReadFromUDP(buf)
		if err != nil {
			return
		}
		resp, ok := fixtureResponse(buf[:n], opts, counts)
		if !ok {
			continue
		}
		if opts.delayMS > 0 {
			time.Sleep(time.Duration(opts.delayMS) * time.Millisecond)
		}
		_, _ = conn.WriteToUDP(resp, remote)
	}
}

func serveTCPFixture(listener net.Listener, opts fixtureOptions, counts *counterStore) {
	for {
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		go func() {
			defer conn.Close()
			var length uint16
			if err := binary.Read(conn, binary.BigEndian, &length); err != nil || length == 0 || length > dns.MaxMsgSize {
				return
			}
			query := make([]byte, length)
			if _, err := io.ReadFull(conn, query); err != nil {
				return
			}
			resp, ok := fixtureResponse(query, opts, counts)
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

func fixtureResponse(wire []byte, opts fixtureOptions, counts *counterStore) ([]byte, bool) {
	query := new(dns.Msg)
	if err := query.Unpack(wire); err != nil || len(query.Question) != 1 {
		return nil, false
	}
	question := query.Question[0]
	_ = counts.add(question.Name, question.Qtype)
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
	return executeStage(stageOptions{
		workload:  cases,
		addr:      *addr,
		transport: *transport,
		scenario:  *scenario,
		stage:     *stageName,
		qps:       *qps,
		duration:  *duration,
		deadline:  *deadline,
		lateDrain: *lateDrain,
		resultDir: *resultDir,
		sutPID:    *sutPID,
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

type stageOptions struct {
	workload  []workloadCase
	addr      string
	transport string
	scenario  string
	stage     string
	qps       float64
	duration  time.Duration
	deadline  time.Duration
	lateDrain time.Duration
	resultDir string
	sutPID    int
}

func executeStage(opts stageOptions) error {
	started := time.Now().UTC()
	stats := new(runStats)
	resourcePath := filepath.Join(opts.resultDir, "resource-samples.jsonl")
	resourceCount := 0
	stopSamples := make(chan struct{})
	var sampleWG sync.WaitGroup
	if opts.sutPID > 0 {
		sampleWG.Add(1)
		go func() {
			defer sampleWG.Done()
			resourceCount = sampleProcess(opts.sutPID, resourcePath, stopSamples)
		}()
	}

	interval := time.Duration(float64(time.Second) / opts.qps)
	if interval <= 0 {
		interval = time.Nanosecond
	}
	end := time.Now().Add(opts.duration)
	next := time.Now()
	for index := 0; next.Before(end); index++ {
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
		stats.mu.Lock()
		stats.counters.Scheduled++
		stats.mu.Unlock()
		go executeRequest(opts, caseValue, stats)
		next = next.Add(interval)
	}

	// executeRequest owns its own bounded deadline. The late drain gives the
	// response association logic a deterministic window before timeout.
	grace := opts.deadline + opts.lateDrain + 50*time.Millisecond
	time.Sleep(grace)
	close(stopSamples)
	sampleWG.Wait()
	finished := time.Now().UTC()
	stats.mu.Lock()
	result := stageResult{
		Stage: opts.stage, Scenario: opts.scenario, Transport: opts.transport,
		TargetQPS: opts.qps, DurationMS: opts.duration.Milliseconds(),
		RequestDeadlineMS: int(opts.deadline / time.Millisecond),
		LateDrainMS:       int(opts.lateDrain / time.Millisecond), Counters: stats.counters,
		LatencySamplesUS: append([]int64(nil), stats.latenciesUS...),
		SenderLagMaxUS:   stats.maxLagUS, StartedAt: started, FinishedAt: finished,
		ResourceSampleCount: resourceCount,
	}
	stats.mu.Unlock()
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
	return closeErr
}

func executeRequest(opts stageOptions, c workloadCase, stats *runStats) {
	qtype, ok := dns.StringToType[strings.ToUpper(c.QType)]
	if !ok {
		recordTerminal(stats, "protocol")
		return
	}
	query := new(dns.Msg)
	query.SetQuestion(dns.Fqdn(c.QName), qtype)
	query.Id = uint16(time.Now().UnixNano())
	payload, err := query.Pack()
	if err != nil {
		recordTerminal(stats, "protocol")
		return
	}
	sendAt := time.Now()
	var wire []byte
	var sent bool
	if opts.transport == "udp" {
		wire, sent, err = exchangeUDP(opts.addr, payload, opts.deadline+opts.lateDrain)
	} else {
		wire, sent, err = exchangeTCP(opts.addr, payload, opts.deadline+opts.lateDrain)
	}
	if sent {
		stats.mu.Lock()
		stats.counters.Sent++
		stats.mu.Unlock()
	}
	if err != nil {
		if errors.Is(err, os.ErrDeadlineExceeded) || errors.Is(err, errTimeout) {
			recordTerminal(stats, "timeout")
		} else if ne, ok := err.(net.Error); ok && ne.Timeout() {
			recordTerminal(stats, "timeout")
		} else {
			recordTerminal(stats, "transport")
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
		return
	}
	if !responseMatches(response, query, c) {
		recordTerminal(stats, "wrong")
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

func responseMatches(response, query *dns.Msg, c workloadCase) bool {
	if response.Id != query.Id || len(response.Question) != 1 {
		return false
	}
	question := response.Question[0]
	if !strings.EqualFold(question.Name, query.Question[0].Name) || question.Qtype != query.Question[0].Qtype || response.Rcode != c.ExpectedRCode {
		return false
	}
	if c.ExpectedRCode != dns.RcodeSuccess {
		return len(response.Answer) == 0
	}
	if c.ExpectedAnswer == "" || question.Qtype != dns.TypeA {
		return false
	}
	for _, rr := range response.Answer {
		if a, ok := rr.(*dns.A); ok && a.A.String() == c.ExpectedAnswer {
			return c.ExpectedAnswerClass == "A"
		}
	}
	return false
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

func readResourceSample(pid int) (resourceSample, bool) {
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
	for _, line := range strings.Split(string(status), "\n") {
		if strings.HasPrefix(line, "VmRSS:") {
			parts := strings.Fields(line)
			if len(parts) >= 2 {
				rss, _ = strconv.ParseInt(parts[1], 10, 64)
			}
		}
	}
	hz := int64(100)
	if out, err := exec.Command("getconf", "CLK_TCK").Output(); err == nil {
		if parsed, parseErr := strconv.ParseInt(strings.TrimSpace(string(out)), 10, 64); parseErr == nil && parsed > 0 {
			hz = parsed
		}
	}
	return resourceSample{Timestamp: time.Now().UTC(), PID: pid, UserTicks: utime, SystemTicks: stime, ClockTicksPerSecond: hz, UserSeconds: float64(utime) / float64(hz), SystemSeconds: float64(stime) / float64(hz), RSSKiB: rss}, true
}

func sampleProcess(pid int, path string, stop <-chan struct{}) int {
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return 0
	}
	defer f.Close()
	enc := json.NewEncoder(f)
	count := 0
	write := func() bool {
		sample, ok := readResourceSample(pid)
		if !ok {
			return false
		}
		if enc.Encode(sample) != nil {
			return false
		}
		count++
		return true
	}
	write()
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-stop:
			return count
		case <-ticker.C:
			write()
		}
	}
}

func sha256File(path string) (string, error) {
	cmd := exec.Command("shasum", "-a", "256", path)
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("sha256 %s: %w", path, err)
	}
	fields := strings.Fields(string(out))
	if len(fields) == 0 {
		return "", fmt.Errorf("empty sha256 output for %s", path)
	}
	return fields[0], nil
}

func writeJSON(w io.Writer, value any) error {
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	return enc.Encode(value)
}
