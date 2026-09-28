# Rust-native fast_mark and flow_setter sequence integration

## Goal

Extend the Rust-native host's supported YAML/sequence subset with the first
representative `fast_mark` matcher/executable and `flow_setter` routing
metadata path, so a real native query can set/observe flags, take the
configured branch, and retain final routing metadata across asynchronous
execution.

This is a new 5B implementation child of
`09-28-rust-next-step-roadmap`. Its parent plan must receive same-chat C2C
`FINAL: PASS` first. It is ordered after the existing fixed-candidate
`09-28-rust-mos-test-native-sidecar-canary`. It may start without the remote
canary result only if the user explicitly defers the canary and authorizes 5B
to proceed in that case. It does not modify or close the canary task.

## Product behavior requirements

- Preserve the Go-observable `fast_mark` contract used as product evidence:
  matcher form OR-matches configured IDs; executable form sets each configured
  bit without clearing unrelated flags; accepted IDs are 0–63; each new DNS
  query starts with no flags from a prior query.
- Keep `fast_mark` 48 and `switch17` bit 49 reservations exactly as documented
  in `docs/ai/config-notes.md`. This task does not implement switch plugins.
- Support the native sequence compiler's documented quick-setup and normal
  configuration/reference forms where each applies. Reject malformed IDs,
  unknown keys, unresolved references, cross-type references, and unsupported
  forms during config compilation, with useful path context. Never accept a
  configured no-op.
- Preserve the Go-observable `flow_setter` configuration contract: quick keys
  `group`, `sequence`, and `upstream` map to `matched_group`, `final_sequence`,
  and `final_upstream`; normal args preserve the same values.
- Freeze the precedence of configured values against native host-derived
  terminal sequence/upstream values before product code changes. The decision
  must cite source/tests and be recorded in this task's design plus behavior
  tests.
- Preserve configured metadata through actual async suspension/resume and make
  all three values visible in the native observer record for the completed
  request. Do not expand scope into a full audit HTTP/API migration.
- Existing representative chain behavior remains unchanged when neither
  plugin appears, including direct child sequence, cache, forwarding, and
  query isolation.

## Observable behavior slices

1. **Compiler contract:** public YAML input with valid quick setup and normal
   args compiles to the expected typed sequence program; malformed or unknown
   configuration returns a path-aware error before listener/upstream startup.
2. **Flags and branch:** a real native YAML sequence executes an ID-setting
   operation and a matcher-controlled branch produces distinct controlled-peer
   results; matcher IDs OR together, setting preserves unrelated bits, and a
   second DNS request starts with fresh flags.
3. **Routing metadata:** a configured flow setter runs before a delayed
   controlled forward. The request resumes with all configured routing values,
   and the native observer reports them according to the frozen precedence.
4. **Regression boundary:** the existing 5B composition integration suite
   passes unchanged with no `fast_mark`/`flow_setter` configuration.

## Scope

Expected primary ownership is `rust/native-host/src/config.rs`,
`execution.rs`, `observer.rs`, a focused native-host integration test, and only
the minimal `rust/sequence-core/src/state.rs` or program changes proven
necessary. Follow the current architecture and specs; this list is a starting
point, not permission for unrelated refactoring.

## Out of scope

- `switch1..17`, `special_groups`, or any Go/cgo adapter, selector, mirror, or
  fallback.
- Full sequence plugin/quick-setup support, full P11/P33/P44, generic dynamic
  plugin registration, or all 5B configuration compatibility.
- Full audit API/HTTP endpoints, Prometheus, WebUI, persistent formats, or
  production/default cutover.
- Runtime/`Send` redesign, performance tuning/benchmark, multi-core,
  capacity, soak, or reliability claims.

## Acceptance Criteria

- [ ] Parent roadmap has same-chat C2C `FINAL: PASS`, and canary ordering is
      satisfied or explicitly waived by the user as described above.
- [ ] Valid quick-setup and normal configuration paths are exercised through
      real YAML compilation; invalid paths fail before I/O with path context.
- [ ] Native listener integration proves matcher OR behavior, executable set
      behavior, retained unrelated flags, cross-request isolation, and a
      distinct matcher-selected branch.
- [ ] Async integration proves all three flow values survive suspension/resume
      and appear in observer output with the tested precedence.
- [ ] Existing 5B composition tests pass unchanged; `cargo fmt --all -- --check`,
      workspace clippy with warnings denied, focused native-host tests, and the
      full Rust workspace tests pass.
- [ ] `docs/rust/feature-coverage.md` is updated only for delivered subitems and
      evidence. No broad P11, P33, P44, switch, or 5B row is marked complete.
- [ ] Same C2C conversation reviews the exact committed task range and returns
      `FINAL: PASS`; all findings are resolved and reviewed in that same chat.
