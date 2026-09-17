"""Conversation-scoped Codex executor and reviewer routing state."""

from __future__ import annotations

import json
import os
import re
import subprocess
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .active_task import resolve_context_key

STATE_VERSION = 1
VALID_DISPATCH_MODES = {"herdr", "inline"}


def _utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _routing_dir(repo_root: Path) -> Path:
    return repo_root / ".trellis" / ".runtime" / "routing"


def routing_path(repo_root: Path, context_key: str) -> Path:
    safe = re.sub(r"[^A-Za-z0-9._-]+", "_", context_key).strip("._-")
    if not safe:
        raise ValueError("empty routing context key")
    return _routing_dir(repo_root) / f"{safe}.json"


def empty_state(context_key: str) -> dict[str, Any]:
    return {
        "version": STATE_VERSION,
        "platform": "codex",
        "context_key": context_key,
        "dispatch": None,
        "reviewer": None,
        "updated_at": _utc_now(),
    }


def _valid_dispatch(value: Any) -> bool:
    if value is None:
        return True
    if not isinstance(value, dict) or value.get("mode") not in VALID_DISPATCH_MODES:
        return False
    if value["mode"] == "inline":
        return True
    return all(isinstance(value.get(key), str) and value[key] for key in ("workspace_id", "executor_pane_id"))


def _valid_reviewer(value: Any) -> bool:
    if value is None:
        return True
    return (
        isinstance(value, dict)
        and value.get("provider") == "chatgpt"
        and isinstance(value.get("conversation_id"), str)
        and bool(value["conversation_id"])
        and isinstance(value.get("url"), str)
        and value["url"].startswith("https://chatgpt.com/")
    )


def validate_state(data: Any, context_key: str) -> bool:
    return (
        isinstance(data, dict)
        and data.get("version") == STATE_VERSION
        and data.get("platform") == "codex"
        and data.get("context_key") == context_key
        and _valid_dispatch(data.get("dispatch"))
        and _valid_reviewer(data.get("reviewer"))
    )


def load_state(repo_root: Path, context_key: str) -> dict[str, Any]:
    path = routing_path(repo_root, context_key)
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, json.JSONDecodeError):
        return empty_state(context_key)
    return data if validate_state(data, context_key) else empty_state(context_key)


