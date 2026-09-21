# Rust Phase 5A Go-only whole-process baseline

Status: **planning only**. This task must not be started or implemented until its planning files receive an explicit root-review `PASS`.

## Goal

在 Phase 5A 最小 Rust-native host 开始实现之前，先冻结一套**可复现、可替换被测二进制、仅针对首批 5A 数据面场景**的整机测量入口，并在 Linux amd64 隔离环境中取得当前 Go-only mosdns 的基准证据。

本任务只建立基线，不实现 Rust 主程序。交付物是：

1. 三组固定 YAML/请求语料：
   - 最小 UDP/TCP 转发；
   - cache；
   - domain/IP rule routing；
2. 受控本地上游和固定请求集；
3. 一个以后可通过 `MOSDNS_BINARY=/path/to/binary`（或等价显式参数）替换被测二进制的复现入口；
4. Go-only whole-process 正确性、延迟、有效吞吐、错误/超时、CPU、RSS 的 Linux amd64 结果；
5. 原始结果 manifest、环境记录和总结报告。

该基线以后可被 Phase 5A Rust-native host 复用，但**本任务的基准语料不自动改变 `docs/rust/feature-coverage.md` 的阶段归属，也不授权任何 Rust plugin/host 实现**。

## Why this is the next task

Planning-time repository anchor: `e70a2408e2dcd2141e48bcc84765c5adfa406fe4` (`chore(task): archive quic reuse multiplexing`). At that revision:

- `.trellis/tasks/` contains no active task directory other than `archive/`;
- the QUIC reuse/multiplexing task is archived;
- `docs/ai/rust-rewrite-plan.md` moves the roadmap to Phase 5A after that task and explicitly says to freeze product/task mapping and process-level baselines before subsequent implementation work;
- `docs/rust/performance-validation.md` says 5A establishes the Go baseline and representative workloads, uses controlled upstreams, fixed request sets, Linux amd64 process evidence, fixed-rate offered load, correctness-aware useful throughput, CPU/RSS, and repeated samples before later Rust comparison.

Therefore a narrow baseline-only predecessor is consistent with the current roadmap and avoids mixing harness/calibration work with the first Rust host implementation diff.

## Scope

### In scope

- Linux amd64 only.
- Build and run the current Go product as an independent process with the repository's normal Go-only release shape (`CGO_ENABLED=0`, no Rust build tag, no `MOSDNS_*_BACKEND=rust` selector).
- New test-only baseline fixtures/tooling under the exact implementation surfaces frozen in `design.md`.
- Deterministic loopback/local controlled upstream(s) with known answers and request counters.
- Fixed request corpus and frozen run manifest hashes.
- Three workload groups defined below.
- Fixed-rate load stages after a bounded pilot calibration; the measured ladder is written before official samples and then cannot move within the task.
- At least three official repetitions per measured variant; no best-run selection.
- Correctness validation for every measured response before it counts as useful throughput.
- Client p50/p95/p99 latency, offered/sent/received/correct-on-time counts, wrong responses, transport errors, timeouts, CPU, and RSS.
- Environment capture sufficient to reproduce the run: source/binary hashes, Go version, build flags, kernel, CPU model/count, affinity/cgroup choices, memory, FD limit, relevant sysctls if changed, GOMAXPROCS/GOGC/GOMEMLIMIT, ports, config/workload hashes.
- A final Go-only baseline report with limitations and explicit future-use instructions.

### Out of scope

- Any `rust/` implementation change.
- A Rust-native `main`, CLI, YAML loader, plugin registry, listener, sequence adapter, matcher/cache integration, upstream wiring, or metrics/audit implementation.
- New UDP/TCP/DoT/DoH/DoQ/DoH3 transport behavior.
- Changes to Go production behavior under `coremain/`, `plugin/`, `pkg/`, `main.go`, or existing runtime selectors.
- General-purpose benchmark framework, arbitrary protocol generator, web dashboard, distributed load platform, or cloud benchmark service.
- Public-Internet performance claims; the authoritative baseline is controlled local/isolated I/O.
- API/WebUI, persistence, cache dump, provider management, production configuration, packaging, release, or deployment.
- Production `mosdns` replacement or service restart outside the isolated benchmark environment.
- Declaring a Rust performance target, capacity threshold, or acceptance winner before a Rust candidate exists.
- Starting the later Phase 5A native-host task after this task closes.

## Workload contract

All scenarios use deterministic `.test.` names and controlled local upstreams. No result depends on public DNS.

### W1. Minimal forwarding — UDP and TCP

One frozen minimal forwarding corpus, measured as two variants:

- **W1-UDP:** UDP client -> mosdns UDP listener -> controlled UDP upstream.
- **W1-TCP:** TCP client -> mosdns TCP listener -> controlled TCP upstream.

