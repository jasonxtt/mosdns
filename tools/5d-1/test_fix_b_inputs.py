import unittest

from fix_b_inputs import build
from load import semantic


class ReplacementOracleTests(unittest.TestCase):
    def inputs(self):
        cell = dict(name='case.example', type=28, weight=1, kind='production')
        go = dict(id=0, rcode=0, questions=[['case.example.', 28, 1]],
                  answers=[['case.example.', 28, 1, '2001:db8::1', 60]],
                  authority=[], additional=[])
        native = dict(go, answers=[])
        return ([cell], [dict(index=0, semantic=semantic(go), dns=go)],
                [dict(index=0, input=cell, dns=native, ttl_bounded=True)])

    def test_only_approved_full_case_difference_can_be_pinned(self):
        corpus, oracle, probes = self.inputs()
        result, changed = build(corpus, oracle, probes, '', 'full:CASE.example\n')
        self.assertEqual(changed, [0])
        self.assertEqual(result[0]['dns']['answers'], [])
        self.assertTrue(oracle[0]['dns']['answers'])

    def test_unclassified_difference_is_rejected(self):
        for rule in ('', 'full:case.example', 'domain:CASE.example'):
            with self.subTest(rule=rule), self.assertRaises(ValueError):
                build(*self.inputs(), '', rule)

    def test_authority_or_wrong_question_cannot_be_waived(self):
        corpus, oracle, probes = self.inputs()
        for field, value in (('authority', [['case.example.', 2, 1, 'ns.example.', 60]]),
                             ('questions', [['other.example.', 28, 1]]), ('rcode', 3)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                build(corpus, oracle, [dict(probes[0], dns=dict(probes[0]['dns'], **{field: value}))],
                      '', 'full:CASE.example')

    def test_duplicate_or_missing_reference_cell_is_rejected(self):
        corpus, oracle, probes = self.inputs()
        for rows in ([], probes + probes):
            with self.assertRaises(ValueError):
                build(corpus, oracle, rows, '', 'full:CASE.example')


if __name__ == '__main__':
    unittest.main()
