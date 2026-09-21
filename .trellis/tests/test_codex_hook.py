import importlib.util
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from common.active_task import resolve_context_key
from common.automation import load_context, save_context, set_executor, set_reviewer
from common.workflow_phase import filter_platform, resolve_effective_platform


def _load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {name}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ROOT = Path(__file__).resolve().parents[2]
HOOK = _load_module(ROOT / ".codex/hooks/inject-workflow-state.py", "trellis_codex_hook")
SESSION = _load_module(ROOT / ".codex/hooks/session-start.py", "trellis_codex_session_start")


class CodexHookTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / ".trellis").mkdir()
        (self.root / ".trellis/config.yaml").write_text(
            "codex:\n  dispatch_mode: inline\n",
            encoding="utf-8",
        )
        self.input_data = {
            "conversation_id": "hook-conversation",
            "_trellis_platform": "codex",
            "cwd": str(self.root),
        }

    def tearDown(self):
        self.temp.cleanup()

    def test_mode_banner_only_describes_simple_execution_mode(self):
        inline = HOOK._codex_mode_banner({"codex": {"dispatch_mode": "inline"}})
        sub_agent = HOOK._codex_mode_banner({"codex": {"dispatch_mode": "sub-agent"}})
        self.assertIn("inline", inline)
        self.assertIn("sub-agent", sub_agent)
        for banner in (inline, sub_agent):
            self.assertNotIn("Herdr", banner)
            self.assertNotIn("DSH", banner)
            self.assertNotIn("surface", banner)
            self.assertNotIn("host", banner)

    def test_breadcrumb_key_is_plain_lifecycle_status(self):
        config = {
            "codex": {
                "dispatch_mode": "sub-agent",
            }
        }
        self.assertEqual(
            HOOK.resolve_breadcrumb_key(
                "in_progress", "codex", config, {"kind": "desktop"}, "herdr"
            ),
            "in_progress",
        )
        self.assertEqual(HOOK.resolve_breadcrumb_key("planning", "codex", config), "planning")

    def test_automation_banner_reports_current_and_missing_reviewer(self):
        with mock.patch.dict(os.environ, {"TRELLIS_CONTEXT_ID": ""}, clear=False):
            banner = HOOK._codex_automation_banner(self.root, self.input_data)
        self.assertIn("executor=current", banner)
        self.assertIn("reviewer=missing", banner)
        self.assertIn("run=none", banner)
        self.assertIn("informational", banner)

    def test_automation_banner_reports_explicit_override_and_reviewer(self):
        with mock.patch.dict(os.environ, {"TRELLIS_CONTEXT_ID": ""}, clear=False):
            key = resolve_context_key(self.input_data, platform="codex")
            context = load_context(self.root, key)
            set_executor(context, "herdr", "workspace:pane", label="worker")
            set_reviewer(context, "chatgpt", "https://chatgpt.com/c/reviewer", label="root review")
            save_context(self.root, context)
            banner = HOOK._codex_automation_banner(self.root, self.input_data)
        self.assertIn("executor=explicit override", banner)
        self.assertIn("reviewer=selected", banner)
        self.assertIn("run=none", banner)

    def test_per_turn_hook_does_not_discover_external_providers(self):
        stdin = io.StringIO(json.dumps(self.input_data))
        stdout = io.StringIO()
        with mock.patch.object(HOOK.sys, "stdin", stdin), mock.patch("subprocess.run") as run, mock.patch.object(
            HOOK.sys, "stdout", stdout
        ), mock.patch.object(HOOK, "_detect_platform", return_value="codex"):
            self.assertEqual(HOOK.main(), 0)
        self.assertFalse(run.called)
        payload = json.loads(stdout.getvalue())
        context = payload["hookSpecificOutput"]["additionalContext"]
        self.assertIn("<automation>", context)
        self.assertIn("<workflow-state>", context)
        self.assertNotIn("codex-routing", context)

    def test_session_start_does_not_discover_external_providers(self):
        stdin = io.StringIO(json.dumps({"cwd": str(self.root), "conversation_id": "session-start"}))
        stdout = io.StringIO()

        class Completed:
            returncode = 0
            stdout = "rust\n"

        with mock.patch.object(SESSION.sys, "stdin", stdin), mock.patch.object(
            SESSION.sys, "stdout", stdout
        ), mock.patch.object(SESSION.subprocess, "run", return_value=Completed()) as run:
            SESSION.main()

        self.assertTrue(run.called)  # generic git state is allowed
        for call in run.call_args_list:
            command = call.args[0]
            self.assertNotIn("herdr", command)
            self.assertNotIn("ps", command)
        payload = json.loads(stdout.getvalue())
        self.assertIn("<current-state>", payload["hookSpecificOutput"]["additionalContext"])

    def test_session_start_includes_existing_automation_summary(self):
        with mock.patch.dict(os.environ, {"TRELLIS_CONTEXT_ID": ""}, clear=False):
            key = resolve_context_key(self.input_data, platform="codex")
            context = load_context(self.root, key)
            set_reviewer(context, "chatgpt", "reviewer-url", label="root reviewer")
            save_context(self.root, context)
            summary = SESSION._automation_summary(self.root, self.input_data)
        self.assertEqual(summary, "Automation: executor=current; reviewer=selected; run=none.")

    def test_platform_filter_keeps_generic_codex_marker(self):
        content = "[codex-inline]\nInline instructions\n[/codex-inline]"
        self.assertEqual(filter_platform(content, "codex-inline").strip(), "Inline instructions")

    def test_surface_and_provider_arguments_never_change_codex_mode(self):
        config = {"codex": {"dispatch_mode": "sub-agent"}}
        self.assertEqual(
            resolve_effective_platform("codex", config, surface="desktop", provider="herdr"),
            "codex-sub-agent",
        )
        self.assertEqual(
            resolve_effective_platform("codex", config, surface="cli", provider="dsh-web"),
            "codex-sub-agent",
        )


if __name__ == "__main__":
    unittest.main()
