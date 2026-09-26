# Post-M7 bounded cost remediation

User authorized fixing discovered issues, following explanation that M7's
sender shortfall and audit-on p99 concern do not establish a Rust-language
performance limit. Executor inline, reviewer002reviewer native task
01a0d43d-d0aa-7401-af0f-2ca3a45ba519. No measured traffic in this unit.

## Source-backed findings and changes

Audit admission called render_qname, which built per-label strings, a label
Vec, join output and formatted escape strings. These are avoidable temporary
allocations for every audit-enabled query. It now makes two bounded passes
over the parsed name and reserves only final escaped-output bytes, then
appends directly. Root/trailing dot/dot-backslash/decimal-byte escapes remain
identical. Disabled audit still skips rendering. No observer synchronization,
retention, event or DNS semantics change.

Client requestLedgerWriter wrote each completed request directly to os.File
under its mutex. This adds one file write per query to the1CPU test client.
It now uses the existing bufio library with a bounded64KiB writer, keeps the
same lock/JSONL records, flushes full buffers and flushes/syncs/closes at stage
completion. Write/flush/sync/close failures still invalidate the stage;
post-close writes reject. This is batched evidence I/O, not schedule tolerance
or silently counting missed queries as sent. Abrupt process termination can
leave up to one buffer absent from disk; incomplete evidence cannot pass.

Neither cost is isolated as the cause of M7 lag or tail latency. No claimed
speedup, capacity qualification or acceptance repair without fresh measured
evidence. Original M7 and earlier source/binary/input artifacts stay frozen;
changed source must be rebuilt/pinned before any new measurement. Existing
M7 driver is not rerun with its old helper/candidate hashes.

## Red/green and validation

Audit admission regression failed on old implementation: a.b. had capacity8
for4 output bytes. New implementation passes exact output/capacity, escaped
bytes, admitted/completed accounting and bounded eviction assertions.
Ledger concurrent-write regression failed on old implementation:4180bytes
already written before close. New implementation buffers those20records,
then persists exactly20valid JSONL rows. Additional tests cover full64KiB
buffer flushing(400records), final flush error, and post-close rejection.
These tests use only memory/filesystem boundaries, no benchmark traffic.

Full native-host suite88tests passed, including W1 TCP/UDP,W2 cache,W3 route,
audit on/off, cancellation, shutdown and rebind. Go helper full race suite
passed; ledger focused race suite and Go vet pass. Rust clippy/format results
are recorded in implement.md before submission. No unsafe/dependencies/new
CLI/YAML surfaces added. Unrelated dirty tasks/journals excluded from commits.

Next boundary is a separately frozen build/input identity and the same simple
real-host comparison after code review. No automatic additional test batch,
production change, fullA5 or task closure is authorized by this code review.
