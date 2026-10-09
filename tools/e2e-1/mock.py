"""Deterministic loopback DNS (UDP/TCP/DoT/DoH) and restricted SOCKS5."""
import argparse
import asyncio
import base64
import json
import pathlib
import signal
import ssl
import struct
import time
from urllib.parse import urlsplit, parse_qs

from dnswire import answer, decode

GROUPS = ["domestic", "foreign", "foreignecs", "cnfake", "nocnfake"]
MARKERS = {
    "domestic": ["223.5.5.101", "2400:3200::101"],
    "foreign": ["203.0.113.20", "2001:db8:2::20"],
    "foreignecs": ["203.0.113.30", "2001:db8:3::30"],
    "cnfake": ["30.0.0.10", "2400::10"],
    "nocnfake": ["28.0.0.10", "2001:2::10"],
}


def ports(group):
    base = 15000 + GROUPS.index(group) * 10
    return {"udp": base, "tcp": base, "tls": base + 1, "https": base + 2}


async def serve(root):
    journal = (root / "mock.jsonl").open("a", buffering=1)
    # Optional 5D latency model; absent file preserves the E2E fixture behavior.
    delay_path = root / "mock-delays.json"
    delays = json.loads(delay_path.read_text()) if delay_path.exists() else {}
    if set(delays) - set(GROUPS) or any(not isinstance(v, (int, float)) or v < 0 for v in delays.values()):
        raise ValueError("invalid mock delays")

    async def delayed(wire, group, protocol):
        await asyncio.sleep(delays.get(group, 0) / 1000)
        return respond(wire, group, protocol)

    def log(value):
        value["monotonic_s"] = time.monotonic()
        journal.write(json.dumps(value) + "\n")

    def respond(wire, group, protocol):
        decoded = decode(wire)
        log({"kind": "dns", "group": group, "protocol": protocol,
             "question": decoded["questions"], "ecs": decoded["ecs"]})
        return answer(wire, *MARKERS[group])

    class UDP(asyncio.DatagramProtocol):
        def __init__(self, group):
            self.group = group

        def connection_made(self, transport):
            self.transport = transport

        def datagram_received(self, wire, peer):
            if delays.get(self.group, 0):
                async def send():
                    self.transport.sendto(await delayed(wire, self.group, "udp"), peer)
                asyncio.create_task(send())
            else:
                self.transport.sendto(respond(wire, self.group, "udp"), peer)

    async def dns_stream(reader, writer, group, protocol):
        try:
            while True:
                length = struct.unpack("!H", await reader.readexactly(2))[0]
                result = await delayed(await reader.readexactly(length), group, protocol)
                writer.write(struct.pack("!H", len(result)) + result)
                await writer.drain()
        except (asyncio.IncompleteReadError, ConnectionError):
            pass
        finally:
            writer.close()
            await writer.wait_closed()

    async def doh(reader, writer, group):
        try:
            while True:
                header = await reader.readuntil(b"\r\n\r\n")
                lines = header.split(b"\r\n")
                method, target, _ = lines[0].split()
                fields = {k.lower(): v.strip() for k, v in (line.split(b":", 1) for line in lines[1:] if b":" in line)}
                if method == b"GET":
                    encoded = parse_qs(urlsplit(target.decode()).query)["dns"][0]
                    query = base64.urlsafe_b64decode(encoded + "=" * (-len(encoded) % 4))
                elif method == b"POST":
                    query = await reader.readexactly(int(fields[b"content-length"]))
                else:
                    raise ValueError("unsupported DoH method")
                result = await delayed(query, group, "https")
                writer.write(b"HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: " + str(len(result)).encode() + b"\r\n\r\n" + result)
                await writer.drain()
        except (asyncio.IncompleteReadError, ConnectionError):
            pass
        finally:
            writer.close()
            await writer.wait_closed()

    allowed = {p for g in GROUPS for proto, p in ports(g).items() if proto != "udp"}

    async def socks(reader, writer):
        remote = None
        try:
            version, count = await reader.readexactly(2)
            methods = await reader.readexactly(count)
            if version != 5 or 0 not in methods:
                return
            writer.write(b"\x05\x00")
            await writer.drain()
            version, command, reserved, kind = await reader.readexactly(4)
            if version != 5 or command != 1 or reserved != 0:
                return
            if kind == 1:
                host = ".".join(str(b) for b in await reader.readexactly(4))
            elif kind == 3:
                host = (await reader.readexactly((await reader.readexactly(1))[0])).decode()
            else:
                return
            port = struct.unpack("!H", await reader.readexactly(2))[0]
            permitted = host in ("localhost", "127.0.0.1") and port in allowed
            log({"kind": "socks", "host": host, "port": port, "allowed": permitted})
            if not permitted:
                writer.write(b"\x05\x02\x00\x01" + b"\0" * 6)
                await writer.drain()
                return
            rr, remote = await asyncio.open_connection("127.0.0.1", port)
            writer.write(b"\x05\x00\x00\x01\x7f\0\0\x01\0\0")
            await writer.drain()

            async def copy(source, target):
                while block := await source.read(65536):
                    target.write(block)
                    await target.drain()

            tasks = [asyncio.create_task(copy(reader, remote)), asyncio.create_task(copy(rr, writer))]
            try:
                await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            finally:
                for task in tasks:
                    task.cancel()
                await asyncio.gather(*tasks, return_exceptions=True)
        except (asyncio.IncompleteReadError, ConnectionError):
            pass
        finally:
            for stream in (remote, writer):
                if stream:
                    stream.close()
                    try:
                        await stream.wait_closed()
                    except ConnectionError:
                        pass

    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(root / "cert.pem", root / "key.pem")
    servers, transports = [], []
    loop = asyncio.get_running_loop()
    for group in GROUPS:
        p = ports(group)
        transport, _ = await loop.create_datagram_endpoint(lambda g=group: UDP(g), local_addr=("127.0.0.1", p["udp"]))
        transports.append(transport)
        for protocol in ("tcp", "tls", "https"):
            callback = (lambda r, w, g=group: doh(r, w, g)) if protocol == "https" else (lambda r, w, g=group, proto=protocol: dns_stream(r, w, g, proto))
            servers.append(await asyncio.start_server(callback, "127.0.0.1", p[protocol], ssl=tls if protocol != "tcp" else None))
    servers.append(await asyncio.start_server(socks, "127.0.0.1", 17891))
    stop = asyncio.Event()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stop.set)
    (root / "mock-ready").touch()
    await stop.wait()
    for server in servers:
        server.close()
        await server.wait_closed()
    for transport in transports:
        transport.close()
    journal.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=pathlib.Path)
    asyncio.run(serve(parser.parse_args().root))
