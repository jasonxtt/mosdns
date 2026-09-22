# Design — native W3 routing

## Source-grounded approach

`research/source-audit.md` maps current source to decisions. W2 completion
left one compiled forward and one owned ForwardAdapter, but sequence-core
already supports matcher seams, multiple external IDs, inline exec lists and
Exit. Generalize native-host ownership/compilation; do not redesign sequence.

## Native matchers and DNS observation

Build one immutable `matcher-core::FullMatcher<()>` from the single reviewed
ASCII full-domain expression. Host qname matcher implements sequence `Matcher`
and reads the existing decoded question. Use an unambiguous wire-label to
ASCII-domain conversion: only plain labels representable in the supported
rule grammar may be joined. Embedded dot/backslash, non-ASCII bytes or other
unsupported label bytes must yield a non-match, never lossy UTF-8/replacement
or reinterpretation as separators. Exact full matching uses matcher-core's
case normalization and one trailing-dot normalization; no wildcard or suffix.
Invalid configured plain domains are rejected before matcher construction.

Build a matcher-core IpPrefixList for the single literal IPv4 /32 rule and
call rebuild before sharing it. The resp_ip adapter inspects only current Raw
response Answer A/AAAA addresses; None/Synthesized and valid empty answers
are false. A valid raw negative response is processed by the same Answer rule.
It returns MatchOutcome without mutating state. `_true` is a constant true
matcher through the same seam. No Go registry or runtime ABI dependency.

dns-core currently has a private record walker with RR type/TTL but no public
Answer-address visitor. Add a narrow checked observation helper using the
same parser machinery, carrying section, RDATA offset/length as needed. Walk
the whole message sufficiently to detect truncated trailing records; bound
all ranges and check A=4 bytes and AAAA=16 bytes. Do not copy resolver-specific
CNAME reachability or its family selection semantics: Go resp_ip scans all
A/AAAA answers. Test compressed owners, unrelated owners/CNAME, multiple
addresses, authority/additional exclusion, OPT, short RDATA and truncation.
Existing TTL/metadata/ABI behavior must remain unchanged. No new external
parser or public general DNS builder is needed.

## Compiler and executable catalog

Extend strict raw-YAML compilation to recognize the W3 graph as a third
explicit alternative to W1/W2. Parse the exact four-rule topology, resolving
all symbols by tag independent of declaration order. Infer A/B/C identities
from the graph, not literal fixture tags or address/answer values. Require
three distinct forward plugin references, the same A reference in domain/IP
hit branches, one B intermediate, one C final, and terminal `exit` in all
selected final branches. Reject other graphs before constructing host resources.

The W3 forward upstream object additionally accepts its explicit nonempty
`tag`; retain that identity alongside plugin tag/endpoint. Reject duplicate
upstream identities in the W3 catalog. Tags identify routes but this task does
not claim full audit field/API integration. Fixed test counter labels are only
fixture expectations, never production routing inputs.

Compile conditions with MatcherSpecInput and build rules with RuleSpec.
Compile list executors `[External(A), Exit]` using existing ProgramSpec list
normalization. A suspended forward inside an inline list must resume that
same machine and then execute Exit, ending all later rules. Do not replace
Exit with an ad-hoc host branch or a separately interpreted list. Unit tests
must assert this nested-list suspension/resumption and no unwanted dispatch.

Replace the single-forward ownership assumption with an immutable mapping
ExecutableId -> compiled forward identity / Rc<ForwardAdapter>. W1/W2 remain
one-entry catalogs. Keep a single source of truth: if temporary convenience
accessors remain for existing tests they resolve into the same catalog, not a
parallel owner. Per-request dispatch looks up only a validated executable ID.
Unknown IDs return a typed execution error; never silently choose the first
upstream. Add only native-host's matcher-core path dependency and corresponding
Cargo.lock edge; do not alter external versions/features or workspace members.

## Request driver, errors and lifecycle

The shared native-host driver owns canonical state, response and cancellation.
Capture one deadline per admitted W3 request (before executing any leg), pass
the same Instant to every exchange and stop before dispatch if expired or
cancelled. Existing single-forward W1/W2 retain their established behavior;
changing their cache TTL publication rules is out of scope. Do not reset W3's
time budget after B. Tests may use a bounded mock exchange recording deadline
identity, plus one real stalled-leg integration; avoid timing-only assertions.

For W3, accepted exchanges must pass structure and matching-question gates
before placing a raw response in state or evaluating resp_ip. A malformed or
wrong-question response is a terminal local failure; clear/replace earlier
B state with synthesized SERVFAIL and stop, without a C fallback. Upstream
NXDOMAIN/SERVFAIL are valid DNS responses if structurally associated; an empty
B Answer then follows the ordinary miss branch to C. If A/C exchanges fail,
return SERVFAIL, never B's earlier result. Preserve query ID/question in local
errors. Cancellation uses the existing no-send path; no late downstream leg.
W1/W2's current forwarding and cache gates are regression contracts, not an
excuse to run W3's `_true` after a local transport failure.

No W3 cache is created as a plugin and no cache token is armed for W3. Existing
host cache ownership may remain for W1/W2 compatibility; do not perform an
unrelated allocation/lifecycle optimization in this task.

UDP/TCP listeners receive shared catalog handles and call the one driver.
UDP serves W3; TCP changes are ownership plumbing for existing W1 only.
Supervisor teardown must close all catalog owners even if an earlier close
fails; retain/request evidence for the primary error rather than early-return
and leak the rest. Cancel/join all tasks and retain W1/W2 rebind guarantees.
No test observer runs as detached production work.

## Validation and compatibility limits

New `w3_routing.rs` uses three independent loopback upstreams with per-query
counters and an ordered event log. Read/check all immutable routing.jsonl rows
and parse exact routing.yaml in config tests; network tests substitute only
ephemeral numeric loopback ports and test deadlines. Record event order per
query ID/name so interleaved clients cannot corrupt the oracle. Tests must
check final answer/question/ID/RCODE plus all route counts and forbidden legs.
Tampered route evidence must be rejected independently of final response.

Linux correctness, once authorized, uses `ssh mosdns-rust` and a fresh isolated
temporary directory, disk-backed build target with space check (W2 exposed a
small /tmp tmpfs), exact source SHA and full commands. Never invoke the frozen
baseline runner or mutate its inputs/evidence. Final records distinguish
locally repeated checks from remote results and reviewer conclusions. A final
PASS ends the authorized range; finish/archive remains a user decision.
