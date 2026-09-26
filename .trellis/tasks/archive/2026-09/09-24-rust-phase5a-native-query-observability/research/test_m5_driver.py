import importlib.util
import unittest
from pathlib import Path


class FixedPlanTests(unittest.TestCase):
    def test_both_batches_have_exact_original_balanced_w1_order(self):
        spec = importlib.util.spec_from_file_location('m5_driver', Path(__file__).with_name('run-m5-w1.py'))
        driver = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(driver)
        plan = driver.make_plan(Path('/example/fresh'))
        expected = [(1, 'before_off'), (1, 'after_off'), (1, 'after_on'),
                    (2, 'after_off'), (2, 'after_on'), (2, 'before_off'),
                    (3, 'after_on'), (3, 'before_off'), (3, 'after_off')]
        self.assertEqual(list(plan), ['batch1', 'batch2'])
        for rows in plan.values():
            self.assertEqual([(r['repetition'], r['variant']) for r in rows], expected)
            self.assertTrue(all(r['scenario'] == 'w1-tcp' for r in rows))
        self.assertEqual(len({r['run_id'] for rows in plan.values() for r in rows}), 18)


if __name__ == '__main__':
    unittest.main()
