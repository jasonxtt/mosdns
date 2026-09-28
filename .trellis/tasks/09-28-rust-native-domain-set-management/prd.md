# Rust-native domain_set management save and query closure

## Goal

Deliver the first bounded 5C management loop for one file-backed native
`domain_set`: show its live rules, save the current rules, post a replacement,
and prove that persistence and the next DNS query observe the complete new
generation.

This is a new child of `09-28-rust-next-step-roadmap`. The parent plan must
receive same-chat C2C `FINAL: PASS`, and the user's canary execute/defer decision
must be recorded. The planned default is to run this after the 5B
`fast_mark`/`flow_setter` child receives its same-chat review PASS. There is no
architectural dependency between these children; if 5B is explicitly deferred
or blocked, obtain the user's explicit decision to reorder before starting this
child.

## Requirements

- Keep existing native `domain_set` load-time query behavior.
- Mount plugin management at `/plugins/{tag}` and implement `GET /show`,
  `GET /save`, and `POST /post` for the bounded file-backed text-rule profile.
- `GET /show` returns the live rules as UTF-8 plain text, one rule per line.
  `GET /save` persists the currently published rules and returns the compatible
  success/error status. `POST /post` accepts the UI payload
  `{ "values": ["..."] }`, follows the existing provider's per-rule
  validation behavior, writes the selected `.txt` file, and publishes the new
  matcher generation only after successful persistence.
- Characterize and explicitly support the management-eligible config profile
  before coding. The first slice must be file-backed with one unambiguous
  writable text target; combinations whose Go persistence semantics would
  overwrite or lose separate `exps`/`files`/`sets` sources must either preserve
  their observed behavior or fail visibly as unsupported. Do not silently
  mutate only part of a composite ruleset.
- Malformed JSON, invalid/unsupported management configuration, or persistence
  failure must leave the live rules and matcher generation unchanged.
- Concurrent DNS queries observe an entire old or new immutable generation,
  never a partial update. A restart loads the last successfully persisted
  rules. Closing the native host releases both API and DNS listeners.
- Match externally visible status, body, content type, path, and UI request
  shape from `plugin/data_provider/domain_set/domain_set.go`,
  `coremain/mosdns.go`, and `webui-log/src/components/ListManager.vue`. Preserve
  product behavior, not unsafe Go file-write internals.
- This task is a native-host/API foundation only; it does not migrate the Vue
  editor or the full 5C API surface.

## Observable behavior slices

1. **HTTP contract:** real native router accepts GET show/save and POST post at
   `/plugins/{tag}`. Show returns the exact live rules text. Save persists the
   current generation. Post follows the UI JSON shape and compatible status/body.
2. **Atomic update:** valid post writes a rule file and the next query through
   the real DNS path changes from rule set A to rule set B. Malformed JSON and
   forced storage error return errors while the old file/generation/query
   behavior remains intact.
3. **Concurrent readers:** hold queries across an update and prove every result
   belongs wholly to the old or new ruleset, with no mixed generation.
4. **Restart and shutdown:** a new host instance loads the successfully written
   rule file; shutdown releases API and DNS listeners so they can be rebound.

## Out of scope

- SRS/geodata, multiple-file and arbitrary composite `exps`/`files`/`sets`
  editing unless required to preserve the frozen eligible profile.
- Downloads, deletion, general provider API, `RuleExporter` subscriptions,
  Vue/UI changes, all other plugin handlers, full configuration manager,
  persistent cache formats, Prometheus, or complete 5C.
- Broad audit HTTP/API, upstream/special-groups controls, package presets,
  performance, capacity, multi-core, or production cutover.

## Acceptance Criteria

- [ ] Parent roadmap has same-chat C2C `FINAL: PASS`, and the canary
      execute/defer choice is recorded.
- [ ] By default, prior 5B child has same-chat C2C `FINAL: PASS`. If 5B is
      explicitly deferred or blocked, the user explicitly approves reordering
      5C before this child starts.
- [ ] The file-backed management eligibility and any unsupported composite
      config forms are frozen with source evidence before implementation.
- [ ] Real native HTTP tests prove `/show`, `/save`, and `/post` status/body,
      payload, persistence, and next-query behavior.
- [ ] Invalid JSON and injected persistence failure do not change the previous
      disk contents or live matcher generation; per-rule behavior matches the
      characterized provider contract.
- [ ] Concurrent DNS readers see whole generations; restart sees the committed
      generation; shutdown releases API and DNS sockets and permits rebind.
- [ ] Focused native-host tests, existing 5B regressions, `cargo fmt --all --
      --check`, workspace clippy with warnings denied, and full Rust workspace
      tests pass. Any selected Linux functional E2E is correctness-only; no
      performance PASS is claimed.
- [ ] Update only delivered domain_set management subitems and attach exact
      evidence. Do not claim complete P02, C04, C10, C11, C17, 5C, or package
      compatibility.
- [ ] The same C2C conversation reviews the exact committed task range and
      returns `FINAL: PASS`; resolve and re-review all findings there.
