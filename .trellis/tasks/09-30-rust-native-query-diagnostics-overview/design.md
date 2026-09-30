# Design — query diagnostics and overview

## Ownership and reuse

Retain the existing native host and top-level DNS/HTTP supervisor. Extend
`native-host` execution/checkpoint/observer/API types; reuse sequence-core's
owned RoutingState and matcher/engine execution boundaries. Add the minimum
DNS read-only answer projection in `dns-core` using its validated wire walk,
compression guards and response metadata. Keep current cache/upstream owners.
Do not create a second collector, API server, Go bridge or UI rewrite.

Affected code: `rust/native-host/src/{config,matchers,managed,execution,
observer,api,udp,tcp,assembly}.rs` as required by the actual call chain;
`rust/sequence-core/src/{engine,program,state}.rs` only for verified
provenance boundaries; `rust/dns-core/src/{response,lib}.rs`; existing
`webui-log/src/components/{QueryManager,OverviewManager}.vue` and shared
helpers if needed; `rust/native-host/Cargo.toml` for direct getrandom facility.
Touch only necessary owning files, not every listed file.

## Data flow

1. Admission allocates request ID and existing question/client/time fields.
2. Compiler retains source identity for named provider references and inline
   rules. Evaluation returns matching evidence from the same published rule
   generation that actually matched; do not look up a later generation after
   a hot rule update. Current QnameMatcher discards identity and source.
3. The sequence driver distinguishes a matcher candidate, a completely matched
   rule with executable, and a committed routing/response outcome. Negation,
   failed later matcher, no-exec rules, child return, parent continuation and
   response replacement must have characterization tests. Prefer a small
   provenance object at the existing execution seam; no generic tracing bus.
   Preserve public DNS/control-flow/fast_mark behavior while preventing
   intermediate metadata from being advertised as final effective routing.
4. At the existing UDP/TCP final response_wire -> capture_execution seam,
   decode once outside the observer lock, after all transforms. Attach immutable
   projection to terminal facts; retain no duplicate wire. Safe header facts
   are distinct from validated extended-code/answer projection; decode failure
   emits the frozen all-or-none diagnostic without changing wire behavior. Preserve
   no-response and Drop/cancel checkpoints without copying an earlier answer.
5. Eligible terminalization commits one immutable typed AuditRecord, sampling
   runtime capturing as before. Carry owned/shared detail types through both
   listeners and the interrupted-execution path. Request ID lifetime and
   capture eligibility do not rely on DNS packet transaction ID.
6. HTTP parses a typed filter, obtains a coherent read view, then evaluates,
   selects and serializes off the DNS execution thread and outside store lock.
7. Existing Vue displays/filter/drill-down use that same JSON projection.

## Provenance and final labels

Use configuration-owned source descriptors (tag, YAML rule position, source
kind/path as established) at provider-generation or inline-YAML rule precision. Exact provider-line
evidence is out of scope; do not change matching order to manufacture it. The compiler/provider must
preserve rule-generation ownership. Candidates must not mutate RoutingState;
only fully successful rules with an executable commit provenance, then child/
parent final response ownership selects the surviving effective decision. Label semantics come from product behavior
in Go `computeEffectiveTag` and source normalization helpers, not accidental
Go storage/worker internals. Read their full pure-label logic and existing tests
before implementation; implement pure Rust equivalents with table cases for
all special/ban/direct/proxy/memory/fakeip precedence. This does not implement
those missing query plugins. Do not infer a specialized label from an arbitrary
upstream tag. Default unmatched_rule is permitted only for a genuinely
unmatched path, not missing instrumentation.

Keep configured flow_setter fields separate from factual supplier identity.
`final_upstream` uses existing configured-over-host precedence;
`selected_upstream` is the actual numeric endpoint that supplied final wire;
`upstream_targets` lists the configured target(s) for that final supplying leg.
Cache/local response has no selected network supplier. Historical attempt IDs
remain native diagnostic facts, not selected_upstream. Source paths rendered
must be safe config provenance, never secrets.

## Read model and scheduling

Retain rich records under immutable shared ownership. Expensive reads acquire
one of two host read slots with no unbounded queue, then run a blocking job
that holds the permit until it actually completes. In that job take a vector
of cheap record handles under the store lock, release it, and filter/top-k/
encode outside the lock. Only selected records become JSON. Cancellation is
checked at bounded scan intervals and before serialization. Excess work returns
the frozen 503; HTTP disconnect must not release the slot before job exit.
Slowest shares the same immutable records. Tiny control/stats paths stay direct.

One read's snapshot is coherent. Old snapshots during clear may complete with
old data, but new reads begin after the new generation. Do not speculatively
implement versioned retries or a general indexing framework. Measure snapshot
lock time and DNS progress at 400000; repair the specific seam if it fails,
then document/review material changes.

Retain the existing count cap. Variable detail bytes need a measured finite
worst-case bound, recorded peak memory and allocation failure policy; no silent
answer truncation or second hidden eviction policy. If the proposed projection
cannot fit reasonable resources at configured capacity, present a concrete
byte-limit/truncation tradeoff for user approval instead of silently lowering
capacity. The user approved preserving the 400000-record cap and complete
answers: detailed-retention allocation failure drops only the detailed record
projection after terminalization/lifetime metrics are kept, while diagnostic
read allocation/encoding failure returns HTTP 500. Neither path may affect DNS
service behavior or statistics. Do not retain raw response wire alongside
decoded answers.

## Compatibility, failure and rollback

`research/contracts.md` is the source for wire schemas/filters/limits and
proposed deviations. Existing stats/windows/v1 control/settings remain frozen.
Full protocol/plugin compatibility is not claimed. Unknown API remains explicit
404; wrong methods 405. Client aliases/capture/upstream sections may fail as
currently unsupported, with truthful UI errors. Do not change production Go
semantics or weaken supported API errors to hide native capability gaps.
Rollback is stop isolated native/Vite/tunnel processes and restore only this
task's exact files, preserving unrelated work. No active-service changes.

## Review rhythm

One consolidated planning discussion, one final whole-task implementation
review. During work run behavior-specific tests; after final code run full
VM regression and integrated browser proof. Add intermediate review only for
material product-contract/scope changes or actual high-risk findings.
