# Slice 2 calibration, Go reference pilot, and G2 manifest candidate

Status: calibration and Go-only pilot complete; G2 remediation is implemented
and the manifest is awaiting the C2C re-review. No Rust latency or capacity
result is included here.

## Remote identity and staging

- Host alias: `mosdns-rust`
- Task root: `/root/mosdns-rust-phase5a-measurement-reliability-20260927`
- OS/kernel: Debian 13, Linux `7.0.9-x64v3-xanmod1`, amd64, 2 online CPUs
- Harness CPU: `0`; candidate/fixture CPU: `1`
- Helper: `phase5a-baseline-helper/v10`, SHA-256
  `9a0ddb7d5cb52690c55bc915540c639b656b17bf8ff0d786066edffa6e1fc836`
- Runner SHA-256:
  `7e9ded3f2c68f8289e69b1c4863f5e5ce00d9d1868bf7e13e945c6e911bd6703`
- Go candidate source: `5b1eca69e0668ad1ddb6db88c0f39202557d5b98`, binary SHA-256
  `fece7ece823a1493eb1a495a472016cfa94df4470d48064fcef301668efdb137`
- Rust candidate source: `5478015f7998be5335a7019915af558da5c74b4b`, release binary
  SHA-256 `13785b388787fe87f370f608f0f69392288ec0631ba8210e410aa317441130ce`.
  This identity was built and validated but the binary has not been measured.

The remote service at PID 425 was not stopped, restarted, or used as a
candidate. All task listeners and owned processes were reaped after each run.

## Fixture-direct W1 qualification

Result root: `slice2-fixture-calibration-final/`; the raw index SHA-256 is
`f4cd0ef8a5d7ddd6aba6e029149c087ccd01aa30313eb9d9e14a3c75488f6fac`, and its
sidecar SHA-256 is
`7f5f1911872364551397fd200abd59e9d0e807ac5a7e1ab7366acef7d6ba803d`.
`sha256sum -c raw-file-hashes.sha256` passed locally after copying the durable
result tree.

The final qualification used two repeats, five seconds of cooling, `GOMAXPROCS=1`,
fresh TCP connections, a 500 ms request budget and 100 ms late drain. Each
repeat ran 200/300/350/400 QPS normal/common/near/overload, 200 QPS recovery,
and a 500 QPS peak envelope. The peak is exactly 1.25 times the proposed
official maximum of 400 QPS. Both repeats passed sender, fixture, correctness,
resource-sample, and cleanup checks; scheduled, sent, received, and correct
counts matched for every stage and maximum sender lag was zero.

The qualification exercised two complete official-length ladders before the
peak envelope, not just a single peak point. It recorded ephemeral ports
`32768-60999` (28,232 ports), `tcp_tw_reuse=2`, `tcp_fin_timeout=60`, at most
one established connection, final TIME_WAIT 8,544, fixture max RSS 11,916 KiB,
fixture max FD 8, and no dial errno. These values are copied into the manifest
calibration envelope; they are safety evidence, not capacity claims.

## Go-only reference pilot

Durable raw results are under `slice2-go-pilot/`; the 264-file hash index is
`slice2-go-pilot-file-hashes.sha256` with SHA-256
`a4310070842abc010cb0275d7f3a1d2645735abbe5f8d63ad10c9ce8d1e4f086`, and the
index verifies from the research directory.

The three W1 TCP pilots used one 3-second continuous ladder at 200/300/350/400
QPS plus recovery. The three W2 pilots used independent-prefilled 1-second
measured points at the same rates, with a fresh cold point and TTL check for
each point. All six runs passed DNS correctness, sender schedule, resource
sampling, W1 session counters, W2 per-key TTL, and W2 upstream zero-increment
checks. W2 recovery is explicitly indeterminate because each point is an
independent prefilled lifecycle.

Observed Go-only maxima used to form the manifest health bands:

| Scope | max p95 (µs) | max p99 (µs) | frozen band |
| --- | ---: | ---: | ---: |
| W1 TCP normal/recovery | 876 | 1,933 | 1,200 / 2,500 |
| W1 TCP overload (reported, not a recovery gate) | 700 | 6,566 | no overload threshold inferred |
| W2 measured warm stages | 373 | 963 | 800 / 2,000 |
| W2 cold correctness point | 337 | 1,642 | included only as correctness evidence |

The bands are rounded above Go-only observations with explicit sampling margin.
They are technical health guards for the final same-rate stage, not product SLA,
capacity, or a claim that the overload point is a real saturation boundary.

## Manifest candidate

`official-manifest-v1.json` and its sidecar pin the exact fixed corpus, runner,
helper source/tree/binary, both candidate identities, host/toolchains, CPU masks,
200/300/350/400 common points, 3-second stages, W2 independent-prefilled
lifecycle, thresholds, calibration envelope, resource ceilings, profiler
separation, and alternating `go/rust`, `rust/go`, `go/rust` pair order. The
manifest has `official_frozen=true` because the current helper validator
requires that value; operationally it is not authorized for a candidate run
until the health-basis remediation is accepted by C2C for this exact content
and SHA-256
`2396817f92449825046cd6280a7d9da2f3709c8ba5790bfd31461411638fd63b`.

The manifest validator was run on the remote host against the current runner,
helper, Go binary, Rust binary, and both scenario plans without starting either
candidate; all four candidate/scenario combinations passed. The official
runner now invokes `reliability-run` for every stage, writes a separate
`reliability-assess` result, and projects only compatibility views into the
legacy stage/counter/TTL files. It enforces the typed execution contract,
512-FD/256-in-flight/512-queue/256-MiB-role/768-MiB-combined/1-GiB-free-disk
budgets, and stops/preserves the attempt when evidence, sender, correctness,
or resource checks fail. W2 cold-session and independent-prefilled correctness
failures stop the sequence before warm or subsequent points. Official results
must use a new result root and retain every attempt. The official reliability
record budget is frozen at 32768 bytes so every 3-second 200/300/350/400 stage
fits the helper's 64 MiB control budget.

The terminal same-process health gate is now explicitly pinned to
`dispatch-to-finish`, matching the pilot-derived W1 health band. The primary
`planned-slot-to-finish` summary remains available for measurement and is not
silently reused as the health gate; see `c2c-health-basis-remediation.md`.
