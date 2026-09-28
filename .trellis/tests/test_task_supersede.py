import json
import os
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch

import task
from common import task_store
from common.active_task import resolve_active_task, set_active_task
from common.automation_run import AutomationRun, save_run


class TaskSupersedeTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.old = self.root / ".trellis/tasks/old"
        self.new = self.root / ".trellis/tasks/new"
        for path, status in ((self.old, "in_progress"), (self.new, "in_progress")):
            path.mkdir(parents=True)
            (path / "task.json").write_text(json.dumps({"status": status, "meta": {}}))
        self.env = patch.dict(os.environ, {"TRELLIS_CONTEXT_ID": "codex_test"})
        self.env.start(); self.addCleanup(self.env.stop)
        self.root_patch = patch.object(task, "get_repo_root", return_value=self.root)
        self.root_patch.start(); self.addCleanup(self.root_patch.stop)
        set_active_task(str(self.old), self.root)

    def supersede(self):
        return task.cmd_supersede(Namespace(old=str(self.old), replacement=str(self.new), reason="missing historical snapshot"))

    def test_rejects_unaccepted_replacement_without_changing_old_task(self):
        self.assertNotEqual(self.supersede(), 0)
        self.assertEqual(json.loads((self.old / "task.json").read_text())["status"], "in_progress")
        self.assertEqual(resolve_active_task(self.root).task_path, ".trellis/tasks/old")

    def test_rejects_missing_replacement(self):
        (self.new / "task.json").unlink()
        self.assertNotEqual(self.supersede(), 0)
        self.assertEqual(json.loads((self.old / "task.json").read_text())["status"], "in_progress")

    def test_rejects_direct_pass_without_recorded_reviewer_result(self):
        save_run(self.root, AutomationRun(
            context_key="codex_test", task=".trellis/tasks/new", authorized_units=["Slice 1"],
            authorized_at="2026-09-28T00:00:00Z", current_unit=None,
            status="authorized_scope_complete", units={"Slice 1": {
                "phase": "passed", "submission": {"parent_sha": "a"*40, "head_sha": "b"*40,
                "review_round": 0, "request_kind": "bootstrap",
                "submitted_to": {"provider": "c2c-web", "reference": "https://chatgpt.com/c/conversation-1"}},
                "findings": {}, "result": {"status": "pass"}}},
        ))
        self.assertNotEqual(self.supersede(), 0)
        self.assertEqual(json.loads((self.old / "task.json").read_text())["status"], "in_progress")

    def test_rejects_missing_submitted_target_without_crashing(self):
        save_run(self.root, AutomationRun(
            context_key="codex_test", task=".trellis/tasks/new", authorized_units=["Slice 1"],
            authorized_at="2026-09-28T00:00:00Z", current_unit=None,
            status="authorized_scope_complete", units={"Slice 1": {
                "phase": "passed", "submission": {"parent_sha": "a"*40, "head_sha": "b"*40,
                "review_round": 0, "request_kind": "bootstrap", "submitted_to": None},
                "findings": {}, "result": {"status": "pass"}, "review_result_recorded": True}},
        ))
        self.assertNotEqual(self.supersede(), 0)
        self.assertEqual(json.loads((self.old / "task.json").read_text())["status"], "in_progress")

    def test_rejects_empty_review_unit_set_without_mutation(self):
        malformed = AutomationRun(
            context_key="codex_test", task=".trellis/tasks/new", authorized_units=[],
            authorized_at="2026-09-28T00:00:00Z", current_unit=None,
            status="authorized_scope_complete", units={},
        )
        with patch.object(task, "load_run", return_value=malformed):
            self.assertNotEqual(self.supersede(), 0)
        self.assertEqual(json.loads((self.old / "task.json").read_text())["status"], "in_progress")
        self.assertEqual(resolve_active_task(self.root).task_path, ".trellis/tasks/old")

    def test_reviewed_replacement_supersedes_and_clears_pointer(self):
        unit = "Slice 1"
        save_run(self.root, AutomationRun(
            context_key="codex_test", task=".trellis/tasks/new", authorized_units=[unit],
            authorized_at="2026-09-28T00:00:00Z", current_unit=None,
            reviewer_bootstrap_sent=True, status="authorized_scope_complete",
            units={unit: {"phase": "passed", "submission": {"parent_sha": "a"*40,
                "head_sha": "b"*40, "review_round": 0, "request_kind": "bootstrap",
                "submitted_to": {"provider": "c2c-web", "reference": "https://chatgpt.com/c/conversation-1"}},
                "findings": {}, "result": {"status": "pass"}, "review_result_recorded": True}},
        ))
        self.assertEqual(self.supersede(), 0)
        data = json.loads((self.old / "task.json").read_text())
        self.assertEqual(data["status"], "superseded")
        self.assertEqual(data["meta"]["superseded_by"], "new")
        self.assertIsNone(resolve_active_task(self.root).task_path)
        with patch.object(task_store, "get_repo_root", return_value=self.root):
            self.assertNotEqual(task_store.cmd_archive(Namespace(name=str(self.old), no_commit=True)), 0)
        self.assertTrue(self.old.is_dir())
        self.assertNotEqual(self.supersede(), 0)


if __name__ == "__main__":
    unittest.main()
