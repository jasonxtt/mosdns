import json
import os
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch

import task
from common.active_task import resolve_active_task
from common.automation import AutomationContext, save_context, set_reviewer
from common.automation_run import authorization_path, authorize


class TaskStartGuardTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.task_dir = self.root / ".trellis/tasks/guarded"
        self.task_dir.mkdir(parents=True)
        self.task_json = self.task_dir / "task.json"
        self.task_json.write_text(json.dumps({"status": "planning", "meta": {"automation_required": "true"}}))
        (self.task_dir / "implement.md").write_text("## Slice 1 — review\n")
        self.env = patch.dict(os.environ, {"TRELLIS_CONTEXT_ID": "codex_test"})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.root_patch = patch.object(task, "get_repo_root", return_value=self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def start(self):
        return task.cmd_start(Namespace(dir=str(self.task_dir)))

    def test_missing_snapshot_does_not_start_or_change_pointer(self):
        self.assertNotEqual(self.start(), 0)
        self.assertEqual(json.loads(self.task_json.read_text())["status"], "planning")
        self.assertIsNone(resolve_active_task(self.root).task_path)

    def test_matching_snapshot_starts(self):
        context = AutomationContext("codex_test")
        set_reviewer(context, "chatgpt", "conversation-1")
        save_context(self.root, context)
        authorize(
            self.root,
            self.task_dir,
            "Slice 1",
            context_key="codex_test",
            reviewer_transport_evidence={
                "verified": True,
                "target": {"provider": "chatgpt", "reference": "conversation-1"},
                "mechanism": "test-transport",
                "verified_at": "2026-09-28T00:00:00Z",
            },
        )
        self.assertEqual(self.start(), 0)
        self.assertEqual(json.loads(self.task_json.read_text())["status"], "in_progress")
        self.assertEqual(resolve_active_task(self.root).task_path, ".trellis/tasks/guarded")

    def test_snapshot_for_another_task_does_not_start(self):
        other = self.root / ".trellis/tasks/other"
        other.mkdir()
        (other / "task.json").write_text(json.dumps({"status": "planning"}))
        (other / "implement.md").write_text("## Slice 1 — review\n")
        context = AutomationContext("codex_test")
        set_reviewer(context, "chatgpt", "conversation-1")
        save_context(self.root, context)
        authorize(self.root, other, "Slice 1", context_key="codex_test", reviewer_transport_evidence=self._evidence())
        self.assertNotEqual(self.start(), 0)
        self.assertEqual(json.loads(self.task_json.read_text())["status"], "planning")
        self.assertIsNone(resolve_active_task(self.root).task_path)

    def test_snapshot_for_another_session_does_not_start(self):
        context = AutomationContext("codex_test")
        set_reviewer(context, "chatgpt", "conversation-1")
        save_context(self.root, context)
        authorize(self.root, self.task_dir, "Slice 1", context_key="codex_test", reviewer_transport_evidence=self._evidence())
        path = authorization_path(self.root, "codex_test")
        snapshot = json.loads(path.read_text())
        snapshot["context_key"] = "codex_other"
        path.write_text(json.dumps(snapshot))
        self.assertNotEqual(self.start(), 0)
        self.assertEqual(json.loads(self.task_json.read_text())["status"], "planning")
        self.assertIsNone(resolve_active_task(self.root).task_path)

    def test_reviewer_drift_does_not_start(self):
        context = AutomationContext("codex_test")
        set_reviewer(context, "chatgpt", "conversation-1")
        save_context(self.root, context)
        authorize(self.root, self.task_dir, "Slice 1", context_key="codex_test", reviewer_transport_evidence=self._evidence())
        set_reviewer(context, "chatgpt", "conversation-2")
        save_context(self.root, context)
        self.assertNotEqual(self.start(), 0)
        self.assertEqual(json.loads(self.task_json.read_text())["status"], "planning")
        self.assertIsNone(resolve_active_task(self.root).task_path)

    def test_ordinary_task_starts_without_snapshot(self):
        self.task_json.write_text(json.dumps({"status": "planning", "meta": {}}))
        self.assertEqual(self.start(), 0)
        self.assertEqual(json.loads(self.task_json.read_text())["status"], "in_progress")

    def _evidence(self):
        return {"verified": True, "target": {"provider": "chatgpt", "reference": "conversation-1"},
                "mechanism": "test-transport", "verified_at": "2026-09-28T00:00:00Z"}


if __name__ == "__main__":
    unittest.main()
