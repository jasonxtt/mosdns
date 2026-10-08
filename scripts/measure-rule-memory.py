#!/usr/bin/env python3
"""Repeatable rule-set memory measurement for the native host and Go mosdns.

Builds a configuration from a production snapshot (outside the repository):
every plugin of `sub_config/rule_set.yaml` except `rewrite`, the plugins of
`sub_config/adguard.yaml`, one sequence that runs `unified_matcher1`, a UDP
listener and the API. Then, on Linux:

  1. starts the binary and waits until DNS answers; after --settle seconds
     reads VmRSS (start-up steady) and VmHWM (start-up peak);
  2. --rebuilds times: resets VmHWM (clear_refs=5), posts the blocklist plus
     one new `full:` rule (rebuilds `unified_matcher1`), waits until a DNS probe
     for the new rule is answered as blocked, waits --settle seconds and reads
     VmRSS/VmHWM;
  3. waits --final-wait seconds and reads VmRSS once more;
  4. queries every name of `gen/top_domains.txt` once and reports a sha256 of
     the `name rcode` lines (must be identical between builds that answer
     identically; the sequence maps the mapper's marks to distinct rcodes).

With --qps > 0 a paced UDP client (one query outstanding) queries those names
during steps 1-3 and reports latency percentiles, overall and while a rebuild
runs. Readings are kB from /proc/<pid>/status.

Usage:
  measure-rule-memory.py --binary ./mosdns --snapshot DIR --work DIR \
      [--label NAME] [--rebuilds 6] [--settle 5] [--final-wait 30] [--qps 0] \
      [--env KEY=VALUE ...]
The work directory is deleted and recreated. Prints one JSON object.
"""

import argparse
import hashlib
import itertools
import json
import os
import random
import shutil
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.request

SEQUENCE = """
  - tag: unused_forward
    type: forward
    args: {{upstreams: [{{addr: "udp://127.0.0.1:9"}}]}}
  - tag: entry
    type: sequence
    args:
      - exec: $unified_matcher1
      - matches: fast_mark 1
        exec: reject 3
      - matches: fast_mark 5
        exec: reject 5
      - matches: fast_mark 16
        exec: reject 2
      - matches: fast_mark 14
        exec: reject 4
      - matches: fast_mark 17
        exec: reject 0
      - exec: reject 9
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: "127.0.0.1:{dns}"}}
"""


def plugin_blocks(text):
    """Splits a `plugins:` YAML file into its `  - tag:` blocks (text level)."""
    blocks, current = [], None
    for line in text.splitlines():
        if line.startswith("  - tag:"):
            current = [line]
            blocks.append(current)
        elif current is not None and not line.startswith("#"):
            current.append(line)
    return ["\n".join(block).rstrip() for block in blocks]


def prepare(snapshot, work, dns, api):
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    for name in ("rule", "gen", "srs", "adguard"):
        shutil.copytree(os.path.join(snapshot, name), os.path.join(work, name))
    blocks = []
    for name in ("adguard.yaml", "rule_set.yaml"):
        with open(os.path.join(snapshot, "sub_config", name), encoding="utf-8") as f:
            blocks += [b for b in plugin_blocks(f.read()) if b.split("\n")[0].split()[-1] != "rewrite"]
    config = 'log: {level: error}\napi: {http: "127.0.0.1:%d"}\nplugins:\n' % api
    config += "\n".join(blocks) + "\n" + SEQUENCE.format(dns=dns)
    with open(os.path.join(work, "config.yaml"), "w", encoding="utf-8") as f:
        f.write(config)


def query(sock, name, qid):
    labels = b"".join(bytes([len(p)]) + p.encode() for p in name.strip(".").split(".") if p)
    sock.send(struct.pack(">HHHHHH", qid, 0x0100, 1, 0, 0, 0) + labels + b"\x00\x00\x01\x00\x01")
    while True:
        data = sock.recv(4096)
        if len(data) >= 4 and struct.unpack(">H", data[:2])[0] == qid:
            return data[3] & 0x0F


def dns_socket(port, timeout):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(timeout)
    sock.connect(("127.0.0.1", port))
    return sock


def status(pid):
    values = {}
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            key, _, rest = line.partition(":")
            if key in ("VmRSS", "VmHWM", "RssAnon", "Threads"):
                values[key] = int(rest.split()[0])
    return values


def reset_hwm(pid):
    with open(f"/proc/{pid}/clear_refs", "w") as f:
        f.write("5")


class Load(threading.Thread):
    """One query outstanding at a time, paced to `qps`; records latencies."""

    def __init__(self, port, qps, names):
        super().__init__(daemon=True)
        self.port, self.qps, self.names = port, qps, names
        self.stop = threading.Event()
        self.samples, self.rcodes, self.timeouts = [], {}, 0

    def run(self):
        sock = dns_socket(self.port, 1.0)
        interval, nxt, i = 1.0 / self.qps, time.perf_counter(), 0
        while not self.stop.is_set():
            name = self.names[i % len(self.names)]
            start = time.perf_counter()
            try:
                rcode = query(sock, name, i & 0xFFFF)
                self.samples.append((time.monotonic(), time.perf_counter() - start))
                self.rcodes[rcode] = self.rcodes.get(rcode, 0) + 1
            except socket.timeout:
                self.timeouts += 1
            i += 1
            nxt += interval
            delay = nxt - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
            else:
                nxt = time.perf_counter()

    def summary(self, windows):
        """Latency percentiles of all queries and of those sent while a
        rebuild was running (post to probe flip, plus one second)."""

        def stats(samples):
            samples = sorted(samples)
            if not samples:
                return {"queries": 0}
            pick = lambda q: round(samples[min(len(samples) - 1, int(q * len(samples)))] * 1e6)
            return {"queries": len(samples), "p50_us": pick(0.50), "p95_us": pick(0.95),
                    "p99_us": pick(0.99), "max_us": round(samples[-1] * 1e6)}

        during = [lat for t, lat in self.samples if any(a <= t <= b for a, b in windows)]
        return {
            "all": stats([lat for _, lat in self.samples]),
            "during_rebuild": stats(during),
            "timeouts": self.timeouts,
        }


