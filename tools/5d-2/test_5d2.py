import json
import os
import pathlib
import socket
import sys
import tempfile
import threading
import time
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import analyze  # noqa: E402
import fload  # noqa: E402
from dnswire import answer, decode, question  # noqa: E402
from load import semantic  # noqa: E402
from mock import MARKERS  # noqa: E402


def row(status, planned, finished, group='g'):
    return dict(status=status, planned=planned, finished=finished, sent=planned, group=group)


class Classification(unittest.TestCase):
    def setUp(self):
        self.good = decode(answer(question('a.example.com', 1, 5), *MARKERS['domestic']))
        self.expected = semantic(self.good)

    def test_correct_servfail_fallback_and_wrong_are_distinct(self):
        self.assertEqual(fload.classify(self.good, self.expected), 'correct')
        servfail = dict(self.good, rcode=2, answers=[], authority=[], additional=[])
        self.assertEqual(fload.classify(servfail, self.expected), 'servfail')
        other = decode(answer(question('a.example.com', 1, 5), *MARKERS['foreign']))
        self.assertEqual(fload.classify(other, self.expected), 'wrong')
        self.assertEqual(fload.classify(other, self.expected, (semantic(other),)), 'fallback')
        refused = dict(self.good, rcode=5, answers=[])
        self.assertEqual(fload.classify(refused, self.expected), 'wrong', 'only SERVFAIL is an accepted failure')
        stale = dict(self.good, answers=[(a[0], a[1], a[2], a[3], 301) for a in self.good['answers']])
        self.assertEqual(fload.classify(stale, self.expected), 'wrong', 'TTL above the mock maximum is wrong')


class Windows(unittest.TestCase):
    def test_window_counts_denominator_and_latency(self):
        rows = [row('correct', 0.0, 0.01), row('correct', 0.5, 0.9), row('correct', 0.6, 1.3),
                row('servfail', 1.0, 1.2), row('timeout', 1.5, None)]
        value = analyze.window(rows, 0, 2)
        self.assertEqual(value['planned'], 5)
        self.assertEqual(value['statuses'], {'correct': 3, 'servfail': 1, 'timeout': 1})
        # 0.4 s is healthy, 0.7 s is a correct but late answer, a SERVFAIL or timeout is never healthy.
        self.assertEqual(value['healthy'], 2)

    def test_recovery_needs_five_consecutive_healthy_seconds_after_the_clear(self):
        rows = []
        for tenth in range(0, 400):
            planned = tenth / 10
            # unhealthy until 12.0, one bad row at 14.2, healthy afterwards
            bad = planned < 12 or abs(planned - 14.2) < 0.05
            rows.append(row('servfail' if bad else 'correct', planned, planned + 0.01))
        value = analyze.recovery_seconds(rows, 10, {'g'})
        # healthy bins: 12,13 | 14 broken | 15..19 -> five bins end at 20; recovery = 20 - 10
        self.assertEqual(value, 10)
        self.assertIsNone(analyze.recovery_seconds(rows[:150], 10, {'g'}))

    def test_resources_report_first_last_and_peak(self):
        samples = [dict(time=100 + i, processes=dict(sut=dict(rss_kib=1000 + i * 10, fd=20 + (i == 2), threads=3)),
                        connections={'01:15012': i}) for i in range(5)]
        value = analyze.resources(samples, 100, 0, 5)
        self.assertEqual((value['rss_kib']['first'], value['rss_kib']['last'], value['rss_kib']['peak']), (1000, 1040, 1040))
        self.assertEqual((value['fd']['first'], value['fd']['peak'], value['fd']['last']), (20, 21, 20))
        self.assertEqual(value['established']['peak'], 4)


