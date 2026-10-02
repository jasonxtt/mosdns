# S6 controlled DNS/API/Vue proof

All services ran only in `/dev/shm/mosdns-rust-client-ecs-20261002/proof`
on SSH alias mosdns-rust. Native binary from task target; owned pid/start/hash
in *.owned.json. UDP then TCP listener on loopback; upstream is always UDP.
No public DNS or port53. Client127.0.0.2 and127.0.0.3 produced generated ECS /32,
controlled supplier answered192.0.2.20/.30. Original noOPT queries receive no
fabricated OPT/ECS. Third same-peer request uses the partitioned cache.

udp-oracle.json and tcp-browser-cold-oracle.json retain actual wire, API logs,
cache keys and metrics. TCP first run inherited UDP persisted entries; its
hit-only current details correctly had no network supplier. Then explicit flush
and cold TCP queries produced source-backed network supplier records.

Browser observed the maintained Vue query table with both answers, then opened
current query details for TCP cold records ending0004 and0005 in the native
trace session n-b7a728f762b906b5af72fba98a29770a. Both were complete/NOERROR/RA,
with controlled at127.0.0.1:21954 overudp, attempt0/branch0/QTYPE1/response and
respectively192.0.2.20/.30 TTL60. Client field respectively127.0.0.2/.3.
A screenshot was emitted in the Codex tool record; the browser DOM was also
read for the actual current detail, beyond table/API-only evidence.

UI source/manifests54 files exactly matched the previously built isolated
/root/mosdns-rust-cache-lifecycle-20261001 webui-log. Reused that existing
bundle through a task-owned loopback static/API proxy; UI sources unchanged.
Local SSH forwarding and task services stopped after proof. shutdown.jsonl
and s6-owned-cleanup.json record close/dump existence/port release.
TCP config initially missed required idle_timeout; startup failed and generated
proxy connection-refused logs, then was fixed. Failure logs remain accurate.

A second TCP cold cycle plus SIGTERM and restart immediately reloaded both ECS
partitions; tcp-before-restart.json and tcp-restart-oracle.json retain fresh
supplier records versus hit-only restarted records. All restart requests have
correct distinct answers and no fabricated supplier. Final services closed.