def answers_digest(port, names):
    """Queries every distinct name once; sha256 of `name rcode` lines and the
    rcode histogram (identical for builds that answer identically)."""
    sock, digest, histogram = dns_socket(port, 2.0), hashlib.sha256(), {}
    for i, name in enumerate(sorted(set(names))):
        rcode = query(sock, name, i & 0xFFFF)
        digest.update(f"{name} {rcode}\n".encode())
        histogram[str(rcode)] = histogram.get(str(rcode), 0) + 1
    return {"names": len(set(names)), "sha256": digest.hexdigest(), "rcodes": histogram}


def disable_thp():
    import ctypes
    if ctypes.CDLL(None, use_errno=True).prctl(41, 1, 0, 0, 0) != 0:  # PR_SET_THP_DISABLE
        raise OSError(ctypes.get_errno(), "prctl(PR_SET_THP_DISABLE)")


def wait_for(predicate, timeout, what, proc):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise SystemExit(f"process exited ({proc.returncode}) while waiting for {what}")
        try:
            if predicate():
                return
        except OSError:
            pass
        time.sleep(0.05)
    raise SystemExit(f"timed out waiting for {what}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--snapshot", required=True)
    parser.add_argument("--work", required=True)
    parser.add_argument("--label", default="")
    parser.add_argument("--rebuilds", type=int, default=6)
    parser.add_argument("--settle", type=float, default=5.0)
    parser.add_argument("--final-wait", type=float, default=30.0)
    parser.add_argument("--qps", type=float, default=0.0)
    parser.add_argument("--dns-port", type=int, default=25353)
    parser.add_argument("--api-port", type=int, default=28080)
    parser.add_argument("--env", action="append", default=[])
    parser.add_argument("--no-thp", action="store_true",
                        help="start the binary with PR_SET_THP_DISABLE (diagnostic)")
    args = parser.parse_args()

    work = os.path.abspath(args.work)
    prepare(os.path.abspath(args.snapshot), work, args.dns_port, args.api_port)
    with open(os.path.join(work, "rule", "blocklist.txt"), encoding="utf-8") as f:
        blocklist = [line.strip() for line in f if line.strip() and not line.startswith("#")]
    with open(os.path.join(work, "gen", "top_domains.txt"), encoding="utf-8") as f:
        names = [line.split()[-1] for line in f if line.strip() and not line.startswith("#")]
    random.Random(1).shuffle(names)

    env = dict(os.environ)
    env.update(item.split("=", 1) for item in args.env)
    binary = os.path.abspath(args.binary)
    began = time.monotonic()
    proc = subprocess.Popen(
        [binary, "start", "-d", work, "-c", os.path.join(work, "config.yaml")],
        cwd=work, env=env, stdout=subprocess.DEVNULL, stderr=open(os.path.join(work, "stderr.log"), "w"),
        preexec_fn=disable_thp if args.no_thp else None,
    )
    result = {"label": args.label, "binary": binary, "env": args.env, "qps": args.qps, "no_thp": args.no_thp}
    load = None
    try:
        probe, qids = dns_socket(args.dns_port, 0.2), itertools.cycle(range(1, 0x10000))
        wait_for(lambda: query(probe, "example.invalid", next(qids)) is not None, 120, "DNS", proc)
        result["ready_s"] = round(time.monotonic() - began, 2)
        if args.qps > 0:
            load = Load(args.dns_port, args.qps, names)
            load.start()
        time.sleep(args.settle)
        result["start"] = status(proc.pid)
        result["rebuilds"] = []
        windows = []
        for i in range(1, args.rebuilds + 1):
            reset_hwm(proc.pid)
            marker = f"mem-probe-{i}-{random.randrange(1 << 30)}.example"
            if query(probe, marker, next(qids)) == 3:
                raise SystemExit(f"{marker} is already blocked; the probe cannot detect the rebuild")
            body = json.dumps({"values": blocklist + [f"full:{marker}"]}).encode()
            posted = time.monotonic()
            request = urllib.request.Request(
                f"http://127.0.0.1:{args.api_port}/plugins/blocklist/post", data=body,
                headers={"Content-Type": "application/json"}, method="POST",
            )
            urllib.request.urlopen(request, timeout=30).read()
            # A fresh id per probe: a late answer to an earlier probe never counts.
            wait_for(lambda: query(probe, marker, next(qids)) == 3, 120, f"rebuild {i}", proc)
            built = round(time.monotonic() - posted, 2)
            windows.append((posted, time.monotonic() + 1.0))
            time.sleep(args.settle)
            result["rebuilds"].append({"n": i, "rebuild_s": built, **status(proc.pid)})
        time.sleep(args.final_wait)
        result["final"] = status(proc.pid)
        if load is not None:
            load.stop.set()
            load.join()
            result["load"] = load.summary(windows)
        result["answers"] = answers_digest(args.dns_port, names)
    finally:
        if load is not None:
            load.stop.set()
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            proc.kill()
    print(json.dumps(result))


if __name__ == "__main__":
    sys.exit(main())
