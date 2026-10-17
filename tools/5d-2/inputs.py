"""Build the native oracle for a prepared input set (5D-2), the same way fix-B did.

1. `python3 tools/5d-1/run.py prepare ...`           (prepared/{go,native}, corpus)
2. `python3 tools/5d-1/run.py run --mode oracle ...`  (prepared/oracle.json from Go)
3. `python3 tools/5d-2/inputs.py native-oracle ...`   (this file)

The native host answers every corpus cell once; fix_b_inputs.py accepts only the
already approved ASCII case-insensitive `full:` no-v4/no-v6 differences from the
Go oracle and fails on any other difference. No candidate answer is used to
build expectations beyond those classified cells.
"""
import argparse
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from harness import Env  # noqa: E402


def native_oracle(work, binary, output):
    rows = json.loads((work / 'weighted-corpus.json').read_text())
    with Env(work, 'native', binary, output) as env:
        indexes = output / 'indexes.json'
        indexes.write_text(json.dumps(list(range(len(rows)))))
        subprocess.run(['ip', 'netns', 'exec', env.ns, 'taskset', '-c', '1', sys.executable,
                        str(HERE.parent / '5d-1' / 'diagnostic.py'), '--probe-only', '--output', str(output),
                        '--work', str(work), '--role', 'go', '--indexes', str(indexes), '--probe-passes', '1'],
                       check=True)
    probes = output / 'dns-probes-private.json'
    subprocess.run([sys.executable, str(HERE.parent / '5d-1' / 'fix_b_inputs.py'),
                    '--corpus', str(work / 'weighted-corpus.json'), '--oracle', str(work / 'oracle.json'),
                    '--probes', str(probes), '--nov4', str(work / 'native/gen/nov4rule.txt'),
                    '--nov6', str(work / 'native/gen/nov6rule.txt'),
                    '--native-oracle', str(work / 'native-oracle.json'),
                    '--receipt', str(work / 'native-oracle-receipt.json')], check=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=('native-oracle',))
    parser.add_argument('--work', type=pathlib.Path, required=True)
    parser.add_argument('--binary', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    native_oracle(args.work.resolve(), args.binary.resolve(), args.output.resolve())
