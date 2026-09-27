# C2C independent planning review

Reviewer conversation: [mosdns-rust planning review](https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6ab7f1f2-e4c4-83e8-af85-d0ff49e8cd73)

User authorized a new c2c planning-review conversation on 2026-09-27. Workspace identity and AGENTS.md reading were verified: mosdns-rust, branch rust. The reviewer reads current uncommitted planning files, **not** a committed-range diff. C2C task: `c2c_8c43`.

Input digests: `c2c-planning-review-input-v1.json`, `c2c-planning-review-input-v2.json` and `c2c-planning-review-input-v3.json`. These inputs represent planning files, not executed product changes, tests or benchmark results. Trellis remains planning; implementation_authorized=false.

## Round 0 — FINAL: FAIL

| Finding | Root cause | Planning remediation in revision 2 | Reviewer closure |
| --- | --- | --- | --- |
| P1-1 | Absolute request deadline/late collection and API send-time anchors were not fixed; old TCP helper resets timeout after connect. | planned-at monotonic service deadline; separate collection deadline only for fully sent requests; no phase reset/expired new send; explicit API write start/complete; segment/race/wall-clock tests. | Closed by round 1 |
| P2-1 | A peak-rate calibration alone cannot qualify cumulative fresh-TCP port/TIME_WAIT pressure for a complete ladder. | Full-duration/total-connection calibration plus peak margin, port/reuse/errno/repeat cooling evidence and frozen G2 envelope. | Closed by round 1 |
| P2-2 | Writer-error/queue-full tests did not explicitly cover a permanently blocked non-error sink. | Permanently blocked sink case, bounded cancellation, independent control accounting, visible lost-journal ranges and evidence/load invalidation. | Closed by round 1 |

Revision 2 also maps behavior slices to public reliability-run/reliability-assess CLI boundaries and allowed system mocks. No Rust/Go product code, SSH, project build or measurement was executed. Local planning consistency and context validation pass; automatic injection warnings for large specs are handled by explicit full reads in inline execution.

## Round 1 — FINAL: FAIL

The reviewer independently re-read revision 2 and verified all six input hashes match. P1-1, P2-1 and P2-2 explicitly closed. One new root cause:

- P2-3: Slice 3 still requested two latency views after the sender design defined three. Revision 3 explicitly requires planned-slot-to-finish, dispatch-to-finish and write-start-to-finish, with start offsets, sample counts and failure denominators reproduced offline. Requests that never dispatch/write do not acquire fictional samples.

## Round 2 — STATE: DONE, FINAL: PASS

The reviewer explicitly closed P2-3, verified the current implement.md hash against revision 3, and confirmed P1-1, P2-1 and P2-2 remain closed after rereading the substantive planning files. No new actionable planning findings were reported. All four findings are closed; open findings: 0.

This verdict accepts the revision-3 planning only. It does not authorize implementation, mark the Trellis task complete, qualify a measurement environment, or claim product/test/benchmark acceptance. The future implementation reviewer remains unassigned.
