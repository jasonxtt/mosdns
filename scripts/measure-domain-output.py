#!/usr/bin/env python3
"""Repeatable `domain_output` resource measurement for the native host.

Builds a configuration from a production snapshot (outside the repository):
every plugin of `sub_config/rule_set.yaml` except `rewrite`, those of
`sub_config/adguard.yaml`, the eight `domain_output` plugins of
`sub_config/domain_output.yaml` (unless --without-outputs), one sequence that
runs `unified_matcher1` and then every output, a UDP listener, and the API
on --api-port, to which the snapshot's `domain_set_url` values (port 9099) are
rewritten, so the rule publication is the in-process path.

On Linux: start, wait for DNS, read VmRSS (start-up steady); send --names
distinct queries (the snapshot's `gen/top_domains.txt` names first, then
synthetic ones) and read VmRSS and the throughput; run `/save` of every
output while a paced client measures latency (publication rebuilds the
mapper); read VmRSS/VmHWM after --settle seconds and after --final-wait.
Finally stop with SIGTERM and report the lines of the statistics files.
Prints one JSON object. Readings are kB from /proc/<pid>/status.

Usage:
  measure-domain-output.py --binary ./mosdns --snapshot DIR --work DIR \
      [--names 100000] [--qps 200] [--without-outputs]
"""

import argparse
import json
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.request

OUTPUTS = ["top_domains", "my_sv4list", "my_realiplist", "my_fakeiplist", "my_nov6list", "my_nov4list",
           "my_nodenov6list", "my_nodenov4list"]

SEQUENCE = """
  - tag: unused_forward
    type: forward
    args: {{upstreams: [{{addr: "udp://127.0.0.1:9"}}]}}
  - tag: entry
    type: sequence
    args:
      - exec: $unified_matcher1
{execs}      - exec: reject 0
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: "127.0.0.1:{dns}"}}
"""


def plugin_blocks(text):
    blocks, current = [], None
    for line in text.splitlines():
        if line.startswith("  - tag:"):
            current = [line]
            blocks.append(current)
        elif current is not None and not line.startswith("#"):
            current.append(line)
    return ["\n".join(block).rstrip() for block in blocks]


def prepare(snapshot, work, dns, api, outputs):
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    for name in ("rule", "gen", "srs", "adguard"):
        shutil.copytree(os.path.join(snapshot, name), os.path.join(work, name))
    files = ["adguard.yaml", "rule_set.yaml"] + (["domain_output.yaml"] if outputs else [])
    blocks = []
    for name in files:
        with open(os.path.join(snapshot, "sub_config", name), encoding="utf-8") as f:
            blocks += [b for b in plugin_blocks(f.read()) if b.split("\n")[0].split()[-1] != "rewrite"]
    blocks = [b.replace("127.0.0.1:9099", "127.0.0.1:%d" % api) for b in blocks]
    execs = "".join(f"      - exec: ${tag}\n" for tag in OUTPUTS) if outputs else ""
    config = 'log: {level: error}\napi: {http: "127.0.0.1:%d"}\nplugins:\n' % api
    config += "\n".join(blocks) + "\n" + SEQUENCE.format(dns=dns, execs=execs)
    with open(os.path.join(work, "config.yaml"), "w", encoding="utf-8") as f:
        f.write(config)


def packet(name, qid):
    labels = b"".join(bytes([len(p)]) + p.encode() for p in name.strip(".").split(".") if p)
    return struct.pack(">HHHHHH", qid, 0x0100, 1, 0, 0, 0) + labels + b"\x00\x00\x01\x00\x01"


def query(sock, name, qid):
    sock.send(packet(name, qid))
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
            if key in ("VmRSS", "VmHWM", "RssAnon"):
                values[key] = int(rest.split()[0])
    return values


