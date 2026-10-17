"""Deterministic loopback DNS (UDP/TCP/DoT/DoH), restricted SOCKS5 and an optional
HTTP file server.

Without ``faults.json`` in the root the behavior is the E2E/5D-1 fixture:
fixed answers, optional per-group delays from ``mock-delays.json``. A
``faults.json`` that appears or changes at run time (polled every 50 ms) injects
the failure modes in ``FAULT_MODES``; removing a fault restores normal service.
"""
import argparse
import asyncio
import base64
import json
import pathlib
import signal
import ssl
import struct
import time
from urllib.parse import urlsplit, parse_qs, unquote

from dnswire import answer, decode

GROUPS = ["domestic", "foreign", "foreignecs", "cnfake", "nocnfake"]
MARKERS = {
    "domestic": ["223.5.5.101", "2400:3200::101"],
    "foreign": ["203.0.113.20", "2001:db8:2::20"],
    "foreignecs": ["203.0.113.30", "2001:db8:3::30"],
    "cnfake": ["30.0.0.10", "2400::10"],
    "nocnfake": ["28.0.0.10", "2001:2::10"],
}
# DNS upstream faults (per group, optionally limited to some protocols):
#   blackhole       read the query, never answer (UDP: drop; streams stay open)
#   refuse          listener closed and live connections dropped (TCP/TLS/DoH,
#                   UDP: closed socket); restored when the fault is removed
#   reset           accept, read the query, then close (mid-stream disconnect)
#   handshake_reset TLS/DoH only: close before any TLS byte is exchanged
#   bad_id          answer with a different DNS ID
#   truncated       answer cut to `truncate` bytes (default 20)
#   wrong_question  answer whose question name differs from the query
#   servfail        well-formed SERVFAIL answer with the right ID and question
#   slow            answer after `delay_ms`
FAULT_MODES = ("blackhole", "refuse", "reset", "handshake_reset", "bad_id",
               "truncated", "wrong_question", "servfail", "slow")
SOCKS_MODES = ("down", "blackhole", "reset")
SOCKS_PORTS = (17891, 17892)
DOWNLOAD_PORT = 18080
PROTOCOLS = ("udp", "tcp", "tls", "https")


def ports(group):
    base = 15000 + GROUPS.index(group) * 10
    return {"udp": base, "tcp": base, "tls": base + 1, "https": base + 2}


def validate_faults(value):
    """Returns a normalized fault table or raises ValueError (pure; unit-tested)."""
    if not isinstance(value, dict) or set(value) - {"groups", "socks", "download"}:
        raise ValueError("faults: unknown top-level key")
    result = {"groups": {}, "socks": None, "download": None}
    for group, spec in (value.get("groups") or {}).items():
        if group not in GROUPS:
            raise ValueError("faults: unknown group " + str(group))
        mode = spec.get("mode")
        if mode not in FAULT_MODES:
            raise ValueError("faults: unknown mode " + str(mode))
        protocols = spec.get("protocols") or list(PROTOCOLS)
        if set(protocols) - set(PROTOCOLS):
            raise ValueError("faults: unknown protocol")
        if mode == "slow" and not isinstance(spec.get("delay_ms"), (int, float)):
            raise ValueError("faults: slow needs delay_ms")
        if mode == "handshake_reset" and set(protocols) - {"tls", "https"}:
            raise ValueError("faults: handshake_reset applies to tls/https only")
        result["groups"][group] = dict(spec, protocols=list(protocols))
    socks = value.get("socks")
    if socks is not None:
        if socks.get("mode") not in SOCKS_MODES:
            raise ValueError("faults: unknown socks mode")
        result["socks"] = socks
    download = value.get("download")
    if download is not None:
        if download.get("mode") not in ("ok", "status", "stall", "oversize", "short", "trickle"):
            raise ValueError("faults: unknown download mode")
        result["download"] = download
    return result


def corrupt(wire, mode, spec):
    """The faulted copy of a well-formed answer (pure; unit-tested)."""
    if mode == "bad_id":
        return bytes([wire[0] ^ 0x01]) + wire[1:]
    if mode == "truncated":
        return wire[:int(spec.get("truncate", 20))]
    if mode == "wrong_question":
        # Flip the first question-name character so the question no longer matches.
        mutable = bytearray(wire)
        mutable[13] = mutable[13] ^ 0x01 if mutable[13] not in (0x2e,) else mutable[13]
        return bytes(mutable)
    if mode == "servfail":
        header = bytearray(wire[:12])
        header[3] = (header[3] & 0xF0) | 2
        header[6:12] = b"\0" * 6
        end = 12
        while wire[end]:
            end += wire[end] + 1
        return bytes(header) + wire[12:end + 5]
    return wire


