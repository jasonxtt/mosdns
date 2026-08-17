# Design — Rust Phase 3 query-state and wire foundation

## Boundary and ownership

The first Phase 3 increment has one coarse call and keeps the existing Go
context authoritative:

```text
Go EntryHandler / query_context.Context
              |
       immutable v1 snapshot
              |
      one coarse query ABI call
              |
   Rust dns-core + query snapshot handle
              |
       caller-owned result bytes
              |
       Go parity oracle / fallback
```

Go owns the live `query_context.Context`, cancellation, plugin values, marks,
audit fields, response selection, and all lifecycle decisions. The adapter
copies only the fields listed in the v1 schema into a Rust-owned immutable
snapshot. Rust may inspect and transform that copy until its handle is closed;
it must not retain Go pointers, mutate Go memory, or return Rust-owned pointers.
The caller owns every output buffer and decides whether to apply a result to a
live context in a later, explicitly approved slice. This slice therefore cannot
leave a partially updated Go request on any error path.

## Pure Rust wire layer

Add `rust/dns-core` as a dependency-light pure Rust crate. It contains no cgo,
Go types, server state, plugin registry, or network I/O.

The initial module surface is deliberately small:

- strict query header/question inspection: query bit, supported opcode,
  exactly one question, legal names and compression, class/type bounds;
- EDNS inspection: presence, advertised UDP size, DO bit, and ECS family,
  source/scope, and fixed-width address (zero-padded/truncated; no bit
  masking);
- response validation and TTL walking that skips OPT records;
- copy-before-patch TTL aging/replacement with saturating arithmetic;
- response ID/RA patching and pure UDP/stream/HTTP framing helpers.

Malformed input or an unsupported operation returns a typed error before any
caller-visible output is changed. Existing `cache-core` wire behavior must use
these routines through a compatibility shim or shared implementation, without
changing the cache public API, status values, or cache semantics. The crate
must preserve the current cache byte/field results for the existing fixtures.

## Versioned snapshot and result schema

The v1 schema uses fixed-width little-endian scalars, explicit lengths,
reserved flags, and checked nested/total lengths. It is carried by the existing
static library rather than a second runtime library.

The input snapshot contains only:

- schema version and flags;
- bounded raw DNS query bytes;
- `from_udp`, advertised UDP payload size, and stream/HTTP mode;
- pre-fast flags needed by the wire operation.

The result contains only caller-owned, versioned data:

- normalized header/question fields;
- EDNS/DO/ECS values and presence flags;
- a transformed response wire buffer when the requested operation produces
  one;
- required output length before a caller writes into a buffer.

There are no maps, plugin objects, audit objects, cancellation pointers, or
Rust-owned output allocations in this ABI. Unknown flags and future reserved
fields are rejected or ignored according to the explicit v1 contract, never by
guessing from a struct size.

Slice 2 freezes the first concrete records in the existing runtime header:
`MosdnsQuerySnapshotInput` is a 48-byte `repr(C)` record with an exact
`struct_size`, version 1, zero-only reserved fields, a by-value borrowed query
slice, transport metadata, and pre-fast flags. `MosdnsQueryInspectResult` is
an 80-byte record with status, normalized ID/question fields, separate input
and EDNS UDP sizes, EDNS/DO/ECS presence and fixed 16-byte address storage, and
required/written lengths. The first coarse inspect operation copies the
immutable query wire into caller storage; response transforms remain a later
operation. Query capability bits occupy bits 5 and 6, and query handles use a
`0x4...` namespace disjoint from the existing cache and matcher ranges.

## Runtime ABI and handles

Extend the existing `mosdns_cache_core.h` and its single static library with a
`Query Snapshot v1` section. Keep cache, matcher, and valued-domain symbols
source-compatible. Add capability/version negotiation and these coarse
operations (exact names are finalized with the red tests): create an immutable
snapshot, inspect/transform into caller buffers, query required output length,
and close.

All structs are `repr(C)`-compatible fixed-width records. Borrowed input and
writable output slices carry pointer/length pairs; null and overflow cases are
validated before dereference. Query handles use a namespace disjoint from
cache/matcher handles. The registry stores immutable snapshots behind the
existing runtime synchronization model; an in-flight read guard keeps a close
from freeing a snapshot until the operation returns. Close accepts only the
exact query handle, is deterministic and idempotent per the contract, and
cannot close a replacement handle. Rust panics are caught at the ABI boundary
and converted to the established status/error form.

The default and non-cgo builds expose the same Go-facing seam as a stub that
always selects the Go oracle. The opt-in selector is
`MOSDNS_QUERY_BACKEND=rust`; it must not load Rust when unset or set to `go`.

## Go adapter placement and compatibility

Place the shared snapshot encoder, decoder, Go oracle, and backend selector in
`pkg/query_context/rust_bridge` (or the repository's final package-equivalent
chosen by the red tests). Keep Linux cgo binding code and the default/non-cgo
stub behind build tags. The adapter performs one coarse call, retries only
when the Rust result reports a required buffer length, compares the opt-in
result with the Go oracle, and falls back to that oracle on build, ABI,
unsupported-input, timeout, panic, or runtime failure.

The adapter never changes `EntryHandler`, `plugin/executable/sequence`,
upstream code, listeners, configuration, metrics, audit data, or WebUI in this
task. A later sequence slice may consume this seam, but this slice returns
parity data without transferring live query ownership.

## Failure matrix

| Condition | Required result |
| --- | --- |
| cgo unavailable, selector unset, or selector `go` | Use the Go oracle; do not load Rust |
| ABI version/capability/length mismatch | Typed adapter error, then the same Go oracle result |
| malformed query/response or unsupported wire operation | No partial output; typed error and Go fallback |
| Rust panic or poisoned handle | Caught ABI error; close only the exact active handle; Go fallback |
| concurrent inspect and close | Read guard keeps the snapshot alive; close completes after the read |
| output buffer too small | Return required length without writing; retry with caller-owned buffer |
| empty optional field or empty ruleset | Follow v1 presence/length contract; remain a valid snapshot |

## Verification strategy

Go golden vectors are derived from the existing miekg/query-context and
server-handler behavior and cover valid, malformed, unsupported, empty, and
non-mutation cases. `dns-core` adds pure unit/property/malformed tests and
cache compatibility tests. ABI tests cover negotiation, handle namespaces,
length checks, required lengths, caller buffers, panic containment, concurrent
close, and misuse. Adapter tests run default, non-cgo, tagged-stub, cgo, and
race variants and compare every opt-in result with the Go oracle.

Linux is the first real cgo target. Host verification uses an isolated process,
temporary configuration, random high ports, and no port 53 or production
service. Full repository tests, vet, build, `CGO_ENABLED=0`, Rust fmt/test/
clippy/release, and a final scope audit are required before the task can be
marked complete.
