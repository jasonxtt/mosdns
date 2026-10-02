# Native response policy compilation

## 1. Scope / Trigger

S1 of `rust-native-response-policy-ip-rules` compiles immutable startup policy descriptors. Runtime hosts/TTL, scoped redirect and expanded IP execution follow in S2–S4. Do not serve descriptors that the request driver cannot execute: assembly currently rejects incomplete policies before listener I/O.

## 2. Signatures

`compile_yaml` / `compile_yaml_with_base` produce `CompiledConfig.response_policies`, `ip_sets` and `response_ip_rules`. `DomainPayload<T>::lookup(&str)` selects one immutable payload; `TtlPolicy` is `Fixed(u32)` or `Range { min: u32, max: u32 }`. Hosts preserve separate IPv4 and IPv6 ordered address vectors.

## 3. Contracts

Named `hosts` accepts entries/files; `redirect` rules/files; `ip_set` ips/files and empty sets only. Missing/null lists are empty. Quick `ttl` accepts exactly one unsigned decimal uint32 or min-max pair; zero and inverted bounds compile. Expanded `resp_ip` combines literal/CIDR, `$ip_set` and `&text-file` by OR. Relative files resolve against the declaring YAML/include directory. Domain payload defaults to full match; existing core full/suffix/regexp/keyword precedence applies. Duplicate patterns replace payload without changing first registration order. Files follow inline rules and may replace their payloads. IPv4-mapped hosts literals remain in the IPv6 family.

## 4. Validation & Error Matrix

| Condition | Result |
|---|---|
| Missing IP file | warning, skip |
| Missing hosts/redirect file | startup error |
| Bad UTF-8, I/O, unknown fields or rule syntax | error with declaration/line location |
| Nonempty provider sets, SRS/binary/compressed text | unsupported error |
| Line over 64 KiB, aggregate source over 64 MiB, over 1,000,000 noncomment rules | startup error |
| Blank/comment-only file | valid empty snapshot |
| Bad overwritten rule | error; replacement cannot hide malformed input |

Source byte budgets include comments and repeated rules count toward the rule budget. Anonymous IP sources share the owning sequence's budget. Inline ip_set items must contain exactly one IP/CIDR; text IP files may include trailing fields. No request-time file reads or hot reload.

## 5. Good/Base/Bad Cases

Good: `resp_ip $networks 2001:db8::/32` compiles immutable OR snapshots. Base: empty hosts rules produce an empty payload matcher. Bad: `::1/129`, `ttl +1`, invalid redirect target, or nonempty sets fail at startup.

## 6. Tests Required

`native-host/tests/policy_config.rs` verifies public YAML, payload precedence/replacement, declaration paths, immutable snapshots, empty/missing/bad/binary files, all resource limits, numeric bounds and fail-closed assembly. Existing config negative tests must reject invalid IPv6 widths rather than valid newly supported IPv6 syntax. Run remote workspace clippy, full native-host tests and fmt. After rsync preserving source mtimes, touch changed sources before Cargo validation to avoid stale cached artifacts.

## 7. Wrong vs Correct

Wrong: compile a policy then silently execute the ordinary forward path, or reload files on each request. Correct: snapshot and validate at startup; refuse assembly until its driver supports the descriptor, then pass the immutable snapshot into the real native execution path.

# Hosts and TTL execution (S2)

## 1. Scope / Trigger

Native root, branch sequence and direct fallback target dispatch execute hosts and quick TTL policies. Redirect and expanded IP runtime remain gated.

## 2. Signatures

`apply_wire_policy(&ResponsePolicy, &mut ExecutionState, &[u8]) -> Result<bool, ExecutorError>` returns true only when hosts replaces the response. `clamp_response_ttls(&[u8], min: u32, max: u32) -> Result<Vec<u8>, ResponseError>` shares dns-core's bounded record walker.

## 3. Contracts

Hosts only answers IN A/AAAA, preserving configured order with TTL 10. Empty selected family emits NOERROR and one SOA at the current question: TTL 300, ns `fake-ns.mosdns.fake.root.`, mbox `fake-mbox.mosdns.fake.root.`, serial 2021110400, refresh 1800, retry 900, expire 604800, minimum 86400. No match, both families empty, non-IN or non-address types leave the entire response/generation unchanged. Successful hosts sets Local supplier and clears selected endpoint, preserving historical attempts. Policies Continue; only subsequent sequence rules may accept/reject or overwrite them.

TTL fixed zero is a validated no-op. Positive fixed TTL replaces all non-OPT record TTLs in Answer/Authority/Additional. Ranges apply nonzero minimum then nonzero maximum, including inverted ranges. OPT bytes remain unchanged; TTL preserves supplier identity. Wire edits are prepared on a copy and committed only on success.

## 4. Validation & Error Matrix

Malformed TTL wire returns executor error with unchanged state; root converts executor failure to the existing local SERVFAIL path, branch propagates typed failure. Too many hosts answers or a response exceeding 65535 bytes fails before committing. Assembly permits hosts/TTL but still rejects pending redirect/new IP evaluation.

## 5. Good/Base/Bad Cases

Good: hosts → ttl 42 → has_resp accept yields a Local TTL-42 answer. Base: empty hosts match preserves an earlier response. Bad: ttl applied to malformed response cannot publish a partially changed packet.

## 6. Tests Required

`policy_wire.rs` drives real UDP for dual-stack order, FakeSOA fields, no-op/Continue behavior, numeric TTL semantics, upstream→hosts supplier replacement, and direct fallback hosts→TTL. `policy::wire_tests` asserts response/generation preservation and malformed atomic failure. dns-core tests assert all-section range edits, unchanged OPT and malformed late-record failure. Regress full native-host and dns-core suites plus workspace all-target clippy/fmt remotely.

## 7. Wrong vs Correct

Wrong: treating hosts as implicit Accept or erasing all upstream attempts when it replaces a response. Correct: Continue through the canonical sequence machine; clear current selection and retain completed attempt history.
