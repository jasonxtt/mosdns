package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"time"
)

type reliabilityVerdict string

const (
	reliabilityVerdictPass          reliabilityVerdict = "pass"
	reliabilityVerdictDegraded      reliabilityVerdict = "service_degraded"
	reliabilityVerdictInvalidLoad   reliabilityVerdict = "invalid_load"
	reliabilityVerdictCorrectness   reliabilityVerdict = "correctness_failure"
	reliabilityVerdictIndeterminate reliabilityVerdict = "indeterminate"
)

type reliabilityLatencyView struct {
	Name               string  `json:"name"`
	Samples            int     `json:"samples"`
	EligibleSamples    int     `json:"eligible_samples"`
	FailureDenominator int     `json:"failure_denominator"`
	IneligibleSamples  int     `json:"ineligible_samples"`
	P50US              int64   `json:"p50_us"`
	P95US              int64   `json:"p95_us"`
	P99US              int64   `json:"p99_us"`
	ValuesUS           []int64 `json:"values_us,omitempty"`
}

type reliabilityWindow struct {
	WindowID             string  `json:"window_id"`
	SlotIDs              []int64 `json:"slot_ids"`
	StartOffsetUS        int64   `json:"start_offset_us"`
	EndOffsetUS          int64   `json:"end_offset_us"`
	Phase                string  `json:"phase"`
	OfferedQPS           float64 `json:"offered_qps"`
	SUTPID               int     `json:"sut_pid"`
	SUTStartIdentity     string  `json:"sut_start_identity"`
	ResourceWithinBudget *bool   `json:"resource_within_budget,omitempty"`
}

type reliabilityWindowFacts struct {
	WindowID             string
	P95US                int64
	TimeoutRate          float64
	OnTimeRate           float64
	CorrectnessValid     bool
	EvidenceValid        bool
	LoadValid            bool
	SUTPID               int
	SUTStartIdentity     string
	ResourceWithinBudget bool
	Phase                string
	OfferedQPS           float64
	Healthy              bool
}

type reliabilityCriteria struct {
	ServiceP95CeilingUS int64   `json:"service_p95_ceiling_us"`
	TimeoutRateCeiling  float64 `json:"timeout_rate_ceiling"`
	OnTimeRateFloor     float64 `json:"on_time_rate_floor"`
	ReferenceQPS        float64 `json:"reference_qps"`
	OverloadWindows     int     `json:"overload_windows"`
	RecoveryWindows     int     `json:"recovery_windows"`
}

type reliabilityAssessment struct {
	SchemaVersion     string                   `json:"schema_version"`
	RunID             string                   `json:"run_id"`
	Verdict           reliabilityVerdict       `json:"verdict"`
	Reason            string                   `json:"reason"`
	EvidenceValid     bool                     `json:"evidence_valid"`
	LoadValid         bool                     `json:"load_valid"`
	CorrectnessValid  bool                     `json:"correctness_valid"`
	ServiceDegraded   bool                     `json:"service_degraded"`
	Accounting        reliabilityAccounting    `json:"accounting"`
	LatencyViews      []reliabilityLatencyView `json:"latency_views"`
	CorrectOnTime     int                      `json:"correct_on_time"`
	OfferedStageUS    int64                    `json:"offered_stage_duration_us"`
	GoodputQPS        float64                  `json:"goodput_qps"`
	Criteria          reliabilityCriteria      `json:"criteria"`
	RecoveryState     string                   `json:"recovery_state"`
	OverloadWindowIDs []string                 `json:"overload_window_ids,omitempty"`
}

