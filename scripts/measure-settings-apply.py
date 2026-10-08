#!/usr/bin/env python3
"""Cost of a settings hot apply on the production rule set (native host only).

Builds a configuration from a production snapshot (outside the repository):
every plugin of `sub_config/rule_set.yaml` except `rewrite`, those of
`sub_config/adguard.yaml`, a `socks5_proxy`, an `aliapi` group in front of a
local UDP stub, one sequence (`unified_matcher1`, then the group) and the API.
Then, on Linux: start, read VmRSS (start-up steady) and VmHWM; with a paced UDP
client running, POST /api/v1/overrides (a new proxy address) --applies times,
each time recording how long the API took, the longest query during it, and the
VmHWM/VmRSS afterwards (HWM reset before each apply); a last reading after
--final-wait seconds. Prints one JSON object (kB for memory, ms for time).

Usage:
  measure-settings-apply.py --binary ./mosdns --snapshot DIR --work DIR [--applies 4] [--qps 200]
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

STUB_PORT = 29054

SEQUENCE = """
  - tag: main_proxy
    type: socks5_proxy
    args: {{addr: "127.0.0.1:19"}}
  - tag: foreign
    type: aliapi
    args:
      proxy: $main_proxy
      upstreams:
        - {{tag: stub, addr: "udp://127.0.0.1:{stub}"}}
  - tag: entry
    type: sequence
    args:
      - exec: $unified_matcher1
      - exec: $foreign
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
    config += "\n".join(blocks) + "\n" + SEQUENCE.format(dns=dns, stub=STUB_PORT)
    with open(os.path.join(work, "config.yaml"), "w", encoding="utf-8") as f:
        f.write(config)


def stub():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", STUB_PORT))
    while True:
        q, a = s.recvfrom(4096)
        i = 12
        while q[i]:
            i += 1 + q[i]
        end = i + 5
        r = q[:2] + b"\x81\x80\x00\x01\x00\x01\x00\x00\x00\x00" + q[12:end] + b"\xc0\x0c" + struct.pack(">HHIH", 1, 1, 60, 4) + bytes([192, 0, 2, 1])
        s.sendto(r, a)


def packet(name, qid):
    labels = b"".join(bytes([len(p)]) + p.encode() for p in name.strip(".").split(".") if p)
    return struct.pack(">HHHHHH", qid, 0x0100, 1, 0, 0, 0) + labels + b"\x00\x00\x01\x00\x01"


def status(pid):
    values = {}
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            key, _, rest = line.partition(":")
            if key in ("VmRSS", "VmHWM"):
                values[key] = int(rest.split()[0])
    return values


class Load(threading.Thread):
    def __init__(self, port, qps):
        super().__init__(daemon=True)
        self.port, self.qps = port, qps
        self.stop = threading.Event()
        self.samples = []  # (monotonic send time, latency; 2.0 = no answer)

    def run(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.settimeout(2.0)
        sock.connect(("127.0.0.1", self.port))
        interval, nxt, i = 1.0 / self.qps, time.perf_counter(), 0
        while not self.stop.is_set():
            start = time.perf_counter()
            sent = time.monotonic()
            try:
                sock.send(packet("load%d.example" % (i % 5000), i & 0xFFFF))
                while True:
                    data = sock.recv(4096)
                    if struct.unpack(">H", data[:2])[0] == (i & 0xFFFF):
                        break
                self.samples.append((sent, time.perf_counter() - start))
            except socket.timeout:
                self.samples.append((sent, 2.0))
            i += 1
            nxt += interval
            delay = nxt - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
            else:
                nxt = time.perf_counter()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--snapshot", required=True)
    parser.add_argument("--work", required=True)
    parser.add_argument("--applies", type=int, default=4)
    parser.add_argument("--qps", type=float, default=200.0)
    parser.add_argument("--settle", type=float, default=5.0)
    parser.add_argument("--final-wait", type=float, default=20.0)
    parser.add_argument("--dns-port", type=int, default=25353)
    parser.add_argument("--api-port", type=int, default=29099)
    args = parser.parse_args()
    work = os.path.abspath(args.work)
    prepare(os.path.abspath(args.snapshot), work, args.dns_port, args.api_port)
    threading.Thread(target=stub, daemon=True).start()
    proc = subprocess.Popen([os.path.abspath(args.binary), "start", "-d", work, "-c", os.path.join(work, "config.yaml")],
                            cwd=work, stdout=subprocess.DEVNULL, stderr=open(os.path.join(work, "stderr.log"), "w"))
    result = {"qps": args.qps}
    load = None
    try:
        probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        probe.settimeout(0.3)
        probe.connect(("127.0.0.1", args.dns_port))
        deadline = time.monotonic() + 120
        while True:
            if proc.poll() is not None:
                raise SystemExit(f"process exited ({proc.returncode})")
            try:
                probe.send(packet("example.invalid", 1))
                probe.recv(4096)
                break
            except OSError:
                if time.monotonic() > deadline:
                    raise SystemExit("DNS never answered")
        time.sleep(args.settle)
        result["start"] = status(proc.pid)
        load = Load(args.dns_port, args.qps)
        load.start()
        time.sleep(2.0)
        result["applies"] = []
        for n in range(args.applies):
            with open(f"/proc/{proc.pid}/clear_refs", "w") as f:
                f.write("5")
            body = json.dumps({"socks5": "127.0.0.1:%d" % (20 + n), "ecs": "", "replacements": []}).encode()
            began_wall, began = time.monotonic(), time.perf_counter()
            request = urllib.request.Request(f"http://127.0.0.1:{args.api_port}/api/v1/overrides", data=body,
                                             headers={"Content-Type": "application/json"}, method="POST")
            urllib.request.urlopen(request, timeout=120).read()
            took = time.perf_counter() - began
            ended = time.monotonic()
            time.sleep(args.settle)
            window = [lat for t, lat in load.samples if began_wall - 0.5 <= t <= ended + 0.5]
            lost_at = [round(t - began_wall, 3) for t, lat in load.samples if lat >= 2.0 and began_wall - 0.5 <= t <= ended + 0.5]
            result["applies"].append({"n": n + 1, "api_ms": round(took * 1000), "max_query_ms_during": round(max(window, default=0) * 1000, 1),
                                      "queries_during": len(window), "timeouts_during": len(lost_at), "lost_sent_at_s_after_post": lost_at, "api_s": round(ended - began_wall, 3),
                                      **status(proc.pid)})
        time.sleep(args.final_wait)
        result["final"] = status(proc.pid)
        load.stop.set()
        load.join()
        all_latencies = sorted(lat for _, lat in load.samples)
        result["all_queries"] = {"count": len(all_latencies), "p99_ms": round(all_latencies[int(0.99 * len(all_latencies))] * 1000, 2),
                                 "max_ms": round(all_latencies[-1] * 1000, 1), "timeouts": sum(1 for lat in all_latencies if lat >= 2.0)}
    finally:
        if load is not None:
            load.stop.set()
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=60)
        except subprocess.TimeoutExpired:
            proc.kill()
    print(json.dumps(result))


if __name__ == "__main__":
    sys.exit(main())
