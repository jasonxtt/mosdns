import pathlib
import sys
import unittest

sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
from summarize import cache, primary_cache


class SummaryTests(unittest.TestCase):
    def test_nested_lookups_do_not_inflate_primary_cache_denominator(self):
        text='\n'.join(['mosdns_cache_query_total{tag="cache_all"} 100',
                        'mosdns_cache_hit_total{tag="cache_all"} 60',
                        'mosdns_cache_query_total{tag="cache_cn"} 40',
                        'mosdns_cache_hit_total{tag="cache_cn"} 5'])
        self.assertEqual(cache(text)['mosdns_cache_query_total'],140)
        self.assertEqual(primary_cache(text)['mosdns_cache_query_total'],100)
        self.assertEqual(primary_cache(text)['mosdns_cache_hit_total'],60)


if __name__=='__main__': unittest.main()
