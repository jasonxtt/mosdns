# Design — isolated Linux sidecar canary

## Boundaries and preconditions

- Candidate source is the reviewed commit `016103f3c21ed2d659694ce10e64aaf24b5c2767`; do not build from the dirty checkout or a later `HEAD`.
- The existing `rust/Cargo.lock` is tracked at that commit. Local Git verified blob `7dc20cd5fbe4e90adc9a3c0ef3b15035c31dee5f`, byte length `38673`, SHA-256 `d78f204b017fc01f316e5af84e23bef30ef0597ea56952aa87c062fe8a51b6ca`, and `git archive 016103f3c21ed2d659694ce10e64aaf24b5c2767 rust/Cargo.lock | tar -tf -` listed `rust/Cargo.lock`. Transfer a clean source archive through the `mos-test` SSH alias; verify archive hash at both ends, then verify this lockfile blob/hash after extraction before building the native host once with `--locked`. If any lockfile check differs, the remote toolchain is absent, or target identity is wrong, stop before starting any sidecar; do not install packages.
- The read-only source snapshot identifiers in `research/canary-inputs.md` were captured from a sibling `file` repository at commit `28c64936a0a1889a02dec4617e258e01d5501866`. That repository and its bytes are outside the exact-range C2C review, so those identifiers are planning-captured inputs, not facts independently verified by that review. Recheck the package commit and exact file hashes from the accessible read-only snapshot at execution. If they changed or are inaccessible, stop and revise the plan rather than substituting live `/cus/mosdns` state or recreating the source from memory.
- The four proposed execution defaults are: use a read-only config-package snapshot; use same-config Go only as a safe, non-gating optional comparison (no Go build); use deterministic owned peers as the hard oracle and skip public resolver dependence; treat TERM plus complete owned-resource release as rollback, without claiming graceful shutdown. The user must approve execution and may change these defaults before the canary starts.

## Isolation topology

```text
existing mos-test MosDNS service (baseline only; untouched)

canary-owned UDP/TCP DNS peers (127.0.0.1, high ports, counted requests)
                    ↑
new /tmp/mosdns-rust-canary.<unique>/
  source archive → exact-source build → one mosdns binary
  config/udp.yaml → Rust UDP listener, audit enabled → Q1–Q6
  stop/check cleanup
  config/tcp.yaml → Rust TCP listener, audit disabled → Q1–Q6
  stop/check cleanup → capture compact evidence → remove root
```

Use one sidecar listener at a time because each config declares one listener. Keep local/default peer processes alive across both sequential runs, but reset their counters before each run. Every peer and listener binds only `127.0.0.1` on independently selected, unoccupied high ports. Recheck availability immediately before start; on a collision, choose a different port before the process starts. Port 53 is forbidden.

All canary-owned files (source, binary, configs, relative rules, peer helper, logs, PID receipts, and temporary evidence) stay under the unique remote temp root. A small local source archive used for transfer is also temporary and removed after verification. Never write under `/cus/mosdns`; do not change service units, service config, firewall, or traffic routing.

## Build and provenance

1. Create a source archive from the exact commit, limited to the Rust workspace, and record its SHA-256.
2. Transfer it using `scp` with the `mos-test` alias into the new remote temp root. Verify the remote SHA-256 before extraction.
3. Record `rustc --version` and `cargo --version`; build exactly once:

   ```sh
   CARGO_TARGET_DIR="$ROOT/target" cargo build \
     --manifest-path "$ROOT/source/rust/Cargo.toml" \
     -p mosdns-native-host --release --locked
   ```

4. Record the committed lockfile SHA-256, build exit status, binary SHA-256, byte size, and ELF architecture. Copy the binary only within the temp root if a shorter runtime path is useful. Do not rebuild for UDP/TCP or optional Go comparison.

This source-only Rust build is the intended path: the current host has no standalone native-host build script, and `scripts/build-rust-experimental.sh` builds the transitional Go/cgo executable, so it is out of scope.

