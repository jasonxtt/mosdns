# Implementation plan — Rust Phase 3 query-state and wire foundation

## Working agreement

Implement one slice at a time using red behavior tests, the smallest green
implementation, and slice-level validation. Stop after each slice for root
review; do not start the next slice until that review is explicitly complete.
Do not stage, commit, archive, or start a later phase as part of slice work.
Preserve unrelated dirty-worktree files, including WebUI, coremain, OpenWrt,
and local files.

Current task status: this task is `in_progress` on branch `rust`. Slices 0–4
are root-reviewed; Slice 4 final root review was approved on 2026-08-17 after
the narrow Rust wire/parity remediation recorded below. Rust remains
experimental / opt-in and the default backend remains Go-only; the query
adapter is not wired into `EntryHandler`, `sequence`, `upstream`, or
`server/listener`. Phase 4 has not started and is not authorized.

Implementation is coordinated through the independent Luna agent running in
the neighboring Herdr pane. The root Codex session owns the plan, sends the
slice brief through Herdr, reads Luna's completion report, and performs the
review and final validation. Luna is a separate full agent window, not a
Trellis-native implement/check sub-agent. The root must still stop after every
slice and never let a completion report implicitly approve the next slice.

## Test seams and mock boundaries

The public boundary under test is the smallest one introduced by each slice;
tests must not mock the behavior they claim to prove.

- Slice 0 tests the Go snapshot/oracle boundary with in-memory DNS fixtures.
  There is no Rust call and no `EntryHandler` replacement; copied input bytes
  and caller-owned result buffers are the ownership seam.
- Slice 1 tests pure `dns-core` functions directly. Cache compatibility tests
  use fixed wire vectors, not a fake parser or fake cache implementation.
- Slice 2 tests the C ABI through its exported records and functions. Registry
  and panic/length cases are exercised with real Rust handles; only the Go
  caller's input/output buffers are test doubles, never Rust internals.
- Slice 3 tests the Go backend selector against a narrow fake ABI seam for
  version, required-length, and runtime-failure branches, then runs the real
  Linux cgo binding separately. The Go oracle is the non-Rust reference, not a
  mock of Rust behavior.
- Slice 4 uses the real static library and isolated host process; no network
  service or production listener is substituted for the opt-in boundary.

## Slice map

### Slice 0 — Go snapshot and oracle contract

**Boundary:** `pkg/query_context` test seam and the future adapter contract;
the existing Go context and server behavior remain authoritative.

**Red tests first:** add deterministic Go golden fixtures for query header and
question validation, EDNS/DO/ECS extraction, raw response ID/RA behavior,
UDP/stream framing decisions, TTL observations, malformed input,
unsupported input, empty fields, and non-mutation. Add a snapshot/result seam
test that makes ownership and required lengths explicit without calling Rust.

**Minimal green:** implement only the Go snapshot encoder, Go oracle, and
caller-owned result model required by those fixtures. Do not add a Rust build,
sequence dispatch, or live `EntryHandler` integration.

**Validation and review gate:** run focused Go tests, focused race tests,
`gofmt`, and `git diff --check`. Review field ownership, copied bytes,
presence/length flags, and parity against current `query_context` and
`server_handler` behavior before Slice 1.

**Progress (2026-08-15):** The first red/green behavior under this slice is
reviewed and passed: `pkg/query_context/rust_bridge` now proves copied input,
caller-owned result buffers, required-length reporting, untouched tail bytes,
and no mutation on a too-small-buffer error. Header/question, EDNS/DO/ECS,
TTL, and framing behaviors remain pending in this same Slice 0; do not treat
this progress entry as approval to start Slice 1.

The header/question behavior is also reviewed and passed: malformed versus
unsupported classification, QR/opcode/count checks, strict question
type/class truncation, name/compression parsing via the existing Go DNS parser,
and the answer/authority count-overflow regression are covered. The single
extra record is now parsed for the EDNS/DO/ECS behavior below.

