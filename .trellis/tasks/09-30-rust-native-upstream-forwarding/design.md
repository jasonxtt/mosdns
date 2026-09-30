# Design — bundled native upstream forwarding

Final planning draft for user review. Family default is explicitly approved;
full-scope implementation approval is still required. No runtime work or task
start is authorized by this document alone. Parameter/identity matrix and test
seams are frozen in research/forward-contracts.md; PRD owns R/A acceptance.

## Owners and composition

Extend the existing compiler/catalog/execution seams. A forward owns an ordered
set of entry descriptors and a selection policy. Each descriptor retains plugin
identity, optional entry tag, configured service address, dial override,
bootstrap/family policy, TLS policy and timeout. A compiled invocation resolves
its tagged subset before I/O; unresolved/duplicate selection tags and cross-type references
are configuration errors. Reuse existing compiler source paths for errors.

Use a small closed endpoint/owner enum for UDP, TCP, DoT and DoH. Reuse
`UdpTcpPolicy`, `BootstrapResolver`, `ResolverComposition`, `ReuseOwner`,
`SecureReuseOwner`, `DohReuseOwner` and their existing validation/lifecycle
facilities. No generic transport framework, second runtime, framing/parser,
TLS verifier or resolver is warranted.

### Compiled invocation representation (P1-1)

Keep sequence-core's `External { target: ExecutableId }` unchanged. In the
native compiler allocate one internal external for each actual forward call
site, including unparameterized `$forward`, tagged subset and quick forward.
Its immutable `ForwardInvocationDescriptor` stores the parent `ForwardDefinitionId`
and the ordered original entry indices. Resolve these at compile time and map
the validated external `ExecutableId` to that descriptor in ForwardCatalog.
Subset ordering never reindexes entries or duplicates transport owners.

Quick forward creates a source-owned anonymous forward definition, then uses
the same descriptor/dispatch path. Named definitions remain the canonical owner
of shared transports/resolvers; two invocations of one definition share those
owners while each runtime invocation gets its own attempt ledger/cancel scope.
Internal external names are compiler-reserved and collision-checked against
all externals; they are never public supplier identities or subset aliases.

`primary_forward` traverses existing sequence control flow and maps the first
reachable invocation external to its parent definition and descriptor. The
legacy single numeric-entry introspection helpers retain their narrow behavior;
generalized introspection must return optional/typed descriptor data, never
invent a first-entry runtime fallback for a secure/group invocation. Checkpoints
hold invocation external ID plus original EntryId, not the generated name.

Keep pre-I/O graph construction: syntax, references, numeric addresses and
trust preparation complete before listeners bind. Hostname lookup is lazy on
the host-owned runtime. One configured entry may own a resolver and current
resolved transport; replace the current owner only on a changed published
numeric target. An in-flight exchange keeps its original immutable owner.
Close retired owners after their scoped users finish; never retain a historical
map of every resolved address. Do not cancel an active exchange solely because
another request published a new resolver generation.

## Forward result selection

The proposed fanout is `min(selected entries, normalized concurrent, 3)`;
missing/nonpositive concurrent normalizes to 1 and values above 3 normalize to
3, matching the existing configured count behavior except duplicate attempts.
Quick `forward addr...` defaults to 3 like the existing product entry point.
Choose a random starting entry and adjacent distinct entries with a injectable
selection seam for deterministic tests; no promise of Go PRNG parity.

Among valid correlated replies, preserve current response priority:

1. First completed reply containing a valid Answer A/AAAA wins immediately.
2. Otherwise retain the first completed NOERROR/NXDOMAIN reply; return it only
   when all selected legs have finished.
3. Otherwise retain the first completed other valid reply.
4. With no valid reply, report the first completed typed failure.

Malformed/correlation-invalid replies are failures, never winners. An elapsed
caller deadline or cancellation wins over a buffered lower-priority reply as
in the current host contract. Preserve non-address query response semantics;
do not reinterpret CNAME-only/TXT/MX as an IP winner. Simultaneous ready results
use stable configured-order polling; actual completion ordering remains
observable and timing-dependent, so tests control peer barriers.

Return an owned selected response plus copyable selected EntryId/SocketAddr/
transport enum and ordered typed slot facts. Do not discover a winner afterwards by looking up endpoint
equality or matching response bytes. Cancel and await losers before the scope
returns, collecting completed, failed, timed-out and canceled facts. Preserve
the selected successful response during cleanup; cleanup must not grant a
fresh query deadline or permit a later response to replace the winner.

