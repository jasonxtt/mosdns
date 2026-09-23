# Design — paired Go-only / Rust-native W1–W3 measurement

Status: planning. No official measurements have been run for this task.

## Evidence model

Use the archived baseline's exact four YAML configs, three workload files, controlled upstream fixture and correctness oracle as immutable inputs. The Go-only and native binaries are independently built from frozen commits and passed to the existing `MOSDNS_BINARY` entrypoint. Each measured tuple is `(scenario, offered rate, repetition, implementation, manifest SHA)`; Go and Rust use the same tuple settings and distinct result directories. Neither the historical 4-CPU run manifest nor its results may be rewritten.

The existing runner starts `MOSDNS_BINARY start -c <config>`, supports smoke/pilot/official, collects SUT/fixture results, and verifies W2/W3 counters. Before editing, check current source support for release native-host CLI, fixed-rate sender, response classification, resource samples and cleanup. Extend only the smallest test-only surface needed for paired scheduling, per-run metadata and report aggregation. Do not add a generic benchmark platform or implementation-specific oracle. If any adapter normalizes dynamic DNS fields, document exactly which wire fields are ignored and why; never normalize rcode, answers, routes or cache behavior.

## Provenance and production reference

Prefer rebuilding the archived Go-only source commit `5b1eca69e0668ad1ddb6db88c0f39202557d5b98` on the test VM with the documented `CGO_ENABLED=0`, empty tags and no Rust selectors. Build native-host from one reviewed Rust commit with `--release --locked`. Record hashes of both outputs and all fixed inputs. A copied production binary, if needed later, is a **secondary reference** on the test VM only; its SHA/Go module metadata/config compatibility must be established and it must never replace the primary comparator silently. The production config is excluded from official inputs because it uses includes and contains deployment-specific data. Read-only structure/hash inspection is enough for this planning task.

## Preflight and freeze

On `ssh mosdns-rust`, use a task-owned directory on the disk-backed filesystem for build/result artifacts; `/tmp` may be a 2 GiB tmpfs. Verify architecture, toolchains, available cores/memory/disk, FD/cgroup/affinity, idle load, ports and correct cleanup. Run both binaries through the unchanged smoke matrix. Pin helper version and ensure the same helper drives both processes. A short non-official pilot determines offered-rate ladder and whether the generator and loopback fixtures can sustain it. Validate that failed sends, dropped scheduling slots, timer slip and fixture saturation are observable. Freeze a **new** JSON manifest and hash before official samples. Pilot rows remain labeled non-official.

On the 2-CPU VM the default credible setup is one pinned SUT core and one pinned generator/fixture core, with sequential Go and Rust runs; this measures comparable single-core service behavior, not multi-core scaling. An unpinned exploratory run may be retained separately but cannot serve as the capacity verdict. If helper/upstream CPU approaches saturation, scheduling slips, or machine load changes materially, invalidate the affected point; do not lower its QPS after viewing Rust results. A multi-core capacity claim waits for more isolated capacity.

## Official run and output

For each W1-UDP, W1-TCP, W2 and W3 scenario, run the frozen rate ladder with at least three repetitions and alternate Go-first/Rust-first order. W2 yields separate cold and warm measured stage rows with explicit prefill outside timing. Preserve fresh-process and cache lifecycle semantics per frozen scenario. W1 TCP retains the frozen connection policy. After each stage, validate DNS responses and W2/W3 fixture deltas before considering performance data. Store environment, manifest hash, SUT SHA, helper SHA, per-stage counters/latencies, /proc CPU/RSS/FD samples, fixture counters and logs. Retain invalid attempts with reason and no overwrite. Stop task-owned processes on error or interruption; do not touch other VM services.

Aggregate paired points by scenario and offered rate. Report per-run values plus median/range across repetitions; p99 interpretation must mention sample counts, especially at low QPS. Effective throughput is correct and within deadline per measured second, not all received packets. Report failures and offered/sent deltas beside latency. CPU/query denominator is correct-on-time count; an empty denominator is invalid, not zero. RSS must distinguish startup/stable/peak and note VM noise. For W2, show cold/warm hit and upstream deltas; for W3, show route identity/order evidence. Overload and recovery stages are separate. If the current helper lacks a reliable metric, either add focused test-only instrumentation before freeze or mark it unavailable; never infer a precise figure from unrelated historical output.

## Decision and change controls

Only test/benchmark tooling, new task evidence and a bounded report under `docs/rust/` are in scope. Immutable `tests/phase5a-baseline/configs/**`, `workloads/**`, archived baseline evidence and Go/Rust product code stay untouched. If a product defect blocks a scenario, preserve its repro, pause that scene and route a separately reviewed implementation fix; afterwards rebuild, re-freeze and rerun both candidates. Comparator, config, timeout or method changes after seeing official results require a new manifest and full paired rerun.

Interpretation order: correctness gate → measurement validity → valid latency/effective-throughput/resource curves → uncertainty → optimization suggestion. The report explicitly labels all-feature, long-run stability, multi-core scaling and production behavior as unmeasured.
