# W3 source audit

Source: `0fb56189f04820c79d1cbb52fef6571aefdfe536` after verified W2 archive.
No implementation or remote/benchmark work during this planning session.

| Source | Finding / decision |
|---|---|
| `tests/phase5a-baseline/configs/routing.yaml` | Six plugins, one inline full domain, three tagged UDP forwards, four sequence rules, matcher list/scalar+list exec and exit. Exact config acceptance target. |
| `tests/phase5a-baseline/workloads/routing.jsonl` | Three A cases: domain-hit → .11, ip-hit → .11, ip-miss → .12; correctness requires route identity too. |
| `tests/phase5a-baseline/cmd/phase5a-baseline/main.go` | route-b returns 192.0.2.10 for ip-hit and .30 for ip-miss; route-a .11, route-c .12. Existing verifyRoutingCounters checks required/forbidden legs; new native tests additionally record per-query order. |
| `rust/native-host/src/config.rs` | One forward/executable and W1/W2 graph assumptions; extend to a validated catalog and strict W3 graph rather than hardcode names. |
| `rust/native-host/src/assembly.rs`, `udp.rs`, `tcp.rs` | One owned forward and shutdown close; multiple owners must be all closed even after an error. Keep one runtime and shared driver. |
| `rust/native-host/src/execution.rs` | Existing driver uses one forward, creates deadline at dispatch, resumes Continue after local synthesized SERVFAIL. W3 must stop errors and preserve one deadline or B failures could incorrectly fall through to C. |
| `rust/sequence-core/src/program.rs` | Matcher trait, typed MatchOutcome, multiple externals, RuleSpec and exec-list normalization already exist. Use these directly; no new engine. |
| `rust/sequence-core/src/engine.rs` | Canonical suspension/resumption plus Exit/inline frame semantics already exist; tests must cover real multi-forward composition. |
| `rust/matcher-core/src/full.rs`, `normalize.rs`, `ipnet.rs` | Safe FullMatcher and IpPrefixList APIs; case/trailing-dot normalization, rebuild-before-contains requirement. No native need for handles/ABI. |
| `plugin/matcher/qname/qname.go` | Query name matching; use native full-domain adapter for the frozen subset. Broader provider loading remains deferred. |
| `plugin/matcher/resp_ip/resp_ip.go` | Iterate Answer only, type A/AAAA, any matching address is true; no resolver CNAME reachability policy. |
| `plugin/executable/sequence/chain.go` | Execution errors stop the chain. Valid DNS negative responses are not transport/execution errors. |
| `rust/dns-core/src/response.rs` | Existing checked RR walker and decoded question metadata can support a narrow section-aware address observer. Current visitor exposes type/TTL only; avoid a separate parser. |
| W2 `research/closure-review.md` in archive | Final PASS confirmed, 220 local tests+lint/fmt repeated, historical input digests unchanged. Prior Linux /tmp tmpfs issue informs disk-backed build target. |
| `docs/ai/rust-rewrite-plan.md` Phase 5A | Real matcher/cache/forward, basic observability and native comparative evidence are separate obligations. W3 does not finish the phase or full product. |

Planning choices are restricted integration choices, not a decision to remove
full MosDNS features. IPv4-literal-only config, full-domain-only provider and
no cache+W3 graph keep this task small; complete grammar/composition belongs to
subsequent 5B work. Existing W1/W2 query behavior is preserved.
