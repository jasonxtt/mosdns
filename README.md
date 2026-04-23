# mosdns

[简体中文文档](README.zh-CN.md)

An enhanced fork of `yyysuo/mosdns`, focused on making long-term DNS rule maintenance practical through a stronger WebUI workflow, dedicated routing groups, and a Vue-based dashboard.

This fork keeps the upstream architecture and core configuration style, while adding a set of features for rule-driven routing, upstream management, and day-to-day operations.

The current UI entry points are:

- `/` for the maintained Vue UI
- `/log` for the previous legacy dashboard kept for compatibility and comparison

## Config Packages

The maintained config packages for this fork are published in:

- [`mosdns/config/config_all.zip`](https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip)
- [`mosdns/config/config_up.zip`](https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_up.zip)

Use `config_all.zip` for a fresh deployment or a full template replacement.

Use `config_up.zip` only for incremental config updates on an existing deployment.

The old `config_tom.zip` template has been retired.

After extracting the full package, the runtime path should be:

- `/cus/mosdns`

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
- Matched rule source, final upstream group, and final upstream are available for troubleshooting

## Typical Use Cases

- Home network DNS splitting
- Bypass DNS deployments
- Proxy environments with long-lived geosite / srs rule maintenance
- Users who want to manage routing and upstream behavior from WebUI instead of editing YAML repeatedly

## Key Differences From Upstream

This fork currently adds:

- dedicated routing group APIs and WebUI flows
- a Vue-based main dashboard at `/`, with the previous UI retained at `/log`
- upstream hot reload for aliapi groups
- automatic online rule download after save
- friendlier dedicated-group display in query logs

A detailed summary is available here:

- [Fork Diff Summary (Chinese)](docs/fork_diff_summary_zh.md)

## Release Status

The current released version is:

- `v0.3.1`

This fork is already used as a maintained WebUI-enhanced branch, not just a one-off preview build.

## Documentation

- [Chinese README](README.zh-CN.md)
- [Project Intro Draft (Chinese)](docs/github_project_intro_zh.md)
- [Fork Diff Summary (Chinese)](docs/fork_diff_summary_zh.md)
- [GitHub Release Checklist (Chinese)](docs/github_release_checklist_zh.md)

## Acknowledgements

This project is based on:

- `yyysuo/mosdns`
