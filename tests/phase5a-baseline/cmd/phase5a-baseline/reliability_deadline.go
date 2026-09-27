package main

import (
	"context"
	"time"
)

type reliabilityDeadlines struct {
	Planned    time.Duration
	Service    time.Duration
	Collection time.Duration
}

func makeReliabilityDeadlines(planned, requestDeadline, lateDrain time.Duration) reliabilityDeadlines {
	service := planned + requestDeadline
	return reliabilityDeadlines{Planned: planned, Service: service, Collection: service + lateDrain}
}

func reliabilityRemainingBudget(deadline, now time.Duration) time.Duration {
	if now >= deadline {
		return 0
	}
	return deadline - now
}

func reliabilityIsLate(deadlines reliabilityDeadlines, responseAt time.Duration) bool {
	return responseAt > deadlines.Service && responseAt <= deadlines.Collection
}

type reliabilityRequest struct {
	SlotID     int64
	Case       workloadCase
	FrameBytes int
	Payload    []byte
}

type reliabilityExchangeResult struct {
	BytesWritten      int
	FrameBytes        int
	BytesRead         int
	WriteStart        time.Duration
	WriteComplete     time.Duration
	HasWriteStart     bool
	HasWriteComplete  bool
	WriteDeadlineRace bool
	ResponseAt        time.Duration
	ResponseOK        bool
	WrongResponse     bool
	ProtocolError     bool
	TimedOut          bool
	Err               error
	DNSSent           bool
	Terminal          reliabilityTerminal
}

type reliabilityExpected struct {
	ServiceDeadline    time.Duration
	CollectionDeadline time.Duration
}

func classifyReliabilityExchange(result reliabilityExchangeResult, expected *reliabilityExpected) reliabilityExchangeResult {
	if result.FrameBytes > 0 && result.BytesWritten < result.FrameBytes {
		result.DNSSent = false
		result.Terminal = reliabilityTerminalFailedBeforeDNSSend
		return result
	}
	if result.FrameBytes > 0 {
		result.DNSSent = true
	}
	if !result.DNSSent {
		result.Terminal = reliabilityTerminalFailedBeforeDNSSend
		return result
	}
	if expected == nil {
		if result.ResponseOK {
			result.Terminal = reliabilityTerminalCorrectOnTime
		}
		return result
	}
	if result.ResponseAt <= 0 || result.ResponseAt > expected.CollectionDeadline || result.TimedOut {
		result.Terminal = reliabilityTerminalTimeout
		return result
	}
	if result.ProtocolError {
		result.Terminal = reliabilityTerminalProtocolError
		return result
	}
	if result.WrongResponse {
		result.Terminal = reliabilityTerminalWrongResponse
		return result
	}
	if !result.ResponseOK {
		result.Terminal = reliabilityTerminalTransportError
		return result
	}
	if result.WriteDeadlineRace || result.ResponseAt > expected.ServiceDeadline {
		result.Terminal = reliabilityTerminalCorrectLate
	} else {
		result.Terminal = reliabilityTerminalCorrectOnTime
	}
	return result
}

type reliabilityExchange interface {
	Exchange(context.Context, reliabilityRequest, reliabilityDeadlines) reliabilityExchangeResult
}

type reliabilityExchangeFunc func(context.Context, reliabilityRequest, reliabilityDeadlines) reliabilityExchangeResult

func (f reliabilityExchangeFunc) Exchange(ctx context.Context, request reliabilityRequest, deadlines reliabilityDeadlines) reliabilityExchangeResult {
	return f(ctx, request, deadlines)
}

type reliabilityClock interface {
	Now() time.Duration
	WallNow() time.Time
	SleepUntil(context.Context, time.Duration) error
}

type realReliabilityClock struct {
	started time.Time
}

func newRealReliabilityClock() *realReliabilityClock {
	return &realReliabilityClock{started: time.Now()}
}
func (c *realReliabilityClock) Now() time.Duration { return time.Since(c.started) }
func (c *realReliabilityClock) WallNow() time.Time { return time.Now() }
func (c *realReliabilityClock) SleepUntil(ctx context.Context, target time.Duration) error {
	remaining := target - c.Now()
	if remaining <= 0 {
		return nil
	}
	timer := time.NewTimer(remaining)
	defer timer.Stop()
	select {
	case <-timer.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
