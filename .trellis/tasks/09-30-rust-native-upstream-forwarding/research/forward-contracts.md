# Forward configuration and diagnostic contract

This is the proposed full-task contract submitted for final-summary approval.
Family default omitted/0 dual with IPv4 preference was explicitly approved on
2026-09-30; the optional schema1 diagnostics/UI migration below was separately
approved the same day. Other scope choices become implementation authorization only
after the latest complete planning summary is approved.

## Configuration surface

| Surface | Behavior |
| --- | --- |
| forward args | upstreams required/nonempty; concurrent, bootstrap and bootstrap_version additionally accepted. |
| upstream entry | addr required/nonempty; tag, numeric dial_addr, bootstrap, bootstrap_version, upstream_query_timeout and insecure_skip_verify accepted; inactive idle/pipeline/H3 fields only as specified below. |
| concurrent | Integer only; omitted/nonpositive =>1, above3 =>3; fanout min(count, normalized value). No duplicate entry work. |
| quick forward | One or more addr tokens; generated entries; concurrent defaults3. Reuse the same validation/endpoint construction. |
| named invocation | `$plugin` all entries; `$plugin tag1 tag2` selects unique explicit entry tags in supplied order. Unknown/empty/duplicate selection fails load. |
| optional entry tag | Nonempty when supplied; whitespace disallowed because subset syntax uses tokens. |
| bare/UDP/TCP address | Numeric or hostname; omitted port53; bracketed IPv6 supported. No invalid port0, userinfo, fragment or transport-inappropriate path/query. |
| tls address | Default853; service hostname/IP retained separately from dial. No userinfo/path/query/fragment. |
| https address | Default443; original authority and validated path/query retained. No credentials/userinfo/fragment; reuse secure endpoint constraints. |
| dial_addr | Numeric IP with optional explicit port; omitted port inherits service port. Never changes SNI/HTTP identity. Hostname dial_addr remains unsupported. |
| bootstrap | Numeric IP, optional port53, per-entry nonempty value overrides global; no recursive hostname bootstrap. |
| bootstrap_version | Explicit entry overrides explicit global; neither supplied =>dual. 0 dual/A preference, 4 A-only, 6 AAAA-only; others/type errors fail load. Do not reuse Go's zero-as-missing inheritance. |
| hostname without bootstrap/dial | Explicit load error; OS resolver remains deferred. Numeric target needs neither. |
| per-entry timeout | Milliseconds; omitted/zero =>5000; negative/type/overflow fail load. Effective deadline min(admission deadline, invocation start + entry timeout); no resets between lookup/dial/fallback. |
| insecure_skip_verify | Boolean; defaultfalse. True is explicit chain/name/time skip under existing core policy; no downgrade on failure. |
| idle_timeout | Omitted/zero uses foundation idle policy; positive override unsupported, negative/type errors rejected. |
| enable_pipeline / enable_http3 | Omitted/false accepted as inactive; true unsupported. Pipeline helper schemes, h3/quic schemes remain unsupported. |
| max_conns | Unsupported, including values Go currently ignores; no implicit promise to honor or silently discard it. |
| socks5/so_mark/bind_to_device | Unsupported; do not silently accept global or per-entry values. |
| unknown fields / malformed URL / references | ConfigError with source file and precise field/index, before listener bind or target/lookup I/O. |

Parsed zero/false must remain distinguishable from omission where inheritance
matters. A per-entry explicit bootstrap_version0 overrides global4/6. Invalid
supplied values fail even when numeric dialing would bypass resolution.

## TLS trust

Linux host discovers configured system trust once before secure I/O, builds an
explicit root store and shares an immutable TlsPolicy. Core does not discover
roots. No verified secure entry may proceed with an empty/unusable store; a
configuration containing only plain or explicitly insecure entries must not
require roots unnecessarily. Root loading reports operational failures without
printing trust contents or credentials. Synthetic CA injection is test-only.
No public config field for custom roots, trust reload or automatic insecure
fallback is introduced in this batch.

## Identity and public diagnostics

Use structured `(forward plugin, entry index)` internal owner/metric keys; tags
are display/subset identities, not a reason to find the winner by endpoint.

- Existing single entry retains effective identity: explicit entry tag else
  forward plugin tag, and existing host-wide collision checks remain valid.
- A multi-entry explicit tag uses that tag as effective identity. An untagged
  entry uses `@native-forward:<lowercase UTF-8 hex plugin tag>:<zero-based index>`.
  This is stable native diagnostic identity, not a new invocation alias.
- Check all generated and explicit effective identities together for collision;
  a user tag colliding with generated/single-entry identity fails at load with
  both source locations. Do not reinterpret or reserve unrelated tags silently.
- Quick-generated entry identity is
  `@native-quick:<lowercase UTF-8 hex enclosing sequence tag>:<zero-based rule index>:<zero-based exec index>:<zero-based entry index>`.
  Scalar exec has exec index0; list exec uses its actual list index. Subset
  invocation does not reindex entries. Apply the same global collision check.
- `final_upstream` still uses configured flow_setter precedence; without it,
  use the causally selected entry identity. `selected_upstream` is the actual
  numeric final peer formatted as SocketAddr, not URL or resolver/bootstrap IP.
- Preserve `upstream_targets` string for the final supplying entry's resolved
  numeric target unless configured routing metadata overrides it. It does not
  claim every group member was attempted. Per-leg facts retain distinct entry
  and actual destination/transport internally and in the explicitly approved
  versioned object below, without replacing existing fields.
- TC observation is not a final supplying response. Record its UDP/TCP leg facts
  without double-counting one exchange as two selected responses. A completed
  nonwinning leg remains completed; cancellation does not rewrite its outcome.
