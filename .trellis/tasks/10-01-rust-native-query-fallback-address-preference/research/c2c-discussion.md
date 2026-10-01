# C2C planning discussion — 2026-10-01

User explicitly requested another C2C planning opinion and independent triage.
New project chat, workspace mosdns-rust / branch rust confirmed in completed reply:
https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6abdc416-9dcc-83ee-8d3d-c6fb3e6c5449
This is a planning discussion, NOT an implementation reviewer binding, committed
range review, authorization snapshot or task lifecycle verdict.

Initial completed reply: NOT READY pending three ownership clarifications.
Codex source-checked judgment and disposition:

| Finding | Judgment | Action |
| --- | --- | --- |
| P1 inherited-response eligibility/supplier matrix | Accept clarification; existing intent correct but table useful. | Normative matrix separates control winner from actual supplier, including both-fail/outer-try. |
| P1 root fuel fork API | Accept; current ExecutionControl Clone copies local u64. | Explicit shared constructor/fork, root Rc<Cell> debit; forbid control.clone for branches; hierarchical loser cancellation. |
| P1 schema2 canonical owner | Accept precision; shared projection already exists in current API. | Root execution/checkpoint bit -> one typed AuditRecord -> existing common JSON mapper; no new storage framework. |
| P2 evidence-cache namespace | Partial: isolation is already guaranteed per callsite/fixed family. | Explain logical namespace/separate storage; do NOT mandate redundant physical tuple keys. |
| P2 threshold start clock | Accept precision. | Driver entry before child first poll, monotonic and capped; no timer reset at network send. |
| P2 schema2 causal red tests | Accept intent. | Add concrete interface tests for inherited/local/cache/loser and all HTTP views. |
| P3 execution start / extra panel | Already frozen. | Preserve approval gates; reiterate existing renderer only in handoff. |

Rejected details: inherited response cannot promote a failed child to a successful
fallback winner. Both-fail returns typed failure, with outer caller/try semantics
unchanged. Preference suppression is a NEW LOCAL empty NOERROR, not original or
preexisting response wire. A branch control decision=selected may exist with no
network supplier (cache/local); diagnostics.selected is a separate network-only
object. Do not impose network supplier constraints on control selection.

Only planning artifacts modified; approved product scope/defaults unchanged.
Follow-up discussion completed after these corrections. No product code edits, start, commit/push/deploy or tests.


## Completed second response

Same conversation, iteration 1: **Planning READY**. The reply explicitly closes
inherited-response decision matrix, root shared budget/hierarchical cancellation
and canonical schema ownership; confirms cache namespace/clock/suppression
corrections and existing detail-only scope. No remaining blocking planning issue.
This is advisory planning readiness, NOT FINAL: PASS for committed code and NOT
implementation authorization.

Two nonblocking implementation cautions: preserve old synchronous core callers
while keeping private mechanisms narrow, and keep BranchResult immutable terminal
outcome separate from mutable diagnostic collector/checkpoint lifecycle. These
are already consistent with the chosen architecture; no scope/framework added.
No more message is sent merely to solicit a stronger verdict.

Planning authorization remains unchanged. Final user review of the revised
artifacts precedes implementation. New planning chat is saved only as the C2C
planning session; implementation dedicated reviewer must still be separately
resolved/verified for this task before its genuine authorization snapshot/start.
