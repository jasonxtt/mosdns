# Config Notes

## Core Principle

In this fork, the WebUI is part of the configuration system, not just a viewer.

When the user edits upstreams, rule lists, dedicated routing groups, or related settings in the UI, the expected result is:

- persistent state is updated
- generated config changes accordingly
- runtime behavior changes accordingly

Any UI change that ignores config generation is incomplete.

## Runtime State

Runtime config lives under `/cus/mosdns`.

Small UI/runtime JSON state is managed under `/cus/mosdns/webinfo`. When both an old path and `webinfo/` exist, prefer `webinfo/`.

Do not put user rules, WebUI state, upstream state, generated files, caches, SRS data, or `webinfo/config_update_state.json` into config package `managed_files`.

## Config Package Updates

Config compatibility is declared by `requiredConfigSchema` and `requiredConfigPackageID` in `coremain/config_update.go`.

- Binary-only releases keep both values unchanged.
- Structural config releases bump both values and publish a matching external `config_up.zip`.
- The internal schema is not the user-facing version label. When schema changes, update the UI display mapping separately.

External config package source:

- source: `/Users/tom/github/file/mosdns/config/config_up`
- builder: `/Users/tom/github/file/mosdns/config/build_config_up.sh`

For structural config releases:

- edit the YAML files under `config_up/`
- update `SCHEMA` and `PACKAGE_ID` in `build_config_up.sh`
- update `managed_files` / `deleted_files` as needed
- run `build_config_up.sh`
- commit and push regenerated `config_up.zip`, synced `config_all/`, and `config_all.zip` in the `jasonxtt/file` repository

Package boundaries:

- `managed_files` may replace only maintained structure files
- `create_if_missing` may add defaults without overwriting operator choices
- `delete_files` may remove obsolete `sub_config/*.yaml` files only

## Dedicated Routing And Rule Order

The canonical feature name is `special_groups`.

Dedicated routing groups bind rules/lists to upstream groups and generated routing entries. UI changes to groups, local lists, or online rules can change real routing behavior.

Rule order is behavior. Earlier generated routing rules usually win, so any UI that edits list order or priority is editing runtime behavior.

## Switch Bits And Fast Marks

Switch plugin bit allocation shares a namespace with query-context fast marks and config `fast_mark` checks.

Before adding or renumbering a switcher:

- inspect `pkg/query_context` for the current switch mask range
- grep active config packages for the candidate `fast_mark` value
- check both this repo and `/Users/tom/github/file/mosdns/config`

Known reservation:

- `fast_mark 48` is the unified-matcher sentinel and must not be used by a switcher
- `switch17` uses bit 49 to avoid that collision

A collision can skip `unified_matcher1`, make rules show as `unmatched_rule`, and fall through to FakeIP. Verify with deployed audit fields such as `domain_set`, `effective_tag`, `final_sequence`, `final_upstream`, and `matched_rule_source`.

## Query Log Interpretation

The user cares about the final effective routing path for a domain.

UI and diagnostics should prefer showing:

- the effective routing label
- the final upstream group
- the final upstream path

Showing every intermediate tag as if all were effective is misleading.

## Deployment

The user typically validates real behavior on deployed hosts, not only local tests.

Use the repo build scripts/workflows for binaries that embed Vue assets. They rebuild the UI before Go compilation. If bypassing them, preserve that order manually.

Usual validation order:

- build locally
- verify on `mos-test` / `10.0.0.91`
- promote to `mosdns` / `10.0.0.3` only after confirmation

Do not commit private credentials or passwords into repo docs.