The EDNS/DO/ECS behavior is reviewed and passed. The Go seam reports OPT
presence, advertised UDP size, DO, and a deep-copied first ECS option with
family, source netmask/scope, and fixed-width address bytes. Malformed OPT
RDLENGTH, option length, and undersized ECS payloads return ErrMalformedQuery
without creating a snapshot or mutating input; unknown options and a valid
non-OPT extra remain compatible with the Go parser. The test fixture's
option-length-overflow case was corrected to mutate the actual option-length
field, and focused tests, race tests, vet, formatting, and diff checks pass.
The documented miekg leniencies (family 0, extra ECS bytes, owner name, and
scope semantics) remain caveats for the Rust dns-core parity slice. A valid
non-OPT extra remains accepted by the Go oracle; the Rust foundation now
rejects non-OPT RDATA as explicitly unsupported after generic body bounds so
the adapter falls back instead of accepting an unvalidated record.

The response-TTL behavior is reviewed and passed. `ResponseSnapshot` validates
the response wire using the cache-compatible walk, `ObserveTTL` reports the
minimal non-OPT TTL and count, and `AgeTTL`/`ReplaceTTL` patch only non-OPT
record TTLs on caller-owned copies. Construction rejects malformed bounds and
QR-clear input without mutating the caller; trailing bytes and bounded
compression pointers retain the cache-wire contract. The saturation-to-zero
choice is explicit and differs from the decoded `dnsutils.SubtractTTL` floor
of one. The repeated-result ownership assertion was corrected so separate
`AgeTTL` calls are proven not to alias. Focused tests, race tests, vet,
formatting, and diff checks pass. Response ID/RA and framing remain pending;
Slice 1 is not approved by this progress entry.

The response-header ID/RA behavior is reviewed and passed. The oracle writes
the request ID as a big-endian uint16 and sets only the wire RA bit on a
caller-owned copy; QR, TC, RD, opcode, RCODE, counts, record bytes, and the
immutable snapshot remain unchanged. Non-response and malformed inputs are
rejected during `NewResponseSnapshot`, before patching. The focused tests
cover empty/error responses, OPT-only responses, zero IDs, flag/count
preservation, and independent returned buffers. The final cleanup removed the
unused QR constant and uses the RA constant at the patch site; focused tests,
race tests, vet, formatting, and diff checks pass. UDP truncation and
UDP/stream/HTTP framing remain pending; Slice 1 is not approved.

The response framing behavior is reviewed and passed, completing the planned
Slice 0 Go oracle behaviors. `FrameResponse` returns caller-owned copies for
UDP and HTTP pass-through, prefixes stream/TCP responses with a two-byte
big-endian length, and rejects oversized UDP/stream responses without partial
output. HTTP/DoH remains an unprefixed pass-through even above 65535 bytes,
matching the `streamTransport=false` raw-response branch. `EffectiveUDPSize`
freezes the 512-byte floor; decoded-message UDP truncation, response options,
and transport listeners remain explicit caveats. Focused and baseline tests,
race tests, vet, formatting, and diff checks pass. Slice 1 has not started.

### Slice 1 — Pure `dns-core` wire layer

**Red tests first:** add Rust tests for strict query/header/question parsing,
EDNS/DO/ECS extraction, malformed compression and lengths, TTL aging and
replacement (including OPT skipping and saturation), response ID/RA patching,
and UDP/stream/HTTP framing. Add byte/field parity vectors for the existing
`cache-core` wire behavior.

**Minimal green:** create `rust/dns-core` and the smallest pure APIs that make
the tests pass. Route `cache-core` through a compatibility shim or shared
routine only where the existing tests prove no semantic change. Do not expose
the query ABI yet.

**Validation and review gate:** run the crate tests, cache-core tests, Rust
format check, and the relevant Go parity tests. Review bounds, compression,
OPT handling, TTL saturation, malformed-input no-partial-output behavior, and
cache API/status stability before Slice 2.

### Slice 1 — Completion (root-reviewed)

