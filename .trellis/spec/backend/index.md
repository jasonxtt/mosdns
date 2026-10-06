# Backend Development Guidelines

These files are short, source-backed rules for MosDNS-T backend and Rust migration work. The detailed project sources of truth remain `AGENTS.md` and `docs/ai/`.

## Guidelines index

| Guide | Use it for |
|---|---|
| [Directory structure](./directory-structure.md) | Choosing the owning package and avoiding cross-layer coupling |
| [Configuration compatibility](./config-compatibility.md) | YAML, generated config, runtime state, API, and WebUI compatibility |
| [Error handling](./error-handling.md) | Go propagation, API failures, fallback, and FFI safety |
| [Logging](./logging-guidelines.md) | Structured operational logs without secrets |
| [Quality](./quality-guidelines.md) | Tests, builds, deployment order, and surgical changes |
| [Rust migration](./rust-migration.md) | Architecture, reuse policy, ABI, and phase gates |
| [Native audit control](./native-audit-control.md) | Runtime capture, settings, v1/v2 API and Vue audit proof |
| [Native response policy](./native-response-policy.md) | Immutable hosts/redirect/IP snapshots and TTL compilation |
| [Native client/ECS](./native-client-ecs.md) | Trusted socket identity and client_ip; ECS slices tracked by owning task |
| [Native cache lifecycle](./native-cache-lifecycle.md) | Cache publication/refresh, v2 persistence, management HTTP and Vue workflow |
| [Native switch state](./native-switch-state.md) | Switch admission facts, durable owner state, managed rebind and native UI controls |

## Before development

1. Read the project documents in the order defined by `AGENTS.md`.
2. Use `rg` and direct source reads for call chains and impact analysis.
3. For Rust work, confirm the active Trellis task has reviewed `prd.md`, `design.md`, and `implement.md` before changing runtime code.
4. Preserve unrelated dirty worktree changes and do not let task tooling auto-commit them.
