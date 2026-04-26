# Changelog

## Unreleased

## v0.3.11

### Changed

- fixed dark-theme readability regressions in custom-background/panel-background mode:
  - restored visible text color for `专属分流组` name chips
  - fixed contrast for `msg.success` / `result-badge.fail` / requery status chips under glass background
- fixed mobile overlap risk in data-management `刷新分流缓存` module:
  - removed rigid fill-height constraints in inline modules
  - made requery status/actions/scheduler rows wrap safely on narrow screens
- bumped embedded Vue asset query version in `log.html` to force cache refresh for updated CSS

### Upgrade Notes

- this release does **not** require manual `config_up.zip` updates
- if client-side cache is stale, refresh once to load `v20260427-v052` frontend assets

## v0.3.10

### Added

- added panel-background solid-color picker support directly in the `面板背景` row:
  - the picker is now inline (left of `上传`) instead of a separate control block
  - solid-color background can be applied with the same appearance workflow as image backgrounds
- added theme-isolated panel solid-color storage:
  - `明亮` and `黑暗` now keep independent panel solid-color values
  - backend appearance payload now carries `light_color` and `dark_color` while keeping legacy `color` compatibility

### Changed

- refactored top-level and second-level page chrome to reduce redundant stacked panels:
  - removed extra top title-wrapper panels for overview/data/upstream/system pages
  - query/rules subnavigation now uses a clean strip + divider layout
- refined list/data/system module structure to reduce duplicate background layers and tighten action alignment
- updated appearance handling:
  - panel background preview now follows theme switching immediately
  - removed low-value success toasts for panel-background apply/upload flows
- strengthened mobile safety styling:
  - enabled `-webkit-text-size-adjust: 100%`
  - removed fixed-height pressure points in dual system cards
  - simplified narrow-screen WebUI-port row layout to prevent overlap on iOS Safari
- bumped embedded Vue asset query version in `log.html` to force cache refresh for the updated frontend bundle

### Upgrade Notes

- this release does **not** require manual `config_up.zip` updates
- existing panel background `color` settings remain compatible; on next save they will be normalized into theme-isolated fields

## v0.3.9

### Added

- added end-to-end WebUI port management:
  - new system APIs: `GET /api/v1/system/webui-port` and `POST /api/v1/system/webui-port`
  - persistent settings file: `webui_port_settings.json`
  - system-settings UI module for current/target port with confirm-and-restart flow

### Changed

- replaced hardcoded local restart endpoint usage with dynamic endpoint resolution from active WebUI listen address
- updated update-manager and config-manager post-save/post-upgrade restart hooks to follow configured WebUI port
- updated requery URL action calls to normalize local loopback targets to the configured WebUI port instead of fixed `9099`
- improved requery diagnostics in UI:
  - added `last_error` in task status
  - data-management panel now shows latest failure reason
  - sub-second completion now renders as `耗时 <1秒`
- refined menu/switch theme behavior:
  - dark mode switch ON knob uses white for better contrast
  - first-level and second-level menu colors now fully invert with light/dark theme
  - removed the extra selected-outline effect on second-level menu buttons
- normalized page top-subnav panel structure and fixed scrollbar-gutter behavior to reduce visible horizontal layout jitter when switching main tabs

### Upgrade Notes

- this release can generate and use `webui_port_settings.json` under runtime config directory when WebUI port is saved
- no manual `config_up.zip` update is required for this source release

## v0.3.8

### Added

- added a panel-background history workflow in Vue system settings:
  - list previously uploaded background images
  - re-apply a historical image as current panel background
  - delete single history entries or clear all history entries
- added per-theme text color customization support:
  - light/dark theme text colors are stored independently
  - color changes apply and save immediately from the picker
  - optional eyedropper entry is available on supported browsers

### Changed

- improved second-level tab selected-state visibility by adding a clearer selected border for:
  - `实时查询 / 诊断抓取`
  - `本地规则 / 订阅规则 / 拦截规则`
- upgraded appearance reset behavior to reset the whole theme/appearance stack with confirmation:
  - theme -> `明亮`
  - panel background -> cleared
  - transparency -> `100%`
  - blur -> `0px`
  - text color -> default
- removed remaining hardcoded green text/button text colors in multiple UI surfaces so text color follows the active theme text color more consistently

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.3.7

### Changed

