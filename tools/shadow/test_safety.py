import unittest

from safety import Safety, SafetyError


class SafetyTests(unittest.TestCase):
    def test_health_reset_grace_and_restart(self):
        props = {"MainPID": "7", "NRestarts": "0", "ActiveState": "active"}
        guard = Safety(0)
        guard.health(0, props, True)
        guard.health(10, props, False)
        guard.health(39, props, False)
        guard.health(39.5, props, True)
        guard.health(40, props, False)
        with self.assertRaisesRegex(SafetyError, "health"):
            guard.health(70, props, False)
        for field, value in [("MainPID", "8"), ("NRestarts", "1"), ("ActiveState", "failed")]:
            with self.subTest(field=field), self.assertRaises(SafetyError):
                guard.health(71, dict(props, **{field: value}), True)

    def test_cpu_is_a_five_minute_average_and_rss_guard_has_no_grace(self):
        guard = Safety(0)
        guard.resources(0, 0, 96 * 1024, [0, 0, 0])
        guard.resources(299, 29.9, 96 * 1024, [0, 0, 0])
        guard.resources(300, 30, 96 * 1024, [0, 0, 0])
        with self.assertRaisesRegex(SafetyError, "CPU"):
            guard.resources(301, 30.2, 96 * 1024, [0, 0, 0])
        with self.assertRaisesRegex(SafetyError, "RSS"):
            Safety(0).resources(0, 0, 96 * 1024 + 1, [0, 0, 0])
        with self.assertRaisesRegex(SafetyError, "leak"):
            Safety(0).resources(0, 0, 0, [0, 1, 0])

    def test_servfail_rolling_percent_and_cumulative_limit(self):
        guard = Safety(0)
        for _ in range(100):
            guard.pair(1, 0, 0)
        guard.pair(299, 0, 2)
        guard.pair(300, 0, 0)  # 1/102 <= 1%
        with self.assertRaisesRegex(SafetyError, "rolling"):
            guard.pair(300.5, 0, 2)
        guard = Safety(0)
        for _ in range(10000):
            guard.pair(0, 0, 0)
        for i in range(20):
            guard.pair(i, 0, 2)
        with self.assertRaisesRegex(SafetyError, "total"):
            guard.pair(20, 0, 2)

    def test_first_partial_window_also_enforces_ratio(self):
        with self.assertRaisesRegex(SafetyError, "rolling"):
            Safety(0).pair(1, 0, 2)

    def test_old_failures_expire_and_other_semantic_differences_do_not_stop(self):
        guard = Safety(0)
        for _ in range(99):
            guard.pair(0, 0, 0)
        guard.pair(1, 0, 2)
        snapshot = guard.pair(302, 0, 0)
        self.assertEqual(snapshot["rolling_paired"], 1)
        self.assertEqual(snapshot["rolling_bad"], 0)
        guard.pair(303, 3, 2)  # Go did not answer NOERROR.


if __name__ == "__main__":
    unittest.main()
