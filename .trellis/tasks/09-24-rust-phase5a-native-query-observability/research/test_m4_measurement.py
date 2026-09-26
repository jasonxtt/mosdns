import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from test_m3_measurement import M3Controls, RunnerPlan, qualify


class M4Controls(unittest.TestCase):
    def setUp(self):
        fixture = M3Controls()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.root = fixture.root
        for batch in ('batch1', 'batch2'):
            for attempt in (self.root / batch).glob('*-r*-*'):
                (attempt / 'environment.txt').write_text('measurement_profile=m4\ngomaxprocs_environment=1\ngogc_environment=off\ngodebug_environment=gctrace=1\ngomemlimit_environment=unset\n')
                count = 3 if attempt.name.startswith('w3-') else 1
                peaks = {'load-generator': 16000, **{f'fixture-{i}': 16000 for i in range(1, count + 1)}}
                (attempt / 'gc-evidence.json').write_text(json.dumps({'qualified': True, 'gc_trace_lines': 0, 'sampled_rss_peaks_kib': peaks, 'logs': [{'gc_trace_lines': 0}] * (count + 1)}))

    def test_complete_fixed_controls_qualify(self):
        result = qualify.qualify(self.root, profile='m4')
        self.assertTrue(result['qualified'], result['failures'])

    def test_gc_profile_evidence_and_resource_cap_fail_closed(self):
        attempt = self.root / 'batch1' / 'w1-tcp-r1-before_off'
        path = attempt / 'gc-evidence.json'
        original = json.loads(path.read_text())
        for value in (dict(original, gc_trace_lines=1),
                      dict(original, sampled_rss_peaks_kib={'load-generator': 300000, 'fixture-1': 10000})):
            path.write_text(json.dumps(value))
            self.assertFalse(qualify.qualify(self.root, profile='m4')['qualified'])
        path.write_text(json.dumps(original))
        (attempt / 'environment.txt').write_text('measurement_profile=m4\ngomaxprocs_environment=1\ngogc_environment=100\n')
        self.assertFalse(qualify.qualify(self.root, profile='m4')['qualified'])


class M4Runner(RunnerPlan):
    def test_gc_settings_required_and_primary_plan_preserved(self):
        self.assertEqual(self.run_plan('m4').returncode, 2)
        result = self.run_plan('m4', GOGC='off', GODEBUG='gctrace=1', GOMEMLIMIT='')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ['normal-reference:200', 'overload:400'])


class TraceEvidence(unittest.TestCase):
    def test_trace_detection_and_memory_cap(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            directory = root / '.run.test'
            directory.mkdir()
            (directory / 'forward.stderr').write_text('')
            (root / 'pilot.stderr.log').write_text('')
            samples = [{'role': role, 'rss_kib': 16000} for role in ('load-generator', 'fixture-1')]
            (root / 'resource-samples.jsonl').write_text('\n'.join(json.dumps(s) for s in samples))
            script = Path(__file__).with_name('summarize-m4-gc.py')
            def evidence():
                result = subprocess.run(['python3', str(script), str(root), 'w1-tcp'], capture_output=True, text=True, check=True)
                return json.loads(result.stdout)
            self.assertTrue(evidence()['qualified'])
            (directory / 'forward.stderr').write_text('gc 1 @0.1s 1%: something\n')
            self.assertFalse(evidence()['qualified'])


if __name__ == '__main__':
    unittest.main()
