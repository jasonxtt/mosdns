import gzip
import json
import pathlib
import struct
import tempfile
import unittest
from unittest import mock

from longrun_store import HourStore
from safety import Safety, SafetyError
from longrun_events import hot_setting, main as event_main, restart
from longrun_report import generate


class LongRunTests(unittest.TestCase):
    def test_equal_retains_only_numeric_aggregates_and_exact_latency(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            store = HourStore(root, 100)
            row = {'classification': 'semantic_equal', 'different_fields': [], 'latency_ms': .123456,
                   'query_wire': '000000000000000000000000', 'go_wire': 'abcd', 'rust_wire': 'abcd',
                   'go': {'tc': False}, 'rust': {'tc': False}, 'trace': {'private': 'secret'},
                   'source': '192.0.2.7', 'sent_monotonic': 1}
            store.record(row, 101)
            store.record({**row, 'classification': 'unexplained', 'different_fields': ['tc']}, 3701)
            store.close()
            first = root / 'hours/000'
            self.assertEqual(json.loads((first / 'stats.json').read_text())['metrics']['class:semantic_equal'], 1)
            self.assertFalse((first / 'differences.jsonl.gz').exists())
            with gzip.open(first / 'latencies.bin.gz', 'rb') as stream:
                self.assertEqual(struct.unpack('!dd', stream.read()), (101, .123456))
            with gzip.open(root / 'hours/001/differences.jsonl.gz', 'rt') as stream:
                self.assertEqual(json.loads(stream.readline())['trace'], {'private': 'secret'})
            self.assertNotIn('secret', (first / 'stats.json').read_text())

    def test_planned_restart_only_waives_identity_and_brief_inactive(self):
        s = Safety(0)
        good = {'MainPID': '10', 'NRestarts': '0', 'ActiveState': 'active'}
        s.health(0, good, True)
        s.health(1, {**good, 'MainPID': '0', 'ActiveState': 'activating'}, False, planned=True)
        s.health(2, {**good, 'MainPID': '20'}, True, planned=True)
        with self.assertRaises(SafetyError):
            s.health(3, {**good, 'MainPID': '21'}, True)
        with self.assertRaises(SafetyError):
            s.health(4, {**good, 'MainPID': '22', 'NRestarts': '1'}, True, planned=True)
        s = Safety(0)
        s.health(0, good, True)
        s.health(1, good, False, planned=True)
        with self.assertRaises(SafetyError):
            s.health(31, good, False, planned=True)

    def test_setting_failure_reverts_and_a_claim_is_never_applied_twice(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / 'events').mkdir()
            before = {'ecs': '198.51.100.1', 'socks5': '', 'replacements': []}
            calls = []
            def request(path, value=None):
                calls.append(value)
                if value and value['ecs'] == '192.0.2.77':
                    raise OSError('apply response lost')
                return {'http_status': 200, 'body': before}
            with mock.patch('longrun_events.request', side_effect=request):
                with self.assertRaises(OSError):
                    hot_setting(root, {})
            self.assertEqual(calls[-2], before)
            (root / 'events/1.claim').write_text('100')
            (root / 'events/1.json').write_text('{"success":true}')
            with mock.patch('longrun_events.hot_setting') as apply:
                event_main(root, 1)
                apply.assert_not_called()

    def test_fast_restart_intent_remains_until_a_five_second_sampler_can_see_it(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)
            before={'MainPID':'10','NRestarts':'0','ActiveState':'active'}
            after={**before,'MainPID':'20'}
            with mock.patch('longrun_events.service',side_effect=[before,after]), \
                 mock.patch('longrun_events.subprocess.run'), mock.patch('longrun_events.wait_ready'):
                restart(root,4,'rust-v0.1.2',{})
            intent=json.loads((root/'restart-intent.json').read_text())
            self.assertEqual(intent['after_pid'],'20')
            self.assertGreater(intent['until']-intent['issued'],5)

    def test_report_counts_retained_equal_outcomes_in_exact_event_window(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            segment = root / 'segments/001'
            segment.mkdir(parents=True)
            (root / 'events').mkdir()
            (root / 'run.json').write_text(json.dumps({'capture_start_wall': 100, 'end_wall': 1000}))
            (segment / 'segment.json').write_text('{"started_wall":100,"ended_wall":1000}')
            (root / 'events/1.json').write_text(json.dumps({'event':1,'started_wall':100,'ended_wall':110,
                'success':True,'effect_window':{'start_wall':110,'end_wall':710}}))
            store = HourStore(segment, 100)
            for wall in (109,110,709,710):
                store.record({'classification':'semantic_equal','different_fields':[], 'latency_ms':.123456},wall)
            store.close()
            result = generate(root)
            self.assertEqual(result['classifications'], {'semantic_equal':4})
            self.assertEqual(result['event_effects'][0]['post_10m']['classifications'], {'semantic_equal':2})
            self.assertEqual(result['latency_ms']['p99'], .123)
            self.assertTrue((root/'numeric-evidence.tar.gz').exists())


if __name__ == '__main__':
    unittest.main()
