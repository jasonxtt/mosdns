"""Synthetic schema-4 differential; all services run in loopback-only namespaces."""
import argparse
import asyncio
import json
import os
import pathlib
import re
import signal
import socket
import subprocess
import sys
import time
import urllib.request

from dnswire import answer, decode, question


def api(path, value=None, method=None):
    body = None if value is None else json.dumps(value).encode()
    request = urllib.request.Request("http://127.0.0.1:9099" + path, body, method=method,
                                     headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=5) as response:
        data = response.read()
        if not data:
            return None
        if response.headers.get_content_type() == "application/json":
            return json.loads(data)
        return data.decode("utf-8")


def ready():
    deadline = time.monotonic() + 30
    while True:
        try:
            api("/api/v1/special-groups")
            return
        except OSError:
            if time.monotonic() > deadline:
                raise RuntimeError("host readiness failed")
            time.sleep(.1)


async def mock(root):
    class UDP(asyncio.DatagramProtocol):
        def __init__(self, marker):
            self.marker = marker

        def connection_made(self, transport):
            self.transport = transport

        def datagram_received(self, wire, peer):
            self.transport.sendto(answer(wire, f"192.0.2.{self.marker}",
                                         f"2001:db8::{self.marker}"), peer)
    loop = asyncio.get_running_loop()
    transports = []
    for port, marker in [(15400, 10), (15401, 20)]:
        transport, _ = await loop.create_datagram_endpoint(lambda m=marker: UDP(m),
                                                          local_addr=("127.0.0.1", port))
        transports.append(transport)
    (root / "mock-ready").touch()
    await asyncio.Event().wait()


def probe(root, role, create):
    ready()
    if create:
        api("/api/v1/special-groups", {"name": "main", "upstreams": [
            {"tag": "owned", "enabled": True, "protocol": "udp",
             "addr": "127.0.0.1:15401", "use_socks_proxy": False}]})
        if role == "go":
            time.sleep(3)
        ready()
        api("/api/v1/special-groups", {"name": "port", "listen_port": 15354,
            "custom_port_only": True, "upstream_sources": [{"kind": "group", "plugin_tag": "normal"}]})
        if role == "go":
            time.sleep(3)
        ready()
        for slot, name in [(50, "main.example.com"), (51, "port.example.com")]:
            api(f"/plugins/special_manual_{slot}/post", {"values": ["full:" + name]})
        time.sleep(1)
    api("/api/v1/audit/start", {}, "POST")
    api("/api/v1/audit/clear", {}, "POST")
    for slot in [50, 51]:
        api(f"/plugins/cache_special_{slot}/flush")
    rows = []
    for run in [1, 2]:
        for port, name, marker in [(15353, "main.example.com", 20),
                                   (15353, "port.example.com", 10),
                                   (15354, "port.example.com", 10)]:
            for qtype in [1, 28]:
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
                    sock.settimeout(5)
                    ident = len(rows) + 1
                    sock.sendto(question(name, qtype, ident), ("127.0.0.1", port))
                    dns = decode(sock.recv(65535))
                    expected = f"192.0.2.{marker}" if qtype == 1 else f"2001:db8::{marker}"
                    assert dns["id"] == ident and dns["rcode"] == 0 and len(dns["answers"]) == 1, dns
                    assert dns["answers"][0][3] == expected, dns
                audit = None
                for _ in range(100):
                    entries = api("/api/v2/audit/logs?page=1&limit=1")["logs"]
                    if entries and entries[0]["query_name"].rstrip(".") == name and entries[0]["query_type"] == {1: "A", 28: "AAAA"}[qtype]:
                        audit = entries[0]
                        break
                    time.sleep(.01)
                assert audit is not None, "missing correlated audit"
                route = {key: audit.get(key) for key in ["domain_set", "effective_tag", "final_sequence", "final_upstream"]}
                rows.append({"run": run, "port": port, "name": name, "qtype": qtype,
                             "answers": [item[:4] for item in dns["answers"]], "route": route})
    (root / ("created.json" if create else "imported.json")).write_text(json.dumps(rows, indent=2))


