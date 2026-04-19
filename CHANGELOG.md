# Changelog

## Unreleased

## v0.2.0

### Added

- introduced a Vue 3 WebUI implementation under `/log` with modular pages for overview, query logs, rules, upstreams, data management, and system settings
- added modal-based detail and edit flows across the new UI (query detail, top-domain detail, slow-query detail, rule edit, upstream edit, adblock/subscription edit)
- added donut-chart visualization and percentage display for domain-set hit ranking in overview

### Changed

- aligned `/log` information architecture, first-level navigation, and major interaction patterns with the legacy dashboard behavior
- switched overview ranking tables (`Top 域名`, `Top 客户端`, `最慢查询`, `分流命中排行`) to adaptive table layouts to avoid horizontal drag on typical widths
- unified page refresh behavior with a global refresh action and removed redundant per-module refresh controls in migrated modules
- refined system settings layout and mode descriptions:
  - `兼容模式`: 表外域名优先国内dns解析，保证速度
  - `安全模式`: 表外域名仅用国外dns解析，阻止dns泄漏

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.1.13

### Changed

- fixed diversion type labels in WebUI so `cuscn` is displayed as `!cn@cn` and `cusnocn` is displayed as `cn@!cn`, matching the actual routing behavior
- updated diversion-rule help text to avoid confusion between label wording and backend rule semantics
- improved inline confirmation popover placement near viewport edges; when the trigger is near the bottom of the screen, the popover now flips upward and remains clickable

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.1.12

### Added

- added sortable headers for subscription-rule and ad-block rule lists in the merged dashboard
- added sortable headers for the upstream table and a quick toggle to hide disabled upstreams

### Changed

- default rule-list ordering now shows newly added items first until the user chooses a different sort key
- default upstream ordering now shows newly added upstreams first until the user chooses a different sort key
- local rule tabs are now labeled `本地规则 / 订阅规则 / 广告拦截` with clearer inline guidance for whitelist, greylist, DDNS, and special-group lists
- the diversion-rule creation dialog now auto-fills the rule name and local `srs/...` file path from the URL
- saving a diversion subscription now closes the dialog immediately while background download and refresh continue
- refreshed README wording to reflect the WebUI module refactor and the current released version

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.1.11

### Changed

- unified confirmation interactions in the merged dashboard so destructive and important actions now use the same inline popover style instead of mixed native dialogs
- updated the `SOCKS5 / ECS IP` action buttons for clearer contrast and aligned the `保存` button with the main primary-button styling

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.1.10

### Changed

- removed the `fastCache` FNV-1a input truncation so long DNS questions no longer hash on only the first 128 bytes
- normalized `ClientAddr` with `Unmap()` before the UDP fast path runs, reducing IPv4-mapped IPv6 ambiguity in the fast path

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.1.9

### Added

- added a dedicated `数据管理` section to the merged dashboard for cache and generated-domain maintenance
- added a one-click `清空所有缓存` action in cache management
- added an inline confirmation popover when switching the core mode between `兼容模式` and `安全模式`

### Changed

- refined the merged dashboard information architecture and moved upstream management into a cleaner `上游设置` area
- updated the upstream page so the top strip is lighter and the upstream table remains the primary focus
- simplified special-group management in the upstream header and improved its visual hierarchy
- moved `重启 MosDNS` into the system information module
- moved `SOCKS5 / ECS IP` into the system settings area
- removed automatic background update checks from the dashboard; update checks are now user-triggered only
- renamed the update action from `强制检查` to `检查更新`

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.1.8

### Added

- added a new merged dashboard UI at `/` with top-level sections for 概览, 查询日志, 规则管理, 上游, and 系统
- old root UI is still available at `/legacy` during the transition

### Changed

- merged the old `:9099` log-capture and analysis workflow into the new dashboard's 查询日志 section
- promoted upstream management to a top-level navigation area instead of keeping it buried under system settings
- reorganized system-facing controls so query, rule, upstream, and system functions are separated more clearly
- updated README and release-facing docs to use `mosdns/config/config_all.zip` as the full template package
- retired the legacy `config_tom.zip` template reference from the source repo documentation

### Upgrade Notes

- this release does not require a config change for existing users
- the maintained full template package is now `mosdns/config/config_all.zip`
- the maintained incremental package remains `mosdns/config/config_up.zip`

## v0.1.7

### Added

- special groups now support manual domain lists in WebUI list management, alongside the existing online `srs` sources
- special group metadata now exposes the manual list plugin tag and manual rule file path for the WebUI

### Changed

- query-log details now show `matched_rule_source` for online diversion matches
- query-log details now distinguish `最终上游组` and the actual winning `最终上游`
- the `最终序列` field is no longer shown in the WebUI query-log detail panel
- list management now renders special groups dynamically after they are created in advanced settings
- removed the legacy `NFT IP` item from list management
- special-group deletion now also removes the corresponding manual rule file under `rule/special_<slot>.txt`

### Upgrade Notes

- for existing WebUI fork users, the incremental package `mosdns/config/config_up.zip` now updates `sub_config/special_groups.yaml`
- no pre-created `rule/special_<slot>.txt` files are required; they will be created when the user saves manual rules in WebUI

## v0.1.6

### Changed

- removed nft-related integrations from the binary and WebUI
- removed legacy repo cruft that was not part of the maintained product surface
- full config packages were refreshed to match the nft-free runtime

### Upgrade Notes

- old configs that still reference `nft_add` are not compatible with this version
- for existing WebUI fork users, the only required config change is `sub_config/rule_set.yaml`
- the incremental package `mosdns/config/config_up.zip` updates only `sub_config/rule_set.yaml` and does not reset user-maintained override files

- update checking now targets `jasonxtt/mosdns` instead of the upstream repository
- the built-in updater now matches the fork's Linux `tar.gz` release assets
- WebUI project links now point to `jasonxtt/mosdns`
- build version injection is now consistent across default builds, preview builds, and tagged releases

## v0.1.0-preview

Initial preview release for the WebUI-enhanced fork based on `yyysuo/mosdns`.

### Added

- dedicated routing groups in WebUI
- dedicated group APIs with support for up to 10 WebUI-managed groups
- rule-to-upstream binding for dedicated groups
- automatic `.srs` download after saving online diversion rules
- hot reload for aliapi upstream groups after WebUI save
- improved query-log display for dedicated routing groups

### Changed

- rule management now supports dynamic dedicated-group types
- upstream management is integrated with dedicated routing groups
- query log tags display dedicated group names together with stable mark identifiers