### Started-entry ledger and abnormal drop (P1-3/P2-1)

For each dynamic invocation allocate a bounded ledger of at most three entry
slots, shared on the current-thread LocalSet by the forward driver and
ExecutionCheckpoint using `Rc<RefCell<...>>`; no borrow spans an await. Each
slot holds invocation ID, original EntryId, selection/start ordinal, optional
numeric peer, last started target transport, phase and optional terminal
outcome. Register a slot before resolution/target I/O; entries never started
have no slot. Bootstrap sends are resolver activity, not extra entry attempts.

A LegGuard owns each slot. Completion writes one terminal outcome exactly
once; guard drop fills only an unfinished slot with canceled if the relevant
scope is canceled, otherwise interrupted. A valid nonwinner keeps response,
not canceled. TC fallback stays within the same entry slot: UDP TC is an
intermediate phase and TCP phase is marked before TCP I/O; one configured-entry
metric attempt is not counted twice. Physical UDP/TCP activity is independently
proven by peer counters and typed phase state.

Attempt order is registration/start order, never completion order. Across
sequential invocations append the next started slots in that order; preserve
original EntryId when subsets reorder the selection. Winner selection remains
based on completion/priority independently of slot order. Preserve zero/one
attempt inline storage and only promote/reserve when actual additional entries
start, with capacity informed by the compiled entry count.

Attach the live ledger to checkpoint before polling child work, so caller
drop does not depend on the exchange returning. Normal cancellation and winner
paths cancel and await every polled leg before returning/sealing the ledger.
On exceptional future drop, an invocation guard first cancels its scope;
remaining leg guards/checkpoint sealing finalize only open slots. The checkpoint
uses this same ledger snapshot once, never appends the old plugin-level
in_flight_executable fallback in addition to entry slots. Sealing and later
guard drops are idempotent; no late write changes the terminal snapshot.

Concretely, extend the native-host ExchangeExecutor seam (not sequence-core)
to `exchange(executable, query, deadline, cancellation, attempts: AttemptSink)`.
AttemptSink is a cloneable handle to that invocation ledger; the execution
driver creates/attaches it to ExecutionCheckpoint before calling exchange and
passes the same handle to the catalog adapter/leg guards. On successful return,
ForwardExchangeResult carries response plus copyable selected supplier facts;
on Err or drop, the checkpoint still owns all started slots. After an awaited
return the driver seals/consumes the ledger once into query-global attempts and
clears the live checkpoint handle. Normal result and partial-drop accounting
never maintain independent attempt arrays. Existing host test executors adapt
to this explicit seam; no arbitrary source file or new core payload is needed.

RAII cannot await asynchronous teardown. It cancels/drops forward-owned child
futures and triggers existing secure scope aborts; stable parent transport and
resolver owners retain drain responsibility. The existing supervisor/catalog
async close must await all registered exchanges and secure children, including
dropped invocations and retired owners, before reporting drain/rebind. Forced
drop tests must await this drain and separately assert ledger completeness;
an interrupted slot alone is not resource-release proof.

Configured identity is compiled once. Pending/runtime selection uses small
IDs/enums/SocketAddr, with no per-query display/source/route strings when the
admission-time detailed-audit eligibility bit is off. Required entry outcome
metrics still update through a prebuilt ID-to-static-label registry, borrowing
configured labels without allocating per-query identity strings. When capture
is eligible, materialize display identities only at terminal detailed-record
construction; preserve existing terminal-time runtime capturing sampling.

## Endpoint, resolution and TLS contract

Accept bare addresses as UDP, and udp/tcp/tls/https addresses with default
ports 53/53/853/443. HTTPS path/query and authority come from the original URL;
numeric `dial_addr` changes network destination only. Bootstrap is a numeric
UDP peer with optional port 53, inherited globally unless overridden per entry.
In the approved host policy omitted bootstrap_version and explicit 0
use existing A+AAAA collection/A preference, 4 is A-only and 6 is AAAA-only.
Map omission explicitly in the compiler; preserve the existing resolver core's
None-to-IPv4 contract and archived decisions. Per-entry explicit zero must not
be lost by global-default inheritance.
No system resolver, cross-family connect race or connect-failure family retry.
Numeric targets bypass lookup. Hostname targets without numeric dial override
or explicit bootstrap fail clearly at load time.

