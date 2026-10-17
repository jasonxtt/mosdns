# Native WebUI hosting and runtime capabilities

This contract belongs to the opt-in pure Rust `mosdns-native-host`. It does not
select the production/default binary or close the full native migration gate.

## Same-origin hosting

`api.http` serves the maintained Vue UI at `/` and its recursively embedded
`/assets/...` from the executable. **The native host serves no `/log`**: the
compatibility UI (`/log`, `webui-log/src-log1/`, `assets/vue-log1`) belongs to the Go
host only; `/log`, `/log/`, `/log1` and `/assets/vue-log1/...` are plain 404s here
and the native build neither builds nor embeds that UI (`scripts/build-rust-native.sh`
runs `npm run build` only). No Go, Node, frontend proxy, source
checkout or external web server is required at runtime. Cargo validates required
roots and their referenced assets, regular files, forbidden secret/map names and
symlinks, and tracks source assets for rebuilds. Roots use `no-cache`, accurate
MIME, `nosniff`, representation lengths and SHA-256 strong ETags. GET/HEAD support
weak/list/star If-None-Match; HEAD and 304 have no body. Other methods return 405
with Allow; missing assets return 404, without SPA fallback or directory listings.

External UI is registered once from immediate real directories under the
**top-level configuration directory** `ui/<name>`. A basename `-c config.yaml`
uses that file's startup directory. In-memory compilation requires an explicit
nonempty base; no base means no external mounts. Includes, process directory
changes and hot managed generations do not remount the registry. Reserved names:
`root`, `log`, `log1`, `legacy`, `blog`, `assets`, `debug`, `metrics`, `plugins`, `api`.
A safe mount `/name` redirects to `/name/`; directory requests require index.html.
Files are live rereads with `no-store`, no listing and a 16 MiB maximum (413 above).

Linux uses pinned directory descriptors and handle-relative `openat2` beneath
and no-symlink resolution. Path decoding occurs once and rejects malformed
encoding, encoded separators, backslashes, control/NUL bytes and dot traversal.
Symlink/device/FIFO/socket/nonregular/escape targets return no file contents.
Root replacement does not change the pinned mount. Unsupported platforms or
kernels fail closed for external UI; embedded UI and API continue. Startup
failures produce bounded diagnostics, without filenames/content disclosure.

**External UI packages (UI-1).** The built-in UI at `/` is the preset; a newer UI can be shipped
without rebuilding the program by unpacking an external package into **any** directory
`<config root>/ui/<name>/`, which the mount serves at `/<name>/` (`ui/pg` → `/pg/`, `ui/a` → `/a/`;
`main` is only an example; the built-in `/` stays available as the fallback). Directory names follow
the rules above: non-empty, no slash/backslash/control characters, a real directory (no symlink), not
a reserved name (`root`, `log`, `log1`, `legacy`, `blog`, `assets`, `debug`, `metrics`, `plugins`,
`api`). `npm run build:external` in `webui-log/` builds the same `src/` into
`webui-log/dist-external/package/` with **relative** resource URLs (`base: './'`: the package does
not depend on its directory name, so the same package works under any `ui/<name>/`; API calls are
same-origin absolute paths) and writes `release/mosdns-ui-<version>.zip` whose top level is the
package. A **mosdns UI package** is a directory that carries a `ui-manifest.json`:

```json
{"manifest_schema": 1, "ui_version": "v0.7.3-12-gabcdef", "built_at": "2026-10-17T10:00:00Z", "api_level": 1, "requires_api_level": 1}
```

`manifest_schema` must be 1; `ui_version` is required (non-empty, at most 64 characters, no
control characters); `built_at` is optional; `api_level` is the interface level the UI was built for
and `requires_api_level` (default `api_level`) the least level it needs from the host. The host
provides `HOST_API_LEVEL = 1` and still serves UIs built for `MIN_UI_API_LEVEL = 1` or later
(`rust/native-host/src/ui_manifest.rs`; backward-compatible additions do not change the level, the UI
degrades by `/api/v1/capabilities`). **Compatible ⇔ `requires_api_level ≤ HOST_API_LEVEL` and
`api_level ≥ MIN_UI_API_LEVEL`.** The manifest check applies **only to directories that have a
manifest**. At startup the registry looks in each pinned `ui/<name>` directory for
`ui-manifest.json` (beneath-only, no symlinks, a regular file of at most 64 KiB):
- **no manifest** → not a mosdns UI package: the directory is mounted as the generic static mount it
  always was (existing behaviour; other static pages an operator put there keep working), and health
  notes "非 mosdns UI 包（无 manifest）" (`kind: "static"`);
