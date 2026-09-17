import tempfile
import unittest
from pathlib import Path

from common.config import get_codex_dispatch_mode
from common.codex_routing import (
    HerdrInventory,
    executor_validity,
    invalidate,
    load_state,
    parse_herdr_inventory,
    selection_prompt,
    set_dispatch,
    set_reviewer,
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
        self.assertEqual(load_state(self.root, "codex_one")["dispatch"]["executor_pane_id"], "w1:p2")
        self.assertIsNone(load_state(self.root, "codex_two")["dispatch"])

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
        self.assertIsNone(state["dispatch"])
        self.assertEqual(state["reviewer"]["conversation_id"], "c1")

    def test_corrupt_state_fails_closed(self):
        path = self.root / ".trellis/.runtime/routing/codex_one.json"
        path.parent.mkdir(parents=True)
        path.write_text("not json", encoding="utf-8")
        state = load_state(self.root, "codex_one")
        self.assertIsNone(state["dispatch"])
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
        prompt = selection_prompt(inventory, load_state(self.root, "codex_one"))
        self.assertIn("w1:p9", prompt)
        self.assertIn("no candidate is auto-selected", prompt)
        self.assertIn("Choose one ChatGPT reviewer", prompt)

    def test_no_candidate_offers_inline_or_wait(self):
        prompt = selection_prompt(HerdrInventory({"pane_id": "w1:p1"}, []), load_state(self.root, "codex_one"))
        self.assertIn("choose inline or wait", prompt)

    def test_herdr_is_a_first_class_codex_mode(self):
        config_dir = self.root / ".trellis"
        config_dir.mkdir(parents=True)
        (config_dir / "config.yaml").write_text("codex:\n  dispatch_mode: herdr\n", encoding="utf-8")
        self.assertEqual(get_codex_dispatch_mode(self.root), "herdr")
        self.assertEqual(resolve_effective_platform("codex", {"codex": {"dispatch_mode": "herdr"}}), "codex-herdr")

    def test_existing_codex_modes_remain_compatible(self):
        self.assertEqual(resolve_effective_platform("codex", {"codex": {"dispatch_mode": "auto"}}), "codex-sub-agent")
        self.assertEqual(resolve_effective_platform("codex", {"codex": {"dispatch_mode": "sub-agent"}}), "codex-sub-agent")
        self.assertEqual(resolve_effective_platform("codex", {"codex": {"dispatch_mode": "inline"}}), "codex-inline")


if __name__ == "__main__":
    unittest.main()