Pure `dns-core` wire layer is complete and every review finding has been
accepted by root. `rust/dns-core` exposes typed-error pure APIs mirroring the
frozen Go oracle: strict query header/question parsing (QR/opcode/question
count/section checks, label/compression bounds, multi-pointer compression
cycle rejection), EDNS/DO/ECS extraction (non-root OPT owners, fixed-width
zero-padded/truncated ECS addresses with family/netmask/scope validation),
response validation and TTL observation/aging/replacement (declared-count
walk, OPT skip, saturating arithmetic), and response ID/RA patching plus
UDP/stream/HTTP framing (caller-owned copies, length guards, typed errors, no
production panics). The Go oracle continues to report valid non-OPT extras as
EDNS absent; the Rust foundation explicitly rejects non-OPT RDATA as
unsupported after generic body bounds so the adapter falls back safely.
Unused public markers were removed.

Reviewed findings resolved and verified: multi-pointer compression cycle
rejection; crate-root re-exports (`parse_query`, `QueryError`, `HeaderError`);
production no-`expect`/no-panic walk; family-0 ECS 16-byte v4-mapped zero
address; non-root OPT owner parsing; bounded pointer-following owner walker
with self/cycle/out-of-slice rejection (prior-packet targets documented as out
of scope); `patch_response_id_ra` returning `HeaderError` (TooShort /
NotResponse) with no partial output; fixed-width-address documentation and
minimal public surface.

Validation gates passing: cargo fmt `--check`, `cargo test --all-targets
--locked` (49 dns-core lib + parity + public-api + all workspace suites),
clippy `-D warnings` on all targets, `cargo build --release --locked`, the Go
oracle/parity focused tests, and `git diff --check`.

Rust remains experimental with the default backend still Go-only: no ABI,
cgo, Go adapter, server, sequence, upstream, or cache production changes were
made, and no shared-routine replacement was applied to `cache-core`
(byte/field parity with the existing cache wire walk is proven by tests).
Slice 2 (versioned Rust query ABI) has not started.

### Slice 2 — Versioned Rust query ABI

**Red tests first:** add ABI/header tests for version and capability
negotiation, fixed-width records, total/nested length checks, required output
length, caller-owned buffers, null/overflow rejection, query-handle namespace,
concurrent inspect/close, panic containment, exact-handle close, and misuse.

**Minimal green:** extend the existing `mosdns_cache_core.h` and runtime
static library with immutable query snapshot handles and one coarse
inspect/transform call. Keep existing cache/matcher/valued-domain symbols
unchanged. Add only the registry synchronization and status plumbing needed
by the red tests.

**Validation and review gate:** run focused Rust tests, the complete Rust
workspace tests, fmt, clippy with warnings denied, and a release build. Review
ABI layout, ownership, caller buffers, panic safety, handle replacement/close,
and absence of cache/matcher ABI drift before Slice 3.

### Slice 2 — Completion (root-reviewed)

The versioned query snapshot ABI is complete. The existing runtime static
library/header now expose query capability/version negotiation, fixed-width
48-byte input and 80-byte result records, by-value borrowed descriptors,
immutable snapshot creation, required-length reporting, caller-owned wire
inspection, EDNS/DO/ECS metadata, and deterministic exact-handle close. Input
version/size/flags/reserved/transport checks reject malformed records before a
handle is published; null and `isize`-overflow descriptors are rejected before
slice construction. Short outputs return `BufferTooSmall` with the exact
required length and do not write output bytes.

The registry uses a `0x4...` query namespace, monotonic non-reused handles,
read-locked inspection, write-locked close, and explicit cache/matcher
namespace misuse tests. Lock-poison/error paths return statuses, and the ABI
boundary catches panics. Existing cache, matcher, and valued-domain symbols
remain unchanged apart from the additive query capability bits; the cache wire
implementation remains independent.

Validation passed: focused and all-target Rust tests (including 11 query ABI
integration tests), Rust fmt, clippy with warnings denied, release build,
C header compile/size probe, focused Go query/server tests,
focused Go race tests, Go vet, and `git diff --check`. No Go adapter, cgo
binding, server, sequence, upstream, WebUI, or default-backend behavior was
changed. Rust remains experimental and Go-only by default. Slice 3 has not
started.

