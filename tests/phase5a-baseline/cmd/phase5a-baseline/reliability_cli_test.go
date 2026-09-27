package main

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/miekg/dns"
)

func TestReliabilityCLIUDPAndOfflineAssessment(t *testing.T) {
	packetConn, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer packetConn.Close()
	stop := make(chan struct{})
	defer close(stop)
	go serveReliabilityUDP(packetConn, stop, "192.0.2.55", 0)

	root := t.TempDir()
	workload := filepath.Join(root, "workload.jsonl")
	writeReliabilityWorkload(t, workload, "udp", "192.0.2.55")
	resultDir := filepath.Join(root, "raw")
	if err := runReliabilityCommand([]string{
		"--workload", workload,
		"--scenario", "w1",
		"--transport", "udp",
		"--addr", packetConn.LocalAddr().String(),
		"--result", resultDir,
		"--slots", "3",
		"--target-qps", "100",
		"--request-deadline-ms", "500",
		"--late-drain-ms", "20",
		"--workers", "2",
		"--in-flight", "2",
		"--dispatch-queue", "2",
		"--evidence-queue", "4",
	}); err != nil {
		t.Fatal(err)
	}
	raw, err := loadReliabilityRaw(filepath.Join(resultDir, "reliability-raw.json"))
	if err != nil {
		t.Fatal(err)
	}
	if len(raw.ControlRecords) != 3 || !raw.EvidenceValid || !raw.LoadValid || raw.Accounting.DNSSent != 3 {
		t.Fatalf("unexpected UDP reliability raw bundle: %+v", raw)
	}
	derived := filepath.Join(root, "derived")
	if err := assessReliabilityCommand([]string{"--raw", filepath.Join(resultDir, "reliability-raw.json"), "--output", derived}); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(derived, "assessment.json"))
	if err != nil {
		t.Fatal(err)
	}
	var assessment reliabilityAssessment
	if err := json.Unmarshal(data, &assessment); err != nil {
		t.Fatal(err)
	}
	if assessment.Verdict != reliabilityVerdictPass || len(assessment.LatencyViews) != 3 || assessment.LatencyViews[1].FailureDenominator != 3 || assessment.LatencyViews[2].FailureDenominator != 3 || assessment.GoodputQPS <= 0 {
		t.Fatalf("unexpected assessment: %+v", assessment)
	}
}

func TestReliabilityOfficialStageProjectionWritesRawAssessmentInputs(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("official stage projection uses Linux process sampling")
	}
	packetConn, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer packetConn.Close()
	stop := make(chan struct{})
	defer close(stop)
	go serveReliabilityUDP(packetConn, stop, "192.0.2.55", 0)

	sut := exec.Command("sleep", "30")
	if err := sut.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = sut.Process.Kill()
		_ = sut.Wait()
	}()

	root := t.TempDir()
	workload := filepath.Join(root, "workload.jsonl")
	writeReliabilityWorkload(t, workload, "udp", "192.0.2.55")
	rawDir := filepath.Join(root, "raw")
	stageDir := filepath.Join(root, "stage")
	ledger := filepath.Join(stageDir, "requests.jsonl")
	if err := runReliabilityCommand([]string{
		"--workload", workload, "--scenario", "w1", "--transport", "udp", "--addr", packetConn.LocalAddr().String(),
		"--result", rawDir, "--stage-output", stageDir, "--stage", "normal-reference", "--stage-duration-ms", "1000",
		"--run-id", "official-stage", "--fixture-session-id", "fixtures", "--sut-pid", strconv.Itoa(sut.Process.Pid),
		"--fixture-pid", strconv.Itoa(os.Getpid()), "--request-ledger", ledger, "--slots", "3", "--target-qps", "100",
	}); err != nil {
		t.Fatal(err)
	}
	raw, err := loadReliabilityRaw(filepath.Join(rawDir, "reliability-raw.json"))
	if err != nil {
		t.Fatal(err)
	}
	if !raw.LoadValid || !raw.EvidenceValid || raw.Accounting.DNSSent != 3 {
		t.Fatalf("unexpected official raw bundle: %+v", raw)
	}
	stage, err := readStageResult(filepath.Join(stageDir, "stages.jsonl"), "normal-reference")
	if err != nil {
		t.Fatal(err)
	}
	if stage.Counters.Scheduled != 3 || stage.Counters.Sent != 3 || stage.Counters.CorrectOnTime != 3 || stage.ResourceSampleCounts["sut"] == 0 || stage.ResourceSampleCounts["fixture-1"] == 0 {
		t.Fatalf("unexpected compatibility stage projection: %+v", stage)
	}
	requests, err := readRequestLedger(ledger)
	if err != nil {
		t.Fatal(err)
	}
	if len(requests) != 3 || requests[0].RequestSeq != 1 || requests[2].Outcome != "correct_on_time" {
		t.Fatalf("unexpected projected request ledger: %+v", requests)
	}
	if _, err := os.Stat(filepath.Join(rawDir, "resource-samples.jsonl")); err != nil {
		t.Fatalf("resource evidence was not retained: %v", err)
	}
}

