"""Local-only DNS interruption probe. Outputs timings/counts, never names."""
import argparse
import json
import pathlib
import signal
import socket
import struct
import time

from protocol import decode_dns


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--name", default="example.com")
    args = parser.parse_args()
    running = True

    def stop(*_):
        nonlocal running
        running = False

    signal.signal(signal.SIGTERM, stop)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("127.0.0.1", 0))
    sock.settimeout(.2)
    body = b"".join(bytes([len(label)]) + label.encode("ascii") for label in args.name.rstrip(".").split(".")) + b"\0\0\x01\0\x01"
    sent = failed = 0
    last = None
    maximum = 0
    started = time.time()
    with args.output.open("w") as stream:
        while running:
            ident = sent % 65536
            sent += 1
            start = time.monotonic()
            sock.sendto(struct.pack("!6H", ident, 0x100, 1, 0, 0, 0) + body, ("127.0.0.1", 53))
            good = False
            try:
                deadline = start + .2
                while time.monotonic() < deadline:
                    data, peer = sock.recvfrom(65535)
                    dns = decode_dns(data)
                    if peer == ("127.0.0.1", 53) and dns["id"] == ident:
                        good = dns["response"] and dns["rcode"] == 0 and bool(dns["answers"])
                        break
            except (OSError, ValueError):
                pass
            now = time.monotonic()
            if good:
                maximum = max(maximum, now - last if last is not None else 0)
                last = now
            else:
                failed += 1
            stream.write(json.dumps({"time": time.time(), "good": good, "elapsed_ms": (now - start) * 1000}) + "\n")
            stream.flush()
            time.sleep(max(0, .02 - (time.monotonic() - start)))
    args.output.with_suffix(".summary.json").write_text(json.dumps({"started": started, "ended": time.time(), "sent": sent,
        "failed": failed, "max_success_gap_seconds": maximum}, indent=2) + "\n")


if __name__ == "__main__":
    main()