class Faults:
    """Polls ``faults.json``; unchanged files cost one stat call."""

    def __init__(self, path):
        self.path = path
        self.stamp = None
        self.table = {"groups": {}, "socks": None, "download": None}
        self.version = 0

    def refresh(self):
        try:
            stat = self.path.stat()
        except FileNotFoundError:
            stamp = None
        else:
            stamp = (stat.st_mtime_ns, stat.st_size)
        if stamp == self.stamp:
            return False
        self.stamp = stamp
        if stamp is None:
            self.table = {"groups": {}, "socks": None, "download": None}
        else:
            try:
                self.table = validate_faults(json.loads(self.path.read_text()))
            except (ValueError, OSError):
                return False  # keep the last valid table; a writer may be mid-rename
        self.version += 1
        return True

    def mode(self, group, protocol):
        spec = self.table["groups"].get(group)
        if spec and protocol in spec["protocols"]:
            return spec
        return None


async def serve(root):
    journal = (root / "mock.jsonl").open("a", buffering=1)
    # Optional 5D latency model; absent file preserves the E2E fixture behavior.
    delay_path = root / "mock-delays.json"
    delays = json.loads(delay_path.read_text()) if delay_path.exists() else {}
    if set(delays) - set(GROUPS) or any(not isinstance(v, (int, float)) or v < 0 for v in delays.values()):
        raise ValueError("invalid mock delays")
    faults = Faults(root / "faults.json")
    counters = {}
    stats = (root / "mock-stats.jsonl").open("a", buffering=1) if (root / "faults.json").exists() or (root / "mock-stats").exists() else None

    def count(key):
        counters[key] = counters.get(key, 0) + 1

    async def delayed(wire, group, protocol):
        spec = faults.mode(group, protocol)
        delay = delays.get(group, 0)
        if spec and spec["mode"] == "slow":
            delay = spec["delay_ms"]
            count(f"{group}:{protocol}:fault:slow")
        await asyncio.sleep(delay / 1000)
        return respond(wire, group, protocol)

    def log(value):
        value["monotonic_s"] = time.monotonic()
        journal.write(json.dumps(value) + "\n")

    def respond(wire, group, protocol):
        decoded = decode(wire)
        count(f"{group}:{protocol}:query")
        spec = faults.mode(group, protocol)
        log({"kind": "dns", "group": group, "protocol": protocol,
             "question": decoded["questions"], "ecs": decoded["ecs"],
             **({"fault": spec["mode"]} if spec else {})})
        result = answer(wire, *MARKERS[group])
        if spec and spec["mode"] in ("bad_id", "truncated", "wrong_question", "servfail"):
            count(f"{group}:{protocol}:fault:{spec['mode']}")
            return corrupt(result, spec["mode"], spec)
        return result

    def swallow(group, protocol):
        spec = faults.mode(group, protocol)
        return spec is not None and spec["mode"] in ("blackhole", "reset", "refuse")

    class UDP(asyncio.DatagramProtocol):
        def __init__(self, group):
            self.group = group

        def connection_made(self, transport):
            self.transport = transport

        def datagram_received(self, wire, peer):
            if swallow(self.group, "udp"):
                count(f"{self.group}:udp:fault:blackhole")
                return
            spec = faults.mode(self.group, "udp")
            if delays.get(self.group, 0) or (spec and spec["mode"] == "slow"):
                async def send():
                    self.transport.sendto(await delayed(wire, self.group, "udp"), peer)
                asyncio.create_task(send())
            else:
                self.transport.sendto(respond(wire, self.group, "udp"), peer)

    live = {}  # (group, protocol) -> set of open stream writers

    async def dns_stream(reader, writer, group, protocol):
        writers = live.setdefault((group, protocol), set())
        writers.add(writer)
        count(f"{group}:{protocol}:accept")
        try:
            while True:
                length = struct.unpack("!H", await reader.readexactly(2))[0]
                wire = await reader.readexactly(length)
                spec = faults.mode(group, protocol)
                if spec and spec["mode"] == "blackhole":
                    count(f"{group}:{protocol}:fault:blackhole")
                    continue
                if spec and spec["mode"] in ("reset", "refuse"):
                    count(f"{group}:{protocol}:fault:reset")
                    return
                result = await delayed(wire, group, protocol)
                writer.write(struct.pack("!H", len(result)) + result)
                await writer.drain()
        except (asyncio.IncompleteReadError, ConnectionError, ssl.SSLError):
            pass
        finally:
            writers.discard(writer)
            writer.close()
            try:
                await writer.wait_closed()
            except (ConnectionError, ssl.SSLError):
                pass

    async def doh(reader, writer, group):
        writers = live.setdefault((group, "https"), set())
        writers.add(writer)
        count(f"{group}:https:accept")
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
                spec = faults.mode(group, "https")
                if spec and spec["mode"] == "blackhole":
                    count(f"{group}:https:fault:blackhole")
                    continue
                if spec and spec["mode"] in ("reset", "refuse"):
                    count(f"{group}:https:fault:reset")
                    return
                result = await delayed(query, group, "https")
                writer.write(b"HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: " + str(len(result)).encode() + b"\r\n\r\n" + result)
                await writer.drain()
        except (asyncio.IncompleteReadError, ConnectionError, ssl.SSLError, ValueError, KeyError):
            pass
        finally:
            writers.discard(writer)
            writer.close()
            try:
                await writer.wait_closed()
            except (ConnectionError, ssl.SSLError):
                pass

    allowed = {p for g in GROUPS for proto, p in ports(g).items() if proto != "udp"} | {DOWNLOAD_PORT}
    socks_live = set()

    async def socks(reader, writer, listener):
        remote = None
        socks_live.add(writer)
        try:
            mode = (faults.table["socks"] or {}).get("mode")
            count("socks:accept")
            if mode == "blackhole":
                count("socks:fault:blackhole")
                await reader.read()  # hold the connection open, never answer
                return
            if mode == "reset":
                count("socks:fault:reset")
                await reader.readexactly(2)
                return
            version, count_ = await reader.readexactly(2)
            methods = await reader.readexactly(count_)
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
            log({"kind": "socks", "host": host, "port": port, "allowed": permitted, "via": listener})
            if not permitted:
                writer.write(b"\x05\x02\x00\x01" + b"\0" * 6)
                await writer.drain()
                return
            rr, remote = await asyncio.open_connection("127.0.0.1", port)
            writer.write(b"\x05\x00\x00\x01\x7f\0\0\x01\0\0")
            await writer.drain()

            async def copy(source, target):
                while block := await source.read(65536):
                    if (faults.table["socks"] or {}).get("mode") == "blackhole":
                        count("socks:fault:dropped_block")  # a proxy that stopped forwarding
                        continue
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
            socks_live.discard(writer)
            for stream in (remote, writer):
                if stream:
                    stream.close()
                    try:
                        await stream.wait_closed()
                    except ConnectionError:
                        pass

    async def download(reader, writer):
        """Serves root/downloads/<name>; behavior follows faults.json `download`."""
        try:
            header = await reader.readuntil(b"\r\n\r\n")
            target = header.split(b"\r\n")[0].split()[1].decode()
            spec = faults.table["download"] or {"mode": "ok"}
            mode = spec["mode"]
            count("download:request")
            log({"kind": "download", "target": target, "mode": mode})
            path = root / "downloads" / pathlib.PurePosixPath(unquote(urlsplit(target).path)).name
            if mode == "stall":
                await reader.read()
                return
            if mode == "status":
                code = int(spec.get("code", 500))
                writer.write(f"HTTP/1.1 {code} Fault\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".encode())
            elif mode == "oversize":
                size = int(spec.get("bytes", 70 * 1024 * 1024))
                writer.write(f"HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n".encode())
                block = b"\0" * 65536
                sent = 0
                while sent < size:
                    writer.write(block[:min(len(block), size - sent)])
                    await writer.drain()
                    sent += len(block)
            elif not path.is_file():
                writer.write(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            else:
                body = path.read_bytes()
                if mode == "short":
                    writer.write(f"HTTP/1.1 200 OK\r\nContent-Length: {len(body) + 4096}\r\nConnection: close\r\n\r\n".encode() + body[:len(body) // 2])
                elif mode == "trickle":
                    writer.write(f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode())
                    for offset in range(0, len(body), 1024):
                        writer.write(body[offset:offset + 1024])
                        await writer.drain()
                        await asyncio.sleep(float(spec.get("interval_s", 0.5)))
                else:
                    writer.write(f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body)
            await writer.drain()
        except (asyncio.IncompleteReadError, ConnectionError):
            pass
        finally:
            writer.close()
            try:
                await writer.wait_closed()
            except ConnectionError:
                pass

    async def shut(server, writers):
        """Stops listening, ends the given connections and waits (bounded) for the server."""
        server.close()
        for writer in list(writers):
            writer.transport.abort()
        try:
            await asyncio.wait_for(server.wait_closed(), 3)
        except asyncio.TimeoutError:
            pass

    async def refuse_now(reader, writer):
        # handshake_reset: drop the connection before any TLS byte is exchanged.
        count("handshake_reset")
        writer.transport.abort()

    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(root / "cert.pem", root / "key.pem")
    loop = asyncio.get_running_loop()
    transports = []
    # One listener slot per (group, protocol); `desired` follows the fault table.
    slots = {}
    plain_servers = []

    def stream_callback(group, protocol):
        if protocol == "https":
            return lambda r, w, g=group: doh(r, w, g)
        return lambda r, w, g=group, proto=protocol: dns_stream(r, w, g, proto)

    async def open_slot(group, protocol, state):
        p = ports(group)[protocol]
        if state == "closed":
            return None
        if state == "abort":
            return await asyncio.start_server(refuse_now, "127.0.0.1", p)
        return await asyncio.start_server(stream_callback(group, protocol), "127.0.0.1", p, ssl=tls if protocol != "tcp" else None)

    def desired(group, protocol):
        spec = faults.mode(group, protocol)
        if spec and spec["mode"] == "refuse":
            return "closed"
        if spec and spec["mode"] == "handshake_reset":
            return "abort"
        return "normal"

    for group in GROUPS:
        p = ports(group)
        transport, _ = await loop.create_datagram_endpoint(lambda g=group: UDP(g), local_addr=("127.0.0.1", p["udp"]))
        transports.append(transport)
        for protocol in ("tcp", "tls", "https"):
            slots[(group, protocol)] = ["normal", await open_slot(group, protocol, "normal")]
    socks_slots = {}
    for port in SOCKS_PORTS:
        socks_slots[port] = await asyncio.start_server(lambda r, w, port=port: socks(r, w, port), "127.0.0.1", port)
    download_server = await asyncio.start_server(download, "127.0.0.1", DOWNLOAD_PORT) if (root / "downloads").is_dir() else None
    udp_closed = {}  # group -> transport when the udp listener is down

    async def reconcile():
        for (group, protocol), slot in slots.items():
            want = desired(group, protocol)
            if want == slot[0]:
                continue
            if slot[1] is not None:
                # Drop the live connections first: on Python >= 3.12 wait_closed() blocks
                # until every connection of the server has ended.
                await shut(slot[1], live.get((group, protocol), ()) if want != "normal" else ())
            slot[0], slot[1] = want, await open_slot(group, protocol, want)
            log({"kind": "fault-state", "group": group, "protocol": protocol, "listener": want})
        for group in GROUPS:
            spec = faults.mode(group, "udp")
            down = spec is not None and spec["mode"] == "refuse"
            if down and group not in udp_closed:
                transports[GROUPS.index(group)].close()
                udp_closed[group] = GROUPS.index(group)
                log({"kind": "fault-state", "group": group, "protocol": "udp", "listener": "closed"})
            elif not down and group in udp_closed:
                transport, _ = await loop.create_datagram_endpoint(lambda g=group: UDP(g), local_addr=("127.0.0.1", ports(group)["udp"]))
                transports[udp_closed.pop(group)] = transport
                log({"kind": "fault-state", "group": group, "protocol": "udp", "listener": "normal"})
        socks_mode = (faults.table["socks"] or {}).get("mode")
        for port in SOCKS_PORTS:
            server = socks_slots[port]
            if socks_mode == "down" and server is not None:
                await shut(server, socks_live)
                socks_slots[port] = None
                log({"kind": "fault-state", "socks": port, "listener": "closed"})
            elif socks_mode != "down" and server is None:
                socks_slots[port] = await asyncio.start_server(lambda r, w, port=port: socks(r, w, port), "127.0.0.1", port)
                log({"kind": "fault-state", "socks": port, "listener": "normal"})

    async def watch():
        while True:
            try:
                if faults.refresh():
                    log({"kind": "faults", "version": faults.version, "table": faults.table})
                    await reconcile()
            except Exception as error:  # keep polling; the journal records why a change failed
                log({"kind": "fault-error", "error": repr(error)})
            await asyncio.sleep(0.05)

    async def flush():
        while stats is not None:
            await asyncio.sleep(1)
            stats.write(json.dumps({"t": time.monotonic(), "counters": counters}) + "\n")

    watcher = asyncio.create_task(watch())
    flusher = asyncio.create_task(flush())
    stop = asyncio.Event()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stop.set)
    (root / "mock-ready").touch()
    await stop.wait()
    watcher.cancel()
    flusher.cancel()
    for (group, protocol), slot in slots.items():
        if slot[1] is not None:
            await shut(slot[1], live.get((group, protocol), ()))
    for server in list(socks_slots.values()) + [download_server]:
        if server is not None:
            await shut(server, socks_live)
    for transport in transports:
        transport.close()
    journal.close()
    if stats is not None:
        stats.write(json.dumps({"t": time.monotonic(), "counters": counters}) + "\n")
        stats.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=pathlib.Path)
    asyncio.run(serve(parser.parse_args().root))
