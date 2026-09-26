import importlib.util
import unittest
import subprocess
import tempfile
from unittest.mock import patch
from pathlib import Path


def load():
    spec = importlib.util.spec_from_file_location('simple_probe', Path(__file__).with_name('run-m8-simple.py'))
    module = importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module


class SimplifiedTests(unittest.TestCase):
    def test_transport_error_after_launch_still_stops_owned_session(self):
        m=load();calls=[]
        def remote(args,client,command,**kwargs):
            calls.append(command)
            if 'm8-server-control.py start' in command:
                raise subprocess.TimeoutExpired('ssh',15)
            if 'hash-tree' in command:
                raise ValueError('no synthetic evidence')
            return subprocess.CompletedProcess([],0,stdout='',stderr='')
        with tempfile.TemporaryDirectory() as directory,patch.object(m.t,'remote',side_effect=remote):
            result=m.run_one(None,m.plan()[0],Path(directory)/'slot')
            self.assertEqual(result['runner_exit'],1)
            self.assertTrue((Path(directory)/'slot/error.txt').exists())
        self.assertTrue(any('m8-server-control.py stop' in command for command in calls))

    def test_exact_nine_balanced_actual_variants(self):
        m=load();p=m.plan()
        self.assertEqual(len(p),9)
        self.assertEqual([r['variant'] for r in p],['before_off','after_off','after_on','after_off','after_on','before_off','after_on','before_off','after_off'])
        self.assertTrue(all(r['qps']==100 and r['duration_ms']==30000 for r in p))

    def test_shortfall_rejects_and_repeated_latency_regression_detected(self):
        m=load();rows=[]
        for slot in m.plan():
            rows.append(dict(slot,p95_us=100,p99_us=200,correct=3000,shortfall=0,errors=0,runner_exit=0))
        self.assertTrue(m.assess(rows)['passed'])
        rows[0]['shortfall']=1
        self.assertFalse(m.assess(rows)['passed'])
        rows[0]['shortfall']=0
        for row in rows:
            if row['variant']=='after_on':row['p99_us']=230
        self.assertFalse(m.assess(rows)['passed'])


if __name__=='__main__':unittest.main()
