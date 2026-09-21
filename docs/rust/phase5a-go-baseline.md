# Phase 5A Go-only whole-process baseline

Status: **accepted baseline candidate pending final task review**
Date: 2026-09-21
Task: `.trellis/tasks/09-21-rust-phase5a-baseline`

This document records the frozen Go-only baseline for the first Phase 5A
comparison corpus. It is controlled-local evidence on Linux amd64, not a
Rust performance result and not a claim of full Phase 5A compatibility. The
work stops at this report and acceptance; it does not authorize a Rust native
host, transport work, production deployment, API/WebUI work, or a generic
benchmark platform.

## Result at a glance

The official matrix contains 3 repetitions × 3 offered rates × 5 measured
stages: W1 UDP, W1 TCP, W2 cold, W2 warm, and W3 routing. There are 36 final
run directories and 45 measured stage rows. Every final stage has:

- scheduled = sent = received = correct-on-time;
- zero wrong-response, protocol, transport, timeout, and sender-shortfall
  counters;
- p50 ≤ p95 ≤ p99 latency samples;
- eleven independent SUT `/proc` CPU/RSS samples;
- valid W2 upstream-counter deltas and W3 route-counter evidence.

The effective throughput equals the offered rate at all frozen points (5, 10,
and 20 QPS). These values are bounded fixed-rate observations, not capacity
limits.

## Identity and frozen inputs

The source anchor is `e70a2408e2dcd2141e48bcc84765c5adfa406fe4`. The final
official build/source identity is recorded in
`.trellis/tasks/09-21-rust-phase5a-baseline/research/results/official-20260921/environment-frozen.json`:

```text
source_git_sha: afa071f1cb2fd05bf2f3727ffaa706715019526a
SUT git_sha:    5b1eca69e0668ad1ddb6db88c0f39202557d5b98
SUT SHA-256:    50ff4ff29a2ac23ede0dc8722672c428ceb441dde0b98bf47032e709427a3183
Go:             go1.26.5 linux/amd64
GOOS/GOARCH:    linux/amd64
CGO_ENABLED:    0
GO_TAGS:        empty
Rust selectors: none
task revision:   1072824b1878ec07f0dd6cecde2a1bea81c5dd11
```

The build command was:

```text
SKIP_UI_BUILD=1 CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GO_TAGS= BUILD_VERSION=phase5a-go-5b1eca6 OUTPUT=<isolated-output>/mosdns-go ./scripts/build-local.sh
```

Frozen input hashes, all SHA-256:

| Input | SHA-256 |
|---|---|
| `tests/phase5a-baseline/configs/forward-udp.yaml` | `f6758fb2b2eb8056d6d1c2aef320a3357d005b92685136dec457d5543bd9d729` |
| `tests/phase5a-baseline/configs/forward-tcp.yaml` | `1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1` |
| `tests/phase5a-baseline/configs/cache.yaml` | `7f521ae011c28b4e102ff75437ebb943ba11f4bdc09aa656c41290ccde4b61c7` |
| `tests/phase5a-baseline/configs/routing.yaml` | `66f358dbe2315df2aab1ad81668cec980b2f11aa68f863acad522c8e0c9fc651` |
| `tests/phase5a-baseline/workloads/forward.jsonl` | `32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2` |
| `tests/phase5a-baseline/workloads/cache.jsonl` | `7680cd55ccf477972cd700c05e72c83c52ab1a55643b9a98bb610629035214ed` |
| `tests/phase5a-baseline/workloads/routing.jsonl` | `dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1` |
| `scripts/run-phase5a-baseline.sh` | `977658509360ee3b17ef44933a22afd7a8272b81351e2322a34b6657d2b27a33` |
| `tests/phase5a-baseline/cmd/phase5a-baseline/main.go` | `0b53a45a0b6035ff806282d8655e91a561b30d2eac50d7a9d651661aa228d309` |
| `go.mod` | `c00a26a0b60448ce0ffb45eb64ccd7ed3150ba69a675691479ea24729dae4728` |
| `go.sum` | `312cc28ef8d6c3c1c492f4b60b1a90e6f5c41d84a5a95fd1b348488cfd195072` |

