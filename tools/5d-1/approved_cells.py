"""Prospectively pin the 462 cells accepted in the 5D-1 external review.

Only prior evidence and immutable rules are inputs; candidate responses never
participate in constructing the oracle. No field or class of replies is waived.
"""
import argparse
import copy
import hashlib
import json
import pathlib

from load import semantic


def build(corpus, oracle, probes, indexes, nov4, nov6):
    if len(indexes) != 462 or len(set(indexes)) != 462:
        raise ValueError('exact accepted 462-cell set required')
    rules = {}
    for qtype, text in ((1, nov4), (28, nov6)):
        rules[qtype] = {line.strip()[5:].rstrip('.').lower()
                        for line in text.splitlines()
                        if line.strip().startswith('full:')
                        and line.strip()[5:].isascii()
                        and line.strip()[5:] != line.strip()[5:].lower()}
    prior = {}
    for row in probes:
        if row['index'] in indexes:
            prior.setdefault(row['index'], []).append(row)
    result = copy.deepcopy(oracle)
    for index in indexes:
        cell = corpus[index]
        name, qtype = cell['name'].rstrip('.').lower(), cell['type']
        if name not in rules.get(qtype, set()):
            raise ValueError('cell is not an accepted uppercase full: rule')
        rows = prior.get(index, [])
        if sorted(row['pass'] for row in rows) != [0, 1, 2]:
            raise ValueError('three distinct prior probes required')
        if oracle[index]['index'] != index:
            raise ValueError('oracle index mismatch')
        for row in rows:
            dns = row['dns']
            if (row['input'] != cell or row['semantic_matches']
                    or not row['ttl_bounded']
                    or row['expected'] != oracle[index]['semantic']
                    or dns['rcode'] != 0
                    or dns['questions'] != [[name + '.', qtype, 1]]
                    or any(dns[section] for section in ('answers', 'authority', 'additional'))):
                raise ValueError('prior accepted evidence mismatch')
        result[index]['dns'] = rows[0]['dns']
        result[index]['semantic'] = semantic(rows[0]['dns'])
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    for key in ('corpus', 'oracle', 'probes', 'diagnosis', 'nov4', 'nov6',
                'native-oracle', 'output'):
        parser.add_argument('--' + key, type=pathlib.Path, required=True)
    args = parser.parse_args()
    indexes = json.loads(args.diagnosis.read_text())['native']['indexes']
    values = build(json.loads(args.corpus.read_text()), json.loads(args.oracle.read_text()),
                   json.loads(args.probes.read_text()), indexes,
                   args.nov4.read_text(), args.nov6.read_text())
    args.native_oracle.write_text(json.dumps(values))
    hashes = {key: hashlib.sha256(getattr(args, key).read_bytes()).hexdigest()
              for key in ('corpus', 'oracle', 'probes', 'diagnosis', 'nov4', 'nov6', 'native_oracle')}
    args.output.write_text(json.dumps(dict(
        approval='5D-1 Claude round 1 accepted by user; fix-A approved 2026-10-09',
        contract='docs/rust/contracts/native-rule-providers.md:57',
        candidate_measurements_used=False, indexes=indexes, hashes=hashes), indent=2) + '\n')