func TestReliabilityCLITCPStrictWrongAnswerIsCorrectnessFailure(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	stop := make(chan struct{})
	defer close(stop)
	go serveReliabilityTCP(listener, stop, "192.0.2.99")

	root := t.TempDir()
	workload := filepath.Join(root, "workload.jsonl")
	writeReliabilityWorkload(t, workload, "tcp", "192.0.2.55")
	resultDir := filepath.Join(root, "raw")
	if err := runReliabilityCommand([]string{
		"--workload", workload,
		"--scenario", "w1",
		"--transport", "tcp",
		"--addr", listener.Addr().String(),
		"--result", resultDir,
		"--slots", "1",
		"--target-qps", "10",
	}); err != nil {
		t.Fatal(err)
	}
	derived := filepath.Join(root, "derived")
	if err := assessReliabilityCommand([]string{"--raw", filepath.Join(resultDir, "reliability-raw.json"), "--output", derived}); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(derived, "assessment.json"))
	if err != nil {
		t.Fatal(err)
	}
	var assessment reliabilityAssessment
	if err := json.Unmarshal(data, &assessment); err != nil {
		t.Fatal(err)
	}
	if assessment.Verdict != reliabilityVerdictCorrectness {
		t.Fatalf("wrong strict DNS answer must be correctness failure: %+v", assessment)
	}
}

func TestReliabilityCLISlowLoopbackResponseIsServiceDegradation(t *testing.T) {
	packetConn, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer packetConn.Close()
	stop := make(chan struct{})
	defer close(stop)
	go serveReliabilityUDP(packetConn, stop, "192.0.2.55", 20*time.Millisecond)
	root := t.TempDir()
	workload := filepath.Join(root, "workload.jsonl")
	writeReliabilityWorkload(t, workload, "udp", "192.0.2.55")
	resultDir := filepath.Join(root, "raw")
	if err := runReliabilityCommand([]string{"--workload", workload, "--scenario", "w1", "--transport", "udp", "--addr", packetConn.LocalAddr().String(), "--result", resultDir, "--slots", "1", "--target-qps", "1", "--request-deadline-ms", "5", "--late-drain-ms", "100"}); err != nil {
		t.Fatal(err)
	}
	raw, err := loadReliabilityRaw(filepath.Join(resultDir, "reliability-raw.json"))
	if err != nil {
		t.Fatal(err)
	}
	if raw.ControlRecords[0].Terminal != reliabilityTerminalCorrectLate {
		t.Fatalf("slow response must remain a complete late service outcome: %+v", raw.ControlRecords[0])
	}
	derived := filepath.Join(root, "derived")
	if err := assessReliabilityCommand([]string{"--raw", filepath.Join(resultDir, "reliability-raw.json"), "--output", derived}); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(derived, "assessment.json"))
	if err != nil {
		t.Fatal(err)
	}
	var assessment reliabilityAssessment
	if err := json.Unmarshal(data, &assessment); err != nil {
		t.Fatal(err)
	}
	if assessment.Verdict != reliabilityVerdictDegraded {
		t.Fatalf("late response must be service degradation: %+v", assessment)
	}
}

