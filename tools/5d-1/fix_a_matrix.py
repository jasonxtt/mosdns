"""Fixed fix-A rerun: F and normal 20/100/500, three interleaved pairs.

Uses the original sender, stage durations, warmup, tail and immutable freeze.
Never resumes, replaces, invalidates or retries a formal attempt.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import sys


def run(root, freeze):
    digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    if digest(freeze) != 'afe441e906ab0ca0d0ffad2c5d36c854c2d61a5725b79608e70f54d6cb8c70a9':
        raise ValueError('original byte-identical user freeze required')
    here = pathlib.Path(__file__).resolve().parent
    paths = [here/name for name in ('load.py', 'run.py', 'scenario_f.py', 'fix_a_matrix.py')]
    paths += [here.parent/'e2e-1'/name for name in ('prepare.py', 'mock.py', 'dnswire.py')]
    paths += [root/name for name in ('mosdns-go', 'mosdns-native')]
    paths += [root/'prepared-final'/name for name in ('weighted-corpus.json', 'oracle.json', 'native-oracle.json')]
    hashes = {str(path): digest(path) for path in paths}
    plan = [dict(name=f'{role}-{scenario}-r{repeat}', role=role, scenario=scenario, seed=seed)
            for repeat, seed in enumerate((501, 502, 503), 1)
            for scenario in ('f', 'normal')
            for role in (('go', 'native') if repeat != 2 else ('native', 'go'))]
    evidence = dict(freeze_sha256=digest(freeze), hashes=hashes, plan=plan, attempts=[])
    manifest = root/'fix-a-matrix.json'
    if manifest.exists():
        raise ValueError('retain existing attempts; matrix cannot overwrite or resume')
    manifest.write_text(json.dumps(evidence, indent=2))
    for item in plan:
        if digest(freeze) != evidence['freeze_sha256'] or any(digest(path) != hashes[str(path)] for path in paths):
            raise ValueError('pinned inputs changed during matrix')
        argv = [sys.executable, str(here/'run.py'), 'run', '--work', str(root/'prepared-final'),
                '--output', str(root/item['name']), '--binary', str(root/('mosdns-'+item['role'])),
                '--role', item['role'], '--seed', str(item['seed']), '--freeze-record', str(freeze)]
        if item['role'] == 'native':
            argv += ['--oracle', str(root/'prepared-final/native-oracle.json')]
        argv += ['--scenario-f', '--rates', '100'] if item['scenario'] == 'f' else ['--rates', '20', '100', '500']
        print(json.dumps(dict(starting=item)), flush=True)
        with (root/(item['name']+'.log')).open('x') as log:
            result = subprocess.run(argv, stdout=log, stderr=subprocess.STDOUT)
        receipt = dict(item, returncode=result.returncode)
        cleanup = root/item['name']/'cleanup.json'
        receipt['cleanup'] = json.loads(cleanup.read_text()) if cleanup.exists() else None
        evidence['attempts'].append(receipt)
        manifest.write_text(json.dumps(evidence, indent=2))
        print(json.dumps(dict(completed=receipt)), flush=True)
        if result.returncode or not receipt['cleanup'] or not all(receipt['cleanup'].values()):
            raise RuntimeError('attempt failed; evidence retained; no automatic replacement')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=pathlib.Path, required=True)
    parser.add_argument('--freeze-record', type=pathlib.Path, required=True)
    args = parser.parse_args()
    run(args.root, args.freeze_record)
