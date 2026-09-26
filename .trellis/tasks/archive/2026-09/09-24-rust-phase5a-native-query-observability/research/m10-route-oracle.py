#!/usr/bin/env python3
"""Offline native W3 join for unique DNS IDs; never compare host clocks."""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import copy
import hashlib
import importlib.util
import json
import re
from pathlib import Path

PATHS={'DOMAIN_HIT':['route-a'],'IP_RULE_HIT':['route-b','route-a'],'IP_RULE_MISS':['route-b','route-c']}


def name(value):
    return value.lower().rstrip('.')+'.'


def timestamp(value):
    match=re.fullmatch(r'(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d{1,9}))?Z',value)
    if not match:raise ValueError('invalid UTC fixture/ledger timestamp')
    seconds=int(datetime.strptime(match[1],'%Y-%m-%dT%H:%M:%S').replace(tzinfo=timezone.utc).timestamp())
    return seconds*1_000_000_000+int((match[2] or '').ljust(9,'0'))


def verify(stage,requests,events,cases):
    if not stage['run_id'] or stage['scenario']!='w3' or stage['transport']!='udp':raise ValueError('wrong native W3 stage')
    count=len(requests);c=stage['counters']
    if not count or any(c[k]!=count for k in ('scheduled','sent','received','correct_on_time')):raise ValueError('stage/ledger count mismatch')
    if any(c[k] for k in ('correct_late','wrong_response','protocol_error','transport_error','timeout','sender_shortfall')):raise ValueError('stage has errors/shortfall')
    case_map={item['case_id']:item for item in cases}
    if len(case_map)!=len(cases) or not cases:raise ValueError('duplicate/empty cases')
    if any(item['qtype']!='A' or item['expected_route_class'] not in PATHS for item in cases):raise ValueError('unsupported frozen route case')
    owners={};sequences=set();scheduled=Counter()
    for request in requests:
        if request['run_id']!=stage['run_id'] or request['stage_id']!=stage['stage']:raise ValueError('mixed request identity/stages')
        if not request['sent'] or request['outcome']!='correct_on_time':raise ValueError('incorrect request')
        if timestamp(request['finished_at'])<timestamp(request['sent_at']):raise ValueError('invalid client-local interval')
        dns_id=request['dns_id'];sequence=request['request_seq']
        if not isinstance(dns_id,int) or not 0<=dns_id<=65535 or dns_id in owners or sequence in sequences:raise ValueError('ambiguous DNS ID/request sequence')
        case=case_map.get(request['case_id'])
        if not case or name(request['qname'])!=name(case['qname']) or request['qtype']!=case['qtype'] or request['qclass']!=1:raise ValueError('request/workload question mismatch')
        owners[dns_id]=request;sequences.add(sequence);scheduled[request['case_id']]+=1
    if sequences!=set(range(stage['request_seq_start'],stage['request_seq_end']+1)) or len(sequences)!=count:raise ValueError('request sequence gap')
    if dict(scheduled)!=stage['case_scheduled'] or set(scheduled)!=set(case_map):raise ValueError('case coverage/count mismatch')
    if [e['fixture_seq'] for e in events]!=list(range(1,len(events)+1)):raise ValueError('fresh-session fixture sequence gap/duplicate/order')
    expected_count=sum(len(PATHS[case_map[r['case_id']]['expected_route_class']]) for r in requests)
    if len(events)!=expected_count:raise ValueError('missing/extra fixture events')
    paths=defaultdict(list)
    for event in events:
        timestamp(event['occurred_at'])  # Validate its own format; no cross-host comparison.
        request=owners.get(event['dns_id'])
        if not request or name(event['qname'])!=name(request['qname']) or event['qtype']!=1 or event['qclass']!=request['qclass']:raise ValueError('unmatched fixture DNS ID/question')
        paths[event['dns_id']].append(event['upstream'])
    for dns_id,request in owners.items():
        if paths[dns_id]!=PATHS[case_map[request['case_id']]['expected_route_class']]:raise ValueError('wrong ordered route legs')
    return dict(passed=True,run_id=stage['run_id'],requests_verified=count,events_verified=len(events),unique_dns_ids=count,case_counts=dict(scheduled),join='unique session DNS ID + exact question; ordered fixture sequence; no shared clock')


