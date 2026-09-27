package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/miekg/dns"
)

type reliabilityRawBundle struct {
	SchemaVersion   string                  `json:"schema_version"`
	RunID           string                  `json:"run_id"`
	Scenario        string                  `json:"scenario,omitempty"`
	Transport       string                  `json:"transport,omitempty"`
	StartedAt       time.Time               `json:"started_at"`
	FinishedAt      time.Time               `json:"finished_at"`
	Config          reliabilityRunConfig    `json:"config"`
	ControlRecords  []reliabilitySlotRecord `json:"control_records"`
	EvidenceRecords []reliabilitySlotRecord `json:"evidence_records"`
	Accounting      reliabilityAccounting   `json:"accounting"`
	EvidenceValid   bool                    `json:"evidence_valid"`
	LoadValid       bool                    `json:"load_valid"`
	WriterError     string                  `json:"writer_error,omitempty"`
	CleanupFailure  string                  `json:"cleanup_failure,omitempty"`
	MissingSlotIDs  []int64                 `json:"missing_slot_ids,omitempty"`
	Windows         []reliabilityWindow     `json:"windows,omitempty"`
}

type reliabilityRunResult struct {
	Raw        reliabilityRawBundle
	Accounting reliabilityAccounting
}

type reliabilityJob struct {
	slotID    int64
	planned   time.Duration
	caseValue workloadCase
}

type reliabilityRecordSink interface {
	Write(context.Context, reliabilitySlotRecord) error
}

type memoryReliabilitySink struct {
	mu      sync.Mutex
	records []reliabilitySlotRecord
}

func newMemoryReliabilitySink() *memoryReliabilitySink { return &memoryReliabilitySink{} }

func (s *memoryReliabilitySink) Write(ctx context.Context, record reliabilitySlotRecord) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	default:
	}
	s.mu.Lock()
	s.records = append(s.records, record)
	s.mu.Unlock()
	return nil
}

func (s *memoryReliabilitySink) Records() []reliabilitySlotRecord {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]reliabilitySlotRecord(nil), s.records...)
}

type reliabilityEvidenceWriter struct {
	sink           reliabilityRecordSink
	queue          chan reliabilitySlotRecord
	cleanupTimeout time.Duration
	done           chan struct{}
	failed         chan struct{}
	ctx            context.Context
	cancel         context.CancelFunc
	cancelOnce     sync.Once
	closeOnce      sync.Once
	failOnce       sync.Once
	mu             sync.Mutex
	started        bool
	closed         bool
	persisted      []reliabilitySlotRecord
	writerErr      error
}

func newReliabilityEvidenceWriter(sink reliabilityRecordSink, capacity int, cleanupTimeout time.Duration) *reliabilityEvidenceWriter {
	if capacity < 1 {
		capacity = 1
	}
	if cleanupTimeout <= 0 {
		cleanupTimeout = time.Second
	}
	return &reliabilityEvidenceWriter{
		sink:           sink,
		queue:          make(chan reliabilitySlotRecord, capacity),
		cleanupTimeout: cleanupTimeout,
		done:           make(chan struct{}),
		failed:         make(chan struct{}),
	}
}

func (w *reliabilityEvidenceWriter) Start(ctx context.Context) {
	w.mu.Lock()
	if w.started {
		w.mu.Unlock()
		return
	}
	if ctx == nil {
		ctx = context.Background()
	}
	writerCtx, cancel := context.WithCancel(ctx)
	w.ctx = writerCtx
	w.cancel = cancel
	w.started = true
	w.mu.Unlock()
	go w.loop()
}

func (w *reliabilityEvidenceWriter) Enqueue(record reliabilitySlotRecord) bool {
	select {
	case <-w.failed:
		return false
	default:
	}
	w.mu.Lock()
	closed := w.closed
	w.mu.Unlock()
	if closed {
		return false
	}
	select {
	case w.queue <- record:
		return true
	default:
		return false
	}
}

