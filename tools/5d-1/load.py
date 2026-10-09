"""Private corpus and open-loop UDP measurement; Python stdlib only."""
import argparse
import bisect
import collections
import hashlib
import heapq
import json
import math
import os
import pathlib
import random
import re
import selectors
import socket
import sys
import time
import urllib.request

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / 'e2e-1'))
from dnswire import decode, question


def corpus(text):
    weights = collections.Counter()
    for line in text.splitlines():
        fields = line.split()
        if len(fields) < 3 or not fields[0].isdigit():
            continue
        name = fields[2].split('|')[0].rstrip('.').lower()
        if len(name) <= 253 and re.fullmatch(r'[a-z0-9_-]+(?:\.[a-z0-9_-]+)+', name) and all(len(p) <= 63 for p in name.split('.')):
            weights[name] += int(fields[0])
    if not weights or not sum(weights.values()):
        raise ValueError('no weighted production corpus')
    rows = []
    for name, weight in weights.items():
        for qt, fraction in ((1, .7), (28, .3)):
            rows.append(dict(name=name, type=qt, weight=weight*fraction, kind='production'))
    probes = ('block', 'rewrite-ip', 'rewrite-name', 'unmatched')
    probe_weight = sum(weights.values()) * .02 / .98 / (len(probes)*2)
    for probe in probes:
        for qt in (1, 28):
            rows.append(dict(name=f'e2e1-{probe}.example.com', type=qt, weight=probe_weight, kind=probe))
    return rows


def semantic(value):
    # Normalize only TTL; preserve all three DNS sections, question and rcode.
    return json.dumps([value['rcode'], value['questions'],
                       *[sorted([list(r[:4]) for r in value.get(s, [])])
                         for s in ('answers', 'authority', 'additional')]], sort_keys=True)


def quantiles(values):
    values = sorted(values)
    if not values:
        return {f'p{p}': None for p in (50, 95, 99, 99.9)}
    return {f'p{p}': round(values[math.ceil(len(values)*p/100)-1], 6)
            for p in (50, 95, 99, 99.9) if p != 99.9 or len(values) >= 10000}


def summarize(rows, duration, deadline):
    counts = collections.Counter(r['status'] for r in rows)
    sent = sum(r['sent'] is not None for r in rows)
    completed = [r for r in rows if r['finished'] is not None]
    correct = sum(r['status'] == 'correct' and r['finished']-r['planned'] <= deadline for r in rows)
    return dict(planned=len(rows), sent=sent, received=len(completed), correct_on_time=correct,
                correct_rate=correct/len(rows) if rows else 0,
                sent_correct_rate=correct/sent if sent else 0, send_coverage=sent/len(rows) if rows else 0,
                statuses=dict(counts), effective_qps=correct/duration,
                planned_latency_ms=quantiles([(r['finished']-r['planned'])*1000 for r in completed]),
                sent_latency_ms=quantiles([(r['finished']-r['sent'])*1000 for r in completed]),
                dispatch_lag_ms=quantiles([(r['sent']-r['planned'])*1000 for r in rows if r['sent'] is not None]))


def classify(value, expected, finished, planned, deadline):
    ttl_ok=all(0<=rr[4]<=300 for section in ('answers','authority','additional') for rr in value[section])
    status='correct' if semantic(value)==expected and ttl_ok else 'wrong'
    return 'late_'+status if finished-planned>deadline else status


def resource(pid):
    root = pathlib.Path(f'/proc/{pid}')
    try:
        fields = (root/'stat').read_text().rsplit(')', 1)[1].split()
        status = dict(line.split(':', 1) for line in (root/'status').read_text().splitlines() if ':' in line)
        return dict(cpu_s=(int(fields[11])+int(fields[12]))/os.sysconf('SC_CLK_TCK'),
                    rss_kib=int(status['VmRSS'].split()[0]), fd=len(list((root/'fd').iterdir())),
                    threads=int(status['Threads']), affinity=status['Cpus_allowed_list'].strip())
    except (OSError, KeyError):
        return {'missing': True}


def sample(pids):
    cpus = {}
    for line in pathlib.Path('/proc/stat').read_text().splitlines():
        fields = line.split()
        if fields and fields[0] in ('cpu0', 'cpu1'):
            cpus[fields[0]] = list(map(int, fields[1:]))
    return dict(time=time.monotonic(), processes={k: resource(v) for k,v in pids.items()}, cpu=cpus)


