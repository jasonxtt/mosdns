# Implementation: matcher adapter typed-nil errors

## Frozen authorization and review target

- Authorized unit: fix failed-create return values from the three Linux+cgo
  matcher snapshot constructors and add their regression tests; then update the
  applicable error-handling spec, validate, commit, push, and request review.
- Branch: `rust`.
- Parent/source SHA at task start: `93e21c6c6439dfb8266d9bf838a8ab4866ceb775`.
- Reviewer conversation: `01a0c7fe-fd97-7ce1-aed2-d389bbefa3e3`
  (`成为001号 reviewer`, local Codex task).
- Review boundary: this one bugfix only. No W3 evidence edits, Rust/ABI or
  production/default wiring changes, deployment, or task archival.

## Steps and status

- [x] Domain: make the failed-create test assert direct interface nilness, see
  the typed-nil failure, then fix `BuildDomainSnapshot` and pass the test.
- [x] IP: add an invalid-prefix test, see the typed-nil failure, then fix
  `BuildIPSnapshot` and pass the test.
- [x] Valued domain: make the failed-create test assert direct interface
  nilness, see the typed-nil failure, then fix `BuildValuedDomainSnapshot` and
  pass the test.
- [x] Run the full Linux+cgo tagged suite in normal and race modes, plus
  `go test ./...`.
- [x] Review the exact diff and update the relevant error-handling guidance
  with the Go typed-nil interface rule.
- [x] Stage exact paths, commit, and push the scoped implementation to `rust`
  as `214796fb8292a7a35ac7b03a4bd6f01657569620`.
- [x] Rerun the Linux tagged normal/race suite against an archive of that exact
  pushed commit.
- [ ] Send one complete review request with the exact commit and test evidence
  to the frozen reviewer conversation; wait for an explicit PASS or scoped
  FAIL.

## Validation commands

On Linux with the Rust static library built at the repository's expected path:

```sh
CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust go test -tags mosdns_rust ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set ./plugin/data_provider/sd_set ./plugin/data_provider/si_set ./plugin/data_provider/domain_mapper ./plugin/data_provider/matcher_adapter ./plugin/matcher/... -count=1
CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust go test -race -tags mosdns_rust ./plugin/data_provider/domain_set ./plugin/data_provider/ip_set ./plugin/data_provider/sd_set ./plugin/data_provider/si_set ./plugin/data_provider/domain_mapper ./plugin/data_provider/matcher_adapter ./plugin/matcher/... -count=1
go test ./...
```

#### Executed evidence

- Linux validation host: `mosdns-rust`, Linux amd64 (`x86_64`), Go 1.24.4,
  Rust 1.95.0. A task-scoped temporary source checkout used the task-start
  commit plus the two scoped Go source/test files from this worktree.
- Built the cgo static library with:

  ```sh
  CARGO_TARGET_DIR=/root/mosdns-matcher-typednil-target-red.jKm8lv CARGO_BUILD_JOBS=1 scripts/build-rust-cache.sh
  ```

  Staged `rust/target/release/libmosdns_runtime.a` in that temporary checkout;
  SHA-256: `8847a4b6213a5920fb6d2b6fb863a90ee7cda437a5745024dfebce6218544187`.
- The normal tagged suite and the same suite with `-race` both passed across
  all listed data-provider and matcher packages before commit.
- The pre-fix red checks failed specifically because the returned interface
  held `*matcher_adapter.snapshot` for domain and IP, and
  `*matcher_adapter.valuedSnapshot` for valued domain. Each focused check passed
  after its corresponding constructor fix.
- Local host: Darwin arm64, Go 1.26.4. `go test ./...`, `go vet ./...`,
  `gofmt -d` for the two Go files, and `git diff --check` all passed.
- Exact pushed source recheck: archived commit
  `214796fb8292a7a35ac7b03a4bd6f01657569620` with
  `git archive --format=tar 214796fb8292a7a35ac7b03a4bd6f01657569620` into a
  fresh Linux checkout, rebuilt `libmosdns_runtime.a` with
  `CARGO_TARGET_DIR=/root/mosdns-matcher-typednil-target-final.3VCNGm` and
  `CARGO_BUILD_JOBS=1`, and verified the same library SHA-256. The full tagged
  normal and `-race` commands above both passed against that checkout.
