# Real combination evidence

Date: 2026-10-01 (Asia/Shanghai)

All remote commands below ran through the SSH alias `mosdns-rust` in the
task-owned isolated directory:

`/root/codex-rust-native-query-fallback-20261001-0300`

## Remote resource and ownership checks

- Preflight and final checks reported approximately 3.1 GiB free on `/root`
  and 16% inode use after each exact task-owned Cargo target was removed.
- One earlier monolithic workspace test attempt filled the task-owned Cargo
  target filesystem. Its exact task-owned test processes were stopped and its
  exact task-owned target directory was removed. That attempt is not counted
  as a passing result.
- The successful final checks used separate task-owned Cargo target
  directories and `-j 1`; no unrelated process or directory was removed.

## Rust validation

- `cargo test -p mosdns-native-host --tests -j 1`: passed. This covered 82
  native-host unit tests and all native integration suites, including the
  observability, cache, config, composition, policy, routing, TCP, UDP, and
  upstream suites.
- `cargo test -p mosdns-native-host --doc`: passed.
- `cargo test --workspace --exclude mosdns-native-host -j 1`: passed, including
  non-native unit, integration, and doctest suites.
- After the final trace-attempt guard change:
  `cargo check -p mosdns-native-host --tests`: passed;
  fallback and preference policy unit tests: passed.
- `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo build -p mosdns-native-host -j 1`: passed.

After the dedicated review's scoped FAIL, the repair round repeated the
native-host test package and the non-native workspace split. Both passed
again, including integrations and doctests. The repair round also repeated
format check, workspace clippy, the fallback/preference unit tests, and the
native-host binary build. A later clean-target rerun passed the complete
native-host integration package again after the final caller/cache/diagnostic
repairs; the first rerun attempt is retained as an unresolved disk/linker
failure and is not counted as green.

The split workspace commands cover the same workspace packages while keeping
the task-owned remote target below the available disk budget. The monolithic
workspace attempt is retained as a resource limitation, not reported as green.

After the final dedicated-review repair, the clean rerun passed the complete
native-host integration package again (82 unit tests plus all native
integration suites), the complete non-native workspace split again (including
the long DoQ/DoH3 composition tests and doctests), and final fmt/clippy. The
repair specifically removed the extra threshold cap, made cache guards
branch-local, preserved nested factual supplier selection, used the
primary-first standby poll, closed policy failures with the already-driven
successor state, and allowed a named sequence target with no response to feed
the common successor. The transient full-target disk failures were retained
as evidence but are not counted as green.

## Real fallback DNS + HTTP + Vue proof

Two task-owned loopback UDP upstreams were started: primary `127.0.0.1:18531`
with a 200 ms response delay and secondary `127.0.0.1:18532` with a 10 ms
delay. The native host listened on `127.0.0.1:18553`, with audit HTTP on
`127.0.0.1:18080`.

`dig @127.0.0.1 -p 18553 example.com A` returned `192.0.2.32` from the
secondary. The HTTP audit response showed:

- `schema_version: 2`;
- root branch completed;
- primary branch `decision: canceled` with an `interrupted` attempt;
- secondary branch `decision: selected`;
- selected source `secondary_forward`, peer `127.0.0.1:18532`, transport `udp`.

The repaired run additionally showed the primary attempt's peer and preserved
registration order: ordinal 0 primary/canceled, ordinal 1 secondary/response.

The built Vue bundle was served through a task-owned HTTP proxy on port 18082
and returned `HTTP/1.0 200 OK` for `/index.html`. The in-app browser then
opened the actual QueryManager record and its existing detail renderer. The
accessibility snapshot showed the schema-2 branch table with `root/primary/
secondary`, numeric QTYPE `1`, `selected/canceled`, and the attempt table with
`primary_forward` ordinal 0 and `secondary_forward` ordinal 1. This is the
browser rendering proof, not only a static bundle fetch.

The same final binary was restarted with a real preference config. A real
`dig @127.0.0.1 -p 18553 example.com AAAA` produced empty `NOERROR` after the
loopback peer received both the original AAAA and rewritten A queries. The
HTTP record contained schema 2 branches `original` QTYPE 28 `suppressed` and
`reference` QTYPE 1 `completed`, attempts 0/1 with QTYPE 28/1, and omitted
`selected` because the final wire was local suppression. The browser detail
renderer displayed those same rows and the text `未选中最终网络上游`.

