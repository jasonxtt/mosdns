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
`applied_config_schema:null`, `config_management_enabled:false`. Null schemas do
not mean YAML schema zero or disable native managed group/upstream operations.
State values are `starting`, `ready`, `applying`, `recovery_required`, `stopping`,
and `closed`. Health is read-only and crosses the admission rejection barrier without a lease.
Only all-listener-ready lifecycle returns 200/ready:true; startup, applying,
recovery-required, stopping and closed return 503/ready:false. Other methods405.

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
independently gated. Capture, AdGuard, remembered lists, aliases, server appearance,
Go process metrics and Go-only system/config/update functions stay suppressed.
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