def without_upstreams(data):
    # Only the explicitly permitted per-group upstream block is excluded.
    return re.sub(rb"(  - tag: special_upstream_\d+\n    type: aliapi\n    args:\n      concurrent: 2\n      upstreams:\n)(?:        .*\n)+", rb"\1", data)


def compare_rows(go, native):
    assert len(go) == len(native) == 12, "incomplete replay"
    differences = []
    for left, right in zip(go, native):
        assert {k: v for k, v in left.items() if k != "route"} == {k: v for k, v in right.items() if k != "route"}, "DNS/input mismatch"
        a, b = left["route"], right["route"]
        slot = 50 if left["name"] == "main.example.com" else 51 if left["port"] == 15354 else None
        expected_sequence = f"sequence_special_{slot}" if slot else "main"
        expected_upstream = f"special_upstream_{slot}" if slot else "normal"
        expected_label = "特殊上游50" if slot == 50 else "unmatched_rule"
        sentinel = lambda value: "unmatched_rule" if value in (None, "", "unmatched_rule") else value
        for key in ["domain_set", "effective_tag"]:
            assert sentinel(a[key]) == sentinel(b[key]) == expected_label, (key, left, right)
        assert b["final_sequence"] == expected_sequence, (left, right)
        aliases = {"special_50_0": "special_upstream_50", "special_51_0": "special_upstream_51"}
        assert aliases.get(b["final_upstream"], b["final_upstream"]) == expected_upstream, (left, right)
        cached = slot is not None and left["run"] == 2
        assert a["final_sequence"] == (None if cached else expected_sequence), (left, right)
        assert a["final_upstream"] == (None if cached else expected_upstream), (left, right)
        if a != b:
            differences.append({"input": {k: left[k] for k in ["run", "port", "name", "qtype"]},
                "go": a, "native": b,
                "classification": "empty unmatched sentinel; Go cached route fields absent; native actual cache sequence/supplier retained"})
    return differences


CONFIG = """log: {level: error}
api: {http: '127.0.0.1:9099'}
include: [sub_config/special_groups.yaml]
plugins:
  - tag: ddns
    type: domain_set
    args: {files: [rule/ddns.txt]}
  - tag: switch4
    type: switch4
    args: {initial_value: rule/switch4.txt}
  - tag: normal
    type: aliapi
    args: {upstreams: [{tag: original, addr: 'udp://127.0.0.1:15400'}]}
  - tag: main
    type: sequence
    args:
      - exec: $special_upstream_matcher
      - exec: $sequence_special
      - matches: qname $ddns
        exec: $normal
      - matches: qname $ddns
        exec: exit
      - exec: flow_setter group=normal sequence=main upstream=normal
      - exec: $normal
  - tag: server
    type: udp_server
    args: {listen: '127.0.0.1:15353', entry: main, enable_audit: true}
"""


