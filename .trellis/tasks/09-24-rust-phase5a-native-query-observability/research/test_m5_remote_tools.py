import json
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

TOOLS = Path(__file__).with_name('m5-remote-tools.py')


class ServerSamplerTests(unittest.TestCase):
    def test_ready_after_first_sample_and_stop_records_final_local_sample(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            proc = root / 'proc'
            for pid, cpu in ((10, '0'), (11, '1')):
                directory = proc / str(pid)
                (directory / 'fd').mkdir(parents=True)
                (directory / 'fd' / '0').touch()
                fields = ['0'] * 25
                fields[0] = 'S'
                fields[11], fields[12], fields[19] = '5', '3', '999'
                (directory / 'stat').write_text(f'{pid} (name with (parens)) ' + ' '.join(fields))
                (directory / 'status').write_text(f'VmRSS:\t512 kB\nCpus_allowed_list:\t{cpu}\n')
            output = root / 'samples'
            command = ['python3', str(TOOLS), 'sample-server', '--sut-pid', '10', '--fixture-pid', '11',
                       '--run-id', 'run1', '--stage', 'normal-reference', '--result', str(output),
                       '--proc-root', str(proc), '--max-seconds', '5']
            child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                deadline = time.monotonic() + 3
                while not (output / 'ready').exists() and child.poll() is None and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertTrue((output / 'ready').exists(), child.communicate() if child.poll() is not None else 'sampler not ready')
                (output / 'stop').touch()
                stdout, stderr = child.communicate(timeout=3)
                self.assertEqual(child.returncode, 0, stdout + stderr)
                metadata = json.loads((output / 'metadata.json').read_text())
                self.assertTrue(metadata['valid'])
                self.assertEqual(metadata['counts'], {'sut': 2, 'fixture-1': 2})
                samples = [json.loads(l) for l in (output / 'resource-samples.jsonl').read_text().splitlines()]
                self.assertEqual({s['host'] for s in samples}, {metadata['host']})
                self.assertEqual({s['role'] for s in samples}, {'sut', 'fixture-1'})
                self.assertTrue(all(s['run_id'] == 'run1' and s['stage_id'] == 'normal-reference' for s in samples))
            finally:
                if child.poll() is None:
                    child.kill()
                    child.communicate()


class MergeStageTests(unittest.TestCase):
    def test_merge_preserves_client_latency_and_separates_pid_namespaces(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            client, server, result = root / 'client', root / 'server', root / 'merged'
            client.mkdir(); server.mkdir()
            stage = dict(stage='normal-reference', run_id='run1', fixture_session_id='run1',
                         scenario='w1', transport='tcp', target_qps=200, duration_ms=25000,
                         request_deadline_ms=500, late_drain_ms=100, sut_pid=0,
                         harness_pid=13, harness_cpu_set='0', harness_host='Debian', p95_us=123, p99_us=456,
                         resource_sample_counts={'load-generator': 2})
            (client / 'stages.client.jsonl').write_text(json.dumps(stage) + '\n')
            client_samples = [dict(run_id='run1', stage_id='normal-reference', role='load-generator', pid=13,
                                   clock_ticks_per_second=100, rss_kib=1024, fd_count=3)] * 2
            (client / 'resource-samples.jsonl').write_text('\n'.join(json.dumps(s) for s in client_samples))
            metadata = dict(valid=True, host='mosdns-rust', run_id='run1', stage='normal-reference', proc_root='/proc',
                            counts={'sut': 2, 'fixture-1': 2}, processes={'sut': dict(pid=13, cpus='0', start_identity='111'),
                            'fixture-1': dict(pid=14, cpus='1', start_identity='222')})
            (server / 'metadata.json').write_text(json.dumps(metadata))
            samples = [dict(run_id='run1', stage_id='normal-reference', role=role, host='mosdns-rust', pid=pid,
                            clock_ticks_per_second=100, rss_kib=1024, fd_count=3) for role, pid in (('sut', 13), ('fixture-1', 14))] * 2
            (server / 'resource-samples.jsonl').write_text('\n'.join(json.dumps(s) for s in samples))
            command = ['python3', str(TOOLS), 'merge-stage', '--client', str(client), '--server', str(server),
                       '--result', str(result), '--client-host', 'Debian']
            done = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(done.returncode, 0, done.stderr)
            merged = json.loads((result / 'stages.jsonl').read_text())
            self.assertEqual((merged['p95_us'], merged['p99_us']), (123, 456))
            self.assertEqual(merged['sut_pid'], 0)
            self.assertEqual(merged['server_resources']['processes']['sut']['pid'], 13)
            self.assertEqual(merged['resource_sample_counts'], {'load-generator': 2, 'sut': 2, 'fixture-1': 2})
            # Identical numeric PIDs on different hosts are valid; fake local server sampling is not.
            metadata['host'] = 'Debian'
            (server / 'metadata.json').write_text(json.dumps(metadata))
            rejected = subprocess.run(command, capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)


if __name__ == '__main__':
    unittest.main()