- **manifest, compatible, `index.html` a regular file** → mounted (`kind: "mosdns_ui"`, with its version);
- **manifest present but unreadable / not JSON / wrong schema / invalid version / incompatible level /
  `index.html` missing** → **not mounted** (`/<name>/` is 404) with a fixed, content-free reason.
Reserved names and non-directories (a symlink, a file with the name of a directory entry) are listed as
`kind: "unusable"` and not mounted; plain files directly under `ui/` are ignored. The result is
decided at startup, like every mount; file contents are re-read live, so replacing files of the same
package takes effect at once, but a new package's manifest is judged (and the version shown) only at
the next start — restart once after an update. Usage: [docs/rust/external-ui.md](../external-ui.md).

Both static services share four active representation owners; saturation is
503. Non-representation 301/404/405 retain their status. Headers have a five-second
read budget; response transfers have ten seconds and chunks at most 64 KiB.
External metadata/open/read work runs outside the LocalSet, through one bounded
channel per owner. Cancellation closes the receiver and joins started blocking
workers before ownership/permits are released. Shutdown drains connections and
workers; a started stalled filesystem operation can delay that join. Static
requests do not acquire query/mutation leases or create DNS audit/metric events.

## Version and readiness

`mosdns version` prints only the product version and newline, exits zero before
config/runtime construction, and rejects extra arguments. Bare Cargo defaults
to `dev`; explicit `MOSDNS_BUILD_VERSION` is nonempty and cannot contain CR/LF.
Health GET `/api/v1/system/health` shares this exact version. JSON fields are
`ready`, `runtime:"rust"`, `version`, `state`, `required_config_schema:null`,
`applied_config_schema:null`, `config_management_enabled:false`, and
`pending_cleanup` (array of paths, relative to the configuration root, of deleted
special-group files that could not be removed yet; empty normally, never affects
`ready`; see native-special-groups.md), `external_ui`
(an array, sorted by name, at most 64 entries, one per directory under `ui/`:
`{"name", "mounted", "kind": "mosdns_ui"|"static"|"unusable", "ui_version", "built_at", "reason"}`;
`reason` explains a refusal, or notes that a manifest-less directory is not a mosdns UI package; the
built-in UI shows a link to `/<name>/` for **every** mounted compatible mosdns UI package (not the one
being viewed) and the reason of each refused one; never affects `ready`) and `udp_receive_buffers` (one
`{listen, bytes}` per live UDP listener: the kernel receive buffer it was
granted, `null` when unreadable; see "UDP receive buffer and apply pause"
below; never affects `ready`). Null schemas do
not mean YAML schema zero or disable native managed group/upstream operations.
State values are `starting`, `ready`, `applying`, `recovery_required`, `stopping`,
and `closed`. Health is read-only and crosses the admission rejection barrier without a lease.
Only all-listener-ready lifecycle returns 200/ready:true; startup, applying,
recovery-required, stopping and closed return 503/ready:false. Other methods405.

## UDP receive buffer and apply pause

Saving settings (`native-settings.md`) or a special group (`native-special-groups.md`)
compiles the candidate configuration on the single DNS thread, so DNS answers
pause for about 0.4–0.65 s (measured with the production rule set; the value
grows with the rule set). UDP queries that arrive meanwhile wait in the kernel
receive buffer of the listener and are answered when the thread resumes; they
are not dropped as long as the buffer holds them. Every UDP DNS listener
therefore asks for an effective receive buffer of 4 MiB (about 3 400 small
packets, ten times the 500 qps × 0.65 s burst): first `SO_RCVBUF` (half of that,
since Linux stores double), then, if `net.core.rmem_max` capped it, `SO_RCVBUFFORCE`
(needs `CAP_NET_ADMIN`). Failure to reach the target never stops the host: the
value the system granted is kept, a warning names it, and `udp_receive_buffers`
in the health document shows it. On a host whose `rmem_max` is below 2 MiB and
which cannot force the option, a longer burst can still overflow the buffer;
raise `net.core.rmem_max`. TCP listeners are unaffected (connection-level flow
control). Removing the pause itself (compiling off the DNS thread) is the M2-3
backlog item.

## System and appearance operations (F-1)

Four operations that were reported unsupported are real now; each is gated by its
`ui_operations` entry and answers exactly the routes the maintained WebUI calls. Only the
maintained shell at `/` is in scope; the compatibility shell at `/log` is untouched and
scheduled for removal.

