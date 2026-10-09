"""Excluded diagnostic runs: perf, RSS maps, audit isolation, DNS differences."""
import argparse
import concurrent.futures
import json
import os
import pathlib
import re
import signal
import socket
import subprocess
import sys
import time

from load import semantic
from dnswire import question, decode


def probes(root, indexes, work, role, passes):
    rows=json.loads((work/'weighted-corpus.json').read_text())
    reference=json.loads((work/('native-oracle.json' if role=='native' else 'oracle.json')).read_text())
    def query(index):
        row=rows[index]
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
            sock.settimeout(5); sock.connect(('127.0.0.1',53))
            sock.send(question(row['name'],row['type'],index))
            value=decode(sock.recv(65535))
        ttl_ok=all(0<=rr[4]<=300 for section in ('answers','authority','additional') for rr in value[section])
        return dict(index=index,input=row,dns=value,expected=reference[index]['semantic'],
                    semantic_matches=semantic(value)==reference[index]['semantic'],ttl_bounded=ttl_ok,
                    matches=semantic(value)==reference[index]['semantic'] and ttl_ok and value['id']==index)
    # This branch executes inside the owned namespace in a separate invocation.
    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        values=[]
        for repetition in range(passes):
            for value in pool.map(query,indexes):
                value['pass']=repetition; values.append(value)
    (root/'dns-probes-private.json').write_text(json.dumps(values))


def run(args):
    def interrupted(signum,frame): raise InterruptedError('diagnostic interrupted')
    signal.signal(signal.SIGTERM,interrupted); signal.signal(signal.SIGINT,interrupted)
    here=pathlib.Path(__file__).resolve().parent
    argv=[sys.executable,str(here/'run.py'),'run','--work',str(args.work),'--output',str(args.output),
          '--binary',str(args.binary),'--role',args.role,'--seed',str(args.seed),'--freeze-record',str(args.freeze_record),
          '--rates',*map(str,args.rates),'--duration',str(args.duration),'--warmup','10','--recovery',str(args.recovery)]
    if args.role=='native': argv+=['--oracle',str(args.work/'native-oracle.json')]
    if args.scenario_f: argv+=['--scenario-f']
    env=dict(os.environ)
    if args.observe: env['FIVE_D_OBSERVE_PATH']=str(args.output.parent/(args.output.name+'-observer.jsonl'))
    logs=[]; child=None; perf=None
    try:
        stream=(args.output.parent/(args.output.name+'-driver.log')).open('w'); logs.append(stream)
        child=subprocess.Popen(argv,stdout=stream,stderr=subprocess.STDOUT,env=env,start_new_session=True)
        end=time.monotonic()+90
        while not (args.output/'run.json').exists():
            if child.poll() is not None or time.monotonic()>end: raise RuntimeError('diagnostic host not ready')
            time.sleep(.1)
        meta=json.loads((args.output/'run.json').read_text())
        pid=meta['sut_pid']; ns=subprocess.check_output(['ip','netns','identify',str(pid)],text=True).strip()
        port=json.loads((args.output/'runtime/webinfo/webui_port_settings.json').read_text())['port'] if args.role=='go' else 9099
        if args.audit_off:
            script='import urllib.request; urllib.request.urlopen(urllib.request.Request("http://127.0.0.1:%d/api/v1/audit/stop",data=b"{}",headers={"Content-Type":"application/json"})).read()'%port
            subprocess.run(['ip','netns','exec',ns,sys.executable,'-c',script],check=True)
        anchor=dict(epoch_s=time.time(),monotonic_s=time.monotonic(),audit_off=args.audit_off,
                    observer=args.observe,excluded_from_formal=True,binary=str(args.binary))
        (args.output/'diagnostic.json').write_text(json.dumps(anchor))
        if args.indexes:
            subprocess.run(['ip','netns','exec',ns,'taskset','-c','1',sys.executable,str(here/'diagnostic.py'),
                            '--probe-only','--output',str(args.output),'--work',str(args.work),
                            '--role',args.role,'--indexes',str(args.indexes),'--probe-passes',str(args.probe_passes)],check=True)
        began=time.monotonic(); next_maps=began; perf_started=False
        with (args.output/'memory-maps.jsonl').open('w') as maps:
            while child.poll() is None:
                now=time.monotonic()
                if now>=next_maps:
                    record=dict(time=now,elapsed_s=now-began)
                    for name in ('smaps_rollup','status'):
                        try: record[name]=pathlib.Path(f'/proc/{pid}/{name}').read_text()
                        except OSError: pass
                    maps.write(json.dumps(record)+'\n'); maps.flush(); next_maps=now+5
                if args.perf and not perf_started and now-began>=20:
                    stream=(args.output/'perf.log').open('w'); logs.append(stream)
                    perf=subprocess.Popen(['perf','record','-e','cpu-clock','-F','499','-g','--call-graph','dwarf','-p',str(pid),
                                           '-o',str(args.output/'perf.data'),'--','sleep','60'],
                                          stdout=stream,stderr=subprocess.STDOUT,start_new_session=True)
                    anchor['perf_pid']=perf.pid
                    (args.output/'diagnostic.json').write_text(json.dumps(anchor))
                    perf_started=True
                time.sleep(.25)
        if child.returncode: raise RuntimeError('diagnostic run failed')
        if perf:
            if perf.wait()!=0: raise RuntimeError('perf failed')
            with (args.output/'perf-report.txt').open('w') as report:
                subprocess.run(['perf','report','--stdio','--no-children','--percent-limit','0.5',
                                '-i',str(args.output/'perf.data')],stdout=report,check=True)
            text=(args.output/'perf-report.txt').read_text()
            samples=re.search(r'# Samples:\s*([0-9.]+)',text)
            if samples is None or float(samples[1])<=0:
                raise RuntimeError('perf produced no samples; do not interpret the profile')
    finally:
        for process in (perf,child):
            if process is not None and process.poll() is None:
                os.killpg(process.pid,signal.SIGTERM)
                try: process.wait(timeout=35)
                except subprocess.TimeoutExpired: os.killpg(process.pid,signal.SIGKILL); process.wait()
        for stream in logs: stream.close()


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for flag in ('work','output','binary','freeze-record','indexes'):
        parser.add_argument('--'+flag,type=pathlib.Path,required=flag in ('work','output'))
    parser.add_argument('--role',choices=('go','native'),default='native')
    parser.add_argument('--rates',type=int,nargs='+',default=[8000])
    parser.add_argument('--duration',type=float,default=120)
    parser.add_argument('--recovery',type=float,default=60)
    parser.add_argument('--seed',type=int,default=501)
    parser.add_argument('--probe-passes',type=int,default=3)
    for flag in ('perf','observe','audit-off','scenario-f','probe-only'):
        parser.add_argument('--'+flag,action='store_true')
    args=parser.parse_args()
    if not args.probe_only and (args.binary is None or args.freeze_record is None):
        parser.error('diagnostic run requires --binary and --freeze-record')
    if args.probe_only: probes(args.output,json.loads(args.indexes.read_text()),args.work,args.role,args.probe_passes)
    else: run(args)
