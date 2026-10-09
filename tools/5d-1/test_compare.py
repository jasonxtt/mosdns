import copy
import unittest
from budget import evaluate


def evidence(cpu, rss):
    stable=dict(correct_rate=1,send_coverage=1,statuses={'correct':100},planned_latency_ms={'p50':1,'p95':2,'p99':3})
    stages=[dict(stage=name,rate=rate,stable=copy.deepcopy(stable),
                 resources=dict(cpu_us_per_correct=cpu,rss_median_kib=rss,fd_end=30,threads_end=2))
            for name,rate in (('q2000',2000),('recovery',100))]
    stages[-1]['seconds']=[copy.deepcopy(stable) for _ in range(10)]
    aggregates=[dict(stage=s['stage'],rate=s['rate'],planned_latency_ms={k:{'median':v} for k,v in stable['planned_latency_ms'].items()},
                     cpu_us_per_correct={'median':cpu},rss_median_kib={'median':rss}) for s in stages]
    return dict(aggregates=aggregates,runs=[dict(seed=seed,stages=copy.deepcopy(stages),startup_peak_rss_kib=rss,
                                             processes={'sut':{'peak_rss_kib':rss}}) for seed in (501,502,503)])


class BudgetTests(unittest.TestCase):
    def test_absolute_capacity_and_cpu_report_only_with_individual_trigger(self):
        go,native=evidence(100,200000),evidence(120,80000)
        native['runs'][2]['stages'][0]['resources']['cpu_us_per_correct']=160
        freeze=dict(latency_ms={'q100':{'p95':14,'p99':15}},capacity_latency_ms={'p95':14,'p99':15})
        result=evaluate(go,native,freeze)
        self.assertTrue(result['capacity_floor_pass'])
        self.assertEqual(result['tested_sustainable_capacity'],2000)
        self.assertTrue(result['stages'][0]['profiling_required'])
        self.assertTrue(result['stages'][0]['steady_rss_pass'])
        native['runs'][2]['stages'][0]['resources']['rss_median_kib']=129*1024
        self.assertFalse(evaluate(go,native,freeze)['stages'][0]['steady_rss_pass'])
