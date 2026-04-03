# Changelog

## Unreleased

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
