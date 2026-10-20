"""Bounded pcapng/IPv4/UDP and DNS projections for the shadow experiment.

Based on the small codec in tools/e2e-1/dnswire.py. This projection deliberately
compares result classes rather than concrete addresses or TTLs.
"""
import ipaddress
import struct


def exact(stream, size):
    data = stream.read(size)
    if len(data) != size:
        raise ValueError("truncated pcapng stream")
    return data


class Pcapng:
    def __init__(self, stream):
        self.stream = stream

    def __iter__(self):
        endian, interfaces = None, []
        while True:
            head = self.stream.read(8)
            if not head:
                return
            if len(head) != 8:
                raise ValueError("truncated block header")
            prefix = b""
            if head[:4] == b"\x0a\x0d\x0d\x0a":
                prefix = exact(self.stream, 4)
                if prefix not in (b"\x4d\x3c\x2b\x1a", b"\x1a\x2b\x3c\x4d"):
                    raise ValueError("bad pcapng byte order")
                endian = "<" if prefix[0] == 0x4D else ">"
                interfaces = []
            if endian is None:
                raise ValueError("missing section header")
            kind, size = struct.unpack(endian + "II", head)
            if size < 12 + len(prefix) or size > 2 * 1024 * 1024 or size % 4:
                raise ValueError("invalid block length")
            tail = exact(self.stream, size - 8 - len(prefix))
            if struct.unpack(endian + "I", tail[-4:])[0] != size:
                raise ValueError("block length mismatch")
            body = prefix + tail[:-4]
            if kind == 1:
                if len(body) < 8 or len(interfaces) >= 256:
                    raise ValueError("bad interface block")
                linktype, _, snaplen = struct.unpack_from(endian + "HHI", body)
                resolution = 1e-6
                off = 8
                while off + 4 <= len(body):
                    code, length = struct.unpack_from(endian + "HH", body, off)
                    off += 4
                    value = body[off:off + length]
                    if len(value) != length:
                        raise ValueError("truncated interface option")
                    if code == 0:
                        break
                    if code == 9 and length == 1:
                        resolution = (2 if value[0] & 128 else 10) ** -(value[0] & 127)
                    off += (length + 3) & ~3
                interfaces.append((linktype, resolution, snaplen))
            elif kind == 6:
                if len(body) < 20:
                    raise ValueError("short packet block")
                index, high, low, caplen, wirelen = struct.unpack_from(endian + "5I", body)
                if index >= len(interfaces) or caplen > len(body) - 20 or caplen != wirelen:
                    raise ValueError("unknown interface or truncated packet")
                linktype, resolution, _ = interfaces[index]
                yield ((high << 32 | low) * resolution, body[20:20 + caplen], linktype)
            elif kind == 3:
                raise ValueError("untimestamped packet unsupported")


def udp4(packet, linktype):
    if linktype == 1:
        if len(packet) < 14:
            raise ValueError("short ethernet frame")
        kind, offset = struct.unpack_from("!H", packet, 12)[0], 14
        while kind in (0x8100, 0x88A8):
            if len(packet) < offset + 4:
                raise ValueError("short VLAN header")
            kind = struct.unpack_from("!H", packet, offset + 2)[0]
            offset += 4
        if kind != 0x0800:
            return None
        packet = packet[offset:]
    elif linktype != 101:
        raise ValueError("unsupported link type")
    if len(packet) < 20 or packet[0] >> 4 != 4:
        raise ValueError("short IPv4 header")
    ihl = (packet[0] & 15) * 4
    total, frag = struct.unpack_from("!HH", packet, 2)[0], struct.unpack_from("!H", packet, 6)[0]
    if ihl < 20 or total < ihl or len(packet) < total:
        raise ValueError("truncated IPv4 packet")
    if frag & 0x3FFF:
        raise ValueError("fragmented IPv4 packet")
    if packet[9] != 17:
        return None
    if total < ihl + 8:
        raise ValueError("short UDP header")
    sport, dport, size, _ = struct.unpack_from("!4H", packet, ihl)
    if size < 8 or ihl + size != total:
        raise ValueError("invalid UDP length")
    return (str(ipaddress.ip_address(packet[12:16])), sport,
            str(ipaddress.ip_address(packet[16:20])), dport, packet[ihl + 8:total])


