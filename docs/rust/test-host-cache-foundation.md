# Rust cache test-host record

Status: isolated smoke and extended verification (dump restart, runtime fault
injection, sustained concurrency) complete; Rust cache remains experimental.

Date: `2026-08-13`

Host: `mos-test` (`10.0.0.91`), Linux x86_64. Existing services and the
production port were not changed: `mosdns-rust.service` remained active on
port 53 and the legacy `mosdns.service` remained inactive.

## Artifact and configuration

- isolated source/build directory: `/tmp/mosdns-rust-smoke.DfMJkB`
- binary: `/tmp/mosdns-rust-smoke.DfMJkB/dist/mosdns-rust-cache`
- build entrypoint: `scripts/build-rust-experimental.sh`
- smoke config: `plugin/executable/cache/testdata/rust-smoke.yaml`
- runtime selection: `MOSDNS_CACHE_BACKEND=rust`
- isolated listeners: UDP `127.0.0.1:15353`, HTTP `127.0.0.1:19099`
- upstream: the already-running local resolver at `127.0.0.1:53`

The build compiled the locked Rust static library, rebuilt both Vue bundles,
and linked the tagged Linux+cgo MosDNS binary.

## Results

- Startup negotiated `mosdns-cache-core/abi-1` and logged the Rust backend as
  enabled.
- Two identical A queries returned the same answer. The second query stopped at
  `smoke_cache`; metrics reported 2 queries, 1 hit, and Rust size 1.
- `/plugins/smoke_cache/show?limit=1` retained the existing text format and
  showed the mirrored entry, timestamps, and DNS message.
- Shutdown completed cleanly and closed all smoke plugins.
- Transactional valid/malformed dump paths were separately exercised against
  the real Linux+cgo bridge by `TestRustCGODumpFacadeImportsOnlyValidatedEntries`.

## Rollback and remaining soak gates

Rollback was process termination only. The isolated process was stopped; ports
15353 and 19099 are no longer listening. No systemd unit, `/usr/local/bin`
binary, `/cus` config, port-53 listener, or production host was modified.

The remaining gates were closed on `2026-08-13` by the reproducible replay/soak
(`docs/rust/benchmarks/cache-foundation.md`) and the extended isolated-process
verification below.

## Extended isolated verification (2026-08-13)

Host: `mos-test` (`10.0.0.91`). Binary built from the isolated source copy
`/tmp/mosdns-cache-soak-*` with `build-rust-experimental.sh` flow
(`SKIP_UI_BUILD=1`; the Rust `staticlib` was compiled there with `rustc
1.95.0`). Listeners were isolated UDP `127.0.0.1:15353/15354` and HTTP
`127.0.0.1:19099`; upstream was the already-running local resolver at
`127.0.0.1:53`. No systemd unit, installed binary, `/cus` config, port-53
listener, or production host was changed.

### 1. Dump write → restart → restore

Run A populated the cache, saved the dump, and shut down; Run B restarted with
the same config and served a hit from the restored entry.

```text
Run A: "experimental rust cache enabled" (mosdns-cache-core/abi-1)
  query -> miss;  metrics query_total=1 hit_total=0 size=1
  POST /plugins/soak_cache/save -> "Cache successfully saved"
  dump file 148 bytes; shutdown log "cache dumped entries=1"
Run B: "cache dump loaded entries=1"; rust backend enabled again
  query -> HIT;  metrics query_total=1 hit_total=1 size=1
  /plugins/soak_cache/show: entry StoredTime = Run A's timestamp
```

The `hit_total=1` on Run B with a single query proves the second answer came
from the restored cache (Rust backend), not a fresh forward.

### 2. Explicit runtime fault injection → Go fallback

Run with `MOSDNS_CACHE_BACKEND=rust MOSDNS_CACHE_FAULT_INJECT=1`. The env-gated
toggle (added to the experimental Linux backend only) makes the backend's
`Lookup` fail deterministically once traffic starts, tripping the facade's
one-way circuit breaker:

```text
startup: "experimental rust cache enabled"
query 1 -> status NXDOMAIN (valid answer; injected Lookup fault tripped breaker)
log: "experimental rust cache failed; circuit breaker selected go backend
      operation=lookup backend=mosdns-cache-core/abi-1 error=injected rust cache lookup fault"
query 2 -> status NXDOMAIN (served by Go fallback)
metrics: query_total=2 hit_total=1 size=1 (Go fallback is itself caching)
no panic / fatal in log
```

The backend was demonstrably active before the fault, the failure was a clean
runtime fault rather than an init failure, and the service continued to answer
correctly through the Go path with no panic.

### 3. Sustained concurrency

24 workers × 60 s of UDP queries against the Rust backend over 220 domains
(200 cached, 20 uncached), matched by transaction ID:

```text
sent=5,713,697 ok=5,713,697 mismatch=0 timeout=0 err=0   success=100%
qps=95,216 sustained
metrics: query_total=5,713,697 hit_total=5,711,798 size=220
log errors/panics/SERVFAIL/circuit-breaker: 0
RSS: 31 MiB before -> 83 MiB after 60 s -> 83 MiB +5 s (plateau, no leak)
```

Clean shutdown. Rollback was process termination only; all isolated ports were
closed and the `/tmp` verification artifacts (source copy, binary, configs,
logs, dump) were removed. Port 53 remained the pre-existing service.
