# Native typed settings (M1-f1)

Scope: the values the WebUI adjusts at run time — proxy address, ECS address, `aliapi` upstream lists, the `domain_output` generation switches — as named
configuration objects referenced with `$tag`, a versioned `webinfo/settings.json` that stores only differences from the YAML, one function that combines
both, and hot apply of a change. The reference host's `upstream_overrides.json`, `config_overrides.json`, global `replacements` and
`domain_generation_settings.json` are **not read**. Evidence: `docs/rust/validation-records/10-08-m1-f1/`. Design notes: the task's `research/design.md`.

## Objects and references

```yaml
- tag: main_proxy
  type: socks5_proxy        # args.addr: host:port, nonzero port, no authentication; empty or absent = no proxy (M1-f2)
- tag: client_ecs
  type: ecs_address         # args.addr: an IP, optionally /mask (the mask is ignored, as in quick `ecs`)
```
An empty `socks5_proxy` address is the "no proxy" state (the reference's empty global `socks5`): everything that references it connects directly, `use_proxy: true` included, and the overrides page shows `socks5: ""`. `ecs_address` always needs an address.
Both are non-executable (`exec: $main_proxy` is an error). References are `$tag` strings; an unknown tag or a tag of the wrong kind is a load error naming the YAML path.

- `aliapi`: plugin `proxy: $main_proxy` is the default proxy of entries that say `use_proxy: true` (default false); an entry may instead say `proxy: $other`. `use_proxy` without a plugin
  `proxy` is an error. The proxy applies to `tcp`/`tls`/`https` entries only (UDP ignores it, as in M1-b1). Literal `socks5:` stays accepted (plugin or entry) but excludes `proxy`/`use_proxy` in any form (even `use_proxy: false`), an entry `proxy`, and stored `upstreams` settings for a plugin with a literal `socks5` (all are load errors); use `proxy: $x` instead.
  Whether an entry is proxied is decided only by these keys; the reference's "`foreign`/`foreignecs` fall back to the global proxy" rule does not exist here.
- `sd_set_light`, `si_set`, `adguard_rule`: `proxy: $main_proxy` for downloads; a literal `socks5` excludes it.
- ECS: the quick form `ecs $client_ecs` (inside any sequence argument) and `ecs_handler.preset: $client_ecs`. The resolved address is a fixed address for the cache placement rule (M1-g).
- `domain_output`: new optional arg `enabled` (default true).

## `webinfo/settings.json` (under the configuration root)

```json
{"version":1,
 "proxies":{"main_proxy":{"addr":"192.0.2.9:7890"}},
 "ecs":{"client_ecs":{"addr":"198.51.100.5"}},
 "upstreams":{"foreign":[{"tag":"cf","enabled":true,"protocol":"tcp","addr":"1.1.1.1","use_proxy":true}]},
 "domain_generation":{"enabled":false,"plugins":{"my_realiplist":{"enabled":false}}}}
```
Only differences are stored: a value equal to what the YAML says (an address, an upstream list, a switch) is removed from the file by the API (sections omitted when empty; keys sorted; two-space indent; trailing newline). `upstreams.<tag>` replaces that `aliapi`'s whole YAML list (only `enabled`
entries compile; `concurrent` stays the YAML value; all entries disabled is an error). Entry fields are the WebUI's: `tag`, `enabled`, `protocol`
(`udp|tcp|tls|https`, aliases `dot|doh`), `addr` (a scheme-less address gets the protocol's scheme), `dial_addr`, `upstream_query_timeout` (ms), `insecure_skip_verify`, `bootstrap`,
`bootstrap_version`, `use_proxy` (the reference's `use_socks_proxy` is accepted on input). Refused with a reason: `protocol: aliapi` / signed-API fields, a literal `socks5`,
`so_mark`, `bind_to_device`, `enable_pipeline`, `enable_http3`, positive `idle_timeout`, `quic`, unknown fields.
Loading errors (start, `check`): unreadable or unparsable file, a version other than 1, an unknown top-level key, a settings key that names no object / the wrong kind, an invalid
value. A missing or empty file is "no settings". Compiling and `check` never write. With `native_management` the file is still read at start, but the settings API is not served and a settings apply is refused (that mode keeps its own upstream
overrides until M1-e).

## The effective configuration

`config::settings::apply(definitions, settings)` runs inside the shared compile (`start`, `check`, hot apply): validate, resolve references to the literal forms the plugin compilers accept,
overlay settings, and report every item (`kind`, `object`, `field`, `value`, `source` = `yaml` | `settings`). `domain_output` `enabled` is the plugin's setting (else the YAML `enabled`), and only while the master setting is on. A disabled list records and publishes nothing and injects nothing into the mapper layer.
`mosdns check` prints `EFFECTIVE <kind> <object> <field>=<value> source=<source>` lines; `GET /api/v1/settings/effective` returns `{settings_file, items}`.

## Hot apply

A change builds new settings, recompiles the same YAML with them (any error rejects the change), and publishes the result as the next generation through the whole-generation path the managed mode uses
(admission pause, management drain, switch rebase, cache policy gate, install, retirement of the old generation with rule sources / `domain_output` / `requery` fenced and joined, then a memory trim).
**Commit order:** (1) caches whose identity changed get an empty dump written; (2) the settings file is replaced durably (temporary file and parent directory are fsynced around the rename) after checking it still holds the bytes the candidate was built against (else
409); (3) the generation is published. Failure before (2) changes nothing; failure of (2) leaves the old file and generation (dumps may be empty: harmless, caches are only read at start); a crash
after (2) restarts into the new state, which the candidate compile already proved loadable; a failure while publishing stops the host the same way. A query keeps the generation it started with; the next
query uses the new one. The candidate compile runs on the DNS thread (its cost with the production rule set is in the validation record).

## API (without `native_management`; with it the settings routes are not served and the existing managed routes are unchanged)

- `GET /api/v1/upstream/tags` (the `aliapi` tags), `GET /api/v1/upstream/config` (`{tag:[entries]}`: the effective list in the WebUI shape, `use_socks_proxy` = the effective switch),
  `GET /api/v1/upstream/runtime/{tag}` (`override_config` = the settings entries), `POST /api/v1/upstream/config {plugin_tag, upstreams}` (validated, stored, applied; an empty list removes the override;
  404 unknown tag; 400 with the reason above). `/api/v1/upstream/sources` stays unsupported (special-group bindings, M1-e).
- `GET /api/v1/overrides` -> `{socks5, ecs, replacements: []}` from the unique `socks5_proxy` / `ecs_address`; `POST` sets them (an empty value returns to the YAML value; non-empty `replacements` -> 400;
  several objects of a kind -> 409 `ambiguous`; no object but a value -> 400).
- `GET /api/v1/domain-generation` -> `{enabled, remember_direct, remember_proxy, no_v4, no_v6}`; `POST` takes a partial of the same keys and answers `{settings, flush_targets}`. `enabled` is the
  master switch; the four others are the `domain_output` plugins tagged `my_realiplist`, `my_fakeiplist`, `my_nov4list`, `my_nov6list` (the WebUI is bound to these tags). A list whose switch went from
  on to off (every list when the master goes off) is flushed after the apply; if a flush fails the answer is 500 `settings were applied, but flushing failed: ...` (the settings stay applied).
- Capabilities: `upstreams.manage` (an `aliapi` exists, file-backed), `system.global_overrides` (a `socks5_proxy` or `ecs_address`), `system.domain_generation` (a `domain_output`).

## Migration from the reference override files (M1-f2)

`mosdns-settings-migrate --from <reference dir> [-d <root>] -c <config> --out <dir>` reads `config_overrides.json`, `upstream_overrides.json` and `domain_generation_settings.json`
(in `webinfo/`, `state/` or the directory itself), compiles the native configuration to get the YAML defaults, and writes `settings.json` (differences only) and `migration-report.txt` to `--out`.
It never changes its inputs or the native root. Exit code 0: everything mapped; 1: the report lists items that need a person (enabled signed-API entries, `so_mark`/`bind_to_device`/pipeline/HTTP3/QUIC/idle-timeout entries,
an entry's own `socks5` that differs from the global one, `replacements`, tags the configuration does not have, an enabled entry without a tag); 2: usage, I/O, parse or the migrated settings do not load.
The reference's rules are reproduced: `use_socks_proxy` unset means "TCP-class protocol and (own address or a `foreign`/`foreignecs` group)"; only those two groups fall back to the global address; UDP never uses a proxy.
They become explicit `use_proxy` values. A repeated upstream tag in a later group is renamed `<tag>-<group>` (upstream tags are unique here) and reported.

The GET entries of the settings-backed upstream page carry the reference field `use_socks_proxy` only (no `use_proxy`); `unsupported_features` omits `socks_proxy` there.
`check` marks an `aliapi` entry that really connects through a proxy address with ` [proxy]` in its `EFFECTIVE` value.

## Known differences from the reference

No `replacements`; no global override of every `socks5`/`ecs` string (explicit references instead); no implicit proxy for `foreign`/`foreignecs`; `concurrent` is not recomputed from the enabled count;
changes apply immediately (the reference asks for a restart on the overrides page); settings are per object tag.
