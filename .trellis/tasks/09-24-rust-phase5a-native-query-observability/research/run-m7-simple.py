#!/usr/bin/env python3
"""Nine-run100QPS W1 regression; reuse existing transport and DNS oracles."""
import argparse
import hashlib
import importlib.util
import json
import math
import statistics
import subprocess
import sys
from pathlib import Path

HERE=Path(__file__).resolve().parent


def load(file):
    spec=importlib.util.spec_from_file_location(file,HERE/file)
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m


t=load('run-m6-w1.py')
t.SERVER_INPUT=t.SERVER_BASE+'/measurement-v7'
t.SERVER_RESULTS=t.SERVER_BASE+'/results-m7-server'
t.CLIENT_BASE='/root/mosdns-phase5a-m7-client'
t.CLIENT_INPUT=t.CLIENT_BASE+'/tools';t.CLIENT_RESULTS=t.CLIENT_BASE+'/results'
CANDIDATE=t.SERVER_BASE+'/candidate-v12/rust/target/release/mosdns'
CANDIDATE_SHA='8d3e9dc7f5f1ae46365dda4d46e72b2e24dc2ca2b94b8e8c91f70e4e8e9edd0d'
TOOLS=('run-m7-simple.py','m7-server-control.py','run-m6-w1.py','m5-remote-tools.py','measurement-revision-v7.md')


def plan():
    orders=(('before_off','after_off','after_on'),('after_off','after_on','before_off'),('after_on','before_off','after_off'))
    return [dict(repetition=i,variant=v,run_id=f'm7-r{i}-{v}',qps=100,duration_ms=30000) for i,order in enumerate(orders,1) for v in order]


def assess(rows):
    failures=[];comparisons=[]
    if len(rows)!=9 or [(r['repetition'],r['variant']) for r in rows]!=[(r['repetition'],r['variant']) for r in plan()]:
        failures.append('incomplete or wrong fixed plan')
    for r in rows:
        if r['runner_exit'] or r['correct']!=3000 or r['shortfall'] or r['errors']:
            failures.append(r['run_id']+': invalid correctness/offered load')
        if any(not math.isfinite(r[k]) or r[k]<=0 for k in ('p95_us','p99_us')):
            failures.append(r['run_id']+': invalid latency')
    indexed={(r['repetition'],r['variant']):r for r in rows}
    for a,b in (('before_off','after_off'),('after_off','after_on')):
        for metric in ('p95_us','p99_us'):
            pairs=[indexed[(i,b)][metric]/indexed[(i,a)][metric] for i in range(1,4) if (i,a) in indexed and (i,b) in indexed and indexed[(i,a)][metric]>0]
            median=statistics.median(pairs) if len(pairs)==3 else None
            comparisons.append(dict(comparison=b+'_vs_'+a,metric=metric,paired_ratios=pairs,median_ratio=median))
            if median is None or median>1.10:failures.append(b+'_vs_'+a+' '+metric+': median regression/incomplete')
    return dict(passed=not failures,scope='100QPS W1 TCP only; no capacity/fullA5 claim',failures=failures,comparisons=comparisons,rows=rows)


def identity(args):
    hashes={f:hashlib.sha256((HERE/f).read_bytes()).hexdigest() for f in TOOLS}
    files={t.SERVER_INPUT+'/phase5a-baseline-helper-v10':t.HELPER_SHA,
           '/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/mosdns-rust':t.BASELINE_SHA,
           CANDIDATE:CANDIDATE_SHA}
    files.update({t.SERVER_INPUT+'/'+f:hashes[f] for f in ('m7-server-control.py','m5-remote-tools.py')})
    original=(t.REPO/'tests/phase5a-baseline/configs/forward-tcp.yaml').read_bytes()
    off=original.replace(b'127.0.0.1:15354',b'10.0.0.92:15354')
    files[t.SERVER_INPUT+'/forward-off.yaml']=hashlib.sha256(off).hexdigest()
    files[t.SERVER_INPUT+'/forward-on.yaml']=hashlib.sha256(off.replace(b'enable_audit: false',b'enable_audit: true')).hexdigest()
    files[t.SERVER_INPUT+'/repo/tests/phase5a-baseline/workloads/forward.jsonl']=t.WORKLOAD_SHA
    client={t.CLIENT_INPUT+'/phase5a-baseline-helper-v10':t.HELPER_SHA,t.CLIENT_INPUT+'/forward.jsonl':t.WORKLOAD_SHA,t.CLIENT_INPUT+'/m5-remote-tools.py':hashes['m5-remote-tools.py']}
    for is_client,expected in ((False,files),(True,client)):
        lines=t.remote(args,is_client,t.quoted(['sha256sum',*expected])).stdout.splitlines()
        if {l.split(maxsplit=1)[1]:l.split()[0] for l in lines}!=expected:raise ValueError('fixed input identity mismatch')
    return dict(local_tools=hashes,server_inputs=files,client_inputs=client)