func assessReliability(raw reliabilityRawBundle, criteria reliabilityCriteria) (reliabilityAssessment, error) {
	assessment := reliabilityAssessment{SchemaVersion: reliabilitySchemaVersion, RunID: raw.RunID, Criteria: criteria}
	if raw.SchemaVersion != "" && raw.SchemaVersion != reliabilitySchemaVersion {
		return assessment, fmt.Errorf("unsupported reliability schema %q", raw.SchemaVersion)
	}
	if len(raw.Windows) == 0 {
		if criteria.OverloadWindows <= 0 {
			criteria.OverloadWindows = 2
		}
		if criteria.RecoveryWindows <= 0 {
			criteria.RecoveryWindows = 3
		}
		if criteria.TimeoutRateCeiling < 0 {
			criteria.TimeoutRateCeiling = 0.01
		}
		if criteria.OnTimeRateFloor < 0 {
			criteria.OnTimeRateFloor = 0
		}
		if criteria.ServiceP95CeilingUS <= 0 {
			criteria.ServiceP95CeilingUS = raw.Config.RequestDeadline.Microseconds()
		}
		if criteria.ReferenceQPS < 0 {
			criteria.ReferenceQPS = 0
		}
	}
	assessment.Criteria = criteria
	accounting, err := recomputeReliabilityAccounting(raw.ControlRecords)
	if err != nil {
		return assessment, err
	}
	assessment.Accounting = accounting
	assessment.EvidenceValid = reliabilityEvidenceMatches(raw.ControlRecords, raw.EvidenceRecords)
	assessment.LoadValid = assessment.EvidenceValid && accounting.LoadValid()
	assessment.CorrectnessValid = accounting.PostSendTerminals[reliabilityTerminalWrongResponse] == 0 && accounting.PostSendTerminals[reliabilityTerminalProtocolError] == 0
	assessment.LatencyViews = reliabilityLatencyViews(raw.ControlRecords)
	assessment.CorrectOnTime, assessment.OfferedStageUS, assessment.GoodputQPS = reliabilityGoodput(raw.ControlRecords, raw.Config)
	if !assessment.EvidenceValid {
		assessment.Verdict = reliabilityVerdictIndeterminate
		assessment.RecoveryState = "indeterminate-missing-evidence"
		assessment.Reason = "raw evidence does not cover every control slot"
		return assessment, nil
	}
	if !assessment.LoadValid {
		assessment.Verdict = reliabilityVerdictInvalidLoad
		assessment.RecoveryState = "invalid-load"
		assessment.Reason = "harness skipped or rejected slots, or accounting is not fully sent"
		return assessment, nil
	}
	if !assessment.CorrectnessValid {
		assessment.Verdict = reliabilityVerdictCorrectness
		assessment.RecoveryState = "correctness-failure"
		assessment.Reason = "strict DNS correctness/protocol validation failed"
		return assessment, nil
	}
	if len(raw.Windows) > 0 {
		if err := validateReliabilityWindowCriteria(criteria); err != nil {
			assessment.Verdict = reliabilityVerdictIndeterminate
			assessment.RecoveryState = "indeterminate-criteria-not-frozen"
			assessment.Reason = err.Error()
			return assessment, nil
		}
	}
	assessment.ServiceDegraded = reliabilityServiceDegraded(raw.ControlRecords, criteria, raw.Config.RequestDeadline)
	windowFacts, windowErr := deriveReliabilityWindowFacts(raw, criteria)
	if windowErr != nil {
		assessment.Verdict = reliabilityVerdictIndeterminate
		assessment.RecoveryState = "indeterminate-missing-evidence"
		assessment.Reason = windowErr.Error()
		return assessment, nil
	}
	assessment.RecoveryState, assessment.OverloadWindowIDs = reliabilityRecoveryState(windowFacts, criteria)
	if assessment.RecoveryState == "indeterminate-missing-evidence" {
		assessment.Verdict = reliabilityVerdictIndeterminate
		assessment.Reason = assessment.RecoveryState
		return assessment, nil
	}
	if assessment.ServiceDegraded {
		assessment.Verdict = reliabilityVerdictDegraded
		assessment.Reason = "valid load has timeout or late service outcomes"
		return assessment, nil
	}
	assessment.Verdict = reliabilityVerdictPass
	assessment.Reason = "evidence, load, correctness, and service budget checks passed"
	return assessment, nil
}

func validateReliabilityWindowCriteria(criteria reliabilityCriteria) error {
	if criteria.ServiceP95CeilingUS <= 0 {
		return errors.New("window service p95 ceiling is not frozen")
	}
	if criteria.TimeoutRateCeiling < 0 || criteria.TimeoutRateCeiling > 1 {
		return errors.New("window timeout-rate ceiling is not frozen")
	}
	if criteria.OnTimeRateFloor <= 0 || criteria.OnTimeRateFloor > 1 {
		return errors.New("window on-time-rate floor is not frozen")
	}
	if criteria.ReferenceQPS <= 0 {
		return errors.New("window reference qps is not frozen")
	}
	if criteria.OverloadWindows <= 0 || criteria.RecoveryWindows <= 0 {
		return errors.New("window overload/recovery counts are not frozen")
	}
	return nil
}

