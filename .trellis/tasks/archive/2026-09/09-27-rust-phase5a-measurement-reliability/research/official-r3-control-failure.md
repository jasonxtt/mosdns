# Official-r3 W1-TCP control failure

After the health-basis C2C PASS, the official-r3 attempt began in a fresh
remote result root with the reviewed manifest and helper identities. The
prearranged W1-TCP order was Go/Rust, Rust/Go, Go/Rust.

The first four attempts completed all five raw stages and passed the evidence,
load, correctness, sender, resource, and same-process health gates. Their
dispatch-to-finish normal/recovery health p95/p99 values were:

| Attempt | Normal | Recovery |
| --- | ---: | ---: |
| Go r1 | 918 / 1358 µs | 1011 / 2113 µs |
| Rust r1 | 738 / 1433 µs | 631 / 1166 µs |
| Rust r2 | 623 / 1266 µs | 659 / 1329 µs |
| Go r2 | 799 / 1174 µs | 995 / 2000 µs |

Go r3 completed all five stages but its terminal health gate failed at
recovery: dispatch p95/p99 was `1231/2478 µs` against the frozen
`1200/2500 µs` W1 band. The runner recorded
`recovery\tsame-process recovery criteria failed` and preserved the full raw
attempt. No paired Rust r3 candidate was started after the control failure.

The complete W1-TCP evidence and sidecar are under
`slice3-official/official-r3-20260927/w1-tcp/`; the raw-file-hashes sidecar
SHA-256 is `c8460f6e80e55fe65995a24709fce3ad4539299f225dec98b6f20c1da44e90a4`.
This is an invalid/incomplete official attempt, not a Rust result and not a
threshold adjustment. Formal continuation is gated on a fresh C2C review of
this control failure; no W2 run, profile, or capacity conclusion is implied
by the failed control.

The subsequent C2C review returned `FINAL: PASS` for evidence handling, but
closed the next-scope question as stop-and-report: official-r2 already consumed
the one reviewed retry represented by official-r3, and the r3 Go control failure
does not permit another retry or continuation into W2. The first four W1
attempts are two valid pairs, not a complete three-pair official matrix.