All task-owned DNS, API, proxy, upstream, and SSH-forward processes were
killed by their exact PIDs after capture.

## Real preference proof

The native-host policy test and the final live native DNS/API/browser proof
captured the AAAA-to-A reference rewrite: the original AAAA branch was
suppressed and the reference A branch completed, while the final wire
response preserved the original AAAA question and returned empty `NOERROR`.
The corresponding audit response and existing detail renderer carried schema
2 branch and attempt `qtype` fields.

The live preference peer used for this proof was intentionally A-only, so its
original AAAA response was malformed and the proof does not claim a valid
AAAA upstream answer. The deterministic native test separately verifies the
QTYPE rewrite, branch correlation, suppression, and final-question behavior.

## UI build

- `npm ci`: passed; npm reported five audit findings (1 low, 1 moderate,
  3 high) in the existing dependency tree.
- `npm run build`: passed.
- `npm run build:log1`: passed.

Vite emitted only the existing large-chunk warnings. No production service,
public DNS, port 53, or installed deployment was touched.

## Secure and stream composition evidence

The final remote native integration run passed the real TCP listener suites
(`w1_tcp`, `slice2_tcp`, `slice3_composition`) and
`slice9_forwarding::native_secure_forwarding_proves_dot_and_doh_h1_h2_with_synthetic_ca`,
which exercises native DoT and DoH HTTP/1.1/HTTP/2 peers with actual TLS and
DNS wire exchanges. The same run passed the secure busy-admission and close
drain test `slice9_forwarding::secure_busy_admission_uses_a_fresh_connection_for_dot_and_doh`.

## Final scoped repair validation

The dedicated review's last scoped FAIL identified three remaining ownership
gaps. The final repair sends fallback/preference policy failure through
`ExecutionMachine::resume(..., Err(ExecutorError))` with the already-driven
successor state, keeps cache access in a path-local shared cell while
resetting it only for policy siblings, synchronizes the root cache wrapper
after a winning branch, and commits an eligible primary success immediately
even when an always-standby secondary is still pending.

After that repair, over SSH alias `mosdns-rust`, the complete native package
passed again: `cargo test -p mosdns-native-host --tests -j 1` (82 unit tests
plus all integration suites). The complete non-native split passed again with
`cargo test --workspace --exclude mosdns-native-host -j 1`, including
integration suites and doctests. `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo build -p mosdns-native-host -j 1` also passed. The earlier live DNS,
HTTP, and in-app Vue detail proof remains on the same final product paths;
the repair changed only policy/control ownership and does not add a route or
renderer.

The follow-up root-cache synchronization commit was then rebuilt and the
complete native package was rerun once more; all 82 unit tests and native
integration suites passed again.

The final policy-trace repair was then rebuilt and the complete native
package was rerun again with the same result. Root recovery ledger entries
now register directly into the live schema-2 trace after policy branches,
and a caller/try-absorbed policy error clears its temporary local failure
provenance before a later successful response is terminalized.

## Dedicated-review repair rerun

The dedicated reviewer then identified three additional in-scope ownership
gaps. The repair keeps every traced root invocation on the canonical metric
and legacy summary/selected bookkeeping path while suppressing only its
duplicate schema-1 diagnostic append; it terminalizes a registered root trace
slot before finalizing a forced-drop snapshot, so a dropped policy recovery
has one branch/qtype-complete attempt; and it leaves root `Cancelled` and
`BudgetExceeded` policy failures as terminal native results instead of
converting them to recoverable `ExecutorError` values.

After this repair, the isolated SSH run passed `cargo test
-p mosdns-native-host --tests -j 1` (82 unit tests plus all native integration
suites, including secure DoT/DoH and stream/close coverage),
`cargo fmt --all -- --check`, workspace clippy with `-D warnings`, and
`cargo build -p mosdns-native-host -j 1`. The task-owned remote target was
removed afterward; final resource checks again reported approximately 3.1
GiB free and 16% inode use. The non-native workspace/integration/doctest
split and the real DNS/HTTP/Vue and secure proofs remain unchanged because
this repair is confined to `rust/native-host/src/execution.rs`.