The plugin/sequence shape must remain minimal and identical in semantics across the two variants. The response corpus contains deterministic success answers and at least one expected negative result so that expected NXDOMAIN-like outcomes are not misclassified as failures.

This group establishes whole-process listener + query + sequence/forward + upstream + response cost for the two minimum Phase 5A socket paths; it is not a complete Phase 4 transport benchmark.

### W2. Cache

UDP client/upstream only to keep the first cache baseline narrow. Freeze one cache YAML and one request corpus with two measured phases:

- **cold/controlled miss:** cache begins empty and each request's upstream behavior is known;
- **warm/hot:** an explicit prefill step completes before timing, then the same frozen hot set is replayed.

The controlled upstream counts requests. Cache correctness requires response semantics plus expected upstream-request deltas; a fast answer that bypasses expected cache behavior is not a success. Cache size/TTL and log/audit settings are frozen and recorded.

### W3. Domain/IP rule routing

One routing YAML uses controlled rule data and at least two deterministic upstream identities/answer maps so the final answer proves which route executed. The fixed corpus contains three classes:

1. a domain-rule hit routed directly to its designated upstream;
2. a non-domain hit whose first response matches the configured IP rule and follows the expected branch;
3. a non-domain hit whose first response does not match the IP rule and follows the alternate/fallback branch defined by the frozen YAML.

The implementation must derive the exact YAML syntax from the current Go plugin contracts (`domain_set`/`ip_set`, qname/response-IP matching, sequence, forward) and record the final config hash. This scenario is a **Go behavior baseline corpus**. Its presence here does not by itself move every involved plugin from 5B into the later 5A native implementation.

## Requirements

### R1. Freeze a source and build identity

- Record planning source anchor `e70a2408e2dcd2141e48bcc84765c5adfa406fe4`.
- Before official runs, record the exact task revision and prove no product-path change relative to the anchor except task/baseline tooling paths explicitly authorized by this task.
- Build the Go SUT with the repository's Go-only release defaults: Linux amd64, `CGO_ENABLED=0`, no Rust tag, and no Rust backend selector in the environment.
- Record `go version`, build command, binary SHA-256, `go.mod`/`go.sum` hashes, and relevant runtime environment.

### R2. Stable replaceable SUT entrypoint

The runner accepts an explicit executable path (`MOSDNS_BINARY` and/or `--binary`). It must not assume the executable is Go, inspect implementation-specific internals, or rebuild the SUT implicitly during a measured run. It records the resolved path and SHA-256 and refuses a missing/non-executable input.

The official evidence for this task uses only the Go-only binary. Replaceability is proven by running the same Go binary from a second path/copy through the same entrypoint; no Rust candidate is required or allowed.

### R3. Controlled upstreams

Provide task-scoped deterministic UDP/TCP DNS upstream fixtures using repository-available Go DNS support and the standard library only. They must:

- bind isolated loopback/test addresses and ephemeral or manifest-frozen ports;
- return deterministic answers for the frozen request names;
- expose machine-readable request counters per upstream/name/type;
- support a fixed optional response delay used only when frozen in the scenario manifest;
- shut down deterministically and leave no listener/process behind.

Do not add a new module dependency merely for the fixture.

### R4. Fixed request corpus and hashes

Every request has a stable case ID, qname, qtype, expected rcode, expected answer/route class, timeout, and scenario membership. The official corpus and configs receive SHA-256 hashes in a run manifest before official measurement.

Random query generation, public-name scraping, and changing the corpus between repeats are forbidden.

### R5. Correctness-aware load generator

The baseline driver must validate enough DNS semantics to reject wrong/mixed responses: request association, qname/qtype, rcode, and expected answer/route class. A response counts toward useful throughput only when it is correct and arrives within the frozen per-request deadline.

Expected negative DNS results remain correct results. Wrong answer, malformed response, timeout, connection error, or missing response is separately counted and never hidden from latency/throughput reporting.

### R6. Fixed-rate measured load

Measured stages use a sender schedule independent of response completion. Closed-loop request/response pacing may be used only as a clearly labeled preflight diagnostic, never as the official throughput/latency evidence.

A short pilot may determine a reasonable offered-rate range. Before official repetitions start, write the selected ordered QPS ladder, stage duration, warm-up duration, deadlines, and affinity choices into the immutable run manifest. All official repeats use that same ladder. If the environment cannot sustain the sender itself, the affected point is invalid and must be reported rather than silently lowered.

This baseline reports curves and effective throughput; it does **not** invent a Phase 5A Rust pass/fail capacity threshold.

### R7. Latency and effective throughput

For each official stage, record at minimum:

