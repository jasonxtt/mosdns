# Design — Rust Phase 5A Go-only whole-process baseline

Status: **planning only**; no implementation or benchmark execution before root-review approval and `task.py start`.

## 0. Design intent

This task is deliberately smaller than the Phase 5A native-host implementation. It establishes one stable comparison corpus and one process-level measurement path first, so later Rust work does not simultaneously invent the benchmark, choose the workload, and optimize against a moving target.

The design follows `docs/rust/performance-validation.md` but implements only the minimum needed for the first three 5A data-plane scenarios. It is not a reusable generic benchmark product.

## 1. Source anchor and safety boundary

Planning source anchor:

`e70a2408e2dcd2141e48bcc84765c5adfa406fe4`

At task implementation time, exact task revisions may move because planning/review files are added. The Go product source anchor remains the reference for a product-path no-change check. Authorized implementation paths are limited to:

```text
.trellis/tasks/09-21-rust-phase5a-baseline/**
tests/phase5a-baseline/**
scripts/run-phase5a-baseline.sh
docs/rust/phase5a-go-baseline.md
```

No existing product source or manifest is an allowed implementation surface. In particular, the following are forbidden unless planning is reopened and root-reviewed:

```text
rust/**
coremain/**
plugin/**
pkg/**
main.go
go.mod
go.sum
.github/**
web/**
webui-log/**
```

The harness must consume current Go behavior; it must not patch product behavior to make the benchmark easier.

## 2. Stable artifact layout

Planned stable layout:

```text
tests/phase5a-baseline/
  README.md
  configs/
    forward-udp.yaml
    forward-tcp.yaml
    cache.yaml
    routing.yaml
  workloads/
    forward.jsonl
    cache.jsonl
    routing.jsonl
  cmd/phase5a-baseline/
    main.go
  internal/...

scripts/
  run-phase5a-baseline.sh

docs/rust/
  phase5a-go-baseline.md

.trellis/tasks/09-21-rust-phase5a-baseline/research/
  baseline-evidence.md
  run-manifest.json
  results/
    <run-id>/...
```

`tests/phase5a-baseline` remains after task archive so the same corpus/runner can be reused by a later Rust binary. Task-local `research/results` is historical evidence only.

No second module is created. The helper builds inside the repository module and may use only standard library plus dependencies already present for the Go project (notably the existing DNS package). `go.mod`/`go.sum` must remain unchanged.

## 3. Process topology

Official run topology:

```text
fixed-rate load process
        |
        | UDP or TCP
        v
  SUT mosdns process  <---- sampled via /proc
        |
        | controlled UDP or TCP
        v
controlled upstream fixture process(es)
```

The runner orchestrates processes but keeps them separately identifiable. On a sufficiently provisioned host, SUT and harness/upstream receive disjoint CPU affinity sets. If the host cannot provide meaningful separation, the run is labeled unsuitable for authoritative baseline evidence and does not become the accepted baseline.

No public network is required.

## 4. Go-only SUT build

The accepted Go baseline uses the repository's normal local release shape:

```bash
SKIP_UI_BUILD=1 \
CGO_ENABLED=0 \
GOOS=linux \
GOARCH=amd64 \
GO_TAGS='' \
OUTPUT=<isolated-output>/mosdns-go \
./scripts/build-local.sh
```

The implementation must verify and record:

- current Git SHA and source anchor;
- product-path diff against the anchor;
- `go version`;
- `go env GOOS GOARCH`;
- exact build flags;
- `sha256sum` of the SUT;
- `sha256sum` of `go.mod` and `go.sum`;
- relevant `GOMAXPROCS`, `GOGC`, `GOMEMLIMIT` values (including unset/default).

Before launching the SUT, the wrapper removes/refuses Rust backend selectors from the measured environment. It must not set `MOSDNS_CACHE_BACKEND`, `MOSDNS_MATCHER_BACKEND`, `MOSDNS_QUERY_BACKEND`, or future equivalent selectors to Rust.

## 5. Replaceable binary contract

The wrapper interface is intentionally implementation-neutral:

```bash
MOSDNS_BINARY=/absolute/path/to/mosdns \
RESULT_DIR=/absolute/path/to/results \
./scripts/run-phase5a-baseline.sh
```

Equivalent explicit flags are allowed, but exactly one authoritative interface is documented in `README.md` and the final report.

The runner passes only product-level inputs: config path, signal/termination, network queries, and process observation. It must not call Go-specific debug endpoints or Rust-specific internals.

For this task, replaceability is proven by copying the same Go binary to another path and running the smoke/correctness stage through the unchanged interface. This avoids implementing or requiring a Rust candidate.