### Slice 3 — Go adapter and opt-in fallback

**Red tests first:** add tests for `MOSDNS_QUERY_BACKEND` default/unset/`go` /
`rust`, non-cgo stub selection, schema and buffer retry, Go-oracle parity,
runtime failure fallback, malformed/unsupported fallback, and proof that the
live context is not mutated.

**Minimal green:** add the shared Go encoder/decoder/oracle seam, Linux cgo
binding, and default/non-cgo stub. Select Rust only for the explicit env value;
on any construction, ABI, timeout, panic, or runtime error return the Go
oracle result. Keep `EntryHandler`, `sequence`, upstream, listeners, and
configuration untouched.

**Validation and review gate:** run focused default and race tests, tagged
stub tests, `CGO_ENABLED=0 go test`, Linux cgo normal/race tests, and the
relevant Rust ABI tests. Review fallback determinism and the no-live-mutation
property before Slice 4.

### Slice 3 — Completion (root-reviewed)

The Go adapter and opt-in fallback are complete. `MOSDNS_QUERY_BACKEND`
selects Rust only for the explicit case-insensitive value `rust`; default,
`go`, and stub builds always return the Go oracle. The shared oracle/compare
seam (`GoOracle`, `Inspect`, `FallbackError` with typed reasons incl.
`FallbackPanic`, `FallbackABIMismatch`, `FallbackInvalidResult`) is in
`pkg/query_context/rust_bridge`; the Linux cgo binding (`adapter_linux.go`)
and the non-cgo/default stub (`adapter_stub.go`) are build-tagged
(`linux && cgo && mosdns_rust`). The adapter performs one create →
required-length → inspect → close round trip, retries only on a reported
required length, copies caller-owned output, compares every result against
the Go oracle, and falls back to the same-generation Go result on any
construction/version/capability/runtime/panic/validation error without
mutating the live request. `adapter_linux_test.go` drives the real
`newNativeABI`/`cgoQueryABI` against the actual static library for plain
query, ECS, repeated-call handle lifecycle, no-alias, and
BUFFER_TOO_SMALL-required-length paths.

Validation passed on macOS (default, `CGO_ENABLED=0`, tagged stub, and race
variants) and on Linux `mos-test`; see the Slice 4 evidence below. No
`EntryHandler`, `sequence`, upstream, listener, or config change was made,
and Rust remains opt-in/Go-only by default. Slice 4 evidence and its review
remediation were approved by final root review on 2026-08-17.

### Slice 4 — Evidence, host check, and handover

Run fixed fixture-based performance/size checks; do not use bridge allocations
per operation or Rust RSS as release claims. Run Linux ABI/header normal and
race checks, Rust fmt/test/clippy/release, and isolated host verification with
temporary configuration and random high ports. Then run the full Go test,
vet, build, `CGO_ENABLED=0`, tagged-stub, and `git diff --check` gates.

Update only the Rust migration spec/handover/rewrite-plan notes that are
needed to record the delivered boundary and its caveats. Finish with a root
scope audit proving no sequence, upstream, server, WebUI, OpenWrt, lite,
docker, production, or default-backend behavior changed. Stop for final review
before any task finish/archive action.

### Slice 4 — Completion (root-reviewed, 2026-08-17)

Slice 3 residual audit, timeout contract, Linux+cgo real gate, Rust
ABI/header gate, fixed-fixture performance/size evidence, host
compatibility smoke, full default gates, and documentation/handover are
recorded. The final whole-task review requested changes on 2026-08-17 for
three cross-slice Rust wire/parity gaps. The earlier findings from the
independent review (a real Linux+cgo `DO=true` parity test, and a
recategorization of host smoke versus adapter-boundary evidence) are also
incorporated below. An additional
non-blocking check (full tagged `go test -race -tags mosdns_rust ./...` across
the whole tree on Linux) is pending and recorded as such below; it is not a
Slice 4 blocking completion gate.

**Final whole-task review remediation**

- `dns-core` query-name walking now enforces the miekg-compatible expanded
  DNS wire-name budget of 255 bytes, so overlong names are malformed before a
  snapshot handle can be published.