The frozen manifest is
`.trellis/tasks/09-21-rust-phase5a-baseline/research/run-manifest.json`; its
SHA-256 is
`a5cd4d791ca9a88f4a1217e86f5b46b89d71625d344263c222f8eab0a797d8d7`.
Official runs were required to reject a manifest hash mismatch.

## Environment and procedure

The historical official execution used an isolated Linux amd64 container in a
Colima QEMU Linux VM. The container had 4 CPUs, 6 GiB memory, and file
descriptor limit 1024. CPU affinity was authoritative: SUT `0,1`, harness
`2,3`. The image was `golang:1.26-bookworm`; kernel was
`6.8.0-117-generic` on `x86_64`. Runtime variables were
`GOMAXPROCS=2`, `GOGC=100`, unset `GOMEMLIMIT`, and
`MOSDNS_CONFIG_PACKAGE_URL=/dev/null`.

This is historical evidence already collected and retained in the repository.
The current execution workflow must not start a local VM; any future rerun
must use the user-designated `ssh mosdns-rust` environment or another
explicitly approved Linux environment. The current SSH host is not a
substitute for the frozen evidence: its observed machine has 2 CPUs and
Go 1.24.4, while the frozen manifest requires the recorded 4-CPU/Go 1.26.5
environment.

The official manifest fixed:

```text
scenario order:        w1-udp, w1-tcp, w2, w3
repetitions:           3
offered QPS:           5, 10, 20
measured stage:        10 seconds
warmup:                0 seconds
request deadline:      500 ms
late drain:            100 ms
fixture delay:         0 ms
TCP policy:            fresh-connection-per-request
SUT startup margin:    3 seconds
```

The runner is `scripts/run-phase5a-baseline.sh`. It accepts a replacement
binary through `MOSDNS_BINARY`, validates and records its SHA-256, and does
not implicitly rebuild the SUT. The exact shape for a future official rerun is:

```text
MOSDNS_BINARY=/absolute/path/to/mosdns-go \
SCENARIO=w1-udp \
RUN_MODE=official \
OFFERED_QPS=5 \
MANIFEST_SHA256=a5cd4d791ca9a88f4a1217e86f5b46b89d71625d344263c222f8eab0a797d8d7 \
RESULT_DIR=/absolute/path/to/run-dir \
  bash scripts/run-phase5a-baseline.sh
```

Repeat that command with the frozen scenario/QPS pairs and a distinct result
directory; the runner interface is environment-variable based, not
positional arguments. `SUT_CPU_SET`, `HARNESS_CPU_SET`, and `HELPER_BINARY`
are set by the approved environment when required.

The unchanged Go SUT was also copied to a second executable path and passed
the same W1-UDP smoke, proving that the runner does not depend on a particular
build-path name or hidden Go implementation wiring.

The Go helper uses an open-loop fixed-rate sender. Useful throughput includes
only correct responses received before the deadline; late, wrong, protocol,
transport, timeout, and sender-shortfall results remain separate counters.
CPU/query below is the SUT user+system CPU-seconds delta divided by
correct-on-time responses. RSS median and peak are KiB over the eleven
stage-local SUT `/proc` samples.

## Scenario contract and correctness

W1 UDP and W1 TCP exercise minimal forwarding through controlled loopback
upstreams. W1 includes positive and expected-negative fixed cases; the
negative cases are tracked separately and are not counted as errors when the
expected negative response is correct. TCP uses a fresh connection per
request, as frozen in the manifest.

W2 measures two separate lifecycles. Cold starts from an empty cache and
requires one exact upstream-counter increment per committed hot case. Warm
performs one unmeasured hot-set prefill, verifies its exact counter delta,
takes the counter barrier, and then measures the same hot set; the measured
warm delta must be zero.