def rows(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def derive(raw_root,result_root,workload,cleanup_proof,postbatch_identity):
    raw=Path(raw_root);out=Path(result_root)
    spec=importlib.util.spec_from_file_location('m10',Path(__file__).with_name('run-m10-w3.py'))
    driver=importlib.util.module_from_spec(spec);spec.loader.exec_module(driver)
    original=json.loads((raw/'rows.json').read_text())
    if [r['run_id'] for r in original]!=[r['run_id'] for r in driver.plan()]:raise ValueError('incomplete/wrong M10 matrix')
    identity=json.loads((raw/'identity.json').read_text())
    head=(raw/'source-head.txt').read_text().strip()
    # Analyze the committed executed version, allowing a later offline cleanup
    # repair without pretending that repair was used for these measurements.
    # The task directory moves when the task is archived, so the measuring
    # commit's own research path is resolved from that commit; the files loaded
    # from here stay the repaired offline analysis, never the measured tools.
    measured=driver.committed_research_root(head)
    committed=json.loads(driver.committed_bytes(head,measured/'m10-preflight/identity.json'))
    if committed!=identity or json.loads(Path(postbatch_identity).read_text())!=identity or (raw/'identity-error.txt').exists():raise ValueError('reviewed/postbatch execution identity differs')
    for file,digest in identity['local_tools'].items():
        body=driver.committed_bytes(head,measured/file)
        if hashlib.sha256(body).hexdigest()!=digest:raise ValueError('executed tool differs from reviewed preflight')
    cleanup=json.loads(Path(cleanup_proof).read_text());receipts={(r['run_id'],r['role']):r for r in cleanup}
    expected_owned={}
    for row in original:
        for role,record in json.loads((raw/row['run_id']/'server/owned.json').read_text()).items():expected_owned[(row['run_id'],role)]=record
    if len(receipts)!=len(cleanup) or set(receipts)!=set(expected_owned):raise ValueError('incomplete/duplicate cleanup receipt')
    for key,record in expected_owned.items():
        receipt=receipts[key]
        if receipt['owned_process_active'] or receipt['pid']!=record['pid'] or receipt['start_identity']!=record['start_identity']:raise ValueError('owned process remains active or receipt identity differs')
    corpus=Path(workload)
    expected=identity['server_inputs'][driver.t.SERVER_INPUT+'/repo/tests/phase5a-baseline/workloads/routing.jsonl']
    if hashlib.sha256(corpus.read_bytes()).hexdigest()!=expected:raise ValueError('workload differs from reviewed corpus')
    derived=[];proofs={}
    for row in original:
        session=raw/row['run_id'];stage=rows(session/'stages.jsonl')
        if len(stage)!=1:raise ValueError('multiple primary stages')
        stage=stage[0]
        for host,hostname in (('client','Debian'),('server','mosdns-rust')):driver.t.verify_transfer(session/host,hostname)
        if any((session/name).exists() for name in ('error.txt','evidence-error.txt')):raise ValueError('session has other errors')
        cleanup_error=(session/'cleanup-error.txt')
        if cleanup_error.exists() and 'stderr:\n[Errno 3] No such process' not in cleanup_error.read_text():raise ValueError('cleanup error is not the proven exited-process race')
        exits=[int(line.split('=')[1]) for line in (session/'oracles.txt').read_text().splitlines() if line.startswith('exit=')]
        if row['runner_exit']!=1 or exits!=[0,0,1,0] or 'fixture event loss in stage window: seq_start=0 seq_end=0 events=0' not in (session/'oracles.txt').read_text():raise ValueError('not solely the known legacy routing oracle incompatibility')
        if stage['fixture_seq_start']!=0 or stage['fixture_seq_end']!=0:raise ValueError('not a client without server barriers')
        sut=json.loads((session/'server/sut.json').read_text())
        expected_sha=driver.t.BASELINE_SHA if row['variant']=='before_off' else driver.CANDIDATE_SHA
        config=driver.t.SERVER_INPUT+'/w3-'+('on' if row['variant']=='after_on' else 'off')+'.yaml'
        if sut['variant']!=row['variant'] or sut['audit_enabled']!=(row['variant']=='after_on') or sut['sha256']!=expected_sha or sut['config_sha256']!=identity['server_inputs'][config]:raise ValueError('actual variant identity differs')
        proof=verify(stage,rows(session/'client/requests.jsonl'),rows(session/'server/routing-events.jsonl'),rows(corpus))
        if proof['run_id']!=row['run_id'] or proof['requests_verified']!=row['planned']:raise ValueError('oracle session/count differs')
        for field in ('p50_us','p95_us','p99_us'):
            if row[field]!=stage[field]:raise ValueError('latency differs from raw stage')
        if row['correct']!=stage['counters']['correct_on_time'] or row['shortfall'] or row['errors']:raise ValueError('row correctness differs')
        adjusted=copy.deepcopy(row);adjusted['original_runner_exit']=row['runner_exit'];adjusted['runner_exit']=0
        adjusted['offline_route_verified']=True;derived.append(adjusted);proofs[row['run_id']]=proof
        adjusted['cleanup_exit_receipt_used']=cleanup_error.exists()
    assessment=driver.assess(derived)
    assessment['scope']='W3 same numeric gate with unique-ID offline route proof; original driver FAIL preserved'
    out.mkdir(parents=True,exist_ok=False)
    (out/'rows.json').write_text(json.dumps(derived,indent=2)+'\n')
    (out/'assessment.json').write_text(json.dumps(assessment,indent=2)+'\n')
    (out/'route-proofs.json').write_text(json.dumps(proofs,indent=2)+'\n')
    (out/'provenance.json').write_text(json.dumps(dict(original_driver_assessment_sha256=hashlib.sha256((raw/'assessment.json').read_bytes()).hexdigest(),original_rows_sha256=hashlib.sha256((raw/'rows.json').read_bytes()).hexdigest(),original_verdict_unchanged=True,original_helper_routing_failures=9,other_original_helper_oracles_passed=27,cleanup_exit_receipts=sum(r['cleanup_exit_receipt_used'] for r in derived),cleanup_proof_sha256=hashlib.sha256(Path(cleanup_proof).read_bytes()).hexdigest(),postbatch_identity_sha256=hashlib.sha256(Path(postbatch_identity).read_bytes()).hexdigest(),new_queries=0,oracle_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()),indent=2)+'\n')
    return assessment


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--workload',required=True)
    for argument in ('stage','requests','events','raw-root','result-root','cleanup-proof','postbatch-identity'):parser.add_argument('--'+argument)
    args=parser.parse_args()
    if args.raw_root:
        if not all((args.result_root,args.cleanup_proof,args.postbatch_identity)) or any((args.stage,args.requests,args.events)):raise ValueError('raw-root mode requires distinct result-root/cleanup-proof/postbatch-identity')
        result=derive(args.raw_root,args.result_root,args.workload,args.cleanup_proof,args.postbatch_identity);print(json.dumps(dict(passed=result['passed'],comparisons=result['comparisons']),indent=2));return
    if not all((args.stage,args.requests,args.events)) or args.result_root:raise ValueError('single-stage mode requires stage/requests/events')
    stage_rows=rows(args.stage)
    if len(stage_rows)!=1:raise ValueError('one fresh-session primary required')
    print(json.dumps(verify(stage_rows[0],rows(args.requests),rows(args.events),rows(args.workload)),indent=2))


if __name__=='__main__':main()