func TestReliabilityCLIAbsentLoopbackResponseIsTimeout(t *testing.T) {
	packetConn, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer packetConn.Close()
	root := t.TempDir()
	workload := filepath.Join(root, "workload.jsonl")
	writeReliabilityWorkload(t, workload, "udp", "192.0.2.55")
	resultDir := filepath.Join(root, "raw")
	if err := runReliabilityCommand([]string{"--workload", workload, "--scenario", "w1", "--transport", "udp", "--addr", packetConn.LocalAddr().String(), "--result", resultDir, "--slots", "1", "--target-qps", "1", "--request-deadline-ms", "10", "--late-drain-ms", "5"}); err != nil {
		t.Fatal(err)
	}
	raw, err := loadReliabilityRaw(filepath.Join(resultDir, "reliability-raw.json"))
	if err != nil {
		t.Fatal(err)
	}
	if raw.ControlRecords[0].Terminal != reliabilityTerminalTimeout || !raw.ControlRecords[0].DNSSent {
		t.Fatalf("absent response must be a post-send timeout: %+v", raw.ControlRecords[0])
	}
}

func TestReliabilityCLIConnectionRefusalIsPreSendServiceFailure(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	address := listener.Addr().String()
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
	root := t.TempDir()
	workload := filepath.Join(root, "workload.jsonl")
	writeReliabilityWorkload(t, workload, "tcp", "192.0.2.55")
	resultDir := filepath.Join(root, "raw")
	if err := runReliabilityCommand([]string{"--workload", workload, "--scenario", "w1", "--transport", "tcp", "--addr", address, "--result", resultDir, "--slots", "1", "--target-qps", "1", "--request-deadline-ms", "50"}); err != nil {
		t.Fatal(err)
	}
	raw, err := loadReliabilityRaw(filepath.Join(resultDir, "reliability-raw.json"))
	if err != nil {
		t.Fatal(err)
	}
	if raw.ControlRecords[0].Terminal != reliabilityTerminalFailedBeforeDNSSend || raw.ControlRecords[0].DNSSent {
		t.Fatalf("connection refusal must be pre-send: %+v", raw.ControlRecords[0])
	}
}

func TestReliabilityCLIPortCanBeReboundAfterRun(t *testing.T) {
	packetConn, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	address := packetConn.LocalAddr().String()
	stop := make(chan struct{})
	go serveReliabilityUDP(packetConn, stop, "192.0.2.55", 0)
	root := t.TempDir()
	workload := filepath.Join(root, "workload.jsonl")
	writeReliabilityWorkload(t, workload, "udp", "192.0.2.55")
	if err := runReliabilityCommand([]string{"--workload", workload, "--scenario", "w1", "--transport", "udp", "--addr", address, "--result", filepath.Join(root, "first"), "--slots", "1", "--target-qps", "1"}); err != nil {
		t.Fatal(err)
	}
	close(stop)
	if err := packetConn.Close(); err != nil {
		t.Fatal(err)
	}
	rebound, err := net.ListenPacket("udp", address)
	if err != nil {
		t.Fatalf("loopback port was not reusable after run: %v", err)
	}
	defer rebound.Close()
	secondStop := make(chan struct{})
	defer close(secondStop)
	go serveReliabilityUDP(rebound, secondStop, "192.0.2.55", 0)
	if err := runReliabilityCommand([]string{"--workload", workload, "--scenario", "w1", "--transport", "udp", "--addr", address, "--result", filepath.Join(root, "second"), "--slots", "1", "--target-qps", "1"}); err != nil {
		t.Fatal(err)
	}
}