def dns_name(data, offset):
    labels, seen, end = [], set(), None
    while True:
        if offset in seen or offset >= len(data) or len(seen) > 255:
            raise ValueError("invalid DNS name")
        seen.add(offset)
        size = data[offset]
        offset += 1
        if size & 0xC0 == 0xC0:
            if offset >= len(data):
                raise ValueError("truncated name pointer")
            end = end or offset + 1
            offset = (size & 63) * 256 + data[offset]
        elif size == 0:
            return b".".join(labels).lower().hex(), end or offset
        elif size <= 63 and offset + size <= len(data):
            labels.append(data[offset:offset + size])
            offset += size
        else:
            raise ValueError("invalid DNS label")


def decode_dns(data):
    if len(data) < 12:
        raise ValueError("short DNS header")
    ident, flags, nq, na, ns, nr = struct.unpack_from("!6H", data)
    if not nq or nq > 16:
        raise ValueError("invalid question count")
    questions, answers, offset, extended = [], [], 12, 0
    opt_sizes = []
    for _ in range(nq):
        name, offset = dns_name(data, offset)
        if offset + 4 > len(data):
            raise ValueError("short question")
        qt, qc = struct.unpack_from("!2H", data, offset)
        questions.append((name, qt, qc))
        offset += 4
    for index in range(na + ns + nr):
        owner, offset = dns_name(data, offset)
        if offset + 10 > len(data):
            raise ValueError("short RR")
        kind, cls, ttl, size = struct.unpack_from("!HHIH", data, offset)
        offset += 10
        if offset + size > len(data):
            raise ValueError("short RDATA")
        raw = data[offset:offset + size]
        value = None
        if kind in (1, 28):
            if size != (4 if kind == 1 else 16):
                raise ValueError("bad address RDATA")
            value = str(ipaddress.ip_address(raw))
        elif kind == 5:
            value, end = dns_name(data, offset)
            if end != offset + size:
                raise ValueError("bad CNAME RDATA")
        elif kind in (64, 65):
            if size < 3:
                raise ValueError("bad SVCB RDATA")
            target, end = dns_name(data, offset + 2)
            if end > offset + size:
                raise ValueError("bad SVCB target")
            value = (struct.unpack_from("!H", raw)[0], target)
        elif kind == 41:
            extended = ttl >> 24
            if index >= na + ns:
                opt_sizes.append(cls)
        if index < na:
            answers.append((owner, kind, cls, value))
        offset += size
    return {"id": ident, "response": bool(flags & 0x8000), "rcode": extended * 16 + (flags & 15),
            "tc": bool(flags & 0x200), "questions": tuple(questions), "answers": answers,
            "udp_limit": max(512, opt_sizes[0]) if len(opt_sizes) == 1 else 512 if not opt_sizes else None}


def semantic(dns, fake_ranges):
    networks = [ipaddress.ip_network(value) for value in fake_ranges]
    types, address_kinds, cnames, owners, service_targets = set(), set(), set(), set(), set()
    for owner, kind, cls, value in dns["answers"]:
        types.add((kind, cls))
        if kind in (1, 28):
            address = ipaddress.ip_address(value)
            address_kinds.add((kind, "fake" if any(address in n for n in networks if n.version == address.version) else "real"))
        elif kind == 5:
            owners.add(owner)
            cnames.add(value)
        elif kind in (64, 65):
            service_targets.add((kind, value))
    return {"rcode": dns["rcode"], "has_answer": bool(dns["answers"]), "tc": dns["tc"],
            "types": sorted(types), "address_kinds": sorted(address_kinds),
            "cname_terminals": sorted(cnames - owners), "service_targets": sorted(service_targets)}
