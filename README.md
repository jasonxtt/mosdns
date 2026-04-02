# mosdns

[简体中文文档](README.zh-CN.md)

An enhanced fork of `yyysuo/mosdns`, focused on making long-term DNS rule maintenance practical through a stronger WebUI workflow.

This fork keeps the upstream architecture and core configuration style, while adding a set of features for rule-driven routing, upstream management, and day-to-day operations.

## Default Config Package

The default config package for this fork is available here:

- [config_tom.zip](https://github.com/jasonxtt/file/blob/main/mosdns/config_tom.zip)

Usage:

1. Download `config_tom.zip`
2. Extract it
3. Rename the extracted directory to `mosdns`
4. Place it under `/cus`

The final runtime config path should be:

- `/cus/mosdns`

After that, `mosdns` can be started directly with this config package.

## Upstream

- Upstream repository: `yyysuo/mosdns`

## What This Fork Adds

### Dedicated Routing Groups

- Create dedicated routing groups from the WebUI
- The current implementation supports up to 10 dedicated routing groups
- Each group can bind its own:
  - online rule set
  - upstream group
  - cache

### Rule-to-Upstream Binding

- Select a dedicated routing group directly in rule management
- Domains matched by that group are sent to the bound upstream path

### Automatic Rule Download After Save

- When an online diversion rule is created with `url + files`, the backend downloads the `.srs` automatically
- No extra manual "Update" click is required

### Upstream Hot Reload

- Saving upstream settings in the WebUI takes effect immediately
- No manual `mosdns` restart is required for supported aliapi upstream groups

### Better Query Log Display

- Dedicated routing groups are shown with readable names in the query log
- Final sequence and upstream information are available for troubleshooting

## Typical Use Cases

- Home network DNS splitting
- Bypass DNS deployments
- Proxy environments with long-lived geosite / srs rule maintenance
- Users who want to manage routing and upstream behavior from WebUI instead of editing YAML repeatedly

## Key Differences From Upstream

This fork currently adds:

- dedicated routing group APIs and WebUI flows
- upstream hot reload for aliapi groups
- automatic online rule download after save
- friendlier dedicated-group display in query logs

A detailed summary is available here:

- [Fork Diff Summary (Chinese)](docs/fork_diff_summary_zh.md)

## Release Positioning

The current state is suitable for a first preview release, for example:

- `v0.1.0-preview`

The implementation is already usable end-to-end, but this is still a scenario-driven enhancement fork rather than a generic upstream replacement.

## Documentation

- [Chinese README](README.zh-CN.md)
- [Project Intro Draft (Chinese)](docs/github_project_intro_zh.md)
- [Fork Diff Summary (Chinese)](docs/fork_diff_summary_zh.md)
- [GitHub Release Checklist (Chinese)](docs/github_release_checklist_zh.md)

## Acknowledgements

This project is based on:

- `yyysuo/mosdns`
