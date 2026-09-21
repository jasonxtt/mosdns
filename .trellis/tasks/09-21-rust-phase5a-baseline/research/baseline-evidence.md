# Planning evidence — Rust Phase 5A Go-only whole-process baseline

Evidence date: 2026-09-21. This file records planning-time repository facts only. It is **not** a benchmark report and does not authorize implementation.

## 1. Repository state

Planning-time `rust` branch HEAD:

`e70a2408e2dcd2141e48bcc84765c5adfa406fe4`

Commit message: `chore(task): archive quic reuse multiplexing`.

At this revision, `.trellis/tasks/` contains only `archive/`; the previously active `09-20-rust-phase4-quic-reuse-multiplexing` task is archived under `.trellis/tasks/archive/2026-09/`.

This closes the roadmap prerequisite that the current bounded QUIC reuse task finish before the next Phase 5A planning/baseline work begins. It does not mean all Phase 4 product data-plane work is complete.

## 2. Architecture evidence

`docs/ai/rust-rewrite-plan.md` states:

- Phase 5A minimal native host is intentionally brought forward after the current QUIC reuse task instead of waiting for every remaining advanced transport/listener item.
- The future chain is YAML supported subset -> plugin/config registration -> UDP/TCP listener -> async sequence -> matcher/cache -> upstream -> response -> basic audit/metrics.
- Phase 5A requires isolated real-query E2E and Go/Rust comparison evidence; unsupported config must be rejected explicitly.
- The execution-order section says that after the current QUIC task, later implementation planning should first freeze functional mapping and process-level baselines, then prioritize Phase 5A and its minimum listener/async dependencies.

Therefore a small baseline-only task is a valid immediate predecessor to the native-host implementation task.

## 3. Performance-method evidence

`docs/rust/performance-validation.md` applies to Linux amd64 Phase 5A-6 and requires, among other things:

- independent Go-only and Rust-native process comparison;
- fixed source/build/config/workload identities;
- controlled local/LAN upstream before public-network validation;
- identical request sets and lifecycle/warm-up settings;
- correctness-aware latency/effective throughput rather than peak QPS alone;
- open-loop/fixed-rate offered load for authoritative capacity/latency evidence;
- p50/p95/p99, errors/timeouts/drops, CPU, RSS/resource trends;
- at least three repetitions for key scenarios and retention of all samples;
- reports containing commands, environment, thresholds/manifest, raw result locations, failure counts, scope and uncovered items;
- 5A is where the Go baseline and representative workloads are established; later performance thresholds are frozen before candidate acceptance, not invented after seeing results.

This task intentionally establishes the baseline curves without declaring a Rust pass/fail performance target.

## 4. Feature-coverage evidence

`docs/rust/feature-coverage.md` tracks native integration separately from foundation evidence. Relevant entries include:

- `cache` (P26): Phase 5A subset -> later complete query/management coverage;
- `forward` (P34): Phase 5A subset -> later Phase 4/5B completion;
- `sequence` (P44): Phase 5A async composition -> later full coverage;
- `udp_server`/`tcp_server` (P71/P70): minimum UDP/TCP native integration belongs to 5A, full server coverage later;
- C01/C02/C03/C14/C17 describe YAML, query/server, upstream, CLI/lifecycle and reference-config surfaces whose complete contracts extend beyond 5A.

Domain/IP matcher/provider rows are not all assigned to 5A. For that reason W3 is defined here as a **Go baseline corpus** and does not silently reassign native implementation ownership. A later native-host task must explicitly map the subset it chooses to implement.

## 5. Current Go YAML/source facts used for fixture planning

The following current Go sources are read-only evidence for freezing the baseline configs:

- `coremain/config.go`: top-level `log`, `include`, `plugins`, `api`; plugin entries use `tag`, `type`, `args`.
- `plugin/server/udp_server/udp_server.go`: `udp_server` args include `entry`, `listen`, `enable_audit`.
- `plugin/server/tcp_server/tcp_server.go`: `tcp_server` args include `entry`, `listen`, optional TLS fields, `idle_timeout`, `enable_audit`.
- `plugin/executable/forward/forward.go`: `forward` uses `upstreams`, per-upstream `addr`/`dial_addr`/timeouts and related options; no upstream means initialization error.
- `plugin/executable/cache/cache.go`: `cache` supports `size`, `lazy_cache_ttl`, `enable_ecs`, `exclude_ip`, `dump_file`, `dump_interval`; the baseline will freeze only the narrow parameters needed for W2.
- `plugin/executable/sequence/config.go`: sequence rule YAML exposes `matches` and string/list `exec`, with `$tag` references and quick-setup type forms.
- `plugin/data_provider/domain_set/domain_set.go`: `domain_set` can consume inline expressions/sets/files; W3 uses only committed local inline/fixed data.
- `plugin/data_provider/ip_set/ip_set.go`: `ip_set` can consume inline IP prefixes/sets/files; W3 uses only committed local inline/fixed data.
- `plugin/matcher/qname/qname.go` and `plugin/matcher/resp_ip/resp_ip.go`: current Go quick-setup matchers provide query-domain and response-IP checks used to characterize W3 routing.

The implementation must verify the exact accepted expression/sequence syntax with a correctness smoke before declaring the YAML frozen.

## 6. Go-only build evidence

`scripts/build-local.sh` builds the root product with:

- `-trimpath`;
- release-style `-ldflags`;
- `CGO_ENABLED` defaulting to `0`;
- caller-specified `GOOS`, `GOARCH`, output path and optional tags.

For this task the authoritative Go baseline is Linux/amd64 with `CGO_ENABLED=0`, no Rust tag, and no Rust backend selector environment. UI build may be skipped for this data-plane baseline because the task does not measure or change the UI and the Go binary embeds existing assets; the exact choice is recorded in the build identity.

## 7. Risks this plan closes before native-host work

1. **Moving benchmark target:** committed YAML/workload hashes and manifest prevent tuning against changing queries.
2. **Network noise:** controlled upstream makes initial latency/throughput evidence local and repeatable.
3. **Wrong-answer inflation:** response/route validation is part of useful-throughput counting.
4. **Closed-loop self-throttling:** official stages are fixed-rate/open-loop.
5. **Load-generator bottleneck:** scheduling lag and sender shortfall are first-class result fields.
6. **Resource blind spots:** SUT CPU and RSS are sampled for every stage.
7. **Implementation coupling:** the runner accepts only a binary path and product inputs, allowing a future Rust SUT without rewriting the workload.
8. **Scope creep:** test-only path allowlist and no dependency/product edits keep this from becoming a generic performance platform or a hidden Phase 5A implementation task.

## 8. Explicit unresolved items for Slice 0, not planning blockers

These are implementation-time fixture details that must be frozen before official measurement but do not require expanding task scope:

- exact `.test.` qnames and deterministic A/AAAA values;
- exact valid `domain_set` expression spellings and W3 sequence syntax after smoke validation;
- exact cache size/TTL and optional controlled upstream delay;
- latency histogram bucket representation;
- official offered-QPS ladder, which is selected by the bounded Linux pilot then frozen before official repetitions;
- actual CPU affinity sets for the chosen isolated Linux host.

Any change to scenario semantics, product code, module dependencies, public-network requirement, or native feature ownership is **not** one of these details and requires replanning/root review.