def save_state(repo_root: Path, state: dict[str, Any]) -> None:
    context_key = str(state.get("context_key", ""))
    if not validate_state(state, context_key):
        raise ValueError("invalid Codex routing state")
    state["updated_at"] = _utc_now()
    path = routing_path(repo_root, context_key)
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(path.suffix + f".{os.getpid()}.tmp")
    temp.write_text(json.dumps(state, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(temp, path)


def set_dispatch(
    repo_root: Path,
    context_key: str,
    mode: str,
    *,
    workspace_id: str | None = None,
    executor_pane_id: str | None = None,
) -> dict[str, Any]:
    if mode not in VALID_DISPATCH_MODES:
        raise ValueError(f"unsupported dispatch mode: {mode}")
    state = load_state(repo_root, context_key)
    dispatch: dict[str, Any] = {"mode": mode, "selected_by": "user"}
    if mode == "herdr":
        if not workspace_id or not executor_pane_id:
            raise ValueError("herdr mode requires workspace and executor pane")
        dispatch.update(workspace_id=workspace_id, executor_pane_id=executor_pane_id)
    state["dispatch"] = dispatch
    save_state(repo_root, state)
    return state


def set_reviewer(
    repo_root: Path,
    context_key: str,
    *,
    conversation_id: str,
    url: str,
    conversation_title: str = "",
    project_id: str = "",
    project_title: str = "",
) -> dict[str, Any]:
    state = load_state(repo_root, context_key)
    state["reviewer"] = {
        "provider": "chatgpt",
        "project_title": project_title,
        "project_id": project_id,
        "conversation_title": conversation_title,
        "conversation_id": conversation_id,
        "url": url,
        "selected_by": "user",
    }
    save_state(repo_root, state)
    return state


def invalidate(state: dict[str, Any], part: str) -> dict[str, Any]:
    if part not in {"dispatch", "reviewer"}:
        raise ValueError("part must be dispatch or reviewer")
    state[part] = None
    return state


@dataclass(frozen=True)
class HerdrInventory:
    current: dict[str, Any] | None
    candidates: list[dict[str, Any]]
    error: str | None = None


def parse_herdr_inventory(payload: Any, current_pane_id: str | None = None) -> HerdrInventory:
    if not isinstance(payload, dict):
        return HerdrInventory(None, [], "invalid Herdr response")
    result = payload.get("result")
    agents = result.get("agents") if isinstance(result, dict) else None
    if not isinstance(agents, list):
        return HerdrInventory(None, [], "Herdr response has no agent inventory")
    valid = [item for item in agents if isinstance(item, dict) and isinstance(item.get("pane_id"), str)]
    if current_pane_id:
        current_matches = [item for item in valid if item.get("pane_id") == current_pane_id]
    else:
        current_matches = [item for item in valid if item.get("focused") is True and item.get("agent") == "codex"]
    if len(current_matches) != 1:
        return HerdrInventory(None, [], "cannot uniquely identify the current Codex pane")
    current = current_matches[0]
    if current.get("agent") != "codex":
        return HerdrInventory(None, [], "current Herdr pane is not detected as Codex")
    workspace = current.get("workspace_id")
    candidates = [
        item
        for item in valid
        if item.get("workspace_id") == workspace
        and item.get("pane_id") != current.get("pane_id")
    ]
    return HerdrInventory(current, candidates)


def discover_herdr(command: tuple[str, ...] = ("herdr", "agent", "list")) -> HerdrInventory:
    try:
        completed = subprocess.run(command, check=True, capture_output=True, text=True, timeout=10)
        payload = json.loads(completed.stdout)
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError) as exc:
        return HerdrInventory(None, [], f"Herdr discovery failed: {exc}")
    return parse_herdr_inventory(payload, os.environ.get("HERDR_PANE_ID"))


def executor_validity(inventory: HerdrInventory, state: dict[str, Any]) -> tuple[bool, str]:
    dispatch = state.get("dispatch")
    if not isinstance(dispatch, dict):
        return False, "executor selection is missing"
    if dispatch.get("mode") == "inline":
        return True, "inline was explicitly selected"
    if inventory.error or inventory.current is None:
        return False, inventory.error or "current Herdr pane is unresolved"
    pane_id = dispatch.get("executor_pane_id")
    workspace_id = dispatch.get("workspace_id")
    match = next((item for item in inventory.candidates if item.get("pane_id") == pane_id), None)
    if match is None:
        return False, "selected executor pane is unavailable"
    if match.get("workspace_id") != workspace_id:
        return False, "selected executor moved to another workspace"
    return True, candidate_summary(match)


def candidate_summary(item: dict[str, Any]) -> str:
    return " | ".join(
        str(item.get(key) or "unknown")
        for key in ("pane_id", "agent", "agent_status", "foreground_cwd", "terminal_title_stripped")
    )


def selection_prompt(inventory: HerdrInventory, state: dict[str, Any]) -> str:
    parts: list[str] = []
    if state.get("dispatch") is None:
        if inventory.error:
            parts.append(f"Herdr executor unresolved ({inventory.error}); choose inline or wait for Herdr.")
        elif not inventory.candidates:
            parts.append("No other pane exists in this Herdr workspace; choose inline or wait for another pane.")
        else:
            listed = "\n".join(f"- {candidate_summary(item)}" for item in inventory.candidates)
            parts.append("Choose one Herdr executor pane (no candidate is auto-selected):\n" + listed)
    if state.get("reviewer") is None:
        parts.append(
            "Choose one ChatGPT reviewer: provide an existing conversation reference/URL, "
            "select an existing project chat, create a chat inside a project, or create a non-project chat."
        )
    return "\n\n".join(parts)


def resolve_codex_context_key() -> str:
    key = resolve_context_key(platform="codex")
    if not key:
        raise RuntimeError("cannot resolve the current Codex conversation identity")
    return key