def run(args):
    assert os.geteuid() == 0, "VM root required"
    args.work.mkdir(mode=0o700)
    script = pathlib.Path(__file__).resolve()
    service = subprocess.check_output(["systemctl", "show", "mosdns", "-p", "MainPID", "-p", "ActiveState"])
    owned, spaces, streams = [], [], []
    def command(argv):
        subprocess.run(list(map(str, argv)), check=True)
    def stop(process):
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
    def setup(role, binary):
        root = args.work / role
        for directory in ["webinfo", "sub_config", "rule", "srs", "cache", "tmp"]:
            (root / directory).mkdir(parents=True, exist_ok=True)
        (root / "config.yaml").write_text(CONFIG)
        (root / "webinfo/webui_port_settings.json").write_text('{"port":9099}')
        (root / "webinfo/special_upstream_groups.json").write_text("[]")
        (root / "rule/switch4.txt").write_text("A")
        (root / "rule/ddns.txt").write_text("full:main.example.com\n")
        (root / "sub_config/special_groups.yaml").write_text("plugins:\n  - tag: special_upstream_matcher\n    type: domain_mapper\n    args: {rules: []}\n  - tag: sequence_special\n    type: sequence\n    args: []\n")
        ns = f"m1e-{os.getpid()}-{role}"
        command(["ip", "netns", "add", ns]); spaces.append(ns)
        command(["ip", "-n", ns, "link", "set", "lo", "up"])
        assert not subprocess.check_output(["ip", "-n", ns, "route", "show"]).strip()
        def spawn(argv, log):
            stream = (root / log).open("w"); streams.append(stream)
            process = subprocess.Popen(["ip", "netns", "exec", ns, *map(str, argv)],
                stdout=stream, stderr=subprocess.STDOUT, start_new_session=True,
                env={key: value for key, value in os.environ.items() if "proxy" not in key.lower()})
            owned.append(process)
            return process
        mock_process = spawn([sys.executable, script, "mock", "--work", root], "mock.log")
        deadline = time.monotonic() + 10
        while not (root / "mock-ready").exists():
            assert mock_process.poll() is None and time.monotonic() < deadline, "mock readiness failed"
            time.sleep(.1)
        def host():
            return spawn(["unshare", "--mount", "--propagation", "private", "sh", "-c",
                'mount --bind "$1" /tmp; shift; exec "$@"', "m1e", root / "tmp", binary,
                "start", "-d", root, "-c", "config.yaml"], "host.log")
        return root, ns, host, mock_process
    try:
        roles = {}
        for role, binary in [("go", args.go), ("native", args.native)]:
            root, ns, host, mock_process = setup(role, binary)
            process = host()
            command(["ip", "netns", "exec", ns, sys.executable, script, "probe", "--work", root, "--role", role, "--create"])
            stop(process)
            roles[role] = (root, ns, host)
        go, native = (roles[key][0] for key in ["go", "native"])
        differences = compare_rows(json.loads((go / "created.json").read_text()), json.loads((native / "created.json").read_text()))
        (args.work / "audit-differences.json").write_text(json.dumps(differences, ensure_ascii=False, indent=2))
        assert without_upstreams((go / "sub_config/special_groups.yaml").read_bytes()) == without_upstreams((native / "sub_config/special_groups.yaml").read_bytes()), "schema-4 render mismatch"
        states = {key: (roles[key][0] / "webinfo/special_upstream_groups.json").read_bytes() for key in roles}
        for role, (root, ns, host) in roles.items():
            other = "native" if role == "go" else "go"
            (root / "webinfo/special_upstream_groups.json").write_bytes(states[other])
            process = host()
            command(["ip", "netns", "exec", ns, sys.executable, script, "probe", "--work", root, "--role", role])
            if args.browser:
                command(["ip", "netns", "exec", ns, "node", args.browser, root, role])
            stop(process)
            assert json.loads((root / "imported.json").read_text()) == json.loads((root / "created.json").read_text()), "state import behavior mismatch"
        (args.work / "result.json").write_text(json.dumps({"dns_audit_rows_each": 12, "raw_audit_different_rows": len(differences), "classified_audit_rows": 12, "render_equal_except_upstreams": True, "state_bidirectional": True}, indent=2))
    finally:
        for process in reversed(owned):
            stop(process)
        for ns in reversed(spaces):
            assert not subprocess.check_output(["ip", "netns", "pids", ns]).strip(), "process leaked"
            command(["ip", "netns", "delete", ns])
        for stream in streams:
            stream.close()
        after = subprocess.check_output(["systemctl", "show", "mosdns", "-p", "MainPID", "-p", "ActiveState"])
        (args.work / "cleanup.json").write_text(json.dumps({"processes_exited": all(p.poll() is not None for p in owned), "namespaces_deleted": spaces, "original_service_unchanged": service == after}, indent=2))
        assert service == after, "original service changed"


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["run", "mock", "probe"])
    for flag in ["work", "go", "native", "browser"]:
        parser.add_argument("--" + flag, type=lambda s: pathlib.Path(s).resolve(), required=flag == "work")
    parser.add_argument("--role", choices=["go", "native"])
    parser.add_argument("--create", action="store_true")
    args = parser.parse_args()
    if args.mode == "mock":
        asyncio.run(mock(args.work))
    elif args.mode == "probe":
        probe(args.work, args.role, args.create)
    else:
        run(args)