func (w *reliabilityEvidenceWriter) loop() {
	defer close(w.done)
	for record := range w.queue {
		w.mu.Lock()
		ctx := w.ctx
		w.mu.Unlock()
		err := w.sink.Write(ctx, record)
		w.mu.Lock()
		if err != nil && w.writerErr == nil {
			w.writerErr = err
			w.failOnce.Do(func() { close(w.failed) })
		}
		if err == nil {
			w.persisted = append(w.persisted, record)
		}
		w.mu.Unlock()
		if err != nil {
			w.cancelContext()
		}
	}
}

func (w *reliabilityEvidenceWriter) cancelContext() {
	w.mu.Lock()
	cancel := w.cancel
	w.mu.Unlock()
	if cancel != nil {
		w.cancelOnce.Do(cancel)
	}
}

func (w *reliabilityEvidenceWriter) Failed() bool {
	select {
	case <-w.failed:
		return true
	default:
		return false
	}
}

func (w *reliabilityEvidenceWriter) Close() {
	w.closeOnce.Do(func() {
		w.mu.Lock()
		w.closed = true
		w.mu.Unlock()
		close(w.queue)
		if w.Failed() {
			w.cancelContext()
		}
	})
}

func (w *reliabilityEvidenceWriter) Wait() error {
	w.mu.Lock()
	started := w.started
	w.mu.Unlock()
	if !started {
		return nil
	}
	timer := time.NewTimer(w.cleanupTimeout)
	defer timer.Stop()
	select {
	case <-w.done:
		w.mu.Lock()
		err := w.writerErr
		w.mu.Unlock()
		w.cancelContext()
		if err != nil {
			return fmt.Errorf("writer_error: %w", err)
		}
		return nil
	case <-timer.C:
		w.cancelContext()
		return errors.New("cleanup_failure: evidence writer did not stop within cleanup budget")
	}
}

func (w *reliabilityEvidenceWriter) Persisted() []reliabilitySlotRecord {
	w.mu.Lock()
	defer w.mu.Unlock()
	return append([]reliabilitySlotRecord(nil), w.persisted...)
}

func (w *reliabilityEvidenceWriter) WriterError() error {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.writerErr
}

