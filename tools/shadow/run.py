"""Real-traffic shadow experiment. All runtime inputs/output must be private.

No packet is sent through a physical NIC by this program: the sole injector
is worker.py inside the verified veth-only namespace.
"""
import argparse
import collections
import datetime
import hashlib
import ipaddress
import json
import math
import os
import pathlib
import queue
import signal
import subprocess
import threading
import time
import urllib.request

from isolation_probe import counters, worker
from protocol import Pcapng, decode_dns, semantic, udp4
from learned import explain
from state_counts import Counts
from safety import Safety, SafetyError

TOOL_RSS_BUDGET_MIB = 96
API_LOCAL = threading.local()


def project_audit(data):
    fields = ("trace_id", "client_ip", "query_name", "query_type", "query_time", "effective_tag",
              "domain_set", "final_sequence", "final_upstream", "selected_upstream", "upstream_diagnostics")
    return [{key: entry[key] for key in fields if key in entry} for entry in data["logs"]]


def project_trace(entries):
    fields = ("trace_id", "logger_name", "msg", "matcher_name", "match_result", "plugin_name")
    return [{key: entry[key] for key in fields if key in entry} for entry in entries if entry.get("trace_id")]


def api_opener():
    # urllib handlers refer back to their director. Rebuilding that cyclic
    # object graph on every sample leaves reclamation to generational GC.
    if not hasattr(API_LOCAL, 'opener'):
        API_LOCAL.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    return API_LOCAL.opener


def api(base, path, body=None, raw=False):
    # Ignore ambient proxy settings for the strictly local management endpoint.
    request = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json"})
    with api_opener().open(request, timeout=3) as response:
        return response.read().decode().strip() if raw else json.load(response)


