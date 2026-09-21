import unittest

from common.automation_run import AutomationRun, AutomationRunError
from common.automation_review import (
    ReviewerTransportUnavailable,
    build_review_request,
    parse_review_result,
    record_remediation,
    record_review_result,
    read_review_round_trip,
    submit_review,
    verify_reviewer_transport,
)


class FakeReviewerTransport:
    def __init__(self):
        self.sent = []

    def verify_target(self, target):
        return {"mechanism": "fake-platform-native", "probe_id": "probe-1"}

    def send(self, request):
        self.sent.append(request)

    def wait_result(self, timeout):
        return "ready"

    def read(self):
        return "FINAL: PASS"


class ReviewerContractTest(unittest.TestCase):
    def _run(self):
        return AutomationRun(
            context_key="codex_test",
            task=".trellis/tasks/example",
            authorized_units=["Slice 2"],
            authorized_at="2026-09-22T00:00:00Z",
            current_unit="Slice 2",
            units={
                "Slice 2": {
                    "phase": "implementing",
                    "submission": {
                        "parent_sha": None,
                        "head_sha": None,
                        "review_round": 0,
                        "request_kind": None,
                        "submitted_to": None,
                    },
                    "findings": {},
                    "result": None,
                }
            },
        )

    def _evidence(self):
        return {
            "repository": "jasonxtt/mosdns",
            "branch": "rust",
            "task_goal": "simplify automation",
            "scope_contracts": "Trellis only",
            "reviewer": {"provider": "chatgpt", "reference": "conversation-1"},
            "base_sha": "a" * 40,
            "head_sha": "b" * 40,
            "github_reference": "https://github.com/jasonxtt/mosdns/commit/" + "b" * 40,
            "changed_paths": [".trellis/scripts/common/automation_review.py"],
            "validation": ["30 tests passed"],
            "acceptance": "review contract is durable",
            "forbidden_scope": ["MosDNS runtime", "production wiring"],
        }

    def _finding(self, finding_id, root_cause, root_cause_key, status="open", out_of_scope=False):
        return {
            "id": finding_id,
            "root_cause": root_cause,
            "root_cause_key": root_cause_key,
            "status": status,
            "out_of_scope": out_of_scope,
        }

    def test_fake_transport_verifies_and_round_trips_without_repo_network_code(self):
        target = {"provider": "chatgpt", "reference": "conversation-1"}
        transport = FakeReviewerTransport()

        evidence = verify_reviewer_transport(target, transport)
        self.assertTrue(evidence["verified"])
        self.assertEqual(evidence["target"], target)
        self.assertEqual(evidence["mechanism"], "fake-platform-native")

        result = read_review_round_trip(transport, {"text": "review"}, timeout=3)
        self.assertEqual(result, "FINAL: PASS")
        self.assertEqual(transport.sent, [{"text": "review"}])

    def test_transport_without_platform_probe_is_unavailable(self):
        target = {"provider": "chatgpt", "reference": "conversation-1"}
        with self.assertRaises(ReviewerTransportUnavailable):
            verify_reviewer_transport(target, object())

    def test_review_parser_distinguishes_pending_pass_and_fail(self):
        self.assertEqual(parse_review_result("still working")["status"], "pending")
        self.assertEqual(parse_review_result("FINAL: PASS")["status"], "pass")
        result = parse_review_result("FINAL: FAIL\nP1-1 — activation order is unsafe")
        self.assertEqual(result["status"], "fail")
        self.assertEqual(result["findings"][0]["id"], "P1-1")
        self.assertIn("activation order", result["findings"][0]["root_cause"])

    def test_bootstrap_is_full_and_rereview_is_compact(self):
        run = self._run()
        target = {"provider": "chatgpt", "reference": "conversation-1"}
        request = build_review_request(run, "Slice 2", self._evidence())
        self.assertEqual(request["kind"], "bootstrap")
        for required in ("Task goal", "User-authorized unit range", "Base full SHA", "Forbidden scope", "FINAL: PASS"):
            self.assertIn(required, request["text"])

        submit_review(
            run,
            "Slice 2",
            parent_sha="a" * 40,
            head_sha="b" * 40,
            submitted_to=target,
        )
        record_review_result(
            run,
            "Slice 2",
            {
                "status": "fail",
                "findings": [self._finding("P1-1", "reviewer bootstrap is incomplete", "bootstrap")],
            },
        )
        record_remediation(
            run,
            "Slice 2",
            finding_id="P1-1",
            root_cause_key="bootstrap",
            parent_sha="b" * 40,
            head_sha="c" * 40,
            summary="complete the bootstrap contract",
        )
        submit_review(
            run,
            "Slice 2",
            parent_sha="b" * 40,
            head_sha="c" * 40,
            submitted_to=target,
        )
        compact = build_review_request(run, "Slice 2", self._evidence())
        self.assertEqual(compact["kind"], "rereview")
        self.assertIn("Previous finding ledger", compact["text"])
        self.assertIn("b" * 40, compact["text"])
        self.assertNotIn("Task goal: simplify automation", compact["text"])

    def test_same_root_cause_reaches_blocked_at_exactly_five_remediation_failures(self):
        run = self._run()
        target = {"provider": "chatgpt", "reference": "conversation-1"}
        submit_review(run, "Slice 2", parent_sha="a" * 40, head_sha="b" * 40, submitted_to=target)
        record_review_result(
            run,
            "Slice 2",
            {"status": "fail", "findings": [self._finding("P1-1", "unsafe review gate", "unsafe-gate")]},
        )
        self.assertEqual(run.units["Slice 2"]["findings"]["P1-1"]["failed_remediation_rounds"], 0)

        with self.assertRaises(AutomationRunError):
            submit_review(
                run,
                "Slice 2",
                parent_sha="b" * 40,
                head_sha="c" * 40,
                submitted_to=target,
                request_kind="rereview",
            )

        for round_number in range(1, 6):
            parent = chr(ord("b") + round_number - 1) * 40
            head = chr(ord("c") + round_number - 1) * 40
            record_remediation(
                run,
                "Slice 2",
                finding_id="P1-1" if round_number == 1 else "P2-9",
                root_cause_key="unsafe-gate",
                parent_sha=parent,
                head_sha=head,
                summary=f"remediation round {round_number}",
            )
            submit_review(
                run,
                "Slice 2",
                parent_sha=parent,
                head_sha=head,
                submitted_to=target,
                request_kind="rereview",
            )
            record_review_result(
                run,
                "Slice 2",
                {
                    "status": "fail",
                    "findings": [
                        self._finding(
                            "P2-9",
                            f"rephrased unsafe review gate round {round_number}",
                            "unsafe-gate",
                        )
                    ],
                },
            )
            if round_number < 5:
                self.assertEqual(run.status, "running")
                self.assertEqual(
                    run.units["Slice 2"]["findings"]["P1-1"]["failed_remediation_rounds"],
                    round_number,
                )
        self.assertEqual(run.status, "blocked")
        self.assertEqual(run.units["Slice 2"]["findings"]["P1-1"]["failed_remediation_rounds"], 5)
        self.assertIn("P2-9", run.units["Slice 2"]["findings"]["P1-1"]["aliases"])

    def test_new_finding_is_independent_and_closed_finding_stops_counting(self):
        run = self._run()
        target = {"provider": "chatgpt", "reference": "conversation-1"}
        submit_review(run, "Slice 2", parent_sha="a" * 40, head_sha="b" * 40, submitted_to=target)
        record_review_result(
            run,
            "Slice 2",
            {"status": "fail", "findings": [self._finding("P1-1", "first root", "root-one")]},
        )
        record_remediation(
            run,
            "Slice 2",
            finding_id="P1-1",
            root_cause_key="root-one",
            parent_sha="b" * 40,
            head_sha="c" * 40,
        )
        submit_review(run, "Slice 2", parent_sha="b" * 40, head_sha="c" * 40, submitted_to=target, request_kind="rereview")
        record_review_result(
            run,
            "Slice 2",
            {"status": "fail", "findings": [self._finding("P2-1", "second root", "root-two")]},
        )
        self.assertEqual(run.units["Slice 2"]["findings"]["P1-1"]["failed_remediation_rounds"], 0)
        self.assertEqual(run.units["Slice 2"]["findings"]["P2-1"]["failed_remediation_rounds"], 0)

        record_remediation(
            run,
            "Slice 2",
            finding_id="P1-1",
            root_cause_key="root-one",
            parent_sha="c" * 40,
            head_sha="d" * 40,
        )
        record_remediation(
            run,
            "Slice 2",
            finding_id="P2-1",
            root_cause_key="root-two",
            parent_sha="c" * 40,
            head_sha="d" * 40,
        )
        submit_review(run, "Slice 2", parent_sha="c" * 40, head_sha="d" * 40, submitted_to=target, request_kind="rereview")
        record_review_result(
            run,
            "Slice 2",
            {
                "status": "pass",
                "findings": [
                    self._finding("P1-1", "first root closed", "root-one", status="closed"),
                    self._finding("P2-1", "second root closed", "root-two", status="closed"),
                ],
            },
        )
        self.assertEqual(run.units["Slice 2"]["findings"]["P1-1"]["status"], "closed")
        self.assertEqual(run.status, "authorized_scope_complete")

    def test_same_id_with_different_semantic_root_gets_a_distinct_ledger_entry(self):
        run = self._run()
        target = {"provider": "chatgpt", "reference": "conversation-1"}
        submit_review(run, "Slice 2", parent_sha="a" * 40, head_sha="b" * 40, submitted_to=target)
        record_review_result(
            run,
            "Slice 2",
            {"status": "fail", "findings": [self._finding("P1-1", "root A", "root-a")]},
        )
        record_remediation(
            run,
            "Slice 2",
            finding_id="P1-1",
            root_cause_key="root-a",
            parent_sha="b" * 40,
            head_sha="c" * 40,
        )
        submit_review(run, "Slice 2", parent_sha="b" * 40, head_sha="c" * 40, submitted_to=target, request_kind="rereview")
        record_review_result(
            run,
            "Slice 2",
            {"status": "fail", "findings": [self._finding("P1-1", "root B", "root-b")]},
        )
        self.assertEqual(run.units["Slice 2"]["findings"]["P1-1"]["failed_remediation_rounds"], 0)
        self.assertEqual(run.units["Slice 2"]["findings"]["P1-1#2"]["failed_remediation_rounds"], 0)

    def test_pass_with_an_open_finding_blocks_instead_of_advancing(self):
        run = self._run()
        target = {"provider": "chatgpt", "reference": "conversation-1"}
        submit_review(run, "Slice 2", parent_sha="a" * 40, head_sha="b" * 40, submitted_to=target)
        record_review_result(
            run,
            "Slice 2",
            {"status": "pass", "findings": [self._finding("P1-1", "still open", "still-open")]},
        )
        self.assertEqual(run.status, "blocked")
        self.assertEqual(run.units["Slice 2"]["phase"], "remediating")
        self.assertNotEqual(run.status, "authorized_scope_complete")

    def test_out_of_scope_finding_blocks_immediately_and_pending_is_not_a_result(self):
        run = self._run()
        target = {"provider": "chatgpt", "reference": "conversation-1"}
        submit_review(run, "Slice 2", parent_sha="a" * 40, head_sha="b" * 40, submitted_to=target)
        pending = parse_review_result("reviewer is still thinking")
        record_review_result(run, "Slice 2", pending)
        self.assertEqual(run.units["Slice 2"]["phase"], "awaiting_review")
        record_review_result(
            run,
            "Slice 2",
            parse_review_result("FINAL: FAIL\nP0-1 — change production wiring [out-of-scope]"),
        )
        self.assertEqual(run.status, "blocked")


if __name__ == "__main__":
    unittest.main()
