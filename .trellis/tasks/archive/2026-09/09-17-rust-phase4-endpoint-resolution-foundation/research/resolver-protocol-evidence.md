# Resolver/bootstrap protocol and repository evidence

Date: 2026-09-17. This file is evidence and design rationale; `prd.md` and
`design.md` are normative for the task.

## Repository contract

- Go config exposes global/per-upstream `bootstrap`, `bootstrap_version`, and
  `dial_addr`. `dial_addr` changes the network destination but not TLS SNI or
  HTTP authority (`plugin/executable/forward/forward.go:54-85,131-156`,
  `pkg/upstream/upstream.go:69-77`).
- Bootstrap is numeric IP plus optional port 53. Version `0/4` queries A and
  version `6` queries AAAA; Go explicitly marks dual-stack as TODO
  (`pkg/upstream/upstream.go:103-110`, `pkg/upstream/utils.go:77-90`,
  `pkg/upstream/bootstrap/bootstrap.go:239-247`).
- The existing Go updater uses UDP+EDNS(0), one-second retransmission, a
  five-second private query timeout, a five-minute minimum refresh, two-second
  failure retry, and keeps its last published address. These are
  characterization facts, not automatic Rust implementation requirements
  (`pkg/upstream/bootstrap/bootstrap.go:37-41,88-152,155-237`).
- Rust transport contracts already provide numeric endpoints, one absolute
  deadline, caller/owner cancellation, side-effect states, lifecycle admission,
  and a commit linearization point. Secure endpoints already expose normalized
  `ServerIdentity::dns_name()` and keep numeric dialing separate from identity.

## Protocol evidence and decisions

- RFC 1035 requires response parsing/header reasonableness, full RR framing,
  request matching, and permits limiting excessively long TTLs. It defines UDP
  TC as truncation and DNS-over-TCP framing. Source:
  https://www.rfc-editor.org/rfc/rfc1035.html
- RFC 5452 requires a candidate response to match at least the remote address,
  destination/source port, transaction ID, query name, class, and type. The
  bootstrap socket therefore uses an ephemeral local port, a cryptographically
  generated ID, connected numeric peer, and exact question comparison. Source:
  https://www.rfc-editor.org/rfc/rfc5452.html
- RFC 2308 defines authoritative negative-cache TTL from SOA TTL/MINIMUM, but
  this endpoint foundation is not a general recursive cache. It will not cache
  NXDOMAIN/NODATA or transport errors; a later attempt may retry. Source:
  https://www.rfc-editor.org/rfc/rfc2308.html
- A successful answer is accepted only when an A/AAAA owner is the normalized
  QNAME or is reachable through an in-message CNAME chain. The chain is bounded
  and loop-checked. A response containing only a terminal CNAME is typed NoData;
  this foundation does not recursively issue a second name query.
- Freshness is the minimum TTL across the accepted CNAME chain and selected
  address RR, clamped by explicit policy. Default compatibility policy keeps the
  existing five-minute floor; the upper bound is seven days, following RFC
  1035's example of limiting excessively long TTLs.
- First matching address in answer-wire order is selected deterministically.
  Multi-address racing is part of the required dual-stack/family-selection
  follow-up, not this single-family task.
- TC=1 is a typed truncated response and is not parsed as success. Automatic
  bootstrap TCP fallback is deferred; adding it silently would expand this UDP
  bootstrap task and retry semantics.
- Refresh failure never overwrites the last valid entry, but an expired entry is
  not served as success. The next call may resolve again. This is an intentional
  safety deviation from Go's effectively unbounded stale publication; serving
  stale endpoints later requires an explicit bounded policy.

## Runtime and test evidence

- Tokio `sleep_until` is cancel-safe by dropping the future and works with the
  caller runtime; the library must not create a runtime:
  https://docs.rs/tokio/latest/tokio/time/fn.sleep_until.html
- Tokio paused time/advance is available behind `test-util` on a current-thread
  runtime. It is suitable for retransmission/TTL tests without wall-clock sleeps:
  https://docs.rs/tokio/latest/tokio/time/fn.pause.html
- `getrandom 0.4.3` is already present in `rust/Cargo.lock`, declares Rust 1.85,
  is MIT OR Apache-2.0, and exposes `fill(&mut [u8])`. A direct exact production
  dependency is the planned source for unpredictable DNS IDs; deterministic
  tests inject a fixed ID source. Pinned local source:
  `$CARGO_HOME/registry/src/.../getrandom-0.4.3`.

## Required follow-up debt

Create a later task for native dual-stack resolution and address selection. It
must decide A+AAAA scheduling, Happy Eyeballs/address racing, per-family failure
memory, cache/publication shape, and interaction with QUIC/HTTP3 and connection
reuse. The present public types use an address-family enum and result collection
boundary so that follow-up is additive rather than a config-breaking rewrite.
