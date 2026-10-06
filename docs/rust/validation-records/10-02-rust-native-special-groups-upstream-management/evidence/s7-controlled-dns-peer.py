#!/usr/bin/env python3
"""Controlled high-port loopback DNS peer for the S7 live proof."""

import argparse
import json
import signal
import socket
import socketserver
import threading
import time


def question_end(packet):
    position = 12
    labels = []
    while position < len(packet):
        size = packet[position]
        position += 1
        if size == 0:
            break
        if size & 0xC0:
            raise ValueError("compressed query name is unsupported")
        labels.append(packet[position:position + size].decode("ascii", "replace"))
        position += size
    position += 4
    if position > len(packet):
        raise ValueError("truncated DNS question")
    return ".".join(labels).lower(), position


def make_response(packet, answer_ip):
    qname, end = question_end(packet)
    flags = int.from_bytes(packet[2:4], "big") | 0x8080
    header = packet[:2] + flags.to_bytes(2, "big") + b"\x00\x01\x00\x01\x00\x00\x00\x00"
    answer = b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x3c\x00\x04" + socket.inet_aton(answer_ip)
    return qname, header + packet[12:end] + answer


def append_log(path, record):
    with open(path, "a", encoding="utf-8") as output:
        output.write(json.dumps(record, sort_keys=True) + "\n")


class UdpHandler(socketserver.BaseRequestHandler):
    def handle(self):
        packet, sock = self.request
        self.server.record(packet, self.client_address)
        try:
            _, response = make_response(packet, self.server.answer_ip)
            sock.sendto(response, self.client_address)
        except (OSError, ValueError):
            return


class TcpHandler(socketserver.StreamRequestHandler):
    def handle(self):
        while True:
            prefix = self.rfile.read(2)
            if len(prefix) != 2:
                return
            length = int.from_bytes(prefix, "big")
            packet = self.rfile.read(length)
            if len(packet) != length:
                return
            self.server.record(packet, self.client_address)
            try:
                _, response = make_response(packet, self.server.answer_ip)
            except (OSError, ValueError):
                return
            self.wfile.write(len(response).to_bytes(2, "big") + response)
            self.wfile.flush()


class UdpServer(socketserver.ThreadingUDPServer):
    allow_reuse_address = True
    daemon_threads = True


class TcpServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--transport", choices=("udp", "tcp"), required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--answer", required=True)
    parser.add_argument("--peer-id", required=True)
    parser.add_argument("--log", required=True)
    args = parser.parse_args()
    server_type = UdpServer if args.transport == "udp" else TcpServer
    server = server_type(("127.0.0.1", args.port), UdpHandler if args.transport == "udp" else TcpHandler)
    server.answer_ip = args.answer
    log_lock = threading.Lock()

    def record(packet, client):
        try:
            qname, _ = question_end(packet)
        except ValueError:
            qname = "<malformed>"
        with log_lock:
            append_log(args.log, {
                "time": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "peer_id": args.peer_id,
                "transport": args.transport,
                "qname": qname,
                "client": f"{client[0]}:{client[1]}",
                "answer": args.answer,
            })

    server.record = record
    signal.signal(signal.SIGTERM, lambda *_: threading.Thread(target=server.shutdown, daemon=True).start())
    signal.signal(signal.SIGINT, lambda *_: threading.Thread(target=server.shutdown, daemon=True).start())
    print(f"{args.peer_id} {args.transport} listening 127.0.0.1:{args.port} -> {args.answer}", flush=True)
    server.serve_forever(poll_interval=0.2)
    server.server_close()


if __name__ == "__main__":
    main()