class Load(threading.Thread):
    def __init__(self, port, qps, names):
        super().__init__(daemon=True)
        self.port, self.qps, self.names = port, qps, names
        self.stop = threading.Event()
        self.samples, self.timeouts = [], 0

    def run(self):
        sock = dns_socket(self.port, 1.0)
        interval, nxt, i = 1.0 / self.qps, time.perf_counter(), 0
        while not self.stop.is_set():
            start = time.perf_counter()
            try:
                query(sock, self.names[i % len(self.names)], i & 0xFFFF)
                self.samples.append(time.perf_counter() - start)
            except socket.timeout:
                self.timeouts += 1
            i += 1
            nxt += interval
            delay = nxt - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
            else:
                nxt = time.perf_counter()

    def summary(self):
        samples = sorted(self.samples)
        if not samples:
            return {"queries": 0}
        pick = lambda q: round(samples[min(len(samples) - 1, int(q * len(samples)))] * 1e6)
        return {"queries": len(samples), "p50_us": pick(0.5), "p99_us": pick(0.99),
                "max_us": round(samples[-1] * 1e6), "timeouts": self.timeouts}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--snapshot", required=True)
    parser.add_argument("--work", required=True)
    parser.add_argument("--names", type=int, default=100000)
    parser.add_argument("--qps", type=float, default=200.0)
    parser.add_argument("--settle", type=float, default=5.0)
    parser.add_argument("--final-wait", type=float, default=30.0)
    parser.add_argument("--dns-port", type=int, default=25353)
    parser.add_argument("--api-port", type=int, default=29099)
    parser.add_argument("--without-outputs", action="store_true")
    args = parser.parse_args()

    work = os.path.abspath(args.work)
    prepare(os.path.abspath(args.snapshot), work, args.dns_port, args.api_port, not args.without_outputs)
    with open(os.path.join(work, "gen", "top_domains.txt"), encoding="utf-8") as f:
        real = [line.split()[-1].split("|")[0] for line in f if line.strip()]
    names = real + [f"h{i}.synthetic.example" for i in range(max(0, args.names - len(real)))]

    proc = subprocess.Popen([os.path.abspath(args.binary), "start", "-d", work, "-c", os.path.join(work, "config.yaml")],
                            cwd=work, stdout=subprocess.DEVNULL, stderr=open(os.path.join(work, "stderr.log"), "w"))
    result = {"outputs": not args.without_outputs, "names": len(names), "qps": args.qps}
    load = None
    try:
        probe = dns_socket(args.dns_port, 0.2)
        deadline = time.monotonic() + 120
        while True:
            if proc.poll() is not None:
                raise SystemExit(f"process exited ({proc.returncode})")
            try:
                query(probe, "example.invalid", 1)
                break
            except OSError:
                if time.monotonic() > deadline:
                    raise SystemExit("DNS never answered")
        time.sleep(args.settle)
        result["start"] = status(proc.pid)

        sock = dns_socket(args.dns_port, 2.0)
        began = time.perf_counter()
        for i, name in enumerate(names):
            query(sock, name, i & 0xFFFF)
        elapsed = time.perf_counter() - began
        result["recording"] = {"queries": len(names), "seconds": round(elapsed, 2),
                               "qps": round(len(names) / elapsed)}
        time.sleep(args.settle)
        result["after_recording"] = status(proc.pid)

        if not args.without_outputs:
            load = Load(args.dns_port, args.qps, real)
            load.start()
            time.sleep(1.0)
            with open(f"/proc/{proc.pid}/clear_refs", "w") as f:
                f.write("5")
            saves = {}
            for tag in OUTPUTS:
                began = time.perf_counter()
                urllib.request.urlopen(f"http://127.0.0.1:{args.api_port}/plugins/{tag}/save", timeout=60).read()
                saves[tag] = round(time.perf_counter() - began, 3)
            result["save_seconds"] = saves
            time.sleep(args.settle)
            load.stop.set()
            load.join()
            result["load_during_saves"] = load.summary()
            result["after_saves"] = status(proc.pid)
        time.sleep(args.final_wait)
        result["final"] = status(proc.pid)
    finally:
        if load is not None:
            load.stop.set()
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=60)
        except subprocess.TimeoutExpired:
            proc.kill()
    if not args.without_outputs:
        lines = {}
        for tag in OUTPUTS:
            path = os.path.join(work, "gen", tag.removeprefix("my_") + ".txt")
            if os.path.exists(path):
                with open(path, encoding="utf-8") as f:
                    lines[tag] = sum(1 for _ in f)
        result["stat_file_lines_after_exit"] = lines
        result["exit_code"] = proc.returncode
    print(json.dumps(result))


if __name__ == "__main__":
    sys.exit(main())
