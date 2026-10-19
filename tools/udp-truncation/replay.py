"""VM-only, loopback-netns process proof. Private samples/output stay external.

The supplied corpus contains saved query_wire, rust_wire (complete old reply)
and go_wire. No capture names/addresses are printed or copied into this tool.
"""
import argparse
import json
import pathlib
import socket
import struct
import subprocess
import threading
import time
import urllib.request


def key(packet):
    labels, offset, end, seen = [], 12, None, set()
    while True:
        if offset in seen or offset >= len(packet):
            raise ValueError("bad question")
        seen.add(offset)
        size = packet[offset]
        offset += 1
        if size & 0xc0 == 0xc0:
            end = end or offset + 1
            offset = (size & 63) * 256 + packet[offset]
        elif not size:
            end = end or offset
            return b".".join(labels).lower(), packet[end:end + 4]
        else:
            labels.append(packet[offset:offset + size])
            offset += size


def fields(wire):
    return {"bytes": len(wire), "tc": bool(wire[2] & 2), "counts": list(struct.unpack("!3H", wire[6:12]))}


def main(args):
    if args.work.exists():
        raise RuntimeError("refuse to overwrite private proof")
    args.work.mkdir(mode=0o700)
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    corpus = json.loads(args.samples.read_text())
    by_question = {key(bytes.fromhex(row["query_wire"])): bytes.fromhex(row["rust_wire"]) for row in corpus}
    upstream = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    upstream.bind(("127.0.0.1", 5300))
    upstream.settimeout(.1)
    stop = threading.Event()
    errors, calls = [], []

    def serve():
        while not stop.is_set():
            try:
                query, peer = upstream.recvfrom(65535)
            except socket.timeout:
                continue
            try:
                response = by_question[key(query)]
                calls.append(time.time())
                upstream.sendto(query[:2] + response[2:], peer)
            except (KeyError, ValueError, OSError) as error:
                errors.append(type(error).__name__)

    thread = threading.Thread(target=serve)
    thread.start()
    config = '''log:
  level: error
api:
  http: "127.0.0.1:9099"
plugins:
  - tag: upstream
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:5300"
  - tag: main
    type: sequence
    args:
      - exec: $upstream
  - tag: listener
    type: udp_server
    args:
      entry: main
      listen: "127.0.0.1:5353"
      enable_audit: true
'''
    (args.work / "config.yaml").write_text(config)
    log = (args.work / "host.log").open("w")
    process = subprocess.Popen([str(args.binary), "start", "-d", str(args.work), "-c", "config.yaml"], stdout=log, stderr=subprocess.STDOUT)
    output, private = [], []
    try:
        deadline = time.monotonic() + 15
        while True:
            try:
                with http.open("http://127.0.0.1:9099/api/v1/system/health", timeout=1) as response:
                    health = json.load(response)
                if health["ready"]:
                    break
            except OSError:
                pass
            if process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("private host failed startup; inspect private log")
            time.sleep(.05)
        client = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        client.bind(("127.0.0.1", 0))
        client.settimeout(3)
        # Readiness can precede the serve loop. A bound host has already owned
        # its DNS socket, so the datagram remains queued until it is served.
        for row in corpus:
            query = bytes.fromhex(row["query_wire"])
            client.sendto(query, ("127.0.0.1", 5353))
            wire, peer = client.recvfrom(65535)
            if peer != ("127.0.0.1", 5353) or wire[:2] != query[:2] or key(wire) != key(query):
                raise RuntimeError("uncorrelated private reply")
            original_go = fields(bytes.fromhex(row["go_wire"]))
            actual = fields(wire)
            expected = json.loads(subprocess.check_output([str(args.oracle)], input=json.dumps({"name":str(row["token"]),"wire":row["rust_wire"],"size":512}).encode() + b"\n"))
            oracle = {"bytes": expected["size"], "tc": expected["tc"], "counts": [expected[k] for k in ("answer", "authority", "additional")]}
            audit, audit_ok = {}, False
            for _ in range(20):
                with http.open("http://127.0.0.1:9099/api/v2/audit/logs?limit=1", timeout=2) as response:
                    logs = json.load(response)["logs"]
                audit = logs[0] if logs else {}
                audit_ok = (audit.get("query_name", "").rstrip(".").lower().encode("ascii") == key(query)[0]
                            and audit.get("response_flags", {}).get("TC") is True
                            and len(audit.get("answers", [])) == actual["counts"][0])
                if audit_ok:
                    break
                time.sleep(.01)
            passed = actual == original_go == oracle and actual["bytes"] <= 512 and actual["tc"] and audit_ok
            output.append({"token":row["token"],"actual":actual,"captured_go":original_go,"oracle":oracle,"audit_matches":audit_ok,"pass":passed})
            private.append({"token":row["token"],"query_wire":row["query_wire"],"response_wire":wire.hex(),"audit":audit})
        client.close()
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        stop.set()
        thread.join(timeout=2)
        upstream.close()
        log.close()
    (args.work / "results-private.json").write_text(json.dumps(private, indent=2) + "\n")
    (args.work / "summary.json").write_text(json.dumps({"rows":output,"upstream_calls":len(calls),"mock_errors":errors,"host_exit":process.returncode,"processes_stopped":not thread.is_alive()}, indent=2) + "\n")
    print(json.dumps({"rows":output,"upstream_calls":len(calls),"mock_errors":errors,"processes_stopped":not thread.is_alive()}))
    if errors or len(output) != 3 or not all(row["pass"] for row in output):
        raise RuntimeError("private sample comparison failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    for name in ("binary", "samples", "oracle", "work"):
        parser.add_argument("--" + name, required=True, type=lambda value: pathlib.Path(value).resolve())
    main(parser.parse_args())
