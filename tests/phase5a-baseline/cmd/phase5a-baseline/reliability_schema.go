package main

import (
	"fmt"
	"sort"
	"time"
)

const reliabilitySchemaVersion = "phase5a-reliability-v1"

type reliabilityTerminal string

const (
	reliabilityTerminalHarnessSkipped      reliabilityTerminal = "harness_skipped"
	reliabilityTerminalHarnessRejected     reliabilityTerminal = "harness_rejected"
	reliabilityTerminalFailedBeforeDNSSend reliabilityTerminal = "failed_before_dns_send"
	reliabilityTerminalCorrectOnTime       reliabilityTerminal = "post_send_correct_on_time"
	reliabilityTerminalCorrectLate         reliabilityTerminal = "post_send_correct_late"
	reliabilityTerminalTimeout             reliabilityTerminal = "post_send_timeout"
	reliabilityTerminalTransportError      reliabilityTerminal = "post_send_transport_error"
	reliabilityTerminalProtocolError       reliabilityTerminal = "post_send_protocol_error"
	reliabilityTerminalWrongResponse       reliabilityTerminal = "post_send_wrong_response"
)

type reliabilityLimits struct {
	Workers       int `json:"workers"`
	InFlight      int `json:"in_flight"`
	DispatchQueue int `json:"dispatch_queue"`
	EvidenceQueue int `json:"evidence_queue"`
	RecordBytes   int `json:"record_bytes"`
}

const (
	reliabilityMaxWorkers       = 256
	reliabilityMaxInFlight      = 256
	reliabilityMaxQueue         = 512
	reliabilityMaxRecordBytes   = 64 * 1024
	reliabilityMaxControlBytes  = 64 * 1024 * 1024
	reliabilityMinRecordBytes   = 256
	reliabilityDefaultWorkers   = 1
	reliabilityDefaultInFlight  = 1
	reliabilityDefaultQueue     = 8
	reliabilityDefaultRecordMax = 64 * 1024
)

func (l reliabilityLimits) validate() error {
	if l.Workers <= 0 || l.Workers > reliabilityMaxWorkers {
		return fmt.Errorf("workers must be in 1..%d", reliabilityMaxWorkers)
	}
	if l.InFlight <= 0 || l.InFlight > reliabilityMaxInFlight {
		return fmt.Errorf("in_flight must be in 1..%d", reliabilityMaxInFlight)
	}
	if l.DispatchQueue <= 0 || l.DispatchQueue > reliabilityMaxQueue {
		return fmt.Errorf("dispatch_queue must be in 1..%d", reliabilityMaxQueue)
	}
	if l.EvidenceQueue <= 0 || l.EvidenceQueue > reliabilityMaxQueue {
		return fmt.Errorf("evidence_queue must be in 1..%d", reliabilityMaxQueue)
	}
	if l.RecordBytes < reliabilityMinRecordBytes || l.RecordBytes > reliabilityMaxRecordBytes {
		return fmt.Errorf("record_bytes must be in %d..%d", reliabilityMinRecordBytes, reliabilityMaxRecordBytes)
	}
	return nil
}

type reliabilityRunConfig struct {
	RunID           string            `json:"run_id"`
	Scenario        string            `json:"scenario"`
	Transport       string            `json:"transport"`
	Address         string            `json:"address,omitempty"`
	Slots           int               `json:"slots"`
	TargetQPS       float64           `json:"target_qps"`
	RequestDeadline time.Duration     `json:"request_deadline"`
	LateDrain       time.Duration     `json:"late_drain"`
	CleanupTimeout  time.Duration     `json:"cleanup_timeout"`
	Limits          reliabilityLimits `json:"limits"`
}

