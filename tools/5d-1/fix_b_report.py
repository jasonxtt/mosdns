"""Export only numerical summaries of the complete immutable fix-B matrix."""
import argparse
import json
import pathlib

from budget import evaluate
from summarize import summarize


def report(root, output):
    manifest = json.loads((root / 'fix-b-matrix.json').read_text())
    if (len(manifest['attempts']) != 6
            or any(row['returncode'] or not row['cleanup'] or not all(row['cleanup'].values())
                   for row in manifest['attempts'])):
        raise ValueError('all six original attempts and cleanups required')
    output.mkdir(exist_ok=True)
    summaries = {}
    for role in ('go', 'native'):
        summaries[role] = summarize([root / f'{role}-r{repeat}' for repeat in (1, 2, 3)])
        (output / f'{role}.json').write_text(json.dumps(summaries[role], indent=2) + '\n')
    comparison = evaluate(summaries['go'], summaries['native'],
                          json.loads((root / 'frozen-thresholds.json').read_text()))
    comparison['scope'] = ('Replacement snapshot explicitly authorized 2026-10-16; '
                           'affected RSS/normal latency gates only. F, overload recovery, '
                           '5D-2/3 and production cutover are not accepted by this run.')
    (output / 'comparison.json').write_text(json.dumps(comparison, indent=2) + '\n')
    # The private manifest also hashes every prepared filename. Keep those
    # private; public output contains the fixed plan and numerical receipts.
    public = {key: manifest[key] for key in ('freeze_sha256', 'plan', 'attempts', 'inputs', 'rates')}
    (output / 'matrix.json').write_text(json.dumps(public, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    report(args.root, args.output)
