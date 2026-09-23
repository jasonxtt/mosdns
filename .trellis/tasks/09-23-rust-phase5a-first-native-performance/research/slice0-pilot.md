# Slice 0 pilot and smoke evidence

Status: pilot complete; no official samples or frozen official manifest exist yet. Evidence remains on the test VM under `/root/mosdns-rust-phase5a-first-native-performance-605c305/`.

## Candidate and tool identity

| Artifact | Identity |
|---|---|
| Go-only source | `5b1eca69e0668ad1ddb6db88c0f39202557d5b98` |
| Go binary SHA-256 | `fece7ece823a1493eb1a495a472016cfa94df4470d48064fcef301668efdb137` |
| Rust source | `605c30577b79d397b5695618dbd2980e550ca6f3` |
| Rust native-host binary SHA-256 | `370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa` |
| Runner SHA-256 | `4432d680080a3a015d6f6cb699c399ebfd39b7531180c02553f592de8659cc1c` |
| Helper source SHA-256 | `c8fd88e5d5534487088136ea3615def6d29b6ee12a0620e80d331d0acae31118` |
| Final helper (`phase5a-baseline-helper/v6`) SHA-256 | `ac47723376d8f115ca260b6d11224ab0350a7bfed5fcd4793e398c537de450e5` |

The recorded build commands are in the VM task root's `build-commands.txt`. Go is built with `CGO_ENABLED=0`, `GOOS=linux`, `GOARCH=amd64`, and empty build tags. Rust is built with `--release --locked` for package `mosdns-native-host`, binary `mosdns`.

## Frozen corpus hash check

All seven inputs match the archived baseline report:

| Input | SHA-256 |
|---|---|
| `configs/cache.yaml` | `7f521ae011c28b4e102ff75437ebb943ba11f4bdc09aa656c41290ccde4b61c7` |
| `configs/forward-tcp.yaml` | `1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1` |
| `configs/forward-udp.yaml` | `f6758fb2b2eb8056d6d1c2aef320a3357d005b92685136dec457d5543bd9d729` |
| `configs/routing.yaml` | `66f358dbe2315df2aab1ad81668cec980b2f11aa68f863acad522c8e0c9fc651` |
| `workloads/cache.jsonl` | `7680cd55ccf477972cd700c05e72c83c52ab1a55643b9a98bb610629035214ed` |
| `workloads/forward.jsonl` | `32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2` |
| `workloads/routing.jsonl` | `dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1` |

## VM and placement

The VM is `ssh mosdns-rust`, Linux amd64, 2 online CPUs, 4,102,578,176 bytes RAM, ext4 task storage, Go `go1.24.4`, Rust `1.95.0`, and open-file limit 1024. SUT processes ran on CPU 0; the pinned runner, generator, and fixtures ran on CPU 1. Recorded process affinity masks were disjoint. No task-owned listener remained after the final smoke; all ten TCP/UDP loopback task ports could be rebound with a listener-safe `SO_REUSEADDR` probe.

## Bilateral smoke

The final v6 helper passed W1 UDP, W1 TCP, W2 cold/prefill/warm, and W3 for both Go and Rust. The final result root is `/root/mosdns-rust-phase5a-first-native-performance-605c305/results/smoke-v6-final/`; it has 12 stage JSONL files because W2 has separate cold, prefill, and warm evidence, and has no `invalid-stages.tsv` files. The console logs contain the sender, strict DNS, W2 counter/TTL, and W3 route-event checks.

## Fixed-rate pilot

`results/pilot-final-v2/` contains 24 candidate attempts: four scenarios × three repetitions × two candidates. The fixed ladder was 200/400/800/1,000 QPS, each stage 3 seconds, 500 ms deadline, 100 ms late drain, 30-second W2 fixture TTL with 500 ms safety margin, CPU 0 for SUT and CPU 1 for harness/fixtures. Candidate order alternated Go→Rust, Rust→Go, Go→Rust.

Every one of the 24 normal-reference rows completed 600/600 scheduled queries correctly on time. All 24 common-load rows also completed as scheduled. Four candidate runs have a sender-shortfall stage; these and their downstream recovery rows remain retained and invalid:

| Scenario / repetition / candidate | Invalid stage | Evidence |
|---|---|---|
| W1 TCP / 1 / Go | near-saturation, recovery | 2,399/2,400 sent at 800 QPS; continuous recovery gate failed |
| W1 TCP / 3 / Go | near-saturation, recovery | 2,399/2,400 sent at 800 QPS; continuous recovery gate failed |
| W2 / 2 / Go | overload, recovery | 2,999/3,000 sent at 1,000 QPS; continuous recovery gate failed |
| W3 / 1 / Rust | overload, recovery | 2,999/3,000 sent at 1,000 QPS; continuous recovery gate failed |

The earlier initial v5 W1 UDP Go attempt that dropped two slots is also preserved under `results/pilot-final/`; it was not substituted for any row in `pilot-final-v2`.

Each candidate has three clean reference samples per scenario. The frozen ceiling candidates are the maximum p95/p99 across both candidates' six reference samples; minimum recovery samples are 600:

| Scenario | p95 ceiling (µs) | p99 ceiling (µs) |
|---|---:|---:|
| W1 UDP | 252 | 336 |
| W1 TCP | 374 | 499 |
| W2 warm | 171 | 249 |
| W3 | 399 | 496 |

W2 warm completed 7,799–7,800 successful measured requests per session. Maximum per-key prefill-to-final-response age was 18.368 seconds, within the 29.5-second TTL eligibility window.

Observed pilot resource maxima across roles and runs were approximately 13.0% of one CPU for the SUT, 9.3% for the load generator, and 6.7% summed across fixtures. Peak RSS was 61,076 KiB for the SUT, 17,708 KiB for the load generator, and 15,924 KiB for an individual fixture; maximum observed FDs were 11 for SUT/generator and 7 per fixture. W3 journals were about 1.33 MB per run and did not overflow. Pilot result artifacts use 74 MB on disk.

At 1,000 QPS the SUT remained far below a saturated core; most high-rate stages were valid and p95/p99 did not repeatedly exceed the combined reference ceilings. One W1 TCP Go repetition had p99 605 µs against a 499 µs reference ceiling, but this was not repeated. The pilot therefore did **not** establish a stable valid overload point. Treat 800/1,000 QPS as near-saturation/high-rate probes, keep the ladder fixed for both candidates, and report capacity and service-recovery claims as indeterminate unless official evidence independently demonstrates overload. Do not interpret a passing same-process low-rate recovery check as proof of service recovery after overload.

No official manifest has been frozen, and no official measurement has started. This evidence does not authorize production deployment or a multi-core capacity conclusion.
