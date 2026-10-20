"""Explained differences require evidence from this query, not a cold-state guess."""
import unittest
import struct

from learned import explain
from protocol import decode_dns


class LearnedTests(unittest.TestCase):
    def test_query_udp_limit_uses_opt_with_512_floor(self):
        question = b"\x07example\x03com\0\0\x1c\0\x01"
        for size in [None, 128, 512, 1232]:
            query = struct.pack("!6H", 1, 0x100, 1, 0, 0, int(size is not None)) + question
            if size is not None:
                query += b"\0" + struct.pack("!HHIH", 41, size, 0, 0)
            self.assertEqual(decode_dns(query)["udp_limit"], max(size or 0, 512))

    def test_no_v6_miss_explains_answer_but_not_tc_or_rcode(self):
        rule = {"sequence": "entry", "mapper": "mapper", "mark": 3, "qtype": 28,
                "switch": "switch1", "output": "learn", "label": "learned-v6"}
        def event(**values):
            return dict(trace_id="query-1", logger_name="entry", **values)
        trace = {"policy_valid": True, "entries": [
            event(plugin_name="mapper", msg="dns query flows through plugin"),
            event(matcher_name="anonymous_match(fast_mark: 3)", match_result=False,
                  msg="dns query flows through matcher"),
            dict(trace_id="query-1", logger_name="query", msg="dns query completed")]}
        audit = {"trace_id": "query-1"}
        left = {"rcode": 0, "has_answer": False, "tc": False}
        right = dict(left, has_answer=True)
        self.assertEqual(explain(left, right, 28, audit, trace, [rule])["output"], "learn")
        for bad in [dict(right, rcode=2), dict(right, tc=True), dict(right, has_answer=False)]:
            self.assertIsNone(explain(left, bad, 28, audit, trace, [rule]))
        self.assertIsNone(explain(left, right, 1, audit, trace, [rule]))
        self.assertIsNone(explain(left, right, 28, {"trace_id": "other"}, trace, [rule]))
        self.assertIsNone(explain(left, right, 28, audit, dict(trace, policy_valid=False), [rule]))
        self.assertIsNone(explain(left, right, 28, audit, dict(trace, entries=trace["entries"][:-1]), [rule]))

    def test_tc_on_pass_is_allowed_only_within_client_limit(self):
        rule = {"sequence": "entry", "mapper": "mapper", "mark": 3, "qtype": 28,
                "switch": "switch1", "output": "learn", "label": "learned-v6"}
        trace = {"policy_valid": True, "entries": [
            {"trace_id": "q", "logger_name": "entry", "plugin_name": "mapper"},
            {"trace_id": "q", "logger_name": "entry", "matcher_name": "anonymous_match(fast_mark: 3)", "match_result": False},
            {"trace_id": "q", "logger_name": "query", "msg": "dns query completed"}]}
        left = {"rcode": 0, "tc": False, "has_answer": False}
        right = dict(left, tc=True, has_answer=True)
        audit = {"trace_id": "q"}
        self.assertIsNotNone(explain(left, right, 28, audit, trace, [rule], sizes=(38, 508), limit=512))
        self.assertIsNone(explain(left, right, 28, audit, trace, [rule], sizes=(38, 513), limit=512))
        self.assertIsNotNone(explain(left, right, 28, audit, trace, [rule], sizes=(38, 1232), limit=1232))
        self.assertIsNone(explain(dict(left, tc=True), dict(right, tc=False), 28, audit, trace, [rule], sizes=(38, 508), limit=512))
        self.assertIsNone(explain(left, right, 28, audit, trace, [rule]))

    def test_hit_requires_executed_reject_and_exact_learned_label(self):
        rule = {"sequence": "entry", "mapper": "mapper", "mark": 2, "qtype": 1,
                "switch": "switch1", "output": "learn-v4", "label": "learned-v4"}
        def event(**values):
            return dict(trace_id="q", logger_name="entry", **values)
        trace = {"policy_valid": True, "entries": [
            event(plugin_name="mapper"),
            event(matcher_name="anonymous_match(fast_mark: 2)", match_result=True),
            event(matcher_name="anonymous_match(qtype: 1)", match_result=True),
            event(matcher_name="anonymous_match(switch1: 'A')", match_result=True),
            event(plugin_name="anonymous_exec(reject: 0)"),
            dict(trace_id="q", msg="dns query completed", logger_name="query")]}
        audit = {"trace_id": "q", "domain_set": "learned-v4"}
        left = {"rcode": 0, "tc": False, "has_answer": True}
        right = dict(left, has_answer=False)
        self.assertTrue(explain(left, right, 1, audit, trace, [rule])["matched"])
        self.assertIsNotNone(explain(dict(left, tc=True), right, 1, audit, trace, [rule], sizes=(508, 38), limit=512))
        self.assertIsNone(explain(dict(left, tc=True), right, 1, audit, trace, [rule], sizes=(513, 38), limit=512))
        self.assertIsNone(explain(left, right, 1, dict(audit, domain_set="user-rule"), trace, [rule]))
        bad = dict(trace, entries=trace["entries"][:4] + trace["entries"][5:])
        self.assertIsNone(explain(left, right, 1, audit, bad, [rule]))
        duplicate = dict(trace, entries=trace["entries"][:2] + trace["entries"][1:])
        self.assertIsNone(explain(left, right, 1, audit, duplicate, [rule]))


if __name__ == "__main__":
    unittest.main()
