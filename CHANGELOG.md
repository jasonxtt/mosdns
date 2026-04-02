# Changelog

## Unreleased

### Changed

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
