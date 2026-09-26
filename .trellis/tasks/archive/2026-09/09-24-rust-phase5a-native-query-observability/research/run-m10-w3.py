#!/usr/bin/env python3
"""Nine-session corrected W3 supplement; reuse existing transport and DNS oracles."""
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
TASK_DIR=HERE.parent.name
TASK_PREFIX='.trellis/tasks/'


def load(file):
    spec=importlib.util.spec_from_file_location(file,HERE/file)
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m


t=load('run-m6-w1.py')
t.HELPER_SHA='1fceab7d2f26dbd40dab8b06e56026482fd42168ac7b8d13f792c3e6076ee2f3'
t.SERVER_INPUT=t.SERVER_BASE+'/measurement-v10'
t.SERVER_RESULTS=t.SERVER_BASE+'/results-m10-server'
t.CLIENT_BASE='/root/mosdns-phase5a-m10-client'
t.CLIENT_INPUT=t.CLIENT_BASE+'/tools';t.CLIENT_RESULTS=t.CLIENT_BASE+'/results'
CANDIDATE=t.SERVER_BASE+'/candidate-m8/rust/target/release/mosdns'
CANDIDATE_SHA='13785b388787fe87f370f608f0f69392288ec0631ba8210e410aa317441130ce'
TOOLS=('run-m10-w3.py','m10-server-control.py','run-m6-w1.py','m5-remote-tools.py','measurement-revision-v10.md')


