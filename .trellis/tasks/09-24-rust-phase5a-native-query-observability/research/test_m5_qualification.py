import importlib.util
import unittest
from pathlib import Path


class CoverageTests(unittest.TestCase):
    def test_short_window_and_gc_enabled_profile_rejected(self):
        spec = importlib.util.spec_from_file_location('qualification', Path(__file__).with_name('qualify-m5-w1.py'))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        stage = {'harness_go_profile': dict(GOMAXPROCS='1', GOGC='off', GODEBUG='gctrace=1', GOMEMLIMIT=''),
                 'server_resources': {'bracket_seconds': 26, 'counts': {'sut': 26, 'fixture-1': 26}},
                 'resource_sample_counts': {'sut': 26, 'fixture-1': 26, 'load-generator': 25}}
        module.check_coverage(stage)
        stage['server_resources']['bracket_seconds'] = 2
        with self.assertRaisesRegex(ValueError, 'coverage'):
            module.check_coverage(stage)
        stage['server_resources']['bracket_seconds'] = 26
        stage['harness_go_profile']['GOGC'] = '100'
        with self.assertRaisesRegex(ValueError, 'profile'):
            module.check_coverage(stage)


if __name__ == '__main__':
    unittest.main()
