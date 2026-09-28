# Rust-native domain_set management — implementation plan

## Start gates

- [ ] Parent roadmap review is `FINAL: PASS` in the same C2C conversation.
- [ ] 5B `fast_mark`/`flow_setter` child has its own same-chat `FINAL: PASS`.
- [ ] Activate this child with `task.py start` only after both review gates.
- [ ] Read repository instructions, `.trellis/workflow.md`, Rust host/API specs,
  and run `trellis-before-dev` for the actual native-host package boundary.

## Ordered behavior slices

### Slice 0 — freeze management profile

- [ ] Trace the current Go handler and native config rule source model; record
  first-file, `.txt`, `exps`/`files`/`sets`, and per-rule rejection semantics in
  `design.md`.
- [ ] Decide and test which configuration shapes are management-eligible and
  how unsupported composite shapes fail visibly. Keep existing load-time query
  behavior for shapes not eligible for management.
- [ ] Do not code the API before this contract is clear.

### Slice 1 — HTTP contract (red → green → refactor)

- [ ] Add failing tests through the real native HTTP router for plugin mount,
  GET `/show`, GET `/save`, POST `/post`, payload, status, response body, and
  content type. Include empty/valid rules and unavailable write target.
- [ ] Implement the smallest plugin-scoped handler and provider interface
  needed to serve one live file-backed `domain_set`.
- [ ] Keep the HTTP router real in all contract tests; use a temporary directory.

### Slice 2 — success and failure atomicity

- [ ] Add a failing test: start with A, POST B, verify successful file content,
  GET show reports B, and the next real DNS query reflects B.
- [ ] Add malformed JSON and injected persistence failure cases. Verify old
  file bytes, show output, and live DNS behavior remain A.
- [ ] Characterize and test current invalid individual rule handling; do not
  silently substitute a new policy.
- [ ] Implement complete candidate compilation, safe persistence, then one
  atomic generation swap; refactor after green.

### Slice 3 — concurrency, restart, and lifecycle

- [ ] Add coordinated real DNS queries around the update and prove each query
  sees all-A or all-B rules, never a mixed snapshot.
- [ ] Restart a fresh assembly against the same file and prove it loads B.
- [ ] Close an assembly with API and DNS listeners; verify owned tasks exit and
  both loopback addresses rebind. Add cleanup on failed assembly startup.
- [ ] Implement only the smallest required immutable-generation and listener
  ownership changes; do not create a generic runtime framework.

## Validation

Run from `rust/`:

1. Focused domain-set management tests plus `slice2_config` and
   `slice3_composition` regression tests.
2. `cargo fmt --all -- --check`
3. `cargo clippy --workspace --all-targets -- -D warnings`
4. `cargo test --workspace`
5. If the child design selects a Linux process E2E, run its exact bounded
   functional recipe. It is not a benchmark or performance gate.

## Evidence, review, and closeout

- [ ] Update only actual delivered domain_set management subitems in
  `docs/rust/feature-coverage.md`, linking tests/evidence. Keep complete P02,
  C04/C10/C11/C17, all 5C, and config-package compatibility pending.
- [ ] Run `git diff --check`, inspect the exact changes, and commit only this
  task's paths. Preserve unrelated dirty files.
- [ ] Send exact task ID, `BASE_SHA`, `HEAD_SHA`, and paths to the same C2C
  conversation for a complete review. Do not paste source, diffs, or logs.
- [ ] Fix findings in narrow commits/ranges and repeat in the same chat until
  explicit `FINAL: PASS`.
- [ ] Record tested and deferred config shapes, failures, and lifecycle limits
  in this task and the parent roadmap.
