"""C: frozen-threshold re-measurement (capacity, overload recovery, RSS) with the
unchanged 5D-1 sender: serial Go/native pairs, seeds 501/502/503, order G/N, N/G, G/N,
main ladder (20..16000) then the extension (32000, 64000) per repeat.

The frozen record is byte-checked (SHA256 afe441e9...). Tools and inputs are hashed
before the first run and re-checked before every run; no retry, no replacement.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import sys

FROZEN = 'afe441e906ab0ca0d0ffad2c5d36c854c2d61a5725b79608e70f54d6cb8c70a9'
HERE = pathlib.Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(root, freeze):
    if digest(freeze) != FROZEN:
        raise ValueError('the original byte-identical user freeze is required')
    five_d_1 = HERE.parent / '5d-1'
    paths = [five_d_1 / n for n in ('load.py', 'run.py', 'summarize.py', 'budget.py')]
    paths += [HERE.parent / 'e2e-1' / n for n in ('prepare.py', 'mock.py', 'dnswire.py')]
    paths += [root / n for n in ('mosdns-go', 'mosdns-native')]
    paths += [p for p in (root / 'prepared').rglob('*') if p.is_file() and 'cache' not in p.parts]
    hashes = {str(p): digest(p) for p in paths}
    plan = []
    for repeat, seed in enumerate((501, 502, 503), 1):
        for matrix in ('main', 'ext'):
            for role in (('go', 'native') if repeat != 2 else ('native', 'go')):
                plan.append(dict(name=f'{role}-{matrix}-r{repeat}', role=role, matrix=matrix, seed=seed))
    manifest = root / 'c-matrix.json'
    if manifest.exists():
        raise ValueError('retain every attempt: cannot overwrite or resume a matrix')
    evidence = dict(freeze_sha256=digest(freeze), hashes=hashes, plan=plan, attempts=[])
    manifest.write_text(json.dumps(evidence, indent=2))
    for item in plan:
        if digest(freeze) != FROZEN or any(digest(p) != hashes[str(p)] for p in paths):
            raise ValueError('a pinned input changed during the matrix')
        argv = [sys.executable, str(five_d_1 / 'run.py'), 'run', '--work', str(root / 'prepared'),
                '--output', str(root / item['name']), '--binary', str(root / ('mosdns-' + item['role'])),
                '--role', item['role'], '--seed', str(item['seed']), '--freeze-record', str(freeze)]
        if item['role'] == 'native':
            argv += ['--oracle', str(root / 'prepared/native-oracle.json')]
        if item['matrix'] == 'ext':
            argv += ['--rates', '32000', '64000']
        print(json.dumps(dict(starting=item)), flush=True)
        with (root / (item['name'] + '.log')).open('x') as log:
            result = subprocess.run(argv, stdout=log, stderr=subprocess.STDOUT)
        cleanup = root / item['name'] / 'cleanup.json'
        receipt = dict(item, returncode=result.returncode, cleanup=json.loads(cleanup.read_text()) if cleanup.exists() else None)
        evidence['attempts'].append(receipt)
        manifest.write_text(json.dumps(evidence, indent=2))
        print(json.dumps(dict(completed=receipt)), flush=True)
        if result.returncode or not receipt['cleanup'] or not all(receipt['cleanup'].values()):
            raise RuntimeError('attempt failed; retained without replacement')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=pathlib.Path, required=True)
    parser.add_argument('--freeze-record', type=pathlib.Path, required=True)
    args = parser.parse_args()
    run(args.root.resolve(), args.freeze_record.resolve())
