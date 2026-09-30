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