- Additional-record owner compression is parsed through `extract_edns_at`
  with the full DNS packet and absolute extra-record offset; the previous
  subslice-relative pointer contract is retained only by the explicitly
  standalone-buffer convenience wrapper.
- Non-OPT additional records are checked for owner/fixed-field/body bounds and
  then explicitly rejected as unsupported. This is a safe Rust-foundation
  boundary; the Go adapter uses its existing oracle fallback for valid
  non-OPT records.

These fixes are implemented with Rust and Go regression coverage. The targeted
query ABI Miri run under Tree Borrows also passes all 11 `query_abi` tests.
The final root review approved these remediations on 2026-08-17. Slice 4 is
complete; task finish/archive bookkeeping remains intentionally separate.

**Slice 3 residual audit**

- Default (untagged) build has no Rust/cgo reference: `go build ./...`,
  `go test ./...`, `go vet ./...` all pass with no `-tags mosdns_rust`.
- Non-cgo stub is selected by build tag `!linux || !cgo || !mosdns_rust`;
  stub test proves `Inspect` returns the Go oracle plus a typed
  `FallbackError` when Rust is requested on a stub build.
- `CGO_ENABLED=0 go test ./...` passes; the cgo binding file is excluded by
  its build tag.
- Rust ABI fallback: `FallbackReason` covers unavailable/ABI-mismatch/
  runtime/panic/invalid-result/result-mismatch; panic status maps
  structurally to `FallbackPanic` via `abiStatusError`, so fallback
  classification never depends on error text.
- Panic status: `query.rs` `boundary()` catches panics and returns
  `Status::Panic`; the adapter maps it to `FallbackPanic`.
- Real Linux+cgo test files are consistent with the ABI: see the Linux gate
  section. `adapter_linux_test.go` calls `newNativeABI()`/`cgoQueryABI()`
  (the real staticlib) and is never the fake seam.
- Real Rust tests cover query parity, ECS family/mask/address parity,
  EDNS+DO=true parity, caller-owned output with input unchanged, required
  length, short buffer, close/double close/closed handle, and repeated
  calls, all through the real staticlib on Linux; the fake-ABI failures in
  `adapter_test.go` cover fallback classification separately.

**Timeout contract**

There is no wall-clock deadline or cancel capability in the query ABI or
Go adapter. The ABI call is a single bounded synchronous inspect into
caller-owned buffers; there is no network, blocking, or long-running
operation inside the Rust boundary that a deadline could bound, and the
task explicitly forbids a goroutine-based cancellation of an unscoped cgo
call. Recorded limitation: the adapter/ABI has **no timeout parameter and no
cancellation; a giant/hung Rust call cannot be abandoned by the caller**.
Blocking cgo calls still occupy a Go thread. This is documented as a
limitation for the future sequence slice, not silently treated as a
timeout. No `context.Context`, deadline, or cancel function is passed into
the ABI; no timeout-specific fallback test exists because there is no
timeout to trigger.

**Linux+cgo real gate (real staticlib, `mos-test` 10.0.0.91)**

Host: `x86_64`, Linux, Go 1.26.4, cargo/rustc 1.95.0, `CGO_ENABLED=1`,
source synced from branch `rust`, Rust release static library freshly built
there.

- `cargo build --manifest-path rust/Cargo.toml --release --locked` — OK
  (8.8s, staticlib `libmosdns_runtime.a` 29,417,008 bytes on Linux).
- `CGO_ENABLED=1 go test -tags mosdns_rust -count=1 -v
  ./pkg/query_context/rust_bridge/` — PASS, and every `TestRealRust*` name
  (including the new DO test) appears in the `-v` output.
- `CGO_ENABLED=1 go test -race -tags mosdns_rust -count=1 -v
  ./pkg/query_context/rust_bridge/` — PASS, with the same `TestRealRust*`
  set executed under the race detector.
