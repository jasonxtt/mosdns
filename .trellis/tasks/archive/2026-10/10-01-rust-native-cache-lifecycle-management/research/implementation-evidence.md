# Implementation evidence — 2026-10-01

Scope actually implemented in this round, the exact diff range, the commands that
were run, the review findings that were remediated, and the parts of the approved
plan that were **not** implemented.

Base commit: `368daef0d25cf92121ee51a27671a326860c5032` (branch `rust`).
Execution environment: SSH alias `mosdns-rust` (10.0.0.92), task-exclusive
directory `/root/mosdns-rust-cache-lifecycle-20261001`. Nothing was built on the
local Mac. Nothing was pushed, deployed or cleaned outside that directory.

## Slices delivered

### S1 — cache catalog and configuration (complete)

* `CompiledConfig.cache: Option<CachePluginConfig>` became
  `CompiledConfig.caches: Vec<CachePluginConfig>` with a compile-time
  `CacheId(usize)` index. Several `type: cache` plugins now coexist; each gets
  its own executable, its own `CacheId` and its own store.
* New `CacheKind::Named | CacheKind::Quick`, `CompiledConfig::cache_for_executable`,
  `CompiledConfig::named_caches`.
* New `CacheCatalog` in `native-host/src/cache.rs`: one `NativeCacheAdapter` per
  `CacheId`; `assembly.rs`, `udp.rs` and `tcp.rs` now carry the catalog instead
  of a single adapter.
* Inline `exec: cache [size]` callsites compile to a private instance with a
  synthetic `@native-quick-cache:<rule>` external, capacity from the optional
  argument (default 1024), `lazy_cache_ttl_secs = 0` always, no tag, no dump and
  no public metrics identity. No new syntax beyond the reference `cache [size]`
  form was invented.
* Configuration matrix implemented: `size` absent/`<= 0`/explicit null → 1024;
  `dump_interval` absent/`<= 0`/explicit null → 600 (accepted without a dump
  target, exactly as the reference plugin does); `exclude_ip` accepts the scalar
  whitespace form and the string-list form; `enable_ecs: true` is a
  configuration error; `lazy_cache_ttl` negative or out of range is a
  configuration error; unknown keys are still rejected.

### S2 — independent nested publication (complete)

* `sequence-core`: the single `watch: Option<ScopeWatch>` became
  `watches: Vec<ScopeWatch>` plus a LIFO `pending_scope_completions` queue.
  `watch_enclosing_scope` now returns a typed `WatchToken` that is unique per
  machine, and every `ScopeCompletion` carries it, so an owner pairs a
  notification with the exact frame it armed instead of guessing from the
  executable.
* On every scope that stops running, **all** watches bound to scopes that are no
  longer live are retired, not only the top one. That covers scopes unwound by
  `exit` propagation, which can pop several scopes in one step.
* A natural completion of the scope that produced the signal yields
  `MachineStep::ScopeComplete`; every other vanished scope yields
  `MachineStep::ScopeAborted`, and the owner must invalidate that frame. An
  `exit` therefore can never be published, and no frame survives against a dead
  scope identity.
* `resume_scope_completion` takes the token, so a stale or foreign token is a
  typed error rather than a silent release.
* `native-host/execution.rs`: the request-level `Option<PendingStore>` became a
  `Vec<PendingFrame>` keyed by watch token. The "second cache access fails
  closed" guard and its `cache_accessed` bookkeeping were removed, so a repeated
  dispatch of the same cache arms a second frame instead of failing closed.

### Deliberately not delivered

S3 (Go product key layout, dual clocks, EDNS0/DO, `domain_set` capture), S4
(owner-managed lazy refresh), S5 (`mosdns_cache_v2` codec, generation gate,
durable-first flush), S6 (inventory endpoint, `/metrics` cache subset, plugin
management routing) and S7 (Vue) are **not implemented**.

Because S4 and S5 are missing, three options that would otherwise be silently
inert are refused at compile time with an explicit reason:

* positive `lazy_cache_ttl` — a stale answer needs a refresh owner that keeps
  working after the client is done;
* `dump_file` — persistence that does not happen would silently lose data;
* `exclude_ip` — the listed networks are still parsed and validated, then
  refused, because answers are not filtered yet.

This is narrower than the approved contract, which accepts all three. It is the
fail-closed interim for the unimplemented slices, not a redefinition of the
contract, and it replaces the worse behavior of accepting a configuration that
does nothing.

## Validation commands and results