**Appearance (`appearance.server`)** — `/api/v1/appearance/…` with the reference host's methods,
status codes, bodies and state files under `webinfo/` (`appearance_settings.json`,
`appearance_text_settings.json`, `appearance_button_settings.json`, `panel_background.bin`,
`panel_background_history/<id>.<ext>`), so either host reads what the other wrote (verified in
both directions with `tools/f1-interop`). Errors are `{"error": "..."}`. A save is a same-directory
temporary file plus rename: a failure leaves the previous file. Uploads are multipart field `file`,
at most 20 MiB (the request body may be 20 MiB + 1 KiB, the only route allowed more than 1 MiB),
and the content must sniff as an image (ico/cursor, bmp, gif, webp, png, jpeg — the reference's
detector; svg is not accepted). Only **one upload is read at a time**: the slot is claimed when
the request head has been read, before the body, and the body must arrive within 120 s; a second
concurrent upload is answered 503 at once. This bounds the memory a burst of 20 MiB bodies could
take on the unauthenticated management listener (the body buffer becomes the request body without
a second copy). The history has no entry limit (as in the reference); each upload
is bounded. A corrupt state file reads as the defaults and is never rewritten by a read. Not
reproduced: range and conditional requests for the image routes, and the reference's migration of
state files from older locations (`state/`, the root) — only `webinfo/` is read.

**WebUI port (`system.webui_port`)** — `GET/POST /api/v1/system/webui-port`. The port is saved in
`webinfo/webui_port_settings.json` (`{"port": N}`) and **replaces the `api.http` port at the next
start** (the saved port wins over YAML, as in the reference). Saving does not switch the running
listener; `pending_restart` says so. `POST` rejects ports outside 1–65535 (400), a port that cannot
be bound (409) and answers `target_addr`, `message`. The active address is the one the management
listener actually bound.

**Restart (`system.restart`)** — `POST /api/v1/system/restart {"delay_ms": N}` (`N<=0` → 300; at
most 10 minutes) answers `{"status":"scheduled","delay_ms":N}` at once. The restart is **in place**:
after the delay the host stops exactly as it does for `SIGTERM` (admission stops, in-flight queries
finish or are cancelled, caches/domain lists/rule sources/audit settings are saved, listeners and
upstream connections are closed), then the process image is replaced with `execve` of the same
executable and arguments (the path recorded at start, so a binary upgraded in place is the one
that runs). The **process id does not change**, so the service manager never sees the service stop
— necessary because the production unit waits two minutes before restarting a stopped service.
The request is refused up front (500 `{"error": …}`) when the executable is missing or not
executable, and with 501 when the process did not record one. A final save that fails is logged and the
restart goes ahead (the cache dump is a convenience; an exit would cost the restart delay). If the
`execve` itself fails, the host starts again inside the same process and keeps serving. A
termination signal during a pending restart wins. DNS is unavailable from the close of the listeners
until the new process binds them (drain + save + `execve` + cold start with the loaded cache dump):
unlike the 0.4–0.65 s pause of a configuration save (no listener is closed then), queries sent
in between are lost. The measured gap is in the F-1 validation record.

**Process metrics (`metrics.process`)** — the process section of `GET /metrics` (Prometheus text,
read from `/proc/self`): `process_cpu_seconds_total`, `process_start_time_seconds`,
`process_resident_memory_bytes`, `process_virtual_memory_bytes`, `process_open_fds`,
`process_max_fds`, and the native gauge `mosdns_process_threads`. Go-runtime figures
(`go_gc_*`, `go_goroutines`, `go_info`, `go_memstats_*`) and the system network counters of the
reference's `process_network_*` do not exist in this runtime and are **not emitted**; the WebUI
shows the available figures and marks the rest "不适用于 Rust 运行时". The start time is the
process's (unchanged by an in-place restart, as in the reference). The upstream and cache series
of `/metrics` are described under "Diagnostics (F-2)" below.

## Diagnostics (F-2)