def plan():
    orders=(('before_off','after_off','after_on'),('after_off','after_on','before_off'),('after_on','before_off','after_off'))
    return [dict(scenario=scenario,repetition=i,variant=v,run_id=f'm10-{scenario}-r{i}-{v}',qps=100,duration_ms=duration,planned=duration//10) for scenario,duration in (('w3',30000),) for i,order in enumerate(orders,1) for v in order]


def inventory(args, client):
    command = "hostname; uname -srmo; getconf _NPROCESSORS_ONLN; getconf CLK_TCK; awk '/MemAvailable:/ {print $2}' /proc/meminfo; df -PT /root; ip route get " + ('10.0.0.92' if client else '10.0.0.50')
    text = t.remote(args, client, command).stdout
    rows = text.splitlines()
    if (rows[0] != ('Debian' if client else 'mosdns-rust') or int(rows[2]) != (1 if client else 2)
            or int(rows[3]) != 100 or int(rows[4]) < (786432 if client else 2097152)
            or ' ext4 ' not in text or ('src 10.0.0.50' if client else 'src 10.0.0.92') not in text):
        raise ValueError('fixed host/network/memory/filesystem preflight failed')
    helper = (t.CLIENT_INPUT if client else t.SERVER_INPUT) + '/phase5a-baseline-helper-v11'
    sha = t.remote(args, client, t.quoted(['sha256sum', helper])).stdout.split()[0]
    # Filename denotes this build; the existing helper interface version stays v10.
    if sha != t.HELPER_SHA or t.remote(args, client, t.quoted([helper, 'version'])).stdout.strip() != 'phase5a-baseline-helper/v10':
        raise ValueError('fixed helper identity failed')
    return text


def assess(rows):
    failures=[];comparisons=[]
    if [(r['scenario'],r['repetition'],r['variant']) for r in rows]!=[(r['scenario'],r['repetition'],r['variant']) for r in plan()]:
        failures.append('incomplete or wrong fixed plan')
    for r in rows:
        if r['runner_exit'] or r['correct']!=r['planned'] or r['shortfall'] or r['errors']:
            failures.append(r['run_id']+': invalid correctness/offered load')
        if any(not math.isfinite(r[k]) or r[k]<=0 for k in ('p95_us','p99_us')):
            failures.append(r['run_id']+': invalid latency')
    indexed={(r['scenario'],r['repetition'],r['variant']):r for r in rows}
    for scenario in ('w3',):
        for a,b in (('before_off','after_off'),('after_off','after_on')):
            for metric in ('p95_us','p99_us'):
                pairs=[indexed[(scenario,i,b)][metric]/indexed[(scenario,i,a)][metric] for i in range(1,4) if (scenario,i,a) in indexed and (scenario,i,b) in indexed and indexed[(scenario,i,a)][metric]>0]
                median=statistics.median(pairs) if len(pairs)==3 else None
                comparisons.append(dict(scenario=scenario,comparison=b+'_vs_'+a,metric=metric,paired_ratios=pairs,median_ratio=median))
                if median is None or median>1.10:failures.append(scenario+' '+b+'_vs_'+a+' '+metric+': median regression/incomplete')
    return dict(passed=not failures,scope='100QPS W3 only; existing Linux/W1/W2 retained; no capacity claim',failures=failures,comparisons=comparisons,rows=rows)


def identity(args):
    hashes={f:hashlib.sha256((HERE/f).read_bytes()).hexdigest() for f in TOOLS}
    files={t.SERVER_INPUT+'/phase5a-baseline-helper-v11':t.HELPER_SHA,
           '/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/mosdns-rust':t.BASELINE_SHA,
           CANDIDATE:CANDIDATE_SHA}
    files.update({t.SERVER_INPUT+'/'+f:hashes[f] for f in ('m10-server-control.py','m5-remote-tools.py')})
    client={t.CLIENT_INPUT+'/phase5a-baseline-helper-v11':t.HELPER_SHA,t.CLIENT_INPUT+'/m5-remote-tools.py':hashes['m5-remote-tools.py']}
    for scenario,name,port in (('w3','routing',15356),):
        original=(t.REPO/f'tests/phase5a-baseline/configs/{name}.yaml').read_bytes()
        off=original.replace(f'127.0.0.1:{port}'.encode(),f'10.0.0.92:{port}'.encode())
        files[t.SERVER_INPUT+'/'+scenario+'-off.yaml']=hashlib.sha256(off).hexdigest()
        files[t.SERVER_INPUT+'/'+scenario+'-on.yaml']=hashlib.sha256(off.replace(b'enable_audit: false',b'enable_audit: true')).hexdigest()
        files[t.SERVER_INPUT+f'/repo/tests/phase5a-baseline/configs/{name}.yaml']=hashlib.sha256(original).hexdigest()
        corpus=hashlib.sha256((t.REPO/f'tests/phase5a-baseline/workloads/{name}.jsonl').read_bytes()).hexdigest()
        files[t.SERVER_INPUT+f'/repo/tests/phase5a-baseline/workloads/{name}.jsonl']=corpus
        client[t.CLIENT_INPUT+'/'+name+'.jsonl']=corpus
    for is_client,expected in ((False,files),(True,client)):
        lines=t.remote(args,is_client,t.quoted(['sha256sum',*expected])).stdout.splitlines()
        if {l.split(maxsplit=1)[1]:l.split()[0] for l in lines}!=expected:raise ValueError('fixed input identity mismatch')
    return dict(local_tools=hashes,server_inputs=files,client_inputs=client)


def run_one(args,slot,root):
    root.mkdir();rid=slot['run_id'];scenario=slot['scenario'];name='routing';stage_name='normal-reference';server=t.SERVER_RESULTS+'/'+rid;client=t.CLIENT_RESULTS+'/'+rid
    helper=t.SERVER_INPUT+'/phase5a-baseline-helper-v11';code=0;started=False
    try:
        t.remote(args,True,t.quoted(['mkdir',client]))
        t.remote(args,True,t.quoted(['touch',client+'/session']))
        # A lost SSH reply can follow successful detached launches. Cleanup
        # must use this fresh session's ownership records even then.
        started=True
        t.remote(args,False,t.quoted(['python3',t.SERVER_INPUT+'/m10-server-control.py','start','--scenario',scenario,'--variant',slot['variant'],'--result',server]))
        owned=json.loads(t.remote(args,False,t.quoted(['cat',server+'/owned.json'])).stdout)
        def run_client(stage, duration, one_pass=False):
            command=['env','-u','GOMEMLIMIT','GOMAXPROCS=1','GOGC=off','GODEBUG=gctrace=1','taskset','-c','0',t.CLIENT_INPUT+'/phase5a-baseline-helper-v11','run','--sample-self','--workload',t.CLIENT_INPUT+'/'+name+'.jsonl','--scenario',scenario,'--transport','udp','--addr','10.0.0.92:'+'15356','--stage',stage,'--qps','100','--duration',str(duration)+'ms','--deadline','500ms','--late-drain','100ms','--run-id',rid,'--fixture-session-id',rid,'--result',client+'/'+stage,'--request-ledger',client+'/requests.jsonl','--fail-on-error']
            if one_pass:command+=['--one-pass']
            return t.remote(args,True,t.quoted(command)+' >'+client+'/'+stage+'.stdout 2>'+client+'/'+stage+'.stderr',timeout=duration/1000+8,check=False).returncode
        sample=server+'/samples'
        cmd=['nohup','taskset','-c','1','python3',t.SERVER_INPUT+'/m5-remote-tools.py','sample-server','--sut-pid',owned['sut']['pid'],'--fixture-pid',owned['fixture']['pid'],'--sut-start',owned['sut']['start_identity'],'--fixture-start',owned['fixture']['start_identity'],'--run-id',rid,'--stage',stage_name,'--result',sample,'--max-seconds','40']
        t.remote(args,False,t.quoted(cmd)+' </dev/null >'+server+'/sampler.stdout 2>'+server+'/sampler.stderr &');t.wait_file(args,sample+'/ready')
        try:code=max(code,run_client(stage_name,slot['duration_ms']))
        finally:t.remote(args,False,t.quoted(['touch',sample+'/stop']))
        t.wait_file(args,sample+'/metadata.json')
        t.remote(args,True,t.quoted(['mv',client+'/'+stage_name+'/stages.jsonl',client+'/'+stage_name+'/stages.client.jsonl']))
    except (OSError,ValueError,subprocess.SubprocessError) as e:
        code=1;(root/'error.txt').write_text(t.error_evidence(e))
    finally:
        if started:
            try:t.remote(args,False,t.quoted(['python3',t.SERVER_INPUT+'/m10-server-control.py','stop','--result',server]))
            except (OSError,ValueError,subprocess.SubprocessError) as e:code=1;(root/'cleanup-error.txt').write_text(t.error_evidence(e))
    result=dict(slot,runner_exit=1,correct=0,shortfall=0,errors=0,p95_us=0,p99_us=0)
    try:
        for c,path,tools in ((True,client,t.CLIENT_INPUT),(False,server,t.SERVER_INPUT)):
            t.remote(args,c,t.quoted(['python3',tools+'/m5-remote-tools.py','hash-tree','--result',path]))
        t.transfer(args,True,client,str(root/'client'),recursive=True);t.transfer(args,False,server,str(root/'server'),recursive=True)
        t.verify_transfer(root/'client','Debian');t.verify_transfer(root/'server','mosdns-rust')
        subprocess.run([sys.executable,str(HERE/'m5-remote-tools.py'),'merge-stage','--client',str(root/'client'/stage_name),'--server',str(root/'server/samples'),'--result',str(root),'--client-host','Debian'],check=True,capture_output=True)
        stage=json.loads((root/'stages.jsonl').read_text());sut=json.loads((root/'server/sut.json').read_text())
        if sut['variant']!=slot['variant'] or sut['sha256']!=(t.BASELINE_SHA if slot['variant']=='before_off' else CANDIDATE_SHA) or sut['audit_enabled']!=(slot['variant']=='after_on'):raise ValueError('actual variant differs')
        merged=t.SERVER_INPUT+'/derived/'+rid+'.jsonl';t.remote(args,False,t.quoted(['mkdir','-p',t.SERVER_INPUT+'/derived']));t.transfer(args,False,str(root/'stages.jsonl'),merged,upload=True)
        checks=[[helper,c,'--stage-result',merged,'--stage',stage_name] for c in ('verify-stage','verify-sender')]
        workload=t.SERVER_INPUT+'/repo/tests/phase5a-baseline/workloads/'+name+'.jsonl'
        server_ledger=merged+'.requests';t.transfer(args,False,str(root/'client/requests.jsonl'),server_ledger,upload=True)
        checks.append([helper,'verify-routing-events','--workload',workload,'--request-ledger',server_ledger,'--event-journal',server+'/routing-events.jsonl','--stage-result',merged,'--stage',stage_name])
        checks.append([helper,'verify-counters','--scenario','w3','--workload',workload,'--event-journal',server+'/routing-events.jsonl','--route-a',server+'/fixture-route-a.json','--route-b',server+'/fixture-route-b.json','--route-c',server+'/fixture-route-c.json'])
        for cmd in checks:
            response=t.remote(args,False,t.quoted(cmd),check=False);code=max(code,response.returncode)
            with (root/'oracles.txt').open('a') as f:f.write(t.quoted(cmd)+'\n'+response.stdout+response.stderr+f'exit={response.returncode}\n')
        c=stage['counters'];result.update(runner_exit=code,correct=c['correct_on_time'],shortfall=c['sender_shortfall'],errors=sum(c[k] for k in ('correct_late','wrong_response','protocol_error','transport_error','timeout')),p50_us=stage['p50_us'],p95_us=stage['p95_us'],p99_us=stage['p99_us'])
        if not c['scheduled']==c['sent']==c['received']==slot['planned']:result['runner_exit']=1
        samples=[json.loads(l) for l in (root/'resource-samples.jsonl').read_text().splitlines() if json.loads(l)['role']=='sut']
        result['rss_peak_kib']=max(s['rss_kib'] for s in samples)
        result['cpu_seconds_bracket']=(samples[-1]['user_ticks']+samples[-1]['system_ticks']-samples[0]['user_ticks']-samples[0]['system_ticks'])/100
    except (OSError,ValueError,KeyError,subprocess.SubprocessError) as e:
        result['runner_exit']=1
        (root/'evidence-error.txt').write_text(t.error_evidence(e))
    (root/'summary.json').write_text(json.dumps(result,indent=2)+'\n');return result


def committed_research_root(head):
    """Locate the research directory in a measuring commit.

    A task directory moves into ``archive/<month>/`` when the task closes, so
    the current path is not the path that commit recorded. Read that commit's
    tree and fail unless exactly one task research directory is recorded there.
    """
    try:listing=subprocess.check_output(['git','ls-tree','-r','--name-only','-z',head],cwd=t.REPO,text=True)
    except subprocess.CalledProcessError as error:raise ValueError(head+' has exactly one '+TASK_DIR+' research directory: unresolvable ('+str(error)+')') from error
    marker=f'/{TASK_DIR}/research/';required={'m10-preflight/identity.json',*TOOLS};found={}
    for name in listing.split('\0'):
        directory,separator,relative=name.partition(marker)
        if not separator or not relative:continue
        directory+=marker
        if not directory.startswith(TASK_PREFIX):continue
        found.setdefault(directory,set()).add(relative)
    matches=sorted(directory for directory,names in found.items() if required<=names)
    if len(matches)!=1:raise ValueError(head+' has exactly one '+TASK_DIR+' research directory: found '+str(matches))
    return Path(matches[0])


def committed_bytes(head,path):
    try:return subprocess.check_output(['git','show',head+':'+str(path)],cwd=t.REPO)
    except subprocess.CalledProcessError as error:raise ValueError(str(path)+' is missing in '+head) from error


def verify_reviewed_tools(head):
    research=committed_research_root(head)
    frozen=json.loads(committed_bytes(head,research/'m10-preflight/identity.json'))
    if set(frozen['local_tools'])!=set(TOOLS):raise ValueError('reviewed tool set differs')
    for name in TOOLS:
        committed=committed_bytes(head,research/name)
        digest=hashlib.sha256((HERE/name).read_bytes()).hexdigest()
        if digest!=hashlib.sha256(committed).hexdigest() or digest!=frozen['local_tools'][name]:
            raise ValueError('tool differs from reviewed commit/preflight: '+name)
    return frozen


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--mode',choices=('preflight','run'),required=True);parser.add_argument('--result-root',type=Path,required=True);parser.add_argument('--client-control',required=True);parser.add_argument('--reviewed-head');args=parser.parse_args()
    head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=t.REPO,text=True).strip()
    if args.mode=='run' and args.reviewed_head!=head:raise ValueError('exact reviewed HEAD required')
    reviewed=verify_reviewed_tools(head) if args.mode=='run' else None
    root=args.result_root.resolve();root.mkdir(parents=True,exist_ok=False)
    for client in (False,True):(root/('client.txt' if client else 'server.txt')).write_text(inventory(args,client))
    frozen=identity(args)
    if reviewed is not None and frozen!=reviewed:raise ValueError('inputs differ from committed reviewed preflight')
    (root/'identity.json').write_text(json.dumps(frozen,indent=2)+'\n');(root/'plan.json').write_text(json.dumps(plan(),indent=2)+'\n');(root/'source-head.txt').write_text(head+'\n');rows=[]
    (root/'rows.json').write_text('[]\n')
    if args.mode=='preflight':return 0
    for slot in plan():
        row=run_one(args,slot,root/slot['run_id']);rows.append(row);(root/'rows.json').write_text(json.dumps(rows,indent=2)+'\n');print(slot['run_id']+' exit='+str(row['runner_exit']),flush=True)
    result=assess(rows)
    try:
        if identity(args)!=frozen:raise ValueError('inputs changed during run')
        if verify_reviewed_tools(head)!=reviewed:raise ValueError('reviewed tools changed during run')
    except (OSError,ValueError,subprocess.SubprocessError) as e:
        result['passed']=False;result['failures'].append('final identity verification failed')
        (root/'identity-error.txt').write_text(t.error_evidence(e))
    (root/'assessment.json').write_text(json.dumps(result,indent=2)+'\n')
    manifest={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.rglob('*')) if p.is_file()};(root/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    return 0 if result['passed'] else 2


if __name__=='__main__':sys.exit(main())
