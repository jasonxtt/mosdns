# 5D-fix-B compact audit retention

Execution 2026-10-16/17 (Asia/Taipei). BASE
`353371fa9206d01c01eeb067bf8f7b64dd05f412`, branch `rust`. External Claude
acceptance remains required. Affected frozen gates PASS on the approved new
input. No commit, push, deployment or self-review round.

Concurrent workspace activity advanced HEAD to `b3703f3e` with only an unrelated
M2 backlog document. This executor did not commit it; it is preserved and
excluded from this task. Measured runtime-source hashes remain unchanged.

## Input provenance

The user confirmed the previous 5D-1/fix-A private archives were lost and
authorized replacement inputs, including the explicit read-only export of
production rule/gen/srs/adguard/webinfo to private local/VM directories.
Config package commit remains `0412666b9648a1f52d831e43d7817abd7a5ab556`.
New weighted corpus has 2486 cells, audit enabled, capacity 100000. This is a
new-input comparison; old-snapshot results are historical, not matched controls.

Fresh Go reference responses and unchanged BASE-native probes precede the
candidate. Only 28 differing cells satisfy the previously approved ASCII
case-insensitive `full:` no-v4/no-v6 contract; every other full DNS section
remains strict. Candidate replies do not construct expectations. Real names,
raw packets, settings and credentials remain outside the repository.

The original [frozen budget](../10-09-5d-1/frozen-thresholds.json) is byte-identical,
SHA256 `afe441e906ab0ca0d0ffad2c5d36c854c2d61a5725b79608e70f54d6cb8c70a9`.
Normal 20/100/500 p95/p99 ceilings are 14.025/15.168, 13.853/15.006 and
12.333/13.866 ms. Steady RSS must be <=Go and <=128 MiB each repeat; sampled
startup/load peak <=256 MiB. No threshold or rate definition is revised.

## Representation and equivalence

Keep public AuditRecord lossless. Retained rows own exact-length qname,
timestamp/client/elapsed, numeric trace identity and shared immutable common
facts/response details. Optional large diagnostics live behind the common-data
handle; shared response payloads trim unused answer/vector/text capacity. Two
fixed 4096-slot weak caches check full equality after hashing. Collisions cannot
merge distinct values, and old payloads drop with their final record/snapshot.
Expired weak slots retain only bounded fixed Arc layouts. Clear/resize resets
the hints. No answer truncation, byte eviction, capacity change or new dependency.

The original API projector is unchanged. A fixture runs the same deterministic
record/control sequence on uncompressed BASE and candidate, comparing 55302
response bytes from 24 requests: status/capacity/stats/windows/log pagination,
trace/answer-IP/CNAME/domain filters, five ranks, invalid capacity, stop/start/
clear and empty state. It includes unknown answer types, Unicode/raw data,
None versus empty string, both diagnostic schema versions and all attempt
outcomes. The fixture contains only synthetic example data. Existing real
UDP/TCP and HTTP audit/cancellation tests supplement that byte comparison.

## Field measurement

Release diagnostic hooks inspect original retained allocations before public
snapshot cloning. Count each shared allocation once, including separately held
slowest records and container capacity. Requested bytes exclude allocator bin
rounding/fragmentation/transient query/read memory; they are not process RSS.

Inline AuditRecord is592 bytes (including optional diagnostics136 and failure32);
the new retained row is128 bytes. Both measurements have100000 ring rows and
300 additional distinct slowest records. Same configuration/corpus/seed and
diagnostic schedule; timing determines which slowest records survive, so qname
totals need not match exactly. Requested storage falls from99176350 to21800224
bytes: **991.7635 ->218.00224 bytes per ring row, -78.02%**.