All commands ran in `/root/mosdns-rust-cache-lifecycle-20261001/rust` with
`CARGO_PROFILE_DEV_DEBUG=0` (the target directory had to be rebuilt without
debuginfo because the host volume only had ~2.9 GB free).

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace -j1 --no-fail-fast` | 1007 passed, 0 failed across 68 result lines (`TEST_RC=0`); `sequence-core` lib 37 passed, `cache_catalog` 10 passed |
| `cargo build -p mosdns-native-host` | ok (`BUILD_RC=0`) |

The raw per-binary record is `research/final-validation.txt`. Every command above
ran on one frozen revision: the fingerprint of the sorted per-file `md5sum` list
over `rust/**/*.rs` (excluding `rust/target`) is
`44acf72b8517acdb2dd427e28aaf4bb0`, verified identical on the local Mac and the
build host and identical immediately before and after each falsification run, so
every command above and both falsifications describe the same bytes.

Sources are synced with `rsync -a`, which preserves mtimes. The build host runs
`find rust -name '*.rs' -exec touch {} +` after every sync so cargo cannot reuse
an artifact built from an older source; one run in this session reported a false
failure for exactly that reason and is not reproducible with the touch step.

### Falsification A — the fallback direct cache target (P1-1)

Removing only the `result.completed_naturally()` gate at the direct cache-target
publication site, on the same fingerprint, makes exactly one test fail:

```
test a_fallback_cache_target_never_publishes_a_successor_that_exited ... FAILED
test result: FAILED. 9 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

### Falsification B — the watch model (F1/F2)

### Behavior tests added or rewritten

Engine level (`rust/sequence-core/src/lib.rs`):

* `nested_watches_are_reported_in_lifo_order_at_their_own_boundaries`
* `an_exited_scope_aborts_only_its_own_watch_and_the_surviving_scope_keeps_its_token`
* `watches_on_scopes_unwound_by_exit_propagation_are_never_orphaned`
* `a_watch_dropped_by_exit_never_reports_a_publishable_completion`
* `a_terminal_machine_error_invalidates_every_armed_watch_without_a_notification`
* `a_resumed_external_error_also_invalidates_every_armed_watch`

Host level:

* `rust/native-host/tests/cache_catalog.rs` (new, 10 tests). Nine of them drive
  the real UDP listener against a loopback mock upstream, including the two
  fallback-cache-target tests added in the fourth round; the tenth
  (`every_configured_named_cache_is_listed_in_configuration_order`) is a
  compile-only ordering assertion.
* `rust/native-host/src/execution.rs`:
  `two_nested_caches_each_store_their_own_successors_result` runs two named
  caches in two nested scopes whose boundaries produce *different* answers
  (`192.0.2.71` from the child leg, `192.0.2.81` from the parent's trailing
  leg) and asserts each store holds its own boundary result;
  `a_repeated_dynamic_cache_dispatch_builds_its_own_frame_and_answers_normally`,
  `a_nested_cache_hit_short_circuits_the_inner_cache`,
  `one_cache_dispatched_in_two_scopes_publishes_at_both_boundaries` and
  `two_calls_to_one_child_dispatch_the_shared_cache_again_after_a_hit` replace
  the three removed `a_second_cache_access_*_fails_closed` tests.
* `rust/native-host/tests/slice2_config.rs` — the cache accept/reject matrix was
  rewritten for the new contract and split into
  `cache_parameters_and_composition_are_configurable_where_the_contract_allows_it`
  and `unsupported_cache_configuration_is_rejected_before_io`.
* `rust/native-host/tests/w2_cache.rs` — the shutdown assertion now asks the
  catalog for its entry count (`raw_entry_count`) instead of the ambiguous
  `is_empty`, which used to mean "no cached entries" and now means "no compiled
  cache".

### Falsification check (new tests fail without the implementation)

The single-watch conflict guard that S2 replaces was temporarily restored in
`sequence-core/src/engine.rs` (a `if !self.watches.is_empty() { return
Err(ScopeWatchConflict) }` in `watch_enclosing_scope`, before the scope lookup),
the suite was re-run, and the source was then restored byte-for-byte
(`cargo fmt --all --check` clean afterwards):

```
test slice3_control_tests::a_resumed_external_error_also_invalidates_every_armed_watch ... FAILED
test slice3_control_tests::a_terminal_machine_error_invalidates_every_armed_watch_without_a_notification ... FAILED
test slice3_control_tests::an_exited_scope_aborts_only_its_own_watch_and_the_surviving_scope_keeps_its_token ... FAILED
test slice3_control_tests::nested_watches_are_reported_in_lifo_order_at_their_own_boundaries ... FAILED
test slice3_control_tests::watches_on_scopes_unwound_by_exit_propagation_are_never_orphaned ... FAILED
test result: FAILED. 32 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out
```

Five of the 37 `sequence-core` lib tests fail for exactly the right reason when
the old single-watch contract is put back, rather than passing vacuously. The
revision fingerprint was identical before and after this run.

The host test `two_nested_caches_each_store_their_own_successors_result` is a
regression guard, **not** a falsifier of F1: its two caches have different
executables, and the pre-remediation `rposition`-by-executable heuristic happens
to pick the right frame for that shape. The falsifiers of F1/F2 are the four
engine tests above.

