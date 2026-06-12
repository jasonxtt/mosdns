# Config Notes

## Core principle

In this fork, the WebUI is part of the configuration system. It is not just a viewer.

When the user edits upstreams, rule lists, dedicated routing groups, or related settings in the UI, the expected result is:

- persistent state is updated
- generated config changes accordingly
- runtime behavior changes accordingly

Any UI change that ignores config generation is incomplete.

## Runtime config shape

The maintained config package for deployments is expected to live under:

- `/cus/mosdns`

Published package references in the README:

- `config_all.zip`
- `config_up.zip`

The deployment package contains the runtime files the operator actually uses. Some generated files and local JSON state exist in deployment/runtime form and are not all represented in this repository one-to-one.

Important distinction:

- not every persisted UI state is a YAML rule/config entry
- some operator-facing UI settings are stored as managed runtime JSON files instead

Current examples include appearance-related state such as:

- panel background settings and history
- text color settings
- button color settings
- domain generation switch settings

Those still matter operationally even though they do not belong in the generated routing YAML.

Small runtime JSON state files are now managed under:

- `/cus/mosdns/webinfo`

The binary auto-creates this directory and migrates matching files from the runtime root or legacy `/cus/mosdns/state` there on startup:

- `appearance_settings.json`
- `appearance_text_settings.json`
- `appearance_button_settings.json`
- `audit_settings.json`
- `webui_port_settings.json`
- `config_overrides.json`
- `upstream_overrides.json`
- `special_upstream_groups.json`
- `config_update_state.json`
- `domain_generation_settings.json`

When both an old path and the new path exist, prefer the file under `webinfo/`.

## Config package updates

The binary declares the external config structure it requires through
`requiredConfigSchema` and `requiredConfigPackageID` in
`coremain/config_update.go`.

- Binary-only releases keep both values unchanged.
- Structural config releases bump both values and publish a matching
  `config_up.zip`.
- The internal schema is not the user-facing version label. Keep schema
  monotonic for upgrade logic, and update the UI display mapping separately
  when a structural config release ships. Current labels are `v1` and `v2`.
- The package is external and uses a manifest; it is not embedded in the
  binary.
- The external package source is maintained in
  `/Users/tom/github/file/mosdns/config/config_up`, with the builder script at
  `/Users/tom/github/file/mosdns/config/build_config_up.sh`.
- For structural config releases:
  - edit the YAML files under `config_up/`
  - bump `SCHEMA` and `PACKAGE_ID` in `build_config_up.sh` to match
    `coremain/config_update.go`
  - add new maintained YAML files to the script's `managed_files`
  - add removed `sub_config/*.yaml` files to the script's `deleted_files`
  - run `build_config_up.sh`, which regenerates `manifest.json`,
    `config_up.zip`, syncs `config_all/`, and rebuilds `config_all.zip`
  - commit and push the regenerated files in the `jasonxtt/file` repository
- `managed_files` may replace only maintained structure files.
- `create_if_missing` supplies defaults for newly introduced switch state
  files without overwriting existing operator choices.
- `delete_files` may remove obsolete `sub_config/*.yaml` files only. A deleted
  file must not also be listed in `managed_files`.
- User rules, WebUI state, upstream state, generated files, caches, and SRS
  data are not managed by the package.
- Do not put `webinfo/config_update_state.json` in the package. The binary
  writes that state after the transactional upgrade and plugin initialization
  succeed.

Successful application is recorded in
`webinfo/config_update_state.json`. Backups are kept in unique directories
under `backup/`, and incomplete or failed updates are rolled back.

## Dedicated routing groups

The most important custom config concept is `special_groups`.

Each dedicated routing group is effectively a custom routing slot with its own binding information. In practice that means:

- group identity
- bound upstream group
- list membership
- generated routing entry
- related cache path/behavior

The UI is expected to let the user:

- create or edit dedicated routing groups
- assign upstreams to a group
- bind local lists to a group
- bind online subscription rules to a group

## Rule order

Rule order matters.

When discussing "list order" or "priority", the effective meaning is the generated routing order in config. Earlier matching rules usually win, so any UI that edits rule ordering is implicitly editing behavior.

## Cache model

There are two different cache scopes to keep conceptually separate:

- global cache
- per-upstream or per-routing-path cache

The fork already discussed this distinction and made one explicit decision:

- the newer custom dedicated-routing path should not use global cache for now

Do not casually merge those cache scopes together without re-evaluating behavior.

## Query log interpretation

The user cares about the final effective routing path for a domain.

That means UI and diagnostics should prefer showing:

- the effective routing label
- the final upstream group
- the final upstream path

Showing every intermediate tag as if all of them were effective is misleading.

This matters especially for:

- remembered direct / proxy labels
- subscription-hit labels versus final effective labels
- UI wording such as “命中标签” versus “生效标签”

## Current Type65 / HTTPS note

Current behavior can block `HTTPS` (`Type65`) records entirely.

Current behavior does not yet selectively remove:

- `ipv4hint`
- `ipv6hint`
- `ECH`

If requested, that should be implemented as a response-rewrite plugin after upstream resolution, not as a superficial UI-only toggle.

## Deployment workflow assumptions

The user typically validates real config behavior through deployed hosts, not only local tests.

Usual expectation:

- build locally
- verify on the test machine first
- only then promote to production

For binaries that embed the maintained Vue UI, the practical build workflow is:

- rebuild `webui-log/` first so embedded assets are up to date
- then run `go build`

Do not assume parallel frontend/backend builds are safe for release artifacts in this fork.

Do not commit private credentials or passwords into repo docs. Keep only non-secret operator workflow notes here.

## Domain generation runtime switches

The current first version of domain-generation control is intentionally implemented as runtime binary state, not as a structural external config-package change.

Current exposed switches:

- `enabled`
- `remember_direct`
- `remember_proxy`
- `no_v4`
- `no_v6`

Behavior notes:

- disabling the total switch stops new generated-domain writes and clears the currently generated domain data for the maintained domain-output lists
- disabling an individual sub-switch stops new writes for that list and clears its current generated data
- this design keeps `requiredConfigSchema` and `requiredConfigPackageID` unchanged for the first rollout
