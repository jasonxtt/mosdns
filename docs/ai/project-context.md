# Project Context

## What this fork is

This repository is an enhanced fork of `yyysuo/mosdns`. The primary value is not a large rewrite of the core DNS engine. The value is the operator workflow built around:

- dedicated routing groups (`special_groups`)
- upstream group binding
- online rule download and management
- query/audit visibility
- a maintained Vue-based WebUI

The fork keeps upstream structure where practical and extends the management layer around it.

## Repository map

- `coremain/`
  - main HTTP/API server code
  - embedded web assets under `coremain/www/`
  - audit APIs and dashboard endpoints
- `plugin/`
  - executable plugins and sequence logic
  - response-path behavior should usually be added here, not hardcoded into unrelated UI code
- `pkg/`
  - shared DNS and query context utilities
- `webui-log/`
  - current maintained Vue frontend workspace
  - despite the folder name, this is the main UI source used for the default dashboard workflow
- `webui-blog/`
  - experimental Bento-style UI workspace
  - paused, not the active production UI
- `docs/`
  - fork notes and release docs
- `docs/ai/`
  - successor-agent context docs

## UI topology

Current route expectations:

- `/` -> maintained Vue UI
- `/log` -> legacy/original UI kept for compatibility

Important nuance:

- Directory names do not perfectly match served routes anymore.
- Do not assume `webui-log/` means "the `/log` route source".
- Always verify runtime route mapping before changing frontend behavior.

## Major fork-specific capabilities

### 1. Dedicated routing groups

The user-facing concept is a dedicated routing group. Backend naming uses `special_groups`.

A dedicated routing group is a custom domain-routing bucket that can bind to:

- its own upstream group
- its own list sources
- its own cache behavior
- its own rule entry in the generated routing flow

This feature is already implemented and in use. It is not a draft idea.

### 2. Rule-to-upstream-group binding

Rules are not only classification metadata. They affect actual routing behavior.

Current expectation:

- a local/manual list can bind to one dedicated upstream group
- an online URL-based list can bind to one dedicated upstream group
- the selected group determines which upstream path is used after the rule matches

### 3. Automatic online rule download

The maintained WebUI behavior expects online rules to download after save. The user should not need a second manual step just to fetch a newly added subscription rule.

### 4. Vue main UI

The main UI was rewritten with Vue and promoted to `/`.

Goals of the rewrite:

- unify page structure
- unify modal/edit flows
- keep functional parity with the legacy dashboard
- make future feature work easier than the old dashboard script structure

The old dashboard still exists at `/log` as fallback and comparison target.

## Naming history and common confusion

- `special_groups` is the real feature name.
- `route_group` was an earlier misunderstanding and should not be used as the canonical term.

## Upstream sync policy

- This fork is not trying to track every upstream change.
- The `nft` / `eBPF` line is intentionally not followed.
- Upstream changes on or before `2026-04-18` were already checked in prior work. Do not burn time re-auditing them unless explicitly asked.

## Current release posture

The fork is already published and maintained as a real release branch, not just a local experiment.

Current released version in repo docs:

- `v0.3.8`

## Code areas likely to matter for future feature work

- frontend behavior and modal flows:
  - `webui-log/src/components/`
- embedded static pages and legacy dashboard:
  - `coremain/www/`
- audit / query APIs:
  - `coremain/api_audit.go`
  - `coremain/api_audit_v2.go`
  - `coremain/audit.go`
- routing and response processing:
  - `plugin/executable/`
- DNS query context and response objects:
  - `pkg/query_context/`