func runReliability(ctx context.Context, config reliabilityRunConfig, workload []workloadCase, exchange reliabilityExchange, clock reliabilityClock, sink reliabilityRecordSink) (reliabilityRunResult, error) {
	if config.CleanupTimeout <= 0 {
		config.CleanupTimeout = time.Second
	}
	if err := validateReliabilityRunConfig(config); err != nil {
		return reliabilityRunResult{}, err
	}
	if len(workload) == 0 {
		return reliabilityRunResult{}, errors.New("reliability workload is empty")
	}
	if exchange == nil || clock == nil || sink == nil {
		return reliabilityRunResult{}, errors.New("reliability run requires exchange, clock, and sink")
	}
	startedWall := clock.WallNow().UTC()
	writer := newReliabilityEvidenceWriter(sink, config.Limits.EvidenceQueue, config.CleanupTimeout)
	writer.Start(ctx)

	var recordsMu sync.Mutex
	controlRecords := make([]reliabilitySlotRecord, 0, config.Slots)
	evidenceValid := true
	evidenceFailed := false
	controlBytes := int64(0)
	missing := make([]int64, 0)
	appendRecord := func(record reliabilitySlotRecord) {
		if record.SchemaVersion == "" {
			record.SchemaVersion = reliabilitySchemaVersion
		}
		controlRecord := record
		evidenceEligible := true
		if reliabilityRecordSize(record) > config.Limits.RecordBytes {
			controlRecord = compactReliabilityRecord(record)
			if reliabilityRecordSize(controlRecord) > config.Limits.RecordBytes {
				controlRecord.WindowID = ""
				controlRecord.Error = ""
			}
			evidenceEligible = false
		}
		recordsMu.Lock()
		controlRecords = append(controlRecords, controlRecord)
		controlBytes += int64(reliabilityRecordSize(controlRecord))
		if !evidenceEligible || controlBytes > reliabilityMaxControlBytes {
			evidenceFailed = true
			evidenceValid = false
			missing = append(missing, controlRecord.SlotID)
		} else if evidenceFailed {
			evidenceValid = false
			missing = append(missing, controlRecord.SlotID)
		} else {
			if !writer.Enqueue(record) {
				evidenceValid = false
				evidenceFailed = true
				missing = append(missing, controlRecord.SlotID)
				writer.cancelContext()
			}
		}
		if writer.Failed() {
			evidenceValid = false
			evidenceFailed = true
			writer.cancelContext()
		}
		recordsMu.Unlock()
	}
	isEvidenceFailed := func() bool {
		if writer.Failed() {
			return true
		}
		recordsMu.Lock()
		defer recordsMu.Unlock()
		return evidenceFailed
	}

	dispatch := make(chan reliabilityJob, config.Limits.DispatchQueue)
	inFlight := make(chan struct{}, config.Limits.InFlight)
	var workers sync.WaitGroup
	for i := 0; i < config.Limits.Workers; i++ {
		workers.Add(1)
		go func() {
			defer workers.Done()
			for job := range dispatch {
				record := executeReliabilityJob(ctx, config, job, exchange, clock)
				appendRecord(record)
				<-inFlight
			}
		}()
	}

	interval := time.Duration(float64(time.Second) / config.TargetQPS)
	if interval < time.Nanosecond {
		interval = time.Nanosecond
	}
	for slotID := 0; slotID < config.Slots; slotID++ {
		planned := time.Duration(slotID) * interval
		if isEvidenceFailed() {
			appendRecord(reliabilitySlotRecord{SchemaVersion: reliabilitySchemaVersion, SlotID: int64(slotID), PlannedOffsetUS: planned.Microseconds(), Terminal: reliabilityTerminalHarnessRejected, Error: "evidence_persistence_failure"})
			continue
		}
		if err := clock.SleepUntil(ctx, planned); err != nil {
			appendRecord(reliabilitySlotRecord{SchemaVersion: reliabilitySchemaVersion, SlotID: int64(slotID), PlannedOffsetUS: planned.Microseconds(), Terminal: reliabilityTerminalHarnessSkipped})
			continue
		}
		if clock.Now() > planned+config.RequestDeadline {
			appendRecord(reliabilitySlotRecord{SchemaVersion: reliabilitySchemaVersion, SlotID: int64(slotID), PlannedOffsetUS: planned.Microseconds(), Terminal: reliabilityTerminalHarnessSkipped, Error: "scheduler_lag_exceeded_service_window"})
			continue
		}
		if isEvidenceFailed() {
			appendRecord(reliabilitySlotRecord{SchemaVersion: reliabilitySchemaVersion, SlotID: int64(slotID), PlannedOffsetUS: planned.Microseconds(), Terminal: reliabilityTerminalHarnessRejected, Error: "evidence_persistence_failure"})
			continue
		}
		select {
		case inFlight <- struct{}{}:
		default:
			appendRecord(reliabilitySlotRecord{SchemaVersion: reliabilitySchemaVersion, SlotID: int64(slotID), PlannedOffsetUS: planned.Microseconds(), Terminal: reliabilityTerminalHarnessRejected, Error: "in_flight_limit"})
			continue
		}
		job := reliabilityJob{slotID: int64(slotID), planned: planned, caseValue: workload[slotID%len(workload)]}
		select {
		case dispatch <- job:
		default:
			<-inFlight
			appendRecord(reliabilitySlotRecord{SchemaVersion: reliabilitySchemaVersion, SlotID: int64(slotID), PlannedOffsetUS: planned.Microseconds(), Terminal: reliabilityTerminalHarnessRejected, Error: "dispatch_queue_limit"})
		}
	}
	close(dispatch)
	workers.Wait()
	writer.Close()
	writerErr := writer.Wait()
	if writerErr != nil {
		evidenceValid = false
	}

	recordsMu.Lock()
	records := append([]reliabilitySlotRecord(nil), controlRecords...)
	missingIDs := append([]int64(nil), missing...)
	recordsMu.Unlock()
	sort.Slice(records, func(i, j int) bool { return records[i].SlotID < records[j].SlotID })
	accounting, err := recomputeReliabilityAccounting(records)
	if err != nil {
		return reliabilityRunResult{}, err
	}
	persisted := writer.Persisted()
	if len(persisted) != len(records) {
		evidenceValid = false
	}
	missingIDs = append(missingIDs, missingSlotIDs(records, persisted)...)
	loadValid := accounting.LoadValid() && evidenceValid
	finishedWall := clock.WallNow().UTC()
	raw := reliabilityRawBundle{
		SchemaVersion:   reliabilitySchemaVersion,
		RunID:           config.RunID,
		Scenario:        config.Scenario,
		Transport:       config.Transport,
		StartedAt:       startedWall,
		FinishedAt:      finishedWall,
		Config:          config,
		ControlRecords:  records,
		EvidenceRecords: persisted,
		Accounting:      accounting,
		EvidenceValid:   evidenceValid,
		LoadValid:       loadValid,
		MissingSlotIDs:  uniqueInt64(missingIDs),
	}
	if writerErr != nil {
		if strings.HasPrefix(writerErr.Error(), "cleanup_failure:") {
			raw.CleanupFailure = writerErr.Error()
		} else {
			raw.WriterError = writerErr.Error()
		}
	}
	return reliabilityRunResult{Raw: raw, Accounting: accounting}, nil
}

