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
