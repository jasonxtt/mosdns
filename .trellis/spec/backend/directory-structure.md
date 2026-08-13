# Directory Structure

## Ownership boundaries

```text
coremain/                 process lifecycle, generated config, HTTP API, embedded UI
plugin/                   MosDNS plugin registration and implementations
  executable/cache/      current Go cache plugin and compatibility contract
pkg/                      reusable Go DNS, matcher, transport, cache, and utility code
webui-log/                maintained Vue UI served at /
webui-blog/               legacy-compatible UI assets
rust/                     gradual Rust data-plane crates (introduced by migration tasks)
scripts/                  repository build and release entrypoints
docs/ai/                  durable project context and migration decisions
```

`coremain/mosdns.go` demonstrates process/plugin lifecycle and embedded assets. `plugin/executable/cache/` owns cache plugin semantics; reusable algorithms belong in `pkg/`. Rust code must live under `rust/`, while the smallest necessary Go bridge stays beside the Go owner it integrates with.

## Placement rules

- Keep HTTP endpoints and runtime/config orchestration in `coremain/`; do not make Rust crates aware of Vue or HTTP handlers.
- Keep plugin names, argument parsing, and lifecycle in the existing `plugin/` package during gradual migration.
- Put generally reusable Go code under `pkg/` only when it has more than one real consumer.
- Add a shared Rust runtime/ABI abstraction only when a second Rust module needs it. The first cache crate should not invent a speculative framework.
- Keep tests next to their owning Go or Rust module. Cross-language parity fixtures should have one documented canonical location chosen by the cache task.

## Naming and examples

- Preserve established feature names such as `special_groups`; never reintroduce `route_group`.
- Match existing Go package and filename style. Use Rust crate names that describe the capability, such as `cache-core`, rather than the integration mechanism.
- Representative boundaries: `coremain/config_update.go`, `coremain/api_special_groups_test.go`, `plugin/executable/cache/cache.go`, `pkg/dnsutils/`, and `scripts/build-local.sh`.
