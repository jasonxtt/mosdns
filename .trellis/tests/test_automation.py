import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from common.automation import (
    AutomationContext,
    clear_executor,
    clear_reviewer,
    context_path,
    load_context,
    make_target,
    migrate_legacy_routing,
    resolve_executor,
    save_context,
    set_executor,
    set_reviewer,
    validate_target,
)
from common.automation_dsh_web import (
    available as dsh_web_available,
    collect as dsh_web_collect,
    dispatch as dsh_web_dispatch,
    discover_dsh_web,
)
from common.automation_herdr import (
    available as herdr_available,
    collect as herdr_collect,
    dispatch as herdr_dispatch,
    discover_herdr,
)


class AutomationContextTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def _legacy_path(self, context_key="codex_one", root=None):
        return (root or self.root) / ".trellis/.runtime/routing" / f"{context_key}.json"

    def _write_legacy(self, value, context_key="codex_one", root=None):
        path = self._legacy_path(context_key, root)
        path.parent.mkdir(parents=True, exist_ok=True)
        raw = value if isinstance(value, bytes) else json.dumps(value, separators=(",", ":")).encode()
        path.write_bytes(raw)
        return path, raw

    def test_fresh_context_defaults_to_current_and_has_no_reviewer(self):
        context = load_context(self.root, "codex_one")

        self.assertIsNone(context.executor_override)
        self.assertEqual(resolve_executor(context), "current")
        self.assertIsNone(context.reviewer)
        self.assertFalse(context_path(self.root, "codex_one").exists())

    def test_persistence_and_clear_restore_current_defaults(self):
        context = load_context(self.root, "codex_one")
        set_executor(context, "herdr", "w1:p2", label="worker")
        set_reviewer(context, "chatgpt", "conversation-1", label="review")
        save_context(self.root, context)

        loaded = load_context(self.root, "codex_one")
        self.assertEqual(loaded.executor_override["provider"], "herdr")
        self.assertEqual(resolve_executor(loaded)["reference"], "w1:p2")
        self.assertEqual(loaded.reviewer["reference"], "conversation-1")

        clear_executor(loaded)
        clear_reviewer(loaded)
        save_context(self.root, loaded)
        cleared = load_context(self.root, "codex_one")
        self.assertIsNone(cleared.executor_override)
        self.assertEqual(resolve_executor(cleared), "current")
        self.assertIsNone(cleared.reviewer)

    def test_generic_target_validation_rejects_empty_and_identity_overwrite(self):
        self.assertTrue(validate_target({"provider": "chatgpt", "reference": "c1"}))
        self.assertFalse(validate_target({"provider": "", "reference": "c1"}))
        self.assertFalse(validate_target({"provider": "chatgpt", "reference": ""}))
        with self.assertRaises(ValueError):
            make_target("chatgpt", "c1", metadata={"provider": "other"})
        with self.assertRaises(ValueError):
            make_target("", "c1")

    def test_current_executor_is_an_implicit_default_not_a_persisted_override(self):
        context = AutomationContext(context_key="codex_one")
        set_executor(context, "current", "current")

        self.assertIsNone(context.executor_override)
        self.assertEqual(resolve_executor(context), "current")

    def test_user_provenance_migrates_supported_targets_and_fingerprints_source(self):
        path, raw = self._write_legacy(
            {
                "version": 2,
                "platform": "codex",
                "context_key": "codex_one",
                "surface": {"kind": "desktop", "source": "env_marker", "evidence": ["marker"]},
                "executor": {
                    "provider": "herdr",
                    "reference": "w1:p2",
                    "label": "worker",
                    "selected_by": "user",
                    "workspace_id": "w1",
                    "executor_pane_id": "w1:p2",
                },
                "reviewer": {
                    "provider": "chatgpt",
                    "reference": "conversation-1",
                    "label": "review",
                    "selected_by": "user",
                    "url": "https://chatgpt.com/c/conversation-1",
                },
            }
        )

        context = migrate_legacy_routing(self.root, "codex_one")

        self.assertEqual(context.executor_override["provider"], "herdr")
        self.assertEqual(context.executor_override["reference"], "w1:p2")
        self.assertEqual(context.executor_override["metadata"]["workspace_id"], "w1")
        self.assertEqual(context.reviewer["reference"], "conversation-1")
        self.assertEqual(
            context.migrated_from,
            {
                "path": ".trellis/.runtime/routing/codex_one.json",
                "sha256": hashlib.sha256(raw).hexdigest(),
            },
        )
        self.assertEqual(path.read_bytes(), raw)

    def test_ambiguous_provenance_and_retired_mcp_dsh_are_not_bound(self):
        path, raw = self._write_legacy(
            {
                "version": 2,
                "platform": "codex",
                "context_key": "codex_one",
                "surface": {"kind": "cli", "source": "env_marker", "evidence": ["marker"]},
                "executor": {"provider": "herdr", "reference": "w1:p2", "selected_by": "policy"},
                "reviewer": {"provider": "chatgpt", "reference": "c1", "selected_by": "auto"},
                "legacy_dsh": {"provider": "dsh", "reference": "mcp"},
            }
        )

        context = migrate_legacy_routing(self.root, "codex_one")

        self.assertIsNone(context.executor_override)
        self.assertIsNone(context.reviewer)
        self.assertNotIn("surface", context.to_dict())
        self.assertEqual(path.read_bytes(), raw)

    def test_all_ambiguous_provenance_values_are_dropped(self):
        for selected_by in ("migration", "policy", "auto", None):
            with self.subTest(selected_by=selected_by):
                with tempfile.TemporaryDirectory() as temp:
                    root = Path(temp)
                    value = {
                        "version": 2,
                        "platform": "codex",
                        "context_key": "codex_one",
                        "executor": {"provider": "herdr", "reference": "w1:p2"},
                        "reviewer": {"provider": "chatgpt", "reference": "c1"},
                    }
                    if selected_by is not None:
                        value["executor"]["selected_by"] = selected_by
                        value["reviewer"]["selected_by"] = selected_by
                    self._write_legacy(value, root=root)
                    migrated = migrate_legacy_routing(root, "codex_one")
                    self.assertIsNone(migrated.executor_override)
                    self.assertIsNone(migrated.reviewer)

    def test_explicit_user_codex_current_migrates_to_implicit_default(self):
        self._write_legacy(
            {
                "version": 2,
                "platform": "codex",
                "context_key": "codex_one",
                "executor": {"provider": "codex", "reference": "current", "selected_by": "user"},
            }
        )

        migrated = migrate_legacy_routing(self.root, "codex_one")

        self.assertIsNone(migrated.executor_override)
        self.assertEqual(resolve_executor(migrated), "current")

    def test_legacy_v1_reviewer_without_explicit_user_provenance_is_dropped(self):
        path, raw = self._write_legacy(
            {
                "version": 1,
                "platform": "codex",
                "context_key": "codex_one",
                "dispatch": {
                    "mode": "herdr",
                    "workspace_id": "w1",
                    "executor_pane_id": "w1:p2",
                    "selected_by": "user",
                },
                "reviewer": {"conversation_id": "conversation-1"},
            }
        )

        context = migrate_legacy_routing(self.root, "codex_one")

        self.assertEqual(context.executor_override["provider"], "herdr")
        self.assertIsNone(context.reviewer)
        self.assertEqual(path.read_bytes(), raw)

    def test_inline_current_and_mcp_dsh_v1_dispatch_do_not_create_overrides(self):
        for mode in ("inline", "dsh"):
            with self.subTest(mode=mode):
                with tempfile.TemporaryDirectory() as temp:
                    root = Path(temp)
                    self._write_legacy(
                        {
                            "version": 1,
                            "platform": "codex",
                            "context_key": "codex_one",
                            "dispatch": {"mode": mode, "selected_by": "user"},
                        },
                        root=root,
                    )
                    context = migrate_legacy_routing(root, "codex_one")
                    self.assertIsNone(context.executor_override)
                    self.assertEqual(resolve_executor(context), "current")

    def test_corrupt_legacy_state_returns_safe_defaults_and_preserves_bytes(self):
        path, raw = self._write_legacy(b"not-json")

        context = load_context(self.root, "codex_one")

        self.assertIsNone(context.executor_override)
        self.assertIsNone(context.reviewer)
        self.assertEqual(path.read_bytes(), raw)
        self.assertEqual(context.migrated_from["sha256"], hashlib.sha256(raw).hexdigest())

    def test_migration_is_idempotent_and_does_not_overwrite_new_context(self):
        path, raw = self._write_legacy(
            {
                "version": 2,
                "platform": "codex",
                "context_key": "codex_one",
                "executor": {"provider": "herdr", "reference": "w1:p2", "selected_by": "user"},
            }
        )
        first = load_context(self.root, "codex_one")
        set_reviewer(first, "chatgpt", "new-reviewer")
        save_context(self.root, first)
        path.write_bytes(raw + b"\nchanged legacy bytes")

        second = load_context(self.root, "codex_one")

        self.assertEqual(second.reviewer["reference"], "new-reviewer")
        self.assertEqual(second.executor_override["reference"], "w1:p2")
        self.assertEqual(second.migrated_from["sha256"], hashlib.sha256(raw).hexdigest())