Use existing resolver TTL clamping, single-flight publication, expiry and
failure retention contracts. Expired diagnostic state is not fresh success.
Resolution and every transport/fallback leg share the admission deadline;
per-entry upstream_query_timeout (milliseconds; default 5000, zero default,
negative rejected) can only shorten it. Bootstrap traffic is separate from
target attempts and must not masquerade as selected_upstream.

For secure entries load Linux system trust roots once in the host layer and
construct `TlsPolicy::verified`; do not add implicit root discovery to core.
Empty/unusable trust fails before I/O. Tests inject a scoped root-store seam and
synthetic CA; no public peer, host trust mutation or new public CA-path setting.
`insecure_skip_verify: true` is explicit only, never a failure fallback. Preserve
SNI, chain/name/time checks, signatures and DoH status/media/body/ID constraints.
Root-loading dependency selection must respect the current MSRV, ring provider
and lockfile audit; dependency uncertainty is resolved before its implementation.

## Reuse, fallback and bounded additional work

UDP TC invokes the existing exactly-once TCP fallback to the same numeric
destination with unchanged wire and deadline. A malformed UDP reply, timeout or
non-TC response does not trigger fallback. Distinguish TC observation from a
selected final answer and report the real final supplying transport.

Reuse retains existing serial-per-connection/identity-separated keys and idle
bounds. A busy serial reuse lease must not silently serialize every host query
or make ordinary concurrent requests all fail: use a scoped one-shot exchange
on the same validated destination, only for a typed pre-send busy admission.
Expose that narrow busy distinction if the core currently collapses it into a
general runtime error. Never retry arbitrary Runtime/MaybeSent/Sent failures.
The stable per-entry owner owns/drains both pooled and one-shot exchanges.
No detached task, new wait queue, DNS ID rewriting or same-connection pipeline.

Limits here bound additional per-request fanout and retained idle owner state.
They are not a claim that the pre-existing host has a global traffic admission
or connection-capacity guarantee. A host-wide admission redesign remains out
of scope; exercise concurrent progress and record observed counts honestly.
Positive idle_timeout overrides and enable_pipeline=true remain explicitly
unsupported in this batch; omit/zero/false retain native foundation behavior.
Do not accept an option and silently ignore its required behavior.

## Audit and existing UI

Extend the owning exchange result/checkpoint seams minimally. Record actual
selected numeric destination, final supplying entry identity, configured
targets and per-leg terminal outcomes without leaking intermediate suppliers.
Single-entry existing final_upstream/selected_upstream projections stay stable;
flow_setter retains its configured-label precedence. Untagged multi-entry
diagnostic identity and collision checks are specified in forward-contracts.md.
Internal metrics use structured plugin/entry keys;
do not relax existing public identity collision behavior accidentally.

### Versioned public observation (P1-2; user-approved)

Add optional `upstream_diagnostics` to native v2 rich-log projections, with
`schema_version: 1`, optional factual selected supplier and ordered attempts.
The exact payload/omission/outcome/transport contract is frozen in
research/forward-contracts.md. Existing fields, routes and pagination remain;
configured final_upstream cannot overwrite factual selected.entry. No Go
response mutation is required. The migration is explicitly approved; this
object is not an unversioned replacement of the old response shape.

Carry the same object through logs, exact-domain drill-down and slowest rows.
Update dashboard.ts, QueryManager details and Overview nested details to show
selected entry/peer/transport and ordered attempt outcomes for schema1. Missing
or unknown versions leave existing details usable and show unavailable new
diagnostics without fabricating facts. New fields do not expand existing search,
rank grouping or filter semantics. Same-peer distinct entries and flow_setter
overrides must be covered by HTTP and actual browser assertions.

Cache/local replacements remove any supplier inherited from earlier races.
TCP/UDP interrupted guards still terminalize once. Existing QueryManager details
and Overview drill-down consume these real fields; minimal field adaptation is
allowed if needed, but no upstream editor, synthetic catalog or metrics page.

## Validation and rollback

One reproducible mixed-forward config and controlled peers exercise all seams;
reuse existing deterministic resolver/reuse tests rather than a Cartesian
protocol matrix. Real listener and process tests prove wire/peer/audit agreement,
timeout/close/drain/rebind and concurrent progress. A disposable VM Vite/tunnel
browser pass confirms visible winner details and retained workflows.

All product checks run only on `ssh mosdns-rust` in owned isolated directories.
No service/default changes. Rollback stops owned fixture/native/Vite/tunnel
processes and restores only task paths; unrelated dirty changes stay untouched.