func validateReliabilityRunConfig(config reliabilityRunConfig) error {
	if config.RunID == "" {
		return errors.New("run_id is required")
	}
	if config.Slots <= 0 {
		return errors.New("slots must be positive")
	}
	if config.TargetQPS <= 0 {
		return errors.New("target_qps must be positive")
	}
	if config.RequestDeadline <= 0 || config.LateDrain < 0 {
		return errors.New("request deadline must be positive and late drain non-negative")
	}
	if config.CleanupTimeout <= 0 {
		return errors.New("cleanup timeout must be positive")
	}
	if err := config.Limits.validate(); err != nil {
		return err
	}
	maxSlots := reliabilityMaxControlBytes / int64(config.Limits.RecordBytes)
	if config.Slots > int(maxSlots) {
		return fmt.Errorf("slots and record_bytes exceed the %d-byte control budget", reliabilityMaxControlBytes)
	}
	return nil
}

func reliabilityRecordSize(record reliabilitySlotRecord) int {
	data, err := json.Marshal(record)
	if err != nil {
		return reliabilityMaxRecordBytes + 1
	}
	return len(data)
}

func compactReliabilityRecord(record reliabilitySlotRecord) reliabilitySlotRecord {
	return reliabilitySlotRecord{
		SchemaVersion:         reliabilitySchemaVersion,
		SlotID:                record.SlotID,
		WindowID:              record.WindowID,
		PlannedOffsetUS:       record.PlannedOffsetUS,
		DispatchOffsetUS:      record.DispatchOffsetUS,
		WriteStartOffsetUS:    record.WriteStartOffsetUS,
		WriteCompleteOffsetUS: record.WriteCompleteOffsetUS,
		FinishOffsetUS:        record.FinishOffsetUS,
		BytesWritten:          record.BytesWritten,
		FrameBytes:            record.FrameBytes,
		BytesRead:             record.BytesRead,
		DNSSent:               record.DNSSent,
		WriteDeadlineRace:     record.WriteDeadlineRace,
		Terminal:              record.Terminal,
		Error:                 "record_size_limit",
	}
}

