# Phase 5A Go-only baseline fixtures

This directory is the fixed, test-only corpus for the first Phase 5A
whole-process baseline. It contains only three narrowly scoped workload groups:

- W1: minimal UDP and TCP forwarding;
- W2: UDP cache cold/warm behavior;
- W3: domain-hit, IP-rule-hit, and IP-rule-miss routing.

The basic smoke interface is:

```bash
MOSDNS_BINARY=/absolute/path/to/mosdns \
SCENARIO=w1-udp|w1-tcp|w2|w3 \
RUN_MODE=smoke \
RESULT_DIR=/absolute/path/to/result \
./scripts/run-phase5a-baseline.sh
```

`MOSDNS_BINARY` is required, must be executable, and is never rebuilt by the
runner. `HELPER_BINARY` may point to a prebuilt copy of the task-scoped helper;
when omitted, the runner builds only the helper under a temporary result
directory. The same binary copied to a second executable path is the Slice 0
replaceability smoke. No Go/Rust implementation detail is passed to the
runner.

The four YAML files use the current Go plugin contracts. Ports are loopback
only and are deliberately fixed in the committed configs so hashes identify
the exact scenario. The upstream fixture is deterministic and writes a
machine-readable per-upstream/name/type counter file.

The helper is intentionally not a generic benchmark platform. It does not
support public DNS, arbitrary scenario scripts, remote workers, dashboards,
new transports, or production service control. The current `official` mode
requires an explicit `MANIFEST_PATH` to a reviewed manifest compatible with
the current helper, plus its pinned `MANIFEST_SHA256` and the scenario-specific
plan fields. The Phase 5A paired-run driver records that complete invocation.
The archived 2026-09-21 Go-only `run-manifest.json` has an older schema and
cannot be used as the current runner's default. Reproducing that historical
official baseline requires its frozen runner/helper revision and environment;
new measurements require a newly reviewed manifest. The current runner never
silently substitutes either manifest.

## Slice 1 reliability helper

The bounded, local-only reliability contract is exposed through the helper's
`reliability-run` and `reliability-assess` commands and
`scripts/run-phase5a-reliability.sh`. `reliability-run` requires a loopback
`--addr`, a fixed workload, explicit slot/deadline/queue limits, and a fresh
result directory. It writes a versioned `reliability-raw.json` bundle plus the
raw evidence JSONL. Its planner is open-loop: response completion never
advances future slots, and scheduler/queue/resource limits become explicit
slot outcomes. The slot ledger is recomputable from raw records using the
planned/started/DNS-sent conservation equations; a complete DNS frame is
required before `dns_sent` becomes true. Each started slot retains monotonic
planned, dispatch, write-start, write-complete, and finish offsets; a complete
write that crosses the service deadline is retained as a write/deadline race
and cannot become an on-time success.

`reliability-assess --raw <bundle> --output <fresh-dir>` recomputes the
accounting and the planned-slot-to-finish, dispatch-to-finish, and
write-start-to-finish latency views from raw records. Missing or mismatched
evidence fails closed; each view includes its failure denominator and eligible
sample count, and the assessment includes raw-derived correct-on-time goodput
over the offered stage duration. Window overload/recovery facts are also
derived from raw slot membership and evidence rather than trusted summaries.
If windows are supplied, their raw sequence must include contiguous equal-length
boundaries, phase, offered QPS, process PID/start identity, and an explicit
resource-budget fact; overload/recovery criteria include a frozen reference
QPS and on-time-rate floor.
When archive identity flags are supplied, the command
requires an explicit Git root, an exact historical object/path, and a matching
SHA-256; it never falls back to the current worktree or a parent object.

The reliability shell runner keeps scenario and transport as separate fields:
use `SCENARIO=w1|w2|w3` with `TRANSPORT=udp|tcp`. The legacy `w1-udp` and
`w1-tcp` aliases are intentionally rejected here so transport cannot be
silently encoded twice in a scenario name.

The reviewed Phase 5A official path uses the same reliability protocol through
`reliability-run` for every stage, then requires a fresh raw bundle,
`reliability-assess`, and resource-budget verification before a stage is valid.
The manifest's process, file-size, descriptor, queue, and in-flight limits are
enforced by the runner rather than being descriptive metadata. A failed stage
invalidates the run, stops the task-owned processes, preserves the result root,
and prevents later stages from running. Official runs freeze 32 KiB raw-record
capacity so the complete 3-second ladder remains inside the helper's 64 MiB
control budget.

The official same-process terminal health gate is explicitly evaluated with the
`dispatch-to-finish` latency view. The primary `stages.jsonl` latency summary
remains `planned-slot-to-finish`; its sender scheduling lag is intentionally
not mixed into the dispatch-based health band. Each official stage therefore
records both the primary samples and a separately declared
`health_latency_view`, `health_latency_samples_us`, and health percentiles.
The runner passes the manifest-frozen view to `verify-continuous` and fails
closed if that view is missing or mismatched. This separates a like-for-like
pilot-derived health check from the primary measurement metric without
loosening the frozen p95/p99 ceilings.

This Slice 1 runner is an offline/loopback test helper only. It does not start
MosDNS, use SSH, run pilot/official traffic, profile a process, or make
hotspot/capacity claims.