| Requested allocation bytes | Before | After |
| --- | ---: | ---: |
| Per-row inline + Arc | 60982400 | 14443200 |
| Shared common templates + Arc | included above | 67488 |
| Shared response structs + Arc | included above | 5832 |
| qname | 1898503 | 1898343 |
| trace text backing (numeric field included inline after) | 6820400 | 0 |
| final_sequence | 1309169 | 1800 |
| matched_group | 0 | 0 |
| domain_set | 1676439 | 3129 |
| effective_tag | 1325395 | 1858 |
| matched_rule_source | 1749067 | 2737 |
| final_upstream | 705422 | 632 |
| upstream_targets | 1407960 | 1365 |
| selected_upstream | 1501824 | 1456 |
| response source | 1160 | 76 |
| Answer vector | 12113920 | 2528 |
| Answer data | 1514240 | 871 |
| Answer decode error | 0 | 0 |
| Upstream attempts | 33895 | 2510 |
| Diagnostics heap | 889580 | 9367 |
| Failure heap | 0 | 0 |
| Expired weak Arc layouts | 0 | 44520 |
| Ring/timing/slowest/cache backing arrays | 5246976 | 5312512 |
| **Total** | **99176350** | **21800224** |

After:111 shared common templates and81 shared response payloads. See
[before](memory-before.json), [after](memory-after.json). No Mem-2 trie change
has been made. These live-allocation results do not substitute for RSS gates.

## Validation status

BASE-generated complete API bytes pass on BASE and candidate (fixture SHA256
`4d377fd728383f51a6095e7d2510de4c8aa005962a680681190e312cecc9b8ab`). Final VM
fmt/strict workspace-all-target Clippy pass; full workspace: **1482 passed,
0 failed, 4 existing ignored**. Tool suites14/14 and10/10 pass. BASE config_rust
check gaps=0, with stdout/stderr byte-identical to BASE. Both the mock-mapped
test root and an additional unmodified config_rust/production-data root using
the original migrated settings pass. The latter runs in `unshare --net` with
no public interface/route, has unchanged input-file hashes and byte-identical
BASE/candidate stdout/stderr. [Original-settings check](original-check.json).
Final source, input and
binary hashes are recorded in [environment](environment.json); the uninstrumented
release artifact is kept distinct from diagnostic binaries. All six formal
attempts completed without replacement; [matrix receipts](matrix.json),
[native samples](native.json), [Go controls](go.json),
[frozen comparison](comparison.json) preserve scatter and all shortfalls.

Formal driver: `tools/5d-1/fix_b_matrix.py`, fresh Go/native G/N,N/G,G/N, seeds
501/502/503, original warmup100qps10s, each30s (first5s settling), rates
20/100/500/1000/2000/4000/8000, then100qps60s. Cold dumps, unchanged snapshot,
SUT CPU0/helpers CPU1, loopback-only namespace/private tmp and local mock delays
as 5D-1. Preserve every attempt and cleanup receipt. Builds/tests finish before
formal timing; no build/profiler runs concurrently with acceptance traffic.

## Frozen gate results

Latency is the median of three stable per-run quantiles, as originally frozen.
RSS/correctness/coverage are checked separately for each repeat. Audit on,
capacity100000 throughout. Steady RSS uses the original stage windows; the2k
window includes the original ring-fill ramp, while8k follows a full ring.

| Item | Frozen limit | Native result | Decision |
| --- | --- | --- | --- |
| 20QPS p95/p99 median | 14.025/15.168ms | 12.169223/14.699154ms | PASS |
| 100QPS p95/p99 median | 13.853/15.006ms | 11.717686/13.219337ms | PASS |
| 500QPS p95/p99 median | 12.333/13.866ms | 2.173068/12.244458ms | PASS |
| Normal20/100/500 on-time correct and coverage each repeat | >=99.9% | 100% all9 stable cells | PASS |
| 2k steady RSS each repeat | <=128MiB and<=freshGo171.617MiB | 57.969/57.734/57.887MiB | PASS |
| 8k steady RSS each repeat | <=128MiB and<=freshGo194.789MiB | 62.367/61.805/61.977MiB | PASS |
| Sampled startup/load peak each repeat | <=256MiB | 82.891/86.523/61.977MiB | PASS sampled metric |
| 2k / 8k minimum on-time correct and coverage per repeat | >=99.9% | 100% /99.986% | PASS |
| Wrong/timeout/late/send-error/orphan replies | zero wrong; deadline500ms | zero across every stage of all six runs | PASS |
| CPU profiling trigger | any non-overload repeat>1.5xfreshGo median | not triggered at any stage | PASS trigger check; CPU report-only |

