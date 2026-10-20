import pathlib
import tempfile
import unittest

from state_counts import Counts


class CountTests(unittest.TestCase):
    def test_decrease_and_missing_file_stop_without_logging_names(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            stat = root / "stats.txt"
            stat.write_text("1 2026-10-19 a.example.com\n2 2026-10-19 b.example.com\n")
            monitor = Counts({"learn": str(stat)}, root / "counts.jsonl")
            self.assertEqual(monitor.sample(1)["counts"], {"learn": 2})
            stat.write_text("1 2026-10-19 a.example.com\n")
            with self.assertRaisesRegex(RuntimeError, "decrease"):
                monitor.sample(601)
            stat.unlink()
            with self.assertRaisesRegex(RuntimeError, "decrease"):
                monitor.sample(1201)
            self.assertNotIn("example.com", (root / "counts.jsonl").read_text())


if __name__ == "__main__":
    unittest.main()