W3 checks final DNS correctness and route identity. The fixture-counter gate
requires `DOMAIN_HIT = A`, `IP_RULE_HIT = B→A`, and `IP_RULE_MISS = B→C`.
The helper also has a deliberate tampered-counter regression that rejects a
missing route leg.

Controlled upstreams are local-only and deterministic. Fixture counters are
updated in memory on the query path, snapshots are serialized, and the final
snapshot is copied only after graceful fixture shutdown and final flush.

## Official stage results

`Exp−` is expected-negative responses on time. `Eff` is effective
correct-on-time throughput in QPS. The last column is
`late/wrong/proto/trans/to/short`.

| Rep | QPS | Stage | Sched | Sent | Rcv | COT | Exp− | Eff | p50 us | p95 us | p99 us | CPU/query | RSS med KiB | RSS peak KiB | Samples | late/wrong/proto/trans/to/short |
|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| 1 | 5 | w1-udp | 50 | 50 | 50 | 50 | 16 | 5 | 4129 | 17010 | 35567 | 0.004400 | 20612 | 20620 | 11 | 0/0/0/0/0/0 |
| 1 | 5 | w1-tcp | 50 | 50 | 50 | 50 | 16 | 5 | 7799 | 13740 | 37561 | 0.007600 | 21284 | 21440 | 11 | 0/0/0/0/0/0 |
| 1 | 5 | w2-cold | 50 | 50 | 50 | 50 | 0 | 5 | 4566 | 13382 | 27028 | 0.003600 | 25128 | 25368 | 11 | 0/0/0/0/0/0 |
| 1 | 5 | w2-warm | 50 | 50 | 50 | 50 | 0 | 5 | 3570 | 26583 | 97491 | 0.003000 | 24552 | 24600 | 11 | 0/0/0/0/0/0 |
| 1 | 5 | w3 | 50 | 50 | 50 | 50 | 0 | 5 | 4822 | 10862 | 33849 | 0.004000 | 20180 | 20288 | 11 | 0/0/0/0/0/0 |
| 1 | 10 | w1-udp | 100 | 100 | 100 | 100 | 33 | 10 | 6228 | 15685 | 25578 | 0.004700 | 20868 | 21076 | 11 | 0/0/0/0/0/0 |
| 1 | 10 | w1-tcp | 100 | 100 | 100 | 100 | 33 | 10 | 8051 | 14926 | 55461 | 0.009400 | 21396 | 21692 | 11 | 0/0/0/0/0/0 |
| 1 | 10 | w2-cold | 100 | 100 | 100 | 100 | 0 | 10 | 5288 | 11815 | 19439 | 0.002800 | 24944 | 25024 | 11 | 0/0/0/0/0/0 |
| 1 | 10 | w2-warm | 100 | 100 | 100 | 100 | 0 | 10 | 4007 | 7678 | 11707 | 0.002200 | 24448 | 24488 | 11 | 0/0/0/0/0/0 |
| 1 | 10 | w3 | 100 | 100 | 100 | 100 | 0 | 10 | 5044 | 23282 | 35254 | 0.004500 | 21476 | 21688 | 11 | 0/0/0/0/0/0 |
| 1 | 20 | w1-udp | 200 | 200 | 200 | 200 | 66 | 20 | 3503 | 6803 | 8830 | 0.002850 | 20968 | 21236 | 11 | 0/0/0/0/0/0 |
| 1 | 20 | w1-tcp | 200 | 200 | 200 | 200 | 66 | 20 | 6289 | 11059 | 13679 | 0.006200 | 20972 | 21536 | 11 | 0/0/0/0/0/0 |
| 1 | 20 | w2-cold | 200 | 200 | 200 | 200 | 0 | 20 | 2527 | 14945 | 31595 | 0.001650 | 25108 | 25208 | 11 | 0/0/0/0/0/0 |
| 1 | 20 | w2-warm | 200 | 200 | 200 | 200 | 0 | 20 | 3380 | 8338 | 12655 | 0.002200 | 25164 | 25264 | 11 | 0/0/0/0/0/0 |
| 1 | 20 | w3 | 200 | 200 | 200 | 200 | 0 | 20 | 3825 | 7025 | 20362 | 0.003150 | 21300 | 21720 | 11 | 0/0/0/0/0/0 |
| 2 | 5 | w1-udp | 50 | 50 | 50 | 50 | 16 | 5 | 4553 | 11786 | 42118 | 0.003400 | 20640 | 20700 | 11 | 0/0/0/0/0/0 |
| 2 | 5 | w1-tcp | 50 | 50 | 50 | 50 | 16 | 5 | 7878 | 32258 | 227945 | 0.014000 | 20868 | 21028 | 11 | 0/0/0/0/0/0 |
| 2 | 5 | w2-cold | 50 | 50 | 50 | 50 | 0 | 5 | 3977 | 7826 | 42710 | 0.002400 | 25072 | 25108 | 11 | 0/0/0/0/0/0 |
| 2 | 5 | w2-warm | 50 | 50 | 50 | 50 | 0 | 5 | 3220 | 7251 | 10001 | 0.001800 | 25140 | 25148 | 11 | 0/0/0/0/0/0 |
| 2 | 5 | w3 | 50 | 50 | 50 | 50 | 0 | 5 | 5864 | 20500 | 56264 | 0.005000 | 20760 | 20776 | 11 | 0/0/0/0/0/0 |
| 2 | 10 | w1-udp | 100 | 100 | 100 | 100 | 33 | 10 | 5083 | 10273 | 14188 | 0.003700 | 20684 | 20772 | 11 | 0/0/0/0/0/0 |
| 2 | 10 | w1-tcp | 100 | 100 | 100 | 100 | 33 | 10 | 7528 | 15261 | 21369 | 0.007500 | 20880 | 21192 | 11 | 0/0/0/0/0/0 |
| 2 | 10 | w2-cold | 100 | 100 | 100 | 100 | 0 | 10 | 3296 | 14188 | 25671 | 0.002200 | 25548 | 25604 | 11 | 0/0/0/0/0/0 |
| 2 | 10 | w2-warm | 100 | 100 | 100 | 100 | 0 | 10 | 5109 | 10979 | 20855 | 0.002900 | 24960 | 24992 | 11 | 0/0/0/0/0/0 |
| 2 | 10 | w3 | 100 | 100 | 100 | 100 | 0 | 10 | 4873 | 7902 | 17222 | 0.003800 | 21312 | 21428 | 11 | 0/0/0/0/0/0 |
| 2 | 20 | w1-udp | 200 | 200 | 200 | 200 | 66 | 20 | 3718 | 20032 | 30344 | 0.003000 | 20576 | 20856 | 11 | 0/0/0/0/0/0 |
| 2 | 20 | w1-tcp | 200 | 200 | 200 | 200 | 66 | 20 | 5723 | 9284 | 19556 | 0.006050 | 21220 | 21824 | 11 | 0/0/0/0/0/0 |
| 2 | 20 | w2-cold | 200 | 200 | 200 | 200 | 0 | 20 | 2389 | 6387 | 11178 | 0.001950 | 25436 | 25540 | 11 | 0/0/0/0/0/0 |
| 2 | 20 | w2-warm | 200 | 200 | 200 | 200 | 0 | 20 | 2317 | 5771 | 10879 | 0.001950 | 24948 | 25208 | 11 | 0/0/0/0/0/0 |
| 2 | 20 | w3 | 200 | 200 | 200 | 200 | 0 | 20 | 3483 | 9233 | 22205 | 0.002800 | 21384 | 21768 | 11 | 0/0/0/0/0/0 |
| 3 | 5 | w1-udp | 50 | 50 | 50 | 50 | 16 | 5 | 5156 | 9059 | 44570 | 0.003800 | 20792 | 20860 | 11 | 0/0/0/0/0/0 |
| 3 | 5 | w1-tcp | 50 | 50 | 50 | 50 | 16 | 5 | 14216 | 28641 | 57689 | 0.014400 | 20856 | 21020 | 11 | 0/0/0/0/0/0 |
| 3 | 5 | w2-cold | 50 | 50 | 50 | 50 | 0 | 5 | 7523 | 20567 | 27681 | 0.004200 | 25348 | 25356 | 11 | 0/0/0/0/0/0 |
| 3 | 5 | w2-warm | 50 | 50 | 50 | 50 | 0 | 5 | 2660 | 5310 | 7418 | 0.001600 | 24876 | 24920 | 11 | 0/0/0/0/0/0 |
| 3 | 5 | w3 | 50 | 50 | 50 | 50 | 0 | 5 | 4816 | 10303 | 38552 | 0.004200 | 21116 | 21248 | 11 | 0/0/0/0/0/0 |
| 3 | 10 | w1-udp | 100 | 100 | 100 | 100 | 33 | 10 | 4078 | 9706 | 34088 | 0.003500 | 21316 | 21468 | 11 | 0/0/0/0/0/0 |
| 3 | 10 | w1-tcp | 100 | 100 | 100 | 100 | 33 | 10 | 6998 | 13343 | 16178 | 0.007300 | 20904 | 21168 | 11 | 0/0/0/0/0/0 |
| 3 | 10 | w2-cold | 100 | 100 | 100 | 100 | 0 | 10 | 2855 | 27852 | 37013 | 0.001900 | 24944 | 25168 | 11 | 0/0/0/0/0/0 |
| 3 | 10 | w2-warm | 100 | 100 | 100 | 100 | 0 | 10 | 2675 | 4048 | 7738 | 0.001600 | 24508 | 24796 | 11 | 0/0/0/0/0/0 |
| 3 | 10 | w3 | 100 | 100 | 100 | 100 | 0 | 10 | 4986 | 11009 | 28238 | 0.004100 | 21684 | 21792 | 11 | 0/0/0/0/0/0 |
| 3 | 20 | w1-udp | 200 | 200 | 200 | 200 | 66 | 20 | 3302 | 14150 | 35033 | 0.002950 | 21156 | 21448 | 11 | 0/0/0/0/0/0 |
| 3 | 20 | w1-tcp | 200 | 200 | 200 | 200 | 66 | 20 | 5863 | 10263 | 22558 | 0.005650 | 22004 | 22648 | 11 | 0/0/0/0/0/0 |
| 3 | 20 | w2-cold | 200 | 200 | 200 | 200 | 0 | 20 | 3547 | 5635 | 12171 | 0.002350 | 25120 | 25244 | 11 | 0/0/0/0/0/0 |
| 3 | 20 | w2-warm | 200 | 200 | 200 | 200 | 0 | 20 | 2586 | 5689 | 10947 | 0.001750 | 25372 | 25472 | 11 | 0/0/0/0/0/0 |
| 3 | 20 | w3 | 200 | 200 | 200 | 200 | 0 | 20 | 4013 | 5741 | 16356 | 0.003000 | 21248 | 21544 | 11 | 0/0/0/0/0/0 |

