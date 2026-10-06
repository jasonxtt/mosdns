import json
import socket
import struct
import time


NAME = "s6-go-cache.example"
LISTENER = ("127.0.0.1", 16453)
EXPECTED_ANSWER = "198.51.100.42"


def encode_name(name):
    return b"".join(bytes([len(label)]) + label.encode() for label in name.split(".")) + b"\0"


def query(ident):
    qname = encode_name(NAME)
    packet = struct.pack("!HHHHHH", ident, 0x0100, 1, 0, 0, 0) + qname + struct.pack("!HH", 1, 1)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.settimeout(3)
        sock.sendto(packet, LISTENER)
        data, peer = sock.recvfrom(4096)

    rid, flags, _, answer_count, _, _ = struct.unpack("!HHHHHH", data[:12])
    offset = 12
    while data[offset]:
        offset += data[offset] + 1
    offset += 5

    answers = []
    for _ in range(answer_count):
        if data[offset] & 0xC0 == 0xC0:
            offset += 2
        else:
            while data[offset]:
                offset += data[offset] + 1
            offset += 1
        record_type, _, _, length = struct.unpack("!HHIH", data[offset : offset + 10])
        offset += 10
        rdata = data[offset : offset + length]
        offset += length
        if record_type == 1 and length == 4:
            answers.append(socket.inet_ntoa(rdata))

    result = {
        "query_id": rid,
        "rcode": flags & 0xF,
        "answer_count": answer_count,
        "answers": answers,
        "response_source": peer[0],
    }
    if rid != ident or result["rcode"] != 0 or EXPECTED_ANSWER not in answers:
        raise RuntimeError(f"unexpected controlled DNS response: {result}")
    return result


results = [query(0x6501)]
time.sleep(0.2)
results.append(query(0x6502))
print(
    json.dumps(
        {
            "query_name": NAME,
            "listener": f"{LISTENER[0]}:{LISTENER[1]}",
            "controlled_upstream": "127.0.0.1:25455",
            "expected_answer": EXPECTED_ANSWER,
            "results": results,
        },
        indent=2,
    )
)