- target offered QPS;
- actual scheduled/sent requests;
- total responses;
- correct responses;
- correct responses within deadline (effective throughput numerator);
- expected-negative correct responses;
- wrong responses;
- transport/protocol errors;
- timeouts/dropped requests;
- latency p50/p95/p99 over correct responses, with timeout/error counts displayed beside the percentile data.

Do not compute attractive latency percentiles from a tiny successful subset while omitting failures.

### R8. CPU and RSS

On Linux, sample the SUT process at a documented fixed interval (default design target: 1 s) using `/proc` or equivalent dependency-free process accounting. Record at least:

- process user + system CPU time and derived CPU seconds per correct-on-time query;
- RSS time series, stable-window summary, and peak RSS.

The baseline tool may record FD count as diagnostics but must not expand into a general process profiler. `pprof`, perf flamegraphs, allocator tracing, and optimization work are follow-up evidence only if a later task requests them.

### R9. Isolation and environment disclosure

The Linux amd64 run must separate the SUT from load/upstream work with CPU affinity or equivalent documented isolation when the host permits it. Capture CPU topology, chosen CPU sets, memory, kernel, governor/turbo status if observable, FD limits, and whether processes share a host.

The harness must report its own inability to keep up. A load-generator bottleneck cannot be presented as SUT capacity.

### R10. Repetition and result retention

Run every official measured variant at least three times after warm-up. Keep every run. Do not select the best run. Store machine-readable per-stage summaries/histograms/counters plus environment and manifest data under the task research result directory; the final report links every artifact.

Since this task has no Rust candidate, Go/Rust interleaving is deferred. The later Rust comparison must rerun the Go baseline on the same environment/manifest rather than treating these numbers as timeless.

### R11. Narrow tooling boundary

The baseline helper may support only the fixed Phase 5A baseline operations needed here: deterministic upstream fixture, correctness-aware request replay, fixed-rate stage execution, and Linux SUT resource sampling. It must not grow plugin APIs for arbitrary protocols, remote agents, scenario DSLs, dashboards, or generalized workload scripting.

### R12. Stop after baseline acceptance

After the final baseline report and root-review `PASS`:

- mark this task complete and archive it according to normal Trellis workflow;
- do not create/start the native-host task in the same authorization;
- do not implement Rust host code;
- do not deploy the binary to production.

## Acceptance criteria

### A1. Planning/scope integrity

Final diff contains only task artifacts, baseline test tooling/fixtures, the narrow runner, and the baseline report/results. No `rust/`, `coremain/`, `plugin/`, `pkg/`, `main.go`, Go module dependency, API/WebUI, release, or deployment behavior changes.

### A2. Go-only identity is auditable

The report records source SHA, exact build command, Go/toolchain identity, binary hash, `CGO_ENABLED=0`, empty Rust build tags/selectors, and product-path diff check against the source anchor.

### A3. Three workload groups are frozen

W1 UDP/TCP forwarding, W2 cache cold/warm, and W3 domain/IP routing each have committed YAML/request fixtures, deterministic expected outcomes, and hashes in the run manifest.

### A4. Controlled upstream evidence

No public DNS is needed. Fixture counters prove the expected forwarding/cache/routing path and deterministic teardown leaves no benchmark listener/process behind.

### A5. Replaceable binary contract

The same runner successfully executes the same Go SUT from two executable paths without code/config changes. SUT path/hash are captured in each result.

### A6. Correctness is part of throughput

Every official stage reports correct-on-time, expected-negative, wrong, error, timeout, and missing-response counts. Wrong/late results do not count as useful throughput.

### A7. Fixed-rate evidence

The official QPS ladder is frozen before measured repetitions, every repeat uses it unchanged, and sender shortfall is visible.

### A8. Latency evidence

p50/p95/p99 are present per stage together with complete failure counters and enough histogram/sample summary data to reproduce the percentile calculation.

### A9. CPU/RSS evidence

Every official stage has SUT CPU and RSS data, with CPU/query and peak/stable RSS summaries tied to the exact run.

### A10. Repetition/no cherry-pick

At least three official runs per variant are retained and summarized; report shows spread/variation and does not suppress an outlier without documenting why the run is invalid.

### A11. Reproducible Linux amd64 report

The report contains environment, commands, hashes, configs, workload IDs, raw result locations, known limitations, and an exact rerun command using `MOSDNS_BINARY` or the equivalent explicit binary argument.

### A12. No overclaim

The report says only what the Go baseline proves. It does not claim Rust performance, complete 5A product support, full Phase 4 transport completion, public-network performance, or production readiness.

### A13. Stop boundary

After baseline acceptance there is no native-host implementation, next-task start, production wiring, or deployment in this task.