type reliabilitySlotRecord struct {
	SchemaVersion         string              `json:"schema_version"`
	SlotID                int64               `json:"slot_id"`
	CaseID                string              `json:"case_id,omitempty"`
	QName                 string              `json:"qname,omitempty"`
	QType                 string              `json:"qtype,omitempty"`
	WindowID              string              `json:"window_id,omitempty"`
	PlannedOffsetUS       int64               `json:"planned_offset_us"`
	DispatchOffsetUS      *int64              `json:"dispatch_offset_us,omitempty"`
	WriteStartOffsetUS    *int64              `json:"write_start_offset_us,omitempty"`
	WriteCompleteOffsetUS *int64              `json:"write_complete_offset_us,omitempty"`
	FinishOffsetUS        *int64              `json:"finish_offset_us,omitempty"`
	BytesWritten          int                 `json:"bytes_written"`
	FrameBytes            int                 `json:"frame_bytes"`
	BytesRead             int                 `json:"bytes_read"`
	DNSSent               bool                `json:"dns_sent"`
	WriteDeadlineRace     bool                `json:"write_deadline_race,omitempty"`
	Terminal              reliabilityTerminal `json:"terminal"`
	Error                 string              `json:"error,omitempty"`
	DuplicatePackets      int                 `json:"duplicate_packets,omitempty"`
}

type reliabilityAccounting struct {
	Planned             int                         `json:"planned"`
	HarnessSkipped      int                         `json:"harness_skipped"`
	HarnessRejected     int                         `json:"harness_rejected"`
	Started             int                         `json:"started"`
	FailedBeforeDNSSend int                         `json:"failed_before_dns_send"`
	DNSSent             int                         `json:"dns_sent"`
	DuplicatePackets    int                         `json:"duplicate_packets"`
	PostSendTerminals   map[reliabilityTerminal]int `json:"post_send_terminals"`
}

func (a reliabilityAccounting) LoadValid() bool {
	return a.Planned > 0 && a.HarnessSkipped == 0 && a.HarnessRejected == 0 && a.Planned == a.Started
}

