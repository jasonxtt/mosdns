# Rust-native local rule editing workflow — design

## Architecture and ownership

Extend the existing `mosdns-native-host` assembly. One host runtime owns the existing DNS listener, a new scoped HTTP listener, configured `domain_set` owners and shutdown/cancellation. Each managed provider publishes one immutable generation containing accepted rule text and compiled matcher. `qname $tag` must read the provider generation at the query boundary; a startup-only `Rc<MixMatcher>` reference cannot continue to serve queries after POST.

Use a minimal registry keyed by configured tag. Do not add a general plugin framework, Go ABI, selector, fallback or second DNS matching path. Likely touch points: `rust/native-host/src/config.rs`, `matchers.rs`, `assembly.rs`, `execution.rs`, UDP/TCP owners, a focused management/HTTP module and integration tests. Inspect current code before editing. Keep frontend changes within `webui-log/src/components/ListManager.vue` and its existing request helper unless a real failure shows another file is needed.

Spec note on placement: `.trellis/spec/backend/directory-structure.md:21` keeps HTTP endpoints in `coremain/` and forbids making Rust *crates* aware of Vue or HTTP handlers. That rule governs the Go-era hybrid layout; `docs/ai/rust-rewrite-plan.md` assigns the native HTTP API to the Rust-native host at 5C, and this task's listener lives in the `rust/native-host` binary (the designated future process owner), not in a reusable data-plane crate. No library crate gains an HTTP or UI dependency.

## Source contract to freeze in Slice 0

- `plugin/data_provider/domain_set/domain_set.go`: first configured file is the Go write target; `/show` emits accepted rules with newline and `text/plain; charset=utf-8` and **ignores the query string** (the UI sends `?limit=10000`; Go has no `limit` handling, so the native route must accept and ignore it rather than reject unknown parameters); GET `/save` gives empty 200 on success and 500 for no file/write error; POST `/post` accepts `{values}`, returns 400 `invalid JSON` for malformed JSON, requires `.txt`, skips individual `MixMatcher.Add` failures, persists accepted rules, then publishes and returns `domain_set replaced with N entries`.
- `coremain/mosdns.go` mounts plugin handlers under `/plugins/{tag}`. `coremain/api_special_groups.go` GET returns a sorted JSON array. This strict fixture has no configured special groups, so `[]` is the actual state, not a replacement for group management. Go also registers POST and DELETE group routes; the native side intentionally adds only the GET list (a scoped deferral, never a claim that configured groups work).
- `webui-log/src/App.vue` mounts `ListManager` on Rules → Local rules. `ListManager.vue` fetches group profiles, GETs `/show?limit=10000`, and POSTs trimmed nonempty lines. It currently marks a successful POST draft saved from submitted lines; reconcile this with server-side invalid-rule skipping by reloading canonical `/show` after POST.
- `webui-log/vite.config.js` proxies `/api`, `/plugins` and `/metrics` through `MOSDNS_DEV_TARGET`. Set this explicitly to an isolated native loopback HTTP port, verify the destination, and never use Vite's default external target — the default is a non-loopback LAN host, so an unset variable would send requests to another machine. Rust static UI serving is deferred.
- `rust/native-host/src/config.rs::compile_domain_set` accepts only `exps` and `files`; `sets` fails `reject_unknown`. `rust/native-host/src/matchers.rs::build_domain_set` currently splits every `.txt` line at `#` and fails the entire load on one invalid rule. Go `loadFileInternalWithRules` trims a line, skips only blank/whole-line `#` comments, and skips invalid individual file rules. Existing Rust `slice3_composition` negative tests assume the old failure; change their expectations when aligning text-file semantics.

Before coding, record precise Go rule grammar, blank/comment behavior, response content types and unknown-method behavior in task research with line anchors. The compatibility choice is fixed: native `.txt` initial load and restart trim outer whitespace, skip empty and whole-line `#` comments, keep inline `#` in the candidate rule rather than truncating it, and skip an invalid individual file rule. POST follows Go's per-value matcher validation and skip behavior; for Vue's trimmed nonempty values, the published, saved and reloaded effective rule sets must agree. `exps` remain strict errors, matching Go `LoadExps`. Apply the `.txt` rule handling to the shared native text-file loader so existing supported query-only `exps`/multiple-`files` shapes keep working with corrected text semantics; update affected regression tests. `sets` remains rejected by the native YAML compiler and is not implemented here. For managed Rust configuration, require exactly one `.txt` file and no separate `exps`/other files. Reject management of supported composite query-only shapes clearly. Do not silently drop a source on POST. Per approved amendment A2 (extended by the round-2 review finding), a single-`.txt` candidate is manageable only when no other configured `domain_set` tag -- managed candidate or query-only composite -- resolves to the same file. Conflicting tags still load and query normally, but become management-ineligible with an explicit reason naming the other owners, so a POST can never rewrite another tag's persistent source behind its live matcher; no shared write ever happens (the earlier load-time rejection was itself an unapproved deviation and was replaced).

## Data flow

```text
Vue or HTTP POST
 → real native /plugins/{tag}/post
 → decode + accept/skip individual rules per frozen contract
 → compile complete candidate {rules, matcher}
 → safe same-directory replacement of selected .txt file
 → one generation publication
 → subsequent DNS query sees new generation
```

