package transport

import (
	"net"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/dnsutils"
	"github.com/IrineSistiana/mosdns/v5/pkg/pool"
)

// newDummyEchoNetConn creates a simple in-memory echo connection for reuse transport tests.
// readErrMode >= 1 closes the peer immediately to force an exchange failure.
// respDelay delays each echoed response so tests can trigger read deadlines/timeouts.
// writeErrMode is kept for compatibility with existing test call sites.
func newDummyEchoNetConn(readErrMode int, respDelay time.Duration, writeErrMode int) NetConn {
	client, server := net.Pipe()
	echoDelay := respDelay
	if echoDelay <= 0 {
		echoDelay = time.Millisecond
	}

	go func() {
		defer server.Close()

		if readErrMode >= 1 || writeErrMode >= 1 {
			return
		}

		for {
			msg, err := dnsutils.ReadRawMsgFromTCP(server)
			if err != nil {
				return
			}

			time.Sleep(echoDelay)

			if _, err := dnsutils.WriteRawMsgToTCP(server, *msg); err != nil {
				pool.ReleaseBuf(msg)
				return
			}
			pool.ReleaseBuf(msg)
		}
	}()

	return client
}