func TestReliabilityAssessCLIValidatesHistoricalArchiveIdentity(t *testing.T) {
	rootOutput, err := exec.Command("git", "rev-parse", "--show-toplevel").Output()
	if err != nil {
		t.Fatal(err)
	}
	repoRoot := strings.TrimSpace(string(rootOutput))
	commitOutput, err := exec.Command("git", "-C", repoRoot, "rev-parse", "HEAD").Output()
	if err != nil {
		t.Fatal(err)
	}
	commit := strings.TrimSpace(string(commitOutput))
	source, err := exec.Command("git", "-C", repoRoot, "show", commit+":AGENTS.md").Output()
	if err != nil {
		t.Fatal(err)
	}
	hash := sha256.Sum256(source)
	rawDir := t.TempDir()
	rawPath := filepath.Join(rawDir, "raw.json")
	record := reliabilitySlotRecord{SlotID: 0, PlannedOffsetUS: 0, FinishOffsetUS: ptrInt64(1), DNSSent: true, Terminal: reliabilityTerminalCorrectOnTime}
	raw := reliabilityRawBundle{SchemaVersion: reliabilitySchemaVersion, RunID: "archive", Config: reliabilityRunConfig{RequestDeadline: time.Second}, ControlRecords: []reliabilitySlotRecord{record}, EvidenceRecords: []reliabilitySlotRecord{record}}
	data, err := json.Marshal(raw)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(rawPath, data, 0644); err != nil {
		t.Fatal(err)
	}
	validOutput := filepath.Join(rawDir, "valid")
	if err := assessReliabilityCommand([]string{"--raw", rawPath, "--output", validOutput, "--repo-root", repoRoot, "--source-commit", commit, "--source-path", "AGENTS.md", "--source-sha256", hex.EncodeToString(hash[:])}); err != nil {
		t.Fatal(err)
	}
	badOutput := filepath.Join(rawDir, "bad")
	if err := assessReliabilityCommand([]string{"--raw", rawPath, "--output", badOutput, "--repo-root", repoRoot, "--source-commit", commit, "--source-path", "AGENTS.md", "--source-sha256", strings.Repeat("0", 64)}); err == nil || !strings.Contains(err.Error(), "hash mismatch") {
		t.Fatalf("tampered archive hash must fail closed: %v", err)
	}
	missingOutput := filepath.Join(rawDir, "missing")
	if err := assessReliabilityCommand([]string{"--raw", rawPath, "--output", missingOutput, "--repo-root", repoRoot, "--source-commit", commit, "--source-path", "missing.md", "--source-sha256", strings.Repeat("0", 64)}); err == nil || !strings.Contains(err.Error(), "object is missing") {
		t.Fatalf("missing historical path must fail closed: %v", err)
	}
	ambiguousOutput := filepath.Join(rawDir, "ambiguous")
	if err := assessReliabilityCommand([]string{"--raw", rawPath, "--output", ambiguousOutput, "--repo-root", filepath.Join(repoRoot, "tests"), "--source-commit", commit, "--source-path", "AGENTS.md", "--source-sha256", hex.EncodeToString(hash[:])}); err == nil || !strings.Contains(err.Error(), "explicit Git root") {
		t.Fatalf("ambiguous repo root must fail closed: %v", err)
	}
}

func TestReliabilityPublicSubprocessAndShellRunner(t *testing.T) {
	rootOutput, err := exec.Command("git", "rev-parse", "--show-toplevel").Output()
	if err != nil {
		t.Fatal(err)
	}
	repoRoot := strings.TrimSpace(string(rootOutput))
	helpDir := t.TempDir()
	helper := filepath.Join(helpDir, "phase5a-baseline-helper")
	build := exec.Command("go", "build", "-trimpath", "-o", helper, "./tests/phase5a-baseline/cmd/phase5a-baseline")
	build.Dir = repoRoot
	if output, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build helper: %v\n%s", err, output)
	}
	packetConn, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer packetConn.Close()
	stop := make(chan struct{})
	defer close(stop)
	go serveReliabilityUDP(packetConn, stop, "192.0.2.55", 0)
	root := t.TempDir()
	workload := filepath.Join(root, "workload.jsonl")
	writeReliabilityWorkload(t, workload, "udp", "192.0.2.55")
	rawDir := filepath.Join(root, "subprocess-raw")
	run := exec.Command(helper, "reliability-run", "--workload", workload, "--scenario", "w1", "--transport", "udp", "--addr", packetConn.LocalAddr().String(), "--result", rawDir, "--slots", "1", "--target-qps", "1")
	run.Dir = repoRoot
	if output, err := run.CombinedOutput(); err != nil || !strings.Contains(string(output), "reliability-raw.json") {
		t.Fatalf("public reliability-run failed: %v\n%s", err, output)
	}
	derived := filepath.Join(root, "subprocess-derived")
	assess := exec.Command(helper, "reliability-assess", "--raw", filepath.Join(rawDir, "reliability-raw.json"), "--output", derived)
	assess.Dir = repoRoot
	if output, err := assess.CombinedOutput(); err != nil || !strings.Contains(string(output), "assessment.json") {
		t.Fatalf("public reliability-assess failed: %v\n%s", err, output)
	}
	shellResult := filepath.Join(root, "shell-raw")
	shell := exec.Command("bash", filepath.Join(repoRoot, "scripts/run-phase5a-reliability.sh"))
	shell.Dir = repoRoot
	shell.Env = append(os.Environ(), "WORKLOAD="+workload, "SCENARIO=w1", "TRANSPORT=udp", "ADDR="+packetConn.LocalAddr().String(), "RESULT_DIR="+shellResult, "HELPER_BINARY="+helper, "SLOTS=1", "TARGET_QPS=1")
	if output, err := shell.CombinedOutput(); err != nil || !strings.Contains(string(output), "reliability-raw.json") {
		t.Fatalf("public reliability shell runner failed: %v\n%s", err, output)
	}
	aliasResult := filepath.Join(root, "shell-alias-rejected")
	alias := exec.Command("bash", filepath.Join(repoRoot, "scripts/run-phase5a-reliability.sh"))
	alias.Dir = repoRoot
	alias.Env = append(os.Environ(), "WORKLOAD="+workload, "SCENARIO=w1-udp", "TRANSPORT=udp", "ADDR="+packetConn.LocalAddr().String(), "RESULT_DIR="+aliasResult, "HELPER_BINARY="+helper)
	if output, err := alias.CombinedOutput(); err == nil || !strings.Contains(string(output), "unsupported SCENARIO") {
		t.Fatalf("reliability shell runner must reject transport-encoded scenario aliases: %v\n%s", err, output)
	}
}

