# M5 unit1/2: distributed W1 control on the available hardware

The user authorized proceeding with10.0.0.50 as client and mosdns-rust as
server, with no additional hardware/resource allocation available. Continue
the existing in_progress task and designated002reviewer; no deployment,
archive/new task or runtime edits. M2/M3/M4 stay unqualified, V12 stays failed.

## Authorized units and prospective boundary

Unit1 implements/checks/freezes the distributed W1 harness and fixed control
protocol; review it before any measured data. Unit2, only after unit1 PASS,
executes one fresh fixed W1 calibration and assesses it. Only qualified W1
permits separately reviewed extension to W2 cold/warm and W3 with their full
oracles, then full control qualification. A separately reviewed V12 supplement
is still required after complete controls; W1 alone cannot award A5.

## Fixed topology and measurement

Client root@10.0.0.50, guest Debian/VMware/Pentium8505,1vCPU/2GiB RAM.
Server SSHmosdns-rust resolves10.0.0.92, guest mosdns-rust/KVM,2vCPU/4GiB.
Physical separation is user-supplied information; guest inventory does not
prove exclusive physicalCPU allocation. Keep these exact endpoints throughout.
No cross-hardware old/new comparison: all SUT slots run the identical archived
Rust-before binary on the same server. Client/settings/network path stay fixed.

Server SUT pinnedCPU0. The controlled TCP upstream remains loopback on server
CPU1; the server-local1Hz resource sampler also runsCPU1. Client query helper
alone runsCPU0 on0.50, GOMAXPROCS1/GOGCoff/GODEBUGgctrace1, GOMEMLIMIT absent.
Server Go fixture gets the same boundedGC settings. Client needs768MiB available
RAM and server2GiB; each sampled Go role must stay below256MiB. Requirements
reflect split process placement, not a relaxed latency budget. No host service,
hypervisor, kernel, firewall or credential/SSH-authorization changes.

Only listener bind changes from127.0.0.1:15354 to10.0.0.92:15354 in the frozen
TCP YAML; audit remains off, upstream remains127.0.0.1:15454. Freeze the exact
overlay/hash before traffic. Workload/DNS/counters unchanged. High test port
exposure lasts only each owned test session. Any foreign query changes the
fixture oracle and invalidates the attempt; no production listener is used.

Helperv10 adds client-only `run --sample-self`: reject serverPID flags, record
only own load-generator samples, leave sut_pid0 and server fields absent.
Server resources are sampled locally in a separate bounded bracket: first
successful sample/identity/CPU check, ready barrier, client25-second stage,
client completion, stop barrier/final successful sample.35-second bound; dead/
reused PID, wrong affinity, timeout or missing sample invalidates it. CPU brackets
include SSH handoff/idle gaps; disclose this and100Hz quantization, never infer
precise hot-path CPU or zeroCPU. Client elapsed request latency remains its
monotonic connection-start through fullresponse, including LAN roundtrip.
Cross-host timestamps are not subtracted; no synchronized-clock assumption.

Original client stages/ledgers/resources and server brackets remain immutable.
Offline merge preserves latency/counters and enriches resources with host
identity; equal numeric PIDs on different machines are allowed. Merged sut_pid
stays0 (client namespace); actual server PID/start/CPU are nested separately.
Existing Go sender/stage/sample/session-counter oracles run over merged evidence.
Samplers/control/SSH activity is measurement scaffolding, not Rust performance.

## Fixed controls and decision

Two fixed batches, each three balanced repetitions of before_off/after_off/
after_on slots, all actually baseline/auditoff. Derive W1 rows from original
V12 plan; freeze both before traffic. Fresh roots on client/server/local;
measure primary200/400QPS for25000ms each,500ms deadline/100ms drain, same
SUT/fixture process across the two stages, fresh process per attempt.
No300/350/recovery capacity or recovery claim; recovery remains indeterminate.

All18 attempts must exit0, exactly36 primary rows/270000 requests fully correct
on time, zero late/wrong/protocol/transport/timeout/shortfall, complete namespace/
binary/tool/input/resource/GC/network evidence, consistent single SUT identity
across both stages per attempt. Sender cannot silently underdrive offered load.
All original individual p95/p99 guards must have zero crossings in both batches;
all eight six-pair90% t(5) log-ratio intervals must lie strictly inside ±log1.10.
Same t=2.01504837333302, SD/sqrt6 whole-attempt pairs, assumptions disclosed.
No margin widening, substitutions, exclusions, reruns or old-result mixing.

If validity or stability fails, archive this exact M5 result and keep acceptance
closed. Do not substitute candidate runs or resample until favorable. Success
only allows preparing the next already-authorized separately reviewed full-oracle
extension, never candidate/A5 PASS. Network may add noise; this design tests
whether client/server separation is sufficient, not a promise of improvement.

## Pretraffic evidence

Unit1 helper build: Linux amd64 Go1.26.4, `go build -trimpath`, helperv10 SHA
`28d5faf5f5129aa990aac51efd8752eba655b852f0bcb27619b216e572c0450e`.
Original TCP YAML SHA
`1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1`;
LAN overlay SHA
`bfd243afbbf26cf8d890fc99bbf24a6d5ba2087c728ed7d4692aced0aca0cb7e`.
Go source/tool/input hashes are verified on both endpoints before every
attempt and after evidence collection. Actual client and fixture Go profiles
are recorded; only the four measurement environment keys are persisted.
Qualification reconstructs merged files from originals, requires25 samples
per role and a25–35.5-second bracket, zeroGC traces and all seven oracle exits
zero. SourceHEAD and the exact reviewer-approved HEAD are required for run.
The preflight snapshot in `m5-preflight/` includes two empty attempt ledgers;
its source-head names the prospective unit's parent because it preceded commit.

Pin source parent/head, helper Linux build/toolchain/hash, scripts/analyzers,
baseline SHA370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa,
original YAML/workload and overlay hashes, host inventories/routes/IPs/affinity/
filesystem/memory/CLK_TCK, balanced plans, all tools. Authentication uses a
temporary local SSH multiplex socket; no password in commands/files/repository.
Read-only preflight and unit tests are permitted; no measured SUT traffic until
unit1 review PASS. Both endpoints store measured raw on ext4; local copied
derived data may reside onAPFS. Hash every raw file after sessions stop.