func reliabilityEvidenceMatches(control, evidence []reliabilitySlotRecord) bool {
	if len(control) == 0 || len(control) != len(evidence) {
		return false
	}
	controlCopy := append([]reliabilitySlotRecord(nil), control...)
	evidenceCopy := append([]reliabilitySlotRecord(nil), evidence...)
	sort.Slice(controlCopy, func(i, j int) bool { return controlCopy[i].SlotID < controlCopy[j].SlotID })
	sort.Slice(evidenceCopy, func(i, j int) bool { return evidenceCopy[i].SlotID < evidenceCopy[j].SlotID })
	return reflect.DeepEqual(controlCopy, evidenceCopy)
}

func reliabilityLatencyViews(records []reliabilitySlotRecord) []reliabilityLatencyView {
	views := []reliabilityLatencyView{
		{Name: "planned-slot-to-finish"},
		{Name: "dispatch-to-finish"},
		{Name: "write-start-to-finish"},
	}
	for _, record := range records {
		if record.FinishOffsetUS == nil || !isReliabilitySuccessfulResponse(record.Terminal) {
			continue
		}
		if value := *record.FinishOffsetUS - record.PlannedOffsetUS; value >= 0 {
			views[0].ValuesUS = append(views[0].ValuesUS, value)
		}
		if record.DispatchOffsetUS != nil {
			if value := *record.FinishOffsetUS - *record.DispatchOffsetUS; value >= 0 {
				views[1].ValuesUS = append(views[1].ValuesUS, value)
			}
		}
		if record.WriteStartOffsetUS != nil {
			if value := *record.FinishOffsetUS - *record.WriteStartOffsetUS; value >= 0 {
				views[2].ValuesUS = append(views[2].ValuesUS, value)
			}
		}
	}
	for i := range views {
		sort.Slice(views[i].ValuesUS, func(a, b int) bool { return views[i].ValuesUS[a] < views[i].ValuesUS[b] })
		views[i].Samples = len(views[i].ValuesUS)
		views[i].EligibleSamples = views[i].Samples
		views[i].FailureDenominator = len(records)
		views[i].IneligibleSamples = views[i].FailureDenominator - views[i].EligibleSamples
		views[i].P50US = percentile(views[i].ValuesUS, 0.50)
		views[i].P95US = percentile(views[i].ValuesUS, 0.95)
		views[i].P99US = percentile(views[i].ValuesUS, 0.99)
	}
	return views
}

func isReliabilitySuccessfulResponse(terminal reliabilityTerminal) bool {
	return terminal == reliabilityTerminalCorrectOnTime || terminal == reliabilityTerminalCorrectLate
}

func reliabilityServiceDegraded(records []reliabilitySlotRecord, criteria reliabilityCriteria, requestDeadline time.Duration) bool {
	for _, record := range records {
		if reliabilityWriteDeadlineRace(record, requestDeadline) || record.Terminal == reliabilityTerminalCorrectLate || record.Terminal == reliabilityTerminalTimeout || record.Terminal == reliabilityTerminalTransportError || record.Terminal == reliabilityTerminalFailedBeforeDNSSend {
			return true
		}
	}
	for _, view := range reliabilityLatencyViews(records) {
		if view.Name == "planned-slot-to-finish" && criteria.ServiceP95CeilingUS > 0 && view.P95US > criteria.ServiceP95CeilingUS {
			return true
		}
	}
	return false
}

func reliabilityWriteDeadlineRace(record reliabilitySlotRecord, requestDeadline time.Duration) bool {
	if record.WriteCompleteOffsetUS == nil || requestDeadline <= 0 {
		return false
	}
	return *record.WriteCompleteOffsetUS-record.PlannedOffsetUS > requestDeadline.Microseconds()
}

