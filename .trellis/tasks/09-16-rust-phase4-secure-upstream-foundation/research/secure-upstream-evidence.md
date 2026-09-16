# Secure upstream evidence and decision ledger

Date: 2026-09-16. Repository baseline: `cb15361`, branch rust. Read-only source
inspection and official documentation lookup; no dependency installation,
compilation experiment or protocol test was performed during planning.

## Local product and behavior evidence

| Source anchor | Observed fact | Classification / planned consequence |
| --- | --- | --- |
| `pkg/upstream/upstream.go:67` Opt.DialAddr | Dial destination override explicitly leaves SNI and HTTP Host unchanged | Preserve; independent numeric destination and service identity |
| `pkg/upstream/upstream.go:353` | `tls` default port 853; derives ServerName from service host, clones TLS config, handshakes with context | Preserve service identity and framing; typed numeric endpoint already includes port; host later maps default 853 |
| `pkg/upstream/upstream.go:402` | `https` uses 443, Go HTTP transport with HTTP2 configured, h3 only when selected | Preserve h2/HTTP1 capability; defer h3 explicitly; never silently reinterpret h3 as h2 |
| `pkg/upstream/utils.go:34` | DialAddr replaces whole dial authority; absent override port uses protocol default | Preserve later config adapter semantics; do not accidentally inherit service URL port when override supplies only IP |
| `plugin/executable/forward/forward.go:67`, `:145` | Exposed settings include insecure_skip_verify, bootstrap/version, pipeline, HTTP3, idle and socket options | Product contracts; unimplemented foundation settings are deferred, not dismissed as implementation-only |
| `coremain/api_upstream.go:42`, `webui-log/src/components/UpstreamManager.vue:490` | API and UI persist upstream options | No control-plane edits; host cutover must retain support |
| `pkg/upstream/doh/upstream.go:53` | GET; Accept application/dns-message; no default User-Agent | Preserve method/Accept; do not add a user agent |
| `pkg/upstream/doh/upstream.go:77` | Copy outbound wire, ID zero, unpadded base64url; restore original response ID | Preserve caller contract and outbound convention; HTTP stream associates response |
| `pkg/upstream/doh/upstream.go:112` | Background six-second HTTP request outlives caller cancellation | Intentional Rust deviation: caller-owned cancellation/drain, no detached request |
| `pkg/upstream/doh/upstream.go:138` | Replaces entire URL RawQuery with dns value | Intentional Rust deviation: retain unrelated query fields; replace duplicate dns fields |
| `pkg/upstream/doh/upstream.go:146` | Only status 200 accepted; direct RoundTrip does not follow redirects | Preserve status/no redirect; omit remote body from typed error to avoid leaking data |
| `pkg/upstream/doh/upstream.go:156` | LimitReader caps at DNS maximum but does not prove no trailing bytes; checks only minimum header | Intentional Rust deviation: explicit oversize detection, MIME and full dns-core validation |
| `pkg/upstream/bootstrap/bootstrap.go:47`, `:244` | Explicit bootstrap IP; 0/4 means A, 6 means AAAA; internal background cache/update | Preserve future exposed resolution semantics; defer algorithm and implementation here |
| `pkg/upstream/upstream_test.go:75`, `:103` | Local DoT helper and UDP/TCP/TLS matrix; insecure TLS used in fixture | Discovery only; does not prove certificate-authentication or DoH parity |
| `rust/upstream-core/src/tcp.rs:160`, `:179` | Generic AsyncWrite/AsyncRead framed helpers | Reuse for TLS; include explicit flush since TLS adds buffering |
| `rust/upstream-core/src/lib.rs:654`, `:885` | Lifecycle final-commit ordering and caller control under lock | Reuse Rust contract; do not construct new host/runtime ownership |

Anchors are baseline source line numbers; re-locate symbols if earlier changes
shift lines. Source facts are not test results.

## Official protocol/library references

