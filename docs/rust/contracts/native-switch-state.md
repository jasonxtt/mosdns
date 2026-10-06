# Native switch state and WebUI contract

This contract belongs to the opt-in pure-Rust `mosdns-native-host`. It does not
select the production/default binary or close the full migration gate.

## Configuration and runtime state

The native compiler accepts the named plugin types `switch1` through `switch17`.
Each declaration has a unique configured tag and a required nonempty
`args.initial_value` state-file path. Relative paths resolve from the declaration
base; absolute paths remain valid. A type or state-file collision is rejected
before runtime construction, and compilation does not create or modify state
files.

The host admits one registry per committed runtime snapshot. Startup reads
bounded UTF-8 text and trims surrounding whitespace, while a missing file starts
empty. A POST stores the exact value, including an empty value; a later restart
applies the startup trim rule again. The registry owns serialized per-file
durable replacement, bounded accepted work, atomic publication of query facts,
and shutdown draining. A pre-commit error retains the old value; a post-rename
durability ambiguity fences admission and requires recovery. No switch change
automatically flushes or rebuilds a cache.

Switch values are captured once at UDP datagram or TCP-frame admission and stay
with that query through branches, fallback, preference, redirect, and lazy
continuations. `switch1`–`switch14` use bits 32–45, `switch15` is bitless,
`switch16` uses bit 47, and `switch17` uses bit 49. Bits 46 and 48 remain
reserved. An expected `A` value for a bit-backed switch uses the captured fast
bit; other values use the captured immutable owner value.

## HTTP API

Configured tags use the existing routes, with URI decoding performed once:

* `GET /plugins/{tag}/show` returns the exact committed value as text.
* `POST /plugins/{tag}/post` returns `updated to: {value}\n` after durable
  completion and publication.

POST accepts exactly one string value in JSON `{ "value": "..." }`, the legacy
`{ "Value": "..." }` alias, or form `value=...`. JSON is the default when the
media header is absent. `application/json` and
`application/x-www-form-urlencoded` are accepted case-insensitively; UTF-8
charset parameters are retained, and unsupported media/charset returns 415.
Unknown fields, duplicate form values, malformed encoding, conflicting JSON
keys, and non-string JSON values return 400. The bounded request/value limit is
1 MiB and returns 413 before mutation. Unknown tags return 404, known tags with
the wrong method return 405, read-only owners return 403, conflicts/busy state
return 409, lifecycle/recovery rejection returns 503, and pre-commit I/O errors
return 500.

Native switch reads and writes may include
`X-Mosdns-Config-Generation`. It is a canonical decimal `u64`; malformed or
duplicate headers return 400, and a stale value returns 409 before lease or
owner I/O. Clients that omit it retain direct compatibility and bind the
current owner at admission.

## Capability inventory

`GET /api/v1/capabilities` keeps schema 1 and the existing 30 operation IDs,
and adds the value-free optional extension:

```json
{
  "switches": {
    "schema_version": 1,
    "config_generation": "7",
    "instances": [
      {"type":"switch17","tag":"routing/custom","readable":true,"writable":true,"reason":null}
    ]
  }
}
```

`switches.manage` is supported only when the returned inventory contains an
admitted owner. Each instance reports its configured type/tag and read/write
eligibility without exposing its value. Native clients without this extension
fail closed for switch controls; only a capability endpoint 404 selects the
legacy Go workflow.

## WebUI behavior and evidence

Both maintained Vue shells render every discovered native owner through generic
value controls and URI-encode configured tags. They show read-only reasons and
perform canonical readback after a successful write. Native controls do not
invent FakeIP/RealIP/cache/feature labels or follow-up effects. Product presets
and their historical cache/requery behavior remain in the legacy Go branch.
Overview and data-management switch reads resolve type 17/type 3 through the
actual inventory; an absent owner makes zero requests. Native cache menus use
only the actual cache inventory and do not fall back to hardcoded Go catalogs.

Focused HTTP coverage is recorded in
[S4 status](../validation-records/10-05-rust-native-switch-state-management/s4-status.md),
Vue admission and both-bundle coverage in
[S5 status](../validation-records/10-05-rust-native-switch-state-management/s5-status.md),
and whole-chain/build limitations in
[S6 status](../validation-records/10-05-rust-native-switch-state-management/s6-status.md).
