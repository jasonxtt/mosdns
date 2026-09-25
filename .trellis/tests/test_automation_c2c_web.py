import json
import tempfile
import unittest
from pathlib import Path

from common.automation import AutomationContext, save_context, set_reviewer
from common.automation_c2c_web import (
    C2CReviewerBindingError,
    C2CWebReviewerTransport,
    build_reviewer_only_request,
    normalize_c2c_binding,
    normalize_c2c_workspace_identity,
    parse_c2c_review_result,
    resolve_reviewer_target,
)
from common.automation_review import read_review_round_trip, verify_reviewer_transport
from common.automation_run import (
    ActivationError,
    AutomationRun,
    AutomationRunError,
    activate,
    authorize,
    authorization_path,
)


PROJECT = "https://chatgpt.com/g/g-p-reviewproject/project"
CHAT = "https://chatgpt.com/c/reviewer-chat"
CONNECTOR = "Codex with ChatGPT · mosdns-rust"


class FakeBindingSource:
    def __init__(self, payload):
        self.payload = payload
        self.calls = []

    def read(self, repo_root: Path):
        self.calls.append(repo_root)
        return self.payload


class FakeWorkspaceIdentitySource:
    def __init__(self, payload):
        self.payload = payload
        self.calls = []

    def read(self, repo_root: Path):
        self.calls.append(repo_root)
        return self.payload


class FakeC2CHost:
    def __init__(
        self,
        responses=(),
        *,
        failed_sends=0,
        verification=None,
        baseline_text="old reviewer result",
        assistant_id="new-assistant",
        baseline_assistant_id="old-assistant",
        retry_evidence=None,
    ):
        self.responses = list(responses)
        self.failed_sends = failed_sends
        self.verification = verification
        self.baseline_text = baseline_text
        self.assistant_id = assistant_id
        self.baseline_assistant_id = baseline_assistant_id
        self.retry_evidence = retry_evidence
        self.baseline_read = False
        self.verify_calls = []
        self.send_attempts = []
        self.read_calls = []
        self.retry_evidence_calls = []

    def verify_target(self, target):
        self.verify_calls.append(target)
        return self.verification if self.verification is not None else {
            "verified": True,
            "mechanism": "fake-chatgpt-platform",
        }

    def send_message(self, target, message):
        self.send_attempts.append((target, message))
        if self.failed_sends:
            self.failed_sends -= 1
            raise RuntimeError("temporary host failure")
        return {"accepted": True}

    def confirm_retry(self, target, message, timeout):
        self.retry_evidence_calls.append((target, message, timeout))
        return self.retry_evidence

    def read_thread(self, target, cursor=None):
        self.read_calls.append((target, cursor))
        if cursor is None and not self.baseline_read:
            self.baseline_read = True
            baseline_message = {"text": self.baseline_text, "status": "completed"}
            if self.baseline_assistant_id is not None:
                baseline_message["id"] = self.baseline_assistant_id
            return {
                "cursor": "baseline",
                "latestAssistantMessage": baseline_message,
            }
        if self.responses:
            payload = self.responses.pop(0)
        else:
            payload = {"cursor": cursor or "baseline", "text": "still working"}
        if "latestAssistantMessage" not in payload and "text" in payload:
            payload = dict(payload)
            payload["latestAssistantMessage"] = {
                "id": self.assistant_id,
                "text": payload.pop("text"),
                "status": payload.pop("assistant_status", "completed"),
            }
        return payload


