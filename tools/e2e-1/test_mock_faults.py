import json
import pathlib
import socket
import struct
import subprocess
import sys
import tempfile
import time
import unittest

import dnswire
import mock

HERE = pathlib.Path(__file__).resolve().parent


class Pure(unittest.TestCase):
    def query(self, name="a.example.com", qtype=1, ident=0x1234):
        return dnswire.question(name, qtype, ident)

    def test_validation_rejects_unknown_shapes(self):
        for bad in ({"groups": {"nope": {"mode": "blackhole"}}}, {"groups": {"domestic": {"mode": "melt"}}},
                    {"groups": {"domestic": {"mode": "slow"}}}, {"socks": {"mode": "x"}}, {"x": 1},
                    {"groups": {"domestic": {"mode": "handshake_reset", "protocols": ["udp"]}}},
                    {"download": {"mode": "x"}}):
            with self.assertRaises(ValueError):
                mock.validate_faults(bad)
        table = mock.validate_faults({"groups": {"domestic": {"mode": "slow", "delay_ms": 5, "protocols": ["udp"]}}})
        self.assertEqual(table["groups"]["domestic"]["protocols"], ["udp"])

    def test_answer_corruptions_are_what_they_claim(self):
        query = self.query()
        good = dnswire.decode(mock.answer(query, *mock.MARKERS["domestic"]))
        bad_id = dnswire.decode(mock.corrupt(mock.answer(query, *mock.MARKERS["domestic"]), "bad_id", {}))
        self.assertNotEqual(bad_id["id"], good["id"])
        self.assertEqual(bad_id["answers"], good["answers"])
        servfail = dnswire.decode(mock.corrupt(mock.answer(query, *mock.MARKERS["domestic"]), "servfail", {}))
        self.assertEqual((servfail["rcode"], servfail["id"], servfail["questions"], servfail["answers"]),
                         (2, good["id"], good["questions"], []))
        wrong = dnswire.decode(mock.corrupt(mock.answer(query, *mock.MARKERS["domestic"]), "wrong_question", {}))
        self.assertNotEqual(wrong["questions"], good["questions"])
        cut = mock.corrupt(mock.answer(query, *mock.MARKERS["domestic"]), "truncated", {"truncate": 17})
        self.assertEqual(len(cut), 17)
        with self.assertRaises(Exception):
            dnswire.decode(cut)

    def test_polling_keeps_the_last_valid_table_and_restores_on_removal(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "faults.json"
            faults = mock.Faults(path)
            self.assertFalse(faults.refresh())
            path.write_text(json.dumps({"groups": {"foreign": {"mode": "blackhole"}}}))
            self.assertTrue(faults.refresh())
            self.assertEqual(faults.mode("foreign", "tcp")["mode"], "blackhole")
            self.assertIsNone(faults.mode("domestic", "tcp"))
            path.write_text("{ not json")
            faults.refresh()
            self.assertEqual(faults.mode("foreign", "udp")["mode"], "blackhole", "a torn write keeps the old table")
            path.unlink()
            self.assertTrue(faults.refresh())
            self.assertIsNone(faults.mode("foreign", "udp"))


class Live(unittest.TestCase):
    """The real mock process: normal answers, each fault mode and recovery."""

    @classmethod
    def setUpClass(cls):
        cls.root = pathlib.Path(tempfile.mkdtemp(prefix="mockfault-"))
        quiet = dict(stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", cls.root / "key.pem",
                        "-out", cls.root / "cert.pem", "-days", "1", "-subj", "/CN=localhost"], **quiet)
        (cls.root / "faults.json").write_text("{}")
        cls.process = subprocess.Popen([sys.executable, str(HERE / "mock.py"), str(cls.root)])
        end = time.monotonic() + 15
        while not (cls.root / "mock-ready").exists():
            if time.monotonic() > end:
                raise RuntimeError("mock not ready")
            time.sleep(0.05)

    @classmethod
    def tearDownClass(cls):
        cls.process.terminate()
        cls.process.wait(10)

    def inject(self, table):
        path = self.root / "faults.json"
        path.write_text(json.dumps(table))
        time.sleep(0.4)  # the mock polls every 50 ms; listeners are re-bound

    def clear(self):
        self.inject({})

    def udp(self, group="domestic", timeout=1.0, ident=7):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(timeout)
            sock.connect(("127.0.0.1", mock.ports(group)["udp"]))
            sock.send(dnswire.question("x.example.com", 1, ident))
            return sock.recv(65535)

    def tcp(self, group="domestic", timeout=1.0):
        with socket.create_connection(("127.0.0.1", mock.ports(group)["tcp"]), timeout=timeout) as sock:
            query = dnswire.question("x.example.com", 1, 9)
            sock.sendall(struct.pack("!H", len(query)) + query)
            length = struct.unpack("!H", sock.recv(2))[0]
            return sock.recv(length)

    def tearDown(self):
        self.clear()

    def test_normal_blackhole_and_recovery(self):
        self.assertEqual(dnswire.decode(self.udp())["answers"][0][3], "223.5.5.101")
        self.inject({"groups": {"domestic": {"mode": "blackhole"}}})
        with self.assertRaises(socket.timeout):
            self.udp()
        with self.assertRaises(socket.timeout):
            self.tcp()
        self.assertEqual(dnswire.decode(self.udp("foreign"))["answers"][0][3], "203.0.113.20", "other groups unaffected")
        self.clear()
        self.assertEqual(dnswire.decode(self.udp())["id"], 7)
        self.assertEqual(dnswire.decode(self.tcp())["id"], 9)

    def test_refuse_reset_and_corrupt_answers(self):
        self.inject({"groups": {"domestic": {"mode": "refuse", "protocols": ["tcp"]}}})
        with self.assertRaises(ConnectionRefusedError):
            self.tcp()
        self.assertEqual(dnswire.decode(self.udp())["id"], 7, "udp was not in the fault's protocols")
        self.clear()
        self.assertEqual(dnswire.decode(self.tcp())["id"], 9)
        self.inject({"groups": {"domestic": {"mode": "reset", "protocols": ["tcp"]}}})
        with self.assertRaises((ConnectionError, struct.error, socket.timeout)):
            self.tcp()
        self.inject({"groups": {"domestic": {"mode": "bad_id"}}})
        self.assertNotEqual(dnswire.decode(self.udp())["id"], 7)
        self.inject({"groups": {"domestic": {"mode": "servfail"}}})
        self.assertEqual(dnswire.decode(self.udp())["rcode"], 2)
        self.inject({"groups": {"domestic": {"mode": "refuse", "protocols": ["udp"]}}})
        with self.assertRaises(ConnectionRefusedError):
            self.udp()
        self.clear()
        self.assertEqual(dnswire.decode(self.udp())["id"], 7)

    def test_socks_down_drops_live_tunnels_and_handshake_reset_aborts_before_tls(self):
        import ssl
        # A live SOCKS5 tunnel to the domestic TCP listener, established before the fault.
        tunnel = socket.create_connection(("127.0.0.1", 17891), timeout=2)
        tunnel.sendall(b"\x05\x01\x00")
        self.assertEqual(tunnel.recv(2), b"\x05\x00")
        tunnel.sendall(b"\x05\x01\x00\x01\x7f\x00\x00\x01" + struct.pack("!H", mock.ports("domestic")["tcp"]))
        self.assertEqual(tunnel.recv(10)[1], 0)
        self.inject({"socks": {"mode": "down"}})
        with self.assertRaises(ConnectionRefusedError):
            socket.create_connection(("127.0.0.1", 17891), timeout=1)
        tunnel.settimeout(2)
        try:
            self.assertEqual(tunnel.recv(1), b"", "the live tunnel was closed")
        except ConnectionError:
            pass
        tunnel.close()
        self.clear()
        socket.create_connection(("127.0.0.1", 17891), timeout=1).close()
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        self.inject({"groups": {"foreign": {"mode": "handshake_reset", "protocols": ["https"]}}})
        with self.assertRaises((ssl.SSLError, ConnectionError, OSError)):
            with socket.create_connection(("127.0.0.1", mock.ports("foreign")["https"]), timeout=2) as raw:
                context.wrap_socket(raw)
        self.clear()
        with socket.create_connection(("127.0.0.1", mock.ports("foreign")["https"]), timeout=2) as raw:
            context.wrap_socket(raw).close()

    def test_socks_blackhole_freezes_live_tunnels_too(self):
        tunnel = socket.create_connection(("127.0.0.1", 17891), timeout=2)
        tunnel.sendall(b"\x05\x01\x00")
        self.assertEqual(tunnel.recv(2), b"\x05\x00")
        tunnel.sendall(b"\x05\x01\x00\x01\x7f\x00\x00\x01" + struct.pack("!H", mock.ports("domestic")["tcp"]))
        self.assertEqual(tunnel.recv(10)[1], 0)
        query = dnswire.question("x.example.com", 1, 11)
        tunnel.sendall(struct.pack("!H", len(query)) + query)
        self.assertEqual(struct.unpack("!H", tunnel.recv(2))[0] > 0, True, "forwarded while healthy")
        tunnel.recv(65535)
        self.inject({"socks": {"mode": "blackhole"}})
        tunnel.sendall(struct.pack("!H", len(query)) + query)
        tunnel.settimeout(1)
        with self.assertRaises(socket.timeout):
            tunnel.recv(2)
        # A new connection is accepted but never greeted.
        fresh = socket.create_connection(("127.0.0.1", 17891), timeout=2)
        fresh.sendall(b"\x05\x01\x00")
        fresh.settimeout(1)
        with self.assertRaises(socket.timeout):
            fresh.recv(2)
        fresh.close()
        tunnel.close()

    def test_slow_mode_and_counters(self):
        self.inject({"groups": {"domestic": {"mode": "slow", "delay_ms": 300}}})
        began = time.monotonic()
        self.udp(timeout=2)
        self.assertGreaterEqual(time.monotonic() - began, 0.28)
        self.clear()
        began = time.monotonic()
        self.udp()
        self.assertLess(time.monotonic() - began, 0.2)


if __name__ == "__main__":
    unittest.main()