def metrics(port):
    with urllib.request.urlopen(f'http://127.0.0.1:{port}/metrics', timeout=5) as response:
        return response.read().decode()


def characterize(rows, output):
    # A closed-loop functional oracle, never used as performance evidence.
    import concurrent.futures
    def probe(pair):
        index, row = pair
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.connect(('127.0.0.1', 53))
            sock.settimeout(5)
            sock.send(question(row['name'], row['type'], index % 65536))
            value = decode(sock.recv(65535))
        if value['id'] != index % 65536 or value['questions'] != [(row['name']+'.', row['type'],1)] or value['rcode'] == 2:
            raise ValueError('oracle wrong question/id or SERVFAIL at index '+str(index))
        return dict(index=index, semantic=semantic(value), dns=value)
    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        values = list(pool.map(probe, enumerate(rows)))
    output.write_text(json.dumps(values))
    print(json.dumps({'oracle_rows':len(values), 'sha256':hashlib.sha256(output.read_bytes()).hexdigest()}), flush=True)


def measure(args):
    args.output.mkdir()
    rows = json.loads(args.corpus.read_text())
    if getattr(args,'unique',False) and len({r['name'] for r in rows}) != len(rows):
        raise ValueError('unique mode requires globally unique names')
    expected = {r['index']:r['semantic'] for r in json.loads(args.oracle.read_text())}
    cumulative, total = [], 0
    for row in rows:
        total += row['weight']; cumulative.append(total)
    selector, sockets = selectors.DefaultSelector(), []
    for i in range(128):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.connect(('127.0.0.1', 53)); sock.setblocking(False)
        selector.register(sock, selectors.EVENT_READ, i); sockets.append(sock)
    rng = random.Random(args.seed)
    pids = dict(sut=args.pid, mock=args.mock_pid, sender=os.getpid())
    samples, all_rows, summaries, active, expired = [], [], [], {}, []
    orphan = 0
    stages = [('warmup', 100, args.warmup), *[(f'q{q}',q,args.duration) for q in args.rates], ('recovery',100,args.recovery)]
    next_sample = 0
    prepared, offset = [], 0
    for stage, rate, duration in stages:
        data, wires = [], []
        for i in range(round(rate*duration)):
            seq=len(all_rows)+i
            idx=seq if getattr(args,'unique',False) else bisect.bisect(cumulative,rng.random()*total)
            if idx >= len(rows): raise ValueError('unique corpus exhausted')
            if seq//128 >= 65536: raise ValueError('unique DNS ID envelope exceeded')
            wires.append(question(rows[idx]['name'],rows[idx]['type'],seq//128))
            data.append(dict(stage=stage,seq=seq,index=idx,planned=offset+i/rate,sent=None,finished=None,status='unsent'))
        all_rows.extend(data); prepared.append((offset,data,wires)); offset+=duration
    metric_values = {'start':metrics(args.api_port)}
    startup=sample(pids); samples.append(startup)
    start = time.monotonic()+1
    for row in all_rows: row['planned']+=start
    try:
        for (stage, rate, duration),(offset,stage_rows,wires) in zip(stages,prepared):
            # The entire ladder was prepared before timing: no gap before recovery.
            t0 = start+offset
            cursor = 0
            while cursor < len(stage_rows) or time.monotonic() < t0+duration:
                now = time.monotonic()
                if now >= next_sample:
                    samples.append(sample(pids)); next_sample = now+.25
                while expired and expired[0][0] <= now:
                    _,key = heapq.heappop(expired)
                    row = active.pop(key, None)
                    if row is not None: row['status']='timeout'
                while cursor < len(stage_rows) and stage_rows[cursor]['planned'] <= now:
                    row=stage_rows[cursor]; key=(row['seq']%128,row['seq']//128)
                    if now-row['planned'] <= .005:
                        try:
                            sockets[key[0]].send(wires[cursor]); row['sent']=time.monotonic(); row['status']='pending'
                            active[key]=row; heapq.heappush(expired,(row['planned']+args.deadline,key))
                        except OSError:
                            row['status']='send_error'
                    cursor+=1
                    now=time.monotonic()
                wait_until = stage_rows[cursor]['planned'] if cursor < len(stage_rows) else t0+duration
                for event,_ in selector.select(max(0,min(.005,wait_until-time.monotonic()))):
                    sock=event.fileobj
                    # Bound receive batches so responses never stop the send schedule.
                    for _ in range(32):
                        try: wire=sock.recv(65535)
                        except BlockingIOError: break
                        finished=time.monotonic()
                        try:
                            value=decode(wire); key=(event.data,value['id']); row=active.pop(key,None)
                            if row is None:
                                seq=value['id']*128+event.data
                                row=all_rows[seq] if seq<len(all_rows) else None
                                if row is None or row['sent'] is None or row['finished'] is not None:
                                    orphan+=1; continue
                            row['finished']=finished
                            row['status']=classify(value,expected[row['index']],finished,row['planned'],args.deadline)
                        except (ValueError,IndexError,KeyError,OSError):
                            orphan+=1
            # Outstanding traffic remains live across stages, including recovery.
            samples.append(sample(pids))
            print(json.dumps({'stage_finished':stage,'rate':rate}),flush=True)
        drain_end=time.monotonic()+args.deadline
        while time.monotonic()<drain_end:
            for event,_ in selector.select(.01):
                try:
                    value=decode(event.fileobj.recv(65535)); row=active.pop((event.data,value['id']),None)
                    if row:
                        row['finished']=time.monotonic()
                        row['status']=classify(value,expected[row['index']],row['finished'],row['planned'],args.deadline)
                    else: orphan+=1
                except (OSError,ValueError,IndexError): orphan+=1
        for row in active.values(): row['status']='timeout'
        metric_values['end']=metrics(args.api_port)
    finally:
        for sock in sockets: sock.close()
        selector.close()
    for stage,rate,duration in stages:
        data=[r for r in all_rows if r['stage']==stage]
        settle=min(5,duration/6) if stage!='recovery' else 0
        stable=[r for r in data if r['planned']>=data[0]['planned']+settle]
        lo=data[0]['planned']+settle; hi=data[0]['planned']+duration
        window=[s for s in samples if lo<=s['time']<=hi]
        sut=[s['processes']['sut'] for s in window if not s['processes']['sut'].get('missing')]
        result=dict(stage=stage,rate=rate,duration=duration,all=summarize(data,duration,args.deadline),
                    stable=summarize(stable,duration-settle,args.deadline))
        if len(sut)>=2:
            cpu=sut[-1]['cpu_s']-sut[0]['cpu_s']
            result['resources']=dict(cpu_s=cpu,cpu_us_per_correct=cpu*1e6/max(1,result['stable']['correct_on_time']),
                                     rss_median_kib=sorted(s['rss_kib'] for s in sut)[len(sut)//2],
                                     rss_peak_kib=max(s['rss_kib'] for s in sut),
                                     fd_start=sut[0]['fd'],fd_end=sut[-1]['fd'],threads_end=sut[-1]['threads'])
        if stage=='recovery':
            result['seconds']=[summarize([r for r in data if int(r['planned']-data[0]['planned'])==i],1,args.deadline) for i in range(int(duration))]
        summaries.append(result)
    (args.output/'requests.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in all_rows))
    (args.output/'resources.json').write_text(json.dumps(samples))
    (args.output/'metrics.json').write_text(json.dumps(metric_values))
    summary=dict(seed=args.seed,corpus_sha256=hashlib.sha256(args.corpus.read_bytes()).hexdigest(),
                 oracle_sha256=hashlib.sha256(args.oracle.read_bytes()).hexdigest(),started=start,
                 startup=startup['processes']['sut'],orphan_or_late_packets=orphan,stages=summaries)
    (args.output/'summary.json').write_text(json.dumps(summary,indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('mode',choices=('corpus','oracle','measure'))
    parser.add_argument('--corpus',type=pathlib.Path,required=True)
    parser.add_argument('--oracle',type=pathlib.Path)
    parser.add_argument('--output',type=pathlib.Path)
    parser.add_argument('--top',type=pathlib.Path)
    parser.add_argument('--rates',type=int,nargs='+',default=[20,100,500,1000,2000,4000,8000,16000])
    parser.add_argument('--duration',type=float,default=30)
    parser.add_argument('--warmup',type=float,default=10)
    parser.add_argument('--recovery',type=float,default=60)
    parser.add_argument('--deadline',type=float,default=.5)
    parser.add_argument('--seed',type=int,default=501)
    parser.add_argument('--unique',action='store_true')
    parser.add_argument('--pid',type=int)
    parser.add_argument('--mock-pid',type=int)
    parser.add_argument('--api-port',type=int,default=9099)
    args=parser.parse_args()
    if args.mode=='corpus': args.corpus.write_text(json.dumps(corpus(args.top.read_text())))
    elif args.mode=='oracle': characterize(json.loads(args.corpus.read_text()),args.oracle)
    else: measure(args)
