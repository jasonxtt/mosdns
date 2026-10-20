"""Real veth round trip with a documentation client, plus physical observation."""
import json
import pathlib
import queue
import socket
import struct
import subprocess
import sys
import threading
import time

from protocol import decode_dns, udp4


def counters():
    data = json.loads(subprocess.check_output(["nft", "-j", "list", "table", "netdev", "shadow1_guard"]))
    return [expr["counter"]["packets"] for item in data["nftables"]
            for expr in item.get("rule", {}).get("expr", []) if "counter" in expr]


def worker(root):
    state = json.loads((root / "network.json").read_text())
    if state["phase"] != "ready":
        raise RuntimeError("network incomplete")
    return subprocess.Popen(["ip", "netns", "exec", "shadow1-replay", "python3", "-u",
        str(pathlib.Path(__file__).with_name("worker.py")), "--interface", "shadow1-p", "--target", state["target"],
        "--source-mac", state["peer_mac"], "--target-mac", state["host_mac"]],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)


def main(root):
    before = counters()
    if not before or any(before):
        raise RuntimeError("physical guard counters already nonzero")
    iface = json.loads(subprocess.check_output(["ip", "-j", "route", "show", "default"]))[0]["dev"]
    physical = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0003))
    physical.bind((iface, 0))
    physical.setblocking(False)
    child = worker(root)
    try:
        if not json.loads(child.stdout.readline()).get("ready"):
            raise RuntimeError("worker not ready")
        wire = struct.pack("!6H", 0x1234, 0x100, 1, 0, 0, 0) + b"\x07example\x03com\0\0\x01\0\x01"
        events = queue.Queue()
        def read_worker():
            try:
                for line in child.stdout:
                    events.put(json.loads(line))
            finally:
                events.put({'worker_eof': True})
        reader = threading.Thread(target=read_worker, daemon=True)
        reader.start()
        child.stdin.write(json.dumps({"token": 0, "source": "198.51.100.77", "port": 53001, "wire": wire.hex()}) + "\n")
        child.stdin.flush()
        answer, leaked, sent = None, 0, False
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            # TextIOWrapper may already hold the second JSON line after the
            # first readline. Selecting only the underlying fd misses it.
            try:
                row = events.get(timeout=.05)
                if row.get('worker_eof'):
                    raise RuntimeError('worker exited')
                if "sent" in row:
                    sent = True
                elif "wire" in row:
                    dns = decode_dns(bytes.fromhex(row["wire"]))
                    if row["source"] == "198.51.100.77" and row["port"] == 53001 and dns["id"] == 0x1234 and dns["questions"] == decode_dns(wire)["questions"]:
                        answer = {"rcode": dns["rcode"], "has_answer": bool(dns["answers"])}
            except queue.Empty:
                pass
            while True:
                try:
                    packet, address = physical.recvfrom(65535)
                    if address[2] == socket.PACKET_OUTGOING:
                        row = udp4(packet, 1)
                        if row and row[1] == 53:
                            leaked += 1
                except BlockingIOError:
                    break
        after = counters()
        result = {"sent": sent, "answer": answer, "physical_observed_udp53": leaked,
                  "guard_before": before, "guard_after": after}
        (root / "isolation-probe.json").write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result))
        if not sent or answer is None or leaked or any(after):
            raise RuntimeError("isolation probe failed")
    finally:
        child.stdin.close()
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            child.terminate()
            child.wait(timeout=3)
        if 'reader' in locals():
            reader.join(timeout=1)
        physical.close()


if __name__ == "__main__":
    main(pathlib.Path(sys.argv[1]))
