"""Serial interleaved controls/candidates after a user-frozen JSON record."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import sys


def run(root, freeze, resume_f_schema_fix=False):
    record = json.loads(freeze.read_text())
    if record.get('confirmed_by') != '用户' or 'F' not in record['latency_ms']:
        raise ValueError('complete user freeze including F required')
    frozen_hash = hashlib.sha256(freeze.read_bytes()).hexdigest()
    here = pathlib.Path(__file__).resolve().parent
    measured_files=[here/name for name in ('load.py','run.py','scenario_f.py')]
    measured_files+=[here.parent/'e2e-1'/name for name in ('prepare.py','mock.py','dnswire.py')]
    def tool_hashes():
        return {str(path.relative_to(here.parent.parent)):hashlib.sha256(path.read_bytes()).hexdigest() for path in measured_files}
    initial_tools=tool_hashes()
    plan = []
    for repetition, seed in enumerate((501, 502, 503), 1):
        for matrix in ('main', 'ext', 'f'):
            for role in (('go','native') if repetition != 2 else ('native','go')):
                name = f'{role}-control-{matrix}-r{repetition}' if role=='go' else f'native-{matrix}-r{repetition}'
                plan.append(dict(name=name,role=role,matrix=matrix,seed=seed))
    evidence = dict(freeze_sha256=frozen_hash,tool_hashes=initial_tools,plan=plan,completed=[])
    manifest = root/'matrix-progress.json'
    if resume_f_schema_fix:
        evidence=json.loads(manifest.read_text())
        if evidence['freeze_sha256']!=frozen_hash or evidence['plan']!=plan:
            raise ValueError('resume must preserve the exact freeze and plan')
        changed=[name for name,value in initial_tools.items() if evidence['tool_hashes'].get(name)!=value]
        if changed!=['tools/5d-1/scenario_f.py']:
            raise ValueError('only the F schema mapper may change for this resume')
        completed=evidence['completed']
        if completed != [item['name'] for item in plan[:len(completed)]]:
            raise ValueError('completed prefix mismatch')
        failed=plan[len(completed)]
        failed_root=root/failed['name']
        if failed['role']!='native' or failed['matrix']!='f' or (failed_root/'load/summary.json').exists():
            raise ValueError('resume is only for native F startup failure before samples')
        if not all(json.loads((failed_root/'cleanup.json').read_text()).values()):
            raise ValueError('failed attempt cleanup incomplete')
        archive=failed['name']+'-startup-failure'
        failed_root.rename(root/archive)
        (root/(failed['name']+'.log')).rename(root/(archive+'.log'))
        evidence['F_schema_correction']=dict(previous_tool_hashes=evidence['tool_hashes'],
                                             failed_attempt=archive,native_formal_F_samples_before_fix=0)
        evidence['tool_hashes']=initial_tools
    manifest.write_text(json.dumps(evidence,indent=2))
    for item in plan:
        if item['name'] in evidence['completed']: continue
        if hashlib.sha256(freeze.read_bytes()).hexdigest()!=frozen_hash:
            raise ValueError('freeze changed during measurement')
        if tool_hashes()!=initial_tools: raise ValueError('measurement tools changed during matrix')
        argv=[sys.executable,str(here/'run.py'),'run','--work',str(root/'prepared-final'),
              '--output',str(root/item['name']),'--binary',str(root/('mosdns-'+item['role'])),
              '--role',item['role'],'--seed',str(item['seed']),'--freeze-record',str(freeze)]
        if item['role']=='native': argv+=['--oracle',str(root/'prepared-final/native-oracle.json')]
        if item['matrix']=='ext': argv+=['--rates','32000','64000']
        if item['matrix']=='f': argv+=['--scenario-f','--rates','100']
        print(json.dumps(dict(starting=item)),flush=True)
        with (root/(item['name']+'.log')).open('w') as log:
            subprocess.run(argv,check=True,stdout=log,stderr=subprocess.STDOUT)
        cleanup=json.loads((root/item['name']/'cleanup.json').read_text())
        if not all(cleanup.values()): raise ValueError('incomplete cleanup')
        evidence['completed'].append(item['name'])
        manifest.write_text(json.dumps(evidence,indent=2))
        print(json.dumps(dict(completed=item['name'])),flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--root',type=pathlib.Path,required=True)
    parser.add_argument('--freeze-record',type=pathlib.Path,required=True)
    parser.add_argument('--resume-f-schema-fix',action='store_true')
    args=parser.parse_args()
    run(args.root,args.freeze_record,args.resume_f_schema_fix)