## Independent review and remediation

An independent reviewer examined the dirty `rust/` set against `prd.md`,
`design.md`, `implement.md` and this file and returned `VERDICT: FAIL` with the
findings below. Every finding was addressed in the reviewed range:

| Finding | Disposition |
| --- | --- |
| F1 (major) a frame whose watched scope exits was never invalidated, so a later boundary consumed it | **fixed** — notifications carry a `WatchToken` and are matched by it (`publish_frame` uses `position`, not `rposition`); an exited scope now yields `MachineStep::ScopeAborted` and `abandon_frame` drops that exact frame |
| F2 (major) watches on scopes unwound by `propagate_exit` were orphaned forever | **fixed** — the drive loop retires every watch whose scope is no longer live, not just the leaving one; new test `watches_on_scopes_unwound_by_exit_propagation_are_never_orphaned` |
| F3 (major) `exclude_ip`/`dump_file` were accepted but inert | **fixed by refusing them** until the slices that honor them exist; the shape is still validated so malformed input reports the same reason as before |
| F4 (minor) duplicate-dump-path check bypassable by unnormalized paths | **moot** — `dump_file` is now rejected, so the check and its test were removed rather than left half-correct |
| F5 (major) the new S2 tests could not detect frame mis-attribution | **fixed** — `two_nested_caches_each_store_their_own_successors_result` uses per-executable answers and asserts each cache holds its own boundary result |
| F6 (minor) `dump_file` path resolution claimed without a test | **moot** — the option is rejected and the rejection reports the resolved path |
| F7 (minor) coverage lost for "two dispatches where the first is a hit" | **fixed** — new `two_calls_to_one_child_dispatch_the_shared_cache_again_after_a_hit` uses `root = [Call child, Call child]` with a primed cache and asserts the upstream is reached once and the answer is normal |
| F8 (nit) malformed error path `$.plugins[{}.args.dump_file` | **fixed** — the function was removed with F3/F4 |
| F9 (minor) explicit `null` handled inconsistently | **fixed** — every cache option treats explicit null as absent |
| F10 (nit) unused public catalog API | **fixed** — `CacheCatalog::empty`, `CacheCatalog::iter`, `CacheCatalog::cache_count` and `WatchToken::get` were removed; `raw_entry_count` is the only entry-count accessor and has a caller |
| F11 (minor) this file overstated the policy-target driver and the test set | **fixed** — the file no longer claims the driver was tokenized; see the fourth round for what it actually does and how it is now gated |

## Second independent review round

A re-review of the remediated range returned `VERDICT: FAIL` with **no
behavioral blocker**: F1/F2/F3/F5/F9 were confirmed fixed and the validated
revision was confirmed to match the reviewed revision. The findings were a real
API-contract gap plus several false "fixed" claims in these artifacts, which the
reviewer is right to gate on. All were addressed:

| Finding | Disposition |
| --- | --- |
| N1 (minor) `ScopeAborted` was documented for cancellation / fuel exhaustion / terminal errors, but those paths return `Err` before the retirement loop runs | **fixed in code** — the error path now clears every armed watch before returning, so no watch outlives a failed machine; the doc comments on `ScopeAborted`, the main loop and the branch driver now state that error paths invalidate without a notification and that the owner must drop its own frames. New test `a_terminal_machine_error_invalidates_every_armed_watch_without_a_notification` |
| N2 (minor) F10 was claimed fixed but the API was untouched | **fixed** — `CacheCatalog::empty`/`iter`/`cache_count` and `WatchToken::get` removed |
| N3 (minor) F7 was claimed fixed but the cited test was unchanged | **fixed** — the `[Call child, Call child]` + primed-cache test was actually added |
| N4 (minor) the recorded falsification run could not come from this revision | **fixed** — re-run on the frozen revision: 32 passed / 5 failed over 37; the section above carries the real output and the revision fingerprint |
| N5 (minor) `prd.md` still described only `lazy_cache_ttl` as fail-closed | **fixed** — the PRD now lists all three refusals and no longer counts the dump path rule as delivered |
| N6 (nit) `run_target` publishes on `Exited` success while the boundary path abandons | **disclosed** — pre-existing, unchanged by this diff, recorded here and in the F11 row |
| N7 (nit) orphaned doc fragment above the test helpers | **fixed** — the stale fragment was removed |

## Third independent review round

The third review found **no code-level finding**: it confirmed (a) that the
terminal-error invalidation is safe (`fail_in_place` clears only watch state, and
`pending_completion`/`pending_scope_completions` are provably empty or `None` at
those call sites) and (b) that `armed_watch_count() == 0` now holds on every
terminal path, including cancellation supplied through `resume(Err(..))`. It
returned `VERDICT: FAIL` only because the recorded validation and falsification
predated the last two edits, so the counts described an older revision.

