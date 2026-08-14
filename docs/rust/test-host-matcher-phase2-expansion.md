# Rust matcher Phase 2 expansion test-host record

Date: `2026-08-14`

Host verification was run on isolated `mos-test` (`10.0.0.91`, Linux x86_64)
from `/tmp/mosdns-rust-slice5.58Ydys`. The installed MosDNS service, its
configuration, port 53, and production hosts were not touched.

## Gates passed

```text
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo test --manifest-path rust/Cargo.toml --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml --all-targets --locked -- -D warnings
scripts/build-rust-cache.sh

CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust \
  go test -tags mosdns_rust \
  ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set \
  ./plugin/data_provider/sd_set ./plugin/data_provider/si_set \
  ./plugin/data_provider/domain_mapper ./plugin/data_provider/matcher_adapter \
  ./plugin/matcher/...

CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust \
  go test -race -tags mosdns_rust \
  ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set \
  ./plugin/data_provider/sd_set ./plugin/data_provider/si_set \
  ./plugin/data_provider/domain_mapper ./plugin/data_provider/matcher_adapter \
  ./plugin/matcher/base_domain ./plugin/matcher/base_ip

CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust BENCHTIME=250ms COUNT=3 \
  scripts/benchmark-rust-matchers.sh

OUTPUT=/tmp/mosdns-rust-slice5.58Ydys-bin-full \
  scripts/build-rust-experimental.sh

CGO_ENABLED=0 SKIP_UI_BUILD=1 \
  OUTPUT=/tmp/mosdns-rust-slice5.58Ydys-go-only scripts/build-local.sh
```

The checked-in header gate and the full `valued_abi` integration test also
passed. `npm ci`, both Vue builds, and the experimental host artifact build
with embedded UI assets passed; the result was a Linux amd64 ELF.

## Isolated `mos-test` smoke

```text
MOSDNS_RUST_BINARY=/tmp/mosdns-rust-slice5.58Ydys-bin-full \
MOSDNS_GO_ONLY_BINARY=/tmp/mosdns-rust-slice5.58Ydys-go-only \
  scripts/smoke-rust-matcher-mos-test.sh
```

The smoke passed with random loopback API/DNS ports and temporary rules. It
covered:

- `domain_set` and `ip_set` positive/negative matching;
- `domain_mapper` overlap (domain + keyword), audit source metadata, provider
  reload, and restart;
- concurrent query/reload while replacing provider rules;
- malformed reload retaining the previous valid generation;
- Rust-selected startup with the no-cgo Go-only binary, proving the unavailable
  Rust path falls back without changing the query result;
- Rust → Go-only restart → Rust restart behavior.

The script cleans its temporary directory and process on exit. It checks the
mapper's joined source names through the public audit API and uses DNS answers
as the process-level observable; it does not inspect private runtime state.

## Remaining boundary

This is host evidence for the experimental bridge, not a production rollout
approval. Default builds remain Go-only. macOS arm64 cannot execute these real
Linux+cgo and process checks.
