#!/usr/bin/env python3
"""Run the S7 isolated native DNS/management HTTP proof and save JSON evidence."""

import json
import sys
import urllib.error
import urllib.request

API = "http://127.0.0.1:16580"
MAIN = 16555
GROUP_50 = 16556
GROUP_51 = 16557
GROUP_52 = 16558


def query(port, name, query_id):
    import socket

    labels = b"".join(bytes((len(label),)) + label.encode("ascii") for label in name.split(".")) + b"\0"
    packet = query_id.to_bytes(2, "big") + b"\x01\x00\0\x01\0\0\0\0\0\0" + labels + b"\0\x01\0\x01"
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
        client.settimeout(3)
        client.sendto(packet, ("127.0.0.1", port))
        response, _ = client.recvfrom(4096)
    position = 12
    while response[position]:
        position += response[position] + 1
    position += 5
    if response[position:position + 2] == b"\xc0\x0c":
        position += 2
    else:
        while response[position]:
            position += response[position] + 1
        position += 1
    rtype = int.from_bytes(response[position:position + 2], "big")
    position += 8
    length = int.from_bytes(response[position:position + 2], "big")
    position += 2
    answer = socket.inet_ntoa(response[position:position + length]) if rtype == 1 and length == 4 else None
    return {"port": port, "qname": name, "rcode": response[3] & 0x0F, "answer": answer}


def request(method, path, payload=None):
    body = None if payload is None else json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(API + path, data=body, method=method, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            return response.status, response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8")


def main():
    proof = {"queries": []}
    for port, name, query_id in [
        (MAIN, "shared.example", 1),
        (MAIN, "shared.example", 2),
        (GROUP_50, "shared.example", 3),
        (GROUP_51, "shared.example", 4),
        (MAIN, "exclusive.example", 5),
        (GROUP_52, "exclusive.example", 6),
        (MAIN, "no-match.example", 7),
    ]:
        proof["queries"].append(query(port, name, query_id))

    save_status, save_body = request("POST", "/api/v1/upstream/config", {
        "plugin_tag": "special_upstream_50",
        "upstreams": [{
            "tag": "lower_supplier_v2", "enabled": True, "protocol": "udp",
            "addr": "udp://127.0.0.1:25559",
        }],
    })
    proof["save"] = {"status": save_status, "body": json.loads(save_body)}
    proof["queries"].append(query(MAIN, "shared.example", 8))
    proof["queries"].append(query(GROUP_51, "shared.example", 9))

    failure_status, failure_body = request("POST", "/api/v1/upstream/config", {
        "plugin_tag": "special_upstream_50",
        "upstreams": [{
            "tag": "unsupported", "enabled": True, "protocol": "quic",
            "addr": "quic://127.0.0.1:25560",
        }],
    })
    proof["unsupported_save"] = {"status": failure_status, "body": failure_body}
    runtime_status, runtime_body = request("GET", "/api/v1/upstream/runtime/special_upstream_50")
    proof["committed_runtime"] = {"status": runtime_status, "body": json.loads(runtime_body)}
    proof["queries"].append(query(MAIN, "shared.example", 10))
    audit_status, audit_body = request("GET", "/api/v2/audit/logs?page=1&limit=100")
    proof["audit"] = {"status": audit_status, "body": json.loads(audit_body)}
    metrics_status, metrics_body = request("GET", "/metrics")
    proof["metrics"] = {"status": metrics_status, "body": metrics_body}
    json.dump(proof, sys.stdout, ensure_ascii=False, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