class C2CReviewerBindingTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def _payload(self):
        return {
            "ok": True,
            "binding": {
                "projectUrl": f"{PROJECT}/?unused=1",
                "chatUrl": f"{CHAT}/?unused=1",
                "connectorName": CONNECTOR,
                "title": "Root reviewer",
                "boundAt": "2026-09-25T00:00:00Z",
            },
        }

    def _workspace_identity(self):
        return {
            "ok": True,
            "conversation": {
                "mode": "project",
                "projectUrl": PROJECT,
                "connectorName": CONNECTOR,
            },
        }

    def _task(self):
        task_dir = self.root / ".trellis/tasks/example"
        task_dir.mkdir(parents=True)
        (task_dir / "task.json").write_text(json.dumps({"status": "planning"}), encoding="utf-8")
        (task_dir / "implement.md").write_text("## Slice 0 — binding\n", encoding="utf-8")
        return task_dir

    def _evidence(self, target):
        return {
            "verified": True,
            "target": target,
            "mechanism": "fake-platform-native",
            "verified_at": "2026-09-25T00:00:00Z",
        }

    def test_normalizes_binding_to_c2c_target_without_secrets(self):
        target = normalize_c2c_binding(self._payload())
        self.assertEqual(target["provider"], "c2c-web")
        self.assertEqual(target["reference"], CHAT)
        self.assertEqual(target["metadata"]["project_url"], PROJECT)
        self.assertEqual(target["metadata"]["chat_url"], CHAT)
        self.assertEqual(target["metadata"]["connector_name"], CONNECTOR)
        self.assertNotRegex(json.dumps(target), r"token|cookie|message|diff")

    def test_rejects_missing_or_mismatched_binding_identity(self):
        with self.assertRaises(C2CReviewerBindingError):
            normalize_c2c_binding({"ok": True, "binding": None})
        with self.assertRaises(C2CReviewerBindingError):
            normalize_c2c_binding(self._payload(), expected_connector_name="another-workspace")
        with self.assertRaises(C2CReviewerBindingError):
            normalize_c2c_binding(self._payload(), expected_project_url="https://chatgpt.com/g/g-p-other/project")

    def test_normalizes_workspace_identity_without_using_the_planning_chat(self):
        self.assertEqual(
            normalize_c2c_workspace_identity(self._workspace_identity()),
            (PROJECT, CONNECTOR),
        )
        for payload in (
            {"ok": False, "conversation": self._workspace_identity()["conversation"]},
            {"ok": True, "conversation": {"mode": "long-chat", "connectorName": CONNECTOR}},
            {"ok": True, "conversation": {"mode": "project", "projectUrl": PROJECT}},
        ):
            with self.assertRaises(C2CReviewerBindingError):
                normalize_c2c_workspace_identity(payload)

    def test_explicit_and_persisted_reviewers_precede_default_source(self):
        explicit = {"provider": "codex", "reference": "01a0d43d"}
        source = FakeBindingSource(self._payload())
        context = AutomationContext("codex_test")

        target, provenance = resolve_reviewer_target(
            context,
            self.root,
            current_turn=explicit,
            binding_source=source,
        )
        self.assertEqual(target, explicit)
        self.assertEqual(provenance, "current-turn-explicit")
        self.assertEqual(source.calls, [])

        set_reviewer(context, "codex", "persisted-reviewer")
        target, provenance = resolve_reviewer_target(context, self.root, binding_source=source)
        self.assertEqual(target["reference"], "persisted-reviewer")
        self.assertEqual(provenance, "persisted-explicit")
        self.assertEqual(source.calls, [])

    def test_default_resolves_only_when_context_has_no_reviewer(self):
        source = FakeBindingSource(self._payload())
        identity = FakeWorkspaceIdentitySource(self._workspace_identity())
        context = AutomationContext("codex_test")
        target, provenance = resolve_reviewer_target(
            context,
            self.root,
            binding_source=source,
            workspace_identity_source=identity,
        )
        self.assertEqual(target["provider"], "c2c-web")
        self.assertEqual(provenance, "c2c-reviewer-binding")
        self.assertEqual(source.calls, [self.root])
        self.assertEqual(identity.calls, [self.root])

    def test_default_blocks_binding_that_does_not_match_workspace_identity(self):
        binding = self._payload()
        binding["binding"]["projectUrl"] = "https://chatgpt.com/g/g-p-other/project"
        source = FakeBindingSource(binding)
        identity = FakeWorkspaceIdentitySource(self._workspace_identity())
        with self.assertRaisesRegex(C2CReviewerBindingError, "does not match"):
            resolve_reviewer_target(
                AutomationContext("codex_test"),
                self.root,
                binding_source=source,
                workspace_identity_source=identity,
            )

    def test_authorize_persists_default_and_activation_rejects_identity_drift(self):
        task_dir = self._task()
        source = FakeBindingSource(self._payload())
        context = AutomationContext("codex_test")
        save_context(self.root, context)

        identity = FakeWorkspaceIdentitySource(self._workspace_identity())

        def resolve(context):
            return resolve_reviewer_target(
                context,
                self.root,
                binding_source=source,
                workspace_identity_source=identity,
            )[0]

        target = normalize_c2c_binding(self._payload())
        snapshot = authorize(
            self.root,
            task_dir,
            "Slice 0",
            context_key="codex_test",
            reviewer_transport_evidence=self._evidence(target),
            reviewer_resolver=resolve,
        )
        self.assertEqual(snapshot.reviewer, target)
        self.assertTrue(authorization_path(self.root, "codex_test").exists())
        stored = json.loads((self.root / ".trellis/.runtime/automation/codex_test.json").read_text())
        self.assertEqual(stored["reviewer"], target)

        (task_dir / "task.json").write_text(json.dumps({"status": "in_progress"}), encoding="utf-8")
        changed = AutomationContext("codex_test")
        set_reviewer(changed, "c2c-web", CHAT + "-changed", metadata=target["metadata"])
        save_context(self.root, changed)
        with self.assertRaisesRegex(ActivationError, "changed"):
            activate(self.root, task_dir, context_key="codex_test")

    def test_authorize_current_turn_target_overrides_persisted_and_default(self):
        task_dir = self._task()
        source = FakeBindingSource(self._payload())
        context = AutomationContext("codex_test")
        persisted = {"provider": "codex", "reference": "persisted-reviewer"}
        set_reviewer(context, persisted["provider"], persisted["reference"])
        save_context(self.root, context)
        current_turn = {"provider": "codex", "reference": "current-turn-reviewer"}

        snapshot = authorize(
            self.root,
            task_dir,
            "Slice 0",
            context_key="codex_test",
            reviewer_target=current_turn,
            reviewer_transport_evidence=self._evidence(current_turn),
            reviewer_resolver=lambda current: resolve_reviewer_target(
                current, self.root, binding_source=source
            )[0],
        )
        self.assertEqual(snapshot.reviewer, current_turn)
        self.assertEqual(source.calls, [])
        stored = json.loads((self.root / ".trellis/.runtime/automation/codex_test.json").read_text())
        self.assertEqual(stored["reviewer"], current_turn)

        changed = {"provider": "codex", "reference": "different-current-turn"}
        with self.assertRaisesRegex(ActivationError, "frozen authorization"):
            authorize(
                self.root,
                task_dir,
                "Slice 0",
                context_key="codex_test",
                reviewer_target=changed,
                reviewer_transport_evidence=self._evidence(changed),
            )

        (task_dir / "task.json").write_text(json.dumps({"status": "in_progress"}), encoding="utf-8")
        activate(self.root, task_dir, context_key="codex_test")
        with self.assertRaisesRegex(ActivationError, "frozen authorization"):
            authorize(
                self.root,
                task_dir,
                "Slice 0",
                context_key="codex_test",
                reviewer_target=changed,
                reviewer_transport_evidence=self._evidence(changed),
            )