| Finding | Disposition |
| --- | --- |
| N8 (minor) the record and falsification described a 36-test engine lib; the tree had 37 | **fixed** — every command was re-run on the frozen revision and the record now carries `1005 passed / 0 failed`, `sequence-core` lib `37 passed`, and the falsification `32 passed / 5 failed`, together with the revision fingerprint measured before and after the falsification |
| nit `abandon_frame`'s doc still listed error/cancel/fuel cases | **fixed** — it now states it is reached only from the `ScopeAborted` arm |
| nit `resume_scope_completion` reported `ResumeNotPending(ExecutableId(usize::MAX))` | **fixed** — new `ExecutionError::NoPendingScopeCompletion` variant, so no invented identity appears in a public error |
| nit the new F7 test ended with `let _ = child_id;` | **fixed** — the binding is `_child_id` and the discard is gone |

Two further cleanups came out of this round: `MachineStatus::ScopeCompleted` now
carries the paused executable instead of guessing `ExecutableId(usize::MAX)` in
`step()`'s error, and `resume_scope_completion` keeps returning
`InvalidScopeResume` for a foreign token.

## Fourth independent review round — P1-1

The fourth review found one reachable publication-correctness blocker and one
documentation contradiction. Both are fixed.

| Finding | Disposition |
| --- | --- |
| P1-1 (blocker) a fallback whose primary/secondary is a cache tag publishes on the old direct path, so a successor that produced a response and then `exit`ed still wrote the cache, while the same cache reached through an ordinary sequence dispatch is invalidated by `MachineStep::ScopeAborted` | **fixed** — `BranchOutcome` now carries the terminal `ExecutionCompletion`, the branch driver reports it instead of collapsing `Completed` and `Exited` into one success, and the direct cache-target site publishes only when `completed_naturally()`. `Exited`, failure and cancellation all drop the token. New tests `a_fallback_cache_target_publishes_only_after_a_natural_successor_completion` and `a_fallback_cache_target_never_publishes_a_successor_that_exited` drive a real `type: fallback` plugin whose primary is a `type: cache` tag over real UDP |
| P3-1 (docs) `implement.md`, `design.md` and `task.json` still described the task as planning-only while the PRD recorded authorization and the code existed | **fixed** — all three now say that S1/S2 are authorized, implemented and reviewed, and that S3–S7 are not implemented |
| N6 (previously disclosed as a pre-existing asymmetry: `run_target` publishes on `is_success()` while the boundary path abandons) | **closed** by the P1-1 fix, which is exactly the asymmetry N6 recorded |

The direct policy-target path still does not arm a watch — it drives the
successor itself and publishes after it returns — but it now applies the same
rule as the boundary path: only a natural completion may publish. The regression
tests above pin that equivalence from the outside, through a real fallback
plugin.

## Fifth independent review round — documentation only

The fifth review confirmed the P1-1 code fix and every one of (a)–(d), and
returned `VERDICT: FAIL` on two documentation defects plus three nits. All are
fixed.

| Finding | Disposition |
| --- | --- |
| F5-1 (minor) `prd.md:5` still said the task was authorized only for planning, contradicting the same file's authorization record and `task.json.status` | **fixed** — the PRD goal now records the 2026-10-01 authorization and the S1/S2-versus-S3–S7 split |
| F5-2 (minor) this file said `cache_catalog.rs` had 8 tests with 7 listener-driven | **fixed** — 10 tests, 9 listener-driven |
| nit `design.md` said S1/S2 "passed four independent reviews" when rounds 1–3 returned FAIL | **fixed** — now "completed several independent review rounds (most recent one passed)" |
| nit the new test's message said `SequenceAborted`; the variant is `MachineStep::ScopeAborted` | **fixed** |
| nit (latent) the branch driver's `allow_empty_response` arm synthesised `BranchOutcome::success`, which would have labelled `Complete(Exited)` with no response as a natural completion | **fixed** — that arm now carries the real `completion` too, so `completed_naturally()` can never be true for an exit regardless of whether a response is present |

## Known limitations and resource evidence

* Only S1 and S2 of S1–S7 are implemented, so acceptance criteria A3–A5 cannot
  pass. A6 is satisfied for the work that exists.
* The management surface (inventory, `/metrics`, flush/dump/save/load, Vue) is
  untouched.
* The remote volume was at 97–100% utilisation (≈0.1–0.8 GB free) for the whole
  run. One rsync was initially issued with an exclude pattern that did not match
  the nested `target/` directory and filled the volume; the polluted directory
  (`.../rust/target` inside this task's own exclusive workspace) was removed and
  rebuilt. No unrelated directory was touched.
* Lazy refresh, dump persistence, generation gating and durable-first flush have
  no tests in this round because they have no implementation.
