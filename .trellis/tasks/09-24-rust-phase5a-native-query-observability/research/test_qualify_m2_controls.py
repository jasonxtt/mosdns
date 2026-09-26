import importlib.util
import math
import csv
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('qualify', Path(__file__).with_name('qualify-m2-controls.py'))
qualify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qualify)


class ControlPrecisionTests(unittest.TestCase):
    def test_identical_pairs_are_equivalent(self):
        self.assertEqual(qualify.interval([0.0] * 6), (0.0, 0.0))

    def test_zero_center_with_wide_variance_is_not_stable(self):
        low, high = qualify.interval([math.log(1.4), math.log(1 / 1.4)] * 3)
        self.assertLess(low, -qualify.MARGIN)
        self.assertGreater(high, qualify.MARGIN)

    def test_consistent_offset_outside_margin_is_not_stable(self):
        low, high = qualify.interval([math.log(1.2)] * 6)
        self.assertGreaterEqual(low, qualify.MARGIN)
        self.assertGreaterEqual(high, qualify.MARGIN)

    def test_wrong_sample_count_or_nonfinite_input_is_rejected(self):
        for values in ([0.0] * 5, [float('nan')] * 6, [float('inf')] * 6):
            with self.assertRaises(ValueError):
                qualify.interval(values)


class CompleteControlGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for batch in ('batch1', 'batch2'):
            directory = self.root / batch
            directory.mkdir()
            rows = []
            pairs = []
            for scenario, phase, qps in qualify.GROUPS:
                for variant in qualify.VARIANTS:
                    for rep in range(1, 4):
                        rows.append(dict(scenario=scenario, phase=phase, qps=qps,
                                         variant=variant, repetition=rep, valid=1,
                                         runner_exit=0, scheduled=100, sent=100,
                                         received=100, correct_on_time=100,
                                         correct_late=0, wrong_response=0,
                                         protocol_error=0, transport_error=0,
                                         timeout=0, sender_shortfall=0,
                                         p95_us=100, p99_us=100))
                        attempt = directory / f'{scenario}-r{rep}-{variant}'
                        attempt.mkdir(exist_ok=True)
                        (attempt / 'environment.txt').write_text('measurement_profile=m2\ngomaxprocs_environment=1\n')
                        (attempt / 'sut.json').write_text(json.dumps({'sha256': '370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa'}))
                for comparison in ('after_off_vs_before_off', 'after_on_vs_after_off'):
                    for metric in ('p95_us', 'p99_us', 'cpu_us_per_correct_query', 'rss_peak_kib_sampled'):
                        pairs.append(dict(scenario=scenario, phase=phase, qps=qps,
                                          comparison=comparison, metric=metric,
                                          valid_pairs=3, pairs_above_guard=0,
                                          verdict='no repeatable regression under frozen guard'))
            self.write(directory / 'derived-primary-measurements.tsv', rows)
            self.write(directory / 'derived-paired-assessments.tsv', pairs)

    @staticmethod
    def write(path, rows):
        with path.open('w') as f:
            writer = csv.DictWriter(f, list(rows[0]), delimiter='\t')
            writer.writeheader()
            writer.writerows(rows)

    def test_complete_identical_controls_qualify(self):
        result = qualify.qualify(self.root)
        self.assertTrue(result['qualified'], result['failures'])
        self.assertEqual(len(result['intervals']), 28)

    def test_single_crossing_blocks_even_if_all_intervals_are_exact(self):
        path = self.root / 'batch1' / 'derived-paired-assessments.tsv'
        with path.open() as f:
            rows = list(csv.DictReader(f, delimiter='\t'))
        rows[0]['pairs_above_guard'] = 1
        self.write(path, rows)
        result = qualify.qualify(self.root)
        self.assertFalse(result['qualified'])
        self.assertTrue(all(row['qualified'] for row in result['intervals']))

    def test_wrong_runtime_profile_blocks(self):
        path = self.root / 'batch2' / 'w1-tcp-r1-before_off' / 'environment.txt'
        path.write_text('measurement_profile=m2\ngomaxprocs_environment=2\n')
        self.assertFalse(qualify.qualify(self.root)['qualified'])

    def test_duplicate_and_missing_primary_row_blocks(self):
        path = self.root / 'batch1' / 'derived-primary-measurements.tsv'
        with path.open() as f:
            rows = list(csv.DictReader(f, delimiter='\t'))
        rows[0] = rows[1]
        self.write(path, rows)
        self.assertFalse(qualify.qualify(self.root)['qualified'])


if __name__ == '__main__':
    unittest.main()