## Cross-repetition spread

The following table is the mean of the three repetition rows at each offered
rate and stage. The p99 and CPU columns retain the min–max range across the
three repetitions; RSS columns show the per-repetition range.

| QPS | Stage | Reps | p50 mean us | p95 mean us | p99 mean us (range) | CPU/query mean (range) | RSS median range KiB | RSS peak range KiB |
|---:|---|---:|---:|---:|---:|---:|---:|---:|
| 5 | w1-udp | 3 | 4612.7 | 12618.3 | 40751.7 (35567–44570) | 0.003867 (0.003400–0.004400) | 20612–20792 | 20620–20860 |
| 5 | w1-tcp | 3 | 9964.3 | 24879.7 | 107731.7 (37561–227945) | 0.012000 (0.007600–0.014400) | 20856–21284 | 21020–21440 |
| 5 | w2-cold | 3 | 5355.3 | 13925.0 | 32473.0 (27028–42710) | 0.003400 (0.002400–0.004200) | 25072–25348 | 25108–25368 |
| 5 | w2-warm | 3 | 3150.0 | 13048.0 | 38303.3 (7418–97491) | 0.002133 (0.001600–0.003000) | 24552–25140 | 24600–25148 |
| 5 | w3 | 3 | 5167.3 | 13888.3 | 42888.3 (33849–56264) | 0.004400 (0.004000–0.005000) | 20180–21116 | 20288–21248 |
| 10 | w1-udp | 3 | 5129.7 | 11888.0 | 24618.0 (14188–34088) | 0.003967 (0.003500–0.004700) | 20684–21316 | 20772–21468 |
| 10 | w1-tcp | 3 | 7525.7 | 14510.0 | 31002.7 (16178–55461) | 0.008067 (0.007300–0.009400) | 20880–21396 | 21168–21692 |
| 10 | w2-cold | 3 | 3813.0 | 17951.7 | 27374.3 (19439–37013) | 0.002300 (0.001900–0.002800) | 24944–25548 | 25024–25604 |
| 10 | w2-warm | 3 | 3930.3 | 7568.3 | 13433.3 (7738–20855) | 0.002233 (0.001600–0.002900) | 24448–24960 | 24488–24992 |
| 10 | w3 | 3 | 4967.7 | 14064.3 | 26904.7 (17222–35254) | 0.004133 (0.003800–0.004500) | 21312–21684 | 21428–21792 |
| 20 | w1-udp | 3 | 3507.7 | 13661.7 | 24735.7 (8830–35033) | 0.002933 (0.002850–0.003000) | 20576–21156 | 20856–21448 |
| 20 | w1-tcp | 3 | 5958.3 | 10202.0 | 18597.7 (13679–22558) | 0.005967 (0.005650–0.006200) | 20972–22004 | 21536–22648 |
| 20 | w2-cold | 3 | 2821.0 | 8989.0 | 18314.7 (11178–31595) | 0.001983 (0.001650–0.002350) | 25108–25436 | 25208–25540 |
| 20 | w2-warm | 3 | 2761.0 | 6599.3 | 11493.7 (10879–12655) | 0.001967 (0.001750–0.002200) | 24948–25372 | 25208–25472 |
| 20 | w3 | 3 | 3773.7 | 7333.0 | 19641.0 (16356–22205) | 0.002983 (0.002800–0.003150) | 21248–21384 | 21544–21768 |

