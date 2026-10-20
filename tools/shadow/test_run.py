import collections
import struct
import threading
import unittest
from unittest import mock

from protocol import decode_dns
from run import Experiment, project_audit, project_trace
import run
from safety import Safety


class RunTests(unittest.TestCase):
    def test_local_api_reuses_one_opener_per_thread_instead_of_accumulating_handler_cycles(self):
        with mock.patch.object(run, 'API_LOCAL', threading.local(), create=True), \
             mock.patch.object(run.urllib.request, 'build_opener') as build:
            first=run.api_opener()
            self.assertIs(first,run.api_opener())
            self.assertEqual(build.call_count,1)
            thread=threading.Thread(target=run.api_opener)
            thread.start();thread.join()
            self.assertEqual(build.call_count,2)

    def test_poll_queue_keeps_correlation_and_proof_without_unused_api_payload(self):
        raw = {"trace_id": "q", "client_ip": "192.0.2.10", "query_name": "example.com",
               "query_type": "AAAA", "query_time": "2026-10-19T00:00:00Z",
               "upstream_diagnostics": {"attempts": []}, "unused_answers": "x" * 100000}
        audit = project_audit({"logs": [raw], "unused_summary": "y" * 100000})[0]
        self.assertEqual(audit["client_ip"], raw["client_ip"])
        self.assertEqual(audit["upstream_diagnostics"], raw["upstream_diagnostics"])
        self.assertNotIn("unused_answers", audit)
        trace = project_trace([{"trace_id": "q", "logger_name": "entry", "match_result": False,
                                "matcher_name": "anonymous_match(fast_mark: 3)", "domain": "example.com",
                                "msg": "dns query flows through matcher", "unused": "z" * 100000}])[0]
        self.assertIs(trace["match_result"], False)
        self.assertEqual(trace["matcher_name"], "anonymous_match(fast_mark: 3)")
        self.assertNotIn("unused", trace)
        self.assertNotIn("domain", trace)

    def test_unexplained_row_is_kept_without_stopping_replay(self):
        question = b"\x07example\x03com\0\0\x01\0\x01"
        query = struct.pack("!6H", 1, 0x100, 1, 0, 0, 0) + question
        row = {"query_wire": query.hex(), "audit": {"trace_id": "q"}}
        for side, flags in [("go", 0x8183), ("rust", 0x8180)]:
            wire = struct.pack("!6H", 1, flags, 1, 0, 0, 0) + question
            row[side] = decode_dns(wire)
            row[side + "_wire"] = wire.hex()
        experiment = Experiment.__new__(Experiment)
        experiment.pending = {"key": row}
        experiment.config = {"fake_ranges": []}
        experiment.policy = {"rules": []}
        original_trace = {"policy_valid": True, "entries": [{"trace_id": "q", "msg": "dns query completed"}]}
        experiment.traces = {"q": original_trace}
        experiment.counts = collections.Counter()
        experiment.stop = threading.Event()
        experiment.safety = Safety(0)
        records = []
        experiment.record = records.append
        experiment.finish("key")
        self.assertFalse(experiment.stop.is_set())
        self.assertEqual(experiment.counts["unexplained"], 1)
        self.assertEqual(records[0]["classification"], "unexplained")
        self.assertEqual(records[0]["trace"], original_trace)
        self.assertNotIn("q", experiment.traces, "a finalized query must release its in-flight trace")


if __name__ == "__main__":
    unittest.main()
