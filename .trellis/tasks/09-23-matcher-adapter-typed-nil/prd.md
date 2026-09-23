# Fix matcher adapter typed-nil errors

## Goal

Ensure the three Linux+cgo Rust matcher snapshot constructors return a true nil
interface whenever snapshot creation fails, so callers can safely branch on
`snapshot == nil` alongside the returned error.

## Requirements

- Correct error returns from `BuildDomainSnapshot`, `BuildIPSnapshot`, and
  `BuildValuedDomainSnapshot` without changing their successful-build behavior,
  Go-side validation, FFI status mapping, or non-Linux/stub behavior.
- Add Linux+cgo regression coverage for a failed create through each public
  constructor. Each case must assert a non-nil error and a nil interface without
  invoking methods on the failed result.
- Keep this independent from the completed W3 routing/evidence task; do not
  modify its evidence or runtime routing behavior.

## Acceptance Criteria

- [x] All three failed-create paths return `nil` interfaces with errors.
- [x] The tagged matcher/data-provider suite passes normally and under Go's race
  detector on Linux+cgo.
- [x] `go test ./...` passes on the supported local Go environment.
- [ ] The scoped code and evidence are pushed on `rust` and receive an explicit
  `FINAL: PASS` from reviewer task
  `01a0c7fe-fd97-7ce1-aed2-d389bbefa3e3`.

## Constraints

- No Rust/ABI changes, production/default wiring, deployment, or unrelated W3
  edits.
- Preserve unrelated dirty worktree changes and stage only files in this task's
  scope.