The variability is itself part of the baseline. In particular, TCP at 5 QPS
has a wide p99 spread, so this report must not be read as a single latency
SLO or capacity claim.

## Retained invalid history

The runner never overwrote failed attempts. The final evidence root retains
the following non-official history:

- `r1-*`, `r2-*`, and `r3-*`: invalid first attempts because the minimal
  container did not contain `rg`.
- `final-*`: six invalid attempts, including q10/r2 W1 UDP, q10/r3 W1 TCP,
  q10/r3 W3, q20/r3 W1 UDP, q20/r3 W2, and q20/r3 W3. These failed before a
  valid final stage because of transport/readiness/config-package startup
  issues; they are not in the frozen matrix.
- `replacement-*`: invalid methodology because W2 warm was one-pass before
  the fixed-rate warm-stage correction.
- Pilot roots: 100 QPS W1 UDP had transport errors; 50 QPS W2 cold had an
  inconsistent counter delta. The 5/10/20 QPS pilot range was retained after
  the startup-margin and offline-config corrections.

`environment-final.json`, where present, is an older superseded metadata
artifact and is not authoritative. `environment-frozen.json` and the frozen
manifest are the authoritative identity records.

## Raw evidence locations

The authoritative raw evidence is under
`.trellis/tasks/09-21-rust-phase5a-baseline/research/results/official-20260921/`.
Each `frozen-*` directory contains `environment.json`, `run-metadata.txt`,
`manifest.sha256`, `sut.json`, `fixture.json`, fixture counter snapshots,
`stages.jsonl`, and `resource-samples.jsonl`, plus retained SUT logs. The
top-level `environment-frozen.json` records the build, platform, module
hashes, runtime environment, affinity, and manifest hash.

