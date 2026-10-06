#!/usr/bin/env python3
"""Local-only DNS peer and query helper for the S6 browser proof.

Both modes bind/contact only 127.0.0.1 and require an explicit high port. The
peer returns a fixed documentation-range address and appends observed qnames to
the supplied JSONL file.
"""

import argparse
import ipaddress
import json
import socket
import struct
import time


def name_wire(name):
    labels = [label.encode("ascii") for label in name.rstrip(".").split(".")]
    return b"".join(bytes([len(label)]) + label for label in labels) + b"\0"


def decode_name(packet, offset):
    labels = []
    while True:
        length = packet[offset]
        offset += 1
        if length == 0:
            break
        labels.append(packet[offset : offset + length].decode("ascii"))
        offset += length
    return ".".join(labels), offset


def serve(args):
    answer = ipaddress.IPv4Address(args.answer).packed
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("127.0.0.1", args.port))
    sock.settimeout(0.5)
    with open(args.log, "a", encoding="utf-8") as log:
        while True:
            try:
                packet, peer = sock.recvfrom(4096)
            except socket.timeout:
                continue
            if len(packet) < 17:
                continue
            qname, end = decode_name(packet, 12)
            qtype, qclass = struct.unpack("!HH", packet[end : end + 4])
            log.write(json.dumps({"time": time.time(), "qname": qname, "qtype": qtype, "peer": peer[0]}) + "\n")
            log.flush()
            if qtype != 1 or qclass != 1:
                response = packet[:2] + struct.pack("!HHHHH", 0x8180, 1, 0, 0, 0) + packet[12 : end + 4]
            else:
                question = packet[12 : end + 4]
                answer_rr = b"\xc0\x0c" + struct.pack("!HHIH", 1, 1, 60, 4) + answer
                response = packet[:2] + struct.pack("!HHHHH", 0x8180, 1, 1, 0, 0) + question + answer_rr
            sock.sendto(response, peer)


def query(args):
    ident = 0x5A61
    question = name_wire(args.name) + struct.pack("!HH", 1, 1)
    request = struct.pack("!HHHHHH", ident, 0x0100, 1, 0, 0, 0) + question
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(3)
    sock.sendto(request, ("127.0.0.1", args.port))
    response, _peer = sock.recvfrom(4096)
    qname, offset = decode_name(response, 12)
    offset += 4
    answers = []
    answer_count = struct.unpack("!H", response[6:8])[0]
    for _ in range(answer_count):
        if response[offset] & 0xC0 == 0xC0:
            offset += 2
        else:
            _owner, offset = decode_name(response, offset)
        kind, klass, ttl, size = struct.unpack("!HHIH", response[offset : offset + 10])
        offset += 10
        rdata = response[offset : offset + size]
        offset += size
        if kind == 1 and klass == 1 and size == 4:
            answers.append(str(ipaddress.IPv4Address(rdata)))
    print(json.dumps({"qname": qname, "rcode": response[3] & 0x0F, "answers": answers}))


def main():
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    server = commands.add_parser("serve")
    server.add_argument("--port", type=int, required=True)
    server.add_argument("--answer", default="198.51.100.42")
    server.add_argument("--log", required=True)
    server.set_defaults(run=serve)
    client = commands.add_parser("query")
    client.add_argument("--port", type=int, required=True)
    client.add_argument("--name", required=True)
    client.set_defaults(run=query)
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535:
        parser.error("use an explicit high port in 1024..65535")
    args.run(args)


if __name__ == "__main__":
    main()
