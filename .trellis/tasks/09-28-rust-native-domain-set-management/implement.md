# Rust-native local rule editing workflow — implementation plan

One enlarged 5C task; four independently verifiable behavior slices and one complete final review. The executing agent owns product-code implementation. This planning turn must not run `task.py start`.

## Start gate and handoff

- [x] Refresh `task.py current/list`, parent roadmap and archived canary/5B evidence. Confirm their C2C `FINAL: PASS` results and preserve unrelated dirty paths. 2026-09-29: parent roadmap (`implement.md:42/57`, `prd.md:110-111`), canary (via `09-28-rust-mosdns-rust-canary-review-restart/implement.md:11,21`) and `fast_mark`/`flow_setter` (`implement.md:95-99`) PASS verified committed; the enlarged 2026-09-29 parent/child revisions postdate every recorded PASS and are closed by this task's explicit approval.
- [x] Read `AGENTS.md`, `docs/ai/project-context.md`, `config-notes.md`, `rust-handover.md`, `rust-rewrite-plan.md`, `.trellis/workflow.md`; run `trellis-before-dev` for backend before product-code edits. 2026-09-29: all six docs read; `trellis-before-dev` runs immediately after `task.py start`, before any product-code edit.
- [x] Have this revised PRD/design/implement plan reviewed under active Trellis/C2C workflow. A later explicit user approval of the latest summary authorizes `task.py start`; this request to prepare documents does not. 2026-09-29: two prior review rounds' findings were accepted and corrected (planning-evidence.md:19,21); the user then explicitly approved the final summary and authorized `task.py start` for all four slices, and **elected to perform the exact-range review personally instead of binding an external C2C conversation**. Claim completion only after the user's own explicit PASS; do not assert or fabricate an external `FINAL: PASS`.
- [x] Source anchors re-verified read-only on 2026-09-29; five corrections recorded in `research/planning-evidence.md` (`domain_set.go:306-393`, Vite proxy `46-52` with non-loopback default, `/show` query-string/`limit` behavior, Go group POST/DELETE intentionally not ported, `fast_mark` citation `:95-99`).
- [ ] For the planning commit and exact-range review, stage only this child task's `prd.md`, `design.md`, `implement.md`, `task.json`, `implement.jsonl`, `check.jsonl`, and `research/planning-evidence.md` unless an explicitly reviewed plan range lists additional paths. Do not stage the separate canary active→archive move, replacement archive edits, `.trellis/workflow.md`, backend quality spec, automation test, or the entire `.trellis/tasks` tree. Parent roadmap/stage-plan edits need their own inspected scope if submitted.
- [ ] Keep auto-commit disabled. Build, Cargo tests, integration and E2E run through only the `mosdns-rust` SSH alias per `docs/rust/next-stage-plan.md`, using isolated ports/files and source snapshot. Do not touch live service or `/cus/mosdns`. Record exact candidate SHA.

## Slice 0 — freeze contract and eligible config

- [ ] Record concise research with source line anchors: Go domain_set show/save/post and text-file rule semantics; native `.txt` inline-`#` truncation and invalid-rule abort; native `sets` rejection; API mount; Vue save/draft behavior; Vite proxy and build output paths.
- [ ] Freeze strict native HTTP listen config and managed profile: exactly one writable `.txt` file per tag. Add failing public config tests for two eligible tags, relative included-file paths, ineligible composite management, duplicate tags, missing/invalid file and unsupported API shape. Assert `sets` stays a load-time unsupported error; do not implement it. Preserve supported `exps`/multiple-`files` query-only shapes.
- [ ] Add behavior tests that distinguish Go's `.txt` handling from current Rust: whole-line versus inline `#`, one invalid file rule followed by a valid rule, strict invalid `exps`, and equivalent effective rules for initial load, Vue-shaped POST and restart. Update the old `slice3_composition` test that expected an invalid `.txt` rule to abort loading; retain a separate negative file-read/source-path test.
- [ ] Implement only typed config and minimal provider registry needed to make these pass.

## Slice 1 — scoped HTTP API and host lifecycle