## 6. Controlled upstream fixture

The `phase5a-baseline` helper has a narrowly scoped `upstream` mode. It supports only what the frozen scenarios need:

- UDP DNS listener;
- TCP DNS listener;
- deterministic table-driven A/AAAA/NXDOMAIN-style answers used by the committed corpus;
- optional fixed response delay from the frozen manifest;
- per-upstream/name/type counters exposed through a local result file or stdout JSON;
- clean signal/parent-death shutdown.

It is not an arbitrary DNS server or proxy framework.

Every route used by W3 returns a distinguishable result so final response validation proves the path. Cache upstream counters prove W2 hit/miss behavior.

## 7. Workload schema

Each workload record is line-oriented and reviewable. Exact JSON field names may be refined in Slice 0, but the normative information is:

```text
case_id
scenario
transport
qname
qtype
expected_rcode
expected_answer_class
expected_route_class
request_deadline_ms
weight
```

`weight` expands a fixed corpus deterministically; it is not random generation. The runner records workload file SHA-256 and effective expanded case count.

Names are under a task-owned `.test.` namespace so they never require Internet resolution.

## 8. Scenario design

### 8.1 W1-UDP

- SUT UDP listener.
- Minimal sequence/forward path.
- Controlled UDP upstream.
- Fixed deterministic request corpus.
- No cache/rule provider/audit extras beyond what the YAML explicitly freezes.

### 8.2 W1-TCP

Same product semantics as W1-UDP, with TCP ingress and TCP controlled upstream. TCP connection policy used by the load generator is frozen in the run manifest; it must not silently change between repetitions.

### 8.3 W2 cache

- UDP ingress/upstream.
- One fixed cache size and TTL.
- Cold phase starts from a newly launched SUT or explicit supported cache-empty lifecycle defined by the YAML/run procedure.
- Warm phase has an explicit unmeasured prefill pass followed by a barrier that verifies upstream request count and waits for completion before timing starts.
- Measured warm requests use the identical committed hot set.

No dump/import, lazy-cache, ECS, exclusion, provider management, or API operations are added unless the planning task is reopened; those belong to later coverage.

### 8.4 W3 domain/IP routing

One committed routing config is assembled only from existing Go YAML semantics. It uses inline/fixed rule data rather than downloaded data so no network/source drift exists.

Required route classes:

```text
DOMAIN_HIT     -> designated upstream A -> distinguishable final answer
IP_RULE_HIT    -> initial upstream B -> response IP matches rule -> frozen expected branch
IP_RULE_MISS   -> initial upstream B -> response IP misses rule -> alternate/fallback upstream C -> distinguishable final answer
```

The exact sequence syntax and whether the IP-hit branch returns the first response or executes a second action are frozen from repository behavior during Slice 0 and captured in committed YAML + correctness tests. No semantics are invented for the future Rust host here.

## 9. Load schedule

### 9.1 Pilot

A bounded pilot is permitted only to choose a useful fixed-rate range. It is not part of final comparative evidence.

The pilot outputs a candidate ladder. Before official runs, the operator writes the final ordered offered-QPS values, stage durations, warm-up, request deadline, process affinities, and fixture delay into `research/run-manifest.json`; the file is then treated as immutable for all official task runs.

### 9.2 Official stages

Official load is open-loop/fixed-rate: request dispatch follows the schedule even if prior responses are slow. The driver tracks scheduling lag. When it cannot dispatch at the planned rate, that shortfall is reported and the point cannot be claimed as SUT capacity.

No adaptive rate change occurs inside an official run.

The task intentionally does not define a Rust pass/fail QPS threshold. The output is a Go curve that later tasks can compare against after they freeze their own acceptance budgets.

## 10. Correctness accounting

Each response is associated with its request and checked online. Minimum checks:

- response corresponds to the request case;
- question qname/qtype are consistent where present;
- rcode equals the expected rcode;
- answer class/value sufficient to distinguish the expected controlled route;
- response arrives before the frozen request deadline for `correct_on_time`.

Separate aggregate counters from the per-request terminal classification. The
aggregate counters are totals and are not mutually exclusive:

```text
scheduled
sent
received
```

Each request that reaches a terminal outcome has exactly one mutually
exclusive classification:

```text
correct_on_time
correct_late
wrong_response
protocol_error
transport_error
timeout
sender_shortfall
```

`expected_negative_on_time` is a transparent annotation/sub-counter of
`correct_on_time`, not another mutually exclusive outcome. It must never be
added to `correct_on_time` as a second request. A valid expected-negative
response that arrives after the deadline is classified as `correct_late` and
does not receive the on-time annotation.