func reliabilityGoodput(records []reliabilitySlotRecord, config reliabilityRunConfig) (int, int64, float64) {
	correctOnTime := 0
	maxPlannedUS := int64(0)
	for _, record := range records {
		if record.Terminal == reliabilityTerminalCorrectOnTime {
			correctOnTime++
		}
		if record.PlannedOffsetUS > maxPlannedUS {
			maxPlannedUS = record.PlannedOffsetUS
		}
	}
	intervalUS := int64(0)
	if config.TargetQPS > 0 {
		intervalUS = time.Duration(float64(time.Second) / config.TargetQPS).Microseconds()
	}
	if intervalUS <= 0 {
		intervalUS = config.RequestDeadline.Microseconds()
	}
	if intervalUS <= 0 {
		intervalUS = 1
	}
	offeredStageUS := maxPlannedUS + intervalUS
	goodput := float64(correctOnTime) / (float64(offeredStageUS) / float64(time.Second/time.Microsecond))
	return correctOnTime, offeredStageUS, goodput
}

func deriveReliabilityWindowFacts(raw reliabilityRawBundle, criteria reliabilityCriteria) ([]reliabilityWindowFacts, error) {
	if len(raw.Windows) == 0 {
		return nil, nil
	}
	if err := validateReliabilityWindowSequence(raw, criteria); err != nil {
		return nil, err
	}
	controlByID := make(map[int64]reliabilitySlotRecord, len(raw.ControlRecords))
	evidenceByID := make(map[int64]reliabilitySlotRecord, len(raw.EvidenceRecords))
	for _, record := range raw.ControlRecords {
		controlByID[record.SlotID] = record
	}
	for _, record := range raw.EvidenceRecords {
		evidenceByID[record.SlotID] = record
	}
	seenSlots := make(map[int64]string)
	facts := make([]reliabilityWindowFacts, 0, len(raw.Windows))
	for _, window := range raw.Windows {
		if window.WindowID == "" {
			return nil, errors.New("reliability window is missing window_id")
		}
		if window.SUTPID <= 0 || window.SUTStartIdentity == "" {
			return nil, fmt.Errorf("reliability window %q is missing a non-empty SUT PID/start identity", window.WindowID)
		}
		if window.ResourceWithinBudget == nil {
			return nil, fmt.Errorf("reliability window %q is missing the resource budget fact", window.WindowID)
		}
		slotIDs := append([]int64(nil), window.SlotIDs...)
		if len(slotIDs) == 0 {
			for _, record := range raw.ControlRecords {
				if record.WindowID == window.WindowID {
					slotIDs = append(slotIDs, record.SlotID)
				}
			}
		}
		if len(slotIDs) == 0 {
			return nil, fmt.Errorf("reliability window %q has no raw slot membership", window.WindowID)
		}
		controlRecords := make([]reliabilitySlotRecord, 0, len(slotIDs))
		evidenceRecords := make([]reliabilitySlotRecord, 0, len(slotIDs))
		for _, slotID := range slotIDs {
			if previous, exists := seenSlots[slotID]; exists {
				return nil, fmt.Errorf("slot %d belongs to windows %q and %q", slotID, previous, window.WindowID)
			}
			seenSlots[slotID] = window.WindowID
			controlRecord, ok := controlByID[slotID]
			if !ok {
				return nil, fmt.Errorf("window %q references missing control slot %d", window.WindowID, slotID)
			}
			controlRecords = append(controlRecords, controlRecord)
			if evidenceRecord, ok := evidenceByID[slotID]; ok {
				evidenceRecords = append(evidenceRecords, evidenceRecord)
			}
		}
		accounting, err := recomputeReliabilityAccounting(controlRecords)
		if err != nil {
			return nil, fmt.Errorf("window %q accounting: %w", window.WindowID, err)
		}
		evidenceValid := len(evidenceRecords) == len(controlRecords) && reliabilityEvidenceMatches(controlRecords, evidenceRecords)
		loadValid := evidenceValid && accounting.LoadValid()
		correctnessValid := accounting.PostSendTerminals[reliabilityTerminalWrongResponse] == 0 && accounting.PostSendTerminals[reliabilityTerminalProtocolError] == 0
		p95 := int64(0)
		for _, view := range reliabilityLatencyViews(controlRecords) {
			if view.Name == "planned-slot-to-finish" {
				p95 = view.P95US
				break
			}
		}
		timeoutRate := 0.0
		onTimeRate := 0.0
		if accounting.Planned > 0 {
			timeoutRate = float64(accounting.PostSendTerminals[reliabilityTerminalTimeout]) / float64(accounting.Planned)
			onTimeRate = float64(accounting.PostSendTerminals[reliabilityTerminalCorrectOnTime]) / float64(accounting.Planned)
		}
		serviceDegraded := reliabilityServiceDegraded(controlRecords, criteria, raw.Config.RequestDeadline)
		facts = append(facts, reliabilityWindowFacts{
			WindowID:             window.WindowID,
			P95US:                p95,
			TimeoutRate:          timeoutRate,
			OnTimeRate:           onTimeRate,
			CorrectnessValid:     correctnessValid,
			EvidenceValid:        evidenceValid,
			LoadValid:            loadValid,
			SUTPID:               window.SUTPID,
			SUTStartIdentity:     window.SUTStartIdentity,
			ResourceWithinBudget: *window.ResourceWithinBudget,
			Phase:                window.Phase,
			OfferedQPS:           window.OfferedQPS,
			Healthy:              evidenceValid && loadValid && correctnessValid && *window.ResourceWithinBudget && onTimeRate >= criteria.OnTimeRateFloor && !serviceDegraded,
		})
	}
	return facts, nil
}