class Live(unittest.TestCase):
    """The real sender against a stub resolver: schedule, events and the failure classes."""

    @unittest.skipUnless(sys.platform == 'linux', 'Linux /proc sampling')
    def test_sender_classifies_and_runs_events(self):
        server = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        server.bind(('127.0.0.1', 0))
        port = server.getsockname()[1]
        stop = threading.Event()

        def serve():
            server.settimeout(0.05)
            while not stop.is_set():
                try:
                    wire, peer = server.recvfrom(1500)
                except socket.timeout:
                    continue
                name = decode(wire)['questions'][0][0]
                if name.startswith('silent'):
                    continue
                reply = answer(wire, *MARKERS['domestic'])
                if name.startswith('servfail'):
                    reply = reply[:3] + bytes([(reply[3] & 0xF0) | 2]) + b'\0\1\0\0\0\0\0\0' + reply[12:12 + len(wire) - 12]
                if name.startswith('other'):
                    reply = answer(wire, *MARKERS['foreign'])
                server.sendto(reply, peer)

        threading.Thread(target=serve, daemon=True).start()
        names = ['ok.example.com', 'servfail.example.com', 'other.example.com', 'silent.example.com'] * 10
        rows = [dict(name=n, type=1, group='d' if i % 2 else 'e') for i, n in enumerate(names)]
        oracle = [dict(index=i, semantic=semantic(decode(answer(question(n, 1, 0), *MARKERS['domestic'])))) for i, n in enumerate(names)]
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / 'corpus.json').write_text(json.dumps(rows))
            (root / 'oracle.json').write_text(json.dumps(oracle))
            spec = dict(corpus=str(root / 'corpus.json'), oracle=str(root / 'oracle.json'), output=str(root / 'out'),
                        root=str(root), rate=100, duration=0.4, deadline_s=0.6, pid=os.getpid(), mock_pid=os.getpid(),
                        api_port=1, dns_port=port,
                        events=[dict(at=0.05, action='faults', table={'groups': {}}),
                                dict(at=0.1, action='probe', port=port, name='ok.example.com'),
                                dict(at=0.12, action='note')])
            fload.measure(spec)
            out = root / 'out'
            summary = json.loads((out / 'summary.json').read_text())
            result = analyze.load_rows(out / 'requests.jsonl', summary['start'])
            counts = {}
            for entry in result:
                counts[entry['status']] = counts.get(entry['status'], 0) + 1
            self.assertEqual(sum(counts.values()), 40)
            self.assertEqual(counts, {'correct': 10, 'servfail': 10, 'wrong': 10, 'timeout': 10})
            self.assertEqual((root / 'faults.json').read_text(), json.dumps({'groups': {}}))
            events = json.loads((out / 'events.json').read_text())
            probe = next(e for e in events if e['action'] == 'probe')
            self.assertTrue(probe['result']['answered'])
            self.assertEqual(len(events), 3)


class WarmupProfile(unittest.TestCase):
    def test_the_budget_figure_comes_from_the_warmup_stage_only(self):
        import warmup_profile
        stages = [dict(stage='q20', resources=dict(cpu_s=9, cpu_us_per_correct=9.0), stable=dict(correct_on_time=1), all=dict(statuses={})),
                  dict(stage='warmup', resources=dict(cpu_s=0.2, cpu_us_per_correct=240.0), stable=dict(correct_on_time=833),
                       all=dict(statuses={'correct': 1000}))]
        self.assertEqual(warmup_profile.warmup_cpu(dict(stages=stages)),
                         dict(cpu_s=0.2, cpu_us_per_correct=240.0, correct_on_time=833, statuses={'correct': 1000}))


