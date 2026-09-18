# Research evidence — native dual-stack endpoint selection

This is source-backed planning evidence, not an implementation contract by
itself. The PRD and design are authoritative for this task.

## Current Go configuration and runtime path

- `plugin/executable/forward/forward.go:54-85` declares `bootstrap` and
  `bootstrap_version` at both global and per-upstream config levels.
- `plugin/executable/forward/forward.go:131-165` applies global defaults and
  passes the effective values into `upstream.Opt`.
- `pkg/upstream/upstream.go:103-110` documents the current values as `0,4,6`,
  with `0` defaulting to `4`, and records dual-stack as a TODO.
- `pkg/upstream/upstream.go:169-175` parses the bootstrap peer as a numeric
  address. `pkg/upstream/upstream.go:177-209` uses it for hostname-based UDP
  address resolution, while `pkg/upstream/upstream.go:213-266` uses it for
  TCP/TLS hostname dialing.
- `pkg/upstream/upstream.go:353-474` applies the TCP resolver to DoT/DoH and
  the UDP resolver to DoH3; `:483-550` applies it to DoQ.
- `pkg/upstream/bootstrap/bootstrap.go:47-69` validates the bootstrap peer and
  maps the configured version to one query type. `:138-236` performs the UDP
  query and selects an answer; `:239-247` maps `0,4` to A and `6` to AAAA.

## Current Rust foundation

- `rust/dns-core/src/resolver.rs:83-100` defines a single-family wire enum and
  maps it to A/AAAA wire types. `:197-208` exposes one selected address with
  its effective TTL.
- `rust/upstream-core/src/resolver/mod.rs:59-99` currently models the prior
  single-family `ConfigVersion` mapping. The new task must make omitted/default
  input distinct from explicit zero.
- `rust/upstream-core/src/resolver/owner.rs:427-529` currently admits one
  generation and runs one family-specific bootstrap exchange. Its lifecycle,
  cancellation, and final publication gate are the seams to reuse.
- `rust/upstream-core/src/resolver/mod.rs:647-738` exposes read-only resolver
  diagnostics and keeps state mutation crate-private; the new candidate snapshot
  must preserve that ownership rule.
- The prior task's archived `implement.md` required a separate dual-stack task
  covering A+AAAA, address selection, per-family failure state, multi-address
  cache shape, QUIC/HTTP3 interaction, and `bootstrap_version`. The user has
  narrowed that scope to A-preferred selection with no connection racing or
  connection fallback.

## Decisions recorded from the user

- Explicit `0`: obtain A and AAAA candidates and choose A when available;
  choose AAAA only when no usable A exists.
- `4`: A-only. `6`: AAAA-only.
- Omitted/default config value: `4`.
- No Happy Eyeballs, dual target connection race, or connection-failure
  cross-family fallback.