- The six `TestRealRust*` tests drive the real `newNativeABI`/`cgoQueryABI`
  (not the fake seam): plain query parity, ECS family/mask/address parity,
  EDNS+DO=true parity (`adapterQueryWithDO`, OPT present with TTL
  `0x00008000`, no ECS), repeated-call `create -> required-len -> inspect
  -> close` handle lifecycle, caller-owned no-alias output, exact
  short-buffer BUFFER_TOO_SMALL required-length without writing, double
  close idempotent, use-after-close returns CLOSED. All pass, also under
  `-race`.

**Rust ABI/header gate**

Executed on macOS (and repeated on Linux): `cargo fmt --check` (OK), `cargo
test --all-targets --locked` (9 suites OK: 7 cache-core + 49 dns-core + 1
cache parity + 2 public-api + 48 matcher-core + 2 runtime + 18 abi_contract
+ 11 query_abi + 5 valued_abi), `cargo clippy --all-targets --locked --
-D warnings` (OK), `cargo build --release --locked` (OK). C header probe
(gcc syntax compile + `sizeof`/`offsetof` vs `query_abi.rs` asserts):
`MosdnsQuerySnapshotInput` = 48 bytes, `MosdnsQueryInspectResult` = 80
bytes; `query_wire` offset 16, `pre_fast_flags` offset 40, result
`required_len` offset 48, `ecs_address` offset 64. Exported symbols in
`libmosdns_runtime.a` match the header: `query_abi_version`,
`query_abi_capabilities`, `query_snapshot_create/required_len/inspect/
close`, plus the unchanged cache/matcher/valued symbols; capability bits 5/6
and version 1 agree between Go and Rust. No cache/matcher/valued ABI drift.

**Fixed fixture perf/size evidence (final Linux code, fixed params, 3 runs)**

Fixture = example.com A + EDNS Client Subnet family 1 / mask 24 / 1.2.3.0.
On Linux `mos-test` (x86_64), 50000x across 3 runs:
`BenchmarkQueryInspectRustAdapter`: 1167 / 1524 / 1523 ns/op, 768 B/op,
22 allocs/op, cgo_calls/op = 6.000. The Go oracle on the same host:
465.1 / 655.9 / 792.1 ns/op, 456 B/op, 15 allocs/op. Final release static
library: 29,417,008 bytes; no dylib — ABI ships statically only.

Not claimed: these are Go-observable bridge numbers (Go-side allocs/op,
B/op, and Go-observed cgo call count) for one coarse query boundary call,
**not** Rust heap/RSS; they cannot be extrapolated to production or used as
a Rust-pathtime release claim. The 6 cgo calls/op is the round-trip count
of the adapter path (create + required-len + inspect + close, plus
version/capability), not a per-DNS-packet claim once a later slice
consumes the seam.

**Evidence split: adapter boundary vs host smoke vs reload path**

The two prior kinds of evidence are intentionally kept separate. The query
adapter is **not wired into `EntryHandler`/sequence**, so a high-port DNS
request goes through the existing Go path and does **not** exercise
`rust_bridge.Inspect`.

A. **Real Linux+cgo adapter boundary verification** (this is the isolated
opt-in query adapter boundary evidence):

- `CGO_ENABLED=1 go test -tags mosdns_rust -count=1 -v
  ./pkg/query_context/rust_bridge/` — PASS (`TestRealRust*`, real
  `newNativeABI`/`cgoQueryABI` → staticlib).
- `CGO_ENABLED=1 go test -race -tags mosdns_rust -count=1 -v
  ./pkg/query_context/rust_bridge/` — PASS.

B. **High-port host compatibility/process smoke** (`mos-test` 10.0.0.91),
using a temp dir/config, random high ports (42555 API / 58761 DNS, no 53),
and a temp process (not the installed service at `/cus`):

- The `mosdns_rust`-tagged full binary starts and serves existing Go DNS
  (health `ready:true`, UDP/TCP answer `192.0.2.1` on high port) with no
  panic/fatal in the log.
- The Go-only binary (no Rust tags) on the same temp config answers the
  same query identically.
- Cleanup: both test processes terminated and the temp dir removed on the
  host; the pre-existing production service was not touched and no port 53
  was used.
