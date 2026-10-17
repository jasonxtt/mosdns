"""CPU per correct answer of the cold 100 qps / 10 s warm-up, measured as the 5D-1 budget does.

The 5D-1 budget compares each native repeat's `cpu_us_per_correct` (the CPU the host
used between the stage's samples after the first settling seconds, divided by the
stable window's correct on-time answers) with the Go median stage by stage; the
extension matrix's warm-up (the first 1000 queries of a cold host) was once above
1.5 x Go. This runs the unchanged 5D-1 sender (`load.measure`) for the warm-up and
one short follow-up stage, many times, in the same isolated environment, and
optionally records a `perf` (software cpu-clock) profile of the native host.

    python3 tools/5d-2/warmup_profile.py run --work prepared --binary mosdns-native --role native \
        --output wp-native-1 --seed 501 [--perf]
    python3 tools/5d-2/warmup_profile.py summary wp-native-1 wp-native-2 wp-go-1 ...
"""
import argparse
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / '5d-1'))
sys.path.insert(0, str(HERE.parent / 'e2e-1'))


def warmup_cpu(summary):
    """Pure: the frozen-budget CPU figure of the `warmup` stage of a `load.measure` summary."""
    stage = next(s for s in summary['stages'] if s['stage'] == 'warmup')
    return dict(cpu_s=stage['resources']['cpu_s'], cpu_us_per_correct=stage['resources']['cpu_us_per_correct'],
                correct_on_time=stage['stable']['correct_on_time'], statuses=stage['all']['statuses'])


def send(args):
    """Runs inside the namespace: the 5D-1 sender, warm-up stage plus a short 20 qps stage."""
    import load
    namespace = argparse.Namespace(output=args.output / 'load', corpus=args.corpus, oracle=args.oracle, pid=args.pid,
                                   mock_pid=args.mock_pid, seed=args.seed, warmup=10, rates=[20], duration=3,
                                   recovery=1, deadline=.5, api_port=args.api_port, unique=False)
    load.measure(namespace)


def run(args):
    from harness import Env
    with Env(args.work, args.role, args.binary, args.output, seed=args.seed) as env:
        oracle = args.work / ('native-oracle.json' if args.role == 'native' else 'oracle.json')
        argv = [sys.executable, HERE / 'warmup_profile.py', 'send', '--output', args.output,
                '--corpus', args.work / 'weighted-corpus.json', '--oracle', oracle, '--pid', env.host.pid,
                '--mock-pid', env.mock.pid, '--api-port', env.api_port, '--seed', args.seed]
        perf = None
        if args.perf:
            perf = subprocess.Popen(['taskset', '-c', '1', 'perf', 'record', '-e', 'cpu-clock', '-F', '999',
                                     '--call-graph', 'dwarf,16384', '-p', str(env.host.pid), '-o', str(args.output / 'perf.data')],
                                    stdout=(args.output / 'perf.log').open('w'), stderr=subprocess.STDOUT,
                                    start_new_session=True)
        sender = env.spawn(1, argv, 'sender.log')
        if sender.wait() != 0:
            raise RuntimeError('sender failed; inspect sender.log')
        if perf:
            perf.send_signal(2)
            perf.wait(timeout=60)
    summary = json.loads((args.output / 'load/summary.json').read_text())
    result = dict(role=args.role, seed=args.seed, **warmup_cpu(summary))
    (args.output / 'result.json').write_text(json.dumps(result, indent=2))
    if args.perf:
        with (args.output / 'perf-report.txt').open('w') as report:
            subprocess.run(['perf', 'report', '--stdio', '--no-children', '--percent-limit', '0.7', '-g', 'none',
                            '-i', str(args.output / 'perf.data')], stdout=report, check=True)
    print(json.dumps(result))


def summary(args):
    rows = [json.loads((d / 'result.json').read_text()) for d in args.directories]
    for role in ('go', 'native'):
        values = sorted(r['cpu_us_per_correct'] for r in rows if r['role'] == role)
        if values:
            print(role, len(values), 'runs, us/correct:', values, 'median', values[len(values) // 2])


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=('run', 'send', 'summary'))
    parser.add_argument('directories', nargs='*', type=pathlib.Path)
    for flag in ('work', 'binary', 'output', 'corpus', 'oracle'):
        parser.add_argument('--' + flag, type=lambda s: pathlib.Path(s).resolve())
    parser.add_argument('--role', choices=('go', 'native'), default='native')
    parser.add_argument('--seed', type=int, default=501)
    parser.add_argument('--pid', type=int)
    parser.add_argument('--mock-pid', type=int)
    parser.add_argument('--api-port', type=int, default=9099)
    parser.add_argument('--perf', action='store_true')
    arguments = parser.parse_args()
    {'run': run, 'send': send, 'summary': summary}[arguments.action](arguments)