**Log capture (`capture.logs`)** — `POST /api/v1/capture/start` and `GET /api/v1/capture/logs`, with
the reference's contract. `start` takes an optional body `{"duration_seconds": n}` (empty body or
absent/null field: 120); `1 <= n <= 600`, otherwise `400 Duration must be between 1 and 600 seconds`;
an undecodable body is `400 Invalid request body: <decoder text>` (the text after the prefix is the
native decoder's); success is `200` with `Log capture started for N seconds. Log level set to DEBUG.`.
`logs` returns `200 application/json`, an array of line objects, and **empties the buffer** (read
once); no capture yet or an already read buffer gives `[]`. Other methods are `405`. A second `start`
empties the buffer and restarts the window. The window ends lazily at its deadline (no timer task);
lines already captured stay readable after it.

While a capture runs the diagnostic threshold of the host drops to debug (also for `log.file`/stderr)
and, at the end, the **configured `log.level` is in force again** — it is never overwritten (the
reference resets to info whatever was configured; this is an intentional difference). Captured lines
are `{level:"debug", time (RFC 3339), msg, logger_name, …fields}`: one `dns query completed` line
(`logger_name:"query"`) per query of an `enable_audit` listener with `trace_id`, `domain`, `qtype`,
`client`, `transport`, `outcome`, `cache` (`hit`/`miss`/`notapplicable`/`undetermined`), `elapsed_ms`,
and — when the response or the execution established them — `rcode`, `route`, `matched_group`,
`domain_set`, `upstream`, `failure`; plus the host's own low-frequency debug lines
(`logger_name:"host"`).

**Step trace (F-2b).** Before that summary line the capture also carries the reference's step log,
with its two messages and field names, under the **same `trace_id`** as the summary line and for the
same queries (those of `enable_audit` listeners): after every *named* matcher evaluation
`dns query flows through matcher` with `trace_id`, `domain`, `matcher_name`, `match_result`; and,
once every matcher of a rule matched and before its action runs, `dns query flows through plugin`
with `trace_id`, `query_id`, `domain`, `plugin_name` (a rule without an action is reported too, as
`no_exec`). `domain` is the question name at that step without the trailing dot (so a rewrite or
redirect shows). Lines are reported by the engine at the one place all matchers and rules run, so
the root sequence, `call`/`goto`/`jump` targets, sequences used as an `exec`, multi-`exec` rules
(the rule as `multi_exec[...]`, then each executable) and fallback/preference branches all report
(concurrent branches interleave; the lines carry their times). `logger_name` is the configuration
sequence that is executing; a synthetic inline sequence reports under the sequence it belongs to.
(Difference: the reference reports the rules reached through `jump`/`goto` under the logger of the
jumping sequence; here they are reported under the target sequence, so the UI's "进入序列" marker
names the sequence that really runs.) As in the
reference, the first false matcher of a rule ends its evaluation (later matchers of that rule are not
evaluated or reported), and a sequence that runs to its end reports nothing more.
Names are generated at configuration compile time with the reference's rules (`sequence.go`):
`matcher_name` is the tag of a `$tag` matcher, otherwise `anonymous_match(type: args)` with the
argument text as written (first-space split, trimmed), wrapped in `not(...)` for `!`; `plugin_name` is
`no_exec`, the tag of a single `$tag` exec, `anonymous_exec(type: args)` for a single quick exec, or
`multi_exec[tag-or-type,...]`. (The task text said "raw configuration text"; the reference's actual
output is the generated name above, which is what is implemented and compared.) Matchers and rules
the host inserts that have no configuration text (managed-profile rules, etc.) are not reported.
A query is traced only when it was admitted while a capture was running; queries already in flight
when `start` is called are not.

The per-query lines go **only to the capture buffer**, never to the log file or stderr, so a capture
does not put file I/O on the DNS thread; recording is a short critical section on a dedicated mutex.
With no capture running the engine does one `Option` check per matcher and per rule (no allocation,
no formatting) and admission one relaxed atomic load. The buffer is bounded: at most **32768 lines
and 8 MiB** of serialized text (one query's trace is about 10–20 lines; the reference's buffer is
unbounded), string values cut at 256 characters; the oldest line is dropped first and a `warn` line
`capture buffer limit reached: N older lines dropped` leads the next read.

**`/metrics` upstream series** — for every `aliapi` upstream **with a `tag`** (as in the reference),
from startup at zero: `mosdns_aliapi_query_total`, `_error_total`, `_upstream_winner_total`
(counters) and `mosdns_aliapi_response_latency_millisecond` (histogram: `_bucket{le=…}` with the
bounds `1,5,10,20,50,100,200,500,1000,2000,5000,+Inf`, `_sum`, `_count`), labelled `addr` (the
configured address text), `metrics_tag` (the plugin tag) and `tag` (the upstream tag). Units are
counts and whole milliseconds (truncated, as the reference). Semantics: `query_total` counts every
upstream attempt, including attempts later canceled (a concurrent group's losers); `error_total`
counts failed and timed-out attempts; a canceled or dropped attempt is neither an error nor a
latency observation (the reference lets losers finish, so they would count there); the latency
histogram observes successful attempts only; `winner_total` counts attempts whose response became
their group's final response. Counters stay monotonic across configuration changes (the reference
resets an upstream's counters when its group is rebuilt). The `mosdns_cache_*` series and the
process figures are unchanged.

*Not provided* (no matching native event, and nothing reads them): `mosdns_aliapi_inflight_queries`
(attempts are tallied when a query ends, so attempts in flight are not visible), and
`mosdns_aliapi_conn_opened_total` / `_conn_closed_total`; the `mosdns_forward_*` and
`mosdns_metrics_collector_*` families (`config_rust` uses neither plugin); the `go_*` runtime series.
They are not emitted as zeros.

**`/debug/pprof` — compatibility decision.** The native host does **not** serve the Go profiler
(`/debug/pprof/*`, `cmdline`, `profile`, `symbol`, `trace`): the paths answer `404`. The endpoint is
the Go runtime's profile format (feature-coverage C16, class "implementation detail"), no WebUI or
`config_all` path uses it, and there is no equivalent in a Rust binary. Replacements: the log capture
above, `/metrics`, and `perf` on the host (`perf record -e cpu-clock -F 999 --call-graph dwarf -p
<pid>`, as used for the 5D-2c profile).

## Capability discovery and UI

Schema1 retains its existing keys and adds `ui_operations`: all 30 IDs from
[the approved matrix](../plans/native-webui-runtime-capabilities.md), each with
boolean `supported` and true→null / false→displayable-string `reason`. Eligibility
comes from actual handlers/providers: an empty managed profile alone does not
make local-rule writes available. Existing per-item format/read-only contracts
remain authoritative. Missing Go-only APIs do not gain fake success handlers.

Both Vue shells share pending/ready/error discovery, completing discovery before
optional requests. Only discovery404 selects legacy Go behavior. Network/non404,
invalid schema or incomplete present operations show a retryable error and block
optional writes/requests. Exact old-native schema1 without ui_operations uses the
approved strict-boolean fallback and never becomes legacy. Retries discard failed
promises. Catalog-changing successful mutations refresh capabilities; a later
failed refresh keeps the actual mutation ACK factual and existing drafts visible.

Initial reads, all modes/tabs, refresh paths and handlers use the same operation
admission. Unsupported operations remain visibly disabled with family reasons.
Actual `/plugins/switch*/show|post` uses switches.manage, and follow-up requery is
independently gated. Aliases and Go-only system/config/update functions stay
suppressed (capture is supported since F-2; remembered lists, server appearance and
process metrics are supported where their own sections say so).
`rules.adguard` is supported when an `adguard_rule` tagged `adguard` exists, and
`rules.diversion` also when any `sd_set_light`/`si_set` exists; the diversion tab then
probes the built-in provider tags (`/plugins/{tag}/config`, 404 = not configured) and
uses the reference online-source workflow for the found ones, while `special_route_N`
catalogs keep the native local-text workflow (see native-rule-providers.md).
Local theme/color storage works. Native identity displays Rust/product version,
without fabricated zero Go metrics. Audit/details, named-cache catalog/list/flush,
eligible local rules and native managed group/upstream workflows remain available.
Native switch-capable peers additionally return the value-free `switches` inventory
extension: schema 1, canonical string `config_generation`, and one entry per
configured switch with type, tag, read/write eligibility, and a read-only reason.
Both Vue shells use this inventory for custom-tag switch controls; they do not
invent product-specific cache or feature follow-ups for native owners. The full
state, HTTP, generation, and query-admission contract is in
[Native switch state](native-switch-state.md).

## Opt-in build and evidence

Run `scripts/build-rust-native.sh` from a fresh clone with Rust/Cargo, Node/npm and
network/cache access to the locked dependencies. It installs locked npm inputs,
builds the maintained Vue UI (the compatibility UI is Go-only and not built), validates roots/assets/stamps, and
only then runs locked Cargo release for the native host in a source/asset-hash-keyed
Cargo directory, preventing restored older mtimes from reusing a stale executable. `BUILD_VERSION` overrides
repository `git describe --dirty` (fallback dev). The URL-encoded asset query stamp
decodes to the exact product version. `OUTPUT` defaults to `release/mosdns-native`.
Its adjacent `.manifest.json` records actual source/asset hashes, compiler,
artifact SHA-256 and CLI version. Source ID is null unless `BUILD_SOURCE_ID` is
explicitly supplied; dirty code is not represented as clean HEAD. Build-time Node
is not a runtime dependency. Existing Go/default and hybrid build paths are not
changed. See the [task evidence](../validation-records/10-04-rust-native-webui-runtime-capabilities/s6-status.md).