func writeReliabilityWorkload(t *testing.T, path, transport, expectedAnswer string) {
	t.Helper()
	data := []byte(`{"case_id":"case","scenario":"w1","transport":"` + transport + `","qname":"case.test.","qtype":"A","expected_rcode":0,"expected_answer_class":"A","expected_answer":"` + expectedAnswer + `","expected_route_class":"forward","request_deadline_ms":500,"weight":1}` + "\n")
	if err := os.WriteFile(path, data, 0644); err != nil {
		t.Fatal(err)
	}
}

func serveReliabilityUDP(conn net.PacketConn, stop <-chan struct{}, answer string, delay time.Duration) {
	buffer := make([]byte, dns.MaxMsgSize)
	for {
		_ = conn.SetReadDeadline(time.Now().Add(50 * time.Millisecond))
		n, addr, err := conn.ReadFrom(buffer)
		if err != nil {
			select {
			case <-stop:
				return
			default:
				continue
			}
		}
		query := new(dns.Msg)
		if err := query.Unpack(buffer[:n]); err != nil {
			continue
		}
		if delay > 0 {
			time.Sleep(delay)
		}
		response := new(dns.Msg)
		response.SetReply(query)
		response.Answer = []dns.RR{&dns.A{Hdr: dns.RR_Header{Name: query.Question[0].Name, Rrtype: dns.TypeA, Class: dns.ClassINET}, A: net.ParseIP(answer).To4()}}
		wire, _ := response.Pack()
		_, _ = conn.WriteTo(wire, addr)
	}
}

func serveReliabilityTCP(listener net.Listener, stop <-chan struct{}, answer string) {
	for {
		_ = listener.(*net.TCPListener).SetDeadline(time.Now().Add(50 * time.Millisecond))
		conn, err := listener.Accept()
		if err != nil {
			select {
			case <-stop:
				return
			default:
				continue
			}
		}
		go func() {
			defer conn.Close()
			var length uint16
			if err := binary.Read(conn, binary.BigEndian, &length); err != nil {
				return
			}
			wire := make([]byte, length)
			if _, err := io.ReadFull(conn, wire); err != nil {
				return
			}
			query := new(dns.Msg)
			if err := query.Unpack(wire); err != nil {
				return
			}
			response := new(dns.Msg)
			response.SetReply(query)
			response.Answer = []dns.RR{&dns.A{Hdr: dns.RR_Header{Name: query.Question[0].Name, Rrtype: dns.TypeA, Class: dns.ClassINET}, A: net.ParseIP(answer).To4()}}
			body, _ := response.Pack()
			frame := make([]byte, 2+len(body))
			binary.BigEndian.PutUint16(frame, uint16(len(body)))
			copy(frame[2:], body)
			_, _ = conn.Write(frame)
		}()
	}
}
