"""Controlled all-miss foreign path in disposable roots only."""
import json
import pathlib
import sys

import yaml

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / 'e2e-1'))
from dnswire import question, answer, decode
from mock import MARKERS, ports
from load import semantic


def prepare(root, role, count):
    suffix = '5d1-f.example.com'
    path = root / 'config_custom.yaml'
    config = yaml.safe_load(path.read_text())
    entry = next(p for p in config['plugins'] if p['tag'] == 'sequence_6666')
    # Keep mapping, policy, outer cache and statistics; replace only routing for
    # this reserved suffix after the outer cache, before ordinary A/AAAA routing.
    position = next(i for i, step in enumerate(entry['args']) if step.get('exec') == '$top_domains') + 1
    entry['args'].insert(position, dict(matches='qname domain:' + suffix, exec=['$foreign', 'exit']))
    path.write_text(yaml.safe_dump(config, allow_unicode=True, sort_keys=False))
    upstream = dict(tag='5d1-f', enabled=True, protocol='https',
                    addr=f'https://localhost:{ports("foreign")["https"]}/dns-query',
                    dial_addr=f'127.0.0.1:{ports("foreign")["https"]}')
    if role == 'go': upstream['socks5']='127.0.0.1:17891'
    else: upstream['use_proxy']=True
    settings = root / ('webinfo/upstream_overrides.json' if role == 'go' else 'webinfo/settings.json')
    values = json.loads(settings.read_text())
    groups = values if role == 'go' else values['upstreams']
    if role == 'native' and values.get('proxies',{}).get('main_proxy',{}).get('addr')!='127.0.0.1:17891':
        raise ValueError('scenario F requires the mapped main_proxy')
    groups['foreign'] = [upstream]
    settings.write_text(json.dumps(values))
    rows = [dict(name=f'u{i:06d}.{suffix}', type=1 if i % 10 < 7 else 28,
                 weight=1, kind='unique-foreign') for i in range(count)]
    expected = [dict(index=i, semantic=semantic(decode(answer(question(r['name'], r['type'], 0), *MARKERS['foreign']))))
                for i, r in enumerate(rows)]
    corpus_path, oracle_path = root / 'f-corpus.json', root / 'f-oracle.json'
    corpus_path.write_text(json.dumps(rows)); oracle_path.write_text(json.dumps(expected))
    return corpus_path, oracle_path
