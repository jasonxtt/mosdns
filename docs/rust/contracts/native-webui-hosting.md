# Native WebUI hosting and runtime capabilities

This contract belongs to the opt-in pure Rust `mosdns-native-host`. It does not
select the production/default binary or close the full native migration gate.

## Same-origin hosting

`api.http` serves maintained Vue `/`, compatibility Vue `/log`, and recursively
embedded `/assets/...` from the executable. No Go, Node, frontend proxy, source
checkout or external web server is required at runtime. Cargo validates required
roots and their referenced assets, regular files, forbidden secret/map names and
symlinks, and tracks source assets for rebuilds. Roots use `no-cache`, accurate
MIME, `nosniff`, representation lengths and SHA-256 strong ETags. GET/HEAD support
weak/list/star If-None-Match; HEAD and 304 have no body. `/log/` redirects to `/log`
without losing a safe query. Other methods return 405 with Allow; missing assets
return 404, without SPA fallback or directory listings.

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
`ready`; see native-special-groups.md) and `udp_receive_buffers` (one
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
process's (unchanged by an in-place restart, as in the reference). The complete native `/metrics`
remains a later task.

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
independently gated. Capture, remembered lists, aliases, server appearance,
Go process metrics and Go-only system/config/update functions stay suppressed.
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
builds maintained Vue then compatibility Vue, validates roots/assets/stamps, and
only then runs locked Cargo release for the native host in a source/asset-hash-keyed
Cargo directory, preventing restored older mtimes from reusing a stale executable. `BUILD_VERSION` overrides
repository `git describe --dirty` (fallback dev). The URL-encoded asset query stamp
decodes to the exact product version. `OUTPUT` defaults to `release/mosdns-native`.
Its adjacent `.manifest.json` records actual source/asset hashes, compiler,
artifact SHA-256 and CLI version. Source ID is null unless `BUILD_SOURCE_ID` is
explicitly supplied; dirty code is not represented as clean HEAD. Build-time Node
is not a runtime dependency. Existing Go/default and hybrid build paths are not
changed. See the [task evidence](../validation-records/10-04-rust-native-webui-runtime-capabilities/s6-status.md).