- hardened `刷新分流缓存` task start/status behavior to avoid occasional immediate `0秒完成` false display:
  - backend `requery` trigger/scheduler now persist `running` state atomically before goroutine execution
  - frontend `数据管理` requery panel adds trigger-pending state and adaptive polling (`1s` pending / `5s` running)
  - status text now renders sub-second completion as `耗时 <1秒` instead of misleading `0秒`
- aligned panel-background transparency behavior for additional scheduler/requery card surfaces and button text contrast in dark-theme custom background usage

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.3.6

### Changed

- fixed Vue overview `查询趋势` panel background so transparency/glass settings now apply consistently to the trend card container
- bumped embedded dashboard asset query version to refresh browser cache for updated panel styles

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.3.5

### Added

- introduced a dynamic real-time `查询趋势` monitoring workflow in the Vue dashboard:
  - live polling with a sliding time window for `请求数 / 平均处理时间`
  - synchronized KPI updates for `总查询数 / 平均处理时间 / 当前请求数 / 当前处理时间`
  - interactive series toggles and smoother live trend behavior

### Changed

- refined Vue WebUI interaction and visual behavior around:
  - list/rules/upstream/system operation controls
  - mobile compact layout and overflow handling
  - panel background, transparency, and glass-effect appearance controls

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.3.4

### Changed

- optimized remote config apply (`/api/v1/config/update_from_url`) backup behavior:
  - backup now only includes files that will be overwritten by the incoming ZIP
  - avoids full-directory backup inode pressure on large runtime trees
- refined rules WebUI operations:
  - subscription rules now support `更新全部规则`
  - adguard module buttons renamed to `新增拦截规则` and `更新全部规则`
  - adguard list now supports per-rule `更新`
- updated the subscription `更新全部规则` button style to the same warning/red tone used by update actions in adguard

### Added

- added adguard single-rule update API:
  - `POST /plugins/adguard/update/{id}`

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.3.3

### Changed

- refined mobile layout behavior to avoid module-level overflow:
  - top-level menu (including refresh button) now stays in one row without horizontal scrolling
  - reduced mobile menu button spacing and font size to fit one-line layout
- improved small-screen table behavior with a “compress first, scroll only when needed” approach:
  - cache-management table spacing and font size are compacted on mobile
  - `高级替换规则` and cache/stat tables scroll within table area when width is insufficient
  - prevented panel/module containers from exceeding viewport width
- adjusted inline module width handling:
  - `域名列表统计` and `刷新分流缓存` modules now stay within screen width like other panels

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.3.2

### Changed

- improved mobile UI usability across the Vue dashboard:
  - top-level navigation now stays on one line with the refresh button and supports horizontal scrolling
  - diagnostic capture modules (`请求列表` / `分析结果`) now support horizontal scrolling on narrow screens
  - system `高级替换规则` table now uses a horizontal scroll layout on small screens
  - data management cache/stat tables now use horizontal scrolling to avoid clipped column headers
  - `刷新分流缓存` scheduler inputs are aligned to consistent field width on mobile
- restored list-management hint copy from legacy `/log`:
  - brought back descriptions for 白名单/黑名单/灰名单/DDNS/客户端IP/直连IP/重定向/RealIP 相关名单
  - list status now shows `共 X 行` consistently

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.3.1

### Added

- introduced a dynamic, real-time `查询趋势` module in the Vue dashboard overview:
  - live polling update with sliding time window
  - synchronized `总查询数 / 平均处理时间 / 当前请求数 / 当前处理时间`
  - interactive series toggles for `请求数` and `平均处理时间`

### Changed

- refined the overview trend card layout and responsive behavior for narrow screens
- aligned latency number styling and module interactions for consistent monitoring UX

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

## v0.3.0

### Added

- added a Vue-based main dashboard experience and promoted it to the root path `/`
- added a `经典绿` color preset in the Vue UI appearance settings to match the preferred legacy green palette

### Changed

- swapped WebUI entry routes:
  - `/` now serves the Vue UI
  - `/log` now serves the previous legacy dashboard
- expanded and stabilized Vue UI behavior across overview, query logs, rules, data management, upstream management, and system settings modules
- refined overview presentation:
  - top summary cards now focus on `总查询数` and `平均耗时`
  - detail modal stacking behavior fixed so nested detail dialogs always appear on top
  - module-internal scrollbars are thinner and visually lighter

### Upgrade Notes

- this release does **not** require a config change for existing users
- no `config_up.zip` update is required for this source release

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
