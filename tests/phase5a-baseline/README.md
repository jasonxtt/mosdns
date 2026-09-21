# Phase 5A Go-only baseline fixtures

This directory is the fixed, test-only corpus for the first Phase 5A
whole-process baseline. It contains only three narrowly scoped workload groups:

- W1: minimal UDP and TCP forwarding;
- W2: UDP cache cold/warm behavior;
- W3: domain-hit, IP-rule-hit, and IP-rule-miss routing.

The authoritative runner interface is:

```bash
MOSDNS_BINARY=/absolute/path/to/mosdns \
SCENARIO=w1-udp|w1-tcp|w2|w3 \
RUN_MODE=smoke|pilot|official \
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
new transports, or production service control. Official Linux amd64 runs must
use a pre-frozen task manifest and the Go-only binary built with `CGO_ENABLED=0`.
