# Design — isolated Linux sidecar canary

## Boundaries and preconditions

- Candidate source is the reviewed commit `016103f3c21ed2d659694ce10e64aaf24b5c2767`; do not build from the dirty checkout or a later `HEAD`.
- The existing `rust/Cargo.lock` is tracked at that commit (`git ls-tree` confirms it). Transfer a clean source archive through the `mos-test` SSH alias, verify its hash at both ends, and build the native host once on `mos-test` with that lockfile and `--locked`. If the remote toolchain is absent or the target identity is wrong, stop before starting any sidecar; do not install packages.
- The read-only source snapshot is present in the sibling `file` repository at commit `28c64936a0a1889a02dec4617e258e01d5501866`; the relevant config hashes are recorded in `research/canary-inputs.md`. Recheck those exact files at execution. If they changed or are inaccessible, stop and revise the plan rather than substituting live `/cus/mosdns` state or recreating the source from memory.
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

At execution time, recheck the read-only config-package commit and source-file hashes frozen in `research/canary-inputs.md`. Keep raw/private config out of the repository. Derive two temporary YAMLs and rules under the canary root using this reduction. The derived root config keeps one top-level relative `include` to its sanitized `sub_config/routes.yaml`; it does not copy the package's full include list or claim that the full include graph is supported.

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

The source package's `rule_set.yaml` references relative rule paths for providers including `whitelist` and `blocklist`, but those declarations use `domain_set_light`, which is outside this canary's supported slice. The temporary `rules/local.txt` is a sanitized test fixture with `domain:local.test` and `full:local.only.test`; its path is relative to the temporary config root. This proves relative file resolution and the desired suffix/exact rules without copying production lists or claiming support for the original plugin type.

The UDP and TCP configs must otherwise share identical sequences, provider data, cache, and forwarding semantics. Only listener transport/address and audit value vary; the TCP listener also carries the positive `idle_timeout` required by the parser. Set UDP `enable_audit: true`; set TCP `enable_audit: false`.

## Controlled peers and query oracle

Run two canary-owned deterministic upstreams with request logs/counters:

| Peer | Transport | Deterministic response |
| --- | --- | --- |
| local peer | UDP | Echo question and transaction ID; one A answer `192.0.2.21`, TTL 60 |
| default peer | TCP | Echo question and transaction ID; one A answer `192.0.2.22`, TTL 60 |

Use a minimal temporary DNS probe that sends no EDNS, supports both UDP queries and length-prefixed TCP queries, selects known IDs, and records normalized response fields. This avoids dependence on public DNS and lets the cache-repeat assertion verify ID rewriting. Each peer logs the received question and increments a counter exactly once per request.

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

Before starting peers, record the relevant existing MosDNS service active state/MainPID and listener snapshot. Record each canary-owned PID and exact command/temp-root identity. After each run, send TERM only to those PIDs and allow up to 10 seconds for exit. If any owned process remains, send KILL only to that PID, record the run as FAIL, and continue cleanup. Verify owned PID absence, owned ports absent, no port 53 listener, and unchanged existing-service PID/state/listeners. Preserve only compact hashes, command/result tables, and sanitized failure excerpts in task evidence; then remove the temporary root.

- **PASS:** provenance and isolation are proven; both six-query runs match all wire and peer oracles; no unexpected error/crash; all canary resources are reclaimed; service baseline is unchanged; all unrun work and limitations are explicit; no performance claim is made.
- **FAIL:** valid preflight followed by a Rust startup/wire/route/cache/counter/crash/cleanup failure, source-provenance mismatch, or any canary action affecting the existing service.
- **STOP / environment invalid:** wrong host, unsafe/occupied isolation, unavailable source snapshot, absent preinstalled Rust toolchain, unrelated service instability, or unavailable harness/peer prerequisites before a sidecar starts. Do not label these as Rust product failures.
- **Separate remediation:** if a valid run exposes a product defect, capture evidence and stop. Do not change product code in this canary task.
