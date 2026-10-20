"""Synthetic fixtures only; no captured traffic belongs in this directory."""
import io
import struct
import unittest
import collections

from protocol import Pcapng, decode_dns, semantic, udp4


def query():
    return struct.pack("!6H", 7, 0x100, 1, 0, 0, 0) + b"\x07example\x03com\0\0\x01\0\x01"


def response(address):
    q = query()
    return q[:2] + struct.pack("!5H", 0x8180, 1, 1, 0, 0) + q[12:] + b"\xc0\x0c" + struct.pack("!HHIH", 1, 1, 60, 4) + address


def block(kind, body):
    body += b"\0" * (-len(body) % 4)
    size = len(body) + 12
    return struct.pack("<II", kind, size) + body + struct.pack("<I", size)


class ProtocolTests(unittest.TestCase):
    def test_repeated_question_audit_uses_actual_round_trip_window(self):
        from run import Experiment
        experiment = Experiment.__new__(Experiment)
        experiment.counts = collections.Counter()
        key = ("192.0.2.9", "6578616d706c652e636f6d", "A")
        experiment.pending = {1: {"rust": {}, "audit_key": key, "sent_wall": 100.01, "received_wall": 100.011}}
        experiment.audit = [{"key": key, "wall": 100.0005, "record": {"trace_id": "previous"}},
                            {"key": key, "wall": 100.0105, "record": {"trace_id": "current"}},
                            {"key": key, "wall": 100.0205, "record": {"trace_id": "next"}}]
        experiment.match_audit()
        self.assertEqual(experiment.pending[1]["audit"]["trace_id"], "current")

    def test_semantics_ignore_concrete_real_address(self):
        a = decode_dns(response(b"\xc0\x00\x02\x01"))
        b = decode_dns(response(b"\xcb\x00\x71\x02"))
        self.assertEqual(semantic(a, ["198.51.100.0/24"]), semantic(b, ["198.51.100.0/24"]))
        c = decode_dns(response(b"\xc6\x33\x64\x01"))
        self.assertNotEqual(semantic(a, ["198.51.100.0/24"]), semantic(c, ["198.51.100.0/24"]))

    def test_compression_loop_and_truncation_refused(self):
        with self.assertRaises(ValueError):
            decode_dns(query()[:12] + b"\xc0\x0c\0\x01\0\x01")
        with self.assertRaises(ValueError):
            decode_dns(response(b"\xc0\x00\x02\x01")[:-1])

    def test_pcapng_ethernet_and_truncated_block(self):
        ip = bytes.fromhex("450000390000000040110000c0000201c6336401")
        packet = b"\0" * 12 + b"\x08\0" + ip + struct.pack("!4H", 12345, 53, len(query()) + 8, 0) + query()
        section = block(0x0A0D0D0A, struct.pack("<IHHq", 0x1A2B3C4D, 1, 0, -1))
        interface = block(1, struct.pack("<HHI", 1, 0, 65535))
        enhanced = block(6, struct.pack("<5I", 0, 0, 100, len(packet), len(packet)) + packet)
        rows = list(Pcapng(io.BytesIO(section + interface + enhanced)))
        self.assertEqual(rows[0][1], packet)
        self.assertEqual(udp4(packet, 1)[0:4], ("192.0.2.1", 12345, "198.51.100.1", 53))
        with self.assertRaises(ValueError):
            list(Pcapng(io.BytesIO(section + interface + enhanced[:-1])))


if __name__ == "__main__":
    unittest.main()
