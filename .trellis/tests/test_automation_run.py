import json
import tempfile
import unittest
from pathlib import Path

from common.automation import AutomationContext, save_context, set_reviewer
from common.automation_run import (
    ActivationError,
    AutomationRunError,
    activate,
    authorize,
    authorization_path,
    load_run,
    parse_implementation_units,
    record_pass,
    run_path,
)


class AutomationRunTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.task_dir = self.root / ".trellis/tasks/example"
        self.task_dir.mkdir(parents=True)
        (self.task_dir / "task.json").write_text(
            json.dumps({"status": "planning"}), encoding="utf-8"
        )
        (self.task_dir / "implement.md").write_text(
            "## Slice G — gate\n\n"
            "## Slice 0 — context\n\n"
            "## Slice 1 — routing\n\n"
            "## Slice 2 — run\n\n"
            "## Slice 3 — review\n\n"
            "## Slice 4 — cleanup\n",
            encoding="utf-8",
        )
        context = AutomationContext("codex_test")
        set_reviewer(context, "chatgpt", "conversation-1")
        save_context(self.root, context)

    def tearDown(self):
        self.temp.cleanup()

    def _evidence(self, context_key="codex_test"):
        return {
            "verified": True,
            "target": {"provider": "chatgpt", "reference": "conversation-1"},
            "mechanism": "platform-native-test-transport",
            "verified_at": "2026-09-22T00:00:00Z",
        }

    def _authorize_and_activate(self, units="Slice 0-1", context_key="codex_test"):
        snapshot = authorize(
            self.root,
            self.task_dir,
            units,
            context_key=context_key,
            reviewer_transport_evidence=self._evidence(context_key),
        )
        (self.task_dir / "task.json").write_text(
            json.dumps({"status": "in_progress"}), encoding="utf-8"
        )
        return snapshot, activate(self.root, self.task_dir, context_key=context_key)

    def test_authorize_snapshots_numeric_slice_range_without_gate_or_later_units(self):
        self.assertEqual(
            parse_implementation_units(self.task_dir),
            ["Slice 0", "Slice 1", "Slice 2", "Slice 3", "Slice 4"],
        )

        snapshot = authorize(
            self.root,
            self.task_dir,
            "Slice 0-3",
            context_key="codex_test",
            reviewer_transport_evidence=self._evidence(),
        )

        self.assertEqual(snapshot.authorized_units, ["Slice 0", "Slice 1", "Slice 2", "Slice 3"])
        self.assertFalse(run_path(self.root, "codex_test").exists())
        self.assertTrue(authorization_path(self.root, "codex_test").exists())
        self.assertEqual(json.loads((self.task_dir / "task.json").read_text())["status"], "planning")

        (self.task_dir / "task.json").write_text(
            json.dumps({"status": "in_progress"}), encoding="utf-8"
        )
        run = activate(self.root, self.task_dir, context_key="codex_test")
        self.assertEqual(run.authorized_units, snapshot.authorized_units)
        self.assertEqual(run.current_unit, "Slice 0")
        self.assertEqual(run.status, "running")
        self.assertEqual(run.units["Slice 0"]["phase"], "implementing")
        self.assertIsNotNone(load_run(self.root, "codex_test"))

    def test_authorization_gate_rejects_planning_missing_reviewer_and_unverified_transport(self):
        task_json = self.task_dir / "task.json"
        with self.assertRaisesRegex(ActivationError, "transport"):
            authorize(
                self.root,
                self.task_dir,
                "Slice 0",
                context_key="codex_test",
            )
        self.assertFalse(run_path(self.root, "codex_test").exists())

        context = AutomationContext("codex_missing")
        save_context(self.root, context)
        with self.assertRaisesRegex(ActivationError, "reviewer target"):
            authorize(
                self.root,
                self.task_dir,
                "Slice 0",
                context_key="codex_missing",
            )

        snapshot = authorize(
            self.root,
            self.task_dir,
            "Slice 0",
            context_key="codex_test",
            reviewer_transport_evidence=self._evidence(),
        )
        self.assertEqual(snapshot.authorized_units, ["Slice 0"])
        self.assertFalse(run_path(self.root, "codex_test").exists())

        task_json.write_text(json.dumps({"status": "in_progress"}), encoding="utf-8")
        activated = activate(self.root, self.task_dir, context_key="codex_test")
        self.assertEqual(activated.status, "running")

    def test_start_without_prestart_snapshot_cannot_create_run(self):
        (self.task_dir / "task.json").write_text(
            json.dumps({"status": "in_progress"}), encoding="utf-8"
        )
        with self.assertRaisesRegex(ActivationError, "snapshot"):
            activate(self.root, self.task_dir, context_key="codex_test")

    def test_snapshot_does_not_extend_when_the_plan_changes(self):
        snapshot, run = self._authorize_and_activate("Slice 0-1")
        with (self.task_dir / "implement.md").open("a", encoding="utf-8") as stream:
            stream.write("\n## Slice 5 — added later\n")

        reloaded = load_run(self.root, "codex_test")
        self.assertEqual(reloaded.authorized_units, run.authorized_units)
        self.assertEqual(reloaded.authorized_units, snapshot.authorized_units)
        self.assertNotIn("Slice 5", reloaded.authorized_units)

    def test_pass_auto_advances_and_final_pass_never_changes_task_lifecycle(self):
        task_json = self.task_dir / "task.json"
        self._authorize_and_activate("Slice 0-1")
        original = task_json.read_bytes()

        first = record_pass(self.root, "codex_test")
        self.assertEqual(first.current_unit, "Slice 1")
        self.assertEqual(first.units["Slice 0"]["phase"], "passed")
        resumed = load_run(self.root, "codex_test")
        self.assertEqual(resumed.current_unit, "Slice 1")
        self.assertEqual(resumed.units["Slice 1"]["phase"], "implementing")

        final = record_pass(self.root, "codex_test")
        self.assertEqual(final.status, "authorized_scope_complete")
        self.assertIsNone(final.current_unit)
        self.assertEqual(task_json.read_bytes(), original)
        self.assertEqual(json.loads(task_json.read_text())["status"], "in_progress")

    def test_corrupt_run_fails_closed_as_blocked(self):
        self._authorize_and_activate("Slice 0")
        path = run_path(self.root, "codex_test")
        path.write_text("not-json", encoding="utf-8")

        blocked = load_run(self.root, "codex_test")
        self.assertEqual(blocked.status, "blocked")
        self.assertIn("corrupt", blocked.blocked_reason)
        with self.assertRaisesRegex(AutomationRunError, "corrupt automation run"):
            record_pass(self.root, "codex_test")


if __name__ == "__main__":
    unittest.main()
