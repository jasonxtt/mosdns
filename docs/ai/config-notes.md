# Config Notes

## Core invariant

The WebUI is part of the configuration system, not just a viewer. A change to
upstreams, rule lists, dedicated groups, or related settings may update
persistent state, generated config, and runtime behavior. Rust components must
preserve this control-plane contract.

## Runtime state

The deployed runtime config is rooted at `/cus/mosdns`.

Small UI/runtime JSON state belongs under `/cus/mosdns/webinfo`. When both a
legacy root-level file and a `webinfo/` file exist, follow the current source's
migration/precedence logic and prefer the managed state location.

Do not place user rules, WebUI state, upstream state, generated files, caches,
SRS data, or config-update state into config-package `managed_files`.

## Config package compatibility

For the Go/main-style config-update workflow, compatibility is declared by
`requiredConfigSchema` and `requiredConfigPackageID` in
`coremain/config_update.go`.

- Binary-only releases keep both values unchanged.
- Structural config releases bump both values and publish a matching external package.
- The internal schema is not the user-facing version label; update any UI display mapping separately.

The external package repository is the sibling `file` repository, normally
`../file/mosdns/config` from these worktrees. Keep binary values, manifests,
managed-file boundaries, and published packages consistent.

## Dedicated routing and rule order

The canonical feature name is `special_groups`.

Dedicated groups bind lists to upstream groups and generated routing entries.
Rule order is behavior: earlier generated matches usually win, so UI changes to
list order or priority change runtime routing.

## Switch bits and fast marks

Switch-plugin allocation shares a namespace with query-context fast marks and
config `fast_mark` values. Before adding or renumbering a switcher, inspect the
current mask range, active config packages, and the corresponding Rust/Go
adapter behavior.

Current config reservations include `fast_mark 48` for the unified-matcher
sentinel and bit 49 for `switch17`. A collision can skip `unified_matcher1`,
produce `unmatched_rule`, and fall through to FakeIP. Verify deployed audit
fields such as `domain_set`, `effective_tag`, `final_sequence`,
`final_upstream`, and `matched_rule_source`.

## Rust compatibility boundary

The final target is a pure Rust-native host. Preserve the configuration/control-plane product contract: YAML semantics, plugin/sequence behavior, API/WebUI workflows, metrics/audit outputs, dump/runtime-state formats, config-update rules, and the managed runtime paths documented here.

Existing Phase 1/2/3A Go↔Rust selectors, adapters and fallback paths remain supported only while that transitional code exists; they are not product-contract requirements and should not be copied into Phase 3B+ designs. New pure Rust foundations may intentionally differ from Go internals when the difference is documented and does not break the frozen control-plane or user-visible behavior. Final production replacement waits for Rust-native E2E verification plus the dedicated hybrid-scaffolding retirement gate.

## Query diagnostics

UI and diagnostics should prefer the effective routing label, final upstream
group, and final upstream path. Intermediate tags must not be presented as if
they were all effective.
