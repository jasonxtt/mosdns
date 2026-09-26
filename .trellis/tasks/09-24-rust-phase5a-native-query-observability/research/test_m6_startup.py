import importlib.util
import socket
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


def control():
    spec = importlib.util.spec_from_file_location('m6_control', Path(__file__).with_name('m6-server-control.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AvailabilityTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform.startswith('linux'), 'real Linux TIME_WAIT')
    def test_time_wait_after_active_server_close_allows_next_probe(self):
        module = control()
        with socket.socket() as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(('127.0.0.1', 0)); listener.listen()
            address = listener.getsockname()
            with socket.create_connection(address) as client:
                accepted, _ = listener.accept()
                accepted.close()
                self.assertEqual(client.recv(1), b'')
        with socket.socket() as plain:
            with self.assertRaises(OSError):
                plain.bind(address)
        for _ in range(3):
            module.probe_available(address)

    def test_active_listener_still_rejects_probe(self):
        module = control()
        with socket.socket() as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(('127.0.0.1', 0)); listener.listen()
            with self.assertRaises(OSError):
                module.probe_available(listener.getsockname())

    def test_early_failure_is_retained_in_fresh_session(self):
        module = control()
        with tempfile.TemporaryDirectory() as temp, patch.object(module.socket, 'gethostname', return_value='wrong-host'):
            root = Path(temp) / 'm6-failed'
            with self.assertRaisesRegex(ValueError, 'host'):
                module.start(root)
            self.assertEqual((root / 'owned.json').read_text(), '{}')
            self.assertIn('host', (root / 'startup-error.txt').read_text())


if __name__ == '__main__':
    unittest.main()
