# Rust-native local rule editing workflow

## Goal

Deliver one visible 5C workflow on the pure Rust-native host: in an isolated configuration, edit a configured file-backed `domain_set` through the maintained Vue `/` local-rule page, persist it, and observe the changed result in the next real DNS query. This is one enlarged PRD with independently verifiable behavior slices; it does not complete the whole WebUI or Phase 5C.

## Confirmed facts and start gates

- This is a child of `09-28-rust-next-step-roadmap` on branch `rust`. The parent plan, canary, and preceding `fast_mark`/`flow_setter` child have recorded C2C `FINAL: PASS`. Refresh task state and evidence before starting.
- Native `domain_set` currently accepts only load-time `exps` and `files` in a strict YAML subset; `sets` is rejected at config load. Its `.txt` file parser does **not** yet match Go rule acceptance: it truncates at inline `#` and aborts on an invalid rule, while Go skips only blank/whole-line-comment lines and skips invalid individual file rules. `HostAssembly` owns one UDP or TCP DNS listener and currently has no HTTP API listener.
- Go mounts `domain_set` `/show`, `/save`, `/post` under `/plugins/{tag}`. `ListManager.vue` GETs `/show?limit=10000`, POSTs `{ "values": [...] }`, and GETs `/api/v1/special-groups` for dynamic profiles.
- The maintained Vue UI is `/`; `/log` is the compatibility UI. Vite can proxy `/api` and `/plugins` to an isolated native endpoint. This task uses the real Vue page through Vite for browser proof; native static asset serving remains later 5C work.
- The user approved expanding this task to include the actual Vue local-list flow and asked for PRD/design/implement handoff to another agent. This planning turn does not start product-code implementation.

## Requirements

### R1. Managed profile and HTTP owner

- Support one or more independently tagged, single-writable-file native `domain_set` instances. Each managed profile has exactly one UTF-8 `.txt` rule file resolved relative to its declaring YAML file. Existing native query-only `exps` and multiple-`files` shapes remain supported, but management rejects ambiguous sources visibly rather than discarding rules silently. Per approved amendment A2, two tags that resolve to the same writable file still load and query, but both become management-ineligible with an explicit reason and no shared write ever happens. Native `sets` remains unsupported and is outside this task; do not add it as part of management.
- Align native `.txt` initial load and restart with Go: trim surrounding whitespace; skip empty lines and whole-line `#` comments; do not treat inline `#` as a comment; skip invalid individual file rules. POST normalizes each submitted value the same way the file loader does (approved amendment A1: trim outer whitespace, skip empty and whole-line `#` values, then matcher validation) rather than handing `values` to the matcher verbatim as Go's `MixMatcher.Add` does. For the Vue-shaped trimmed payload in this task, POST publication, persisted file and restart must produce the same effective accepted rule set. Keep `exps` invalid-rule errors strict. Update existing Rust tests that asserted the old `.txt` abort behavior, while retaining source-path diagnostics for file read and other real load errors. This is bounded text-rule compatibility, not full SRS/missing-file parity.
- Add one host-owned HTTP listener for this scoped management API. Its bind, failed startup cleanup, close, and port rebind are coordinated with DNS. Do not create a second runtime or matcher path.
- Expose only eligible configured tags at `/plugins/{tag}/show`, `/save`, `/post`. Preserve Go-visible method, status, body and content type; unknown and ineligible tags fail explicitly.

### R2. Safe persistence and query publication

- `/show` returns accepted live rules as UTF-8 plain text, one per line. GET `/save` persists the current generation. POST `/post` accepts the UI payload, preserves characterized per-rule acceptance, persists the complete candidate before publishing it, and reports the compatible replacement count.
- Per approved amendment A1, a POST value is normalized like a rule-file line instead of being accepted verbatim; per approved amendment A2, a writable file claimed by two tags is never managed. Both are explicit intentional compatibility deviations and are pinned by tests.
- Invalid JSON, ineligible configuration, candidate compilation error or persistence failure leaves old file bytes and matcher generation unchanged. Persistence failure includes the final replace/rename after a temporary file was created; clean that temporary file and do not publish. Use safe same-directory replacement with process-restart retention; this task does not claim crash/power-loss durability. Concurrent DNS queries observe a whole old or new immutable generation. Restart loads the committed generation. No file I/O under a DNS hot-path lock.

### R3. Maintained Vue page to actual DNS

- With an isolated fixture containing a configured fixed-profile tag such as `blocklist`, open the maintained Vue `/` page through Vite pointed only at the native HTTP port. Navigate Rules → Local rules, load the tag, edit and save it, verify the next real DNS query changes, and verify refresh and host restart retain the list.
- For this strict fixture, GET `/api/v1/special-groups` returns the true configured group list, empty when no groups exist. Do not add group mutations, pretend configured groups work, or return synthetic success from unsupported endpoints/tabs. Keep any Vue edit limited to this workflow and make unsupported or unconfigured lists visible without false success.
- Failed POST retains the unsaved draft and visible error. After POST 200, mark a tag clean only if its canonical `/show` reread succeeds; this accounts for rules skipped by server validation. If POST succeeds but that reread fails, keep the user's recoverable draft, mark that tag as saved-on-server but unconfirmed in the UI, and show a distinct reconciliation error. Do not present submitted text as canonical or include the tag in the confirmed-saved count. Process each dirty tag independently so one uncertain tag does not misreport another.

## Observable behavior slices

