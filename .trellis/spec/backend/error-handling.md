# Error Handling

## Go conventions

- Validate configuration at constructors and return contextual errors. Existing code normally wraps causes with `%w`, for example `coremain/run.go`, `pkg/upstream/upstream.go`, and `plugin/switch/switch/switch.go`.
- Use sentinel errors only when callers need `errors.Is` or a stable condition, as in `coremain/update_manager.go` and `pkg/dnsutils/net_io.go`.
- Log an error at the operational boundary that handles it; do not log and repeatedly wrap the same failure at every layer.
- Preserve the current HTTP status/body behavior when replacing an API implementation. Tests such as `coremain/api_special_groups_test.go` and `coremain/config_manager_api_test.go` are the contract.

When returning a concrete pointer through an interface, handle the error before
converting the pointer to the interface. A nil concrete pointer stored in an
interface is not a nil interface, so return an untyped `nil` on failures:

```go
func buildSnapshot() (Snapshot, error) {
	concrete, err := buildConcreteSnapshot()
	if err != nil {
		return nil, err
	}
	return concrete, nil
}
```

Do not directly forward `(*snapshot, error)` from a helper into a public
`(Snapshot, error)` result; when the helper returns a nil `*snapshot` and an
error, callers can observe a non-nil `Snapshot` and invoke its methods.

## Rust and FFI conventions

- No Rust panic may cross `extern "C"`. Catch it at the boundary and return a stable status code.
- Expose opaque handles, fixed-width values, and byte buffers with explicit ownership. The allocator that creates a buffer must provide its matching release function and enough metadata to free it correctly.
- Define deterministic behavior for null pointers, invalid lengths, duplicate close, concurrent close, and lock poisoning.
- Convert Rust failures into observable Go errors and counters. The cache bridge must have an explicit policy for one-request Go fallback versus disabling the Rust backend.
- Never treat malformed DNS wire data or dump data as trusted input.

## Avoid

- `unwrap()` or `expect()` on an FFI-reachable path.
- Reconstructing a Rust `Vec` by assuming capacity equals returned length.
- Silently changing an API response, cache miss semantics, or startup behavior during migration.
- Continuing with partially loaded config when the established Go path treats it as fatal; when current code intentionally warns and keeps the previous state, preserve that behavior.
