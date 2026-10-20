# Shadow traffic experiment

These are operator tools, not part of the DNS runtime. Run only on a separately
authorized test host, as root, with Python 3, iproute2, nftables and systemd.
No product configuration, capture, domain sample or real address belongs here.
All runtime configuration and output directories must be outside this repository
and private (0700 directory, umask 077).

The source packet stream is pcapng over an already restricted capture SSH key.
`protocol.py` filters Ethernet/VLAN IPv4 UDP; the runner selects the reference
server's incoming queries and outgoing replies, excluding its upstream traffic.
Unsupported/truncated packets, missing replies, ambiguity and overload have
separate counters. Only complete Go/Rust pairs count as agreement. Concrete
addresses/TTLs are ignored, while rcode, answer presence, configured fake/real
classes, RR types, CNAME terminals and HTTPS/SVCB targets are compared. Differences
remain unexplained until evidence supports a classification.

The test DNS runtime stays in the host network so it retains actual upstream
access. The sole raw injector runs inside `shadow1-replay`, with only loopback
and a veth. It preserves the captured client IP/port, txid and query payload.
An output mark routes source-port-53 responses back through the veth. Namespace
inbound traffic is limited to the test host's DNS; forwarding is dropped.
A physical-interface egress guard must be installed **before** network setup:
drop and count UDP/TCP source port 53. `network.py` adds a forged-source backstop.
The guard intentionally remains installed when replay ends. Do not start real
replay unless `isolation_probe.py` has proved a documentation-client round trip,
zero observed physical UDP-53 responses and zero guard counters. Production
service management is never part of these tools.

The task-specific deployment owns and records the guard, runtime JSON and service
units. The runtime JSON supplies `target`, `production`, `api` (loopback HTTP),
`capture_target`, `capture_key` and `fake_ranges`; values come from the authorized
environment, not defaults in source. Set these from actual configuration and
verify the physical interface before installing rules. `network.py` takes the
existing test-host address via `--target`; it refuses an existing task state or
occupied route table/priority. It does not change physical addresses or devices.
If setup fails, retain its partial state and correct the exact failed step;
the physical guard must stay in place throughout recovery.

Example after the private runtime inputs/guard have been prepared:

```sh
umask 077
python3 tools/shadow/network.py /private/shadow --target 192.0.2.53
python3 tools/shadow/isolation_probe.py /private/shadow
mkdir -m700 /private/shadow/smoke
systemd-run --unit=shadow1-smoke --property=UMask=0077 \
  --property=KillMode=mixed --property=TimeoutStopSec=30 --property=Restart=no \
  python3 tools/shadow/run.py /private/shadow/smoke \
  --network /private/shadow --config /private/shadow/runtime.json \
  --seconds 3600 --idle-seconds 60 --qps 100
```

The capture transport reconnects with bounded backoff. A collector crash does not
silently restart/reset its counters; inspect the failure and preserve evidence.
Replay is paced (100 qps by default), with at most 512 pending queries and 8192
queued events. Queries keep their capture time separately from replay time.
The final 20 seconds only drain already captured work. Duplicate queries are
deduplicated while pending; changed payloads under an identical pairing key are
counted as conflicts. DNS answers match the full question set and original tuple.
Audit correlation uses the same host's actual send/receive interval, client IP,
qname and qtype; multiple candidates remain ambiguous. An unmappable supplier
is retained explicitly rather than guessed to be an upstream group.

Every minute the collector writes Rust, collector, namespace worker and capture
SSH RSS/HWM, CPU time, PID/start ticks, thread/FD counts, delay percentiles and
cumulative counters. CPU percentages are derived from adjacent monotonic samples.
The initial no-traffic collection interval measures sampler baseline cost.
Every private paired row is retained in a fresh run's `paired.jsonl`; no fixed
backup rotation silently deletes earlier samples. Capture stderr is separate
and does not contain packet/domain payloads. Disk/write failures invalidate
collection rather than being reported as Rust failures. The physical guard and
namespace remain for inspection.

Collector + namespace worker + capture SSH have a combined 96 MiB RSS ceiling.
The earlier 64 MiB ceiling was below the observed roughly 66 MiB Python/SSH
working set; 96 MiB gives bounded headroom while keeping this separate from the
Rust 128 MiB steady / 256 MiB peak limits on the 1 GiB test host. Audit history
retains only the existing 20-second in-flight window. Stop uses KillMode=mixed:
SIGTERM reaches the collector first; it closes the worker pipe and terminates
capture SSH, then systemd kills remaining children only if the stop times out.

