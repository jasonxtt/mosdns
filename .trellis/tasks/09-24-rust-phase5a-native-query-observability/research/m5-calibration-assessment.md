# M5 W1 assessment: invalid control matrix; acceptance stays closed

Reviewed unit1 HEAD `49ac145ca1345860a92f60682b46c3a127cf7273`.
002reviewer M5-UNIT1-002 PASS preceded measured traffic. The one fixed run
completed18 attempt slots once:1 runner exit0,17 runner exits1. Only the first
slot produced2 valid primary windows (200/400QPS,25s each):15000 scheduled,
sent, received and correct on time; zero late, wrong, protocol, transport,
timeout or sender shortfall. The other17 slots failed before server startup
and sent no queries. No replacements, reruns, exclusions or candidate data.

W1 controls are **UNQUALIFIED / INVALID MATRIX**, not a latency stability
failure: no complete pairs or equivalence intervals exist. The fail-closed
qualification returned false on the first missing sut.json. Both original
analysis outputs retain all18 slots, including invalid rows. It stopped
before W2/W3 and before any candidate acceptance. M2–M4/V12/A5 unchanged.

## Diagnosed harness defect

The server controller tests port availability with an ordinary TCP bind before
creating a result directory. After the first successful session, its local
fixture TCP connections leave TIME_WAIT entries. Without SO_REUSEADDR, this
bind rejects even when no listener exists. Read-only bounded diagnostic at
07:22:04.655691UTC records fixture15454:0 listeners,5519 TIME_WAIT, ordinary
bind errno98; native listener15354:0 listeners,0 TIME_WAIT, bind succeeds.
See m5-startup-diagnostic.json and controller source. Controller exception
files retain failing command but omit remote stderr, so this diagnosis combines
the exact source path, absent later server directories and contemporaneous
socket evidence; it is not a captured perattempt bind traceback.

This defect prevents repeated sessions; it does not establish poor hardware,
network or Rust performance. No kernel/hypervisor/service/resource setting was
changed. A later prospective corrective unit must fix and test the availability
probe, capture remote startup stderr, preserve failed empty sessions, and use
fresh generation IDs/roots before another fixed control run can be considered.
This report authorizes no additional measured run. Do not reuse these results
as a partial control matrix or mix them with later data.

## Evidence retention and limits

Complete original run: /tmp/mosdns-phase5a-m5-w1-49ac145-20260926;
durable verified copy:
/Users/tom/.codex/artifacts/mosdns-phase5a-m5-w1-49ac145-20260926.
All175 manifest entries were verified against that durable copy. Adjacent
m5-w1-results retains176 files: all175 manifested files except the client
requests.jsonl, plus manifest.json and its SHA256 sidecar. The omitted request
ledger remains in the full durable tree and on client. No secret is included.

Remote server original root:
/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-m5-server.
Remote client original root: /root/mosdns-phase5a-m5-client/results.
Only first server session directory exists; later client directories are empty
because failure occurs before queries. Source manifests on each endpoint were
checked against transfer copies for the first attempt; qualification independently
reconstructed its merged files and checked ownership/profile/coverage/GC/RSS/
seven oracle exits. The full-matrix qualification still fails as required.

First-window p95/p99:200QPS1029/1884us;400QPS921/1736us. These unpaired values
support only that one correct distributed session ran; they cannot establish
equivalence, audit overhead, capacity or candidate acceptance. CPU remains a
bracket including SSH handoff, not precise hot-path CPU.
