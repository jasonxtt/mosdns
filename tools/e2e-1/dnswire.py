"""Small DNS codec for the controlled A/AAAA experiment (no dependencies)."""
import ipaddress
import struct


def name(data, offset):
    labels, seen = [], set()
    end = None
    while True:
        if offset in seen or offset >= len(data):
            raise ValueError("invalid DNS name")
        seen.add(offset)
        size = data[offset]
        offset += 1
        if size & 0xC0 == 0xC0:
            pointer = ((size & 63) << 8) | data[offset]
            end = end or offset + 1
            offset = pointer
        elif size == 0:
            return ".".join(labels).lower() + ".", end or offset
        elif size <= 63:
            labels.append(data[offset:offset + size].decode("ascii"))
            offset += size
        else:
            raise ValueError("invalid DNS label")


def question(qname, qtype, ident):
    wire = b"".join(bytes([len(s)]) + s.encode("ascii") for s in qname.rstrip(".").split(".")) + b"\0"
    return struct.pack("!6H", ident, 0x100, 1, 0, 0, 0) + wire + struct.pack("!2H", qtype, 1)


def decode(data):
    ident, flags, nq, na, ns, nr = struct.unpack_from("!6H", data)
    pos = 12
    questions = []
    for _ in range(nq):
        qname, pos = name(data, pos)
        qt, qc = struct.unpack_from("!2H", data, pos)
        pos += 4
        questions.append((qname, qt, qc))
    records, authority, additional, ecs = [], [], [], []
    for index in range(na + ns + nr):
        owner, pos = name(data, pos)
        rt, rc, ttl, size = struct.unpack_from("!HHIH", data, pos)
        pos += 10
        raw = data[pos:pos + size]
        if len(raw) != size:
            raise ValueError("short RDATA")
        if rt in (1, 28):
            value = str(ipaddress.ip_address(raw))
        elif rt in (2, 5, 12):
            value = name(data, pos)[0]
        else:
            value = raw.hex()
        if index < na:
            records.append((owner, rt, rc, value, ttl))
        elif rt != 41:
            section = authority if index < na + ns else additional
            section.append((owner, rt, rc, value, ttl))
        if rt == 41:
            off = 0
            while off < len(raw):
                code, length = struct.unpack_from("!HH", raw, off)
                off += 4
                if code == 8:
                    ecs.append(raw[off:off + length].hex())
                off += length
        pos += size
    return {"id": ident, "rcode": flags & 15, "questions": questions,
            "answers": sorted(records), "authority": sorted(authority),
            "additional": sorted(additional), "ecs": ecs}


def answer(query, v4, v6):
    _, end = name(query, 12)
    qt, _ = struct.unpack_from("!2H", query, end)
    end += 4
    raw = ipaddress.ip_address(v4 if qt == 1 else v6).packed if qt in (1, 28) else b""
    flags = 0x8080 | (struct.unpack_from("!H", query, 2)[0] & 0x110)
    header = query[:2] + struct.pack("!5H", flags, 1, int(bool(raw)), 0, 0)
    record = b"\xc0\x0c" + struct.pack("!HHIH", qt, 1, 300, len(raw)) + raw if raw else b""
    return header + query[12:end] + record
