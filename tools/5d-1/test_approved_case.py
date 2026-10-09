import pathlib
import sys
import unittest

sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
from approved_case import build


class ApprovedCaseTests(unittest.TestCase):
    def test_exact_contract_cell_and_no_general_case_waiver(self):
        name='frozen.example.com'
        corpus=[dict(name=name,type=1),dict(name=name,type=28)]
        evidence=dict(contract='docs/rust/contracts/native-rule-providers.md:57',approved_date='2026-10-07',prior_index=3787,
                      input=dict(name=name,type=28),dns=dict(rcode=0,questions=[[name+'.',28,1]],answers=[],authority=[],additional=[]))
        result=build(corpus,evidence,'full:Frozen.example.com\n')
        self.assertEqual([row['index'] for row in result['rows']],[1])
        with self.assertRaises(ValueError): build(corpus,evidence,'full:frozen.example.com\n')
        with self.assertRaises(ValueError): build(corpus,dict(evidence,prior_index=3788),'full:Frozen.example.com\n')


if __name__=='__main__': unittest.main()