class C2CReviewerOnlyContractTest(unittest.TestCase):
    def _target(self):
        return {
            "provider": "c2c-web",
            "reference": CHAT,
            "label": "Root reviewer",
            "metadata": {
                "project_url": PROJECT,
                "chat_url": CHAT,
                "connector_name": CONNECTOR,
                "binding_source": "c2c-reviewer",
            },
        }

    def _run(self):
        return AutomationRun(
            context_key="codex_test",
            task=".trellis/tasks/example",
            authorized_units=["Slice 1"],
            authorized_at="2026-09-25T00:00:00Z",
            current_unit="Slice 1",
            units={
                "Slice 1": {
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
            "reviewer": self._target(),
            "base_sha": "a" * 40,
            "head_sha": "b" * 40,
            "changed_paths": [".trellis/scripts/common/automation_c2c_web.py"],
            "validation": ["48 tests passed", "git diff --check"],
            "acceptance": "reviewer-only transport is bounded and fail-closed",
            "forbidden_scope": ["MosDNS runtime", "browser automation", "credential storage"],
            "body": "secret source body must never be copied",
            "log": "secret validation log must never be copied",
        }

    def test_request_is_atomic_bounded_exact_range_and_body_free(self):
        request = build_reviewer_only_request(self._run(), "Slice 1", self._evidence())
        text = request["text"]
        self.assertEqual(request["kind"], "bootstrap")
        self.assertLessEqual(len(text.encode("utf-8")), 4096)
        for required in (
            "MODE: REVIEW_ONLY",
            "STATE: REVIEW",
            "CONTROLLER: TRELLIS",
            "BASE_SHA: " + "a" * 40,
            "HEAD_SHA: " + "b" * 40,
            "PATHS:",
            "git_compare(base_sha, head_sha, path, offset, max_bytes)",
            "Do not plan, execute, edit",
            "FINAL: PASS or FINAL: FAIL",
            "P0/P1/P2/P3-N",
        ):
            self.assertIn(required, text)
        self.assertNotIn("secret source body", text)
        self.assertNotIn("secret validation log", text)

        oversized = self._evidence()
        oversized["validation"] = ["x" * 5000]
        with self.assertRaisesRegex(ValueError, "bounded message"):
            build_reviewer_only_request(self._run(), "Slice 1", oversized)
        missing_paths = self._evidence()
        missing_paths["changed_paths"] = []
        with self.assertRaisesRegex(ValueError, "changed_paths"):
            build_reviewer_only_request(self._run(), "Slice 1", missing_paths)

    def test_rereview_includes_existing_ledger_without_changing_controller(self):
        run = self._run()
        run.reviewer_bootstrap_sent = True
        run.units["Slice 1"]["findings"] = {
            "P1-1": {"root_cause": "unsafe retry", "status": "open"}
        }
        with self.assertRaisesRegex(AutomationRunError, "explicit prior result"):
            build_reviewer_only_request(run, "Slice 1", self._evidence())

        state = run.units["Slice 1"]
        state["submission"]["request_kind"] = "rereview"
        state["result"] = {"status": "fail"}
        state["review_result_recorded"] = False
        with self.assertRaisesRegex(AutomationRunError, "complete submitted"):
            build_reviewer_only_request(run, "Slice 1", self._evidence())

        state["submission"].update(
            {
                "parent_sha": "b" * 40,
                "head_sha": "c" * 40,
                "request_kind": "rereview",
                "review_round": 1,
            }
        )
        state["result"] = {"status": "fail"}
        state["review_result_recorded"] = False
        evidence = self._evidence()
        evidence["base_sha"] = "b" * 40
        evidence["head_sha"] = "c" * 40
        request = build_reviewer_only_request(run, "Slice 1", evidence)
        self.assertEqual(request["kind"], "rereview")
        self.assertIn("PREVIOUS_FINDINGS:", request["text"])
        self.assertIn("unsafe retry", request["text"])
        self.assertIn("CONTROLLER: TRELLIS", request["text"])

        blocked = self._run()
        blocked.status = "blocked"
        with self.assertRaisesRegex(AutomationRunError, "blocked run"):
            build_reviewer_only_request(blocked, "Slice 1", self._evidence())
        limited = self._run()
        limited.units["Slice 1"]["findings"] = {
            "P1-1": {"status": "open", "failed_remediation_rounds": 5}
        }
        with self.assertRaisesRegex(AutomationRunError, "blocked run"):
            build_reviewer_only_request(limited, "Slice 1", self._evidence())

    def test_parser_requires_one_final_line_and_explicit_stable_findings(self):
        self.assertEqual(parse_c2c_review_result("")["status"], "pending")
        self.assertEqual(parse_c2c_review_result("still working")["status"], "pending")
        self.assertEqual(parse_c2c_review_result("summary\nFINAL: PASS")["status"], "pass")

        failed = parse_c2c_review_result(
            "P1-1: unsafe retry ordering [open]\n"
            "P2-1 — documentation wording is stale [closed]\n"
            "FINAL: FAIL"
        )
        self.assertEqual(failed["status"], "fail")
        self.assertEqual([item["id"] for item in failed["findings"]], ["P1-1", "P2-1"])
        self.assertEqual([item["status"] for item in failed["findings"]], ["open", "closed"])

        for invalid in (
            "P1-1: missing final status [open]",
            "FINAL: FAIL",
            "P1-1: finding [open]\nFINAL: PASS",
            "P1-1: duplicate one [open]\nP1-1: duplicate two [open]\nFINAL: FAIL",
            "P1-1: final is not last [open]\nFINAL: FAIL\nsummary",
        ):
            self.assertEqual(parse_c2c_review_result(invalid)["status"], "pending", invalid)


class C2CReviewerTransportTest(unittest.TestCase):
    def _target(self):
        return {
            "provider": "c2c-web",
            "reference": CHAT,
            "metadata": {
                "project_url": PROJECT,
                "chat_url": CHAT,
                "connector_name": CONNECTOR,
                "binding_source": "c2c-reviewer",
            },
        }

    def test_verifies_target_sends_once_and_polls_partial_response(self):
        host = FakeC2CHost(
            [
                {"cursor": "one", "text": "review is still running"},
                {"cursor": "two", "text": "partial finding"},
                {"cursor": "three", "text": "summary\nFINAL: PASS"},
                {"cursor": "four", "text": "summary\nFINAL: PASS"},
            ]
        )
        now = [0.0]
        transport = C2CWebReviewerTransport(
            host,
            self._target(),
            poll_interval=0.25,
            clock=lambda: now[0],
            sleep=lambda delay: now.__setitem__(0, now[0] + delay),
        )
        evidence = verify_reviewer_transport(self._target(), transport)
        self.assertEqual(evidence["target"], self._target())
        self.assertEqual(len(host.verify_calls), 1)

        result = read_review_round_trip(transport, {"text": "[C2C] REVIEW_ONLY request"}, timeout=2)
        self.assertEqual(parse_c2c_review_result(result)["status"], "pass")
        self.assertEqual(len(host.send_attempts), 1)
        self.assertEqual(
            [cursor for _, cursor in host.read_calls],
            [None, "baseline", "one", "two", "three"],
        )
        with self.assertRaisesRegex(C2CReviewerBindingError, "already sent"):
            transport.send({"text": "[C2C] REVIEW_ONLY request"})

    def test_timeout_is_pending_and_failed_send_is_terminal(self):
        host = FakeC2CHost(failed_sends=1)
        transport = C2CWebReviewerTransport(host, self._target())
        request = {"text": "[C2C] REVIEW_ONLY request"}
        transport.verify_target(self._target())
        with self.assertRaisesRegex(RuntimeError, "temporary host failure"):
            transport.send(request)
        with self.assertRaisesRegex(C2CReviewerBindingError, "terminal"):
            transport.send({"text": "[C2C] different request"})
        with self.assertRaisesRegex(C2CReviewerBindingError, "terminal"):
            transport.send(request)
        self.assertEqual(len(host.send_attempts), 1)
        with self.assertRaisesRegex(C2CReviewerBindingError, "host retry evidence"):
            transport.retry_after_failure(request, timeout=1)

        evidence_host = FakeC2CHost(
            failed_sends=1,
            retry_evidence={
                "confirmed": True,
                "retryable": True,
                "bounded": True,
                "state": "dead",
                "target": self._target(),
                "message": request["text"],
                "observed_at": "2026-09-25T00:00:00Z",
            },
        )
        failed = C2CWebReviewerTransport(evidence_host, self._target())
        failed.verify_target(self._target())
        with self.assertRaisesRegex(RuntimeError, "temporary host failure"):
            failed.send(request)
        replacement = failed.retry_after_failure(request, timeout=1)
        self.assertIsInstance(replacement, C2CWebReviewerTransport)
        self.assertEqual(len(evidence_host.send_attempts), 2)
        self.assertEqual(len(evidence_host.retry_evidence_calls), 1)
        with self.assertRaisesRegex(C2CReviewerBindingError, "already attempted"):
            failed.retry_after_failure(request, timeout=1)
        self.assertEqual(len(evidence_host.send_attempts), 2)
        self.assertEqual(len(evidence_host.retry_evidence_calls), 1)

        clock = [0.0]
        pending_host = FakeC2CHost([{"cursor": "one", "text": "still working"}])
        pending = C2CWebReviewerTransport(
            pending_host,
            self._target(),
            poll_interval=0.5,
            clock=lambda: clock[0],
            sleep=lambda delay: clock.__setitem__(0, clock[0] + delay),
        )
        pending.verify_target(self._target())
        pending.send(request)
        self.assertFalse(pending.wait_result(1))
        self.assertEqual(parse_c2c_review_result(pending.read())["status"], "pending")

    def test_rejects_mismatched_host_identity_and_read_before_send(self):
        different_reference = "https://chatgpt.com/c/other"
        different = dict(
            self._target(),
            reference=different_reference,
            metadata=dict(self._target()["metadata"], chat_url=different_reference),
        )
        host = FakeC2CHost(verification={"verified": True, "target": different, "mechanism": "fake"})
        transport = C2CWebReviewerTransport(host, self._target())
        with self.assertRaisesRegex(C2CReviewerBindingError, "differs from the transport"):
            transport.verify_target(different)
        with self.assertRaisesRegex(C2CReviewerBindingError, "different target"):
            transport.verify_target(self._target())
        with self.assertRaisesRegex(C2CReviewerBindingError, "before the request"):
            transport.wait_result(0)

    def test_requires_affirmative_host_target_verification(self):
        for verification, message in (
            ({}, "affirmative"),
            ({"mechanism": "fake"}, "affirmative"),
            ({"verified": False}, "not verified"),
            ({"verified": "true"}, "affirmative"),
        ):
            host = FakeC2CHost(verification=verification)
            transport = C2CWebReviewerTransport(host, self._target())
            with self.assertRaisesRegex(C2CReviewerBindingError, message):
                transport.verify_target(self._target())

    def test_does_not_accept_a_single_unstable_final_response(self):
        host = FakeC2CHost(
            [
                {"cursor": "one", "text": "summary\nFINAL: PASS"},
                {"cursor": "two", "text": "P1-1: late finding\nFINAL: FAIL"},
            ]
        )
        now = [0.0]
        transport = C2CWebReviewerTransport(
            host,
            self._target(),
            poll_interval=0.25,
            clock=lambda: now[0],
            sleep=lambda delay: now.__setitem__(0, now[0] + delay),
        )
        transport.verify_target(self._target())
        transport.send({"text": "[C2C] REVIEW_ONLY request"})
        self.assertFalse(transport.wait_result(0.75))

    def test_ignores_pre_send_final_until_a_new_message_cursor_exists(self):
        host = FakeC2CHost(
            [
                {
                    "cursor": "user-message",
                    "latestAssistantMessage": {
                        "id": "old-assistant",
                        "text": "old review\nFINAL: PASS",
                        "status": "completed",
                    },
                }
            ],
            baseline_text="old review\nFINAL: PASS",
        )
        now = [0.0]
        transport = C2CWebReviewerTransport(
            host,
            self._target(),
            poll_interval=0.25,
            clock=lambda: now[0],
            sleep=lambda delay: now.__setitem__(0, now[0] + delay),
        )
        transport.verify_target(self._target())
        transport.send({"text": "[C2C] REVIEW_ONLY request"})
        self.assertFalse(transport.wait_result(0.75))

    def test_refuses_to_send_without_a_pre_send_assistant_identity(self):
        host = FakeC2CHost(baseline_assistant_id=None)
        transport = C2CWebReviewerTransport(host, self._target())
        transport.verify_target(self._target())
        with self.assertRaisesRegex(C2CReviewerBindingError, "pre-send assistant message identity"):
            transport.send({"text": "[C2C] REVIEW_ONLY request"})
        self.assertEqual(host.send_attempts, [])

    def test_rejects_a_return_to_the_baseline_assistant_during_polling(self):
        host = FakeC2CHost(
            [
                {
                    "cursor": "new-one",
                    "latestAssistantMessage": {
                        "id": "new-assistant",
                        "text": "summary\nFINAL: PASS",
                        "status": "completed",
                    },
                },
                {
                    "cursor": "old-two",
                    "latestAssistantMessage": {
                        "id": "old-assistant",
                        "text": "old review\nFINAL: PASS",
                        "status": "completed",
                    },
                },
                {
                    "cursor": "old-three",
                    "latestAssistantMessage": {
                        "id": "old-assistant",
                        "text": "old review\nFINAL: PASS",
                        "status": "completed",
                    },
                },
            ]
        )
        now = [0.0]
        transport = C2CWebReviewerTransport(
            host,
            self._target(),
            poll_interval=0.25,
            clock=lambda: now[0],
            sleep=lambda delay: now.__setitem__(0, now[0] + delay),
        )
        transport.verify_target(self._target())
        transport.send({"text": "[C2C] REVIEW_ONLY request"})
        self.assertFalse(transport.wait_result(0.75))


if __name__ == "__main__":
    unittest.main()
