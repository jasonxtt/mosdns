# Rust Phase 5A native routing

Status: planning; reviewer returned PLANNING: PASS for
`7d684c5dee67afeb1eea298813c25b632a6f8438`. User requested the next
task's plan after W2 closure; final implementation approval is pending.
No W3 implementation, automation authorization, task start or dispatch is
currently authorized. Source anchor: `0fb56189f04820c79d1cbb52fef6571aefdfe536`.

## Goal and value

Integrate real domain/response-IP matching and multiple upstreams into the
existing Rust-native host. Execute the immutable W3 YAML/corpus through the
canonical sequence engine while preserving W1 UDP/TCP and W2 cache. This
closes the missing minimal routing link in Phase 5A; it does not complete
Phase 5A, all routing features, or the performance/stability gates.

## Requirements

R1. Accept `tests/phase5a-baseline/configs/routing.yaml` unchanged, including
upstream tags, matcher strings, scalar/list exec and `exit`. Support only its
six-plugin graph: one inline domain_set, three single-UDP-upstream forwards,
one four-rule sequence and one UDP listener with audit false. Names, numeric
endpoints, the full-domain value and single IPv4 rule are configuration data,
not hardcoded fixture answers. Preserve existing W1/W2 accepted graphs.

R2. Execute the following real routes for every immutable W3 corpus row:

| Case | Route in order | Forbidden legs | Final answer |
|---|---|---|---|
| DOMAIN_HIT | A | B, C | 192.0.2.11 |
| IP_RULE_HIT | B then A | C | 192.0.2.11 |
| IP_RULE_MISS | B then C | A | 192.0.2.12 |

The first matcher is qname against the inline `full:` domain_set. If it misses,
B's response is inspected; `resp_ip` matches ordinary A/AAAA addresses in the
Answer section only. Authority/additional addresses must not affect the route.
The final `_true` rule and `exit` use existing sequence semantics. Return the
last successfully selected response, with the current query ID and original
question; intermediate B must never be emitted to the client before A/C.

R3. Use safe native matcher-core APIs and `sequence-core::Matcher`, external
executable IDs and the existing machine. No fixture matcher in production,
Go/cgo/fallback/handle path, listener-level routing or alternate interpreter.
Compile immutable matchers before any I/O; the host owns all upstream owners.

R4. Freeze the narrow grammar: one `full:<ASCII-domain>` expression; matcher
forms `qname $<domain-set-tag>`, `resp_ip <IPv4-literal>`, `_true`; one matcher
per conditional rule; exact four-rule topology with symbolic A/B/C roles.
Permit case/trailing-dot normalization for full domains, not suffix matching.
Arbitrary regex/suffix/files/reload/CIDR/IPv6 configuration and other sequence
forms remain rejected. Preserve any W1/W2 query parsing behavior; malformed
DNS is not silently reinterpreted as a domain string.

R5. Strict compilation rejects unknown/duplicate fields, invalid rule syntax,
roles/counts, references, exec order, empty expressions, unsupported options,
cache+W3 composition, TCP W3, hostnames and invalid/zero ports before assembly
or I/O. Plugin declaration order, tag names and endpoint changes within the
supported numeric-UDP graph must work; tests must expose hardcoding.

R6. All W3 legs share one request deadline and cancellation scope. A failed
exchange or malformed/mismatched W3 response aborts routing and returns
associated SERVFAIL; no subsequent leg is attempted. Caller/server cancellation
returns no late response and starts no later leg. A valid negative DNS response
is not an exchange failure: an empty B Answer does not match resp_ip, so C runs.
A/C failure must not accidentally return the earlier B response. Existing W1
and W2 behavior, including cache qualification and response mapping, remains.

R7. W3 response qualification before matcher evaluation requires QR=response,
QUERY opcode, one matching decoded question (DNS case-insensitive name,
matching qtype/class), valid structure and valid A/AAAA RDATA lengths. Reuse
existing DNS helpers; no second decoder. Request-ID/source checks remain the
upstream adapter's responsibility. An address in a CNAME's RDATA is not an A
record; actual A/AAAA answers can match regardless of owner, as in Go resp_ip.

R8. Concurrent requests isolate matcher/response state, IDs and route legs.
Shutdown stops admission, cancels and joins requests, closes every owned
upstream even after a task failure, releases sockets and permits rebind. No
multithread-runtime change, connection policy redesign or unbounded task pool.

## Acceptance criteria

- [ ] A1 (R1/R4/R5): unchanged W1/W2/W3 YAML compile, alternate names/order/data
  prove generic binding, full negative-config matrix fails before I/O.
- [ ] A2 (R2/R3/R4/R7): native matcher tests cover exact/case/trailing-dot domain
  behavior; Answer-only IPv4/IPv6 extraction, compressed owner names, multiple
  answers/CNAME, empty/negative and malformed cases; no state mutation.
- [ ] A3 (R2/R3): every frozen W3 row returns its exact expected DNS result and
  exactly the required upstream increments, zero forbidden increments, with
  per-request order B→A/B→C verified. A deliberately wrong/missing leg fails
  the test oracle even when the final answer happens to be correct.
- [ ] A4 (R6): multi-leg deadline does not reset per forward; first/second-leg
  failure, malformed/mismatched response and cancellation barriers exercise
  the precise stopping rules with no stale response or extra leg.
- [ ] A5 (R8): mixed-route concurrent requests, distinct IDs, shutdown while B
  is outstanding and while A/C is outstanding, every-owner close and rebind pass.
- [ ] A6: local Rust checks and Linux amd64 W1/W2/W3 correctness pass against
  an exact commit. Historical baseline/corpus hashes remain unchanged; evidence
  records commands, failures/retries, counts, cleanup and actual limitations.
- [ ] A7: designated reviewer gives each slice and final explicit PASS, and
  coverage/handover report only the bounded W3 subset. No implied Phase 5A or
  full-plugin completion, performance gain or production readiness.

## Exclusions and deferred gates

No cache+routing combinations, provider file loading/reload, full matcher
language, special_groups, fallback/racing groups, new transports, API/WebUI,
audit/metrics delivery, performance run, profiling campaign, Go baseline rerun,
production/default change, or hybrid retirement. Basic observability and the
first comparable native process performance gate remain later Phase 5A work;
full query and management features stay in 5B/5C. No blocking product question
remains for this bounded plan; user approval of the final plan is still required
before implementation. Existing executor/reviewer selections may be reused
only when that implementation is authorized.
