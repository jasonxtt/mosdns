"""Numerical reporting must not silently count failed/moved process samples."""
import unittest
import json
import pathlib
import struct
import tempfile

from report import cpu_intervals, distribution, summarize, trend


class ReportTests(unittest.TestCase):
    def test_cpu_uses_process_identity_and_monotonic_time(self):
        samples = [{"pid": 7, "start_ticks": 20, "monotonic": 100, "cpu_seconds": 2},
                   {"pid": 7, "start_ticks": 20, "monotonic": 160, "cpu_seconds": 2.6},
                   {"pid": 7, "start_ticks": 80, "monotonic": 220, "cpu_seconds": .1}]
        result = cpu_intervals(samples)
        self.assertAlmostEqual(result["percent"]["median"], 1)
        self.assertEqual(result["identity_changes"], 1)

    def test_missing_samples_are_not_zero_cpu(self):
        result = cpu_intervals([{"sample_error": "ssh"}, {"sample_error": "ssh"}])
        self.assertEqual(result["percent"]["samples"], 0)
        self.assertEqual(result["sample_gaps"], 1)

    def test_tail_and_growth(self):
        self.assertEqual(distribution(range(1, 101))["p99"], 99)
        self.assertAlmostEqual(trend([{"monotonic": 1000, "rss_kib": 1024},
                                     {"monotonic": 4600, "rss_kib": 2048}]), 1)

    def test_tc_directions_and_no_edns_lengths_are_reported_separately(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            resources = [{"phase": phase, "monotonic": value, "wall": value,
                          "guard": [0, 0, 0], "resources": {}}
                         for phase, value in [("idle_collection_baseline_end", 1), ("capture_finished", 3601)]]
            (root / "resources.jsonl").write_text("".join(json.dumps(row) + "\n" for row in resources))
            (root / "summary.json").write_text(json.dumps({"counts": {"replayed": 3}}))
            (root / "reference.jsonl").write_text("")
            query = struct.pack("!6H", 7, 0x100, 1, 0, 0, 0) + b"\x07example\x03com\0\0\x01\0\x01"
            pairs = [{"go": {"tc": left}, "rust": {"tc": right}, "query_wire": query.hex(),
                      "go_wire": bytes(left_size).hex(), "rust_wire": bytes(right_size).hex(),
                      "classification": "unexplained", "different_fields": ["tc"] if left != right else []}
                     for left, right, left_size, right_size in [(True, False, 500, 512),
                                                               (False, True, 400, 500), (True, True, 503, 503)]]
            (root / "paired.jsonl").write_text("".join(json.dumps(row) + "\n" for row in pairs))
            result = summarize(root, root / "reference.jsonl")
            counts = result["tc_counts"]
            self.assertEqual(counts["go_only_tc"], 1)
            self.assertEqual(counts["rust_only_tc"], 1)
            self.assertEqual(counts["no_edns_tc_length_equal"], 1)
            self.assertEqual(counts["no_edns_tc_length_different"], 2)
            self.assertEqual(counts["no_edns_over_512_without_tc"], 0)
            self.assertTrue(result["detail_complete"])


if __name__ == "__main__":
    unittest.main()
