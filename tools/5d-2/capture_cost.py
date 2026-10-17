"""Cost of a running log capture (F-2) at 500 qps, with the unchanged 5D-1 sender.

One native host runs the 100 qps / 10 s warm-up and a 30 s 500 qps stage; with `--capture on`
a 600 s capture is started through the API first, so every audited query writes its line to the
capture buffer while it is measured. `--capture off` is the control. The buffer is read at the end
(its size is bounded by the host), and the stage's latency/CPU figures are written to result.json.

    python3 tools/5d-2/capture_cost.py run --work prepared --binary mosdns-native --capture on \
        --output cc-on-1 --seed 501
    python3 tools/5d-2/capture_cost.py summary cc-on-1 cc-off-1 ...
"""
import argparse
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / '5d-1'))
sys.path.insert(0, str(HERE.parent / 'e2e-1'))


def stage_figures(summary, name='q500'):
    """Pure: latency and CPU of one stage of a `load.measure` summary."""
    stage = next(s for s in summary['stages'] if s['stage'] == name)
    latency = stage['stable']['planned_latency_ms']
    return dict(p50=latency['p50'], p95=latency['p95'], p99=latency['p99'],
                cpu_us_per_correct=stage['resources']['cpu_us_per_correct'],
                correct_on_time=stage['stable']['correct_on_time'], statuses=stage['all']['statuses'])


def send(args):
    """Runs inside the namespace: warm-up plus one 500 qps stage."""
    import load
    namespace = argparse.Namespace(output=args.output / 'load', corpus=args.corpus, oracle=args.oracle, pid=args.pid,
                                   mock_pid=args.mock_pid, seed=args.seed, warmup=10, rates=[500], duration=30,
                                   recovery=1, deadline=.5, api_port=args.api_port, unique=False)
    load.measure(namespace)


def run(args):
    from harness import Env
    with Env(args.work, 'native', args.binary, args.output, seed=args.seed, saved_port_applies=False) as env:
        if args.capture == 'on':
            status, text, _ = env.api('POST', '/api/v1/capture/start', {'duration_seconds': 600})
            assert status == 200, (status, text)
        argv = [sys.executable, HERE / 'capture_cost.py', 'send', '--output', args.output,
                '--corpus', args.work / 'weighted-corpus.json', '--oracle', args.work / 'native-oracle.json',
                '--pid', env.host.pid, '--mock-pid', env.mock.pid, '--api-port', env.api_port, '--seed', args.seed]
        sender = env.spawn(1, argv, 'sender.log')
        if sender.wait() != 0:
            raise RuntimeError('sender failed; inspect sender.log')
        captured = None
        if args.capture == 'on':
            status, text, _ = env.api('GET', '/api/v1/capture/logs')
            lines = json.loads(text)
            notice = [line for line in lines if line.get('logger_name') == 'capture']
            captured = dict(status=status, bytes=len(text), lines=len(lines),
                            dropped_notice=notice[0]['msg'] if notice else None)
    summary = json.loads((args.output / 'load/summary.json').read_text())
    result = dict(capture=args.capture, seed=args.seed, captured=captured, **stage_figures(summary))
    (args.output / 'result.json').write_text(json.dumps(result, indent=2))
    print(json.dumps(result))


def summary(args):
    rows = [json.loads((d / 'result.json').read_text()) for d in args.directories]
    for mode in ('off', 'on'):
        for r in (r for r in rows if r['capture'] == mode):
            print(mode, r['seed'], 'p95', r['p95'], 'p99', r['p99'], 'cpu_us', round(r['cpu_us_per_correct'], 1),
                  'correct', r['correct_on_time'], 'captured', r['captured'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=('run', 'send', 'summary'))
    parser.add_argument('directories', nargs='*', type=pathlib.Path)
    for flag in ('work', 'binary', 'output', 'corpus', 'oracle'):
        parser.add_argument('--' + flag, type=lambda s: pathlib.Path(s).resolve())
    parser.add_argument('--capture', choices=('on', 'off'), default='off')
    parser.add_argument('--seed', type=int, default=501)
    parser.add_argument('--pid', type=int)
    parser.add_argument('--mock-pid', type=int)
    parser.add_argument('--api-port', type=int, default=9099)
    arguments = parser.parse_args()
    {'run': run, 'send': send, 'summary': summary}[arguments.action](arguments)
