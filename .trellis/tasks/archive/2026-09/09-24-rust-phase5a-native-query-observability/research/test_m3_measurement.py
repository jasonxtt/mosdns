import csv
import os
import subprocess
import unittest
from pathlib import Path
from test_qualify_m2_controls import CompleteControlGateTests, qualify
from repo_paths import repo_root


class M3Controls(CompleteControlGateTests):
    def setUp(self):
        super().setUp()
        for batch in ('batch1', 'batch2'):
            directory = self.root / batch
            for attempt in directory.glob('*-r*-*'):
                (attempt / 'environment.txt').write_text('measurement_profile=m3\ngomaxprocs_environment=1\n')
                (attempt / 'run-metadata.txt').write_text('stage_duration_ms=25000\nnormal_reference_qps=200\noverload_qps=400\nrequest_deadline_ms=500\nlate_drain_ms=100\nw2_warm_lifecycle=independent-prefilled\n')
            path = directory / 'derived-primary-measurements.tsv'
            with path.open() as f:
                rows = list(csv.DictReader(f, delimiter='\t'))
            for row in rows:
                for key in ('scheduled', 'sent', 'received', 'correct_on_time'):
                    row[key] = int(row['qps']) * 25
            self.write(path, rows)

    def test_complete_identical_controls_qualify(self):
        result = qualify.qualify(self.root, profile='m3')
        self.assertTrue(result['qualified'], result['failures'])

    def test_single_crossing_blocks_even_if_all_intervals_are_exact(self):
        path = self.root / 'batch1' / 'derived-paired-assessments.tsv'
        with path.open() as f:
            rows = list(csv.DictReader(f, delimiter='\t'))
        rows[0]['pairs_above_guard'] = 1
        self.write(path, rows)
        self.assertFalse(qualify.qualify(self.root, profile='m3')['qualified'])

    def test_wrong_runtime_profile_blocks(self):
        path = self.root / 'batch2' / 'w1-tcp-r1-before_off' / 'environment.txt'
        path.write_text('measurement_profile=m3\ngomaxprocs_environment=2\n')
        self.assertFalse(qualify.qualify(self.root, profile='m3')['qualified'])

    def test_duplicate_and_missing_primary_row_blocks(self):
        path = self.root / 'batch1' / 'derived-primary-measurements.tsv'
        with path.open() as f:
            rows = list(csv.DictReader(f, delimiter='\t'))
        rows[0] = rows[1]
        self.write(path, rows)
        self.assertFalse(qualify.qualify(self.root, profile='m3')['qualified'])

    def test_short_window_blocks(self):
        path = self.root / 'batch1' / 'w1-tcp-r1-before_off' / 'run-metadata.txt'
        path.write_text(path.read_text().replace('25000', '3000'))
        self.assertFalse(qualify.qualify(self.root, profile='m3')['qualified'])

    def test_fixed_w1_subset_qualifies(self):
        for batch in ('batch1', 'batch2'):
            for name in ('derived-primary-measurements.tsv', 'derived-paired-assessments.tsv'):
                path = self.root / batch / name
                with path.open() as f:
                    rows = list(csv.DictReader(f, delimiter='\t'))
                self.write(path, [r for r in rows if r['scenario'] == 'w1-tcp'])
        result = qualify.qualify(self.root, groups=qualify.GROUPS[:2], profile='m3')
        self.assertTrue(result['qualified'], result['failures'])
        self.assertEqual(len(result['intervals']), 8)
        self.assertFalse(qualify.qualify(self.root, profile='m3')['qualified'])


class RunnerPlan(unittest.TestCase):
    def run_plan(self, profile, **extra):
        runner = repo_root(Path(__file__)) / 'scripts/run-phase5a-baseline.sh'
        env = dict(os.environ, PHASE5A_PLAN_ONLY='1', PHASE5A_MEASUREMENT_PROFILE=profile,
                   GOMAXPROCS='1', RUN_MODE='pilot', CANDIDATE='rust', SCENARIO='w2',
                   STAGE_DURATION_MS='25000', NORMAL_REFERENCE_QPS='200', COMMON_LOAD_QPS='300',
                   NEAR_SATURATION_QPS='350', OVERLOAD_QPS='400', REQUEST_DEADLINE_MS='500',
                   LATE_DRAIN_MS='100', W2_WARM_LIFECYCLE='independent-prefilled',
                   W2_CACHE_TTL_MS='30000', W2_TTL_SAFETY_MARGIN_MS='500')
        env.update(extra)
        return subprocess.run(['bash', str(runner)], env=env, capture_output=True, text=True)

    def test_m3_exact_primary_points(self):
        result = self.run_plan('m3')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ['normal-reference:200', 'overload:400'])

    def test_legacy_and_m2_keep_five_stages(self):
        for profile in ('legacy', 'm2'):
            result = self.run_plan(profile)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(result.stdout.splitlines()), 5)
            self.assertEqual(result.stdout.splitlines()[-1], 'recovery:200')

    def test_m3_bad_window_parallelism_or_lifecycle_rejected(self):
        for override in ({'STAGE_DURATION_MS': '3000'}, {'GOMAXPROCS': '2'}, {'W2_WARM_LIFECYCLE': 'same-process'}):
            self.assertEqual(self.run_plan('m3', **override).returncode, 2)


if __name__ == '__main__':
    unittest.main()
