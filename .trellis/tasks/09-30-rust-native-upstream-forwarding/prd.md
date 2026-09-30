# Rust-native upstream forwarding workflow

## Goal and user value

Deliver one substantial native-forwarding workflow: a YAML-configured upstream
set can resolve its destinations, exchange over UDP/TCP/DoT/DoH, select a useful
response, reuse connections and show the actual final supplier in existing query
diagnostics. Eight related capabilities share the current host and one integrated
acceptance; do not split each protocol or parameter into a separate task.

## Background and authorization

- Workspace `/Users/tom/github/mosdns-rust`, branch `rust`; source baseline
  `860c6253f6f912b1cf99958148101f02a5e5821e`. Preserve unrelated dirty files and
  prior archive/journal changes; Trellis auto-commit remains disabled.
- Native forward currently accepts one numeric UDP/TCP endpoint. Secure,
  bootstrap/dual-stack, fallback and serial reuse libraries already exist.
  Source anchors and original MosDNS discovery evidence are in
  `research/source-contracts.md`; parameter decisions are in
  `research/forward-contracts.md`.
- Local-rule editing, audit control and query diagnostics are delivered bounded
  subitems. Reuse them; neither whole 5B/5C nor production cutover is complete.
- The user authorized task creation/planning and explicitly approved the new
  host family default on 2026-09-30: omitted/0 => dual lookup, IPv4 preferred;
  4/6 => forced single family. This supersedes the proposed A-only host default;
  preserve the archived/core history and implement the mapping at the host edge.
- The user subsequently approved a versioned optional `upstream_diagnostics`
  object and real detail display on 2026-09-30. The supplied planning feedback
  and five source-verified corrections are recorded in research/planning-review.md.
- This turn finishes planning only. Latest complete-summary approval in a later
  message is required for implementation. No product edits, build/test, start,
  commit, push, deployment or archive are authorized by planning approval alone.

## Requirements

### R1 — Multi-upstream configuration and invocation

Accept ordered multi-entry forward declarations, concurrent count, named
`$forward` invocation with optional entry-tag subset, and quick `forward addr...`.
Resolve all syntax/reference errors before network I/O. Preserve existing
single-entry accepted YAML and identity behavior. Default/normalize concurrent
as defined in contracts; fanout uses at most three distinct entries and never
sends duplicate work merely to fill a configured count. Unsupported options
must fail explicitly rather than be ignored.
Compile each call site to a host-owned invocation descriptor containing parent
definition and ordered original entry indices, dispatched through the existing
copyable external ID. Do not add a generic parameter payload to sequence-core.

### R2 — Endpoints and authenticated secure forwarding

Connect bare/default-port UDP, TCP, TLS and HTTPS addresses to existing owners.
Keep configured secure service identity, SNI and HTTP authority/path/query
separate from numeric network destination. Verified Linux trust is default;
explicit insecure_skip_verify is supported with no automatic downgrade.
Certificate, correlation, framing and DoH response checks remain intact.
This is upstream integration, not new encrypted DNS server listeners.

### R3 — Native hostname resolution

Numeric destinations bypass lookup. Hostname destinations use explicit numeric
bootstrap or numeric dial override, with global/per-entry inheritance and
approved omitted/0 dual, 4 A-only, 6 AAAA-only semantics. Preserve current native
TTL freshness, single-flight publication and typed cancellation/close behavior.
Do not serve an expired publication as fresh success. A valid AAAA-only result
can be selected, but this batch does not provide connect-family fallback when
both families resolve and the preferred IPv4 connection fails.

### R4 — Useful response selection and protocol fallback

Preserve product response priority: first completed valid Answer A/AAAA reply;
otherwise first completed NOERROR/NXDOMAIN, then other valid reply, then typed
failure, with the latter classes decided after all chosen legs finish. Invalid
replies never win. UDP TC triggers exactly one same-destination TCP exchange;
malformed replies/timeouts/non-TC do not trigger it. No general retry or
standalone sequence fallback/dual_selector implementation.

### R5 — One budget and scoped lifecycle

Resolution, each target exchange and TC fallback share the original admission
budget; per-entry timeout only shortens it. On winner, cancellation, deadline
or host close, cancel and join remaining owned work. No detached Background
legs, hidden runtime, post-terminal winner replacement or late publication.
Selected response identity must survive cleanup; listener shutdown drains the
complete transport/resolver catalog and frees owned sockets before rebind.
Register every started entry in a checkpoint-visible ledger before I/O and
terminalize its slot exactly once. Normal cancellation drains all legs; abnormal
future drop seals unfinished facts via guards and leaves asynchronous resource
drain to the owning catalog/supervisor, which must await it before rebind.

### R6 — Reuse without defeating concurrent requests

