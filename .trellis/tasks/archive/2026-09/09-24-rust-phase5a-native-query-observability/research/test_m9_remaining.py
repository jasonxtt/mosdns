import importlib.util
import unittest
import subprocess
import tempfile
import socket
import re
from unittest.mock import patch
from pathlib import Path


def load():
    spec=importlib.util.spec_from_file_location('remaining',Path(__file__).with_name('run-m9-remaining.py'))
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module


class RemainingTests(unittest.TestCase):
    def test_fixture_ids_match_the_helper_answer_contract(self):
        spec=importlib.util.spec_from_file_location('control',Path(__file__).with_name('m9-server-control.py'))
        m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
        helper=Path(__file__).resolve().parents[4]/'tests/phase5a-baseline/cmd/phase5a-baseline/main.go'
        source=helper.read_text().split('func fixtureAnswer(')[1].split('func runStage(')[0]
        recognized=set(re.findall(r'case "([^"]+)":',source))
        for scenario in ('w2','w3'):
            self.assertTrue(all(upstream in recognized for _,upstream,_ in m.fixture_specs(scenario)))

    def test_lost_launch_reply_still_stops_owned_session(self):
        m=load();calls=[]
        def remote(args,client,command,**kwargs):
            calls.append(command)
            if 'm9-server-control.py start' in command:raise subprocess.TimeoutExpired('ssh',15)
            if 'hash-tree' in command:raise ValueError('no synthetic evidence')
            return subprocess.CompletedProcess([],0,stdout='',stderr='')
        with tempfile.TemporaryDirectory() as directory,patch.object(m.t,'remote',side_effect=remote):
            self.assertEqual(m.run_one(None,m.plan()[0],Path(directory)/'slot')['runner_exit'],1)
        self.assertTrue(any('m9-server-control.py stop' in c for c in calls))

    def test_udp_probe_rejects_existing_bound_socket(self):
        spec=importlib.util.spec_from_file_location('control',Path(__file__).with_name('m9-server-control.py'))
        m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as occupied:
            occupied.bind(('127.0.0.1',0))
            with self.assertRaises(OSError):m.probe_available(occupied.getsockname())

    def test_frozen_plan_covers_cache_and_routes_once(self):
        p=load().plan();self.assertEqual(len(p),18)
        self.assertEqual([x['scenario'] for x in p],['w2']*9+['w3']*9)
        self.assertTrue(all(x['qps']==100 and x['duration_ms']==(25000 if x['scenario']=='w2' else 30000) for x in p))
        self.assertEqual([x['variant'] for x in p[:9]],['before_off','after_off','after_on','after_off','after_on','before_off','after_on','before_off','after_off'])

    def test_shortfall_or_latency_regression_rejects_complete_matrix(self):
        m=load();rows=[dict(x,runner_exit=0,correct=x['planned'],shortfall=0,errors=0,p95_us=100,p99_us=200) for x in m.plan()]
        self.assertTrue(m.assess(rows)['passed'])
        rows[0]['shortfall']=1;self.assertFalse(m.assess(rows)['passed']);rows[0]['shortfall']=0
        for r in rows:
            if r['scenario']=='w3' and r['variant']=='after_on':r['p99_us']=230
        self.assertFalse(m.assess(rows)['passed'])


if __name__=='__main__':unittest.main()
