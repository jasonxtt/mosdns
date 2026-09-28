# Design: matcher adapter typed-nil errors

## Contract and cause

The private Linux+cgo helpers return concrete pointer types. Returning those
results directly from public functions whose result types are interfaces can
produce a non-nil interface containing a nil pointer when the helper returns
`(nil, err)`. The error contract should instead expose an untyped nil interface
on every failure.

## Change boundary

In each public constructor, retain the concrete result locally, return `nil,
err` when creation fails, and convert to the public interface only on success.
The constructors are `BuildDomainSnapshot`, `BuildIPSnapshot`, and
`BuildValuedDomainSnapshot` in `adapter_linux.go`. No FFI, Rust, Go fallback,
selection, or runtime matcher behavior changes are needed.

Linux+cgo integration tests will exercise one failing create through each
public API. Domain and valued-domain inputs use the existing Rust-unsupported
regexp case; the IP case uses a syntactically invalid prefix. Assertions will
check both the error and direct interface nilness, without calling `Close`.

## Compatibility

Successful Rust snapshots, Go-side ASCII preflight errors, backend selection,
and all non-Linux/stub builds remain unchanged. This corrects only Go interface
nil semantics for errors returned by the Rust create helpers.
