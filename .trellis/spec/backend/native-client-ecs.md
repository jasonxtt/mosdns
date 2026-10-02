# Native client identity and outgoing ECS (S1–S2)

## 1. Scope / trigger

Native UDP/TCP execution and client_ip compilation for task
10-02-rust-native-client-context-ecs. Outgoing ecs_handler/quick ecs are included; supplier echo/cache/dump proof remain pending.

## 2. Signatures

sequence-core ClientContext::from_peer(IpAddr, ClientTransport) owns normalized
identity; Default means None/Unknown. QueryState.client travels with the owned
execution state/snapshots/successor recipes. Native ExecutionRequest receives
it directly from UDP recv peer or TCP accepted peer before sequence entry.

## 3. Contracts

client_ip uses the immutable startup IP prefix catalog, literal/CIDR/$ip_set/
&text-file OR and normal negation. Unknown is false before negation. IPv4-mapped
peers unmap once. No ECS/headers/audit records supply identity. Branch copies,
redirect question restoration, preference probes and refresh recipe state
preserve it. No API/audit schema or cache-key change in S1.

## 4. Validation / error matrix

Empty argument, malformed prefix, missing/wrong-type provider and empty file
reference fail configuration. File rules follow existing startup resource and
missing-file rules. Listener metadata is available when audit is disabled.

## 5. Good / base / bad cases

Good: a dual-stack accepted IPv4 TCP peer matches 127.0.0.1 even with forged
203.0.113/24 ECS. Base: direct embedding defaults unknown. Bad: guessing
loopback for an absent peer, or deriving peer from ECS/observer capture.

## 6. Required tests

native-host/tests/client_context.rs: actual UDP source-address branches,
IPv6 UDP/TCP and mapped TCP, audit on/off, forged ECS independence, unknown
embedding, mapped normalization, literal/provider/file OR and snapshot survival
of file deletion; negative compilation. Run native-host/sequence regressions,
fmt --all --check and all-target clippy remotely.

## 7. Wrong vs correct

Wrong: audit admission is the execution identity store. Correct: pass socket
identity into owned query state independently, then let observer report its
separate existing projection.

## S2 handler contract

Named ecs_handler defaults forward=false/send=false/preset absent/mask4=24/mask6=48.
Zero masks select the default; bounds are 0..32/128 before defaulting. Unknown
fields/types and non-IP presets fail compilation. Quick ecs accepts its first
IP, warns and ignores slash/trailing tokens; empty quick ecs is a no-op.

QueryView owns immutable admission wire independently of its local outbound
wire. The first active handler validates and strips admission ECS, selecting
forwarded incoming, preset, trusted peer or none in that order. Existing policy
ECS wins at later handlers. IPv4 mapped presets unmap. Scope successors share
fuel/deadline and preserve peer context; enclosing query state is restored.
Explicit malformed ECS fails locally: duplicate/family0/unsupported family,
query scope nonzero, invalid address length/prefix or nonzero unused bits.
Non-IN and handler-free paths retain their original wire. OPT size/DO/other
validated options survive; generated OPT uses size1232, DO=false.

Tests: native-host/tests/ecs_wire.rs uses real listener and controlled upstream
wire capture for selection, IPv4/IPv6 masks, noOPT, invalid input/configuration,
current policy retention and legacy behavior. Supplier response adaptation is
S3 acceptance; do not infer its PASS from outgoing-wire tests.

## S3 response ownership

QueryView echo_ecs is an explicit optional token granted only by forwarded
admission ECS. Existing policy views inherit it; presets/peer generation do not
grant one. Redirect/reference clones retain immutable admission and current
policy/token. After the own successor completes, consume only its returned
network supplier's response ECS: one supported exact family/source/network and
legal scope. Cache/local response never fabricates an ECS scope. No original
client OPT means no final OPT; an EDNS client receives original size/DO and
otherwise permitted supplier options even when supplier/cache wire lacks OPT.

Response validation treats malformed/mismatching ECS independently of valid DNS.
Rename its wire option codes without moving bytes, decode/re-encode all names,
remove the renamed options, and conditionally add the sole authorized supplier
ECS. This avoids passing invalid ECS through a permissive foundational parser
or breaking compression pointers when removing bytes. Duplicate ECS is stripped.
Existing root fuel/deadline/cancellation and terminal exit provenance are retained.

Real UDP tests cover legal/mismatching/invalid/duplicate supplier ECS, generated
and noOPT suppression, nested redirect/handler, fallback winner versus loser,
preference local suppression, cache-hit no invented scope and terminal exit.

## S4 cache identity and placement

Named enable_ecs is a boolean opt-in; absent/null/false and every quick cache
retain ECS bypass. True uses base AD/CD/DO/QTYPE/product-name bytes, then one
length byte and canonical Go ECS.String: IPv4 network/mask/0 or bracketed IPv6
network/mask/0. No ECS keeps exactly the old base key. Strict profile failure
bypasses cache; no scope covering or peer-IP-only partition is introduced.
Network forwarding and key formation both read the current scoped query view.
Full keys also govern refresh singleflight. Stored wire still removes OPT.

Configuration assembly builds monotone suffix summaries over the validated
program (including synthetic exec-list scopes), naming ECS/client_ip effects
and reachable cache owners. Every named/quick cache rejects unsafe own
successors, regardless of enable_ecs. Call/goto/jump/try, fallback targets,
preference successors and recursive graphs are covered. Unconditional terminal
flow stops reachability; conditional rules remain possibly reachable. Jump's
same-scope inherited continuation is included; ordinary child/inline publication
boundaries remain separate. Fallback targets also conservatively check inherited
successors. Invalid references already fail program validation. Diagnostics name
cache and offending ECS policy or client_ip rule; no runtime policy reordering.

Tests ecs_cache.rs cover literal wire full-key partition, noECS old key,
false bypass, invalid-profile bypass, full-key refresh singleflight, recursive
and every control-flow placement shape, safe terminal/no-op/own-boundary cases.
ecs_wire.rs proves actual forward-handler -> enabled cache cold/hit flow with
one upstream request and no fabricated hit echo. Dump import extensions are S5.

Go ECS.String selects net.IP.To4 even for family2 mapped addresses; native keys
match that unbracketed text with the original >=96 source prefix, preserving
family2 wire identity and separation from family1. Other IPv6 networks remain
bracketed. A noOPT compressed query may reference trailing name storage: retain
its existing base key and rebuild the expanded question before adding new OPT.
