import unittest

from compare import differences, ttl_result, upstream_group
from dnswire import answer, decode, question


class CodecTests(unittest.TestCase):
    def test_compressed_answer_and_aaaa(self):
        for qt, address in ((1, "192.0.2.1"), (28, "2001:db8::1")):
            query = question("Probe.TEST", qt, 37)
            result = decode(answer(query, "192.0.2.1", "2001:db8::1"))
            self.assertEqual(result["id"], 37)
            self.assertEqual(result["answers"], [("probe.test.", qt, 1, address, 300)])

    def test_missing_audit_cannot_be_agreement(self):
        row = {"dns": {"rcode": 0, "answers": []}, "input": {"port": 53}, "audit": None}
        self.assertIn("audit_missing", differences(row, row))

    def row(self, ttl, kind="top", address="223.5.5.101", port=53):
        return {"dns": {"rcode": 0, "answers": [["x.", 1, 1, address, ttl]]}, "input": {"port": port, "class": kind}, "audit": {}, "started_s": 10, "finished_s": 10.01}

    def test_mock_ttl_range_allows_clock_rounding_but_caps_at_300(self):
        a, b = self.row(300), self.row(299)
        self.assertEqual(differences(a, b), [])
        self.assertIn("ttl_policy", differences(a, self.row(301)))

    def test_rewrite_requires_five_even_when_both_hosts_are_wrong(self):
        for kind in ("rewrite-ip", "rewrite-name"):
            self.assertTrue(ttl_result(self.row(5, kind, "192.0.2.77"))["valid"])
            bad = self.row(300, kind, "192.0.2.77")
            self.assertIn("ttl_policy", differences(bad, bad))

    def test_warm_cache_must_age_but_a_fresh_upstream_may_reset(self):
        cold = self.row(300)
        warm = {**self.row(296), "started_s": 14, "finished_s": 14.01, "fresh_upstream": False}
        self.assertTrue(ttl_result(warm, cold)["valid"])
        warm["dns"] = self.row(300)["dns"]
        self.assertFalse(ttl_result(warm, cold)["valid"])
        warm["fresh_upstream"] = True
        self.assertTrue(ttl_result(warm, cold)["valid"])

    def test_cnfake_wrapper_caps_at_five_and_unknown_local_fails(self):
        self.assertFalse(ttl_result(self.row(300, port=3099))["valid"])
        self.assertTrue(ttl_result(self.row(5, address="30.0.0.10", port=3099))["valid"])
        self.assertFalse(ttl_result(self.row(10, address="192.0.2.90"))["valid"])

    def test_authority_ttl_is_checked(self):
        row = self.row(300)
        row["dns"]["authority"] = [["x.", 6, 1, "soa", 86400]]
        self.assertFalse(ttl_result(row)["valid"])

    def test_upstream_group_uses_mapping_raw_labels_are_diagnostic(self):
        aliases = {"foreign": ["foreign"], "google": ["foreign"], "google-foreignecs": ["foreignecs"]}
        a, b = self.row(300), self.row(299)
        a["audit"] = {"final_upstream": "foreign"}
        b["audit"] = {"final_upstream": "google"}
        for row in (a, b):
            row["audit"]["final_upstream_group"] = upstream_group(row["audit"], aliases)
        self.assertEqual(differences(a, b), [])
        b["audit"]["final_upstream_group"] = upstream_group({"final_upstream": "google-foreignecs"}, aliases)
        self.assertIn("final_upstream_group", differences(a, b))
        self.assertEqual(upstream_group({"final_upstream": "unknown"}, aliases), "unmapped_or_ambiguous")


if __name__ == "__main__":
    unittest.main()