The executable reproduction entry point is
`scripts/run-phase5a-baseline.sh`; the fixed YAML and JSONL corpus is under
`tests/phase5a-baseline/`. A future Go/Rust comparison must run both binaries
in the same approved Linux environment and session, with the same manifest
and fixed inputs. It must not compare this historical Go result with a
different host and call that a binary-only comparison.

## PRD acceptance mapping

| PRD item | Evidence in this task |
|---|---|
| A1: planning/scope integrity | Exact task diff is limited to the baseline report, task records, frozen fixtures/tooling, and retained evidence; product-path audit is clean and module manifests are unchanged. |
| A2: Go-only identity is auditable | Source anchor/task revision, build command, Go/toolchain identity, SUT hash, `CGO_ENABLED=0`, empty selectors, module hashes, and product-path audit are recorded above. |
| A3: three workload groups are frozen | Four YAML fixtures cover W1 UDP/TCP, W2 cache, and W3 routing; three JSONL workloads and all hashes are recorded in the manifest. |
| A4: controlled upstream evidence | Loopback UDP/TCP fixtures, deterministic counters, exact W2/W3 path checks, graceful final flush, and cleanup evidence are recorded. No public DNS is used. |
| A5: replaceable binary contract | `MOSDNS_BINARY` is explicit, the hash is recorded, no implicit rebuild occurs, and the same Go binary passed the unchanged W1-UDP smoke from a second executable path. |
| A6: correctness is part of throughput | DNS identity/rcode/answer validation, expected negatives, W2 deltas, W3 route legs, and separate correct-on-time/error counters define useful throughput. |
| A7: fixed-rate evidence | Open-loop fixed-rate scheduling and the frozen 10-second QPS ladder are recorded; sender shortfall is a dedicated counter. |
| A8: latency evidence | Raw latency samples and p50/p95/p99 are recorded for all 45 measured rows together with their failure counters. |
| A9: CPU/RSS evidence | Independent SUT `/proc` sampling provides 11 samples per stage, CPU/query, RSS median, and RSS peak. |
| A10: repetition/no cherry-pick | Three repetitions at each of three offered rates and five stages are retained and summarized; invalid history is retained with reasons rather than silently selected away. |
| A11: reproducible Linux amd64 report | The report records environment, commands, hashes, fixed inputs, raw locations, limitations, and the exact environment-variable rerun using `MOSDNS_BINARY`. |
| A12: no overclaim | The result is explicitly Go-only controlled-local Linux amd64 evidence; it makes no Rust performance, full 5A compatibility, public-network, or production-readiness claim. |
| A13: stop boundary | The closure statement and task exit gate prohibit native-host implementation, next-task start, production wiring, and deployment after acceptance. |

## Scope and closure statement

This task changed only baseline fixtures/tooling, raw baseline evidence, task
artifacts, and this report. `go.mod` and `go.sum` were not changed. No public
DNS dependency, production deployment, API/WebUI behavior, Rust host code,
transport feature, or feature-coverage ownership decision was introduced.
The result is the frozen Go-only baseline requested before native-host work.
After final task review passes, the task is archived and work stops; no
Phase 5A native-host task is created or started under this authorization.
