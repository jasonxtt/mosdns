#!/usr/bin/env python3
"""Controlled UDP DNS peer for the native/Vue browser proof."""

import socket


def response(query: bytes) -> bytes:
    question_end = 12
    while query[question_end] != 0:
        question_end += 1 + query[question_end]
    question_end += 5
    question = query[12:question_end]
    return (
        query[:2]
        + b"\x81\x80"
        + b"\x00\x01\x00\x01\x00\x00\x00\x00"
        + question
        + b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x3c\x00\x04\xc0\x00\x02\x7b"
    )


with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
    peer.bind(("127.0.0.1", 15453))
    while True:
        query, address = peer.recvfrom(4096)
        peer.sendto(response(query), address)
