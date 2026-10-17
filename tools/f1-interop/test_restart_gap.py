import unittest

import restart_gap


class Gaps(unittest.TestCase):
    def test_a_gap_runs_from_the_first_lost_probe_to_the_next_answered_one(self):
        samples = [(0.00, 0.001), (0.01, None), (0.02, None), (0.03, None), (0.04, 0.041), (0.05, 0.051)]
        self.assertEqual(restart_gap.gaps(samples), [dict(start=0.01, end=0.04, seconds=0.03)])

    def test_separate_gaps_and_an_unfinished_one_are_reported(self):
        samples = [(0.0, None), (0.1, 0.101), (0.2, None), (0.3, None)]
        found = restart_gap.gaps(samples)
        self.assertEqual([g['seconds'] for g in found], [0.1, None])
        self.assertEqual(restart_gap.gaps([(0.0, 0.0), (0.1, 0.1)]), [])


if __name__ == '__main__':
    unittest.main()
