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

Do not commit private credentials or passwords into repo docs. Keep only non-secret operator workflow notes here.