class ExplicitAdapterTest(unittest.TestCase):
    def test_herdr_discovery_and_availability_use_fake_transport_boundary(self):
        payload = {
            "result": {
                "agents": [
                    {"pane_id": "w1:p1", "workspace_id": "w1", "agent": "codex", "focused": True},
                    {"pane_id": "w1:p2", "workspace_id": "w1", "agent": "claude", "focused": False},
                ]
            }
        }
        completed = SimpleNamespace(stdout=json.dumps(payload))
        with mock.patch(
            "common.automation_herdr.subprocess.run", return_value=completed
        ) as run:
            inventory = discover_herdr(("herdr", "agent", "list"))
        run.assert_called_once()
        target = {"provider": "herdr", "reference": "w1:p2", "workspace_id": "w1"}
        self.assertTrue(herdr_available(target, inventory))
        self.assertTrue(
            herdr_available(
                {
                    "provider": "herdr",
                    "reference": "w1:p2",
                    "metadata": {"workspace_id": "w1", "executor_pane_id": "w1:p2"},
                },
                inventory,
            )
        )
        self.assertFalse(herdr_available({"provider": "herdr", "reference": "w1:p9", "workspace_id": "w1"}, inventory))

        class FakeTransport:
            def __init__(self):
                self.calls = []

            def dispatch(self, selected, prompt):
                self.calls.append(("dispatch", selected, prompt))
                return "sent"

            def collect(self, selected):
                self.calls.append(("collect", selected))
                return "result"

        transport = FakeTransport()
        self.assertEqual(herdr_dispatch("review Slice 1", target=target, transport=transport), "sent")
        self.assertEqual(herdr_collect(target=target, transport=transport), "result")
        self.assertEqual([call[0] for call in transport.calls], ["dispatch", "collect"])

    def test_dsh_web_discovery_normalizes_explicit_browser_target(self):
        completed = SimpleNamespace(
            stdout="3003 node /opt/dsh web --port 3080 --trusted-host dsh.example.test\n"
        )
        with mock.patch("common.automation_dsh_web.subprocess.run", return_value=completed) as run:
            inventory = discover_dsh_web(("ps", "-axo", "pid=,command="))
        run.assert_called_once()
        target = {"provider": "dsh-web", "reference": "https://dsh.example.test/"}
        self.assertTrue(dsh_web_available(target, inventory))
        self.assertFalse(dsh_web_available({"provider": "dsh-web", "reference": "https://other.test/"}, inventory))

        class FakeTransport:
            def dispatch(self, selected, prompt):
                return (selected["reference"], prompt)

            def collect(self, selected):
                return selected["reference"]

        transport = FakeTransport()
        self.assertEqual(
            dsh_web_dispatch("review Slice 1", target=target, transport=transport),
            (target["reference"], "review Slice 1"),
        )
        self.assertEqual(dsh_web_collect(target=target, transport=transport), target["reference"])

    def test_adapters_do_not_fake_transport_when_not_supplied(self):
        herdr_target = {"provider": "herdr", "reference": "w1:p2", "workspace_id": "w1"}
        dsh_target = {"provider": "dsh-web", "reference": "https://dsh.example.test/"}
        with self.assertRaisesRegex(RuntimeError, "transport is not available"):
            herdr_dispatch("prompt", target=herdr_target)
        with self.assertRaisesRegex(RuntimeError, "transport is not available"):
            dsh_web_dispatch("prompt", target=dsh_target)


if __name__ == "__main__":
    unittest.main()