## Configuration reduction

At execution time, recheck the read-only config-package commit and source-file hashes frozen in `research/canary-inputs.md`. Keep raw/private config out of the repository. Derive two temporary YAMLs and rules under the canary root using this reduction. Freeze the layout and path literal exactly as:

```text
config/udp.yaml                 # include: sub_config/routes.yaml
config/tcp.yaml                 # same graph, TCP listener/audit setting only
config/sub_config/routes.yaml   # files: [rules/local.txt]
config/sub_config/rules/local.txt
```

The included `routes.yaml` declaration resolves `rules/local.txt` relative to `config/sub_config/`, its own directory. The derived root config keeps one top-level relative `include` to its sanitized `sub_config/routes.yaml`; it does not copy the package's full include list or claim that the full include graph is supported.

Before any listener starts, add a temporary integration-test harness under the extracted temp source tree (never the repository or candidate archive) that calls `HostAssembly::from_config_file` for both derived configs and drops each assembly without calling `run()`. `from_config_file` performs config compilation and constructs the pre-I/O graph; `run_udp` / `run_tcp` are the calls that bind listeners. Record the harness source hash and `cargo test --locked` result. This check must prove the include and relative `files` paths resolve before any sidecar listener is launched. A failure here is `STOP / fixture-or-harness invalid` until the sanitized paths are corrected; it is not a Rust runtime canary failure.

| Source behavior | Canary treatment |
| --- | --- |
| Top-level `include` and relative path resolution | Preserve; included file remains under the config directory |
| Relative file-backed provider convention (`rule_set.yaml` uses relative paths) | Preserve path resolution and multi-rule behavior in a sanitized supported `domain_set` fixture; do not claim `domain_set_light` compatibility or copy live rule data |
| qtype 65 → reject 0; blocked domain → reject 3 | Preserve |
| main → routed child → local child; parent continuation → default child | Preserve ordering and direct named calls |
| child cache → local forward | Preserve; use a small positive cache size and no EDNS in the probe corpus |
| default route | Preserve; route to a second controlled peer |
| Public `aliapi` upstream | Replace with canary-owned deterministic forward peers |
| `switch`, `flow_setter`, `special_groups`, lazy/dump, UI/API, unrelated protocols | Remove/defer; do not treat this slice as validating them |

The source package's `rule_set.yaml` references relative rule paths for providers including `whitelist` and `blocklist`, but those declarations use `domain_set_light`, which is outside this canary's supported slice. The temporary `config/sub_config/rules/local.txt` is a sanitized test fixture with `domain:local.test` and `full:local.only.test`; its path is resolved relative to the declaring included YAML `config/sub_config/routes.yaml`. This proves relative file resolution and the desired suffix/exact rules without copying production lists or claiming support for the original plugin type.

The UDP and TCP configs must otherwise share identical sequences, provider data, cache, and forwarding semantics. Only listener transport/address and audit value vary; the TCP listener also carries the positive `idle_timeout` required by the parser. Set UDP `enable_audit: true`; set TCP `enable_audit: false`.

## Controlled peers and query oracle

Run two canary-owned deterministic upstreams with request logs/counters:

| Peer | Transport | Deterministic response |
| --- | --- | --- |
| local peer | UDP | Echo question and transaction ID; one A answer `192.0.2.21`, TTL 60 |
| default peer | TCP | Echo question and transaction ID; one A answer `192.0.2.22`, TTL 60 |

Use a minimal temporary DNS probe that sends no EDNS, supports both UDP queries and length-prefixed TCP queries, selects known IDs, and records normalized response fields. This avoids dependence on public DNS and lets the cache-repeat assertion verify ID rewriting. Each peer logs the received question and increments a counter exactly once per request.

