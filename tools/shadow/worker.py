"""Namespace-only raw UDP injector/receiver; JSON lines over local pipes."""
import argparse
import ipaddress
import json
import os
import selectors
import socket
import struct
import sys
import time

from protocol import udp4


def frame(source, port, target, wire, source_mac, target_mac):
    source = ipaddress.IPv4Address(source)
    target = ipaddress.IPv4Address(target)
    if source.is_unspecified or source.is_multicast or source.is_loopback or source == target:
        raise ValueError("ineligible client address")
    size = 28 + len(wire)
    if size > 65535 or not 0 < port <= 65535:
        raise ValueError("invalid UDP query size/port")
    header = struct.pack("!BBHHHBBH4s4s", 0x45, 0, size, 0, 0, 64, 17, 0, source.packed, target.packed)
    checksum = sum(struct.unpack("!10H", header))
    while checksum >> 16:
        checksum = (checksum & 65535) + (checksum >> 16)
    header = header[:10] + struct.pack("!H", ~checksum & 65535) + header[12:]
    return bytes.fromhex(target_mac.replace(":", "")) + bytes.fromhex(source_mac.replace(":", "")) + b"\x08\0" + header + struct.pack("!4H", port, 53, len(wire) + 8, 0) + wire


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--interface", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--source-mac", required=True)
    parser.add_argument("--target-mac", required=True)
    args = parser.parse_args()
    sock = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0800))
    sock.bind((args.interface, 0))
    sock.setblocking(False)
    sel = selectors.DefaultSelector()
    sel.register(sock, selectors.EVENT_READ, "packet")
    sel.register(sys.stdin, selectors.EVENT_READ, "query")
    buffered = b""
    next_stats = time.monotonic() + 60

    def stats():
        raw = sock.getsockopt(getattr(socket, "SOL_PACKET", 263), 6, 12)
        packets, dropped = struct.unpack_from("II", raw)
        print(json.dumps({"packet_statistics": {"packets": packets, "dropped": dropped}}), flush=True)

    print(json.dumps({"ready": True, "pid": os.getpid()}), flush=True)
    while True:
        for key, _ in sel.select(1):
            if key.data == "query":
                data = os.read(sys.stdin.fileno(), 65536)
                if not data:
                    stats()
                    return
                buffered += data
                if len(buffered) > 1024 * 1024:
                    raise ValueError("oversized command stream")
                while b"\n" in buffered:
                    line, buffered = buffered.split(b"\n", 1)
                    row = json.loads(line)
                    wire = bytes.fromhex(row["wire"])
                    packet = frame(row["source"], row["port"], args.target, wire, args.source_mac, args.target_mac)
                    sock.send(packet)
                    print(json.dumps({"sent": row["token"], "monotonic": time.monotonic()}), flush=True)
            else:
                packet = sock.recv(65549)
                try:
                    row = udp4(packet, 1)
                except ValueError as error:
                    print(json.dumps({"frame_error": str(error)}), flush=True)
                    continue
                if row and row[0] == args.target and row[1] == 53:
                    print(json.dumps({"source": row[2], "port": row[3], "wire": row[4].hex(), "monotonic": time.monotonic()}), flush=True)
        if time.monotonic() >= next_stats:
            stats()
            next_stats += 60


if __name__ == "__main__":
    main()
