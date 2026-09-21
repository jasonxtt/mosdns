import tempfile
import unittest
from pathlib import Path

from common.config import get_codex_dispatch_mode, get_codex_host_routes
from common.codex_routing import (
    DshWebInventory,
    HerdrInventory,
    detect_surface,
    discover_dsh_web,
    executor_validity,
    invalidate,
    load_state,
    parse_herdr_inventory,
    resolve_codex_provider,
    selection_prompt,
    set_dispatch,
    set_reviewer,
    set_surface,
    set_target,
)
from common.workflow_phase import resolve_effective_platform


class CodexRoutingTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def test_state_is_isolated_by_conversation(self):
        set_dispatch(self.root, "codex_one", "herdr", workspace_id="w1", executor_pane_id="w1:p2")
        self.assertEqual(load_state(self.root, "codex_one")["executor"]["executor_pane_id"], "w1:p2")
        self.assertIsNone(load_state(self.root, "codex_two")["executor"])

    def test_reviewer_and_dispatch_invalidate_independently(self):
        state = set_dispatch(self.root, "codex_one", "herdr", workspace_id="w1", executor_pane_id="w1:p2")
        state = set_reviewer(
            self.root,
            "codex_one",
            conversation_id="c1",
            url="https://chatgpt.com/c/c1",
            conversation_title="review",
        )
        invalidate(state, "dispatch")
        self.assertIsNone(state["executor"])
        self.assertEqual(state["reviewer"]["conversation_id"], "c1")

    def test_corrupt_state_fails_closed(self):
        path = self.root / ".trellis/.runtime/routing/codex_one.json"
        path.parent.mkdir(parents=True)
        path.write_text("not json", encoding="utf-8")
        state = load_state(self.root, "codex_one")
        self.assertIsNone(state["executor"])
        self.assertIsNone(state["reviewer"])

    def test_inventory_keeps_all_non_current_agents(self):
        payload = {
            "result": {
                "agents": [
                    {"pane_id": "w1:p1", "workspace_id": "w1", "agent": "codex", "focused": True},
                    {"pane_id": "w1:p2", "workspace_id": "w1", "agent": "claude", "focused": False},
                    {"pane_id": "w1:p3", "workspace_id": "w1", "agent": "zcode", "focused": False},
                    {"pane_id": "w2:p1", "workspace_id": "w2", "agent": "codex", "focused": False},
                ]
            }
        }
        inventory = parse_herdr_inventory(payload)
        self.assertEqual([item["pane_id"] for item in inventory.candidates], ["w1:p2", "w1:p3"])

    def test_environment_pane_identity_does_not_depend_on_focus(self):
        payload = {"result": {"agents": [
            {"pane_id": "w1:p1", "workspace_id": "w1", "agent": "codex", "focused": False},
            {"pane_id": "w1:p2", "workspace_id": "w1", "agent": "other", "focused": True},
        ]}}
        inventory = parse_herdr_inventory(payload, "w1:p1")
        self.assertEqual(inventory.current["pane_id"], "w1:p1")
        self.assertEqual(inventory.candidates[0]["pane_id"], "w1:p2")

    def test_missing_selected_executor_fails_closed(self):
        state = set_dispatch(self.root, "codex_one", "herdr", workspace_id="w1", executor_pane_id="w1:p9")
        valid, reason = executor_validity(HerdrInventory({"pane_id": "w1:p1"}, []), state)
        self.assertFalse(valid)
        self.assertIn("unavailable", reason)

    def test_one_candidate_is_not_auto_selected(self):
        inventory = HerdrInventory(
            {"pane_id": "w1:p1"},
            [{"pane_id": "w1:p9", "agent": "other", "agent_status": "idle", "foreground_cwd": "/tmp", "terminal_title_stripped": "worker"}],
        )
        prompt = selection_prompt(inventory, load_state(self.root, "codex_one"), provider="herdr")
        self.assertIn("w1:p9", prompt)
        self.assertIn("no candidate is auto-selected", prompt)
        self.assertIn("Choose one ChatGPT reviewer", prompt)

    def test_no_candidate_offers_inline_or_wait(self):
        prompt = selection_prompt(
            HerdrInventory({"pane_id": "w1:p1"}, []),
            load_state(self.root, "codex_one"),
            provider="herdr",
        )
        self.assertIn("choose inline or wait", prompt)

    def test_herdr_is_a_first_class_codex_mode(self):
        config_dir = self.root / ".trellis"
        config_dir.mkdir(parents=True)
        (config_dir / "config.yaml").write_text("codex:\n  dispatch_mode: herdr\n", encoding="utf-8")
        self.assertEqual(get_codex_dispatch_mode(self.root), "herdr")
        self.assertEqual(resolve_effective_platform("codex", {"codex": {"dispatch_mode": "herdr"}}), "codex-herdr")

    def test_retired_dsh_policy_fails_closed(self):
        config_dir = self.root / ".trellis"
        config_dir.mkdir(parents=True)
        (config_dir / "config.yaml").write_text("codex:\n  dispatch_mode: dsh\n", encoding="utf-8")
        self.assertEqual(get_codex_dispatch_mode(self.root), "ask")
        self.assertEqual(resolve_effective_platform("codex", {"codex": {"dispatch_mode": "dsh"}}), "codex-auto")

    def test_dsh_web_is_explicit_browser_executor_and_validates_against_discovery(self):
        endpoint = "https://dsh.959527.xyz/"
        state = set_target(self.root, "codex_one", "executor", "dsh-web", endpoint)
        inventory = DshWebInventory(
            [{"provider": "dsh-web", "reference": endpoint, "label": "DSH Web", "transport": "browser-ui"}]
        )
        valid, reason = executor_validity(HerdrInventory(None, [], "not needed"), state, inventory)
        self.assertTrue(valid, reason)
        self.assertEqual(resolve_codex_provider(self.root, "desktop", state), "dsh-web")
        self.assertEqual(
            resolve_effective_platform("codex", {"codex": {"dispatch_mode": "dsh-web"}}),
            "codex-dsh-web",
        )

    def test_dsh_web_discovery_is_process_based_and_not_mcp(self):
        inventory = discover_dsh_web(
            (
                "echo",
                "3003 node /opt/homebrew/bin/dsh web --port 3080 --no-open --trusted-host dsh.959527.xyz",
            )
        )
        self.assertEqual(len(inventory.candidates), 1)
        self.assertEqual(inventory.candidates[0]["provider"], "dsh-web")
        self.assertEqual(inventory.candidates[0]["reference"], "https://dsh.959527.xyz/")
        self.assertEqual(inventory.candidates[0]["transport"], "browser-ui")

    def test_ask_prompt_recommends_running_dsh_web_without_selecting_it(self):
        inventory = DshWebInventory(
            [{"provider": "dsh-web", "reference": "https://dsh.959527.xyz/", "label": "DSH Web"}]
        )
        prompt = selection_prompt(
            HerdrInventory(None, [], "not needed"),
            load_state(self.root, "codex_one"),
            surface="desktop",
            provider="ask",
            dsh_web=inventory,
        )
        self.assertIn("Recommended executor", prompt)
        self.assertIn("dsh-web:https://dsh.959527.xyz/", prompt)
        self.assertIn("not MCP", prompt)
        self.assertIn("Choose one ChatGPT reviewer", prompt)

    def test_existing_codex_modes_remain_compatible(self):
        self.assertEqual(resolve_effective_platform("codex", {"codex": {"dispatch_mode": "auto"}}), "codex-sub-agent")
        self.assertEqual(resolve_effective_platform("codex", {"codex": {"dispatch_mode": "sub-agent"}}), "codex-sub-agent")
        self.assertEqual(resolve_effective_platform("codex", {"codex": {"dispatch_mode": "inline"}}), "codex-inline")

    def test_v1_inline_state_migrates_to_current_codex_executor(self):
        path = self.root / ".trellis/.runtime/routing/codex_one.json"
        path.parent.mkdir(parents=True)
        path.write_text(
            '{"version": 1, "platform": "codex", "context_key": "codex_one", '
            '"dispatch": {"mode": "inline", "selected_by": "user"}, '
            '"reviewer": null, "updated_at": "2026-09-19T00:00:00Z"}',
            encoding="utf-8",
        )
        state = load_state(self.root, "codex_one")
        self.assertEqual(state["version"], 2)
        self.assertEqual(state["executor"]["provider"], "codex")
        self.assertEqual(state["executor"]["reference"], "current")
        self.assertIsNone(state["reviewer"])

    def test_v1_herdr_and_chatgpt_state_migrates_losslessly(self):
        path = self.root / ".trellis/.runtime/routing/codex_one.json"
        path.parent.mkdir(parents=True)
        path.write_text(
            '{"version": 1, "platform": "codex", "context_key": "codex_one", '
            '"dispatch": {"mode": "herdr", "workspace_id": "w1", '
            '"executor_pane_id": "w1:p2", "selected_by": "user"}, '
            '"reviewer": {"provider": "chatgpt", "conversation_id": "c1", '
            '"url": "https://chatgpt.com/c/c1", "conversation_title": "review"}, '
            '"updated_at": "2026-09-19T00:00:00Z"}',
            encoding="utf-8",
        )
        state = load_state(self.root, "codex_one")
        self.assertEqual(state["executor"]["provider"], "herdr")
        self.assertEqual(state["executor"]["reference"], "w1:p2")
        self.assertEqual(state["executor"]["workspace_id"], "w1")
        self.assertEqual(state["reviewer"]["provider"], "chatgpt")
        self.assertEqual(state["reviewer"]["reference"], "c1")
        self.assertEqual(state["reviewer"]["url"], "https://chatgpt.com/c/c1")

    def test_user_can_select_codex_for_both_roles_without_provider_specific_schema(self):
        state = set_target(self.root, "codex_one", "executor", "codex", "current", label="Codex")
        state = set_target(self.root, "codex_one", "reviewer", "codex", "current", label="self-review")
        self.assertEqual(state["executor"]["provider"], "codex")
        self.assertEqual(state["reviewer"]["provider"], "codex")
        self.assertEqual(state["reviewer"]["reference"], "current")
        valid, reason = executor_validity(HerdrInventory(None, [], "not needed"), state)
        self.assertTrue(valid, reason)

    def test_user_can_select_a_specific_codex_thread_executor(self):
        reference = "codex://threads/01a0bf45-21be-7543-a49d-88f037640f18"
        state = set_target(self.root, "codex_one", "executor", "codex", reference, label="Codex Slice executor")
        valid, reason = executor_validity(HerdrInventory(None, [], "not needed"), state)
        self.assertTrue(valid, reason)

    def test_unknown_provider_target_is_representable_but_not_dispatchable(self):
        state = set_target(self.root, "codex_one", "executor", "future-agent", "opaque-123")
        valid, reason = executor_validity(HerdrInventory(None, [], "not needed"), state)
        self.assertFalse(valid)
        self.assertIn("unsupported", reason)

    def test_surface_detection_uses_safe_evidence(self):
        desktop = detect_surface({"CODEX_APP_TOOLS_PIPE_PATH": "/private/token-containing-path"})
        self.assertEqual(desktop.kind, "desktop")
        self.assertEqual(desktop.source, "env_marker")
        self.assertEqual(desktop.evidence, ("CODEX_APP_TOOLS_PIPE_PATH",))
        self.assertNotIn("private", " ".join(desktop.evidence))

        cli = detect_surface({"CODEX_CLI_SURFACE": "1"})
        self.assertEqual(cli.kind, "cli")
        self.assertEqual(cli.evidence, ("CODEX_CLI_SURFACE",))

    def test_surface_detection_is_unknown_for_missing_or_conflicting_evidence(self):
        self.assertEqual(detect_surface({}).kind, "unknown")
        conflicting = detect_surface({
            "CODEX_APP_TOOLS_PIPE_PATH": "/tmp/app",
            "CODEX_CLI_SURFACE": "1",
        })
        self.assertEqual(conflicting.kind, "unknown")
        self.assertEqual(
            conflicting.evidence,
            ("CODEX_APP_TOOLS_PIPE_PATH", "CODEX_CLI_SURFACE"),
        )

    def test_auto_policy_maps_surface_to_provider_without_selecting_resource(self):
        config_dir = self.root / ".trellis"
        config_dir.mkdir(parents=True)
        (config_dir / "config.yaml").write_text(
            "codex:\n  dispatch_mode: auto\n  host_routes:\n"
            "    cli: herdr\n    desktop: ask\n    unknown: ask\n",
            encoding="utf-8",
        )
        self.assertEqual(get_codex_host_routes(self.root)["desktop"], "ask")
        self.assertEqual(resolve_codex_provider(self.root, "cli"), "herdr")
        self.assertEqual(resolve_codex_provider(self.root, "desktop"), "ask")
        self.assertEqual(resolve_codex_provider(self.root, "unknown"), "ask")

        selected = set_target(self.root, "codex_one", "executor", "codex", "current")
        self.assertEqual(resolve_codex_provider(self.root, "desktop", selected), "codex")

    def test_invalid_policy_fails_closed_and_surface_filter_is_host_aware(self):
        config_dir = self.root / ".trellis"
        config_dir.mkdir(parents=True)
        (config_dir / "config.yaml").write_text(
            "codex:\n  dispatch_mode: invalid-provider\n",
            encoding="utf-8",
        )
        self.assertEqual(get_codex_dispatch_mode(self.root), "ask")
        self.assertEqual(
            resolve_effective_platform(
                "codex",
                {"codex": {"dispatch_mode": "auto"}},
                surface="desktop",
            ),
            "codex-auto",
        )
        self.assertEqual(
            resolve_effective_platform(
                "codex",
                {"codex": {"dispatch_mode": "auto"}},
                surface="cli",
            ),
            "codex-herdr",
        )
        self.assertEqual(
            resolve_effective_platform(
                "codex",
                {"codex": {"dispatch_mode": "auto"}},
                surface="unknown",
            ),
            "codex-auto",
        )
        self.assertEqual(
            resolve_effective_platform(
                "codex",
                {"codex": {"dispatch_mode": "auto"}},
                surface="desktop",
                provider="codex",
            ),
            "codex-inline",
        )

    def test_surface_override_is_conversation_scoped(self):
        state = set_surface(self.root, "codex_one", "desktop")
        self.assertEqual(state["surface"]["kind"], "desktop")
        self.assertIsNone(load_state(self.root, "codex_two")["surface"])

    def test_invalidate_executor_preserves_reviewer(self):
        state = set_target(self.root, "codex_one", "executor", "codex", "current")
        state = set_target(self.root, "codex_one", "reviewer", "codex", "current")
        invalidate(state, "executor")
        self.assertIsNone(state["executor"])
        self.assertEqual(state["reviewer"]["provider"], "codex")


if __name__ == "__main__":
    unittest.main()
