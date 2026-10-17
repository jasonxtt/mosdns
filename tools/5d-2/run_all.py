"""Runs a plan of 5D-2 scenarios one after another; every attempt is kept.

    python3 tools/5d-2/run_all.py --work prepared --root runs \
        --go mosdns-go --native mosdns-native --plan F1,F2a:native=3,go=1

A plan item is `NAME` (defaults: native x3 and, for F scenarios, go x1) or
`NAME:native=N,go=M`. No retries, no replacement: a failed run is recorded and the
next one starts. Receipts go to <root>/plan.json (updated after every run).
"""
import argparse
import json
import pathlib
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from scenarios import build_scenarios  # noqa: E402


def expand(plan):
    scenarios = build_scenarios()
    items = []
    for text in plan.split(';'):
        text = text.strip()
        if not text:
            continue
        name, _, counts = text.partition(':')
        name, _, rate = name.partition('@')  # NAME@qps overrides the load of an H scenario
        kind = scenarios[name]['kind']
        wanted = dict(native=3, go=1 if kind == 'F' else 0)
        for field in filter(None, counts.split(',')):
            role, _, number = field.partition('=')
            wanted[role] = int(number)
        for role in ('native', 'go'):
            for repeat in range(1, wanted[role] + 1):
                items.append(dict(name=name, role=role, repeat=repeat, seed=500 + repeat, rate=int(rate) if rate else None))
    return items


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--work', type=pathlib.Path, required=True)
    parser.add_argument('--root', type=pathlib.Path, required=True)
    parser.add_argument('--go', type=pathlib.Path, required=True)
    parser.add_argument('--native', type=pathlib.Path, required=True)
    parser.add_argument('--plan', required=True, help='NAME[:native=N,go=M];NAME...')
    args = parser.parse_args()
    args.root.mkdir(parents=True, exist_ok=True)
    plan = expand(args.plan)
    receipts = []
    manifest = args.root / 'plan.json'
    for item in plan:
        label = f"{item['name']}{'@' + str(item['rate']) if item['rate'] else ''}-{item['role']}-r{item['repeat']}"
        output = args.root / label
        began = time.time()
        with (args.root / (label + '.log')).open('x') as log:
            result = subprocess.run([sys.executable, str(HERE / 'scenarios.py'), item['name'], '--work', str(args.work),
                                     '--role', item['role'], '--binary', str(args.go if item['role'] == 'go' else args.native),
                                     '--output', str(output), '--seed', str(item['seed'])]
                                    + (['--rate', str(item['rate'])] if item['rate'] else []),
                                    stdout=log, stderr=subprocess.STDOUT)
        cleanup = output / 'cleanup.json'
        receipts.append(dict(item, label=label, returncode=result.returncode, seconds=round(time.time() - began, 1),
                             cleanup=json.loads(cleanup.read_text()) if cleanup.exists() else None,
                             passed=(json.loads((output / 'result.json').read_text())['pass']
                                     if (output / 'result.json').exists() else None)))
        manifest.write_text(json.dumps(receipts, indent=2))
        print(json.dumps(receipts[-1]), flush=True)


if __name__ == '__main__':
    main()
