"""Namespace-local sequential replay with immediately correlated audit rows."""
import argparse
import json
import ipaddress
import pathlib
import socket
import time
import urllib.request

from dnswire import decode, question

API_PORT = 9099

def api(path, post=False):
    request = urllib.request.Request(f"http://127.0.0.1:{API_PORT}" + path, data=b"{}" if post else None, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=10) as response:
        return response.read()


def resources(pid):
    status = pathlib.Path(f"/proc/{pid}/status").read_text()
    rss = int(next(line.split()[1] for line in status.splitlines() if line.startswith("VmRSS:")))
    stat = pathlib.Path(f"/proc/{pid}/stat").read_text().split(")", 1)[1].split()
    import os
    return {"rss_kib": rss, "cpu_s": (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK")}


def run(root, corpus_path, pid):
    corpus = json.loads(corpus_path.read_text())
    audit_ports = set(json.loads((corpus_path.parent / "audit-ports.json").read_text()))
    deadline = time.monotonic() + 60
    while True:
        try:
            api("/api/v1/audit/start", True)
            break
        except (OSError, urllib.error.URLError):
            if time.monotonic() >= deadline:
                raise RuntimeError("host API did not become ready")
            time.sleep(.2)
    # API binding can precede Go's DNS listener construction. No warm-up query.
    expected_ports = {row["port"] for row in corpus}
    while True:
        listening = set()
        for table in ("udp", "udp6"):
            for line in pathlib.Path("/proc/net/" + table).read_text().splitlines()[1:]:
                listening.add(int(line.split()[1].rsplit(":", 1)[1], 16))
        if expected_ports <= listening:
            break
        if time.monotonic() >= deadline:
            raise RuntimeError("DNS listeners did not become ready")
        time.sleep(.1)
    api("/api/v1/audit/clear", True)
    sockets = {}
    for source in {row["source"] for row in corpus}:
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind((source, 0))
        sock.settimeout(6)
        sockets[source] = sock
    samples = [resources(pid)]
    output = (root / "replay.jsonl").open("w")
    for pass_id in (1, 2):
        for index, row in enumerate(corpus):
            ident = (index + pass_id * len(corpus)) % 65536
            wire = question(row["name"], row["type"], ident)
            sock = sockets[row["source"]]
            start = time.perf_counter_ns()
            started_s = time.monotonic()
            sock.sendto(wire, ("127.0.0.1", row["port"]))
            try:
                result = decode(sock.recv(65535))
                if result["id"] != ident or result["questions"] != [(row["name"] + ".", row["type"], 1)]:
                    raise ValueError("uncorrelated DNS response")
            except (OSError, ValueError) as error:
                result = {"error": str(error)}
            latency = (time.perf_counter_ns() - start) / 1e6
            finished_s = time.monotonic()
            audit = None
            if row["port"] in audit_ports:
                for attempt in range(10):
                    logs = json.loads(api("/api/v2/audit/logs?page=1&limit=1"))["logs"]
                    if logs:
                        entry = logs[0]
                        client = ipaddress.ip_address(entry["client_ip"])
                        client = getattr(client, "ipv4_mapped", None) or client
                        if entry["query_name"].rstrip(".").lower() == row["name"] and entry["query_type"] == {1: "A", 28: "AAAA"}[row["type"]] and str(client) == row["source"]:
                            audit = entry
                            break
                    time.sleep(.01)
            output.write(json.dumps({"pass": pass_id, "index": index, "input": row, "dns": result, "audit": audit, "audit_expected": row["port"] in audit_ports, "latency_ms": latency, "started_s": started_s, "finished_s": finished_s}, ensure_ascii=False) + "\n")
        output.flush()
        samples.append(resources(pid))
    output.close()
    for sock in sockets.values():
        sock.close()
    for tag in ("top_domains", "my_realiplist", "my_fakeiplist"):
        (root / (tag + "-show.txt")).write_bytes(api(f"/plugins/{tag}/show?limit=10"))
    (root / "domain-output-probe.txt").write_bytes(api("/plugins/top_domains/show?q=e2e1-unmatched&limit=10"))
    (root / "metrics.txt").write_bytes(api("/metrics"))
    (root / "resources.json").write_text(json.dumps(samples, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=pathlib.Path)
    parser.add_argument("corpus", type=pathlib.Path)
    parser.add_argument("pid", type=int)
    parser.add_argument("--api-port", type=int, default=9099)
    args = parser.parse_args()
    API_PORT = args.api_port
    run(args.root, args.corpus, args.pid)
