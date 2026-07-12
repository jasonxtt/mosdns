# Handover

## Stable baseline

The maintained line has these active realities:

- `/` is the maintained Vue UI
- `/log` is the legacy compatibility UI
- `webui-log/` is the active frontend source even though the folder name still says `log`
- dedicated routing groups use the backend name `special_groups`
- online rules, local lists, and upstream-group binding are already first-class workflow pieces
- the maintained UI already ships operator-facing controls such as `IPv4优先`, `IPV6屏蔽`, appearance persistence, and transactional config-update status
- domain-generation controls are runtime JSON state, not external config-package structure changes
- domain-generation exposes `总开关 / 记忆直连 / 记忆代理 / 记忆无v4 / 记忆无v6`

## Keep in mind

- Infer intended line from the repo folder: `mosdns` means `main`, `mosdns-lite` means `lite`.
- Do not follow upstream `nft` / `eBPF` work for this fork.
- Do not re-review upstream changes on or before `2026-04-18` unless explicitly asked.
- Use `special_groups`, not `route_group`.
- Keep secrets out of repo docs. Non-secret deployment notes that are safe to keep:
  - test host: `10.0.0.91` (`mos-test`)
  - production host: `10.0.0.3` (`mosdns`)
  - related debug hosts: `10.0.0.2` (`sing-box`), `10.0.0.6` (`network-vm`)
  - runtime config root on deployed hosts: `/cus/mosdns`

## Release and config reminders

- For embedded Vue assets, use the repo build scripts/workflows so `webui-log/` is built before `go build`.
- Binary-only releases keep `requiredConfigSchema` and `requiredConfigPackageID` unchanged.
- Structural config releases bump schema/package and rebuild the external `config_up.zip`; see `docs/ai/config-notes.md` for details.
- User-facing config version text is separate from the internal schema. Current UI labels are `v1` and `v2`.
- Validate in this order when deployment matters:
  - local build
  - test host
  - production only after confirmation