Serialize writes per provider with one provider-scoped update lock held across compile, persist and publish. The candidate compile and the file write run on the blocking pool, never on the single-threaded DNS runtime, so a DNS query is still answered while an update is in flight; the generation is exchanged in one step only after the write succeeded. Independent tags do not overwrite each other. GET `/show` and `/save` use one snapshot. DNS readers hold a generation handle for one query or matcher evaluation, so in-flight requests cannot observe a partial replacement. File I/O stays outside hot-path locks. Test both a temporary-write failure and a final replace/rename failure after temp creation: old disk bytes, `/show`, and published matcher remain unchanged, and the temp file is cleaned. Restart recompiles the committed file and must retain the effective rules accepted from this task's Vue-shaped POST. This proves process-restart retention and atomic publication; it does not claim survival of a power failure. Inject only the narrow persistence step needed for these failures; keep parser, matcher, router, HTTP stack and DNS path real.

Use a cache-free sequence or fresh names for the publication oracle. Do not invent cache invalidation policy to make this test pass.

## HTTP and browser boundary

The native YAML needs an explicit, strictly validated management listen address. Prefer the existing Go `api` configuration shape if its scoped fields can be supported without implying that other API fields work; document accepted subset and errors. The native host currently accepts only `log`/`include`/`plugins` at YAML root and has no API key or HTTP code at all, so the scoped key is new and must be documented as this management listener only. Bind loopback in the isolated fixture. The top-level host run/supervisor creates one shared shutdown scope, binds DNS and HTTP before serving, and drives both under the existing runtime. Any bind, startup, or running-side failure cancels the other side, joins/drains both listener and request tasks, releases both sockets, and only then returns. Existing `run_udp`/`run_tcp` each create their own `TransportCancellation` and DNS `serve` drains the upstream catalog; refactor ownership so the new HTTP task is not merely detached alongside a self-owned DNS run, and ensure shared upstream close occurs exactly once after relevant DNS work drains.

GET `/api/v1/special-groups` returns `[]` only when the strict loaded config has no groups. Add no POST/DELETE group routes. A tag with no mounted plugin route is 404 before any method decision (as Go's per-tag mounts behave), a mounted tag with the wrong method is 405, and an ineligible management shape is 400 with an explicit reason.

The browser proof uses the maintained Vue app served by Vite and proxies only to the native test process. Test one configured fixed tag, e.g. `blocklist`; API/DNS tests also cover a second configured domain set. Treat each dirty profile tag independently in `saveList()`: POST 200 means server mutation succeeded, but only a successful canonical `/show` reread confirms the saved text and clears that tag's dirty state. If the reread fails, preserve the entered draft and old clean baseline, mark that tag as needing reconciliation, show an explicit “server save succeeded, current contents unconfirmed” error, and omit it from the confirmed-saved count; other tags continue independently. A later save/load first retries `/show` for an uncertain tag rather than silently repeating POST; preserve local edits if canonical contents differ and let the user decide whether to submit them again. A failed POST also retains draft and error. Do not claim other tabs or native asset serving.

`npm run build` is not a read-only check here: Vite writes `coremain/www/assets/vue-log` and stamps `coremain/www/log.html`. This task validates the Vue source via Vite and builds in a disposable, exact-source snapshot on `mosdns-rust`; it does not stage or commit generated embedded assets. Verify the main worktree has no build-generated diff. The later native static-serving task owns bundled asset integration.

## TDD seams and mock boundaries

| Behavior | Public surface | Allowed controlled boundary |
| --- | --- | --- |
| Eligibility/source path and rule semantics | `load_and_compile` with real included YAML and rule file, including inline `#`, invalid file rule, strict `exps`, rejected `sets`, and a writable file shared by two tags | Temporary filesystem |
| API/tag isolation | Loopback HTTP requests to host-owned listener | Temporary files; no handler/router mock |
| Publication | Real UDP and TCP DNS requests after real HTTP POST | Controlled loopback upstream only if route needs one |
| Failure/concurrency | HTTP POST plus concurrent DNS reads while an update is parked inside persistence and again immediately before publication, including final replace failure after temp creation | Narrow persistence-step failure injector, a bounded persist gate and a publish gate |
| Browser workflow | Existing Vue app via Vite → native API → actual DNS query; per-tag POST-200/GET-failure state | Isolated config/loopback and controlled GET transport failure; no mocked successful API/DNS response |
| Lifecycle | One top-level supervisor, one shutdown scope, failure-driven cancel/join and bind/rebind both listeners | Real loopback sockets |

## Risks and deferred work

The major seams are replacing startup-held matcher references with generation-aware query reads while preserving the current `Rc`/LocalSet model, and moving per-DNS-run cancellation/drain into one host supervisor without double-close or orphan HTTP tasks. Keep changes local to provider/host. Browser evidence must verify the exact proxy destination and that no generated Vue asset diff entered the task. If the scoped page requires broad `special_groups` or other-tab compatibility, stop and return the product-scope change to planning rather than silently expanding the API.

This remains one child task and one final review range, with slice checkpoints. It does not complete all provider APIs, Vue workflows, static serving, 5C or production gates.
