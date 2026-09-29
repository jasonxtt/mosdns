# Linux validation on `mosdns-rust` (isolated)

Authoritative Linux validation for `09-28-rust-native-domain-set-management`,
run on the `mosdns-rust` SSH alias (`10.0.0.92`) only. Nothing here touches the
live service (pid 425, `/usr/local/bin/mosdns -d /cus/mosdns`), any live port,
or `/cus/mosdns`.

Validated revision: `fab8b682d8365857472603b6631eca279b096d4d` (branch `rust`, the second-round
review remediation commit; `git diff --name-only HEAD -- rust/ webui-log/` was empty, so this is
exactly the validated code).
Source archive SHA-256: `2e396315c0bb4f620d9257353e8e6d56c75bec2a92b5bd7f0f7c8c706567418f`.

Two earlier runs are superseded history: revision `03bdc5b8` (archive `cc32b49a…`, the first run
with its 17/20 and Time-Wait probe correction) and revision `8f7b439d` (archive `b9b829a1…`, the
round-1 remediation). The `evidence/` files are the latest run, listed below.

## Re-run (round 2, 2026-09-29)

Re-run after the round-2 review finding (the cross-tag file-conflict fix). Same root layout and
commands with `R=/root/mosdns-rust-domainset-v3-20260929`; the remote root was removed after the
evidence was copied. Results were identical to the round-1 re-run: `cargo fmt` exit 0, clippy exit 0
with no warnings, `cargo test --workspace` exit 0 across 64 `test result: ok` binaries with 0
failures / 0 `error` / 0 `warning`, `cargo build -p mosdns-native-host` exit 0, Vue `npm ci` +
`npm run build` exit 0 with the source snapshot assets byte-identical and the new reconcile state in
the bundle, and the real-process HTTP+DNS proof 20/20.

## Environment

| Fact | Value |
| --- | --- |
| VM | `mosdns-rust` = `10.0.0.92`, Debian GNU/Linux 13 (trixie), x86_64 |
| Toolchain | rustc/cargo 1.95.0, rustfmt 1.9.0-stable, node v20.19.2, npm 9.2.0 |
| Disk before | `/` 24G with **0 available (100 %)** — prior task scratch dirs under `/root` |
| Disk after cleanup of those scratch dirs | 15G available |
| Live service | untouched: pid 425 still up, `/cus/mosdns` mtime unchanged |

Before this run the root filesystem was full. Six stale scratch build
directories left by earlier tasks were deleted after verifying each resolved
path is a directory under `/root`:

```
/root/mosdns-rust-phase5a-native-query-observability-545ba29   9.5G
/root/mosdns-phase5b-slice3-20260927                           1.8G
/root/mosdns-rust-phase5a-first-native-performance-605c305     1.4G
/root/mosdns-rust-build                                        879M
/root/mosdns-rust-slice4-linux-20260920                        460M
/root/mosdns-rust-phase5a-measurement-reliability-20260927      400M
```