class Injected(unittest.TestCase):
    """The proof that a fault reached the mock's traffic, not merely the fault file."""

    def table(self, **parts):
        import mock
        return mock.validate_faults(parts)

    def test_counted_modes_need_their_own_counter(self):
        table = self.table(groups={'domestic': dict(mode='blackhole'), 'foreign': dict(mode='servfail', protocols=['https'])})
        window = {'domestic:udp:fault:blackhole': 5, 'foreign:https:fault:servfail': 2}
        self.assertEqual(analyze.injected(table, window, {}), {'domestic:blackhole': True, 'foreign:servfail': True})
        self.assertEqual(analyze.injected(table, {'domestic:udp:fault:blackhole': 5}, {})['foreign:servfail'], False)
        self.assertEqual(analyze.injected(table, {'domestic:udp:fault:servfail': 5}, {})['domestic:blackhole'], False,
                         'another mode does not count')

    def test_refuse_and_down_need_silence_where_the_baseline_had_traffic(self):
        table = self.table(groups={'foreign': dict(mode='refuse', protocols=['https'])})
        baseline = {'foreign:https:accept': 10, 'foreignecs:https:accept': 10}
        self.assertTrue(analyze.injected(table, {'foreignecs:https:accept': 4}, baseline)['foreign:refuse'])
        self.assertFalse(analyze.injected(table, {'foreign:https:accept': 1}, baseline)['foreign:refuse'])
        self.assertIsNone(analyze.injected(table, {}, {})['foreign:refuse'], 'no baseline traffic proves nothing')
        down = self.table(socks=dict(mode='down'))
        self.assertTrue(analyze.injected(down, {}, {'socks:accept': 3})['socks:down'])
        self.assertFalse(analyze.injected(down, {'socks:accept': 1}, {'socks:accept': 3})['socks:down'])
        self.assertIsNone(analyze.injected(down, {}, {})['socks:down'])
        blackhole = self.table(socks=dict(mode='blackhole'))
        self.assertTrue(analyze.injected(blackhole, {'socks:fault:blackhole': 2}, {})['socks:blackhole'])
        self.assertFalse(analyze.injected(blackhole, {}, {})['socks:blackhole'])

    def test_handshake_reset_counter(self):
        table = self.table(groups={'foreign': dict(mode='handshake_reset', protocols=['https'])})
        self.assertTrue(analyze.injected(table, {'handshake_reset': 3}, {})['foreign:handshake_reset'])
        self.assertFalse(analyze.injected(table, {}, {})['foreign:handshake_reset'])


class Corpus(unittest.TestCase):
    def test_streams_are_unique_interleaved_and_revisit_asks_earlier_names(self):
        import scenarios
        rows, oracle = scenarios.stream_corpus(scenarios.FB_STREAMS, 100, 80,
                                               dict(groups=['domestic'], source=(15, 55), window=(65, 80)))
        self.assertEqual((len(rows), len(oracle)), (8000, 8000))
        self.assertEqual({g: sum(r['group'] == g for r in rows) for g in scenarios.FB_STREAMS}, {g: 1600 for g in scenarios.FB_STREAMS})
        for group in ('foreign', 'fb'):
            names = [r['name'] for r in rows if r['group'] == group]
            self.assertEqual(len(names), len(set(names)), 'a stream without revisit never repeats a name')
        domestic = [(i, r) for i, r in enumerate(rows) if r['group'] == 'domestic']
        early = {r['name']: r['type'] for i, r in domestic if 15 <= i / 100 < 55}
        revisited = [r for i, r in domestic if 65 <= i / 100 < 80]
        self.assertTrue(revisited and all(r['name'] in early and early[r['name']] == r['type'] for r in revisited))
        fb = next(i for i, r in enumerate(rows) if r['group'] == 'fb')
        self.assertEqual(len(oracle[fb]['alt']), 1)
        self.assertNotEqual(oracle[fb]['alt'][0], oracle[fb]['semantic'])
        self.assertTrue(rows[0]['name'].endswith('.5d2-domestic.example.com'))

    def test_f3_revisits_cover_every_ten_second_fault_segment(self):
        import scenarios
        spec = scenarios.build_scenarios()['F3']
        rows, _ = scenarios.stream_corpus(spec['streams'], spec['rate'], spec['duration'], spec['revisit'])
        first = {}
        for i, r in enumerate(rows):
            first.setdefault((r['name'], r['type']), i / spec['rate'])
        for group in ('domestic', 'foreign'):
            revisited = {(r['name'], r['type']) for i, r in enumerate(rows)
                         if r['group'] == group and 65 <= i / spec['rate'] < 80}
            for lo in (15, 25, 35, 45):
                self.assertGreaterEqual(sum(lo <= first[n] < lo + 10 for n in revisited), 50, (group, lo))

    def test_every_scenario_is_well_formed(self):
        import scenarios
        for name, spec in scenarios.build_scenarios().items():
            self.assertEqual(spec['name'], name)
            self.assertIn(spec['kind'], ('F', 'H'))
            for event in spec.get('events', []):
                self.assertLess(event['at'], spec['duration'], name)
                if event['action'] == 'faults':
                    import mock
                    mock.validate_faults(event['table'])
            if spec.get('fault_at') is not None:
                self.assertLess(spec['clear_at'] + 12, spec['duration'], name + ' needs an observation tail')


if __name__ == '__main__':
    unittest.main()