`reference.py` runs on the controller using its existing SSH alias/key, samples
the reference service once a minute, and writes only to an explicitly supplied
outside-repository file. It never installs or writes a file on that host. Report
reference PID/start ticks and failed samples rather than silently filling gaps.

```sh
python3 -m unittest discover -s tools/shadow -p 'test_*.py'
```

Stop replay with the exact systemd unit name. Before removing the namespace,
stop and verify all owned workers. Delete only task-owned rules/routes/veth and
restore the saved `rp_filter` value; remove the physical guard last, only after
all raw injectors have exited. Keep the installed Rust binary and cutover
transaction for the task's authorized final state.

### `explained:learned-state` (prospective runs only)

This is not a blanket exception for a cold cache or for any difference after a
restart. The current implementation supports learned no-v4/no-v6 suppression:
both rcodes must be NOERROR, and exactly one side has answers. If TC differs,
the suppressed side must have no TC, the answering side must have TC, and that
answer's wire length must be at most `max(client OPT size, 512)` (512 without
OPT). Missing/ambiguous OPT evidence cannot qualify a TC difference. This
allows a bounded truncated answer versus an empty suppressed answer; all
other TC shapes remain unexplained.
The private, frozen policy must establish a unique `domain_output` owner for
the mapper mark, its generated provider, and the exact sequence rule
`[fast_mark N, qtype Q, switch 'A'] -> reject 0`. Q is A or AAAA.
The generation switch and rule switch must be enabled. The same query's Rust
audit trace ID must have a completed F-2b trace, with the mapper executed before
exactly one occurrence of this mark matcher in that sequence. Then either:

- Rust answers and the learned mark matcher was false (the suppression rule
  was skipped), while the pinned switch remains A; or
- Rust suppresses and the matcher, qtype and switch guards all passed followed
  immediately by reject 0, and the audit domain-set label is that learned set.

The collector checks pinned YAML/settings/switch inputs every two seconds and
permanently disables this explanation if they change. It records the original-query proof beside the private
pair, counts this class separately, and keeps all raw field-difference counters.
An arbitrary earlier learned match is not enough. Pure TC differences, changed
rcode, nonmatching question type, reentry ambiguity, incomplete trace, and
unproven fake-versus-real routing stay unexplained and are counted without stopping. The latter
may qualify under the review's general rule only when a deciding learned route
can actually be proved; this tool does not guess that route. Historical rows
are not rewritten or reclassified.

F-2b capture is enabled for 600 seconds and renewed after draining at 540
seconds. Audit and trace polling occur every two seconds. Only 20 seconds of
trace projections (at most 4096 queries / 256 steps per query) are kept; overflow
cannot qualify. Renewal/buffer gaps fail closed. Trace collection adds work
inside Rust and the collector: the resource report includes this instrumentation
cost and must not present these numbers as uninstrumented performance. Cleanup
shortens the last owned capture to one second.

Finalizing a pair immediately releases its in-flight trace; any evidence needed
for a difference is already in the private row. API responses are projected to
the existing correlation/proof fields before queueing, so unused HTTP payloads
do not remain live across polls. This changes memory lifetime, not evidence
criteria or safety thresholds. The first round-7 attempt hit 104.48 MiB during
a traffic burst and stopped under the unchanged 96 MiB limit; it remains a
failed attempt rather than being merged into the next one-hour window.

For the unresolved historical learned-state shrink, `learned-counts.jsonl`
records the entry count of **every configured domain_output file_stat** at
start, every 600 seconds and exit. It contains counts, not learned names. Any
decrease stops this run as a possible lifecycle defect. There is no automatic
flush waiver: even an intended flush requires a logged cause and operator
verification before continuing. Keep this monitor in the later 48-hour run;
that phase still needs separate approval. Counts observe persisted files, not
the potentially newer in-memory learned overlay.

### Round-7 stopping rules

Semantic differences alone never stop this run. Keep every row, count its raw
field combination and explanation separately, and list Rust SERVFAIL versus
Go NOERROR rows first in the report even if their rate stays below the limits.
The safety triggers are:

- any physical leak counter above zero;
- Rust inactive/crashed, PID or NRestarts changed, or health continuously not
  ready for at least 30 seconds (sampled every five seconds);
- a persisted learned-file count decrease without a logged/verified flush;
- combined collector/worker/capture-SSH RSS above 96 MiB; or cumulative CPU
  in this dedicated systemd cgroup above 10% of one core over a full trailing
  300 seconds (interpolated cumulative counters at the window boundary);
