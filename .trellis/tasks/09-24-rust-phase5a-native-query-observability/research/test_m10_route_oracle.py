import copy
import importlib.util
import unittest
from pathlib import Path


def load():
    spec=importlib.util.spec_from_file_location('route',Path(__file__).with_name('m10-route-oracle.py'))
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m


def fixture():
    cases=[dict(case_id='a',qname='a.test',qtype='A',expected_route_class='DOMAIN_HIT'),dict(case_id='ba',qname='b.test',qtype='A',expected_route_class='IP_RULE_HIT'),dict(case_id='bc',qname='c.test',qtype='A',expected_route_class='IP_RULE_MISS')]
    requests=[dict(run_id='run',stage_id='normal-reference',request_seq=i,dns_id=10+i,case_id=c['case_id'],qname=c['qname']+'.',qtype='A',qclass=1,sent=True,outcome='correct_on_time',sent_at='2026-09-26T13:00:00Z',finished_at='2026-09-26T13:00:01Z') for i,c in enumerate(cases,1)]
    events=[]
    for r,path in zip(requests,[['route-a'],['route-b','route-a'],['route-b','route-c']]):
        for upstream in path:events.append(dict(fixture_seq=len(events)+1,dns_id=r['dns_id'],qname=r['qname'],qtype=1,qclass=1,upstream=upstream,occurred_at='2026-09-26T13:14:00Z'))
    stage=dict(run_id='run',stage='normal-reference',scenario='w3',transport='udp',request_seq_start=1,request_seq_end=3,case_scheduled={'a':1,'ba':1,'bc':1},counters=dict(scheduled=3,sent=3,received=3,correct_on_time=3,correct_late=0,wrong_response=0,protocol_error=0,transport_error=0,timeout=0,sender_shortfall=0))
    return stage,requests,events,cases


class ClockIndependentRouteTests(unittest.TestCase):
    def test_unique_id_and_question_join_works_without_shared_clock(self):
        data=fixture();result=load().verify(*data)
        self.assertEqual(result['requests_verified'],3);self.assertEqual(result['events_verified'],5)
        self.assertEqual(data,fixture())

    def test_go_nanosecond_timestamps_keep_client_interval_precision(self):
        stage,requests,events,cases=fixture()
        requests[0]['sent_at']='2026-09-26T13:00:00.123456780Z'
        requests[0]['finished_at']='2026-09-26T13:00:00.123456789Z'
        self.assertTrue(load().verify(stage,requests,events,cases)['passed'])
        requests[0]['finished_at']='2026-09-26T13:00:00.123456779Z'
        with self.assertRaises(ValueError):load().verify(stage,requests,events,cases)

    def test_wrong_path_gap_duplicate_id_extra_missing_or_wrong_question_fail(self):
        m=load()
        for change in ('wrong_path','gap','duplicate_id','extra','missing','wrong_question','wrong_run','wrong_count'):
            stage,requests,events,cases=copy.deepcopy(fixture())
            if change=='wrong_path':events[2]['upstream']='route-c'
            if change=='gap':events[2]['fixture_seq']=10
            if change=='duplicate_id':requests[1]['dns_id']=requests[0]['dns_id']
            if change=='extra':events.append(dict(events[-1],fixture_seq=6))
            if change=='missing':events.pop()
            if change=='wrong_question':events[0]['qname']='unknown.test.'
            if change=='wrong_run':requests[0]['run_id']='other'
            if change=='wrong_count':stage['counters']['correct_on_time']=2
            with self.subTest(change=change),self.assertRaises(ValueError):m.verify(stage,requests,events,cases)


if __name__=='__main__':unittest.main()
