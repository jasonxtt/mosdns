# Execution evidence — 2026-10-01

All Rust and Vue commands below ran in the isolated SSH workspace
`mosdns-rust:/root/mosdns-rust-forwarding.8UEU53`; no production process or
public DNS peer was used.

## Focused and integrated checks

- `cargo test -p mosdns-native-host --lib -- --nocapture`: 76 passed.
- `cargo test -p mosdns-native-host --tests`: all native-host unit and
  integration targets passed, including the 9 forwarding tests.
- `cargo test -p mosdns-native-host --test slice9_forwarding -- --nocapture`:
  9 passed, including controlled bootstrap, UDP TC→TCP, multi-entry selection,
  native HTTP diagnostics, synthetic-CA DoT, DoH H1, and DoH H2.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test --workspace`: passed from a clean disposable `target`, including
  all workspace tests and doctests.
- `cargo build -p mosdns-native-host`: passed.
- In `webui-log`, `npm ci`, `npm run build`, and `npm run build:log1`: passed.
  Vite emitted only the existing large-chunk advisories; `npm ci` reported
  audit advisories but no install/build failure.

## Preserved environmental failures and cleanup

The first workspace test attempt filled the remote root filesystem while
linking tests (`No space left on device`). A later retry with the existing
target reached the same disposable limit and the linker terminated with
`Bus error`. The exact remote `rust/target` was removed, leaving the source
checkout intact; the clean-target workspace run then passed. The failure is an
environment-capacity observation, not a product test failure.

The first browser-proof launch used shell background precedence incorrectly and
left only the owned UDP fixture running. The owned process was killed after
the PID/port check. The second launch used explicit `setsid` processes and
passed the browser proof; all owned processes and the SSH forwarding session
were then cleaned up.

## Browser proof

The native binary, the loopback UDP responder, and the Vite proxy served the
same real record. The browser opened the query detail for `browser.test` and
visibly showed:

- selected entry `primary`, peer `127.0.0.1:15453`, transport `udp`;
- ordered attempt `0 / primary / 127.0.0.1:15453 / udp / response`;
- final answer `192.0.2.123` and `NOERROR`.

The directly queried native endpoint returned the same schema-versioned object
from `/api/v2/audit/logs` before the browser interaction.

## Review remediation and final rerun

The dedicated c2c-web reviewer returned `FINAL: FAIL` on the first committed
range. The findings were limited to this forwarding scope: non-address
responses winning too early, stale peer/transport facts during terminalization,
over-broad fresh-connection fallback after reuse busy, retained resolver-owner
history, fixed-prefix rather than rotated bounded selection, last-error rather
than first-completed failure selection, and audit-off identity allocation.

The follow-up range addresses those findings with A/AAAA response priority,
one live RAII attempt ledger plus an explicit UDP/TCP phase hook, typed
`Backpressure(NotSent)` busy handling, scoped owner retention, seeded/injectable
rotation, first-failure preservation, and ID/enumeration-based audit-off
metrics. The native catalog path does not materialize identity strings for
audit-off attempts.

After the fix round, the isolated SSH checkout passed the corrected final
commands:

- `cargo fmt --all -- --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`, including all workspace tests and doctests.
- `cargo build -p mosdns-native-host`.
- `npm ci`, `npm run build`, and `npm run build:log1` in `webui-log`.

One intermediate `w1_tcp` failure occurred while the remote checkout still had
an old copy of `reuse.rs` at an incorrect sync destination; the corrected
explicit source sync changed the typed busy mapping, and the focused test plus
the subsequent complete workspace run passed. This was a remote source-sync
failure, not a product regression. The earlier remote capacity failures and
their cleanup remain recorded above.

The latest browser proof used the updated binary and controlled loopback
upstream. The browser accessibility state showed selected `primary`, peer
`127.0.0.1:15453`, `udp`, ordered attempt `0 / primary / 127.0.0.1:15453 /
udp / response`, answer `192.0.2.123`, and `NOERROR`. The owned processes and
SSH tunnel were then stopped; the final port check returned
`proof-pids-cleaned`.

The second dedicated-review recheck returned `FINAL: FAIL` with two remaining
scope findings. The current follow-up keeps the same shared ledger handle in
the `ExecutionCheckpoint` for the complete async invocation instead of moving
it into an invocation-local owner; abnormal future drop therefore leaves the
started slots, live peer/transport, and RAII outcome visible to checkpoint
terminalization. Ledger terminalization is now guarded as exactly-once, and
response qualification occurs before a transport tracker is terminalized as a
successful response, so a correlation-invalid leg cannot overwrite a finished
slot.

The same follow-up applies the seeded/entropy-backed rotation before the
single-leg branch, so `concurrent: 1` rotates across the original entry order
instead of always selecting entry zero. It adds deterministic seeded coverage
for both single-leg and bounded adjacent selection and a real dropped active
three-leg ledger test. The corrected native-host library run passed 78/78 and
the existing slice9 forwarding run passed 9/9 after these changes.
