# Authorized Linux W1 correctness evidence

## Scope and source

This is task-local correctness evidence only. It is not a benchmark, a Go/Rust
performance comparison, a production deployment, or a rerun of the archived
official baseline.

The run used the user-designated `ssh mosdns-rust` host and a temporary
checkout materialized from the public GitHub archive URL for the exact pushed
commit:

```text
https://github.com/jasonxtt/mosdns/archive/bdfd01614b689fcc7eaabe868e8aeb5195008147.tar.gz
```

The temporary checkout and its build artifacts were removed on the remote host
after the commands completed. No project checkout, production path, or
historical evidence was modified on that host.

## Host and source facts

```text
host: mosdns-rust
uname: Linux mosdns-rust 7.0.9-x64v3-xanmod1 #0~20260517.ga456799 SMP PREEMPT_DYNAMIC Sun May 17 20:10:49 UTC 2026 x86_64 GNU/Linux
nproc: 2
rustc: rustc 1.95.0 (59807616e 2026-04-14)
cargo: cargo 1.95.0 (f2d3ce0bd 2026-03-21)
source archive commit: bdfd01614b689fcc7eaabe868e8aeb5195008147
```

The archived W1 inputs were hashed in that exact checkout:

```text
tests/phase5a-baseline/configs/forward-udp.yaml
  f6758fb2b2eb8056d6d1c2aef320a3357d005b92685136dec457d5543bd9d729
tests/phase5a-baseline/configs/forward-tcp.yaml
  1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1
tests/phase5a-baseline/workloads/forward.jsonl
  32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2
```

These hashes are input provenance only; the historical baseline manifest and
raw evidence remain unchanged.

## Exact correctness commands

The commands run from the temporary checkout were:

```bash
cargo test --manifest-path rust/native-host/Cargo.toml --test w1_udp --locked
cargo test --manifest-path rust/native-host/Cargo.toml --test w1_tcp --locked
cargo test --manifest-path rust/native-host/Cargo.toml --all-targets --locked
```

Results:

```text
w1_udp: 4 passed, 0 failed
w1_tcp: 5 passed, 0 failed
native-host all-targets: 12 unit tests, 0 main tests,
  2 slice2-config tests, 5 w1-tcp tests, 4 w1-udp tests;
  23 passed, 0 failed
```

The semantic response/error and lifecycle assertions passed on Linux:

```text
UDP: positive A, NXDOMAIN, distinct concurrent association,
  malformed-datagram isolation, timeout -> SERVFAIL,
  cancellation without late response, shutdown and rebind
TCP: positive A, NXDOMAIN, fragmented request/response framing,
  sequential per-connection requests, concurrent connections,
  idle_timeout=2, stalled upstream -> SERVFAIL,
  partial-frame EOF isolation, client disconnect,
  shutdown task joining and rebind
```

No benchmark driver, offered-QPS run, VM, deployment, or Go baseline runner
was invoked. The two-CPU host is correctness-only and cannot support the
archived four-CPU performance comparison contract.
