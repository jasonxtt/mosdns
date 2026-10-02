# Native client identity (S1)

## 1. Scope / trigger

Native UDP/TCP execution and client_ip compilation for task
10-02-rust-native-client-context-ecs. ECS policy/cache slices are pending.

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