Before starting either Rust sidecar, run an independent peer-only self-test: send one known request directly to the local UDP peer and one to the default TCP peer. Verify UDP/TCP framing, echoed transaction ID and question, expected `.21`/`.22` answer, and exactly one corresponding counter increment. Then reset both counters to zero and record the reset before Q1. If this self-test fails, classify the run as `STOP / harness invalid`; do not use its results to mark Rust `FAIL`.

Run this ordered corpus against both sidecar configs. Reset counters before each run and record deltas after every request:

| Case | Request | Expected wire result | Local peer delta | Default peer delta |
| --- | --- | --- | ---: | ---: |
| Q1 | `blocked.test A` | RCODE 3 (NXDOMAIN), no answers | 0 | 0 |
| Q2 | `other.test HTTPS` (type 65) | RCODE 0 (NOERROR), no answers | 0 | 0 |
| Q3 | `a.local.test A`, first ID | `192.0.2.21` | +1 | 0 |
| Q4 | same name/type, different ID | `192.0.2.21`, response ID matches Q4; TTL valid and no greater than Q3 | 0 | 0 |
| Q5 | `local.only.test A` | `192.0.2.21` via the exact rule | +1 | 0 |
| Q6 | `other.test A` | `192.0.2.22` through parent continuation/default | 0 | +1 |

Check question name/type, response ID, RCODE, answer addresses/count, and legal positive TTL. Do not assert an exact wall-clock TTL decrement. For Q4, unchanged peer counters are the primary cache-hit oracle. Run each case with a bounded query timeout and record unexpected timeout/SERVFAIL distinctly.

The CLI exposes no audit snapshot/API or stdout audit stream. Record that each audit configuration starts and produces the expected external query/peer results; do not claim that sidecar audit records were externally inspected. Since UDP/audit-on and TCP/audit-off intentionally form two different transport/config combinations, their agreement does not isolate a causal audit-toggle effect. The existing in-process composition tests remain the evidence for retained audit records on/off.

An already available Go binary may be used only if the same reduced config and controlled peers can be run in a fully isolated, read-only-safe process without building Go or touching the existing service. Compare semantic fields and peer deltas, not DNS bytes, exact TTL seconds, logs, or latency. If unavailable, record why and continue; it is non-gating by default. Do not use a differently configured existing service or a public resolver as the golden oracle.

## Rollback and result classification

Before starting peers, record the relevant existing MosDNS service active state/MainPID and listener snapshot. Keep the owning shell alive while its child processes run. For every canary-owned process, record PID, `/proc/<pid>/stat` starttime (field 22), resolved `/proc/<pid>/exe`, PPID, process group, and exact command/temp-root identity at launch. Immediately before TERM or KILL, verify PID still exists and starttime, executable, PPID, and process group match the launch receipt. If the PID vanished, only wait/record it; never signal that number. If any identity field changed, do not signal it and stop for investigation. After each run, send TERM only to a process whose receipt still matches and allow up to 10 seconds for exit. If a matching owned process remains, send KILL only after repeating the full identity check, record the run as FAIL, and continue cleanup. Verify all canary-owned PIDs and ports are absent, no canary-owned listener used port 53, any baseline port-53 listener (if present) still has the same owner/state, and the existing-service PID/state/listeners match baseline. Preserve only compact hashes, command/result tables, and sanitized failure excerpts in task evidence; then remove the temporary root.

- **PASS:** provenance and isolation are proven; both six-query runs match all wire and peer oracles; no unexpected error/crash; all canary resources are reclaimed; service baseline is unchanged; all unrun work and limitations are explicit; no performance claim is made.
- **FAIL:** valid preflight followed by a Rust startup/wire/route/cache/counter/crash/cleanup failure, source-provenance mismatch, or any canary action affecting the existing service.
- **STOP / environment invalid:** wrong host, unsafe/occupied isolation, unavailable source snapshot, absent preinstalled Rust toolchain, unrelated service instability, or unavailable harness/peer prerequisites before a sidecar starts. Do not label these as Rust product failures.
- **Separate remediation:** if a valid run exposes a product defect, capture evidence and stop. Do not change product code in this canary task.