Node/npm were absent; `apt-get install -y nodejs npm` installed Debian trixie's
node 20.19.2 / npm 9.2.0 (which satisfies Vite 7's `^20.19.0` requirement).

## Exact commands

Local packaging and transfer (from `/Users/tom/github/mosdns-rust`):

```bash
HEAD_SHA=$(git rev-parse HEAD)                      # 03bdc5b8dfcba4ee5d50ecc1f8b1e19ad03ad8b7
tar czf /tmp/mosdns-domainset-src.tgz --exclude='rust/target' \
  rust tests/phase5a-baseline \
  webui-log/src webui-log/src-log1 webui-log/index.html webui-log/log1.index.html \
  webui-log/package.json webui-log/package-lock.json \
  webui-log/vite.config.js webui-log/vite.log1.config.js \
  coremain/www \
  .trellis/tasks/09-28-rust-native-domain-set-management/research/linux-validation
ssh mosdns-rust 'mkdir -p /root/mosdns-rust-domainset-20260929/logs'
scp /tmp/mosdns-domainset-src.tgz mosdns-rust:/root/mosdns-rust-domainset-20260929/
ssh mosdns-rust 'cd /root/mosdns-rust-domainset-20260929 && mkdir -p src && \
  tar xzf mosdns-domainset-src.tgz -C src && sha256sum mosdns-domainset-src.tgz'
```

Rust gates (remote):

```bash
R=/root/mosdns-rust-domainset-20260929
cd $R/src/rust
export CARGO_TARGET_DIR=$R/target
cargo fmt --all -- --check                                  # exit 0
cargo clippy --workspace --all-targets -- -D warnings       # exit 0
cargo test --workspace                                      # exit 0
```

Vue build in a disposable copy (remote):

```bash
mkdir -p $R/vue-build
cp -a $R/src/webui-log  $R/vue-build/webui-log
cp -a $R/src/coremain   $R/vue-build/coremain
cd $R/vue-build/webui-log
npm ci --no-audit --no-fund        # exit 0, 38 packages
npm run build                      # exit 0, 615 modules, built in 10.30s
```

Real-process HTTP + DNS functional proof (remote):

```bash
cd $R/src/rust && cargo build -p mosdns-native-host
bash $R/src/.trellis/tasks/09-28-rust-native-domain-set-management/research/linux-validation/remote-functional.sh \
  $R/target/debug/mosdns $R/functional
```

## Remediated re-run (v2, 2026-09-29)

Re-run after the review remediation (non-blocking persistence, tag-existence-before-method HTTP
ordering, UI preserved-vs-adjusted states, approved PRD amendments A1/A2). Same remote root layout
and the same commands as above with `R=/root/mosdns-rust-domainset-v2-20260929`; the remote root was
removed after the evidence was copied.

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0, no warnings |
| `cargo test --workspace` | exit 0, **64 `test result: ok` binaries, 0 failed, 0 `error`, 0 `warning`** |
| `cargo build -p mosdns-native-host` | exit 0 |
| Vue `npm ci` + `npm run build` (disposable copy) | exit 0, source snapshot assets byte-identical |
| Built bundle contains the new reconcile state | `已保留本地编辑` occurs twice in the built `app.js` |
| Real-process functional proof | **20/20 passed**, exit 0 |

## Results (first run, revision 03bdc5b8; superseded)

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0, no warnings |
| `cargo test --workspace` | exit 0, **64 test binaries with `test result: ok`, 0 failed, 0 `error`, 0 `warning`** |
| Vue `npm ci` + `npm run build` (disposable copy) | exit 0; `app.js` 776.07 kB, `app.css` 100.02 kB, `index.html` 0.37 kB |
| The disposable copy's build did not touch the source snapshot | `sha256` of `src/coremain/www` identical before/after |
| The built bundle contains the new reconcile logic | `当前内容未确认` occurs twice in the built `app.js` |
| Real-process functional proof | **20/20 checks passed**, exit 0 |

Full check list and transcripts: `evidence/functional.log`, `evidence/rust-fmt.log`,
`evidence/vue-build.log`, `evidence/rust-gates-summary.txt`. The committed
transcripts have trailing whitespace stripped so `git diff --check` stays clean;
no other byte was changed.

The functional proof covers: scoped API start, `/api/v1/special-groups` =
`[]` with `application/json`, `/show?limit=10000` ignoring the query string,
404 for an unknown tag, 400 for a configured query-only tag (`explist`), 405 for
a wrong method, `400 invalid JSON` with the file unchanged, POST publishing
`domain_set replaced with 1 entries` with only accepted rules written, `/save`
returning an empty 200, the next real UDP query changing (rcode 3) while an
unrelated name stays 0, a restart keeping both the generation and the DNS
effect, and both listeners released afterwards.

## Corrections during this validation

1. **First functional run: 17 passed / 1 failed** — `the management port is
   released after shutdown` failed because the probe used a plain `bind()` while
   the closed HTTP session was still in `TIME_WAIT`. The product is correct:
   Tokio sets `SO_REUSEADDR`, so a real rebind works. The probe was fixed to set
   `SO_REUSEADDR` and, as the primary assertion, to require that `ss` shows no
   listener on the port. Re-run: **20/20 passed**. This is the same TIME_WAIT
   nuance the project already records for repeated TCP sessions.
2. The VM had no `node`/`npm` and a full root filesystem; both were resolved as
   described above, and the stale directories were removed only after verifying
   each path.

## Cleanup

After the evidence was copied into this directory, the remote root
`/root/mosdns-rust-domainset-20260929` (5.8 G, mostly the cargo target
directory) was removed: `rm -rf -- /root/mosdns-rust-domainset-20260929`.
Disk returned to 14 G available; no `mosdns-rust-domainset-*` directory remains
on the VM; the live service is still pid 425.