Integrate serial TCP/DoT/DoH reuse with the existing identity/trust/protocol keys,
idle bounds, original IDs and pre-send-only replacement rules. A typed busy
reuse admission may use a scoped fresh exchange on the same destination;
arbitrary runtime/Sent failures are not retried. No same-connection pipeline,
ID rewrite or new wait queue. Bound added fanout/retained owner history without
claiming a new global host traffic-capacity guarantee.

### R7 — Final supplier diagnostics and existing Vue closure

Project causal selected entry/peer/transport and every started entry's terminal
facts into native checkpoints/records and the user-approved optional versioned
v2 `upstream_diagnostics` object, frozen in forward-contracts.md. Preserve flow_setter configured
label precedence, single-entry compatibility and final-wire answer projection.
Cache/local/parent response replacements must remove superseded suppliers.
Existing QueryManager detail and Overview nested details expose factual selected
entry/peer/target transport and ordered attempts for schema1; absent or unknown
versions retain usable old details without inventing new facts. Attempt slots
stay in start order, while winner selection independently follows completion
priority. No audit-only display/source strings are allocated on audit-disabled
requests; per-entry basic metrics use the same ID-based terminal facts.
Do not add a synthetic upstream catalog, metrics dashboard or management success.

### R8 — Integrated correctness, resource and regression proof

All builds/product tests run only via SSH alias `mosdns-rust`, in owned isolated
source directories with controlled peers and synthetic TLS fixtures. Use real
YAML/listeners/DNS/HTTP and a disposable VM Vite/browser tunnel for final proof.
Preserve exact source/config/commands, original failures and owned cleanup
records. Related focused tests during work and one final complete regression
suffice unless changes/failures justify repeats; no full benchmark ceremony.

## Acceptance criteria

| ID | Observable outcome | Requirements |
| --- | --- | --- |
| A1 | Config/quick/subset tests prove defaults, inheritance, numeric/IPv6/default ports, distinct fanout, tag errors and unsupported fields before I/O. Descriptors preserve original entry indices, share definition owners, keep sequence-core ID-only dispatch, and preserve old single-entry configs/introspection. | R1–R3 |
| A2 | Controlled mixed peers prove every response-priority class, deterministic barrier ordering, invalid response rejection, exact winning wire and no unstarted-leg traffic. | R4–R5 |
| A3 | Real UDP TC produces one TCP leg with original question/ID/deadline; ordinary UDP and failure variants do not cause extra legs. | R4–R5 |
| A4 | Host bootstrap tests cover omitted/0/4/6, AAAA-only success, numeric bypass, publication/expiry/refresh failure, shared lookup and cancellation without OS/public DNS. | R3–R5 |
| A5 | Real DoT and DoH H1/H2 prove authenticated hostname identity vs numeric dial, CA/name/time rejection, explicit insecure path and unchanged wire/HTTP response bounds. | R2 |
| A6 | Peer accept counts prove sequential reuse; concurrent requests progress through busy admission without multiplexing; expired/half-closed owners and NotSent/Sent failures follow contract. | R6 |
| A7 | Real UDP/TCP queries and v2 logs/domain/slowest agree on schema1 selected entry/peer/transport, ordered terminal attempts and final answers, including same-peer distinct entries and flow_setter override, loser cancellation, cache/local/parent replacement. Browser details show the same facts; absent/unknown versions and Go remain usable. Audit-off retains correct per-entry metrics without audit-only strings. | R5–R7 |
| A8 | Caller cancel/deadline/owner close and deliberately dropped multi-leg futures prove checkpoint-visible exactly-once outcomes independently of successful exchange return. After asynchronous parent drain, no owned pending work remains and sockets rebind; retained owners do not grow with historical generations. | R5–R6 |
| A9 | VM focused/full Rust fmt/clippy/tests/native build and disposable Vue build pass, integrated evidence is retained, coverage subitems updated and one exact whole-task independent review is PASS. | R8 |

## Out of scope and limits

QUIC/H3/DoQ, proxy/socket policy, pipeline, positive idle_timeout customization,
system hostname resolver, cross-family connection retry/racing, new listeners,
upstream editing/hot reload/Prometheus dashboards, full configuration-package
compatibility, full 5B/5C/5D, hybrid retirement and production changes are deferred.
Unsupported YAML stays explicit. Partial native integration is not a complete
forward/plugin or platform acceptance claim. Dual lookup may wait longer for a
family; inherited core publication/order rules remain visible in evidence.

## Planning status

PRD/design/implement, source/contracts research and execution prompt are ready
for final user review. Family default and versioned public diagnostics are
approved decisions; the revised full scope
and listed native deviations are submitted for latest-summary approval. This
is one integrated task, not a parent plus protocol children: all delivery shares
one owner/result/diagnostic contract and final acceptance. Technical dependency
selection for host root loading is an implementation preflight, not a new
product decision. Dedicated reviewer binding and genuine automation snapshot
must be verified before implementation activation; no reviewer has been contacted.