def run_one(args,slot,root):
    root.mkdir();rid=slot['run_id'];server=t.SERVER_RESULTS+'/'+rid;client=t.CLIENT_RESULTS+'/'+rid
    helper=t.SERVER_INPUT+'/phase5a-baseline-helper-v10';code=0;started=False
    try:
        t.remote(args,True,t.quoted(['mkdir',client]))
        t.remote(args,True,t.quoted(['touch',client+'/session']))
        t.remote(args,False,t.quoted(['python3',t.SERVER_INPUT+'/m7-server-control.py','start','--variant',slot['variant'],'--result',server]));started=True
        owned=json.loads(t.remote(args,False,t.quoted(['cat',server+'/owned.json'])).stdout)
        sample=server+'/samples'
        cmd=['nohup','taskset','-c','1','python3',t.SERVER_INPUT+'/m5-remote-tools.py','sample-server','--sut-pid',owned['sut']['pid'],'--fixture-pid',owned['fixture']['pid'],'--sut-start',owned['sut']['start_identity'],'--fixture-start',owned['fixture']['start_identity'],'--run-id',rid,'--stage','normal-reference','--result',sample,'--max-seconds','40']
        t.remote(args,False,t.quoted(cmd)+' </dev/null >'+server+'/sampler.stdout 2>'+server+'/sampler.stderr &');t.wait_file(args,sample+'/ready')
        cmd=['env','-u','GOMEMLIMIT','GOMAXPROCS=1','GOGC=off','GODEBUG=gctrace=1','taskset','-c','0',t.CLIENT_INPUT+'/phase5a-baseline-helper-v10','run','--sample-self','--workload',t.CLIENT_INPUT+'/forward.jsonl','--scenario','w1','--transport','tcp','--addr','10.0.0.92:15354','--stage','normal-reference','--qps','100','--duration','30000ms','--deadline','500ms','--late-drain','100ms','--run-id',rid,'--fixture-session-id',rid,'--result',client+'/stage','--request-ledger',client+'/requests.jsonl','--fail-on-error']
        try:code=t.remote(args,True,t.quoted(cmd)+' >'+client+'/run.stdout 2>'+client+'/run.stderr',timeout=38,check=False).returncode
        finally:t.remote(args,False,t.quoted(['touch',sample+'/stop']))
        t.wait_file(args,sample+'/metadata.json')
        t.remote(args,True,t.quoted(['mv',client+'/stage/stages.jsonl',client+'/stage/stages.client.jsonl']))
    except (OSError,ValueError,subprocess.SubprocessError) as e:
        code=1;(root/'error.txt').write_text(t.error_evidence(e))
    finally:
        if started:
            try:t.remote(args,False,t.quoted(['python3',t.SERVER_INPUT+'/m7-server-control.py','stop','--result',server]))
            except (OSError,ValueError,subprocess.SubprocessError) as e:code=1;(root/'cleanup-error.txt').write_text(t.error_evidence(e))
    result=dict(slot,runner_exit=1,correct=0,shortfall=0,errors=0,p95_us=0,p99_us=0)
    try:
        for c,path,tools in ((True,client,t.CLIENT_INPUT),(False,server,t.SERVER_INPUT)):
            t.remote(args,c,t.quoted(['python3',tools+'/m5-remote-tools.py','hash-tree','--result',path]))
        t.transfer(args,True,client,str(root/'client'),recursive=True);t.transfer(args,False,server,str(root/'server'),recursive=True)
        t.verify_transfer(root/'client','Debian');t.verify_transfer(root/'server','mosdns-rust')
        subprocess.run([sys.executable,str(HERE/'m5-remote-tools.py'),'merge-stage','--client',str(root/'client/stage'),'--server',str(root/'server/samples'),'--result',str(root),'--client-host','Debian'],check=True,capture_output=True)
        stage=json.loads((root/'stages.jsonl').read_text());sut=json.loads((root/'server/sut.json').read_text())
        if sut['variant']!=slot['variant'] or sut['sha256']!=(t.BASELINE_SHA if slot['variant']=='before_off' else CANDIDATE_SHA) or sut['audit_enabled']!=(slot['variant']=='after_on'):raise ValueError('actual variant differs')
        merged=t.SERVER_INPUT+'/derived/'+rid+'.jsonl';t.remote(args,False,t.quoted(['mkdir','-p',t.SERVER_INPUT+'/derived']));t.transfer(args,False,str(root/'stages.jsonl'),merged,upload=True)
        checks=[[helper,c,'--stage-result',merged,'--stage','normal-reference'] for c in ('verify-stage','verify-sender')]
        checks.append([helper,'verify-session-counters','--scenario','w1','--workload',t.SERVER_INPUT+'/repo/tests/phase5a-baseline/workloads/forward.jsonl','--counter',server+'/fixture-forward.json','--stage-result',merged,'--run-id',rid])
        for cmd in checks:
            response=t.remote(args,False,t.quoted(cmd),check=False);code=max(code,response.returncode)
            with (root/'oracles.txt').open('a') as f:f.write(t.quoted(cmd)+'\n'+response.stdout+response.stderr+f'exit={response.returncode}\n')
        c=stage['counters'];result.update(runner_exit=code,correct=c['correct_on_time'],shortfall=c['sender_shortfall'],errors=sum(c[k] for k in ('correct_late','wrong_response','protocol_error','transport_error','timeout')),p95_us=stage['p95_us'],p99_us=stage['p99_us'])
        if not c['scheduled']==c['sent']==c['received']==3000:result['runner_exit']=1
        samples=[json.loads(l) for l in (root/'resource-samples.jsonl').read_text().splitlines() if json.loads(l)['role']=='sut']
        result['rss_peak_kib']=max(s['rss_kib'] for s in samples)
        result['cpu_seconds_bracket']=(samples[-1]['user_ticks']+samples[-1]['system_ticks']-samples[0]['user_ticks']-samples[0]['system_ticks'])/100
    except (OSError,ValueError,KeyError,subprocess.SubprocessError) as e:
        result['runner_exit']=1
        (root/'evidence-error.txt').write_text(t.error_evidence(e))
    (root/'summary.json').write_text(json.dumps(result,indent=2)+'\n');return result


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--mode',choices=('preflight','run'),required=True);parser.add_argument('--result-root',type=Path,required=True);parser.add_argument('--client-control',required=True);parser.add_argument('--reviewed-head');args=parser.parse_args()
    head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=t.REPO,text=True).strip()
    if args.mode=='run' and args.reviewed_head!=head:raise ValueError('exact reviewed HEAD required')
    root=args.result_root.resolve();root.mkdir(parents=True,exist_ok=False)
    for client in (False,True):(root/('client.txt' if client else 'server.txt')).write_text(t.inventory(args,client))
    frozen=identity(args);(root/'identity.json').write_text(json.dumps(frozen,indent=2)+'\n');(root/'plan.json').write_text(json.dumps(plan(),indent=2)+'\n');(root/'source-head.txt').write_text(head+'\n');rows=[]
    (root/'rows.json').write_text('[]\n')
    if args.mode=='preflight':return 0
    for slot in plan():
        row=run_one(args,slot,root/slot['run_id']);rows.append(row);(root/'rows.json').write_text(json.dumps(rows,indent=2)+'\n');print(slot['run_id']+' exit='+str(row['runner_exit']),flush=True)
    result=assess(rows)
    try:
        if identity(args)!=frozen:raise ValueError('inputs changed during run')
    except (OSError,ValueError,subprocess.SubprocessError) as e:
        result['passed']=False;result['failures'].append('final identity verification failed')
        (root/'identity-error.txt').write_text(t.error_evidence(e))
    (root/'assessment.json').write_text(json.dumps(result,indent=2)+'\n')
    manifest={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.rglob('*')) if p.is_file()};(root/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    return 0 if result['passed'] else 2


if __name__=='__main__':sys.exit(main())