func validateReliabilityWindowSequence(raw reliabilityRawBundle, criteria reliabilityCriteria) error {
	if len(raw.Windows) == 0 {
		return nil
	}
	controlByID := make(map[int64]reliabilitySlotRecord, len(raw.ControlRecords))
	for _, record := range raw.ControlRecords {
		controlByID[record.SlotID] = record
	}
	seenWindowIDs := make(map[string]struct{}, len(raw.Windows))
	seenSlots := make(map[int64]struct{}, len(raw.ControlRecords))
	var previousEnd int64
	var duration int64
	previousPhaseRank := 0
	for index, window := range raw.Windows {
		if _, exists := seenWindowIDs[window.WindowID]; exists {
			return fmt.Errorf("reliability window %q is duplicated", window.WindowID)
		}
		seenWindowIDs[window.WindowID] = struct{}{}
		if window.StartOffsetUS < 0 || window.EndOffsetUS <= window.StartOffsetUS {
			return fmt.Errorf("reliability window %q has invalid boundaries", window.WindowID)
		}
		if index == 0 {
			if window.StartOffsetUS != 0 {
				return fmt.Errorf("reliability windows do not start at offset zero")
			}
			if window.Phase != "reference" {
				return errors.New("reliability windows must begin with a reference phase")
			}
			duration = window.EndOffsetUS - window.StartOffsetUS
		} else {
			if window.StartOffsetUS != previousEnd {
				return fmt.Errorf("reliability windows are not contiguous at %q", window.WindowID)
			}
			if window.EndOffsetUS-window.StartOffsetUS != duration {
				return fmt.Errorf("reliability windows are not equal length at %q", window.WindowID)
			}
		}
		previousEnd = window.EndOffsetUS
		if window.OfferedQPS <= 0 {
			return fmt.Errorf("reliability window %q is missing offered qps", window.WindowID)
		}
		phaseRank := 0
		switch window.Phase {
		case "reference":
			phaseRank = 0
			if window.OfferedQPS != criteria.ReferenceQPS {
				return fmt.Errorf("reference window %q is not at the frozen reference qps", window.WindowID)
			}
		case "overload":
			phaseRank = 1
			if window.OfferedQPS <= criteria.ReferenceQPS {
				return fmt.Errorf("overload window %q is not above the frozen reference qps", window.WindowID)
			}
		case "recovery":
			phaseRank = 2
			if window.OfferedQPS != criteria.ReferenceQPS {
				return fmt.Errorf("recovery window %q is not at the frozen reference qps", window.WindowID)
			}
		default:
			return fmt.Errorf("reliability window %q has unsupported phase %q", window.WindowID, window.Phase)
		}
		if phaseRank < previousPhaseRank {
			return fmt.Errorf("reliability window phases are out of order at %q", window.WindowID)
		}
		if window.Phase == "recovery" && previousPhaseRank < 1 {
			return fmt.Errorf("reliability recovery window %q has no preceding overload phase", window.WindowID)
		}
		previousPhaseRank = phaseRank
		for _, slotID := range window.SlotIDs {
			if _, exists := seenSlots[slotID]; exists {
				return fmt.Errorf("slot %d is assigned to multiple reliability windows", slotID)
			}
			record, exists := controlByID[slotID]
			if !exists {
				return fmt.Errorf("window %q references missing control slot %d", window.WindowID, slotID)
			}
			if record.PlannedOffsetUS < window.StartOffsetUS || record.PlannedOffsetUS >= window.EndOffsetUS {
				return fmt.Errorf("slot %d is outside reliability window %q boundaries", slotID, window.WindowID)
			}
			seenSlots[slotID] = struct{}{}
		}
	}
	if len(seenSlots) != len(raw.ControlRecords) {
		return errors.New("reliability windows omit control slots")
	}
	return nil
}