- Cache/local/parent replacement retains no superseded supplier; selected
  answer details always come from final wire, and one admitted query terminalizes
  once. Native IDs, capture gating, retained capacity and read admission stay
  unchanged.

## User-approved native v2 schema migration — P1-2

On 2026-09-30 the user selected expansion of versioned diagnostics and actual
detail display, rather than shrinking public acceptance. Add optional
`upstream_diagnostics` to the common v2 log projection used by `/logs`,
`/logs/domain` and `/rank/slowest`; leave URLs/envelopes/old fields/types unchanged.
The retained detailed record owns one typed projection; slowest shares its Arc.
No separate protocol trace buffer, raw packet or new read job is introduced.

```json
{
  "upstream_diagnostics": {
    "schema_version": 1,
    "selected": {
      "entry": "@native-forward:666f7277617264:0",
      "peer": "127.0.0.1:15453",
      "transport": "tcp"
    },
    "attempts": [
      {"ordinal": 0, "entry": "@native-forward:666f7277617264:0", "peer": "127.0.0.1:15453", "transport": "tcp", "outcome": "response"},
      {"ordinal": 1, "entry": "backup", "peer": "127.0.0.1:15454", "transport": "tls", "outcome": "canceled"}
    ]
  }
}
```

- `schema_version` is integer1; incompatible future shapes require another
  version and deliberate reader migration. No dynamic version negotiation or
  new endpoint is needed for this optional object.
- `entry` is the effective configured/native-generated entry identity defined
  above, independent of flow_setter and peer equality. `peer` is actual numeric
  SocketAddr. Target transport values are `udp`, `tcp`, `tls`, `https`, distinct
  from listener transport. DoH H1/H2 does not change the value `https`.
- Selected exists only for the response supplying the final wire. It always
  has entry/peer/transport. Cache/local/final replacement or no final network
  response omits selected, even if earlier attempts succeeded.
- Attempts are ordered started-entry slots across executed invocations, with
  zero-based query-global ordinal. Append on start, update outcome in place;
  no completion-order sort. Outcomes are closed `response`, `failed`,
  `timed_out`, `canceled`, `interrupted`; no public pending terminal row.
- Each actually started entry gets exactly one attempt. TC UDP→TCP changes the
  last-started transport to tcp in the same slot, not two configured attempts.
  The physical phase sequence is internal proof/peer counters, not a new public
  per-packet timeline. An entry resolving/failing before target I/O omits peer
  and transport. If target I/O starts, keep its peer and most recent started
  transport even when it fails/cancels. Never substitute bootstrap's address.
- Started resolver-only entry counts one failed/canceled configured attempt;
  malformed config or never-selected/never-started entries count none. Distinct
  slots can have identical peer values. A valid nonwinner records response.
- Object is present for new native detailed records, with attempts[] empty and
  no selected if no entry started (e.g. cache/local). Omit only for legacy/other
  producer records with no supported instrumentation; absence is not empty
  proof. Go responses remain unchanged. Unknown schema versions are unavailable
  in the new UI section while old fields remain usable.
- QueryManager and Overview nested details display factual selected data and
  attempt rows with ordinal/entry/peer/transport/outcome. Never infer entry from
  final_upstream or numeric peer. Search/rank/filter behavior stays unchanged.

Pending and audit-off data use IDs/SocketAddr/enums; basic metrics consume that
same terminal slot source through a prebuilt identity registry, with no audit-only
per-query String allocation. Display strings and serialized objects are made
only for eligible detailed terminal records. Capture-off metrics still account
for every started configured entry; ledger adoption replaces the old plugin
fallback path rather than double-counting it.

Here audit-disabled means the admission-time detailed-audit eligibility is
false. Runtime stop/start is still sampled at terminalization under the frozen
observer contract; stop does not retroactively erase an eligible query's needed
detail state or prevent an in-flight query from being captured after start.

## Preserved / deliberate deviations / implementation choices

Preserve response ranking, configured labels, wire/question correlation,
secure identity, default ports, timeout units and every existing accepted native
single-entry config. Proposed deliberate native differences are distinct fanout
instead of duplicate legs, scoped cancellation/join instead of Background legs,
explicit bootstrap/dial requirement rather than OS lookup, and strict unsupported
option errors. Approved host family default is a separate explicit deviation.
Random rotation implementation, stable simultaneous-ready polling, owner enum,
serial reuse composition and system-root-loading dependency are implementation
choices; no Go internals or PRNG sequence parity is promised.

## Test seam and mock boundaries

| Behavior slice | Public seam under test | Controlled boundary |
| --- | --- | --- |
| Config/invocation | file loader/HostAssembly from real YAML | Fixture files; no mocked compiler/catalog. |
| Selection/TC | real host UDP/TCP plus controlled target sockets | Peer responses/barriers; inject rotation seed, no fake transport in integrated proof. |
| Bootstrap | configured host -> real numeric bootstrap peer -> target | Synthetic DNS families/CNAME/TTL; injected clock for deterministic freshness. |
| TLS/HTTPS | real DoT/DoH peers and host listener | Synthetic CA/certs, narrow host root-store seam; no global trust mutation. |
| Reuse | public owners composed by real host | Peer accept counters/half-close barriers; no mocked success count. |
| Lifecycle/audit | real host cancellation/close + HTTP retained logs | Existing failure/time boundaries; compare wire, peer counters, selected facts. |
| Browser | existing Vue via VM Vite/native HTTP/DNS | Controlled network peers; no fabricated API/rank/detail response. |

Lower-layer unit tests may mock only peer/time/random/failure boundaries. Keep
one integrated mixed scenario plus decisive focused tests; do not multiply every
priority/family/transport/option into a Cartesian E2E matrix.
