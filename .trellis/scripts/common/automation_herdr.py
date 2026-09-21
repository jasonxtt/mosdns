"""Explicit Herdr executor adapter.

Discovery and target availability live here so the active Trellis workflow can
use generic automation context without selecting Herdr from host evidence.
The host controller supplies the actual transport to ``dispatch`` and
``collect``; this module never invents a transport or silently falls back.
"""

from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class HerdrInventory:
    current: dict[str, Any] | None
    candidates: list[dict[str, Any]]
    error: str | None = None


def parse_herdr_inventory(payload: Any, current_pane_id: str | None = None) -> HerdrInventory:
    """Parse a Herdr agent-list response without choosing a candidate."""
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
        if item.get("workspace_id") == workspace and item.get("pane_id") != current.get("pane_id")
    ]
    return HerdrInventory(current, candidates)


def discover_herdr(command: tuple[str, ...] = ("herdr", "agent", "list")) -> HerdrInventory:
    """Discover Herdr panes when an explicit Herdr target requested it."""
    try:
        completed = subprocess.run(command, check=True, capture_output=True, text=True, timeout=10)
        payload = json.loads(completed.stdout)
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError) as exc:
        return HerdrInventory(None, [], f"Herdr discovery failed: {exc}")
    return parse_herdr_inventory(payload, os.environ.get("HERDR_PANE_ID"))


def available(target: dict[str, Any] | None, inventory: HerdrInventory | None = None) -> bool:
    """Return whether an explicit Herdr target is present in the inventory."""
    if not isinstance(target, dict) or str(target.get("provider", "")).strip().lower() != "herdr":
        return False
    metadata = target.get("metadata") if isinstance(target.get("metadata"), dict) else {}
    reference = target.get("executor_pane_id") or metadata.get("executor_pane_id") or target.get("reference")
    workspace_id = target.get("workspace_id") or metadata.get("workspace_id")
    if (
        not isinstance(reference, str)
        or not reference.strip()
        or not isinstance(workspace_id, str)
        or not workspace_id.strip()
    ):
        return False
    if inventory is None or inventory.error or inventory.current is None:
        return False
    return any(
        item.get("pane_id") == reference and item.get("workspace_id") == workspace_id
        for item in inventory.candidates
    )


def _transport_call(transport: Any, method: str, *args: Any) -> Any:
    operation = getattr(transport, method, None) if transport is not None else None
    if not callable(operation):
        raise RuntimeError(f"Herdr {method} transport is not available")
    return operation(*args)


def dispatch(unit_prompt: str, *, target: dict[str, Any], transport: Any = None) -> Any:
    """Dispatch one authorized unit through a supplied host transport."""
    if not isinstance(unit_prompt, str) or not unit_prompt.strip():
        raise ValueError("Herdr unit prompt must be non-empty")
    if not isinstance(target, dict) or str(target.get("provider", "")).lower() != "herdr":
        raise ValueError("Herdr dispatch requires an explicit Herdr target")
    return _transport_call(transport, "dispatch", target, unit_prompt)


def collect(*, target: dict[str, Any], transport: Any = None) -> Any:
    """Collect a result through the supplied host transport."""
    if not isinstance(target, dict) or str(target.get("provider", "")).lower() != "herdr":
        raise ValueError("Herdr collection requires an explicit Herdr target")
    return _transport_call(transport, "collect", target)
