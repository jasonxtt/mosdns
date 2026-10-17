"""How long DNS is unavailable during a real in-place restart (VM, isolated netns).

    python3 tools/f1-interop/restart_gap.py run --work prepared --binary mosdns-native --output gap-1 [--seed 501]

Runs the host on the prepared production-shaped configuration (reserved ports, mock
upstreams, the 5D-2 harness), sends one UDP query every 10 ms from inside the namespace,
asks for a restart after 6 s and reports the longest stretch without an answer, the time
until the management API answers again, the process id before and after, and the cache
dump the final save wrote. `probe` is the in-namespace half.
"""
import argparse
import json
import os
import pathlib
import selectors
import socket
import sys
import time
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / '5d-2'))
sys.path.insert(0, str(HERE.parent / '5d-1'))
sys.path.insert(0, str(HERE.parent / 'e2e-1'))

DURATION = 40.0
RESTART_AT = 6.0
INTERVAL = 0.010


def gaps(samples):
    """Pure: unanswered stretches. `samples` = [(sent, answered_or_None)] in send order; a stretch
    runs from the first unanswered probe's send time to the next answered probe's send time."""
    result, start = [], None
    for sent, answered in samples:
        if answered is None:
            start = sent if start is None else start
        elif start is not None:
            result.append(dict(start=start, end=sent, seconds=sent - start))
            start = None
    if start is not None:
        result.append(dict(start=start, end=None, seconds=None))
    return result


def probe(args):
    from dnswire import decode, question
    rows = json.loads(pathlib.Path(args.corpus).read_text())
    names = [(r['name'], r['type']) for r in rows[:400]]
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.connect(('127.0.0.1', 53))
    sock.setblocking(False)
    selector = selectors.DefaultSelector()
    selector.register(sock, selectors.EVENT_READ)
    start = time.monotonic() + 0.5
    samples, pending, index, restarted = [], {}, 0, None
    api = f'http://127.0.0.1:{args.api_port}'
    health_failed_at = health_back_at = None
    while True:
        now = time.monotonic()
        if now - start > DURATION:
            break
        planned = start + index * INTERVAL
        if now >= planned:
            name, qtype = names[index % len(names)]
            ident = index % 65536
            try:
                sock.send(question(name, qtype, ident))
            except OSError:
                pass
            pending[ident] = len(samples)
            samples.append([now, None])
            index += 1
        if restarted is None and now - start >= RESTART_AT:
            try:
                request = urllib.request.Request(api + '/api/v1/system/restart', data=b'{"delay_ms": 300}',
                                                 method='POST', headers={'Content-Type': 'application/json'})
                restarted = (now, urllib.request.urlopen(request, timeout=5).status)
            except OSError as error:
                restarted = (now, repr(error))
        if restarted is not None and health_back_at is None and index % 10 == 0:
            try:
                urllib.request.urlopen(api + '/api/v1/system/health', timeout=0.05).read()
                if health_failed_at is not None:
                    health_back_at = time.monotonic()
            except OSError:
                if health_failed_at is None and time.monotonic() - restarted[0] > 0.2:
                    health_failed_at = time.monotonic()
        for key, _ in selector.select(0.001):
            while True:
                try:
                    wire = key.fileobj.recv(65535)
                except BlockingIOError:
                    break
                except OSError:
                    break
                try:
                    value = decode(wire)
                except Exception:
                    continue
                slot = pending.pop(value['id'], None)
                if slot is not None and samples[slot][1] is None:
                    samples[slot][1] = time.monotonic()
    pathlib.Path(args.output).write_text(json.dumps(dict(
        start=start, restart=restarted, health_failed_at=health_failed_at, health_back_at=health_back_at,
        samples=samples)))


def run(args):
    from harness import Env
    with Env(args.work, 'native', args.binary, args.output, seed=args.seed, saved_port_applies=True) as env:
        pid = env.host.pid
        exe_before = os.readlink(f'/proc/{pid}/exe')
        result_file = args.output / 'probe.json'
        sender = env.spawn(1, [sys.executable, HERE / 'restart_gap.py', 'probe', '--corpus', args.work / 'weighted-corpus.json',
                               '--api-port', env.api_port, '--output', result_file], 'probe.log')
        if sender.wait() != 0:
            raise RuntimeError('probe failed; inspect probe.log')
        alive = env.host.poll() is None
        exe_after = os.readlink(f'/proc/{pid}/exe') if alive else None
        dump_files = sorted(str(p.relative_to(env.root)) for p in (env.root / 'cache').glob('*.dump')) if (env.root / 'cache').exists() else []
        dump_bytes = sum((env.root / p).stat().st_size for p in dump_files)
    data = json.loads(result_file.read_text())
    samples = [(s, a) for s, a in data['samples']]
    unanswered = sum(1 for _, a in samples if a is None)
    stretches = gaps(samples)
    report = dict(
        pid=pid, pid_alive_after=alive, exe_before=exe_before, exe_after=exe_after,
        restart_request=data['restart'] and data['restart'][1],
        restart_requested_at_s=round(data['restart'][0] - data['start'], 3) if data['restart'] else None,
        probes=len(samples), unanswered=unanswered,
        longest_gap_s=round(max([g['seconds'] for g in stretches if g['seconds'] is not None], default=0.0), 3),
        gaps=[dict(start_s=round(g['start'] - data['start'], 3), seconds=None if g['seconds'] is None else round(g['seconds'], 3)) for g in stretches if g['seconds'] is None or g['seconds'] > 0.05],
        api_down_s=round(data['health_back_at'] - data['health_failed_at'], 3) if data['health_back_at'] and data['health_failed_at'] else None,
        dump_files=len(dump_files), dump_bytes=dump_bytes)
    (args.output / 'report.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=('run', 'probe'))
    for flag in ('work', 'binary', 'output', 'corpus'):
        parser.add_argument('--' + flag, type=lambda s: pathlib.Path(s).resolve())
    parser.add_argument('--seed', type=int, default=501)
    parser.add_argument('--api-port', type=int, default=9099)
    arguments = parser.parse_args()
    run(arguments) if arguments.action == 'run' else probe(arguments)