Native1,426,800 planned/1,426,741 correct/59 client-unsent; Go1,426,800 planned/
1,426,688 correct/112 client-unsent. No wrong/late responses, timeouts or orphans.
All unsent slots remain in their original denominators.
Stable8k native unsent0/28/1; freshGo66/0/21. No run invalidated or replaced.

**20QPS tail disclosure:** native p99 by seed501/502/503 is
14.699154/**83.479488**/13.625189ms; freshGo15.369278/**82.873236**/13.533403ms.
Seed502 therefore exceeds the ceiling individually for both roles; the frozen
gate is the median of three, not an each-repeat latency ceiling. All values are
retained. Unlike the old snapshot, the new mixed workload reaches
`foreignecs:https` through SOCKS5 (80ms mock) as well as domestic/nocnfake UDP.
The same seven corpus cells account for every>50ms stable20QPS reply in both
seed502 runs. Each has same-question foreignecs HTTPS and nocnfake UDP events
inside its planned-to-finish window; client finishes1.7–3.6ms after the foreign
response event. This is event correlation, not exclusive-supplier attribution
or precise latency decomposition. [Numeric evidence](q20-tail.json). No tail
value is waived or relabeled as an invalid sample.

CPU medians native/freshGo (microseconds per correct):20QPS340/320,
100QPS248/236,500QPS143.2/135.2,2k76.2/77.0,8k49.607/47.505. These are process
tick-bracket observations, not a per-function cost measurement or proof of
CPU improvement. F,16k/actual overload recovery and5D-2/3 are unperformed;
computed capacity/tail components in comparison.json do not accept those scopes.

## Stop and cleanup

All nine oracle/diagnostic/formal run cleanup receipts pass. Prearchive check
finds no task-owned process or5d1 namespace; original VM service remains
active/PID827437. [Prearchive receipt](prearchive-cleanup.json).
Private archive saved at
`/private/tmp/5dfixb-20261016/private-evidence-20261017.tar.gz`, mode0600,
225745570 bytes/4771 members, SHA256
`62f18fef57ad4d3fa99a2689f7e2ab07ce93191e5408c777bbd5c768b12bab03`.
Streaming verification checked all2853600 formal raw request rows, all matrix
pinned files and all measured runtime-source hashes before VM deletion.
[Archive integrity](archive.json). Rebuildable targets/node_modules are excluded.

Deleted only `/root/5dfixb-20261016` and its VM archive copy; no owned process or
namespace remains, original service still active/PID827437.
[Final cleanup](cleanup.json). Archive contains pre-deletion document/receipt
snapshots; this final deletion receipt is separate and authoritative. No task
commit/push/deployment, archive of the Trellis task, or next-task work.
The additional original-settings check temporarily recreated the same private
VM root, then deleted it again; cleanup.json is the latest receipt. Its outputs,
script and preparation-failure log are retained in the separate local
`/private/tmp/5dfixb-20261016/original-check-evidence.tar.gz` (2355 bytes/8 members,
SHA256 `ae76bd4fcb453ec42b0e2571010908285763274c264f9d0c527fb253160dc084`).
The tested binaries and inputs come from the verified main archive; no rebuild,
new production export or performance sample change occurred.
External Claude acceptance and the old-snapshot reproduction limitation remain;
no production/default release or subsequent task is authorized by these results.

## 外部审查结论（2026-10-17）

Claude 第 1 轮 **FINAL: PASS**（无 P0/P1）。每条保留记录 991.8 → 218.0 字节；稳态 RSS 2k≈57.9 MiB、8k≈62.0 MiB（上限 128，Go 171.6/194.8），
峰值 ≤86.5 MiB；延迟与正确率门槛全部通过。连同 5D-fix-A，5D-1 冻结预算的两项 FAIL 均已在不改阈值的前提下解决。
P2：`common()` 返回含占位字段的完整记录，后续可考虑改为不含逐条字段的专用模板类型。