func recomputeReliabilityAccounting(records []reliabilitySlotRecord) (reliabilityAccounting, error) {
	accounting := reliabilityAccounting{PostSendTerminals: make(map[reliabilityTerminal]int)}
	seenIDs := make(map[int64]struct{}, len(records))
	plannedOffsets := make([]int64, 0, len(records))
	for _, record := range records {
		if record.SlotID < 0 {
			return accounting, fmt.Errorf("slot_id must be non-negative: %d", record.SlotID)
		}
		if _, exists := seenIDs[record.SlotID]; exists {
			return accounting, fmt.Errorf("duplicate slot_id: %d", record.SlotID)
		}
		seenIDs[record.SlotID] = struct{}{}
		if record.Terminal == "" {
			return accounting, fmt.Errorf("slot %d has no terminal", record.SlotID)
		}
		if record.PlannedOffsetUS < 0 {
			return accounting, fmt.Errorf("slot %d has negative planned offset", record.SlotID)
		}
		plannedOffsets = append(plannedOffsets, record.PlannedOffsetUS)
		if record.DuplicatePackets > 0 {
			accounting.DuplicatePackets += record.DuplicatePackets
		}
		accounting.Planned++
		switch record.Terminal {
		case reliabilityTerminalHarnessSkipped:
			if record.DNSSent || record.DispatchOffsetUS != nil || record.FinishOffsetUS != nil {
				return accounting, fmt.Errorf("harness-skipped slot %d reached execution", record.SlotID)
			}
			accounting.HarnessSkipped++
		case reliabilityTerminalHarnessRejected:
			if record.DNSSent || record.FinishOffsetUS != nil {
				return accounting, fmt.Errorf("harness-rejected slot %d has execution evidence", record.SlotID)
			}
			accounting.HarnessRejected++
		case reliabilityTerminalFailedBeforeDNSSend:
			if record.DNSSent {
				return accounting, fmt.Errorf("failed-before-dns-send slot %d is dns_sent", record.SlotID)
			}
			accounting.Started++
			accounting.FailedBeforeDNSSend++
		default:
			if !isReliabilityPostSendTerminal(record.Terminal) {
				return accounting, fmt.Errorf("unknown terminal %q", record.Terminal)
			}
			if !record.DNSSent {
				return accounting, fmt.Errorf("post-send terminal %q without dns_sent for slot %d", record.Terminal, record.SlotID)
			}
			if record.FinishOffsetUS == nil {
				return accounting, fmt.Errorf("post-send terminal %q without finish offset for slot %d", record.Terminal, record.SlotID)
			}
			accounting.Started++
			accounting.DNSSent++
			accounting.PostSendTerminals[record.Terminal]++
		}
		if record.Terminal != reliabilityTerminalHarnessSkipped && record.Terminal != reliabilityTerminalHarnessRejected && record.FinishOffsetUS == nil {
			return accounting, fmt.Errorf("started slot %d has no finish offset", record.SlotID)
		}
		if record.DispatchOffsetUS != nil && *record.DispatchOffsetUS < record.PlannedOffsetUS {
			return accounting, fmt.Errorf("slot %d dispatch precedes planned offset", record.SlotID)
		}
		if record.WriteStartOffsetUS != nil && record.DispatchOffsetUS != nil && *record.WriteStartOffsetUS < *record.DispatchOffsetUS {
			return accounting, fmt.Errorf("slot %d write starts before dispatch", record.SlotID)
		}
		if record.WriteCompleteOffsetUS != nil && record.WriteStartOffsetUS == nil {
			return accounting, fmt.Errorf("slot %d write completes without write start", record.SlotID)
		}
		if record.WriteCompleteOffsetUS != nil && record.WriteStartOffsetUS != nil && *record.WriteCompleteOffsetUS < *record.WriteStartOffsetUS {
			return accounting, fmt.Errorf("slot %d write completes before write start", record.SlotID)
		}
		if record.FinishOffsetUS != nil && record.WriteStartOffsetUS != nil && *record.FinishOffsetUS < *record.WriteStartOffsetUS {
			return accounting, fmt.Errorf("slot %d finishes before write start", record.SlotID)
		}
		if record.FinishOffsetUS != nil && record.WriteCompleteOffsetUS != nil && *record.FinishOffsetUS < *record.WriteCompleteOffsetUS {
			return accounting, fmt.Errorf("slot %d finishes before write complete", record.SlotID)
		}
	}
	if !sort.SliceIsSorted(plannedOffsets, func(i, j int) bool { return plannedOffsets[i] < plannedOffsets[j] }) {
		return accounting, fmt.Errorf("planned offsets are not monotonic")
	}
	if err := validateReliabilityAccounting(accounting); err != nil {
		return accounting, err
	}
	return accounting, nil
}

func validateReliabilityAccounting(accounting reliabilityAccounting) error {
	if accounting.Planned != accounting.HarnessSkipped+accounting.HarnessRejected+accounting.Started {
		return fmt.Errorf("planned accounting is not conserved: %+v", accounting)
	}
	if accounting.Started != accounting.FailedBeforeDNSSend+accounting.DNSSent {
		return fmt.Errorf("started accounting is not conserved: %+v", accounting)
	}
	postSend := 0
	for _, count := range accounting.PostSendTerminals {
		postSend += count
	}
	if accounting.DNSSent != postSend {
		return fmt.Errorf("dns_sent accounting is not conserved: %+v", accounting)
	}
	return nil
}

func isReliabilityPostSendTerminal(terminal reliabilityTerminal) bool {
	switch terminal {
	case reliabilityTerminalCorrectOnTime, reliabilityTerminalCorrectLate,
		reliabilityTerminalTimeout, reliabilityTerminalTransportError,
		reliabilityTerminalProtocolError, reliabilityTerminalWrongResponse:
		return true
	default:
		return false
	}
}

func ptrInt64(value int64) *int64 { return &value }

func ptrBool(value bool) *bool { return &value }
