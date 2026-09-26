#!/usr/bin/env python3
"""Fail-closed distributed W1 qualification; never a candidate acceptance."""
import argparse
import csv
import hashlib
import importlib.util
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROFILE = dict(GOMAXPROCS='1', GOGC='off', GODEBUG='gctrace=1', GOMEMLIMIT='')


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def check_coverage(stage):
    if stage['harness_go_profile'] != PROFILE:
        raise ValueError('actual client Go profile differs')
    metadata = stage['server_resources']
    if not 25 <= metadata['bracket_seconds'] <= 35.5 or any(n < 25 for n in stage['resource_sample_counts'].values()):
        raise ValueError('whole-window resource coverage missing')


def check_attempt(root):
    # Rebuild the merge from original files instead of trusting its valid bit.
    with tempfile.TemporaryDirectory() as temp:
        regenerated = Path(temp)
        for stage in ('normal-reference', 'overload'):
            subprocess.run([sys.executable, str(HERE / 'm5-remote-tools.py'), 'merge-stage',
                            '--client', str(root / 'client' / stage), '--server', str(root / 'server' / ('server-' + stage)),
                            '--result', str(regenerated), '--client-host', 'Debian'], check=True, capture_output=True)
        for file in ('stages.jsonl', 'resource-samples.jsonl'):
            if (root / file).read_bytes() != (regenerated / file).read_bytes():
                raise ValueError('merged evidence differs from original reconstruction')
    stages = [json.loads(line) for line in (root / 'stages.jsonl').read_text().splitlines()]
    if len(stages) != 2 or [s['stage'] for s in stages] != ['normal-reference', 'overload']:
        raise ValueError('unexpected stage matrix')
    for stage in stages:
        check_coverage(stage)
        if stage['server_resources']['host'] != 'mosdns-rust':
            raise ValueError('wrong server namespace')
    if stages[0]['server_resources']['processes'] != stages[1]['server_resources']['processes']:
        raise ValueError('server process identity changed between windows')
    if json.loads((root / 'server/fixture-profile.json').read_text()) != PROFILE:
        raise ValueError('actual fixture Go profile differs')
    logs = [root / 'server/fixture.stderr'] + [root / ('client/' + stage + '.stderr') for stage in ('normal-reference', 'overload')]
    if any(re.search(r'^gc \d+ @', path.read_text(), re.M) for path in logs):
        raise ValueError('Go GC trace observed')
    samples = [json.loads(line) for line in (root / 'resource-samples.jsonl').read_text().splitlines()]
    for role in ('load-generator', 'fixture-1'):
        peak = max(s['rss_kib'] for s in samples if s['role'] == role)
        if not 0 < peak <= 262144:
            raise ValueError('sampled Go role RSS cap exceeded')
    exits = re.findall(r'^exit=(-?\d+)$', (root / 'oracle-checks.txt').read_text(), re.M)
    if exits != ['0'] * 7:
        raise ValueError('correctness/sender/sample/session oracles did not all pass')


def qualify(root):
    gate = load('control_gate', 'qualify-m2-controls.py')
    driver = load('distributed_driver', 'run-m5-w1.py')
    failures = []
    identity = json.loads((root / 'input-identity.json').read_text())
    if identity['helper_sha256'] != driver.HELPER_SHA or identity['baseline_sha256'] != driver.BASELINE_SHA:
        failures.append('fixed executable identity differs')
    for file, expected in identity['local_tools'].items():
        if hashlib.sha256((HERE / file).read_bytes()).hexdigest() != expected:
            failures.append('analysis tool identity differs: ' + file)
    for batch, plan in driver.make_plan(root).items():
        rows = list(csv.DictReader((root / batch / 'attempt-order.tsv').open(), delimiter='\t'))
        if len(rows) != 9 or any(any(str(actual[k]) != str(expected[k]) for k in driver.FIELDS[:-1]) or actual['runner_exit'] != '0' for actual, expected in zip(rows, plan)):
            failures.append(batch + ': fixed order/exits differ')
        for expected in plan:
            try:
                attempt = Path(expected['result_dir'])
                if json.loads((attempt / 'input-identity.json').read_text()) != identity:
                    raise ValueError('attempt input identities differ from frozen root')
                check_attempt(attempt)
            except (OSError, ValueError, KeyError, IndexError, subprocess.SubprocessError) as error:
                failures.append(expected['run_id'] + ': ' + str(error))
    result = gate.qualify(root, groups=gate.GROUPS[:2], profile='m5')
    result['failures'] += failures
    result['qualified'] = not result['failures']
    result['scope'] = 'M5 W1 controls only; W2/W3 and candidate acceptance remain closed'
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=Path)
    args = parser.parse_args()
    try:
        result = qualify(args.root.resolve())
    except (OSError, ValueError, KeyError, IndexError, subprocess.SubprocessError) as error:
        result = dict(qualified=False, failures=[str(error)], intervals=[])
    print(json.dumps(result, indent=2, allow_nan=False))
    sys.exit(0 if result['qualified'] else 2)
