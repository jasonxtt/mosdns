import importlib.util
import subprocess
import unittest
from pathlib import Path


def module(file):
    spec = importlib.util.spec_from_file_location(file, Path(__file__).with_name(file))
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


class GenerationTests(unittest.TestCase):
    def test_fresh_roots_and_ids_are_disjoint_from_m5(self):
        old, new = module('run-m5-w1.py'), module('run-m6-w1.py')
        old_ids = {r['run_id'] for rows in old.make_plan(Path('/old')).values() for r in rows}
        new_ids = {r['run_id'] for rows in new.make_plan(Path('/new')).values() for r in rows}
        self.assertEqual(len(new_ids), 18)
        self.assertFalse(old_ids & new_ids)
        for key in ('SERVER_INPUT', 'SERVER_RESULTS', 'CLIENT_BASE'):
            self.assertNotEqual(getattr(old, key), getattr(new, key))
        for batch, rows in new.make_plan(Path('/new')).items():
            self.assertEqual([(r['repetition'], r['variant']) for r in rows],
                             [(r['repetition'], r['variant']) for r in old.make_plan(Path('/old'))[batch]])

    def test_remote_stderr_retained(self):
        new = module('run-m6-w1.py')
        failure = subprocess.CalledProcessError(1, ['ssh', 'test-start'], output='startup output', stderr='EADDRINUSE diagnostic')
        evidence = new.error_evidence(failure)
        self.assertIn('startup output', evidence)
        self.assertIn('EADDRINUSE diagnostic', evidence)


if __name__ == '__main__':
    unittest.main()
