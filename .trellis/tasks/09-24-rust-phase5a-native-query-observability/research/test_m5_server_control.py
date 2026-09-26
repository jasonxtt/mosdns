import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch


class OwnedProcessTests(unittest.TestCase):
    def test_exit_between_identity_check_and_pidfd_is_benign(self):
        spec = importlib.util.spec_from_file_location('control_exit', Path(__file__).with_name('m5-server-control.py'))
        control = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(control)
        with patch.object(control, 'process_start', return_value='123'), patch.object(control.os, 'pidfd_open', create=True, side_effect=ProcessLookupError):
            control.terminate_owned({'pid': 123, 'start_identity': '123'})

    def test_changed_pid_identity_cannot_be_signalled(self):
        spec = importlib.util.spec_from_file_location('server_control', Path(__file__).with_name('m5-server-control.py'))
        control = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(control)
        with patch.object(control, 'process_start', return_value='new-process'), patch.object(control.os, 'kill') as kill:
            with self.assertRaisesRegex(ValueError, 'ownership'):
                control.terminate_owned({'pid': 123, 'start_identity': 'old-process'})
            kill.assert_not_called()


if __name__ == '__main__':
    unittest.main()
