# Slice 1 preflight and manifest v1

Status: manifest v1 is frozen before official samples; all 24 tuples passed on-VM verification and the dry-run schedule; independent manifest review is pending. No official sample has run.

## Frozen artifacts

- Manifest: [official-manifest-v1.json](official-manifest-v1.json); SHA-256 `2ff4e0804b26af77db41c1357e107b00ea9539127665784087b29add3e43698f`, also recorded in [official-manifest-v1.sha256](official-manifest-v1.sha256).
- Pair runner: `run-official-matrix-v1.sh`, SHA-256 `853a801eb57bb58dde33ed24b4546cd1a45adc4efe65d2126cb4878a2dfbeee2`.
- Tool source runs from `/root/mosdns-rust-phase5a-first-native-performance-605c305/src-rust-v9`; runner SHA-256 `18c1e29cdcf23694a458899c26850414fd456529a84349f3a2884b06caf2245e`; helper source SHA-256 `2dec4788eaa3dad251061e380cbd8c32c9122285b852acf8b9a0b2eb81f64da1`.
- Go-only source commit `5b1eca69e0668ad1ddb6db88c0f39202557d5b98`; its archive SHA-256 is `63c902178e159d2d5e99f5ab42365a909f9ef75c6f1ca09aa73639f8682d7bd1`.
- Rust-native source commit `605c30577b79d397b5695618dbd2980e550ca6f3`; its archive SHA-256 is `28c3a4e9eb9d0f78c4e0da2dce253a9a1fe4aa81a983897b1287533581086e3b`.
- Fresh candidate binaries and helper are under `/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/`; their exact hashes and toolchain metadata are in `slice1-build-identities-v1.txt`.
- The seven unchanged config/workload hashes are in `slice1-fixed-input-hashes-v1.txt`; they match the repository files and the archived baseline record.

## Build and smoke evidence

The fresh Go build used `SKIP_UI_BUILD=1 CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GO_TAGS= BUILD_VERSION=phase5a-go-5b1eca6` with `scripts/build-local.sh`. The fresh native build used `cargo build --package mosdns-native-host --bin mosdns --release --locked`. The v8 helper was rebuilt with Go `1.26.4`; the Go candidate module metadata also reports `1.26.4`; Rust is `1.95.0`.

Both fresh candidates passed W1 UDP, W1 TCP, W2 and W3 smoke against the unchanged fixture inputs. All eight result rows have no `invalid-stages.tsv`; W2 cold/prefill/warm stage outputs and its 30-second TTL check are present; W3 exercised timestamp-to-request interval correlation and exact route paths. The status table is `slice1-smoke-v1-status.tsv`. Raw smoke outputs remain on the test VM and are indexed by `slice1-smoke-v1-result-index.sha256` (index SHA-256 `ca99b805d16fb607f97ede16a46d46548ced8878ddce84996d649f8ced2f922f`). Teardown left no task process or listener on any benchmark port.

## Frozen execution plan

The official matrix is four scenarios (`w1-udp`, `w1-tcp`, `w2`, `w3`), three paired repetitions, and 24 candidate attempts. The candidate order is Go→Rust, Rust→Go, Go→Rust. Every measured stage is 3 seconds at 200, 400, 800 and 1,000 QPS in the frozen order normal-reference, common-load, near-saturation, overload, recovery. Requests have a 500 ms deadline and 100 ms late drain. TCP opens a fresh connection per request.

The Go and Rust SUTs run on CPU 0; runner, generator and fixtures run on CPU 1. The W2 cold point is an isolated fresh process. W2 warm uses one unmeasured, one-pass prefill and then the same process for all five measured stages. The fixture TTL is 30 seconds with a 500 ms safety margin. The v7 pilot's maximum per-key prefill-to-final-response age was 18.357547 seconds, below the 29.5-second eligible window. Each official request ledger captures actual per-key prefill and response timestamps; any TTL violation invalidates the corresponding warm stage.

The terminal 200-QPS same-process stage requires at least 600 correct latency samples and uses frozen p95/p99 ceilings: W1 UDP 274/365 μs; W1 TCP 360/465 μs; W2 warm 204/315 μs; W3 437/580 μs. These ceilings are the maxima over the six valid v7 reference samples for each scenario. The manifest fixes service recovery to `indeterminate-no-overload-evidence`; even a passing terminal check is only a post-sequence health check.

The test VM has two online CPUs, 4,102,578,176 bytes of RAM, ext4 storage, 12,786,278,400 bytes free in the task filesystem, and an FD limit of 1,024. Preflight load average was 0.27/0.37/0.22; details are in `slice1-environment-v1.txt`. A pre-existing system `mosdns` process was observed asleep at 0.0% CPU, allowed on CPUs 0–1, and using 108,088 KiB RSS. It was not inspected for configuration, stopped, copied, or modified; the benchmark uses separate ports and records load averages before and after every candidate attempt. Its presence is part of the frozen VM context, not a production reference.

`run-official-matrix-v1.sh --validate-only` verified all 24 scenario/repetition/candidate tuples against the frozen manifest on the test VM without starting a SUT. `--dry-run` printed the same 24-row schedule; output is recorded in `slice1-manifest-v1-dry-run.txt`. The official result root did not exist after validation and there were no task processes or task-port listeners. Only `--execute` starts official samples; it refuses to reuse an existing results directory, calls the official runner for each tuple, retains nonzero attempts, and records per-attempt UTC times and host load averages. The matrix driver itself is pinned to CPU 1.

Raw binaries, complete smoke results, and run outputs remain under the task-owned test-VM directory. This freeze does not authorize production use or a multi-core capacity conclusion. Official samples must not begin until an independent reviewer passes this manifest and its matrix driver.
