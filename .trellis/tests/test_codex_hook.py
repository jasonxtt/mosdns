import importlib.util
import tempfile
import unittest
from pathlib import Path

from common.codex_routing import set_target
from common.workflow_phase import filter_platform


def _load_hook():
    path = Path(__file__).resolve().parents[2] / ".codex/hooks/inject-workflow-state.py"
    spec = importlib.util.spec_from_file_location("trellis_codex_hook", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load Codex hook")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


HOOK = _load_hook()


class CodexHookTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / ".trellis").mkdir()
        (self.root / ".trellis/config.yaml").write_text(
            "codex:\n  dispatch_mode: auto\n  host_routes:\n"
            "    cli: herdr\n    desktop: ask\n    unknown: ask\n",
            encoding="utf-8",
        )
        self.input_data = {
            "conversation_id": "hook-conversation",
            "_trellis_platform": "codex",
        }

    def tearDown(self):
        self.temp.cleanup()

    def test_auto_banner_names_fail_closed_desktop_policy(self):
        banner = HOOK._codex_mode_banner({"codex": {"dispatch_mode": "auto"}})
        self.assertIn("Codex CLI", banner)
        self.assertIn("Desktop/App", banner)
        self.assertIn("fails closed", banner)
        self.assertNotIn("MCP DSH", banner)
        self.assertIn("explicit executor/reviewer targets", banner)
        self.assertIn("inline: the main session", HOOK._codex_mode_banner({}, "codex"))

    def test_dsh_executor_is_rejected(self):
        with self.assertRaises(ValueError):
            set_target(self.root, "codex_hook-conversation", "executor", "dsh", "provider-managed")

    def test_dsh_web_mode_uses_browser_executor_contract(self):
        banner = HOOK._codex_mode_banner({"codex": {"dispatch_mode": "dsh-web"}})
        self.assertIn("browser-backed DSH Web executor", banner)
        self.assertIn("never call the retired MCP dsh provider", banner)
        self.assertEqual(
            HOOK.resolve_breadcrumb_key(
                "in_progress", "codex", {"codex": {"dispatch_mode": "dsh-web"}}
            ),
            "in_progress-dsh-web",
        )

    def test_unknown_surface_reports_both_missing_slots_together(self):
        surface = {"kind": "unknown", "source": "none", "evidence": []}
        banner = HOOK._codex_routing_banner(self.root, self.input_data, surface)
        self.assertIn("policy=ask", banner)
        self.assertIn("executor, reviewer", banner)
        self.assertIn("executor=codex/reviewer=codex", banner)

    def test_breadcrumb_key_uses_host_route(self):
        config = {
            "codex": {
                "dispatch_mode": "auto",
                "host_routes": {"cli": "herdr", "desktop": "ask", "unknown": "ask"},
            }
        }
        self.assertEqual(HOOK.resolve_breadcrumb_key("in_progress", "codex", config, "cli"), "in_progress-herdr")
        self.assertEqual(HOOK.resolve_breadcrumb_key("in_progress", "codex", config, "desktop"), "in_progress-auto")
        self.assertEqual(HOOK.resolve_breadcrumb_key("in_progress", "codex", config, "unknown"), "in_progress-auto")
        self.assertEqual(
            HOOK.resolve_breadcrumb_key("in_progress", "codex", config, "desktop", "codex"),
            "in_progress-inline",
        )

    def test_workflow_filter_keeps_only_resolved_provider_block(self):
        content = "[codex-dsh-web]\nDSH Web instructions\n[/codex-dsh-web]\n[codex-herdr]\nHerdr instructions\n[/codex-herdr]"
        self.assertEqual(filter_platform(content, "codex-dsh-web").strip(), "DSH Web instructions")


if __name__ == "__main__":
    unittest.main()
