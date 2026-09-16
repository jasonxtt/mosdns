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
