# Native switch state and management

## 1. Scope / Trigger

This spec applies to native switch1–17 declaration, query admission facts,
managed-runtime replacement, configured-tag HTTP operations, and native Vue
capability discovery. The public product contract is
`docs/rust/contracts/native-switch-state.md`; this file records the executable
implementation boundaries learned while delivering that contract.

## 2. Signatures

- `SwitchRegistry::rebind` creates the next committed registry from the
  prepared declarations and reuses only unchanged type/tag/path identities.
- `SwitchRegistry::rebase_carried_values(&previous)` copies the latest
  committed `value`, `revision`, and `fingerprint` for unchanged owners after
  management work has drained, then refreshes the aggregate admission facts.
- Native switch HTTP routes remain
  `GET /plugins/{encoded-tag}/show` and
  `POST /plugins/{encoded-tag}/post`; the request body accepts one JSON `value`
  string (or legacy `Value`) or one form `value`.
- Capabilities retain schema version 1 and expose an optional `switches`
  extension containing ordered `{type, tag, readable, writable, reason}`
  instances plus a decimal-string `config_generation`.

## 3. Contracts

- Config generation and switch value revision are different values. A switch
  POST changes the latter, not the former.
- A real UDP datagram or TCP frame captures one immutable admission-value
  snapshot. Branches, continuations, cache recipes, and child execution carry
  that snapshot; later POSTs affect future admissions only.
- `switch1`–`switch14` seed bits32–45, `switch16` seeds bit47, and `switch17`
  seeds bit49. `switch15` is bitless; bits46 and48 are reserved.
- A config-generation header is parsed and compared only after a configured
  native switch tag/action has been identified. Unknown or non-switch plugin
  routes retain their normal routing behavior.
- A managed apply drains accepted switch mutations before publishing its next
  runtime view. The next view must carry the latest values of unchanged owners;
  values captured before the drain are not authoritative.
- Native UI controls use the discovered actual tag and canonical readback.
  They do not infer A/B from empty values or fabricate product-specific
  FakeIP/AdGuard/requery effects.

## 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| Unknown/unconfigured switch tag | 404 before owner I/O |
| Known switch action with wrong method | 405 |
| Malformed/duplicate generation header | 400 before lease or I/O |
| Stale generation for a configured native switch | 409; retain the draft and require rediscovery |
| Unwritable admitted owner | 403 with a nonempty reason |
| Pre-rename persistence failure | 500; committed file/value/revision remain unchanged |
| Observed owner conflict or mutation overload | 409 |
| Post-rename durability/publication ambiguity | 503/fatal recovery state; never claim unchanged state |
| Missing capability extension on old native | Switch controls unavailable; do not probe or fall through to Go |

## 5. Good / Base / Bad Cases

- Good: a POST completes while a candidate graph is being prepared; managed
  apply drains it and the published unchanged owner exposes the POSTed value.
- Base: a direct client omits the generation header and binds the committed
  owner at admission, preserving legacy compatibility.
- Bad: rebind publishes values copied before drain, or a broad header precheck
  rejects unrelated `/plugins/*` routes before route classification.

## 6. Tests Required

- Registry tests must cover failed candidate rebind isolation and the
  post-drain carried-value race.
- HTTP tests must cover custom/encoded tags, malformed and stale generation
  headers, unknown routes, JSON/form compatibility, and no-I/O behavior on
  rejected requests.
- Native admission tests must cover UDP/TCP snapshot boundaries, bitless and
  reserved switch bits, branch/continuation preservation, and missing owners.
- Both Vue shells must prove inventory-based tag resolution, canonical
  readback, zero requests for unconfigured controls, and selective cache
  follow-ups against actual catalog entries.
- Final checks include Rust format, strict clippy, native-host tests, targeted
  management HTTP tests, frontend Node tests, and the exact-source native
  build manifest.

## 7. Wrong vs Correct

### Wrong

Capture owner values while preparing a candidate, publish those values after
draining management work, or reject every malformed generation header before
knowing whether the request targets a configured native switch.

### Correct

Drain accepted mutations first, rebase only unchanged owner identities from the
previous committed registry, refresh the candidate's immutable facts, and
apply generation validation only to the classified native switch operation.
