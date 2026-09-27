package main

import (
	"context"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"os"
	"time"

	"github.com/miekg/dns"
)

type netReliabilityExchange struct {
	address   string
	transport string
	clock     reliabilityClock
	dial      func(context.Context, string, string) (net.Conn, error)
}

func newNetReliabilityExchange(address, transport string, clock reliabilityClock) *netReliabilityExchange {
	return &netReliabilityExchange{address: address, transport: transport, clock: clock}
}

func (e *netReliabilityExchange) Exchange(ctx context.Context, request reliabilityRequest, deadlines reliabilityDeadlines) reliabilityExchangeResult {
	result := reliabilityExchangeResult{FrameBytes: request.FrameBytes}
	if err := ctx.Err(); err != nil {
		result.Err = err
		return result
	}
	remaining := reliabilityRemainingBudget(deadlines.Service, e.clock.Now())
	if remaining <= 0 {
		result.TimedOut = true
		result.Err = context.DeadlineExceeded
		return result
	}
	var conn net.Conn
	var err error
	if e.dial != nil {
		conn, err = e.dial(ctx, e.transport, e.address)
	} else {
		dialer := net.Dialer{Timeout: remaining}
		conn, err = dialer.DialContext(ctx, e.transport, e.address)
	}
	if err != nil {
		result.Err = err
		result.TimedOut = isTimeoutError(err)
		return result
	}
	defer conn.Close()
	frame := request.Payload
	if e.transport == "tcp" {
		if len(frame) > int(^uint16(0)) {
			result.Err = errors.New("DNS query too large")
			return result
		}
		framed := make([]byte, 2+len(frame))
		binary.BigEndian.PutUint16(framed, uint16(len(frame)))
		copy(framed[2:], frame)
		frame = framed
	}
	result.FrameBytes = len(frame)
	remaining = reliabilityRemainingBudget(deadlines.Service, e.clock.Now())
	if remaining <= 0 {
		result.Err = context.DeadlineExceeded
		result.TimedOut = true
		return result
	}
	writeResult := writeReliabilityFrameDetailed(conn, frame, deadlines.Service, e.clock)
	written := writeResult.Written
	writeErr := writeResult.Err
	result.BytesWritten = written
	result.WriteStart = writeResult.Start
	result.HasWriteStart = writeResult.HasStart
	result.WriteComplete = writeResult.Complete
	result.HasWriteComplete = writeResult.HasComplete
	result.WriteDeadlineRace = writeResult.DeadlineRace
	if writeErr != nil {
		result.Err = writeErr
		result.TimedOut = isTimeoutError(writeErr)
		return result
	}
	if result.BytesWritten != len(frame) {
		result.Err = io.ErrShortWrite
		return result
	}
	result.DNSSent = true
	readRemaining := reliabilityRemainingBudget(deadlines.Collection, e.clock.Now())
	if readRemaining <= 0 {
		result.TimedOut = true
		result.Err = context.DeadlineExceeded
		return result
	}
	if err := conn.SetReadDeadline(time.Now().Add(readRemaining)); err != nil {
		result.Err = err
		return result
	}
	var responseWire []byte
	if e.transport == "tcp" {
		var length uint16
		if err := binary.Read(conn, binary.BigEndian, &length); err != nil {
			result.Err = err
			result.TimedOut = isTimeoutError(err)
			return result
		}
		if length == 0 || length > dns.MaxMsgSize {
			result.ProtocolError = true
			result.Err = errors.New("invalid DNS TCP response length")
			return result
		}
		responseWire = make([]byte, length)
		if _, err := io.ReadFull(conn, responseWire); err != nil {
			result.Err = err
			result.TimedOut = isTimeoutError(err)
			return result
		}
	} else {
		buffer := make([]byte, dns.MaxMsgSize)
		n, readErr := conn.Read(buffer)
		if n > 0 {
			result.BytesRead = n
			responseWire = append([]byte(nil), buffer[:n]...)
		}
		if readErr != nil {
			result.Err = readErr
			result.TimedOut = isTimeoutError(readErr)
			return result
		}
	}
	if e.transport == "tcp" {
		result.BytesRead = len(responseWire)
	}
	result.ResponseAt = e.clock.Now()
	query := new(dns.Msg)
	response := new(dns.Msg)
	if err := query.Unpack(request.Payload); err != nil {
		result.ProtocolError = true
		result.Err = err
		return result
	}
	if err := response.Unpack(responseWire); err != nil {
		result.ProtocolError = true
		result.Err = err
		return result
	}
	if responseMatches(response, query, request.Case) {
		result.ResponseOK = true
	} else {
		result.WrongResponse = true
		result.Err = errors.New("response failed strict DNS correctness contract")
	}
	return result
}

type reliabilityWriteConn interface {
	Write([]byte) (int, error)
	SetWriteDeadline(time.Time) error
}

func writeReliabilityFrame(conn reliabilityWriteConn, frame []byte, serviceDeadline time.Duration, clock reliabilityClock) (int, error) {
	result := writeReliabilityFrameDetailed(conn, frame, serviceDeadline, clock)
	return result.Written, result.Err
}

type reliabilityWriteResult struct {
	Written      int
	Start        time.Duration
	HasStart     bool
	Complete     time.Duration
	HasComplete  bool
	DeadlineRace bool
	Err          error
}

func writeReliabilityFrameDetailed(conn reliabilityWriteConn, frame []byte, serviceDeadline time.Duration, clock reliabilityClock) reliabilityWriteResult {
	result := reliabilityWriteResult{}
	for result.Written < len(frame) {
		remaining := reliabilityRemainingBudget(serviceDeadline, clock.Now())
		if remaining <= 0 {
			result.Err = context.DeadlineExceeded
			return result
		}
		if err := conn.SetWriteDeadline(time.Now().Add(remaining)); err != nil {
			result.Err = err
			return result
		}
		if !result.HasStart {
			result.Start = clock.Now()
			result.HasStart = true
		}
		n, err := conn.Write(frame[result.Written:])
		if n > 0 {
			result.Written += n
		}
		if err != nil {
			result.Err = err
			return result
		}
		if n == 0 {
			result.Err = io.ErrShortWrite
			return result
		}
		if result.Written == len(frame) {
			result.Complete = clock.Now()
			result.HasComplete = true
			result.DeadlineRace = result.Complete > serviceDeadline
		}
	}
	return result
}

func isTimeoutError(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, os.ErrDeadlineExceeded) {
		return true
	}
	var netErr net.Error
	return errors.As(err, &netErr) && netErr.Timeout()
}
