# Real combination evidence

Date: 2026-10-01 (Asia/Shanghai)

All remote commands below ran through the SSH alias `mosdns-rust` in the
task-owned isolated directory:

`/root/codex-rust-native-query-fallback-20261001-0300`

## Remote resource and ownership checks

- Preflight and final checks reported approximately 1.7 GiB free on `/root`
  and 16% inode use.
- One earlier monolithic workspace test attempt filled the task-owned Cargo
  target filesystem. Its exact task-owned test processes were stopped and its
  exact task-owned target directory was removed. That attempt is not counted
  as a passing result.
- The successful final checks used separate task-owned Cargo target
  directories and `-j 1`; no unrelated process or directory was removed.

## Rust validation

- `cargo test -p mosdns-native-host --tests -j 1`: passed. This covered 82
  native-host unit tests and all native integration suites, including the
  observability, cache, config, composition, policy, routing, TCP, UDP, and
  upstream suites.
- `cargo test -p mosdns-native-host --doc`: passed.
- `cargo test --workspace --exclude mosdns-native-host -j 1`: passed, including
  non-native unit, integration, and doctest suites.
- After the final trace-attempt guard change:
  `cargo check -p mosdns-native-host --tests`: passed;
  fallback and preference policy unit tests: passed.
- `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo build -p mosdns-native-host -j 1`: passed.

The split workspace commands cover the same workspace packages while keeping
the task-owned remote target below the available disk budget. The monolithic
workspace attempt is retained as a resource limitation, not reported as green.

## Real fallback DNS + HTTP + Vue proof

Two task-owned loopback UDP upstreams were started: primary `127.0.0.1:18531`
with a 200 ms response delay and secondary `127.0.0.1:18532` with a 10 ms
delay. The native host listened on `127.0.0.1:18553`, with audit HTTP on
`127.0.0.1:18080`.

`dig @127.0.0.1 -p 18553 example.com A` returned `192.0.2.32` from the
secondary. The HTTP audit response showed:

- `schema_version: 2`;
- root branch completed;
- primary branch `decision: canceled` with an `interrupted` attempt;
- secondary branch `decision: selected`;
- selected source `secondary_forward`, peer `127.0.0.1:18532`, transport `udp`.

The built Vue bundle was served from a task-owned static HTTP process on port
18082 and returned `HTTP/1.0 200 OK` for `/index.html`. All four task-owned
processes were killed by their exact PIDs after capture.

## Real preference proof

The native-host policy test and live native DNS/API proof captured the
AAAA-to-A reference rewrite: the original AAAA branch was suppressed and the
reference A branch completed, while the final wire response preserved the
original AAAA question and returned empty `NOERROR` when the test peer did
not provide a valid AAAA answer. The corresponding audit response carried
schema 2 branch and attempt `qtype` fields.

The live preference peer used for this proof was intentionally A-only, so its
original AAAA response was malformed and the proof does not claim a valid
AAAA upstream answer. The deterministic native test separately verifies the
QTYPE rewrite, branch correlation, suppression, and final-question behavior.

## UI build

- `npm ci`: passed; npm reported five audit findings (1 low, 1 moderate,
  3 high) in the existing dependency tree.
- `npm run build`: passed.
- `npm run build:log1`: passed.

Vite emitted only the existing large-chunk warnings. No production service,
public DNS, port 53, or installed deployment was touched.