func executeReliabilityJob(ctx context.Context, config reliabilityRunConfig, job reliabilityJob, exchange reliabilityExchange, clock reliabilityClock) reliabilitySlotRecord {
	record := reliabilitySlotRecord{
		SchemaVersion:   reliabilitySchemaVersion,
		SlotID:          job.slotID,
		CaseID:          job.caseValue.CaseID,
		QName:           job.caseValue.QName,
		QType:           job.caseValue.QType,
		PlannedOffsetUS: job.planned.Microseconds(),
	}
	dispatchOffset := clock.Now()
	record.DispatchOffsetUS = ptrInt64(dispatchOffset.Microseconds())
	deadlines := makeReliabilityDeadlines(job.planned, config.RequestDeadline, config.LateDrain)
	if dispatchOffset >= deadlines.Service {
		now := dispatchOffset.Microseconds()
		record.FinishOffsetUS = &now
		record.Terminal = reliabilityTerminalFailedBeforeDNSSend
		record.Error = "service_deadline_expired_before_exchange"
		return record
	}
	query := new(dns.Msg)
	qtype, ok := dns.StringToType[caseInsensitive(job.caseValue.QType)]
	if !ok {
		now := clock.Now().Microseconds()
		record.FinishOffsetUS = &now
		record.Terminal = reliabilityTerminalFailedBeforeDNSSend
		record.Error = "unsupported_qtype"
		return record
	}
	query.SetQuestion(dns.Fqdn(job.caseValue.QName), qtype)
	payload, err := query.Pack()
	if err != nil {
		now := clock.Now().Microseconds()
		record.FinishOffsetUS = &now
		record.Terminal = reliabilityTerminalFailedBeforeDNSSend
		record.Error = err.Error()
		return record
	}
	request := reliabilityRequest{SlotID: job.slotID, Case: job.caseValue, FrameBytes: len(payload), Payload: payload}
	result := exchange.Exchange(ctx, request, deadlines)
	if result.FrameBytes == 0 {
		result.FrameBytes = len(payload)
	}
	classified := classifyReliabilityExchange(result, &reliabilityExpected{ServiceDeadline: deadlines.Service, CollectionDeadline: deadlines.Collection})
	if classified.HasWriteStart {
		value := classified.WriteStart.Microseconds()
		record.WriteStartOffsetUS = &value
	}
	if classified.HasWriteComplete {
		value := classified.WriteComplete.Microseconds()
		record.WriteCompleteOffsetUS = &value
	}
	record.BytesWritten = classified.BytesWritten
	record.FrameBytes = classified.FrameBytes
	record.BytesRead = classified.BytesRead
	record.DNSSent = classified.DNSSent
	record.WriteDeadlineRace = classified.WriteDeadlineRace
	record.Terminal = classified.Terminal
	if classified.Err != nil {
		record.Error = classified.Err.Error()
	}
	finish := classified.ResponseAt
	if finish <= 0 {
		finish = clock.Now()
	}
	finishOffset := finish.Microseconds()
	record.FinishOffsetUS = &finishOffset
	return record
}

func caseInsensitive(value string) string {
	return strings.ToUpper(value)
}

func missingSlotIDs(control, persisted []reliabilitySlotRecord) []int64 {
	present := make(map[int64]struct{}, len(persisted))
	for _, record := range persisted {
		present[record.SlotID] = struct{}{}
	}
	missing := make([]int64, 0)
	for _, record := range control {
		if _, ok := present[record.SlotID]; !ok {
			missing = append(missing, record.SlotID)
		}
	}
	return missing
}

func uniqueInt64(values []int64) []int64 {
	seen := make(map[int64]struct{}, len(values))
	result := make([]int64, 0, len(values))
	for _, value := range values {
		if _, ok := seen[value]; ok {
			continue
		}
		seen[value] = struct{}{}
		result = append(result, value)
	}
	sort.Slice(result, func(i, j int) bool { return result[i] < result[j] })
	return result
}

func validateReliabilityAddress(address string) error {
	host, _, err := net.SplitHostPort(address)
	if err != nil {
		return fmt.Errorf("invalid loopback address %q: %w", address, err)
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		return fmt.Errorf("reliability runner requires a loopback address: %s", address)
	}
	return nil
}

type jsonlReliabilitySink struct {
	mu     sync.Mutex
	f      *os.File
	closed atomic.Bool
}

func newJSONLReliabilitySink(path string) (*jsonlReliabilitySink, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0644)
	if err != nil {
		return nil, err
	}
	return &jsonlReliabilitySink{f: f}, nil
}

func (s *jsonlReliabilitySink) Write(ctx context.Context, record reliabilitySlotRecord) error {
	if s.closed.Load() {
		return errors.New("reliability evidence sink is closed")
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	default:
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed.Load() || s.f == nil {
		return errors.New("reliability evidence sink is closed")
	}
	data, err := json.Marshal(record)
	if err != nil {
		return err
	}
	data = append(data, '\n')
	_, err = s.f.Write(data)
	return err
}

func (s *jsonlReliabilitySink) Close() error {
	if !s.closed.CompareAndSwap(false, true) {
		return nil
	}
	if !s.mu.TryLock() {
		return errors.New("cleanup_failure: evidence sink write is still blocked")
	}
	defer s.mu.Unlock()
	if s.f == nil {
		return nil
	}
	err := s.f.Close()
	s.f = nil
	return err
}