- [ ] First add failing real HTTP tests for `/plugins/{tag}/show`, `/save`, `/post`, unknown tag, wrong method, ineligible tag and GET `/api/v1/special-groups` with empty actual fixture state. Include the UI-shaped `/show?limit=10000` request: it must return 200 with the full accepted rule list, matching Go's ignore-the-query-string behavior.
- [ ] Implement one host-owned HTTP listener and route owner coordinated with DNS. The top-level run/supervisor creates a single shutdown scope, binds both listeners, drives both on the existing runtime, and on either side's startup/run failure cancels and joins/drains both before return. Review the existing per-DNS-run cancellation and upstream-catalog close so no detached HTTP task, orphan socket or double-close remains. Prove both bind-order failures, running-side failure, normal close and rebind.
- [ ] Verify status, body, content type and request shape against Slice 0 source contract; keep current DNS-only config valid.

## Slice 2 — safe persistence and DNS publication

- [ ] First add failing HTTP→file→DNS tests: POST A→B changes next UDP query branch and a TCP variant; two tags remain isolated. Use cache-free fixtures for the publication oracle.
- [ ] Test empty and invalid individual file rules per frozen Go behavior, malformed JSON, injected temp-write failure, and injected final replace/rename failure after temp creation. In both persistence failures verify byte-for-byte old file, `/show` and DNS result remain A, generation is not published, and temporary files are removed. Test GET save, concurrent readers seeing whole generations, and fresh-host restart loading the same effective B rules produced by the Vue-shaped POST. Do not claim power-loss durability.
- [ ] Implement one complete candidate compile, safe same-directory file replace and one immutable-generation publication; keep matcher and sequence real. Cover the final commit step, not only failures before rename.

## Slice 3 — maintained Vue page to real DNS

- [ ] Start isolated native process and Vite with explicit `MOSDNS_DEV_TARGET` set to its loopback HTTP port. Verify proxy destination. Open `/`, navigate Rules → Local rules, select configured fixed tag, load/edit/save; check HTTP trace, file and next actual DNS query. Refresh and restart preserve accepted rules.
- [ ] Force native POST failure; verify visible error and unsaved draft. Then let POST 200 commit while a controlled transport fault prevents the following canonical `/show`: verify that tag remains recoverable and explicitly unconfirmed, the submitted text is not treated as canonical, it is absent from confirmed-saved count, and another dirty tag can still reconcile independently. A later save/load retries GET before another POST and preserves local edits if canonical content differs; a successful reread clears uncertainty only after reconciliation. Verify a skipped invalid rule is not shown as saved until canonical `/show` reload. Restrict any Vue edit to this workflow and preserve other UI behavior.
- [ ] Save reproducible browser evidence: exact config, ports, steps, request/response, DNS oracle and cleanup. Screenshots may support, but not replace, API/file/DNS assertions.

## Final validation and review

- [ ] Run focused native-host tests after each slice. On `mosdns-rust`, run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, Vue `npm run build` in a disposable exact-source snapshot, and bounded real-process HTTP+DNS/browser functional proof. Vite build writes `coremain/www/assets/vue-log` and stamps `coremain/www/log.html`; do not stage those generated files in this task. Verify no build artifact diff in the main worktree. No Go compile is required here.
- [ ] Record environment, revision, config, commands, actual results, failures/corrections and verified API/DNS/Vite process/socket cleanup. No performance or production claim.
- [ ] Update only proven `docs/rust/feature-coverage.md` subitems: bounded P02 management, C10 plugin API and C11 local-rule UI flow. Full C05 groups and broad rows remain pending. Update parent roadmap and stale stage-plan statements only with verified facts.
- [ ] `git diff --check`, inspect exact changes, stage exact task paths and commit coherent result without auto-commit or unrelated dirty files. Submit task ID, exact committed base..HEAD and path list to the user for the exact-range review (the user performs this review personally per the 2026-09-29 decision). Correct and re-review until the user returns an explicit PASS; do not claim completion, and do not assert an external C2C `FINAL: PASS`.
- [ ] Report delivered workflow and deferred 5B/5C/5D/Phase 6 gates. Do not deploy or replace production.
