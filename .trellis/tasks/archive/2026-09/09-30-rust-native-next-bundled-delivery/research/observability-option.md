# Candidate package: audit control, DNS overview card and audit panel

Source inspection on branch `rust` at `96fcd0b9` (2026-09-30). This is research
for planning, not an implementation approval.

## Reusable native state

- `rust/native-host/src/api.rs:31-52,70-111`: a host-owned HTTP listener exists,
  but request tasks currently receive only `Rc<CompiledConfig>`. The existing
  DNS+HTTP supervisor should be reused.
- `rust/native-host/src/observer.rs:618-745`: `QueryObserver` owns lifetime
  metrics and a bounded audit ring. `audit_enabled: bool` and
  `audit_capacity: NonZeroUsize` are fixed at assembly. `admit()` creates an
  audit context at admission; `record_terminal()` chooses audit by the fixed
  flag. Runtime start/stop and capacity zero need a new, explicit owner.
- `rust/native-host/src/assembly.rs:24-81,119-180`: host options default to a
  100000-entry nonzero ring. File-backed config assembly currently loses the
  top-level config directory after compilation; persistent audit settings need
  a host-level state root rather than a plugin's source directory.

## Current Go and maintained Vue surfaces

- `coremain/api_audit.go:16-90`: v1 start, stop, status, clear, capacity methods
  and visible bodies. `coremain/audit.go:236-238,753-844` stores settings at
  `<config-base>/webinfo/audit_settings.json`, defaults to 100000, and clamps
  capacity to 0..400000. Capacity change clears retained logs/stats. Go ignores
  persistence failure after changing memory; a Rust persist-first failure
  policy would be an intentional safety deviation.
- `coremain/audit.go:698-740,918-935`: v2 stats are derived from retained
  audit records, not lifetime host metrics; Go's one-second rebuild throttle is
  an implementation detail. `coremain/api_audit_v2.go:71-82` also exposes
  windows, ranks and logs; this package can only claim an explicit subset.
- `webui-log/src/services/dashboard.ts:49-90` and
  `webui-log/src/composables/useRealtimeMetrics.ts:83-159`: the real-time DNS
  card requests stats, status, capacity and recent logs (limit 160).
  `webui-log/src/components/dashboard/DnsOverviewCard.vue:82-96` also requests
  `/api/v2/audit/stats/windows` when its popover opens.
- `webui-log/src/components/SystemControlManager.vue:455-518`: the audit panel
  uses v1 status/capacity/start/stop/clear/capacity POST. Other System API
  requests are separate and cannot be called complete by this package.
- `webui-log/src/components/OverviewManager.vue:968-1006` waits for domain,
  client, slowest and effective/domain-set ranks together with stats. Stats
  alone do not make that whole page refresh succeed. Unsupported sections
  must not be represented as delivered.

## Compatibility risks to freeze before implementation

1. Keep lifetime host metrics separate from capture-controlled retained audit
   stats. Clear, stop and capacity changes cannot reset lifetime metrics.
2. Decide the capture point for in-flight requests. Go's `Collect` checks
   capturing at completion and its async worker checks again. C2C recommends
   one deterministic terminal-time sample, with listener `enable_audit` as the
   static eligibility gate; this is intentionally clearer than Go's race.
3. Preserve the canonical settings path and legacy read precedence if this
   package claims restart compatibility. Support Go's saved capacity zero; the
   Vue form itself currently accepts only 1..400000.
4. For bounded v2 logs, project the real record fields into the Go/Vue JSON
   names; do not invent absent trace IDs, answers, response flags or effective
   tags. Reject unsupported filters explicitly rather than ignoring them.
5. Include `stats/windows` if the acceptance claim is the entire real-time DNS
   card. Full OverviewManager ranks and QueryManager filters remain separate.
6. Go `AuditLog.QueryTime` is a `time.Time` serialized by `encoding/json`
   (RFC3339Nano), while window `generated_at` and `coverage_start` explicitly
   use `time.RFC3339` (`coremain/audit.go:953,975,1013`). `QueryName` drops one
   trailing dot only when length exceeds one, preserving root `.`;
   `dns.TypeToString` yields empty text for an unknown numeric type. Client
   address projection uses `net.SplitHostPort` and does not unmap IPv4-mapped
   IPv6 (`coremain/audit.go:329-341`).
7. Go window `coverage_start` is the oldest retained record's query/admission
   timestamp, and `complete` is `oldestTime <= cutoff` (`audit.go:965-1000`).
   It is not a proof of uninterrupted capture. An injected test clock must
   control admission time and window now to prove this with real DNS requests.
8. Go direct v2 logs has no positive-limit cap; the maintained dashboard
   helper bounds requests to 500 and the card uses 160. This PRD proposes a
   direct Rust limit of 500 with 400 above it as an explicit safety deviation.
   Go capacity POST also accepts missing/unknown fields more loosely; the
   proposed Rust body is exactly one required integer `capacity` field.
9. Go startup settings parsing (`coremain/audit.go:213-233`) differs from
   POST: an existing canonical path is chosen before parsing, so bad canonical
   JSON does not fall back to valid legacy; unknown object keys are ignored;
   missing or null `capacity` yields zero; saved values outside 0..400000
   clamp; malformed JSON or a wrong capacity type leaves default 100000.
   This file-read contract is distinct from strict Rust capacity POST.
10. C10 in `docs/rust/feature-coverage.md` covers Prometheus and plugin
    `/plugins/{tag}` APIs. This task implements neither, so only proven C08
    audit and C11 bounded Vue subitems may receive coverage updates.

The connected C2C conversation `6abbc3e5-dc34-83e8-9d07-eb45af7783da`
recommended this package ahead of cache dump/management because the native
observer, HTTP lifecycle and existing Vue consumers already line up. The
user's package choice is pending.