- Because the query adapter is not wired into `EntryHandler`/sequence, the
  high-port DNS request itself does not exercise `rust_bridge.Inspect`;
  this is a host compatibility/process smoke, not a query adapter boundary
  execution.

C. **Invalid reload -> HTTP 400** (from the same host smoke) proves only:

- config reload / generation preservation / API transaction behavior.
- It is a config/runtime reload failure-path smoke, **not** a query-adapter
  fallback test. The real query-adapter fallback evidence comes from the
  fake/real adapter fallback tests in `pkg/query_context/rust_bridge`
  (`adapter_test.go`, `adapter_stub_test.go`, and the `FallbackError`
  assertions).

**Full default gates**

macOS (this repo): `go test ./...` (31 pkgs ok), `go test -race ./pkg/
query_context/... ./pkg/server_handler/...` (ok), `go vet ./...` (ok),
`go build ./...` (ok), `CGO_ENABLED=0 go test ./...` (ok), `go test -tags
mosdns_rust ./...` (ok; stub path on macOS), `git diff --check` (ok).
Linux (mirror): `go test ./...`, `go vet ./...`, `go build ./...`, `go test
-tags mosdns_rust ./...`, `CGO_ENABLED=0 go test ./...` — all ok. Default
build does not load Rust; Rust is reached only through the explicit
`MOSDNS_QUERY_BACKEND=rust` opt-in.

**Pending**

- Additional non-blocking check: full `go test -race -tags mosdns_rust ./...`
  across the whole tree on Linux was not run (only the rust_bridge package
  race gate was run on Linux). This is not the current Slice 4 blocking
  completion gate; the required Linux real cgo `rust_bridge` race gate has
  passed.

**Docs**

- `docs/ai/rust-handover.md` updated with the Phase 3 in-progress task row,
  an active-work paragraph (Phase 3 query execution core, not matcher
  expansion), and the evidence summary.
- `implement.md` (this file) records the Slice 4 review fixes: the real
  Linux+cgo `DO=true` parity test, the A/B/C evidence split (adapter
  boundary vs host compatibility smoke vs reload path), the timeout
  limitation, Linux/host platform and results, performance/size evidence
  with non-extrapolation scope, default Go-only/fallback status, and the
  scope audit below.

**Scope audit**

Review-remediation deliverables this round: Rust `dns-core` query/EDNS
validation fixes, Rust runtime full-packet additional-owner wiring and ABI
regressions, plus Go oracle regression fixtures. No `EntryHandler`, sequence,
upstream, listener/server, config, WebUI, coremain, or cache/matcher code was
changed. Rust remains experimental and default Go-only. Nothing was staged,
committed, archived, or pushed, and Phase 4 has not started.

## Validation command set

Use exact package paths after Slice 0 establishes the adapter package; the
baseline command set is:

```text
gofmt -w <changed Go files>
git diff --check
go test ./pkg/query_context/... ./pkg/server_handler/...
go test -race ./pkg/query_context/... ./pkg/server_handler/...
go build ./...
go vet ./...
go test ./...
CGO_ENABLED=0 go test ./...
go test -tags mosdns_rust ./...
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo test --manifest-path rust/Cargo.toml --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path rust/Cargo.toml --release --locked
<Linux+cgo query adapter normal/race and real ABI checks>
<isolated host verification on random high ports>
```

Commands that require a new package or Linux toolchain are added only when the
corresponding slice introduces them; a failed host-only check remains an
explicit caveat and is not silently reported as complete.

## Risks and rollback

- Parser disagreement: keep the Go oracle authoritative and defer the Rust
  operation rather than widening the schema.
- Cache wire regression: remove only the new delegation and preserve the
  existing cache implementation while the pure layer is corrected.
- ABI ownership or close defect: reject the slice, keep Rust disabled, and
  repair the red test/registry contract before proceeding.
- Result mismatch or runtime failure: use the same-generation Go oracle and
  leave the live context untouched.
- Accidental server, sequence, config, WebUI, or deployment edits: stop,
  revert only those task-owned edits, and leave unrelated user changes alone.
