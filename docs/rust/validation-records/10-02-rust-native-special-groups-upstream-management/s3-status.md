# S3 exact-source PASS — remediation validation

S2's dedicated exact-source review passed `a9fe9f0eb5ceac81f7b43407acb2f0e81cdfaedf`. S3's first exact-source review of `a5b3a936fde0cd5ea0ec0f856ad324c8a0c1665e` returned FAIL with findings P1-1 and P1-2; see [round 1](s3-review-round-1-result.md). Both findings now have RED/GREEN regressions, the remediation source passed isolated-host validation, and the same dedicated reviewer returned exact-source PASS on `6b50220d1fe137cd2722c44da7c0e095df9095e1`; see [round 2](s3-review-round-2-result.md). The formal Trellis controller recorded the PASS and advanced to Slice 4. S4–S7 and whole-task PASS remain pending.

## Implemented behavior

- The native managed state root has an OS-level single-writer lock. A bounded, scoped undo/redo journal records path and SHA-256 baselines, staged bytes, backups, input dependencies, generations, and the durable commit marker. Recovery validates containment and artifacts, then rolls back before the marker or completes the selected generation after it. Corrupt or ambiguous state fails closed.
- Candidate compilation consumes changed JSON/manual files and generated YAML as one immutable snapshot. It captures the canonical config/include/rule input set and verifies file digests before and after durable staging, preserving external edits as conflicts. The state-root writer stays owned by the runtime control handle.
- Runtime apply prepares the candidate graph, listener/cache resources, and metadata before admission closes. It fences new DNS and management admissions, waits for admitted management mutations, gates cache persistence, stages and replaces files, writes the durable marker, then publishes the preallocated runtime and metadata. Before-marker failure rolls back; rollback failure or ambiguous marker state enters `recovery_required` and retains the journal. After the marker, the transaction remains owned through publication and retirement.
- API disconnect and host shutdown do not abandon an admitted transaction. Shutdown before the marker rolls back. Once committed, swap/recovery bookkeeping completes before shutdown proceeds. Cache reads by already captured old-generation requests remain available while late writes from the stopped owner stay fenced.

## Exact-source validation checkpoint

All Rust commands ran on the isolated SSH `mosdns-rust` host under `/root/mosdns-rust-special-groups-20261003`, using `src` and `target-candidate`. No public DNS or port 53 was used.

- Native-host serial suite: **352 passed / 0 failed / 3 ignored** across 29 suites. The three ignored entries are subprocess probes explicitly invoked by parent tests.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --target-dir /root/mosdns-rust-special-groups-20261003/target-candidate -p mosdns-native-host --all-targets -- -D warnings`: passed.
- The final tested input manifest covers 140 files (126 Rust source files); local and isolated-host SHA-256 values match with no differences.
- Real process-death tests cover 37 filesystem boundaries. Parent tests also kill/restart across durable marker, runtime swap, retirement, and cleanup, then verify recovery of the committed candidate.
- Focused transaction tests prove a pre-marker shutdown rollback, conflict preservation, rollback failure classification, and a committed apply that completes after client disconnect and host shutdown. Cache read continuity has a retained RED/GREEN regression pair.

Evidence is retained in `evidence/s3-current-native-host-tests-final1.log`, `evidence/s3-current-fmt-recheck3.log`, `evidence/s3-current-clippy-recheck3.log`, `evidence/s3-final-source-manifest.json`, `evidence/s3-sigkill-matrix-green.log`, `evidence/s3-transaction-ownership-tests-recheck2.log`, and `evidence/s3-captured-cache-read-{red,green}.log`. Earlier failed attempts and repaired reruns remain alongside them.

## Remaining gates

### Review findings and remediation

- **P1-1 — startup lock/compile snapshot mismatch.** Reproduction rewrites the root config from unmanaged to managed between preflight and compilation. RED fails when the old implementation compiles the second read into a managed graph without the lock; GREEN passes after startup compiles the already-read YAML snapshot against the same config base. See `evidence/s3-p1-1-{red,green}.log`.
- **P1-2 — optional manual-file absence omitted from candidate inputs.** Reproduction creates `rule/special_50.txt` after candidate compilation. RED allowed preparation; GREEN rejects the newly created input as a conflict and retains the external file. The existence probe now uses the bounded blocking I/O worker. See `evidence/s3-p1-2-{red,green}.log`.

Post-remediation serial native-host validation passed **354 / 0 failed / 3 ignored** across 29 test binaries; the three ignored subprocess probes are explicitly invoked by parent tests. `cargo fmt --all -- --check` and strict all-targets Clippy passed. See `evidence/s3-p1-rerun-native-host.log`, `evidence/s3-p1-rerun-fmt.log`, `evidence/s3-p1-rerun-clippy.log`, and the exact [140-file source manifest](evidence/s3-p1-remediation-source-manifest.json) (126 Rust files, zero local/remote hash differences).

The exact-source re-review range is `a5b3a936fde0cd5ea0ec0f856ad324c8a0c1665e` → `6b50220d1fe137cd2722c44da7c0e095df9095e1`; its tree was checked against all 140 tested input hashes. The reviewer returned `FINAL: PASS` with no actionable findings. The bounded worker and coordinator are not yet wired to management HTTP routes; that is S5. Transitive cache dependency invalidation and old-writer fencing remain S4, and S6/S7 UI and full-chain proofs remain pending. No whole-task, deployment, push, or production claim is made.
