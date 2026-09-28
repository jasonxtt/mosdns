# Review restart design

## Review target

- Repository branch: `rust`.
- Base commit: `c4a785fea6e66531921396def362d0c44d4a1666`.
- Original canary boundary commit: `879f53d283d2ef19cc87bcec59ba80d7f1e25cc2`.
- Review head: the later committed correction head, frozen before review submission.
- Original canary commit changed these six paths:
  - `.trellis/tasks/09-28-rust-mos-test-native-sidecar-canary/design.md`
  - `.trellis/tasks/09-28-rust-mos-test-native-sidecar-canary/implement.md`
  - `.trellis/tasks/09-28-rust-mos-test-native-sidecar-canary/prd.md`
  - `.trellis/tasks/09-28-rust-mos-test-native-sidecar-canary/research/canary-inputs.md`
  - `.trellis/tasks/09-28-rust-mos-test-native-sidecar-canary/task.json`
  - `docs/rust/next-stage-plan.md`

The original canary commit records attempt 3 as PASS on the `mosdns-rust` alias, existing-service isolation and cleanup, plus explicit unrun work and limitations. The later correction adds sanitized [controller evidence](../../../09-28-rust-mos-test-native-sidecar-canary/research/attempt3-controller-evidence.json), actual review chronology, and Trellis guard repairs. The submitted initial range was `c4a785fea6e66531921396def362d0c44d4a1666..242cbcbbc2d02c9ae77a81291a07c5c143ee6b57`, with `879f53d` as the original boundary; the scoped remediation range was `242cbcbbc2d02c9ae77a81291a07c5c143ee6b57..79ddded8edea9f53b07d051ce20b3daf6b56e868`. An earlier same-chat worktree review passed, but it was not acceptance of this committed range. The target-change preflight was reviewed by a different Codex reviewer; the original task lacks a pre-start automation snapshot. Both facts were disclosed in the request.

## Reviewer and authorization boundary

Use the dedicated C2C reviewer binding that points to the user's selected mosdns-rust Project conversation. Before task start, resolve it against the workspace Project/connector identity and verify the platform-native transport against that exact chat. Do not substitute the planning chat URL or another conversation. Create a genuine authorization snapshot for only `Slice 1`; only then start this replacement task and activate the run.

Before each review send, persist `parent_sha`, `head_sha`, request kind, and verified reviewer target in the active automation run. Build one atomic request using the Trellis C2C `REVIEW_ONLY` helper. The message must name the exact range and paths, summarize validation and acceptance, state prohibited scope, and instruct the reviewer to use read-only `git_compare`. Do not include any file text, diff, log, credentials, or unrelated changes. Keep the request within the helper's 4096-byte limit. Wait for one complete explicit final result; pending or partial output remains pending.

For a FAIL limited to the review documentation, fix only the cited root cause and provide the prior reviewed head as the re-review parent plus the new full head SHA. Preserve finding IDs by root cause. Product-code or canary-execution findings are outside this task: stop, retain evidence, and request a separately scoped task rather than expanding authorization.

## Existing validation evidence

- `python3 ./.trellis/scripts/task.py validate .trellis/tasks/09-28-rust-mos-test-native-sidecar-canary` passed before the original commit.
- `git diff --check` passed before that commit.
- The sanitized attempt-3 controller record classifies the canary as PASS and provides source, binary, twelve query/counter cases, process cleanup, and service-baseline evidence. Its raw source output SHA-256 is `5c519eea2974c28a452c1cf718e810d3d75d8e1e28579663d1f85e6ed7880d48`; the sanitized artifact SHA-256 is `34a0743bd5ab5cb360e0489dacff88bb716418facf0dc9aea54c0da614d1a58f`.
- The same C2C conversation returned `FINAL: PASS` on worktree review iteration 9. The exact committed-range review is still pending.
- No canary rerun or project test run is needed for this review-only task.

## Supersession handling

After the new task receives explicit review PASS, use `task.py supersede` to mark the original task terminal `superseded` with the replacement identity and reason. Do not archive it or claim normal completion.
