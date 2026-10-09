"""Root-only VM driver. Private inputs and outputs; never installs/deploys."""
import argparse
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import time

HERE=pathlib.Path(__file__).resolve().parent
E2E=HERE.parent/'e2e-1'
sys.path.insert(0,str(E2E))
from prepare import prepare, load, save
from load import corpus, resource


def command(argv, **kwargs):
    return subprocess.run(list(map(str,argv)),check=True,**kwargs)


def cert(root):
    quiet=dict(stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    command(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',root/'ca-key.pem','-out',root/'ca.pem','-days','7','-subj','/CN=5D1-local-CA','-addext','basicConstraints=critical,CA:TRUE'],**quiet)
    command(['openssl','req','-newkey','rsa:2048','-nodes','-keyout',root/'key.pem','-out',root/'leaf.csr','-subj','/CN=localhost'],**quiet)
    (root/'leaf.ext').write_text('subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n')
    command(['openssl','x509','-req','-in',root/'leaf.csr','-CA',root/'ca.pem','-CAkey',root/'ca-key.pem','-CAcreateserial','-out',root/'cert.pem','-days','7','-extfile',root/'leaf.ext'],**quiet)


def setup(args):
    args.work.mkdir(mode=0o700)
    prepare(args.work,args.inputs,args.migrate,10)
    for role in ('go','native'):
        root=args.work/role
        cert(root)
        save(root/'mock-delays.json',dict(domestic=10,foreign=80,foreignecs=80,cnfake=1,nocnfake=1))
    rows=corpus((args.inputs/'snapshot/gen/top_domains.txt').read_text())
    save(args.work/'weighted-corpus.json',rows)
    save(args.work/'corpus-public.json',dict(production_names=(len(rows)-8)//2,probe_rows=8,production_fraction=.98,A_fraction=.7))


def run(args):
    if os.geteuid()!=0: raise ValueError('VM root required')
    if args.role=='native':
        if args.mode=='oracle': raise ValueError('oracle comes from Go only')
        if args.smoke:
            if args.duration>3 or args.warmup>3 or args.recovery>3 or max(args.rates)>100:
                raise ValueError('native smoke is limited to short tool verification')
        elif not args.freeze_record or not args.freeze_record.is_file():
            raise ValueError('native formal measurement requires user-frozen threshold record')
    def interrupted(signum,frame): raise InterruptedError('interrupted')
    signal.signal(signal.SIGTERM,interrupted); signal.signal(signal.SIGINT,interrupted)
    args.output.mkdir(mode=0o700)
    root=args.output/'runtime'
    shutil.copytree(args.work/args.role,root)
    corpus_path, oracle_path = args.work/'weighted-corpus.json', args.oracle or args.work/'oracle.json'
    if args.scenario_f:
        if args.mode != 'measure' or args.rates != [100]:
            raise ValueError('scenario F requires measurement at 100 QPS only')
        from scenario_f import prepare as prepare_f
        corpus_path, oracle_path = prepare_f(root,args.role,round(100*(args.warmup+args.duration+args.recovery)))
    for path in (root/'cache').glob('*.dump'): path.unlink()
    (root/'tmp').mkdir(); (root/'empty-certs').mkdir(exist_ok=True)
    before=subprocess.check_output(['systemctl','show','mosdns','-p','MainPID','-p','ActiveState'])
    (args.output/'service-before.txt').write_bytes(before)
    ns=f'5d1-{os.getpid()}'
    owned,logs,created=[],[],False
    env=dict(os.environ,SSL_CERT_FILE=str(root/'ca.pem'),SSL_CERT_DIR=str(root/'empty-certs'),GOMAXPROCS='1',PYTHONDONTWRITEBYTECODE='1')
    for key in ('GOGC','GOMEMLIMIT','GODEBUG','HTTP_PROXY','HTTPS_PROXY','ALL_PROXY','http_proxy','https_proxy','all_proxy'):
        env.pop(key,None)
    def spawn(cpu,argv,name):
        stream=(args.output/name).open('w'); logs.append(stream)
        p=subprocess.Popen(['ip','netns','exec',ns,'taskset','-c',str(cpu),*map(str,argv)],stdout=stream,stderr=subprocess.STDOUT,env=env,start_new_session=True)
        owned.append(p); return p
    try:
        command(['ip','netns','add',ns]); created=True
        command(['ip','-n',ns,'link','set','lo','up'])
        (args.output/'routes.txt').write_bytes(subprocess.check_output(['ip','-n',ns,'route','show']))
        mock=spawn(1,['python3',E2E/'mock.py',root],'mock.log')
        end=time.monotonic()+10
        while not (root/'mock-ready').exists():
            if mock.poll() is not None or time.monotonic()>end: raise RuntimeError('mock not ready')
            time.sleep(.1)
        host=spawn(0,['unshare','--mount','--propagation','private','sh','-c','mount --bind "$1" /tmp; shift; exec "$@"','5d1',root/'tmp',args.binary,'start','-d',root,'-c','config_custom.yaml'],'host.log')
        port=load(root/'webinfo/webui_port_settings.json')['port'] if args.role=='go' else 9099
        startup=[]; end=time.monotonic()+90
        while True:
            startup.append(dict(time=time.monotonic(),**resource(host.pid)))
            listening=subprocess.check_output(['ip','netns','exec',ns,'sh','-c',"cat /proc/net/udp /proc/net/udp6"])
            if b':0035 ' in listening: break
            if host.poll() is not None or time.monotonic()>end: raise RuntimeError('host not ready')
            time.sleep(.25)
        (args.output/'startup.json').write_text(json.dumps(startup))
        # Match audit-on capture on both hosts without changing snapshot capacity.
        api_code='import urllib.request; urllib.request.urlopen(urllib.request.Request("http://127.0.0.1:%d/api/v1/audit/start",data=b"{}",headers={"Content-Type":"application/json"}),timeout=10).read()'%port
        command(['ip','netns','exec',ns,'python3','-c',api_code])
        save(args.output/'run.json',dict(role=args.role,binary=str(args.binary),sut_pid=host.pid,mock_pid=mock.pid,seed=args.seed,
                                       sut_affinity=resource(host.pid)['affinity'],mock_affinity=resource(mock.pid)['affinity'],
                                       GOMAXPROCS='1',GOGC='default',GOMEMLIMIT='default',GODEBUG='default',audit='on',scenario='F' if args.scenario_f else 'mixed'))
        argv=['python3',HERE/'load.py',args.mode,'--corpus',corpus_path,'--oracle',oracle_path]
        if args.scenario_f: argv+=['--unique']
        if args.mode=='measure':
            argv+=['--output',args.output/'load','--pid',host.pid,'--mock-pid',mock.pid,'--api-port',port,'--seed',args.seed,
                   '--duration',args.duration,'--warmup',args.warmup,'--recovery',args.recovery,'--rates',*args.rates]
        sender=spawn(1,argv,'sender.log')
        if sender.wait()!=0: raise RuntimeError('sender failed; inspect private sender.log')
        if host.poll() is not None: raise RuntimeError('host exited during load')
    finally:
        for p in reversed(owned):
            if p.poll() is None:
                os.killpg(p.pid,signal.SIGTERM)
                try: p.wait(timeout=30)
                except subprocess.TimeoutExpired: os.killpg(p.pid,signal.SIGKILL); p.wait()
        if created:
            remaining=subprocess.check_output(['ip','netns','pids',ns]).strip()
            if remaining: raise RuntimeError('namespace contains remaining processes')
            command(['ip','netns','delete',ns])
        for stream in logs: stream.close()
        after=subprocess.check_output(['systemctl','show','mosdns','-p','MainPID','-p','ActiveState'])
        save(args.output/'cleanup.json',dict(all_owned_processes_exited=all(p.poll() is not None for p in owned),namespace_deleted=created,original_service_unchanged=after==before))
        if before!=after: raise RuntimeError('original service changed')


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('action',choices=('prepare','run'))
    for flag in ('work','inputs','migrate','output','binary'):
        parser.add_argument('--'+flag,type=lambda s:pathlib.Path(s).resolve(),required=flag=='work')
    parser.add_argument('--role',choices=('go','native'),default='go')
    parser.add_argument('--mode',choices=('oracle','measure'),default='measure')
    parser.add_argument('--rates',type=int,nargs='+',default=[20,100,500,1000,2000,4000,8000,16000])
    parser.add_argument('--duration',type=float,default=30)
    parser.add_argument('--warmup',type=float,default=10)
    parser.add_argument('--recovery',type=float,default=60)
    parser.add_argument('--seed',type=int,default=501)
    parser.add_argument('--smoke',action='store_true')
    parser.add_argument('--scenario-f',action='store_true')
    parser.add_argument('--freeze-record',type=pathlib.Path)
    parser.add_argument('--oracle',type=pathlib.Path)
    args=parser.parse_args()
    setup(args) if args.action=='prepare' else run(args)
