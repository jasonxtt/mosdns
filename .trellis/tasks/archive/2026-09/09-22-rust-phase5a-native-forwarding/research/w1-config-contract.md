# W1 native configuration contract

## Evidence sources

The accepted shape is derived from the frozen fixtures:

- `tests/phase5a-baseline/configs/forward-udp.yaml`
- `tests/phase5a-baseline/configs/forward-tcp.yaml`
- `plugin/server/udp_server/udp_server.go`
- `plugin/server/tcp_server/tcp_server.go`
- `plugin/executable/forward/forward.go`
- `plugin/executable/sequence/config.go`
- `coremain/config.go`

The Go sources are compatibility evidence only. The Rust host must compile a
small explicit subset and reject everything else before binding a socket.

## Accepted graph

```yaml
log:
  level: error
plugins:
  - tag: <unique-forward-tag>
    type: forward
    args:
      upstreams:
        - addr: "udp://<numeric-ip>:<nonzero-port>"
          # or tcp://...
  - tag: <unique-sequence-tag>
    type: sequence
    args:
      - exec: $<forward-tag>
  - tag: <unique-listener-tag>
    type: udp_server # or tcp_server, exactly one listener
    args:
      entry: <sequence-tag>
      listen: "<numeric-ip>:<nonzero-port>"
      enable_audit: false
```

For TCP, `args.idle_timeout` is also accepted as a positive integer; the
frozen `idle_timeout: 2` case must compile and be effective. Declaration
order is irrelevant, but all tags and references must resolve after the whole
plugin list is collected.

## Strict validation matrix

| Location | Accepted | Must fail before bind |
|---|---|---|
| top level | `log`, `plugins` | every other key, missing required key, wrong type |
| `log` | only `level: error` | other levels/fields, wrong type |
| plugin list | exactly one forward, sequence, and one listener | duplicate roles, duplicate tags, unknown plugin, extra plugin |
| forward args | only `upstreams` | every other field, wrong type |
| upstream list | exactly one item | empty/multiple items, upstream tag |
| upstream item | only numeric `udp://`/`tcp://` `addr` | hostname, zero port, malformed address, unknown field |
| sequence args | exactly one unconditional `{exec: $forward}` | matcher/reverse, inline list, anonymous exec, goto/jump/try/return/accept/reject/exit, nested sequence |
| UDP listener | `entry`, numeric `listen`, `enable_audit: false` | audit true, missing/wrong fields, TLS/cert/key, zero/malformed port |
| TCP listener | UDP fields plus positive integer `idle_timeout` | zero/negative/noninteger timeout, audit true, TLS/cert/key, unknown fields |
| forward transport | UDP or TCP only | DoT/DoH/DoQ/DoH3/QUIC, bootstrap, hostname resolution, retry/fallback options |

The unsupported names called out by the forward upstream contract include
`concurrent`, `socks5`, `so_mark`, `bind_to_device`, `bootstrap`,
`bootstrap_version`, `dial_addr`, `idle_timeout`, and
`upstream_query_timeout`, `max_conns`, `enable_pipeline`, `enable_http3`, and
`insecure_skip_verify`; the host must not silently ignore them. Listener
`cert`/`key` and other TLS fields are likewise rejected where presented.

## Compile and bind order

```text
read path
  -> strict YAML decode
  -> top-level/field/type validation
  -> collect tags
  -> resolve references independent of declaration order
  -> parse numeric SocketAddr and endpoint transport
  -> compile typed host graph
  -> construct runtime/upstream/listener
  -> bind only after all prior steps succeed
```

Tests must prove invalid config leaves the requested listen port free. The
compiler must not start a partial graph or retain an unresolved config.

## W1 behavior contract

- UDP uses the frozen listen/upstream shape and must answer positive A and
  NXDOMAIN queries; concurrent distinct transaction IDs/qnames must map to the
  matching responses.
- TCP uses the frozen listen/upstream shape, two-byte big-endian framing,
  partial-read handling, sequential requests per connection, and concurrent
  independent connections.
- A stalled upstream reaches the host's default five-second deadline in the
  product shape; tests use a short injected host deadline and expect associated
  SERVFAIL.
- A parsed request with no sequence response becomes REFUSED. A malformed
  datagram is dropped; a malformed/partial TCP request closes only its
  connection.
- Shutdown cancels pending requests, stops admission, drains upstream, joins
  child tasks, and allows a clean rebind.
