"""Pin replacement-input native expectations from BASE, never the candidate.

Only the already approved ASCII case-insensitive full: no-v4/no-v6 contract
may differ from the fresh Go oracle. Keep every other DNS section strict.
All inputs/outputs are private except the numeric/hash receipt.
"""
import argparse
import copy
import hashlib
import json
import pathlib

from load import semantic


def build(corpus, oracle, probes, nov4, nov6):
    rules = {}
    for qtype, text in ((1, nov4), (28, nov6)):
        rules[qtype] = {line.strip()[5:].rstrip('.').lower()
                        for line in text.splitlines()
                        if line.strip().startswith('full:')
                        and line.strip()[5:].isascii()
                        and line.strip()[5:] != line.strip()[5:].lower()}
    if len(probes) != len(corpus) or sorted(r['index'] for r in probes) != list(range(len(corpus))):
        raise ValueError('one complete BASE probe per corpus cell required')
    result, changed = copy.deepcopy(oracle), []
    for row in probes:
        index = row['index']
        cell, expected, dns = corpus[index], oracle[index], row['dns']
        if (row['input'] != cell or expected['index'] != index
                or dns['id'] != index or not row['ttl_bounded']):
            raise ValueError('BASE input/TTL mismatch')
        if semantic(dns) == expected['semantic']:
            continue
        name, qtype = cell['name'].rstrip('.').lower(), cell['type']
        if (name not in rules.get(qtype, set()) or dns['rcode'] != 0
                or dns['questions'] != [[name + '.', qtype, 1]]
                or any(dns[section] for section in ('answers', 'authority', 'additional'))):
            raise ValueError('unclassified BASE/Go difference at index ' + str(index))
        result[index]['dns'], result[index]['semantic'] = dns, semantic(dns)
        changed.append(index)
    return result, sorted(changed)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    for name in ('corpus', 'oracle', 'probes', 'nov4', 'nov6', 'native_oracle', 'receipt'):
        parser.add_argument('--' + name.replace('_', '-'), type=pathlib.Path, required=True)
    args = parser.parse_args()
    result, changed = build(json.loads(args.corpus.read_text()), json.loads(args.oracle.read_text()),
                            json.loads(args.probes.read_text()), args.nov4.read_text(), args.nov6.read_text())
    args.native_oracle.write_text(json.dumps(result))
    hashes = {name: hashlib.sha256(getattr(args, name).read_bytes()).hexdigest()
              for name in ('corpus', 'oracle', 'probes', 'nov4', 'nov6', 'native_oracle')}
    args.receipt.write_text(json.dumps(dict(
        replacement_inputs_authorized='2026-10-16',
        reference_source='353371fa9206d01c01eeb067bf8f7b64dd05f412',
        candidate_responses_used=False, changed_indexes=changed, hashes=hashes,
        contract='native-rule-providers.md: ASCII case-insensitive full: no-v4/no-v6'), indent=2) + '\n')
