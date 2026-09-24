import json
import tempfile
import unittest
from pathlib import Path

from common.automation import AutomationContext, save_context, set_reviewer
from common.automation_c2c_web import (
    C2CReviewerBindingError,
    normalize_c2c_binding,
    resolve_reviewer_target,
)
from common.automation_run import ActivationError, activate, authorize, authorization_path


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
        context = AutomationContext("codex_test")
        target, provenance = resolve_reviewer_target(context, self.root, binding_source=source)
        self.assertEqual(target["provider"], "c2c-web")
        self.assertEqual(provenance, "c2c-reviewer-binding")
        self.assertEqual(source.calls, [self.root])

    def test_authorize_persists_default_and_activation_rejects_identity_drift(self):
        task_dir = self._task()
        source = FakeBindingSource(self._payload())
        context = AutomationContext("codex_test")
        save_context(self.root, context)

        def resolve(context):
            return resolve_reviewer_target(context, self.root, binding_source=source)[0]

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
        self.assertEqual(json.loads((self.root / ".trellis/.runtime/automation/codex_test.json").read_text())["reviewer"], target)

        (task_dir / "task.json").write_text(json.dumps({"status": "in_progress"}), encoding="utf-8")
        changed = AutomationContext("codex_test")
        set_reviewer(changed, "c2c-web", CHAT + "-changed", metadata=target["metadata"])
        save_context(self.root, changed)
        with self.assertRaisesRegex(ActivationError, "changed"):
            activate(self.root, task_dir, context_key="codex_test")


if __name__ == "__main__":
    unittest.main()
