# Logging Guidelines

## Local pattern

The backend uses non-nil `*zap.Logger` instances. Process-level code obtains it from `Mosdns.Logger()` or `mlog.L()`; plugins use the logger supplied by their bootstrap context. See `coremain/mosdns.go`, `pkg/upstream/doh/upstream.go`, and `plugin/server/udp_server/udp_server.go`.

- `Debug`: request-level diagnostics that are too noisy for normal operation.
- `Info`: lifecycle transitions, loaded configuration, listener startup, and successful operator actions.
- `Warn`: a recoverable failure, retry, fallback, or retained previous state.
- `Error`: an operation failed and cannot be completed at that boundary.

Use structured `zap` fields (`zap.String`, `zap.Bool`, `zap.Duration`, `zap.Error`) rather than interpolating data into the message. Include identifiers needed to diagnose the event, such as plugin tag, path, upstream, backend, or operation.

## Rust migration logging

Rust libraries should return structured status to Go; the Go integration boundary owns operator-facing logs so formatting stays consistent. Backend enable/disable, ABI mismatch, panic containment, fallback, dump import/export, and reload results must be observable without logging every cache lookup.

## Sensitive data

Do not log passwords, tokens, private credentials, full configuration secrets, or raw private credential files. Avoid logging full DNS payloads or high-volume query data outside the established audit subsystem. Repository documentation may contain non-secret host/path notes but never credentials.