def resources(pid):
    root = pathlib.Path("/proc") / str(pid)
    stat = (root / "stat").read_text().rsplit(")", 1)[1].split()
    status = {}
    for line in (root / "status").read_text().splitlines():
        key, _, value = line.partition(":")
        if key in ("VmRSS", "VmHWM", "Threads"):
            status[key] = int(value.split()[0])
    return {"pid": pid, "start_ticks": int(stat[19]), "cpu_seconds": (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK"),
            "rss_kib": status.get("VmRSS"), "hwm_kib": status.get("VmHWM"), "threads": status.get("Threads"),
            "fd_count": len(list((root / "fd").iterdir()))}


class Experiment:
    def __init__(self, args):
        self.args = args
        self.root = args.root
        self.config = json.loads(args.config.read_text())
        self.events = queue.Queue(maxsize=8192)
        self.counts = collections.Counter()
        self.stop = threading.Event()
        self.capture_done = threading.Event()
        self.capture = None
        self.child = None
        self.pending = {}
        self.send_queue = collections.deque()
        self.audit = collections.deque(maxlen=4096)
        self.audit_ids = set()
        self.audit_order = collections.deque()
        self.traces = {}
        self.policy = self.config["learned_policy"]
        self.state_counts = Counts(self.policy["stat_files"], self.root / "learned-counts.jsonl")
        self.trace_renewed = 0
        self.latencies = []
        self.started = time.time()
        self.safety = Safety(time.monotonic())
        self.safety_stop_reason = None
        self.policy_valid = True
        self.trace_invalid_until = 0
        cgroup = next(line[3:].lstrip("/") for line in pathlib.Path("/proc/self/cgroup").read_text().splitlines()
                      if line.startswith("0::"))
        self.cgroup_cpu_file = pathlib.Path("/sys/fs/cgroup") / cgroup / "cpu.stat"
        int(dict(line.split() for line in self.cgroup_cpu_file.read_text().splitlines())["usage_usec"])
        self.aliases = collections.defaultdict(set)
        for group, entries in api(self.config["api"], "/api/v1/upstream/config").items():
            self.aliases[group].add(group)
            for entry in entries:
                if entry.get("enabled", True):
                    self.aliases[entry.get("tag", "")].add(group)
        # Keep every sample in this authorized run; never silently evict old
        # differences through a fixed backupCount rotation policy.
        self.pairs_file = (self.root / "paired.jsonl").open("x", buffering=1)

    def emit(self, kind, value):
        try:
            self.events.put_nowait((kind, value))
        except queue.Full:
            self.counts["event_queue_dropped_" + kind] += 1

    def capture_loop(self):
        attempts, delay = 0, 1
        while not self.stop.is_set() and not self.capture_done.is_set():
            if attempts:
                self.counts["capture_reconnects"] += 1
            attempts += 1
            with (self.root / "capture-ssh.log").open("ab") as errors:
                self.capture = subprocess.Popen(["ssh", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8",
                    "-o", "ServerAliveInterval=10", "-o", "ServerAliveCountMax=2", "-i", self.config["capture_key"],
                    self.config["capture_target"]], stdout=subprocess.PIPE, stderr=errors)
                try:
                    for timestamp, packet, linktype in Pcapng(self.capture.stdout):
                        if self.stop.is_set() or self.capture_done.is_set():
                            break
                        self.counts["captured_packets"] += 1
                        try:
                            row = udp4(packet, linktype)
                        except ValueError:
                            self.counts["capture_parse_errors"] += 1
                            continue
                        if row is None:
                            self.counts["non_ipv4_udp"] += 1
                            continue
                        src, sport, dst, dport, wire = row
                        production = self.config["production"]
                        if dst == production and dport == 53 and src != production:
                            self.emit("query", (timestamp, src, sport, wire))
                        elif src == production and sport == 53 and dst != production:
                            self.emit("go", (timestamp, dst, dport, wire))
                        else:
                            self.counts["excluded_upstream_or_other"] += 1
                    if not self.stop.is_set() and not self.capture_done.is_set():
                        self.counts["capture_stream_ended"] += 1
                except (OSError, ValueError):
                    if not self.stop.is_set() and not self.capture_done.is_set():
                        self.counts["capture_stream_errors"] += 1
                finally:
                    self.capture.stdout.close()
                    self.capture.terminate()
                    try:
                        self.capture.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        self.capture.kill()
                        self.capture.wait()
            self.stop.wait(delay)
            delay = min(30, delay * 2)

    def worker_loop(self):
        for line in self.child.stdout:
            try:
                self.emit("rust", json.loads(line))
            except ValueError:
                self.counts["worker_protocol_errors"] += 1
        self.emit("worker_exit", None)

    def audit_loop(self):
        while not self.stop.is_set():
            try:
                self.emit("audit", project_audit(api(self.config["api"], "/api/v2/audit/logs?page=1&limit=500")))
                # The policy is frozen for this run. A settings/rule/switch
                # edit invalidates the evidence, rather than explaining it.
                for filename, expected in self.policy["files"].items():
                    path = pathlib.Path(filename)
                    actual = hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None
                    if actual != expected:
                        self.emit("policy_changed", None)
                for tag, expected in self.policy["switches"].items():
                    if api(self.config["api"], f"/plugins/{tag}/show", raw=True) != expected:
                        self.emit("policy_changed", None)
                self.emit("trace", project_trace(api(self.config["api"], "/api/v1/capture/logs")))
                if time.monotonic() - self.trace_renewed >= 540:
                    api(self.config["api"], "/api/v1/capture/start", {"duration_seconds": 600}, raw=True)
                    self.trace_renewed = time.monotonic()
                    self.counts["trace_windows"] += 1
            except (OSError, ValueError, KeyError):
                self.counts["audit_read_errors"] += 1
                self.emit("trace_gap", None)
            self.stop.wait(2)

    def record(self, row):
        self.pairs_file.write(json.dumps(row, ensure_ascii=True, separators=(",", ":")) + "\n")

    def trigger_safety(self, reason):
        if self.safety_stop_reason is None:
            self.safety_stop_reason = reason
            (self.root / "safety-stop.json").write_text(json.dumps({"wall": time.time(), "reason": reason}) + "\n")
        self.stop.set()

    def health_loop(self):
        while not self.stop.is_set():
            try:
                props = dict(line.split("=", 1) for line in subprocess.check_output(["systemctl", "show", "mosdns.service",
                    "-p", "MainPID", "-p", "NRestarts", "-p", "ActiveState"], text=True).splitlines())
                try:
                    ready = api(self.config["api"], "/api/v1/system/health").get("ready") is True
                except (OSError, ValueError):
                    ready = False
                    self.counts["health_read_errors"] += 1
                now = time.monotonic()
                rss = 0
                for pid in (os.getpid(), self.child.pid if self.child else 0, self.capture.pid if self.capture else 0):
                    if pid:
                        try:
                            rss += resources(pid).get("rss_kib", 0) or 0
                        except (OSError, ValueError):
                            pass
                cpu = dict(line.split() for line in self.cgroup_cpu_file.read_text().splitlines())
                cpu_seconds = int(cpu["usage_usec"]) / 1e6
                guard = counters()
                data = {"wall": time.time(), "monotonic": now, "service": props, "ready": ready,
                        "tool_cpu_seconds": cpu_seconds, "tool_rss_kib": rss, "guard": guard}
                try:
                    self.safety.health(now, props, ready)
                    data["tool_cpu_5m_percent"] = self.safety.resources(now, cpu_seconds, rss, guard)
                except SafetyError as error:
                    data["safety_stop"] = str(error)
                    self.trigger_safety(str(error))
                with (self.root / "health.jsonl").open("a") as stream:
                    stream.write(json.dumps(data) + "\n")
            except (OSError, ValueError, KeyError, subprocess.CalledProcessError):
                self.counts["health_sampler_errors"] += 1
            self.stop.wait(5)

    def finish(self, key, reason=None):
        row = self.pending.pop(key)
        have = [side for side in ("go", "rust") if side in row]
        fields = []
        classification = "incomplete"
        if len(have) == 2:
            left = semantic(row["go"], self.config["fake_ranges"])
            right = semantic(row["rust"], self.config["fake_ranges"])
            fields = [field for field in left if left[field] != right[field]]
            self.counts["paired"] += 1
            if fields:
                trace = self.traces.get(row.get("audit", {}).get("trace_id"), {})
                query = decode_dns(bytes.fromhex(row["query_wire"]))
                proof = explain(left, right, query["questions"][0][1], row.get("audit", {}), trace, self.policy["rules"],
                                sizes=(len(row["go_wire"]) // 2, len(row["rust_wire"]) // 2), limit=query["udp_limit"])
                classification = "explained:learned-state" if proof else "unexplained"
                self.counts[classification] += 1
                if proof:
                    row["learned_evidence"] = proof
                # Keep the bounded original-query trace, including negative
                # evidence, so an external reviewer can inspect this decision.
                row["trace"] = trace
                for field in fields:
                    self.counts["different_" + field] += 1
            else:
                self.counts["semantic_equal"] += 1
                classification = "semantic_equal"
            row["semantic_go"], row["semantic_rust"] = left, right
            row["rust_servfail_go_noerror"] = left["rcode"] == 0 and right["rcode"] == 2
            self.counts["rust_servfail_go_noerror"] += row["rust_servfail_go_noerror"]
        else:
            self.counts["incomplete_pairs"] += 1
            for side in ("go", "rust"):
                if side not in row:
                    self.counts[side + "_timeouts"] += 1
        if "audit" not in row:
            self.counts["audit_missing"] += 1
        row["different_fields"] = fields
        row["classification"] = classification
        row["finalized_reason"] = reason
        row["finalized_monotonic"] = time.monotonic()
        safety_reason = None
        if len(have) == 2:
            try:
                row["servfail_window"] = self.safety.pair(row["finalized_monotonic"], left["rcode"], right["rcode"])
            except SafetyError as error:
                safety_reason = str(error)
                row["safety_stop"] = safety_reason
        self.record(row)
        # The private row already owns any evidence it needs. Equal queries
        # need no trace after finalization, rather than another 20s of copies.
        self.traces.pop(row.get("audit", {}).get("trace_id"), None)
        if safety_reason:
            self.trigger_safety(safety_reason)

    def receive_trace(self, entries):
        for entry in entries:
            ident = entry.get("trace_id")
            if not ident:
                continue
            if ident not in self.traces:
                if len(self.traces) >= 4096:
                    self.traces.pop(next(iter(self.traces)))
                    self.counts["trace_query_evictions"] += 1
                self.traces[ident] = {"policy_valid": self.policy_valid and time.monotonic() >= self.trace_invalid_until,
                                      "wall": time.time(), "entries": []}
            trace = self.traces[ident]
            if len(trace["entries"]) >= 256:
                trace["policy_valid"] = False
                self.counts["trace_step_overflow"] += 1
                continue
            # The producer already projected this immutable event. Share it
            # until finalization instead of allocating a second dictionary.
            trace["entries"].append(entry)
        cutoff = time.time() - 20
        self.traces = {ident: trace for ident, trace in self.traces.items() if trace["wall"] >= cutoff}

    def trace_complete(self, row):
        trace = self.traces.get(row.get("audit", {}).get("trace_id"), {})
        entries = trace.get("entries", [])
        return entries and entries[-1].get("msg") == "dns query completed"

    def match_audit(self):
        for row in self.pending.values():
            if "audit" in row or "rust" not in row:
                continue
            matches = []
            for audit in self.audit:
                if audit["key"] == row["audit_key"] and row["sent_wall"] <= audit["wall"] <= row["received_wall"]:
                    matches.append(audit)
            if len(matches) == 1:
                row["audit"] = matches[0]["record"]
                self.counts["audit_correlated"] += 1
            elif len(matches) > 1:
                row["audit_ambiguous"] = True

    def receive_audit(self, entries):
        for entry in reversed(entries):
            ident = entry.get("trace_id")
            if not ident or ident in self.audit_ids:
                continue
            self.audit_ids.add(ident)
            self.audit_order.append(ident)
            if len(self.audit_order) > 4096:
                self.audit_ids.discard(self.audit_order.popleft())
            try:
                address = ipaddress.ip_address(entry["client_ip"])
                address = getattr(address, "ipv4_mapped", None) or address
                name = entry["query_name"].rstrip(".").lower().encode("ascii").hex()
                wall = datetime.datetime.fromisoformat(entry["query_time"].replace("Z", "+00:00")).timestamp()
                kind = entry["query_type"]
                upstream = entry.get("final_upstream", "")
                aliases = self.aliases.get(upstream, set())
                group = next(iter(aliases)) if len(aliases) == 1 else "" if not upstream else "unmapped_or_ambiguous"
                record = {k: entry.get(k) for k in ("trace_id", "query_time", "effective_tag", "domain_set", "final_sequence", "final_upstream", "selected_upstream", "upstream_diagnostics")}
                record["final_upstream_group"] = group
                self.audit.append({"key": (str(address), name, kind), "wall": wall, "record": record})
            except (ValueError, KeyError, UnicodeError):
                self.counts["audit_parse_errors"] += 1
        # Only in-flight queries can need these audit projections. Keep recent
        # transport windows, rather than a growing history of completed DNS.
        cutoff = time.time() - 20
        while self.audit and self.audit[0]["wall"] < cutoff:
            self.audit.popleft()

    def sample(self, phase):
        props = dict(line.split("=", 1) for line in subprocess.check_output(["systemctl", "show", "mosdns.service",
            "-p", "MainPID", "-p", "NRestarts", "-p", "ActiveState"], text=True).splitlines())
        data = {"wall": time.time(), "monotonic": time.monotonic(), "phase": phase, "service": props,
                "counts": dict(self.counts), "pending": len(self.pending), "event_queue": self.events.qsize(),
                "guard": counters(), "resources": {}}
        for name, pid in (("rust", int(props["MainPID"])), ("replay_collector", os.getpid()),
                          ("worker", self.child.pid if self.child else 0), ("capture_ssh", self.capture.pid if self.capture else 0)):
            if pid:
                try:
                    data["resources"][name] = resources(pid)
                except (OSError, ValueError):
                    data["resources"][name] = {"pid": pid, "unavailable": True}
        latencies, self.latencies = sorted(self.latencies), []
        data["latencies_ms"] = {"count": len(latencies), **{f"p{p}": latencies[math.ceil(len(latencies) * p / 100) - 1] if latencies else None for p in (50, 95, 99)}}
        with (self.root / "resources.jsonl").open("a") as stream:
            stream.write(json.dumps(data) + "\n")
        summary = {"started": self.started, "updated": time.time(), "phase": phase, "counts": dict(self.counts),
                   "guard": data["guard"], "last_resources": data["resources"], "safety_stop_reason": self.safety_stop_reason}
        temp = self.root / "summary.tmp"
        temp.write_text(json.dumps(summary, indent=2) + "\n")
        temp.replace(self.root / "summary.json")
        if any(data["guard"]):
            self.trigger_safety("physical leak guard nonzero")
        tool_rss = sum(r.get("rss_kib", 0) or 0 for name, r in data["resources"].items() if name != "rust")
        if phase == "smoke" and tool_rss > TOOL_RSS_BUDGET_MIB * 1024:
            self.trigger_safety("tool RSS exceeds 96 MiB")

    def run(self):
        if self.root.stat().st_mode & 0o077:
            raise RuntimeError("private output directory must be 0700")
        probe = json.loads((self.args.network / "isolation-probe.json").read_text())
        if not probe.get("answer") or any(probe["guard_after"]) or any(counters()):
            raise RuntimeError("isolation proof missing/failed")
        if not api(self.config["api"], "/api/v1/audit/status").get("capturing"):
            raise RuntimeError("audit must already be enabled")
        manifest = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in pathlib.Path(__file__).parent.glob("*.py")}
        (self.root / "tool-hashes.json").write_text(json.dumps(manifest, indent=2) + "\n")
        signal.signal(signal.SIGTERM, lambda *_: self.stop.set())
        signal.signal(signal.SIGINT, lambda *_: self.stop.set())
        api(self.config["api"], "/api/v1/capture/start", {"duration_seconds": 600}, raw=True)
        self.trace_renewed = time.monotonic()
        self.counts["trace_windows"] += 1
        self.state_counts.sample(time.time())
        self.sample("idle_collection_baseline")
        health_thread = threading.Thread(target=self.health_loop, daemon=True)
        health_thread.start()
        if self.stop.wait(self.args.idle_seconds):
            health_thread.join(timeout=5)
            api(self.config["api"], "/api/v1/capture/start", {"duration_seconds": 1}, raw=True)
            self.sample("stopped")
            return
        self.sample("idle_collection_baseline_end")
        self.child = worker(self.args.network)
        if not json.loads(self.child.stdout.readline()).get("ready"):
            raise RuntimeError("worker startup failed")
        threads = [health_thread] + [threading.Thread(target=fn, daemon=True) for fn in (self.worker_loop, self.capture_loop, self.audit_loop)]
        self.safety.started = time.monotonic()
        for thread in threads[1:]:
            thread.start()
        deadline = time.monotonic() + self.args.seconds
        next_sample = time.monotonic() + 60
        next_counts = time.monotonic() + 600
        token, next_send = 0, time.monotonic()
        try:
            while not self.stop.is_set() and time.monotonic() < deadline + 20:
                if time.monotonic() >= deadline:
                    self.capture_done.set()
                    if self.capture and self.capture.poll() is None:
                        self.capture.terminate()
                    if not self.pending and self.events.empty():
                        break
                try:
                    timeout = min(.1, max(0, next_send - time.monotonic())) if self.send_queue else .1
                    kind, value = self.events.get(timeout=timeout)
                except queue.Empty:
                    kind, value = None, None
                if kind == "audit":
                    self.receive_audit(value)
                elif kind == "trace":
                    self.receive_trace(value)
                elif kind in ("trace_gap", "policy_changed"):
                    for trace in self.traces.values():
                        trace["policy_valid"] = False
                    if kind == "policy_changed":
                        if self.policy_valid:
                            self.counts["learned_policy_changed"] += 1
                        self.policy_valid = False
                    else:
                        self.trace_invalid_until = time.monotonic() + 20
                elif kind == "policy_restored":
                    # Unattended events may restore every frozen input byte.
                    # Old/in-flight evidence remains invalid through a drain.
                    self.policy_valid = True
                    self.trace_invalid_until = time.monotonic() + 20
                    self.counts["learned_policy_restored"] += 1
                elif kind == "worker_exit":
                    raise RuntimeError("namespace worker exited")
                elif kind == "rust":
                    if "sent" in value:
                        self.counts["worker_sent"] += 1
                    elif "frame_error" in value:
                        self.counts["worker_frame_errors"] += 1
                        self.counts["worker_frame_error_" + value["frame_error"]] += 1
                    elif "packet_statistics" in value:
                        self.counts["worker_kernel_packets"] += value["packet_statistics"]["packets"]
                        self.counts["worker_kernel_dropped"] += value["packet_statistics"]["dropped"]
                    elif "wire" in value:
                        self.handle_packet("rust", time.time(), value["source"], value["port"], bytes.fromhex(value["wire"]), value["monotonic"])
                elif kind in ("query", "go"):
                    timestamp, source, port, wire = value
                    if kind == "query":
                        try:
                            dns = decode_dns(wire)
                            if dns["response"]:
                                raise ValueError("QR on query path")
                            address = ipaddress.ip_address(source)
                            if address.is_loopback or address.is_multicast or address.is_unspecified or source == self.config["target"]:
                                raise ValueError("ineligible source")
                        except ValueError:
                            self.counts["query_parse_or_source_errors"] += 1
                            continue
                        key = (source, port, dns["id"], dns["questions"])
                        self.counts["client_queries"] += 1
                        if key in self.pending:
                            field = "query_retransmissions" if self.pending[key]["query_wire"] == wire.hex() else "query_key_conflicts"
                            self.counts[field] += 1
                            continue
                        if len(self.pending) >= 512:
                            self.counts["rate_or_inflight_dropped"] += 1
                            continue
                        token += 1
                        qname, qtype, _ = dns["questions"][0]
                        qlabel = {1: "A", 28: "AAAA", 65: "HTTPS", 5: "CNAME", 12: "PTR", 16: "TXT", 15: "MX", 2: "NS", 6: "SOA", 64: "SVCB"}.get(qtype, "TYPE" + str(qtype))
                        self.pending[key] = {"token": token, "capture_wall": timestamp, "source": source, "port": port, "query_wire": wire.hex(),
                            "enqueued_monotonic": time.monotonic(), "audit_key": (source, qname, qlabel)}
                        self.send_queue.append((key, {"token": token, "source": source, "port": port, "wire": wire.hex()}))
                    else:
                        self.handle_packet("go", timestamp, source, port, wire, None)
                if kind == "audit":
                    self.match_audit()
                now = time.monotonic()
                if self.send_queue and now >= next_send:
                    send_key, command = self.send_queue.popleft()
                    self.pending[send_key].update(sent_wall=time.time(), sent_monotonic=now)
                    self.child.stdin.write(json.dumps(command) + "\n")
                    self.child.stdin.flush()
                    self.counts["replayed"] += 1
                    next_send = now + 1 / self.args.qps
                for key, row in list(self.pending.items()):
                    if all(side in row for side in ("go", "rust", "audit")) and self.trace_complete(row):
                        self.finish(key)
                    elif "sent_monotonic" in row and now - row["sent_monotonic"] > 15:
                        self.finish(key, "deadline")
                if now >= next_sample:
                    self.sample("smoke")
                    next_sample += 60
                if now >= next_counts:
                    try:
                        self.state_counts.sample(time.time())
                    except RuntimeError as error:
                        self.trigger_safety(str(error))
                    next_counts += 600
            self.sample("capture_finished")
        finally:
            self.stop.set()
            if self.capture and self.capture.poll() is None:
                self.capture.terminate()
            if self.child:
                self.child.stdin.close()
                try:
                    self.child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.child.terminate()
                    self.child.wait(timeout=3)
            for thread in threads:
                thread.join(timeout=4)
            for key in list(self.pending):
                self.finish(key, "stop_boundary")
            try:
                self.state_counts.sample(time.time())
            except RuntimeError as error:
                self.trigger_safety(str(error))
            try:
                api(self.config["api"], "/api/v1/capture/start", {"duration_seconds": 1}, raw=True)
            except (OSError, ValueError):
                self.counts["trace_cleanup_errors"] += 1
            self.sample("stopped")
            self.pairs_file.close()

    def handle_packet(self, side, wall, source, port, wire, monotonic):
        try:
            dns = decode_dns(wire)
            if not dns["response"]:
                raise ValueError("missing QR on response")
        except ValueError:
            self.counts[side + "_parse_errors"] += 1
            return
        key = (source, port, dns["id"], dns["questions"])
        row = self.pending.get(key)
        if row is None:
            self.counts[side + "_orphan_responses"] += 1
        elif side in row:
            self.counts[side + "_duplicate_responses"] += 1
        else:
            row[side] = dns
            row[side + "_wire"] = wire.hex()
            row[side + "_wall"] = wall
            self.counts[side + "_responses"] += 1
            if side == "rust":
                latency = (monotonic - row["sent_monotonic"]) * 1000
                row["received_wall"] = row["sent_wall"] + latency / 1000
                row["latency_ms"] = latency
                if len(self.latencies) < self.args.qps * 65:
                    self.latencies.append(latency)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=pathlib.Path)
    parser.add_argument("--network", required=True, type=pathlib.Path)
    parser.add_argument("--config", required=True, type=pathlib.Path)
    parser.add_argument("--seconds", type=int, default=3600)
    parser.add_argument("--idle-seconds", type=int, default=60)
    parser.add_argument("--qps", type=int, default=100)
    Experiment(parser.parse_args()).run()
