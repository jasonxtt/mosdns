# Slice 0 pilot and smoke evidence

Status: v6 evidence is historical. v7 bilateral smoke and the full 24-attempt pilot are complete; v8 bilateral smoke and the failed-health-check assessment check pass; v9's environment metadata probe confirms the Go toolchain fields. Local validation passed; exact-scope commit and Slice 0 re-review are pending. No official samples or frozen official manifest exist. Full result hashes are indexed in `slice0-v7-v9-evidence-index.sha256`; raw results remain on the test VM under `/root/mosdns-rust-phase5a-first-native-performance-605c305/`.

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

The VM is `ssh mosdns-rust`, Linux amd64, 2 online CPUs, 4,102,578,176 bytes RAM, ext4 task storage, effective Go `go1.26.4`, Rust `1.95.0`, and open-file limit 1024. `/usr/bin/go version` reports the base launcher as `go1.24.4`, but the source-tree `go` command selects Go `1.26.4`; `go version -m` on the Go candidate and v6 helper confirms they were built with `go1.26.4`. Future records use the compiler/runtime embedded in the binary, not the base launcher version. SUT processes ran on CPU 0; the pinned runner, generator, and fixtures ran on CPU 1. Recorded process affinity masks were disjoint. No task-owned listener remained after the final v6 smoke; all ten TCP/UDP loopback task ports could be rebound with a listener-safe `SO_REUSEADDR` probe.

## v6 historical bilateral smoke

The historical v6 helper passed W1 UDP, W1 TCP, W2 cold/prefill/warm, and W3 for both Go and Rust. The result root is `/root/mosdns-rust-phase5a-first-native-performance-605c305/results/smoke-v6-final/`; it has 12 stage JSONL files because W2 has separate cold, prefill, and warm evidence, and has no `invalid-stages.tsv` files. Slice 0 review later found that v6 could redistribute duplicate events across repeated same-question requests, so this smoke does not satisfy the strengthened v7 W3 acceptance gate. The v6 console logs and artifacts remain unchanged.

## v6 historical fixed-rate pilot

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

At 1,000 QPS the SUT remained far below a saturated core; most high-rate stages were valid and p95/p99 did not repeatedly exceed the combined reference ceilings. One W1 TCP Go repetition had p99 605 µs against a 499 µs reference ceiling, but this was not repeated. The pilot did **not** establish a stable valid overload point. Treat 800/1,000 QPS as near-saturation/high-rate probes and keep the ladder fixed for both candidates. This task's service-recovery assessment is categorically indeterminate; future recovery claims need a new task and an objective overload rule frozen before measurement.

No official manifest has been frozen, and no official measurement has started. These historical v6 results do not authorize production deployment or a multi-core capacity conclusion.

## v7 pilot, v8 smoke, and v9 provenance evidence

### Tool and candidate hashes

The candidate binaries used for v7/v8 Slice 0 checks remain the VM builds recorded above; official work will rebuild both candidates after Slice 0 review and freeze the new hashes before the first official sample.

| Artifact | SHA-256 / identity |
|---|---|
| Go-only candidate binary | `fece7ece823a1493eb1a495a472016cfa94df4470d48064fcef301668efdb137` |
| Rust-native candidate binary | `370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa` |
| v7 runner | `46cefe245a0ff2830dc2b6c65d0a55e14aa2fbfc517854d4034031fcad2493c8` |
| v7 helper source / tests / binary | `e981dbed698b25bf3aacc7d3f7f1f14365779d542e27983088271db38f67d85c` / `b16a4c877d66b973701da7e0ad8f3133c008ff315a18f15a2b1e2a2c579938ca` / `665a3affa24f7b8027c75121670e7c71619c3891d5c1dba4694895b14067b910` |
| v8 runner / helper source / tests / binary | `8c121380e93e5942f627587a869cbf362da5dd615162398470b03c73aea9bcf1` / `2dec4788eaa3dad251061e380cbd8c32c9122285b852acf8b9a0b2eb81f64da1` / `8e673e2965caa17919cb158118363534b00c757a52f4fd37342faa428b1eb043` / `13ee620fc38c0b0827c75121670e7c71619c3891d5c1dba4694895b14067b910` |
| v9 runner / v8 helper binary | `18c1e29cdcf23694a458899c26850414fd456529a84349f3a2884b06caf2245e` / `13ee620fc38c0b0827c75121670e7c71619c3891d5c1dba4694895b14067b910` |
| All v7-v9 result files index | `slice0-v7-v9-evidence-index.sha256`, SHA-256 `1fb5bea02cab31cbedbcfb4bbab2499ea0f87b8267e47607e248c35ca4ba7582` |