func reliabilityRecoveryState(windows []reliabilityWindowFacts, criteria reliabilityCriteria) (string, []string) {
	if len(windows) == 0 {
		return "indeterminate-no-overload-evidence", nil
	}
	violating := func(window reliabilityWindowFacts) bool {
		return window.LoadValid && window.EvidenceValid && window.CorrectnessValid && (window.OnTimeRate < criteria.OnTimeRateFloor || window.P95US > criteria.ServiceP95CeilingUS || window.TimeoutRate > criteria.TimeoutRateCeiling)
	}
	consecutive := 0
	overloadSeen := false
	overloadIDs := make([]string, 0)
	var sequencePID int
	var sequenceStart string
	sequenceIdentitySet := false
	sequenceIDs := make([]string, 0, criteria.OverloadWindows)
	healthy := 0
	for _, window := range windows {
		if !window.EvidenceValid || !window.LoadValid || !window.CorrectnessValid {
			return "indeterminate-missing-evidence", overloadIDs
		}
		if violating(window) {
			if !overloadSeen && (window.Phase != "overload" || window.OfferedQPS <= criteria.ReferenceQPS) {
				return "indeterminate-no-overload-evidence", overloadIDs
			}
			if sequenceIdentitySet && (window.SUTPID != sequencePID || window.SUTStartIdentity != sequenceStart) {
				return "non-recovery-restart", overloadIDs
			}
			if !sequenceIdentitySet {
				sequencePID = window.SUTPID
				sequenceStart = window.SUTStartIdentity
				sequenceIdentitySet = true
			}
			if window.Phase == "recovery" {
				healthy = 0
				continue
			}
			consecutive++
			sequenceIDs = append(sequenceIDs, window.WindowID)
			if consecutive >= criteria.OverloadWindows {
				if !overloadSeen {
					overloadSeen = true
					overloadIDs = append(overloadIDs, sequenceIDs...)
				} else {
					overloadIDs = append(overloadIDs, window.WindowID)
				}
			}
			healthy = 0
			continue
		}
		consecutive = 0
		if !overloadSeen {
			sequenceIDs = sequenceIDs[:0]
			sequenceIdentitySet = false
			continue
		}
		if window.SUTPID != sequencePID || window.SUTStartIdentity != sequenceStart {
			return "non-recovery-restart", overloadIDs
		}
		if window.Phase != "recovery" || window.OfferedQPS != criteria.ReferenceQPS {
			healthy = 0
			continue
		}
		if window.Healthy && window.ResourceWithinBudget && window.OnTimeRate >= criteria.OnTimeRateFloor {
			healthy++
			if healthy >= criteria.RecoveryWindows {
				return "recovered", overloadIDs
			}
		} else {
			healthy = 0
		}
	}
	if overloadSeen {
		return "overload-without-recovery", overloadIDs
	}
	return "indeterminate-no-overload-evidence", overloadIDs
}

func ensureFreshReliabilityOutput(path string) error {
	info, err := os.Stat(path)
	if err == nil {
		if !info.IsDir() {
			return fmt.Errorf("derived output is not a directory: %s", path)
		}
		entries, readErr := os.ReadDir(path)
		if readErr != nil {
			return readErr
		}
		if len(entries) != 0 {
			return fmt.Errorf("derived output must be fresh: %s", path)
		}
		return nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return os.MkdirAll(path, 0755)
}

func loadReliabilityRaw(path string) (reliabilityRawBundle, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return reliabilityRawBundle{}, err
	}
	var raw reliabilityRawBundle
	if err := json.Unmarshal(data, &raw); err != nil {
		return reliabilityRawBundle{}, fmt.Errorf("decode reliability raw bundle: %w", err)
	}
	return raw, nil
}

func writeReliabilityAssessment(path string, assessment reliabilityAssessment) error {
	data, err := json.MarshalIndent(assessment, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	return os.WriteFile(filepath.Join(path, "assessment.json"), data, 0644)
}
