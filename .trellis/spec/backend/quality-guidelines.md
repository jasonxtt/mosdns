# Quality Guidelines

## Change discipline

- Make the minimum change that satisfies the active task. Do not refactor adjacent code or reformat unrelated files.
- Every changed line must trace to a requirement in the task. Remove only imports or code made unused by that change.
- Preserve unrelated dirty worktree changes. Trellis auto-commit is disabled for this repository.
- Behavior parity comes before cleanup during the Rust migration. Keep a working Go fallback until the task's removal gate is met.

## Verification

- Add a failing regression or parity test before changing behavior, then make it pass.
- Run focused package tests while iterating and `go test ./...` before handing off backend changes.
- Run race tests for concurrency-sensitive Go bridges and cache code.
- Rust crates must pass `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings`.
- FFI changes require Linux+cgo integration coverage plus sanitizer/Miri or an equivalent focused memory-safety check where applicable.
- Cache migration requires golden parity for TTL, negative answers, ECS, exclusion, lazy update, dump/API behavior, raw UDP/TCP/HTTP responses, concurrency, metrics, and fallback.

## Build and deployment

Release/deployment binaries must contain freshly built Vue assets; use repository build scripts rather than a bare `go build`. For deployment work, validate locally, then on `mos-test` (`10.0.0.91`), and promote to production only after confirmation.

## Review gates

- No config/API/metric/audit compatibility regression.
- No panic or ambiguous ownership across FFI.
- No hot-path O(n) scan, global serialization, or excessive cgo calls introduced by the Rust backend.
- No undocumented KixDNS source copy; pin and attribute extracted code.
- If p99, CPU, or RSS regresses beyond the task's accepted threshold, keep Rust experimental rather than making it default.
