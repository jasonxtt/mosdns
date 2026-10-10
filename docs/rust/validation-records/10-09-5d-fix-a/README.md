# 5D-fix-A: bounded concurrent DoH reuse

Execution2026-10-09/10, user-approved fix after accepted5D-1 external Claude
review. BASE `cd0162cbc8add0bde133f89733fa407b9b080a02`, branch rust.
No Go/config_all/config_rust edits, deployment, commit or protocol expansion.
External Claude review remains required before accepting this task.

DoH alone revises the serial/busy-fresh policy: h2 independent streams on one
authenticated connection, peer SETTINGS plus64 local leases; h1 exclusive
reusable pool12;256 admitted calls and original absolute deadlines. Twelve
leaves50% headroom above100QPS×80ms's eight average in-flight requests.
Single caller cancellation isolates its h2 stream or discards its h1 socket.
Only typed NotSent on a reused attempt permits one replacement; ambiguous
in-flight failures do not replay. Retired scopes retain budget slots until
drain; close/idle expiry reclaim all. Endpoint trust and SOCKS route are fixed.
See the [revised contract](../../contracts/rust-migration.md#scenario-native-doh-bounded-concurrent-reuse-2026-10-09-revision).

The [original user freeze](../10-09-5d-1/frozen-thresholds.json) is byte-identical,
SHA256 `afe441e906ab0ca0d0ffad2c5d36c854c2d61a5725b79608e70f54d6cb8c70a9`.
Latency gates use medians of three stable per-run quantiles; correctness and
send coverage apply separately to each repeat, retaining all failures/unsent.
CPU is report-only; any non-overload repeat above Go's same-stage median×1.5
requires profiling. RSS is reported, belongs to separate fix-B and does not
determine this task's result. Capacity and actual-overload recovery are not
remeasured or newly accepted here.

## Measurement protocol

Exact prior Go binary/snapshot/prepared roles, fresh runtime copies/cold dumps,
audit on, SUT CPU0/helpers CPU1, GOMAXPROCS1/default GC. Same sender,128 lanes,
500ms from planned slot, >5ms-late slots skipped/no catch-up. Seeds501/502/503,
G/N,N/G,G/N pairs, each F then normal20/100/500. Warmup100QPS10s; stages30s
(last25s stable); tail100QPS60s. F has globally unique names, A70/AAAA30,
forced foreign trusted DoH/SOCKS with80ms HTTP/1.1 mock. Its tail continues
uncached load and is not an overload-recovery experiment. h2 concurrency is
covered by trusted local-peer tests, not by F's HTTP/1.1 performance result.

The Go oracle is unchanged. Before candidate measurement, native expectations
include the earlier approved AAAA cell and462 cells accepted in5D-1 Claude
review: exact prior three-pass evidence under the already frozen ASCII
case-insensitive `full:` no-v4/no-v6 contract. This is prospective and fixed;
candidate results are not inputs, and no DNS field/TTL/status class is waived.
All other cells retain Go's full-section functional reference. Public evidence
contains hashes/indices/numeric statistics only; names/configs/raw packets
remain in the private archive outside the repository.

## Results

**Fix-A measured gates PASS; external Claude acceptance pending.** All12
planned runs completed exit0, no formal rerun/invalidation.213600 planned,
213592 correct,8 unsent; no wrong/late/timeout/orphan responses across all runs.
The [fixed plan and hash receipts](matrix.json), [comparison](comparison.json),
[native F](native-f.json), [Go F controls](go-f.json), [native normal](native-normal.json)
and [Go normal controls](go-normal.json) retain all repetitions and scatter.

| Native stable stage | Median p95 / p99 ms | Frozen ceilings ms | Correctness / coverage each repeat | DNS wrong | Result |
|---|---:|---:|---|---:|---|
| F100QPS | 84.017 / 84.915 | 92.401 / 93.339 | 100% stable; all-stage minimum99.9667% | 0 | PASS |
| Normal20QPS | 13.109 / 13.794 | 14.025 / 15.168 | 100% | 0 | PASS |
| Normal100QPS | 11.996 / 13.060 | 13.853 / 15.006 | 100% | 0 | PASS |
| Normal500QPS | 11.163 / 12.496 | 12.333 / 13.866 | stable minimum99.992%; all-stage99.9933% | 0 | PASS |

F per-run p95=83.428/84.017/84.137, p99=84.527/85.141/84.915ms. Native F r2
has one unsent in first5s settling; normal r1 q500 one stable unsent; normal
r1/r2 tail one each. All remain in their original denominators. Go normal r1
q20 has one unsent:stable499/500=99.8% (all599/600=99.8333%), a disclosed fresh
control shortfall; no wrong replies, no invalidation/replacement. Go F r3 and
normal r2/r3 tails each have one unsent. Thus fresh Go controls are not described
as universally passing coverage. Freeze still uses the approved prior baseline.

## Connections, CPU and RSS

| F observation | Accepted5D-1 | Fix-A fresh measurement |
|---|---|---|
| Native SOCKS connections, r1/r2/r3 | 8886 / 8885 / 8886 | 10 / 10 / 9 |
| Go SOCKS connections, r1/r2/r3 | 10 / 10 / 11 | 12 / 11 / 10 |
| Native CPU µs/correct, median | 904 | 416 (range336–464) |
| Go CPU µs/correct, median | 408 | 392 |
| Native/Go CPU median ratio | 2.216 | 1.061 |
| Native p99 median ms | 96.644 (FAIL) | 84.915 (PASS) |

Native stable CPU falls54.0% from prior median. No native non-overload repeat
exceeds fresh Go's same-stage median×1.5, including warmup/tail:
[all trigger checks](cpu-trigger-checks.json). No new profiling required.
Connections are actual allowed SOCKS accepts, not a latency-based estimate.
Every F run has zero outer cache hits, only foreign:https DNS exchanges and
zero denied SOCKS requests; native exchange counts10000/9999/10000 match sent
queries. This supports removing repeated fresh handshakes; no exact p99
decomposition or h2 throughput claim is made.

| Stable native RSS median MiB | Native | Fresh Go |
|---|---:|---:|
| F | 54.73 | 183.10 |
| Normal20 | 43.61 | 192.65 |
| Normal100 | 46.11 | 194.68 |
| Normal500 | 57.44 | 197.22 |

Maximum sampled native startup/load RSS107.12MiB, normal-only95.69MiB. These
low-load observations fit128MiB/relative-to-Go and256MiB sampled-peak budgets,
but do not accept fix-B:capacity100k audit retention/high-load RSS and longer
stability remain separate.250ms sampling is not an allocator high-water mark.
CPU tick differences bracket stable samples and may be up to250ms shorter than
the window. Capacityfloor2k and actual-overload10s recovery are not re-proven.

## Cleanup and stop

All12 runs have successful owned-process/namespace/original-service receipts.
Private raw evidence is archived outside the repository before VM deletion;
see [archive integrity](archive.json), [prearchive check](prearchive-cleanup.json)
and [final cleanup](cleanup.json). Private archive SHA256
`2280294c7a84e7da8f6d8b0374b4375326642a40415f137f3bd8628fb71d00ab`;
all12 raw evidence sets verified before deleting the owned VM root/archive copy.
Original mosdns remains active/MainPID827437. No commit/push/deployment,
task archive, next task,5D2/3 or production replacement. Stop for user-mediated
Claude review; RSS fix-B and wider5D gates remain pending.

## VM implementation verification

SSH alias mosdns-rust only, offline/locked/jobs2; no local Rust build.
fmt --check and strict workspace/all-targets Clippy PASS. Full workspace
--no-fail-fast:1476 passed/0 failed/4 existing ignored,97 result groups including
empty doc tests. Final DoH core108, new concurrency8 and native-host composition
PASS. Tests cover both ALPN outcomes, independent responses, SOCKS-only
reachability, isolated cancellation, peer/local limits, h1 wait deadline,
GOAWAY versus ambiguous disconnect, no replay, idle expiration and drain.

The repository build script rebuilt both Vue bundles before offline release.
[Build manifest](build-manifest.json):candidate SHA256
`663662a0951dd9be0e2e763acf15fa363b88afa261eb81b5ec78596098ef4a3d`,
version5dfixa-cd0162cb. All224 Rust build inputs match the final worktree.
Original config_rust package native check exits0/gaps=0.
[Approved oracle provenance](approved-cells.json) pins prior evidence only.

Intermediate RED/failing/interrupted tests remain in the private archive:
old h1 busy failures/h2 extra connections, pre-SETTINGS optimism/GOAWAY
classification, superseded native-host two-connection assertion and revised
fixture's peer-join-before-owner-close blockage. Final checks supersede those
implementation failures; these are tests, not a self-review round.

## 外部审查结论（2026-10-10）

Claude 第 1 轮 **FINAL: PASS**（无 P0/P1）。场景 F 与常规档位在冻结阈值下全部通过；SOCKS 连接约 8,886 → 9–10，CPU/查询 904 → 416 µs。
容量与过载恢复未重测（改动只在 DoH 路径），由 5D-2/5D-3 覆盖；TCP/DoT 并发复用仍为后续项。
