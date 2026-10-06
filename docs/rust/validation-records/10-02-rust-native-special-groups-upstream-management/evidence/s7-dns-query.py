#!/usr/bin/env python3
"""Issue one A query to an explicit high loopback port and print its answer."""

import json
import socket
import sys


def qname_and_end(packet):
    position = 12
    labels = []
    while position < len(packet):
        size = packet[position]
        position += 1
        if size == 0:
            break
        labels.append(packet[position:position + size].decode("ascii"))
        position += size
    return ".".join(labels), position + 4


def main():
    if len(sys.argv) != 4:
        raise SystemExit("usage: s7-dns-query.py PORT QNAME ID")
    port, qname, query_id = int(sys.argv[1]), sys.argv[2].rstrip("."), int(sys.argv[3])
    labels = b"".join(bytes([len(part)]) + part.encode("ascii") for part in qname.split(".")) + b"\x00"
    packet = query_id.to_bytes(2, "big") + b"\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00" + labels + b"\x00\x01\x00\x01"
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
        client.settimeout(3)
        client.sendto(packet, ("127.0.0.1", port))
        response, _ = client.recvfrom(4096)
    name, answer_start = qname_and_end(response)
    position = answer_start
    if response[position:position + 2] == b"\xc0\x0c":
        position += 2
    else:
        while response[position] != 0:
            position += response[position] + 1
        position += 1
    rtype = int.from_bytes(response[position:position + 2], "big")
    position += 8
    length = int.from_bytes(response[position:position + 2], "big")
    position += 2
    value = socket.inet_ntoa(response[position:position + length]) if rtype == 1 and length == 4 else None
    print(json.dumps({"port": port, "query": name, "id": query_id, "rcode": response[3] & 0x0F, "answer": value}, sort_keys=True))


if __name__ == "__main__":
    main()
