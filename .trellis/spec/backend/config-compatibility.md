# Configuration and Runtime Compatibility

## Sources of truth

Read `docs/ai/config-notes.md` before changing configuration generation, upgrade behavior, runtime state files, or WebUI-backed settings. `coremain/config_update.go` defines `requiredConfigSchema` and `requiredConfigPackageID`; keep both unchanged for binary-only releases.

## Compatibility contract

- Preserve MosDNS YAML syntax, plugin type names, arguments, and sequence semantics while implementations move to Rust.
- Preserve the canonical `special_groups` name and its routing, upstream binding, online/local rule, and audit behavior.
- The maintained Vue UI at `/` and compatibility UI at `/log` remain stable control surfaces. Saving in the UI must continue to affect generated config and runtime behavior.
- Keep existing HTTP paths, response shapes, Prometheus names, audit fields, state files, and cache dump format unless a separately approved migration explicitly versions them.
- Switch IDs share a bit namespace with `fast_mark`; consult `docs/ai/config-notes.md` before allocation. Bit 48 is reserved and `switch17` uses bit 49.

## Rust backend selection

Rust is initially an opt-in implementation behind the existing Go plugin contract. Backend selection must be explicit and observable. Startup ABI/capability checks happen before serving; failures must follow the task's documented fallback policy without silently mutating config.

## Verification examples

Use `coremain/config_update_test.go`, `coremain/api_special_groups_test.go`, `coremain/state_files_test.go`, and plugin-specific tests as compatibility references. Structural configuration changes require the workflow in `docs/ai/config-notes.md`, not only unit tests.
