# S6 validation — whole-chain and public evidence

Status: S6 implementation/evidence complete; cumulative C2C review PASS.

Completed ordered checks on the current dirty source:

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check   PASS
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets -- -D warnings   PASS
TMPDIR=/private/tmp/mosdns-rust-tests cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host
native-host library: 181 passed, 0 failed, 4 ignored
native-host integration: all completed tests passed except client_context::real_udp_trusted_peers_forged_ecs_and_audit_off;
  this environment cannot bind 127.0.0.2:0 (AddrNotAvailable)
node --test webui-log/tests/*.test.mjs   29 passed, 0 failed
npm run build; npm run build:log1   PASS, sequentially
```

The native-host suite must use a canonical temporary root on macOS: the
platform `temp_dir()` commonly begins with `/var`, a system symlink, and the
state/transaction security contract intentionally rejects symlink path
components. With `TMPDIR=/private/tmp/mosdns-rust-tests`, all 181 native-host
library tests pass; four subprocess probes are intentionally ignored by the
parent-driven harness. A default macOS run is not treated as a product failure,
but remains recorded as an environment-sensitive check.

The ordered `scripts/build-rust-native.sh` run rebuilt both embedded bundles
before the locked native release binary. The final4 build reports version
`dev`, eight embedded asset inputs, source digest
`0eedf67f3c5851ea4a5401f68ceb610da4a577ea8b6dbda524c503636c80077c`, and
artifact SHA-256
`c52e43bfe701217ca2c9188c8a5139724d88ae227a9392b739202f2b22a5992f`.
The ordered source manifest retains all 225 input paths and hashes.

The ordered exact-source native release build is recorded in
[native-release-manifest-final.json](evidence/native-release-manifest-final.json),
with 225 source inputs, eight embedded assets, artifact SHA-256
`c52e43bfe701217ca2c9188c8a5139724d88ae227a9392b739202f2b22a5992f`, and
CLI identity `dev`; the per-path evidence is in
[native-source-manifest-final.json](evidence/native-source-manifest-final.json).
Controlled same-origin browser flows against the native artifact
covered both `/` and `/log`, a custom encoded tag, exact whitespace/newline
readback, stale generation rejection, and a startup read-only owner. A local
controlled DNS peer answered through the binary listener; all-seventeen
declaration/admission and real HTTP/DNS integration tests passed. See
[browser/DNS proof](evidence/browser-dns-proof.md).

The first cumulative remediation review found P1-4, P2-10, and P2-11. P1-4
now keeps rebind eligibility changes on a candidate owner copy until
publication and rebases values admitted during the management drain; P2-11
now parses generation preconditions only for a configured native switch route;
P2-10 now has an ordered per-path manifest plus its file hash. A subsequent
exact-snapshot re-review passed before the post-drain regression fix. The final
9-path delta review also returned `FINAL: PASS` in the fresh C2C conversation
https://chatgpt.com/c/6ac45d74-daf4-83ee-8b11-4d70ae7db541, binding BASE
`115b62cd19e86957b85200ba30fc70f72add8a70`, HEAD
`22d15b20e1314f655c43a8d9affae277e15653d3`, and TREE
`f11c8267aad09e96c824393c3ced96685a561757` (9 paths, 15,043 bytes). This
delta TREE is the final4 exact tree. Production/default release, deployment,
push, or cutover are not claimed here.