- Rust SERVFAIL with Go NOERROR on strictly more than 1% of all paired rows
  finalized in the trailing 300 seconds, or strictly more than 20 in total.
  The rate also applies to the initial partial window; exactly 1% or 20 does
  not cross a threshold.

`health.jsonl` keeps five-second lifecycle/readiness, leak and tool-resource
samples; `safety-stop.json` records any safety cause. Minute resources remain
separate. Audit/trace read failures count as evidence gaps, invalidate affected
explanations, and are retried; they do not turn semantic differences into a
stop. An unrecoverable collector/worker/write failure means the experiment is
invalid, not a proven Rust defect. There is no proactive free-space stop under
these rules. Prepare sufficient private disk space before starting; a complete
hour and complete detailed rows are required for an accepted report.

### Approved unattended 48-hour phase (rounds 8–9)

`longrun_control.py PRIVATE_ROOT install|start|status|finish` deploys task-owned
systemd units. Supply `PRIVATE_ROOT/runtime.json` and a staged binary/release
manifest first. `shadow1-isolation.service` is enabled at boot, ordered before
Rust and replay. It preserves existing guard counters; partial isolation refuses
automatic repair. The replay service and event/end timers are deliberately not
enabled at boot. A reboot restores guards and records an interrupted run, without
resuming traffic. Only an explicit `start` creates a fresh absolute deadline.

`unattended.py` reuses the existing replay, pairing and comparator. Tool restarts
create separate numbered segments and retain the native identity, cumulative
SERVFAIL count/window and learned-file baseline. The cgroup CPU window needs a
new 300 seconds after a tool restart because its kernel counter may reset; the
segment marker makes that coverage gap visible. Systemd allows at most three
starts over the run. A shared stop latch prevents safety-triggered restarts.
Free disk below 500,000,000 bytes is an additional round-9 safety trigger. The
96 MiB budget covers collector/worker/capture-SSH as in the accepted smoke;
short-lived event actuator RSS is sampled and reported separately.

`longrun_store.py` retains full wires/audit/trace only for non-equal rows. Each
hour has aggregate counters, minute counters, exact numerical latency frames and
numerical classification frames (timestamp, field bitmask, class). Equal records
contain no names, addresses, wires or trace. Completed hours are gzip-compressed;
the current partial hour remains readable. No differences are evicted. The
numerical frames allow exact event-window counts without reconstructing queries.

`longrun_events.py` applies and reverts a typed ECS setting at approximately 6h
and a documentation-domain local rewrite at 18h; stages a local F-3a update at
30h; and restarts Rust at 42h. Claims prevent repeated application. Retained
backups support revert-only recovery after interruption. Planned restart grants
last at most 30 seconds and identify the completed PID; automatic NRestarts and
30-second readiness failures remain safety triggers. F-3a includes preparation
and environment-cleanup restarts, both recorded inside that event. Frozen policy
proofs are disabled on input changes, and can return only after every original
input byte/switch matches for 20 seconds plus another 20-second trace drain.

The end unit leaves Rust and isolation installed, writes `final-summary.json`,
`hourly-series.json`, `event-effects.json` and a numerical archive. Classification
counts use exact numerical frames. Latency percentiles use 1µs bins (at most
0.0005ms rounding error); exact doubles remain in private hourly files. Rust's
five-second RSS includes its complete service cgroup, including the F-3a child.
Go comparison and sampling coverage are merged in Phase B from the Mac file.

The first unattended attempt crossed 96 MiB and remains a separate failed
window. The replacement reads audit/trace arrays incrementally, projects each
entry immediately, shares identical trace strings within one bounded batch,
and polls every second. This reduces whole-response and burst allocations;
the 500-row audit limit, complete trace fields/order, correlation window,
comparison rules and 96 MiB safety limit are unchanged. An individual JSON
value beyond 2 MiB becomes an explicit evidence-read gap, never an explanation.
The API clients also reuse one urllib opener per thread: handlers refer back
to their director, so creating a director per request defers that cyclic
allocation's release to generational GC. Only a fresh explicit `start`, with
no existing run manifest and an inactive replay unit, resets the previous
preflight's systemd start limiter. Automatic tool restarts keep the bounded
budget throughout the live window.

Mac `reference.py --once --end-wall UNIX_TIME` appends a single best-effort
read-only SSH sample. A user launchd job invokes it every 600 seconds. No Go-host
credentials are installed on rtdns, and missing Go samples never stop replay.
See the active task's `research/48h-runbook.md` for the installed instance and
safe-stop commands. Unit tests cover retention, exact event boundaries,
restart grants and rollback/once-only behavior; there is no self-review agent.
