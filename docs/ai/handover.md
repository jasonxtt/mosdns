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

## Rust migration planning

The canonical cross-session Rust state, resume procedure, task gates, and
dirty-worktree ownership boundary are in `docs/ai/rust-handover.md`. Read it
before changing or staging Rust migration files.

- The long-lived local `rust` branch was created from `main` v0.7.1 (`3896a4a`) on `2026-08-13`.
- The migration architecture and acceptance gates are documented in `docs/ai/rust-rewrite-plan.md`.
- The old `/Users/tom/github/mosdns-rust-cache` workspace served as a selective cache compatibility reference. The new foundation was implemented in this repository with hardened FFI, concurrency, metrics, fallback, and parity coverage; keep the old workspace read-only and never treat it as a drop-in subtree.
- Do not merge the old Rust workspace wholesale or copy its older release workflows over the current Vue-aware workflows.
- KixDNS (`olicesx/kixdns`) is the preferred upstream source for reusable Rust data-plane work. The audited baseline is `2da3a2d` (`2026-08-12`); reuse its raw DNS utilities, ECS, Moka/Bytes cache approach, matchers, indexes, and later transports through mosdns compatibility adapters rather than adopting its JSON pipeline or whole binary.
- Trellis 0.6.14 is initialized as lightweight planning/governance on the `rust` branch. It runs inline with `session_auto_commit: false`; `.trellis/tasks/08-13-rust-cache-foundation/` and `.trellis/tasks/08-13-rust-matcher-foundation/` are archived/completed on `2026-08-13`; A–E exact-scope work commits are completed and reviewed, F is the task-archive finish commit, and G is a journal-only finish commit under the explicit `--no-commit` Trellis sequence. The overall Rust rewrite remains active, and Rust remains experimental/default Go-only.
- Cache hot-path profiling on `mos-test` identified the fixed cgo transition, output copying, and duplicate lifecycle locks as the main costs. Borrowed Moka keys, caller-owned output buffers, atomic handles, and an allocation-free TTL walk reduced the preliminary median gap from about 85.7% to a noisy 39% range and Rust allocations from 704 to 688 B/op; Rust remains experimental because the 10% gate and p99/CPU/RSS soak are not met.

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
