import importlib.util
import re
import socket
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

HERE=Path(__file__).resolve().parent


def load(file='run-m10-w3.py'):
    spec=importlib.util.spec_from_file_location('m10',HERE/file)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module


class W3RemediationTests(unittest.TestCase):
    def test_nine_balanced_w3_only_slots_with_new_ids(self):
        p=load().plan();self.assertEqual(len(p),9)
        self.assertEqual([s['variant'] for s in p],['before_off','after_off','after_on','after_off','after_on','before_off','after_on','before_off','after_off'])
        self.assertTrue(all(s['scenario']=='w3' and s['qps']==100 and s['duration_ms']==30000 and s['planned']==3000 and s['run_id'].startswith('m10-w3-') for s in p))

    def test_complete_evidence_and_latency_gate_are_required(self):
        m=load();rows=[dict(s,runner_exit=0,correct=3000,shortfall=0,errors=0,p95_us=100,p99_us=200) for s in m.plan()]
        self.assertTrue(m.assess(rows)['passed'])
        self.assertFalse(m.assess(rows[:-1])['passed'])
        rows[0]['shortfall']=1;self.assertFalse(m.assess(rows)['passed']);rows[0]['shortfall']=0
        for r in rows:
            if r['variant']=='after_on':r['p99_us']=230
        self.assertFalse(m.assess(rows)['passed'])

    def test_fixture_ids_match_real_helper_dispatch_and_ports(self):
        m=load('m10-server-control.py')
        source=(HERE.parents[3]/'tests/phase5a-baseline/cmd/phase5a-baseline/main.go').read_text().split('func fixtureAnswer(')[1].split('func runStage(')[0]
        recognized=set(re.findall(r'case "([^"]+)":',source))
        specs=m.fixture_specs('w3')
        self.assertEqual([s[2] for s in specs],[15456,15457,15458])
        self.assertTrue(all(s[1] in recognized for s in specs))
        self.assertEqual([s[1] for s in specs],['route-a','route-b','route-c'])
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as occupied:
            occupied.bind(('127.0.0.1',0))
            with self.assertRaises(OSError):m.probe_available(occupied.getsockname())

    def test_ambiguous_launch_reply_still_cleans_owned_session(self):
        m=load();calls=[]
        def remote(args,client,command,**kwargs):
            calls.append(command)
            if 'm10-server-control.py start' in command:raise subprocess.TimeoutExpired('ssh',15)
            if 'hash-tree' in command:raise ValueError('no synthetic evidence')
            return subprocess.CompletedProcess([],0,stdout='',stderr='')
        with tempfile.TemporaryDirectory() as d,patch.object(m.t,'remote',side_effect=remote):
            self.assertEqual(m.run_one(None,m.plan()[0],Path(d)/'slot')['runner_exit'],1)
        self.assertTrue(any('m10-server-control.py stop' in c for c in calls))


if __name__=='__main__':unittest.main()
