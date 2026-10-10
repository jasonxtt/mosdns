import copy
import unittest

from approved_cells import build


class ApprovedCellsTests(unittest.TestCase):
    def fixture(self):
        corpus = [dict(name=f'frozen{i}.example', type=28) for i in range(462)]
        oracle = [dict(index=i, semantic='previous') for i in range(462)]
        probes = [dict(index=i, input=cell, expected='previous',
                       semantic_matches=False, ttl_bounded=True, **{'pass': repeat},
                       dns=dict(rcode=0, questions=[[cell['name']+'.', 28, 1]],
                                answers=[], authority=[], additional=[]))
                  for repeat in range(3) for i, cell in enumerate(corpus)]
        rules = '\n'.join('full:' + cell['name'].capitalize() for cell in corpus)
        return corpus, oracle, probes, list(range(462)), '', rules

    def test_only_previously_accepted_exact_cells(self):
        args = self.fixture()
        result = build(*args)
        self.assertEqual(len(result), 462)
        self.assertNotEqual(result[0]['semantic'], 'previous')
        self.assertEqual(args[1][0]['semantic'], 'previous')
        for field, value in (('pass', 1), ('ttl_bounded', False),
                             ('semantic_matches', True), ('expected', 'other')):
            bad = copy.deepcopy(args)
            bad[2][0][field] = value
            with self.assertRaises(ValueError):
                build(*bad)
        bad = list(args)
        bad[5] = args[5].lower()
        with self.assertRaises(ValueError):
            build(*bad)
        bad = copy.deepcopy(args)
        bad[2][0]['dns']['questions'][0][1] = 1
        with self.assertRaises(ValueError):
            build(*bad)


if __name__ == '__main__':
    unittest.main()
