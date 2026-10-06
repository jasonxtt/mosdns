# Both-shell request inventory and admission proof

Shared owner `src/api/runtimeCapabilities.js`: one pending/ready/error state and cached discovery, strict30-entry validation, deterministic accepted old-native schema1 fallback. Main shell owners `src/App.vue`, compatibility `src-log1/App.vue`; main content first mounts after valid discovery. Refresh failures keep existing view/drafts mounted, disable optional work and expose retry. Browser-local theme/colours remain local; all server appearance operations share one gate. `http.js`, both System raw upload/export helpers, Data raw response helper and TS dashboard service use capabilityFetch; remaining raw fetches are only the shared admitted request and discovery itself. No global fetch replacement.

| Endpoints / methods | Operation | Owners / paths exercised |
|---|---|---|
| GET capabilities | discovery bypass only | Both App boot, cached shared widget reads, explicit retry/inventory invalidation |
| GET system/health; product version from its JSON | system.health / system.version display | Both App and both System owners; null schemas remain not applicable |
| audit v1/v2 status/logs/stats/details/windows | audit.read | Overview, DNS card/service/poll, Query live, System status; only advertised audit version may reach network |
| audit start/stop/clear POST | audit.control | Both System audit panel, user action and state reread |
| audit capacity/settings GET/POST | audit.capacity | Both System capacity and DNS card service |
| v2 audit rank/{domain,client,slowest,effective,domain_set} | query.rank | Overview sequential ranks/detail; alias dialog rankings retain independent rank gate |
| cache/inventory GET | cache.inventory | Data initial/global refresh; remembers actual named tags, including tags without cache prefix |
| plugins/<named-cache>/{show,search,save,dump,flush,load_dump} | cache.manage | Data/cache popup/flush and separately gated switch follow-up; inventory/per-item backend validation retained |
| metrics GET | metrics.cache | Data and Overview cache/upstream series; process parsing separately guarded by metrics.process |
| plugins/<local-provider>/show GET | rules.local.read | List canonical load/reload; unmanaged standard names admitted only after actual canonical show succeeds (404/400 skipped, other errors propagate); managed group provider catalog retained |
| plugins/<local-provider>/post POST, save GET | rules.local.manage | List save/canonical-confirm/error-draft flow; independent from inherited endpoint manual_rules.post |
| special-groups GET / POST / DELETE | groups.read / groups.manage | Query labels, List profiles, Rules/Upstream catalog/edit; System config-package false does not disable these |
| upstream tags/config/runtime GET / config POST | upstreams.read / upstreams.manage | Upstream shared owner, Overview; global-overrides false does not disable managed upstream edits |
| plugins/<diversion>/config GET/PUT/DELETE | rules.diversion | Rules catalog/local text CRUD, preserving existing per-item format eligibility |
| plugins/adguard/rules GET/POST/PUT/DELETE, update[/id] POST | rules.adguard | Both Rules modes, initial reads, reload/update/create/toggle/delete handlers |
| capture/start POST, capture/logs GET | capture.logs | Query diagnostic toolbar and handlers; distinct from audit capture |
| plugins/clientname GET/PUT | client.aliases | Query startup/dialog/import/export and Overview initial/global refresh |
| plugins/switch*/show GET, post POST | switches.manage | Both System switchProfiles (1,2,3,4,5,6,7,8,9,12,13,16,17), Overview17, Data3, Upstream17 legacy path; no invented switches endpoint |
| plugins/requery status/config/stats/trigger/cancel/scheduler | cache.requery | Data mounted/poll/source counts/user changes and separately gated switch follow-ups |
| plugins/my_* and top_domains show/save/flush | lists.remembered | Data initial/global refresh/table/popup/save/clear; distinct from file-backed local rules |
| appearance/* GET/POST/upload/history/remove | appearance.server | Both App boot and both System preferences/history/raw uploads/automatic server saves; local colour/theme persists without server requests |
| system/restart POST | system.restart | Both App reset/restart, System restart and follow-up operations |
| system/webui-port GET/POST | system.webui_port | Both System startup/apply |
| config/export/update/metadata | system.config_management | Both System package controls/raw export; schema null not coerced to0 |
| update/status/check/apply | system.update | Both System initial/refresh/restart-watch/user actions |
| domain-generation GET/POST | system.domain_generation | Both System initial/refresh/toggle |
| overrides GET/POST | system.global_overrides | Both System overrides/replacement and legacy Upstream read |
| Go version/GC/goroutines/process series projection | metrics.process | System and Overview; native displays unsupported reason rather than0; product identity comes from health |

Executable request-spy tests exercise false initial reads, refresh and stale write paths with actual capabilityFetch and assert fetch count0; test exact managed/unmanaged old-native fixtures, malformed/incomplete entries, discovery failure/retry, named cache gates, accepted mutation ACK during capability failure, and separate switch/requery admission. Actual same-binary browser traverses both shells and all mounted submodes plus global refresh. Injected native discovery delay/non404/invalid payload and old-schema1 cases are explicitly labelled faults, not Go evidence; actual Go404 proof remains S6.
