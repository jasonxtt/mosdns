# Changelog

## Unreleased

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