| Slice | Public interface and result | Controlled boundary |
| --- | --- | --- |
| 0: profile | Real file-backed YAML compile preserves source path and tag; ambiguous management is rejected | Temporary YAML/rule files; no compiler mock |
| 1: API/lifecycle | Real HTTP client to host-owned listener tests routes, body/status/content type, unknown tags/methods, empty group list, DNS+HTTP close/rebind | Loopback sockets; no router mock |
| 2: publication | Real POST A→B, file, `/show`, UDP/TCP DNS and restart agree; temp-write and final-replace failures preserve A and remove temp files; readers see whole A or B | Narrow persistence-step failure injection; controlled DNS peer only if needed |
| 3: Vue | Existing Vue `/` page via Vite proxy → real native API → file → real DNS query; POST failure keeps draft, while POST success followed by `/show` failure remains unconfirmed per tag | No mocked API or DNS response; isolated fixture and controlled network fault |

## Acceptance criteria

- [x] Parent/canary/5B prerequisites and revised-plan review pass; a later explicit user approval authorizes the executing agent to run `task.py start`. (2026-09-29: prerequisites verified as recorded committed PASS; the user approved the final summary for all four slices and authorized `task.py start`.)
- [x] Eligible profile, Go-visible `.txt` load/POST/restart semantics, strict `exps`, explicit `sets` rejection, HTTP contract and unsupported composite cases are source-grounded and tested before implementation.
- [x] Real HTTP and DNS tests prove two tags stay isolated, show/save/post, temp-write and final-replace failure atomicity and temp cleanup, whole-generation concurrency, restart, and DNS/HTTP shutdown plus rebind.
- [x] Reproducible browser proof on maintained Vue page covers edit/save, actual next-query effect, refresh/restart, failed-POST draft/error, and POST-200 followed by canonical-GET failure with an unconfirmed per-tag draft, using isolated native API and DNS.
- [x] Rust focused/full tests, fmt, clippy, a Vue build in a disposable source snapshot, and bounded Linux functional E2E run on `mosdns-rust` under the project VM rule, with exact commands/results and resource cleanup recorded. Generated Vue assets are not staged by this source/UI-through-Vite task.
- [x] Only proven feature-coverage subitems are updated. Exact committed task range obtains an explicit `FINAL: PASS` before completion is claimed. (2026-09-29 amendment: the user elected to perform this exact-range review personally, superseding the original same-conversation C2C reviewer requirement; completion is claimed only after the user's own explicit PASS, and no external C2C `FINAL: PASS` may be asserted.)

Closeout (2026-09-30): The user supplied the exact-range review result for
`390a6d97..3e1e2183` (`FINAL: PASS`) and requested a completion check and
conditional archive. The delivered proofs are the Slice 0-3 tests, browser
`54/54`, and isolated `mosdns-rust` validation at `fab8b682` (64 successful
Rust test binaries, functional `20/20`). No `rust/` or `webui-log/` source
changed between that validated revision and the reviewed HEAD. This closeout
records the user's supplied review result; it does not assert an external C2C
review binding or a production cutover.

## Out of scope

Native `domain_set sets` references; full `special_groups` mutations/routing; config package generation/update; other providers and downloads; other Vue tabs; `/log`; bundled UI serving from Rust; generated embedded Vue assets; full C04/C05/C10/C11/C17 or 5C; complete production config; SRS/geodata and missing-file compatibility; crash/power-loss durability; cache dump; full metrics/audit API; capacity/soak; dedicated remote fault/cancel/close E2E; 5D; Phase 6; production replacement. Do not write live `/cus/mosdns` state.

## Approved product-decision amendments (2026-09-29)

Both amendments below were put to the user after the first exact-range review flagged that they
changed frozen PRD behavior without a product decision. The user approved them on 2026-09-29.

### A1 (approved): POST value normalization is an intentional Rust-native deviation

A POST value is normalized exactly like a rule-file line: outer whitespace is trimmed, an empty or
whole-line `#` value is skipped, and the remaining candidate is validated by the same matcher.
Go instead passes every `p.Values` entry to `MixMatcher.Add` verbatim. This differs deliberately:

- It keeps the R1 invariant that initial load, Vue-shaped POST, persisted file and restart produce
  the same effective rule set even for untrimmed input.
- An empty value would otherwise become a root suffix rule that matches every domain
  (`DomainSuffixMatcher::add("")` sets the root value).

This must not be described as replicating Go's POST verbatim behavior. Pinned by
`slice7_management_publication.rs::an_approved_post_normalizes_values_across_http_file_show_and_restart`
(direct HTTP response, persisted file, `/show`, and a restart host) and
`save_persists_the_current_generation_and_a_post_skips_blank_and_invalid_values`.

### A2 (approved as revised): a shared writable file loads normally but is never managed

A single-`.txt` tag whose file is also resolved by **any other** configured `domain_set` tag --
another managed candidate or a query-only composite -- no longer fails configuration load and is
never managed. The conflicting tags keep loading, matching and answering DNS queries as query-only
shapes, and the candidate becomes management-ineligible: `/show`, `/save` and `/post` reject it
explicitly with a reason naming the other owner(s), and no shared write happens. This was extended
in the second review round: the original guard only compared two management candidates, which let a
legal query-only composite tag's persistent source be rewritten by another tag's POST. This replaces the earlier load-time rejection (which was
itself an unapproved deviation from Go, where the two tags would silently overwrite each other).
Pinned by
`slice5_management_config.rs::two_tags_sharing_one_rule_file_still_load_and_are_management_ineligible`
(load + query + reason) and
`slice6_management_http.rs::a_writable_file_shared_by_two_tags_is_never_managed`
(all three routes reject both tags, file unchanged).

## Blocking questions

None for this bounded scope. The executing agent resolves source-level technical details in Slice 0; a required product behavior change returns to planning review.
