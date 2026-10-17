"""L: cold-start overload `late_wrong` diagnosis (5D-1 extension, 32000/64000 qps).

Runs the unchanged 5D-1 sender (`tools/5d-1/load.py: measure`) against a cold host
exactly like the extension matrix, but records every answer classified wrong or
late_wrong WITH its content and timestamps, then classifies each one:

  servfail        well-formed SERVFAIL for the right question (an overload failure
                  of the product or its upstream, not an incorrect DNS result)
  other_rcode     any other non-zero rcode for the right question
  ttl             right question and answer, only the TTL outside 0..300
  answer_differs  right question, rcode 0, different answer records
  question_differs the answer's question is not the request's (cross-talk)

Usage (VM root):
    python3 tools/5d-2/l_diag.py run  --work prepared --binary mosdns-native --role native --output L-native-r1 --seed 501
    python3 tools/5d-2/l_diag.py send ...   (internal: executed inside the namespace)
"""
import argparse
import collections
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / '5d-1'))
sys.path.insert(0, str(HERE.parent / 'e2e-1'))

LIMIT = 200000


def categorize(value, expected_text):
    """Pure: why `value` differs from the oracle `expected_text` (a semantic() string)."""
    expected = json.loads(expected_text)
    same_question = [list(q) for q in value['questions']] == [list(q) for q in expected[1]]
    if not same_question:
        return 'question_differs'
    if value['rcode'] == 2 and not value['answers']:
        return 'servfail'
    if value['rcode'] != expected[0]:
        return 'other_rcode'
    ttl_ok = all(0 <= rr[4] <= 300 for section in ('answers', 'authority', 'additional') for rr in value[section])
    body = [sorted(list(r[:4]) for r in value[s]) for s in ('answers', 'authority', 'additional')]
    if body == expected[2:] and not ttl_ok:
        return 'ttl'
    return 'answer_differs'


def send(args):
    import load
    original = load.classify
    kept, counts = [], collections.Counter()

    def classify(value, expected, finished, planned, deadline):
        status = original(value, expected, finished, planned, deadline)
        if status in ('wrong', 'late_wrong'):
            counts[status] += 1
            if len(kept) < LIMIT:
                kept.append(dict(status=status, finished=finished, planned=planned, category=categorize(value, expected),
                                 rcode=value['rcode'], id=value['id'], questions=value['questions'],
                                 answers=value['answers'][:4], expected=json.loads(expected)))
        return status

    load.classify = classify
    ns = argparse.Namespace(output=args.output / 'load', corpus=args.corpus, oracle=args.oracle, pid=args.pid,
                            mock_pid=args.mock_pid, seed=args.seed, warmup=10, rates=[32000, 64000], duration=30,
                            recovery=60, deadline=.5, api_port=args.api_port, unique=False)
    load.measure(ns)
    summary = json.loads((ns.output / 'summary.json').read_text())
    start = summary['started']
    with (args.output / 'wrong-responses.jsonl').open('w') as handle:
        for item in kept:
            item['finished_s'] = round(item.pop('finished') - start, 4)
            item['planned_s'] = round(item.pop('planned') - start, 4)
            handle.write(json.dumps(item, ensure_ascii=False) + '\n')
    (args.output / 'wrong-counts.json').write_text(json.dumps(dict(counts, kept=len(kept))))


def run(args):
    from fload import read_udp_counters
    from harness import Env
    with Env(args.work, args.role, args.binary, args.output, seed=args.seed) as env:
        oracle = args.work / ('native-oracle.json' if args.role == 'native' else 'oracle.json')
        argv = [sys.executable, HERE / 'l_diag.py', 'send', '--output', args.output, '--corpus', args.work / 'weighted-corpus.json',
                '--oracle', oracle, '--pid', env.host.pid, '--mock-pid', env.mock.pid, '--api-port', env.api_port,
                '--seed', args.seed]
        sender = env.spawn(1, argv, 'sender.log')
        if sender.wait() != 0:
            raise RuntimeError('sender failed; inspect sender.log')
        # The namespace-wide counter also counts datagrams dropped by sockets that no longer exist
        # (the host binds one short-lived socket per upstream call).
        (args.output / 'udp-sockets.json').write_text(json.dumps(dict(
            live_sockets_with_drops=udp_sockets(env.host.pid), namespace=read_udp_counters(env.host.pid))))
    return summarize(args.output)


def udp_sockets(pid):
    """Every UDP socket of the namespace with its local port, queued bytes and kernel drop count
    (`/proc/<pid>/net/udp*`): which side lost datagrams during the run."""
    sockets = []
    for name in ('udp', 'udp6'):
        lines = pathlib.Path(f'/proc/{pid}/net/{name}').read_text().splitlines()[1:]
        for line in lines:
            fields = line.split()
            sockets.append(dict(port=int(fields[1].rsplit(':', 1)[1], 16), rx_queue=int(fields[4].split(':')[1], 16),
                                drops=int(fields[-1])))
    return [s for s in sockets if s['drops']]


def summarize(output):
    """Per-category counts, time distribution and the stages the packets belong to."""
    summary = json.loads((output / 'load/summary.json').read_text())
    rows = [json.loads(line) for line in (output / 'wrong-responses.jsonl').read_text().splitlines()]
    categories = collections.Counter(r['category'] for r in rows)
    status = collections.Counter(r['status'] for r in rows)
    stages, offset = [], 0
    for stage in summary['stages']:
        stages.append((stage['stage'], offset, offset + stage['duration']))
        offset += stage['duration']
    by_stage = collections.defaultdict(collections.Counter)
    for r in rows:
        for name, lo, hi in stages:
            if lo <= r['planned_s'] < hi:
                by_stage[name][r['category']] += 1
    delay = sorted(r['finished_s'] - r['planned_s'] for r in rows)
    result = dict(total_wrong=len(rows), status=dict(status), categories=dict(categories),
                  by_stage={k: dict(v) for k, v in by_stage.items()},
                  planned_to_finished_s=dict(min=delay[0] if delay else None, median=delay[len(delay) // 2] if delay else None,
                                             max=delay[-1] if delay else None),
                  first_planned_s=min((r['planned_s'] for r in rows), default=None),
                  last_planned_s=max((r['planned_s'] for r in rows), default=None),
                  stages={s['stage']: dict(planned=s['all']['planned'], statuses=s['all']['statuses']) for s in summary['stages']},
                  orphan_or_late_packets=summary['orphan_or_late_packets'])
    (output / 'diagnosis.json').write_text(json.dumps(result, indent=2))
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=('run', 'send'))
    for flag in ('work', 'binary', 'output', 'corpus', 'oracle'):
        parser.add_argument('--' + flag, type=lambda s: pathlib.Path(s).resolve())
    parser.add_argument('--role', choices=('go', 'native'), default='native')
    parser.add_argument('--seed', type=int, default=501)
    parser.add_argument('--pid', type=int)
    parser.add_argument('--mock-pid', type=int)
    parser.add_argument('--api-port', type=int, default=9099)
    args = parser.parse_args()
    if args.action == 'send':
        send(args)
    else:
        print(json.dumps(run(args), indent=2))
