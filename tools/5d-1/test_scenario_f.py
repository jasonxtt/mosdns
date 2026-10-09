import json
import pathlib
import tempfile
import unittest

import yaml
from scenario_f import prepare


class ForeignScenarioTests(unittest.TestCase):
    def test_both_roots_preserve_cache_and_use_unique_foreign_doh(self):
        for role in ('go', 'native'):
            with tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                (root/'webinfo').mkdir()
                steps = [{'exec':'$cache_all'}, {'exec':'$top_domains'}, {'exec':'$ordinary'}]
                (root/'config_custom.yaml').write_text(yaml.safe_dump({'plugins':[{'tag':'sequence_6666','args':steps}]}))
                path = root/('webinfo/upstream_overrides.json' if role == 'go' else 'webinfo/settings.json')
                path.write_text(json.dumps({'foreign':[], 'domestic':[1]} if role == 'go' else {'upstreams':{'foreign':[], 'domestic':[1]},'proxies':{'main_proxy':{'addr':'127.0.0.1:17891'}}}))
                corpus, oracle = prepare(root, role, 100)
                rows = json.loads(corpus.read_text())
                self.assertEqual(len({r['name'] for r in rows}),100)
                self.assertEqual(sum(r['type']==1 for r in rows),70)
                self.assertEqual(len(json.loads(oracle.read_text())),100)
                revised = yaml.safe_load((root/'config_custom.yaml').read_text())['plugins'][0]['args']
                self.assertEqual(revised[:2], steps[:2])
                self.assertEqual(revised[-1],steps[-1])
                groups = json.loads(path.read_text())
                if role=='native': groups=groups['upstreams']
                self.assertEqual(groups['domestic'],[1])
                if role=='go': self.assertEqual(groups['foreign'][0]['socks5'],'127.0.0.1:17891')
                else:
                    self.assertTrue(groups['foreign'][0]['use_proxy'])
                    self.assertNotIn('socks5',groups['foreign'][0])
                self.assertEqual(groups['foreign'][0]['protocol'],'https')