The deadline boundary is deterministic: a valid expected response received by
the frozen request deadline is `correct_on_time`; a valid expected response
received after that deadline but before the bounded late-drain end is
`correct_late`; if no terminal response is observed by the late-drain end, the
request is `timeout`. A wrong or malformed response observed in either window
is `wrong_response` or `protocol_error`, respectively, rather than an
additional timeout. A send that the driver cannot issue at its scheduled point
is `sender_shortfall` and is not also a timeout. Only `correct_on_time` is the
effective-throughput numerator; all aggregate and classification counters are
reported together so failures cannot be hidden.

## 11. Latency representation

The helper maintains a deterministic latency histogram for all correct responses and a separate on-time subset if needed. The histogram format and bucket/unit definition are committed with the tool and recorded in results. Per stage, emit p50/p95/p99 plus the complete failure counters.

The implementation need not persist one JSON row per request if that would produce unreasonable repository artifacts; machine-readable per-stage histograms and counters are the raw measurement artifact for this task. The report must not claim a percentile that cannot be reconstructed from the stored histogram.

## 12. CPU/RSS sampling

A Linux-only sampler reads the SUT PID at a fixed interval (target: one second):

- `/proc/<pid>/stat` for user/system CPU ticks;
- `/proc/<pid>/status` (or equivalent documented field) for RSS;
- optional `/proc/<pid>/fd` count only as a lightweight diagnostic.

Capture clock tick rate and sample timestamps. Derived stage summaries:

```text
cpu_user_s
cpu_system_s
cpu_total_s
cpu_s_per_correct_on_time_query
rss_median_or_stable_window_kib
rss_peak_kib
```

No deep profiler is part of this task.

## 13. Isolation/preflight

Before official runs the wrapper checks and records:

- Linux amd64;
- available CPU count/topology;
- requested CPU affinity sets are valid and non-overlapping for SUT vs harness when configured;
- sufficient free memory and FD limit;
- benchmark ports are free;
- SUT binary is executable;
- result directory is writable;
- no prior benchmark fixture/SUT PID is still alive;
- current source/config/workload hashes match the manifest.

If authoritative CPU isolation cannot be achieved on the chosen host, the run may be used for smoke/debug only, not accepted as the baseline report.

## 14. Repetition and ordering

Every official variant runs at least three complete repetitions. The order is fixed in the run manifest (for example W1-UDP, W1-TCP, W2, W3, repeated as a block) so the operator cannot reorder after seeing results. All runs are retained.

Because no Rust candidate exists in this task, Go/Rust interleaving is not applicable. The future Rust task must rerun the Go binary in the same session/environment and interleave implementations there.

## 15. Result schema

Each run directory contains at minimum:

```text
environment.json
sut.json
fixture.json
manifest.sha256
stages.jsonl
resource-samples.jsonl
stdout.log
stderr.log
```

`stages.jsonl` stores counters, histogram, percentile summary, sender shortfall, and fixture counter deltas for each stage. Logs are bounded; routine per-query logging is disabled in the SUT YAML so log I/O is not the benchmark itself.

## 16. Failure and cleanup

Any of these invalidates the current run and is reported:

- SUT exits unexpectedly;
- fixture exits or returns an unknown response;
- load generator cannot maintain the scheduled rate beyond the documented tolerance;
- wrong response/mixup occurs;
- resource sampling loses the SUT PID;
- manifest/hash changes during official repetitions;
- cleanup finds a surviving SUT/fixture process.

Invalid runs are never deleted. They are marked invalid with reason; a replacement run is additional evidence, not a rewrite.

The wrapper uses traps/signal handling so normal failure cleans child processes and temporary directories.

## 17. Report contract

`docs/rust/phase5a-go-baseline.md` must contain:

- exact source and binary identities;
- environment and isolation;
- frozen config/workload manifest hashes;
- build/run commands;
- scenario descriptions;
- all repeat summaries and spread;
- correctness/failure counts;
- latency/effective-throughput curves;
- CPU/RSS results;
- invalid runs and reasons;
- raw result paths;
- known limitations;
- exact future rerun command with a replaceable `MOSDNS_BINARY`;
- explicit statement that no Rust/native-host or production conclusion is made.

## 18. Review boundary

This task has three implementation slices. Each slice stops for an exact scoped review. A `PASS` authorizes only the next slice in this task. A `FAIL`, task-scope change, product-source edit, new dependency, or inability to obtain isolated Linux evidence stops the task for remediation/replanning.

After Slice 2 final acceptance, stop. Do not create/start the native-host implementation task under this authorization.