All helper builds use the selected Go `1.26.4` toolchain on the Linux amd64 VM. Candidate module metadata reports Go `1.26.4` for the Go binary; Rust is `1.95.0`. The old v7 per-run `environment.txt` rows captured the base `/usr/bin/go` launcher (`1.24.4`) because the runner recorded Go outside the module root; they are historical and not the toolchain identity. The v9 runner now distinguishes `go_base_launcher`, `go_project_selected_toolchain`, `go_helper_build_toolchain`, and `go_candidate_build_toolchain`. Its Go and Rust environment probes show project/helper `1.26.4`, Go candidate `1.26.4`, Go candidate `not-applicable` for the Rust run, and Rust toolchain `1.95.0`.

### Bilateral v7 and v8 smoke

The v7 helper passed all eight candidate/scenario smoke runs: Go and Rust across W1 UDP, W1 TCP, W2 cold/prefill/warm, and W3. The strengthened W3 event-time oracle accepted all smoke requests and rejected none. v8 repeated the same eight combinations with all passing and no `invalid-stages.tsv`. Both versions pinned the SUT to CPU 0; v8 teardown left no task process/listener, and a listener-safe TCP/UDP bind check succeeded on all ten task ports. Status copies are `slice0-smoke-v7-status.tsv` and `slice0-smoke-v8-status.tsv`.

### v7 full fixed-rate pilot

`results/pilot-v7-final/` contains all 24 attempts (four scenarios × three repetitions × two candidates), alternating Go→Rust, Rust→Go, Go→Rust. Settings remained 200/400/800/1,000 QPS, 3 seconds per stage, 500 ms deadline, 100 ms late drain, W2 fixture TTL 30 seconds with a 500 ms margin, SUT CPU 0 and harness/fixtures CPU 1. Schedule and per-attempt status are copied to `slice0-pilot-v7-schedule.tsv` and `slice0-pilot-v7-status.tsv`.

Every normal-reference sample completed 600/600 correctly on time; common-load and near-saturation samples also met their full schedules with no timeouts or wrong responses. One W1 UDP Rust overload stage sent 2,999/3,000 scheduled queries; every other overload stage sent its full schedule. All six W2 warm sessions passed per-key TTL checks for all five measured stages; maximum prefill-to-final-response age was 18.358 seconds versus the 29.5-second eligible window. All W3 per-request timestamp joins and exact route arrays passed; there were no route-event invalidations. The six W3 event journals totaled 11,581,195 bytes (1,923,620–1,937,944 bytes each).

The v6 health-check ceilings were applied unchanged during the v7 exploratory pilot. Eleven attempts were marked invalid at the terminal health-check gate; ten exceeded the old p95/p99 band and one was gated by the W1 UDP overload sender shortfall. This does not invalidate earlier stages in those runs. Do not treat any terminal stage as service-recovery evidence. SUT, generator, and fixture resource samples stayed below saturation: observed per-process CPU peaks were about 12.0%, 9.0%, and 4.0%; peak RSS was 61,064 KiB, 16,208 KiB, and 14,224 KiB respectively; max FDs were 11, 11, and 9.

For the new official health-check band, the predeclared rule takes the maximum p95/p99 across the six correctly completed v7 normal-reference samples for each scenario; it does not use terminal health-check results. Each sample had 600 latency observations and 600 correct-on-time responses:

| Scenario | p95 ceiling (µs) | p99 ceiling (µs) |
|---|---:|---:|
| W1 UDP | 274 | 365 |
| W1 TCP | 360 | 465 |
| W2 warm | 204 | 315 |
| W3 | 437 | 580 |

These are pilot-selected candidates for the reviewed official manifest; they are not frozen until Slice 0 review passes. The service-recovery assessment remains `indeterminate-no-overload-evidence` regardless of these health-check values.

### v8 assessment failure-path check and v9 environment probe

The v8 helper rechecked a retained v7 W1 UDP Go sequence whose terminal p95 was 255 µs against the then-frozen 252 µs ceiling. The helper exited nonzero, preserved all stage metrics, and emitted `status=indeterminate`, `mode=indeterminate-no-overload-evidence`, and the specific health-check failure reason. The captured output is `slice0-v8-failed-health-check.txt`.

The v9 runner's two W1 UDP environment probes confirmed the separate toolchain fields. The Go attempt's terminal health check was invalid; the Rust attempt passed. These two probes verify provenance recording only and are not added to the 24-row v7 pilot matrix. Candidate build metadata, not the base launcher version, is used for tool identity.

No official manifest has been frozen and no official paired sample has run. The result index hashes the v7/v8 smoke outputs, full v7 pilot outputs, v8 assessment check, and v9 environment probes without rewriting prior evidence.