The final regression-test addition raised the native unit count to 83. A
fresh isolated rerun passed all 83 unit tests and all native integration
suites, the terminal-error regression, fmt, workspace clippy with
`-D warnings`, and the native-host build. The first clippy attempt during
this rerun exhausted the task-owned filesystem while a second target was
present; both exact task-owned targets were removed before the successful
clean rerun and the resource check passed.

## Final commit-gate repair rerun

The dedicated reviewer then isolated three remaining final-commit gaps. The
repair makes final network supplier selection overwrite stale ancestor peer
identity, clears peer and trace selection when a local/cache/suppressed result
wins, and synchronizes the selected peer from the committed schema-2 trace.
It also routes every buffered preference-original return through
`commit_branch_winner`, and terminalizes any tentative selected branch as
`canceled` or `interrupted` when the root cancellation/deadline gate rejects
the result.

After syncing the changed source to the task-owned directory, the isolated
SSH run passed `cargo test -p mosdns-native-host --tests -j 1`: all 83 native
unit tests and every native integration suite, including secure DoT/DoH and
stream/close coverage. `cargo fmt --all -- --check` and workspace clippy with
`-D warnings` also passed. A `cargo build --workspace --all-targets
--all-features` attempt was stopped after the task-owned filesystem reached
100% during test-target linking; its exact task-owned process and target were
removed. The subsequent serial `cargo build --workspace --all-features`
passed. Final cleanup restored approximately 3.1 GiB free and 16% inode use.
The earlier complete workspace integration/doctest split and real DNS/HTTP/
Vue/secure transport proofs remain valid because this repair is confined to
`rust/native-host/src/execution.rs`.

## Final terminal-state repair rerun

The dedicated reviewer found two coupled terminal-state defects in the last
repair. Root `Cancelled` and `BudgetExceeded` errors were classified only
after the temporary no-usable-upstream provenance had been written, and
terminal cleanup could relabel every historical selected branch. The repair
classifies terminal errors before recording recoverable policy failure and
reduces trace cleanup to clearing only the factual selected supplier. Every
fallback winner and the preferred-family direct path now records branch
selection only after its commit gate has returned, so a rejected tentative
winner cannot leave a newly selected supplier behind; completed historical
branch decisions remain unchanged.

The next isolated SSH rerun passed all 84 native-host unit tests and all
native integration suites. The new regression confirms supplier cleanup does
not relabel historical branch decisions. The cancellation-sensitive API test
briefly failed once with a timing result of 200 instead of 503, passed on its
immediate isolated rerun, and passed again in the complete suite. Fmt,
workspace clippy with `-D warnings`, and serial
`cargo build --workspace --all-features` passed; exact task-owned Cargo
targets were removed afterward and resource checks again reported about 3.1
GiB free and 16% inode use.

## Final commit-gate fact-status rerun

The final reviewer found one remaining distinction: when a valid child
response completes but the parent commit gate is rejected by root cancellation
or deadline, the child execution fact must remain `completed`, not become
`failed` merely because the parent returned a terminal error. The repair adds
a committed-outcome trace helper that preserves completed child status for
that terminal gate case while leaving final selected supplier and root
terminal provenance cleared.

The isolated SSH rerun then passed all 84 native-host unit tests and all
native integration suites, including the secure DoT/DoH and stream/close
coverage. Fmt, workspace clippy with warnings denied, and serial
`cargo build --workspace --all-features` passed. The task-owned Cargo targets
were removed afterward; disk and inode checks again reported about 3.1 GiB
free and 16% inode use.

## Final fallback-terminal aggregation rerun

The dedicated reviewer then found the remaining root-terminal downgrade in
fallback's four no-usable-response aggregation points. When both child paths
ended because the shared root was canceled or its deadline expired, those
points still manufactured a recoverable ExecutorError. The repair centralizes
aggregation in `fallback_failure`: root cancellation and root deadline retain
typed Cancelled and BudgetExceeded respectively, while loser-specific branch
cancellation still produces the ordinary fallback ExecutorError.

The isolated SSH rerun passed all 85 native-host unit tests, including the new
root-terminal aggregation regression, and all native integration suites.
Fmt, workspace clippy with warnings denied, and serial
`cargo build --workspace --all-features` passed. The task-owned Cargo targets
were removed afterward; disk and inode checks again reported about 3.1 GiB
free and 16% inode use.