- [RFC 7858](https://www.rfc-editor.org/rfc/rfc7858.html), §3: TLS DNS stream
  framing and default port. Fresh streams here are a foundation boundary;
  connection reuse remains a later efficiency/lifecycle task.
- [RFC 8484](https://www.rfc-editor.org/rfc/rfc8484.html), §4/§6: DNS over HTTPS,
  GET encoding, media type and message association. Use its HTTP semantics as
  protocol evidence, not a mandate to copy Go's detached exchange.
- [rustls 0.23.35 docs](https://docs.rs/rustls/0.23.35/rustls/): separate TLS
  engine, trust roots and provider selection; std/ring/tls12 features are the
  proposed foundation choice. This documentation says MSRV 1.71; the complete
  resolved graph still needs verification against workspace Rust 1.85.
- [tokio-rustls docs](https://docs.rs/tokio-rustls/latest/tokio_rustls/), observed
  0.26.5: async TLS stream writes buffer data and need an explicit flush. Record
  this as a required test, not an assumption that write_all has reached the wire.
- [Hyper client connection API](https://docs.rs/hyper/latest/hyper/client/conn/index.html),
  observed 1.11.1: low-level connection API leaves dialing and pooling to its
  caller. Use it instead of a high-level client whose retry/pool behavior would
  widen the first task. HTTP2 executor ownership still needs Slice0 proof.
- [rustls-native-certs](https://docs.rs/rustls-native-certs/latest/rustls_native_certs/),
  observed 0.8.4: platform roots are a feasible host-layer route. Do not add it
  here: foundation receives roots, and host startup must later specify trust
  loading/errors and existing environment compatibility.

Version observations are research snapshots, not approved lockfile versions or
claims of latest/security status. Implementation must resolve an MSRV-compatible
set, inspect licenses/features and pin Cargo.lock after authorization. No
runtime/dependency files were modified in this planning turn.

## Dependency decision proposal

| Item | Role | Bounded choice / alternative |
| --- | --- | --- |
| rustls 0.23 + tokio-rustls 0.26 | TLS engine and Tokio stream | direct library dependencies; explicit ring provider, std/tls12, default features off; no aws-lc/OpenSSL default accidentally enabled |
| hyper 1 + hyper-util 0.1 | Low-level HTTP1/2; Tokio IO adapter | client/http1/http2 + minimal tokio adapter features; no legacy pooled client |
| http-body-util 0.1, bytes 1 | Typed empty request body, incremental response frames | avoid collecting unbounded body; reuse workspace-compatible versions |
| base64 0.22, url 2 | GET encoding; structured endpoint query editing | validated URL -> Hyper URI; preserve authority/escaping and reject unsupported forms |
| Test certificate tooling | Synthetic CA/name/expiry fixtures | check in clearly synthetic certificates/keys with generation provenance; no rcgen/time dependency required at runtime |
| KixDNS | Existing audit at pinned `2da3a2d59466e996a0f846c3e7e504970b878b06` | no extraction/direct dependency; old UDP/TCP ledger is not a secure-transport audit |

Do not copy a KixDNS TLS/HTTP implementation using the old ledger as permission.
This task can use direct library APIs; any later source extraction requires a
new fixed-revision secure-transport audit and attribution before scope approval.

## Deferred research has explicit owners

1. Bootstrap/cache/TTL/family and resolver cancellation -> endpoint-resolution task.
2. Reuse, HTTP2 multiplexing across queries, pipeline, idle and recovery ->
   connection-lifecycle task; enabled user options cannot be ignored at cutover.
3. Proxy/mark/device/local bind -> socket-policy task.
4. Root discovery/environment/custom roots and config adaptation -> native host.
5. Full protocol performance/soak -> later composed data-plane/host gates.

No deferred item is a reason to claim full Phase4 or production readiness.

## Slice0 dependency/MSRV resolution record — 2026-09-16

Baseline revision `0cd9f4b` (rust worktree). Slice0 authorization covers exactly
`resolver 3` + the reviewed HTTP/TLS dependency graph; no network/TLS/HTTP runtime
code was written and no Slice1+ work was started.

### RED baseline (before this change)

- `rust/Cargo.toml` declared `resolver = "2"`.
- `cargo metadata --manifest-path rust/Cargo.toml --locked --no-deps` passed and
  listed only the six workspace packages.
- `cargo tree --manifest-path rust/Cargo.toml -p mosdns-upstream-core -e features
  --locked` contained no `hyper`, `hyper-util`, `http-body-util` or
  `tokio-rustls`; `Cargo.lock` had zero `[[package]]` entries for all four.
  `rustls 0.23.45` and `ring 0.17.14` were already present.

### GREEN change

- Workspace resolver raised to `3`, the MSRV-aware resolver supported by the
  actual 1.85 workspace (`rust-version = "1.85"`). Resolution reported
  "Locking 18 packages to latest Rust 1.85 compatible versions".
- `rust/upstream-core/Cargo.toml` now declares exactly:
  `hyper = { version = "=1.11.0", default-features = false, features = ["client", "http1", "http2"] }`,
  `hyper-util = { version = "=0.1.20", default-features = false, features = ["tokio"] }`,
  `http-body-util = { version = "=0.1.3", default-features = false }`,
  `tokio-rustls = { version = "=0.26.4", default-features = false, features = ["ring", "tls12"] }`.
  Existing `rustls` stays `default-features = false, features = ["ring", "std", "tls12"]`.
  No reqwest, hyper-rustls, tower client/pool feature, aws-lc-rs or OpenSSL is
  added and no second runtime is introduced.

### Resolved direct-dependency ledger (exact, from Cargo.lock/metadata)

| Crate | Version | Selected features | rust-version | License |
| --- | --- | --- | --- | --- |
| hyper | 1.11.0 | client, http1, http2 | 1.63 | MIT |
| hyper-util | 0.1.20 | tokio | 1.64 | MIT |
| http-body-util | 0.1.3 | none (default-features = false) | 1.61 | MIT |
| tokio-rustls | 0.26.4 | ring, tls12 | 1.71 | MIT OR Apache-2.0 |
| rustls (existing) | 0.23.45 | ring, std, tls12 | 1.71 | Apache-2.0 OR ISC OR MIT |
| ring (existing) | 0.17.14 | default | 1.66.0 | Apache-2.0 AND ISC |

Key transitive additions: h2 0.4.19 (MIT, MSRV 1.63), http 1.5.0
(MIT OR Apache-2.0, MSRV 1.57.0), http-body 1.1.0 (MIT, MSRV 1.61), httparse
1.10.1 (MIT OR Apache-2.0), atomic-waker 1.1.2 (Apache-2.0 OR MIT, MSRV 1.36),
futures-channel/-util 0.3.34 (MIT OR Apache-2.0, MSRV 1.71), futures-core
0.3.34 (MSRV 1.36), want 0.3.1 (MIT), try-lock 0.2.5 (MIT), fnv 1.0.7
(Apache-2.0 / MIT), itoa 1.0.18 (MIT OR Apache-2.0, MSRV 1.68), tracing 0.1.44
and tracing-core 0.1.36 (MIT, MSRV 1.65.0), indexmap 2.14.2 / hashbrown 0.17.1.
No resolved registry package declares a rust-version above 1.85.0; the highest
are hashbrown 0.17.1 and uuid 1.24.0 at exactly 1.85.0. `aws-lc-rs`/`aws-lc-sys`
appear only as unused feature names of rustls/tokio-rustls, never as resolved
packages; `ring` remains the sole crypto provider.

### Resolver-2 fresh-resolve MSRV risk (reproduced)

In a throwaway copy of the workspace with `resolver = "2"` and no `Cargo.lock`,
a fresh resolve selected `idna_adapter 1.2.2` (rust-version 1.86) plus ICU
`icu_normalizer`/`icu_properties`/`icu_provider`/`icu_collections`/
`icu_locale_core`/`*_data` 2.3.x (rust-version 1.88). Those exceed the declared
1.85 MSRV, so a resolver-2 fresh resolve would silently produce a graph this
workspace promises not to need. Resolver 3 keeps the reviewed `idna_adapter
1.1.0` (rust-version 1.57) and the previously locked graph. This is why the
resolver was changed and why `Cargo.lock` must be regenerated under resolver 3
rather than left to a resolver-2 update.

### Hyper 1.11.0 HTTP/2 ownership source locations

- `src/client/conn/http2.rs:77` `pub async fn handshake(exec, io) ->
  (SendRequest<B>, Connection<T, B, E>)`.
- `src/client/conn/http2.rs:50` `pub struct Connection<T, B, E>` wraps
  `proto::h2::ClientTask` and is only a dispatcher.
- `src/client/conn/http2.rs:150` `Connection::send_request` calls
  `self.dispatch.send(req)` and awaits a channel; it does not drive the socket.
- `src/proto/h2/client.rs:192` `exec.execute_h2_future(H2ClientFuture::Task {
  task: ConnTask::new(..) })` submits the connection driver to the supplied
  executor.
- `src/proto/h2/client.rs:556` (body pipe) and `:566` (send/response) submit
  per-request futures through that same executor.
- `src/rt/bounds.rs:73`-`75` adapts any `Executor` into `Http2ClientConnExec`.
- `src/rt/mod.rs:45` `pub trait Executor<Fut> { fn execute(&self, fut: Fut); }`.
- hyper-util 0.1.20 `src/rt/tokio.rs:75`/`:105` `TokioExecutor` implements
  `execute` with `tokio::spawn` (detached); this slice does not use it.

Conclusion: the connection driver and every per-request future flow through the
caller-supplied `Executor`; Hyper 1.11.0 cannot hand the driver back for inline
polling. Slice3 must select and prove the tracked/queued executor described in
`design.md` section 5.

### Limits of this evidence

No runtime/TLS/HTTP code, handshake, executor or protocol behavior was tested,
because Slice0 is limited to the dependency graph and source inspection. The
workspace was resolved and `cargo check`ed with the installed toolchain
(cargo/rustc 1.95.0); Rust 1.85.0 is not installed here, so MSRV compatibility
is evidenced by resolved `rust-version` metadata and resolver-3 selection, not
by a 1.85 build.

## Slice0 pure DoH request-target record — 2026-09-17

The RED focused test failed before implementation with unresolved
`DohRequestError`, missing `DohEndpoint::get_request_target`, and the missing
`SecureError::DohRequest` variant. The GREEN implementation adds only pure
pre-I/O construction; it does not create a URI, socket, TLS stream, Hyper
driver, retry, pool, or runtime task.

- Direct dependency: `base64 = "=0.22.1"`, `default-features = false`,
  feature `alloc` only. License is MIT OR Apache-2.0 and declared MSRV is
  1.48.0. The feature is required for the allocating `Engine::encode` API;
  no `std` default feature is enabled.
- `DohEndpoint::get_request_target(ExchangeRequest<'_>)` checks the DNS wire
  length before copying/encoding, copies only the outbound bytes, zeroes the
  copied transaction ID, and uses `URL_SAFE_NO_PAD`.
- Existing decoded `dns` query keys, including percent-encoded key spellings,
  are removed through `url`'s structured `query_pairs` API. Unrelated pairs
  and the escaped path are retained at decoded-pair/path semantics; one new
  `dns` pair is appended. The return is origin-form path plus query only, so
  authority, credentials, fragment, and numeric dial address cannot leak into
  the request target.
- `DohRequestError::{QueryTooLarge,TargetTooLarge}` carry no URL, query, or
  encoded-message data. Query lengths above 65535 fail before encoding; target
  lengths above 96 KiB fail after construction and before I/O. Both remain
  `SideEffectState::NotSent`.
- `tests/slice0_secure.rs` now has 24 passing tests, including unchanged caller
  bytes/ID, duplicate decoded-key removal, URL-safe/unpadded output, IPv6 and
  escaped path behavior, both bounds, and Display/Debug redaction. Parent
  reran the full upstream-core suite (125 tests) and clippy/workspace checks.

The structured query API may normalize raw query escaping (for example a
valueless pair can serialize with `=`); byte-for-byte raw query preservation is
not claimed, while decoded pair semantics and escaped path preservation are.

## Slice0 root-review remediation — 2026-09-17

The first formal review of remote commit `7ebc86d08a20d07ed0d8087c7ea048e83bbf6dc2`
returned `FAIL / Slice0 remains OPEN` with two scoped P1 findings. The reviewer
confirmed the TLS policy, dependency boundary, pure DoH target, Hyper source
evidence, and Slice1+ scope boundary; only these endpoint pre-I/O contracts
blocked closure.

1. `DohEndpoint::new` now rejects raw carriage-return and line-feed bytes in the
   original URL before calling `url::Url::parse`. The parser can otherwise
   discard those bytes as a non-fatal syntax normalization. The new data-free
   `ServiceUrlError::ControlCharacter` remains `NotSent` and cannot echo URL or
   query material.
2. URL-derived domain hosts now call the existing
   `ServerIdentity::from_dns_name` path through `from_url_host`, so underscore,
   overlong-label, and leading/trailing-hyphen rules are identical for direct
   and URL-derived identities. IP URL hosts remain unchanged.

The DSH remediation first reproduced the shared-validation defect (URL host
accepted where `ServerIdentity::new` rejected it) and the missing CR/LF error
contract, then turned both tests green. Parent verification now reports 27
focused secure tests and 125 upstream-core tests passing, with fmt, clippy,
workspace check, and diff check passing. The corrected commit is sent back to
the same `rust0916` conversation for another formal review; Slice1 remains
unauthorized.

## Slice1 synthetic certificate fixture record — 2026-09-16

All Slice1 certificate material is synthetic and generated into a throwaway
directory outside the repository; only DER bytes were copied into
`rust/upstream-core/tests/fixtures/`. No real service certificate or private key
is committed, and the fixture module records the exact commands and windows.

- Generator: OpenSSL 3.6.4 (Homebrew, `/opt/homebrew/bin/openssl`). The
  system-default `openssl` on this host is LibreSSL 3.3.6, whose `req`/`x509`
  do not accept `-not_before`/`-not_after`; the Homebrew 3.6.4 binary was used
  specifically so the expired fixture could be backdated deterministically
  instead of relying on the wall clock.
- Keys: EC P-256 (`prime256v1`) for both roots and all leaves. Certificates are
  SHA-256. Leaves carry `basicConstraints=critical,CA:FALSE`,
  `keyUsage=critical,digitalSignature`, `extendedKeyUsage=serverAuth`, and one
  `subjectAltName=DNS:<name>`.
- Roots: `mosdns-slice1-synthetic-root-a` (trusted in tests) and
  `mosdns-slice1-synthetic-root-b` (deliberately never trusted), both
  self-signed with `CA:TRUE`, valid 2024-01-01 to 2036-01-01 UTC.
- Leaves: `dns.example` under A (valid), `other.example` under A (name
  mismatch), `dns.example` under A valid 2020-01-01 to 2021-01-01 (expired),
  and `dns.example` under B (unknown issuer). The expired leaf is expired
  relative to any realistic run, so the expiry case needs no clock control; the
  mismatch and unknown-issuer cases are time-independent.
- `root_store_a()`/`root_store_b()` build single-anchor stores so the
  unknown-issuer case is really about the anchor set. A positive control asserts
  the same untrusted leaf verifies once its real issuer is supplied, which rules
  out an unparseable fixture.

No Cargo dependency was added for fixture generation; the certificates are
checked-in constants, matching the Slice0 approach.

## Slice1 remediation dependency record — 2026-09-16 (test-only certificate generator)

The Slice1 review required removing committed private-key bytes from
`tests/fixtures` and generating the synthetic material at test runtime instead.
The generator is `rcgen`, declared **dev-dependency only** in
`rust/upstream-core/Cargo.toml`.

- Declaration: `rcgen = { version = "=0.14.7", default-features = false,
  features = ["ring"] }`. The exact pin preserves the reviewed lock, as the
  existing secure dependencies do.
- Why an extra dependency was needed: the previous fixtures were checked-in
  DER, which included PKCS#8 private keys. Generating certificates in memory is
  the only way to keep valid / wrong-name / expired / unknown-issuer /
  bad-signature coverage without committing key material. `rcgen` is the
  smallest reviewed option and reuses the `ring` provider already selected for
  the client, so no second crypto backend is introduced.
- Features: rcgen's default features are disabled and only `ring` (which
  implies `crypto`) is enabled. This turns off the optional `pem` encoder and
  the `aws_lc_rs`/`fips`/`zeroize` features, so no `aws-lc-rs`/`aws-lc-sys` or
  OpenSSL package enters the graph.
- **x509-parser: optional, present in the lockfile, NOT activated.**
  Two earlier revisions of this record got this wrong in opposite directions.
  The first said the feature trimming meant "no `x509-parser`"; the second
  corrected that by calling `x509-parser` a *mandatory* dependency and an
  active member of the dev build graph. The second correction was also wrong.
  The three layers must be kept distinct:

  1. **Manifest declarations** (published `rcgen 0.14.7` manifest): the
     `[dependencies.x509-parser]` entry is `version = "0.18"`, `optional =
     true`. Every mention inside rcgen's `[features]` table is a *weak* optional
     reference — `ring = ["crypto", "dep:ring", "x509-parser?/verify"]` and the
     two `x509-parser?/verify-aws` entries in `aws_lc_rs`/
     `aws_lc_rs_unstable`. No feature ever writes `dep:x509-parser`, and no
     feature named `x509-parser` exists. The only non-weak mentions are
     `required-features` on the `sign-leaf-with-ca` and `sign-leaf-with-pem-files`
     *examples*, which are not built by this workspace.
  2. **Lockfile resolution superset**: `rust/Cargo.lock` lists `x509-parser`
     in `rcgen`'s package `dependencies` array, together with every package it
     would pull in (`asn1-rs`, `asn1-rs-derive`, `asn1-rs-impl`, `der-parser`,
     `oid-registry`, `nom`, `rusticata-macros`, `data-encoding`, `lazy_static`,
     `displaydoc`, `num-bigint`, `num-traits`, `thiserror`/`-impl`). The lock
     records resolved *candidate* packages for all optional dependencies of
     every crate in the workspace; it is a superset and is **not** evidence of
     what is compiled.
  3. **Actually activated build graph**: with only `ring` enabled, rcgen's
     activated edges are `ring`, `rustls-pki-types`, `time` and `yasna`.
     `cargo tree -p rcgen -e features --locked` lists exactly those and never
     `x509-parser`; a workspace-wide
     `cargo tree --workspace -e features --locked` contains no `x509-parser`
     line at all. `x509-parser` is therefore **not compiled** in this
     workspace.

  Consequences: the fixture generation path is rcgen + ring + time + yasna +
  rustls-pki-types only. `x509-parser` and its transitives are retained below
  purely as a **lockfile-only conservative license/MSRV audit** — they are
  covered in case a future reviewer or feature change activates them, not
  because they are currently built.
- Activated graph introduced by the rcgen dev-dependency (exact, from
  `cargo tree -p rcgen -e features --locked` and `cargo metadata --locked`):

  | Package | Version | rust-version | License |
  | --- | --- | --- | --- |
  | rcgen | 0.14.7 | 1.71 | MIT OR Apache-2.0 |
  | ring | 0.17.14 | 1.66.0 | Apache-2.0 AND ISC |
  | rustls-pki-types | 1.15.1 | 1.60 | MIT OR Apache-2.0 |
  | yasna | 0.5.2 | (none) | MIT OR Apache-2.0 |
  | time / time-core | 0.3.45 / 0.1.7 | 1.83.0 | MIT OR Apache-2.0 |
  | deranged | 0.5.8 | 1.85.0 | MIT OR Apache-2.0 |
  | num-conv | 0.1.0 | 1.57.0 | MIT OR Apache-2.0 |
  | powerfmt | 0.2.0 | 1.67.0 | MIT OR Apache-2.0 |
  | untrusted | 0.9.0 | (none) | ISC |
  | zeroize | 1.9.0 | 1.85 | Apache-2.0 OR MIT |
  | getrandom | 0.2.17 | (none) | MIT OR Apache-2.0 |
  | cfg-if | 1.0.4 | 1.32 | MIT OR Apache-2.0 |
  | libc | 0.2.189 | 1.65 | MIT OR Apache-2.0 |
  | cc | 1.4.6 | 1.65.0 | MIT OR Apache-2.0 |
  | shlex | 2.0.1 | 1.46.0 | MIT OR Apache-2.0 |
  | find-msvc-tools | 0.1.12 | 1.65.0 | MIT OR Apache-2.0 |

- Lockfile-only packages (present in `Cargo.lock` as a resolution superset for
  rcgen's `x509-parser` optional dependency, **not activated** by the current
  feature set; audited conservatively for license and MSRV):

  | Package | Version | rust-version | License |
  | --- | --- | --- | --- |
  | x509-parser | 0.18.1 | 1.67.1 | MIT OR Apache-2.0 |
  | asn1-rs | 0.7.2 | 1.68 | MIT OR Apache-2.0 |
  | asn1-rs-derive | 0.6.0 | (none) | MIT OR Apache-2.0 |
  | asn1-rs-impl | 0.2.0 | (none) | MIT/Apache-2.0 |
  | der-parser | 10.0.0 | 1.63 | MIT OR Apache-2.0 |
  | oid-registry | 0.8.1 | 1.63 | MIT OR Apache-2.0 |
  | nom | 7.1.3 | 1.48 | MIT |
  | rusticata-macros | 4.1.0 | (none) | MIT/Apache-2.0 |
  | data-encoding | 2.11.1 | 1.48 | MIT |
  | lazy_static | 1.5.0 | (none) | MIT OR Apache-2.0 |
  | displaydoc | 0.2.7 | 1.71.0 | MIT OR Apache-2.0 |
  | num-bigint | 0.4.8 | 1.60 | MIT OR Apache-2.0 |
  | num-traits | 0.2.19 | 1.60 | MIT OR Apache-2.0 |
  | thiserror / -impl | 2.0.20 | 1.71 | MIT OR Apache-2.0 |
  | time-macros | 0.2.25 | 1.83.0 | MIT OR Apache-2.0 |
  | minimal-lexical | 0.2.1 | (none) | MIT/Apache-2.0 |
  | serde_core | 1.0.229 | 1.56 | MIT OR Apache-2.0 |

  Every package in both tables is permissively licensed (MIT, ISC,
  Apache-2.0, Unlicense, or an `MIT OR Apache-2.0` / `Apache-2.0 AND ISC`
  grant), compatible with this GPL-3.0-only project's dependency policy.
- MSRV: `rcgen 0.14.7` declares 1.71. In the *activated* set the binding
  constraints sit exactly at the ceiling: `deranged 0.5.8` and `zeroize 1.9.0`
  both declare 1.85, with `time`/`time-core` at 1.83.0. Resolver 3 selects the
  0.3.45 time line rather than the 1.88-requiring 0.3.5x line, and `cargo add`
  reported "ignoring rcgen@0.14.10 (requires rustc 1.88)" for the same reason.
  A `cargo metadata --locked` audit of the complete resolved graph (including
  lockfile-only entries) reports **no** package whose `rust-version` exceeds
  1.85.
- Feature-aware audit commands used for the record above (all `--locked`):
  `cargo tree -p rcgen -e features` (rcgen's real activated edges),
  `cargo tree -p mosdns-upstream-core -e features` and
  `cargo tree -p mosdns-upstream-core -e normal` (zero rcgen/x509-parser on the
  normal edges), `cargo tree --workspace -e features` (no x509-parser line
  anywhere), `cargo tree -p mosdns-upstream-core -e dev,features`, and
  `cargo metadata` for the version/license/rust-version tables.
- Test-only isolation (re-audited): `cargo tree` with `-e normal` shows
  **zero** `rcgen` and **zero** `x509-parser` entries for `mosdns-upstream-core`
  and for the whole workspace; both appear only on dev edges. No production
  module imports rcgen, and the fixture module is compiled only into `tests/`.
- No network or external `openssl` dependency: generation is in-process and
  offline. The earlier one-off OpenSSL 3.6.4 provenance note is superseded; no
  certificate bytes remain in the repository.
- `rust/Cargo.lock` was updated for this graph; production feature sets and the
  existing secure dependency pins are unchanged.
