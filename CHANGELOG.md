# Changelog

## v0.1.0-preview

Initial preview release for the WebUI-enhanced fork based on `yyysuo/mosdns`.

### Added

- dedicated routing groups in WebUI
- dedicated group APIs backed by `mark50-mark60`
- rule-to-upstream binding for dedicated groups
- automatic `.srs` download after saving online diversion rules
- hot reload for aliapi upstream groups after WebUI save
- improved query-log display for dedicated routing groups

### Changed

- rule management now supports dynamic dedicated-group types
- upstream management is integrated with dedicated routing groups
- query log tags display dedicated group names together with stable mark identifiers
