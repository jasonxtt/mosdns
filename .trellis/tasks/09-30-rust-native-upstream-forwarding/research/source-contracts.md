# Source-backed contracts and planning decisions

Baseline: rust HEAD 860c6253. Read-only discovery; library foundations are
reused according to current code and archived decisions, not Go internals.

| Source | Evidence / planning implication |
| --- | --- |
| rust/native-host/src/config.rs:40 | ForwardConfig currently has one Endpoint and optional upstream tag. |
| rust/native-host/src/config.rs:455 | Effective upstream identities currently unique across the host. Preserve existing single-entry identity compatibility; freeze multi-entry identity before coding. |
| rust/native-host/src/config.rs:742 | Only args.upstreams accepted, exactly one entry, tag/addr only. New accepted and rejected fields need explicit load-time tests. |
| rust/native-host/src/config.rs:1265 | Named executable references currently reject arguments; tagged subset invocation must be compiled explicitly. |
| rust/native-host/src/config.rs:1367 | Endpoint parsing accepts only numeric udp/tcp SocketAddr. |
| rust/native-host/src/assembly.rs:536 | ForwardAdapter wraps Upstream; catalog/execution seam is the integration owner. |
| rust/native-host/src/execution.rs:34 | ExchangeExecutor returns a transport response; richer selected-entry facts belong at this seam, not an after-the-fact config lookup. |
| rust/native-host/src/execution.rs:700 | Final supplier projection currently resolves identity to configured numeric endpoint; must change causally for races and hostname destinations. |
| plugin/executable/forward/forward.go:54 | Config includes concurrent, global/per-entry bootstrap, dial/timeout/options. Unimplemented options must keep explicit unsupported errors. |
| plugin/executable/forward/forward.go:206 | `$forward [entry tags]` subset entry point; quick forward addr list at :391 defaults concurrent to 3. |
| plugin/executable/forward/forward.go:242 | Concurrent <=0 becomes1, >3 becomes3; random rotated entries currently may repeat when count exceeds entries. Duplicate work is not proposed for native host. |
| plugin/executable/forward/forward.go:298 | Go launches Background-based per-leg timers; native scoped cancellation/drain is a deliberate implementation difference requiring explicit planning approval. |
| plugin/executable/forward/forward.go:324 | IP answer wins immediately; first completed NOERROR/NXDOMAIN fallback outranks other valid responses and errors. Preserve product selection. |
| pkg/upstream/upstream.go:126 | Bare UDP/default ports; dial override preserves service identity; pipeline/H3/proxy/socket options are separate capabilities. |
| rust/upstream-core/src/composite.rs:34 | Existing UDP TC-to-TCP same-target policy, one original deadline, no timeout/malformed-triggered retry. |
| rust/upstream-core/src/secure/endpoint.rs:184 | DotEndpoint numeric destination separate from authenticated identity. |
| rust/upstream-core/src/secure/endpoint.rs:227 | DohEndpoint original URL authority/path/query separate from dial. |
| rust/upstream-core/src/secure/tls.rs:94 | Explicit roots required; verified policy refuses empty store; host must supply Linux trust roots. |
| rust/upstream-core/src/resolver/mod.rs:143 | Existing ResolutionMode distinguishes omitted/4, 6 and explicit0 dual A preference. |
| rust/upstream-core/src/resolver/owner.rs:225 | Existing bounded single-flight resolver/close/publication; use current generation rather than recreate lookup logic. |
| rust/upstream-core/src/reuse.rs:490 | Serial ReuseOwner, Busy currently collapsed into Runtime(NotSent); preserve a narrow typed busy seam to avoid retrying arbitrary runtime errors. |
| rust/upstream-core/src/reuse.rs:972 | DoT reuse includes authenticated identity/trust-policy revision. |
| rust/upstream-core/src/reuse.rs:1385 | DoH reuse retains HTTP protocol and scoped H2 ownership. |

## Prior native decisions to carry into planning

Archived dual-stack task `09-18-rust-phase4-dual-stack-endpoint-selection`
explicitly approved omitted/4 A-only, 6 AAAA-only, explicit0 dual collection/A
preference, no connection racing. Its host/config integration was deferred:
this task must obtain fresh approval for that boundary, not assume library
approval alone authorizes YAML integration.

Archived reuse task `09-18-rust-phase4-connection-reuse-pipeline` approved
serial connections, original IDs, idle bounds and only pre-send replacement;
pipeline and arbitrary retry remain deferred. Connection reuse is internal;
explicit idle_timeout/pipeline configuration cannot be silently accepted.

Archived query diagnostics task now supplies final-wire rich records, distinct
configured labels and actual numeric peer, independent retained slowest history,
two-slot reads and per-panel errors. This task extends supplier causality only;
it does not reopen capacity/answers/read policy or claim new management APIs.

## Family decision and final planning approval

The user questioned the IPv4-only default and requested our judgement. It is
not a DNS requirement. Original MosDNS v5.3.4 bootstrap maps omitted/0/4 to A
and 6 to AAAA; without explicit bootstrap, eligible transports use system
resolution. This controls resolving upstream hostnames, not forwarded AAAA
queries or the bootstrap server's transport family.

Primary sources inspected on 2026-09-30:
- https://raw.githubusercontent.com/IrineSistiana/mosdns/v5.3.4/pkg/upstream/bootstrap/bootstrap.go
- https://raw.githubusercontent.com/IrineSistiana/mosdns/v5.3.4/pkg/upstream/upstream.go
- https://www.rfc-editor.org/rfc/rfc8305.html

The user explicitly approved on 2026-09-30: host omitted/0 => existing dual
collection/A preference, 4 => A-only, 6 => AAAA-only. Preserve archived/core
history and make the host mapping explicit. This supports AAAA-only hostnames;
it does not solve broken IPv4 connectivity when A exists, since connect-family
fallback/racing remains deferred. Dual lookup can also wait longer for the
other family under the existing lookup policy. RFC 8305 provides dual-family
connectivity guidance, not a claim this bounded task implements Happy Eyeballs.

The remaining full-summary approval covers explicit bootstrap or numeric dial
requirements, distinct fanout and canceled/joined losers as listed native
deviations. Public identity/error matrix is now concrete in forward-contracts.md;
material changes require renewed planning approval before implementation.

## Deferred technical work

Linux root-loader dependency API/version and test CA injection are selected and
audited before implementation. No dependency has been added or advertised as
validated. Primary-source web checks were performed for the family-default
question. The user supplied external planning feedback subsequently; this
conversation has not contacted a C2C reviewer or independently verified a new
transport/verdict. Its source-backed disposition is in planning-review.md.
The final task review requires a dedicated verified reviewer binding; earlier
tasks' planning/reviewer chats must not be inherited.
