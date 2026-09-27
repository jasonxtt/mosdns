# Slice 0 environment preflight

Status: `G0 environment evidence collected; candidate measurement not authorized
by this artifact`. This is a read-only preflight, captured on 2026-09-27
local time (`2026-09-26T16:57:36Z`) from the dedicated `rust` worktree. No
build, benchmark, query traffic, service restart, kernel tuning, or production
host access was performed.

## Local execution identity

- Workspace: `/Users/tom/github/mosdns-rust`
- Branch/upstream: `rust` / `origin/rust`
- `HEAD`: `5478015f7998be5335a7019915af558da5c74b4b`
- Ahead/behind upstream: `0/0`
- `session_auto_commit=false`
- Trellis task: `rust-phase5a-measurement-reliability`, now `in_progress`
- Allowed edit surface for this slice: this task's `research/` only. Product
  code, old baseline behavior, frozen historical evidence, and unrelated dirty
  paths remain untouched.

The working tree was already dirty before Slice 0. The following paths are
pre-existing or archive/task bookkeeping and are not attributed to this slice:

```text
 D .trellis/tasks/09-22-rust-phase5a-native-routing/check.jsonl
 D .trellis/tasks/09-22-rust-phase5a-native-routing/design.md
 D .trellis/tasks/09-22-rust-phase5a-native-routing/implement.jsonl
 D .trellis/tasks/09-22-rust-phase5a-native-routing/implement.md
 D .trellis/tasks/09-22-rust-phase5a-native-routing/prd.md
 D .trellis/tasks/09-22-rust-phase5a-native-routing/research/planning-review.md
 D .trellis/tasks/09-22-rust-phase5a-native-routing/research/source-audit.md
 D .trellis/tasks/09-22-rust-phase5a-native-routing/task.json
 D .trellis/tasks/09-23-matcher-adapter-typed-nil/check.jsonl
 D .trellis/tasks/09-23-matcher-adapter-typed-nil/design.md
 D .trellis/tasks/09-23-matcher-adapter-typed-nil/implement.jsonl
 D .trellis/tasks/09-23-matcher-adapter-typed-nil/implement.md
 D .trellis/tasks/09-23-matcher-adapter-typed-nil/prd.md
 D .trellis/tasks/09-23-matcher-adapter-typed-nil/task.json
 M .trellis/workspace/tom/index.md
 M .trellis/workspace/tom/journal-1.md
 M docs/ai/rust-handover.md
 M docs/rust/next-stage-plan.md
?? .trellis/.DS_Store
?? .trellis/tasks/.DS_Store
?? .trellis/tasks/archive/2026-09/09-16-rust-phase4-secure-upstream-foundation/.DS_Store
?? .trellis/tasks/archive/2026-09/09-22-rust-phase5a-native-routing/
?? .trellis/tasks/archive/2026-09-23-matcher-adapter-typed-nil/
?? .trellis/tasks/archive/2026-09-24-trellis-c2c-reviewer-adapter/
?? .trellis/tasks/archive/2026-09-24-trellis-c2c-web-compare/
?? .trellis/tasks/archive/2026-09-24-trellis-c2c-web-reviewer/
```

The current task directory itself was untracked before Slice 0; its planning
files are the reviewed inputs, not product changes.

## Remote preflight (`ssh mosdns-rust`)

The command was read-only and queried `/proc`, `/sys`, `/proc/sys`, `ss`, and
tool version/path information. Observed values:

| Area | Observation | Consequence |
| --- | --- | --- |
| Host | Debian GNU/Linux 13, `x86_64`, Linux `7.0.9-x64v3-xanmod1`, KVM | Linux amd64 target is present; disclose VM/kernel identity in any future manifest. |
| CPU | 2 online CPUs (`0-1`), current shell affinity `0,1`; load `0.19 0.12 0.10` at capture | Only a controlled single-core comparison may be claimed; no multi-core result. Use disjoint `taskset` masks only after checking owned processes. |
| cgroup | cgroup v2 mounted; `cpuset.cpus.effective=0-1`; `cpu.max` unavailable | No quota value is available from this path; record quota as unknown, not unlimited. |
| Memory | MemTotal `4006424 kB`; MemAvailable `3412060 kB` | Freeze per-process/task RSS ceilings before any candidate run; do not infer capacity from this snapshot. |
| Disk | task filesystem `/` is ext4 with `2302132` 1-KiB blocks available (about 2.3 GiB, 91% used); `/tmp` is a 2-GiB tmpfs | Evidence must use a task-owned disk-backed directory, not `/tmp`; refuse a run when the reserved free-space floor is not met. |
| FD | shell soft limit `1024` | Reserve capacity for the pre-existing service; bound sender/fixture FD and in-flight connection budgets. |
| TCP | ephemeral range `32768-60999`; `tcp_tw_reuse=2`; `tcp_fin_timeout=60`; TIME_WAIT `0`; established `1` at capture | W1 qualification must record start/during/end TIME_WAIT, connection errors, reuse setting and cooling; current zero is only a baseline. |
| Existing listeners | pre-existing `mosdns` PID `425` owns UDP/TCP listeners on `53`, `2222`, `3077`, `3099`, `3111`, `4444`, `7777`, `8888` | Do not use or alter these listeners. Future task ports must be high, independently probed and owned by the run. |
| Clock | `CLK_TCK=100` | Resolve once before measured load; retain sampling gaps and do not treat coarse ticks as zero CPU. |
| Toolchain | Go `go1.24.4`; Rust `1.95.0`; Cargo `1.95.0` | Record compiler identity in every future binary manifest. |
| Profile access | `perf_event_paranoid=2`; `/proc/kallsyms` and `System.map` readable; tracefs present; effective capability mask was observed | Access policy alone does not provide a profiler. No global security setting may be lowered. |
| Profile tools | `perf`, `strace`, `bpftrace`, `gdb`, `valgrind`, `eu-stack`, `cargo-flamegraph`, `flamegraph`, `trace-cmd` all missing; only `addr2line`, `objdump`, `nm`, `readelf`, Go/gprof are present | Current host cannot yet produce the planned Rust stack/profile evidence. A5 hotspot acceptance is blocked until an approved profiler is available or the user authorizes a separately reviewed tool-environment change. |
| Process sampling | `/proc/self/status` readable | `/proc` resource sampling is possible, but aggregate CPU/RSS alone is insufficient for hotspot classification. |

No command accessed `ssh mos`, no production configuration was read, and no
remote file was written. The active service and listeners are evidence of the
host baseline only; they are not task-owned processes and must not be signaled
or counted as task output.

## Reviewed source identity inputs

- Go reference source: `5b1eca69e0668ad1ddb6db88c0f39202557d5b98`
  (`fix(task): widen Linux SUT startup barrier`), present locally.
- Candidate source baseline: `5478015f7998be5335a7019915af558da5c74b4b`
  (`fix(task): close archive revalidation review findings`). Current task
  artifacts are uncommitted and are not part of the candidate product source.
- Workload hashes at this baseline:
  - `tests/phase5a-baseline/workloads/cache.jsonl`:
    `7680cd55ccf477972cd700c05e72c83c52ab1a55643b9a98bb610629035214ed`
  - `tests/phase5a-baseline/workloads/forward.jsonl`:
    `32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2`
  - `tests/phase5a-baseline/workloads/routing.jsonl`:
    `dbb582dc9d623af54e501639b7a52538b1e1d9e493fa0f0f6a4f4714ea0176c1`
- Current legacy tool hashes:
  - `tests/phase5a-baseline/README.md`:
    `f07fb637830e737b8ed7cd7bbe97bd00f0ec643e4145acbad5c3a24662d6ec2e`
  - `scripts/run-phase5a-baseline.sh`:
    `d888d849f9f314e49b6d1d4fc8a0abe5412a841b5ccf5bc20f52a2c890390bee`
  - `tests/phase5a-baseline/cmd/phase5a-baseline/main.go`:
    `d684907953e0544aa4e956c6ff77cc5f319ef9e4a4a6733a8658778bf667f27c`

The reviewed planning input hashes are preserved in
`research/execution-plan-v1.md`; no current task planning file was changed by
the preflight.

## G0 disposition

The host is usable for read-only evidence collection and bounded local helper
tests, but Slice 0 cannot claim a complete hotspot/profile gate: the required
process-directed profiler is absent. The task therefore freezes the missing
capability as an explicit G0 blocker. It must not be papered over with total
CPU/RSS, a different machine, a lowered kernel policy, or a production run.
The C2C review of this Slice 0 package decides whether the remaining offline
helper work may proceed while the environment blocker remains; official
candidate measurement and A5 remain prohibited until the blocker is resolved.

## Profiler remediation status

After the initial preflight, the user authorized an environment-only change on
`mosdns-rust`. `linux-perf 6.12.107-1` and `libc6-dbg 2.41-12+deb13u4` are now
installed. The kernel policy remains `perf_event_paranoid=2`; no global
security setting was lowered. A controlled process-directed `perf record` with
the software `cpu-clock` event produced 232 samples, zero lost samples, and
call-chain output. A controlled `perf stat` check showed usable `task-clock`
but zero `cycles` and `instructions`, so future profile runs must disclose the
software-event limitation unless a later review enables usable PMU counters.

The exact commands, output, failed smoke attempts, cleanup, and hashes are in
`research/slice0-profiler-remediation.md`. The original G0 decision remains
recorded as accepted by the `002reviewer` follow-up in
`research/c2c-slice0-profiler-review.md`. This resolves the profiler capability
gate, but does not itself constitute a MosDNS profile, hotspot finding, or
capacity result.
