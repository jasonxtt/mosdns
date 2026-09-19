"""Conversation-scoped Codex surface, executor, and reviewer routing."""

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
from .config import get_codex_dispatch_mode, get_codex_host_routes

STATE_VERSION = 2
VALID_SURFACES = {"cli", "desktop", "unknown"}
VALID_DISPATCH_MODES = {"auto", "ask", "codex", "dsh", "herdr", "inline"}
SUPPORTED_EXECUTOR_PROVIDERS = {"codex", "dsh", "herdr"}
SUPPORTED_REVIEWER_PROVIDERS = {"codex", "chatgpt"}


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
        "surface": None,
        "executor": None,
        "reviewer": None,
        "updated_at": _utc_now(),
    }


def _nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _valid_target(value: Any) -> bool:
    if value is None:
        return True
    if not isinstance(value, dict):
        return False
    if not _nonempty_string(value.get("provider")) or not _nonempty_string(value.get("reference")):
        return False
    for key in ("label", "selected_by"):
        if key in value and not isinstance(value[key], str):
            return False
    metadata = value.get("metadata")
    return metadata is None or isinstance(metadata, dict)


def _valid_surface(value: Any) -> bool:
    if value is None:
        return True
    return (
        isinstance(value, dict)
        and value.get("kind") in VALID_SURFACES
        and _nonempty_string(value.get("source"))
        and isinstance(value.get("evidence"), list)
        and all(isinstance(item, str) and item for item in value["evidence"])
    )


def validate_state(data: Any, context_key: str) -> bool:
    """Validate the v2 state shape without constraining future providers."""
    return (
        isinstance(data, dict)
        and data.get("version") == STATE_VERSION
        and data.get("platform") == "codex"
        and data.get("context_key") == context_key
        and _valid_surface(data.get("surface"))
        and _valid_target(data.get("executor"))
        and _valid_target(data.get("reviewer"))
    )


def _target(
    provider: str,
    reference: str,
    *,
    label: str = "",
    selected_by: str = "user",
    metadata: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if not _nonempty_string(provider) or not _nonempty_string(reference):
        raise ValueError("target provider and reference must be non-empty")
    value: dict[str, Any] = {
        "provider": provider.strip().lower(),
        "reference": reference.strip(),
        "label": label.strip() if isinstance(label, str) else "",
        "selected_by": selected_by.strip() if isinstance(selected_by, str) else "user",
    }
    if metadata:
        if set(metadata).intersection({"provider", "reference", "label", "selected_by"}):
            raise ValueError("target metadata cannot overwrite identity fields")
        value["metadata"] = dict(metadata)
        # Keep provider metadata readable to existing callers while identity
        # remains the provider/reference pair.
        value.update(metadata)
    return value


def _migrate_v1(data: dict[str, Any], context_key: str) -> dict[str, Any]:
    """Convert the old closed dispatch/reviewer shape in memory."""
    state = empty_state(context_key)
    if isinstance(data.get("updated_at"), str):
        state["updated_at"] = data["updated_at"]

    dispatch = data.get("dispatch")
    if isinstance(dispatch, dict):
        mode = dispatch.get("mode")
        selected_by = dispatch.get("selected_by", "migration")
        if mode == "inline":
            state["executor"] = _target(
                "codex",
                "current",
                label="Codex",
                selected_by=selected_by,
                metadata={"legacy_dispatch": dict(dispatch), "mode": "inline"},
            )
        elif mode == "herdr" and _nonempty_string(dispatch.get("workspace_id")) and _nonempty_string(
            dispatch.get("executor_pane_id")
        ):
            state["executor"] = _target(
                "herdr",
                dispatch["executor_pane_id"],
                label=f"Herdr {dispatch['workspace_id']}:{dispatch['executor_pane_id']}",
                selected_by=selected_by,
                metadata={
                    "legacy_dispatch": dict(dispatch),
                    "mode": "herdr",
                    "workspace_id": dispatch["workspace_id"],
                    "executor_pane_id": dispatch["executor_pane_id"],
                },
            )
        elif mode == "dsh" and _nonempty_string(dispatch.get("reference")):
            state["executor"] = _target(
                "dsh",
                dispatch["reference"],
                label=dispatch.get("label", "MCP DSH"),
                selected_by=selected_by,
                metadata={"legacy_dispatch": dict(dispatch)},
            )

    reviewer = data.get("reviewer")
    if isinstance(reviewer, dict) and _nonempty_string(reviewer.get("conversation_id")):
        metadata = {
            key: reviewer[key]
            for key in ("conversation_id", "url", "conversation_title", "project_id", "project_title")
            if key in reviewer
        }
        state["reviewer"] = _target(
            "chatgpt",
            reviewer["conversation_id"],
            label=reviewer.get("conversation_title", "") or reviewer["conversation_id"],
            selected_by=reviewer.get("selected_by", "migration"),
            metadata=metadata,
        )

    surface = data.get("surface")
    if _valid_surface(surface):
        state["surface"] = surface
    return state


def load_state(repo_root: Path, context_key: str) -> dict[str, Any]:
    path = routing_path(repo_root, context_key)
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, json.JSONDecodeError):
        return empty_state(context_key)
    if validate_state(data, context_key):
        return data
    if isinstance(data, dict) and data.get("version") == 1 and data.get("platform") == "codex":
        migrated = _migrate_v1(data, context_key)
        if validate_state(migrated, context_key):
            return migrated
    return empty_state(context_key)


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


def set_target(
    repo_root: Path,
    context_key: str,
    role: str,
    provider: str,
    reference: str,
    *,
    label: str = "",
    selected_by: str = "user",
    metadata: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if role not in {"executor", "reviewer"}:
        raise ValueError("role must be executor or reviewer")
    state = load_state(repo_root, context_key)
    state[role] = _target(
        provider,
        reference,
        label=label,
        selected_by=selected_by,
        metadata=metadata,
    )
    save_state(repo_root, state)
    return state


def set_surface(
    repo_root: Path,
    context_key: str,
    kind: str,
    *,
    source: str = "user_override",
    evidence: list[str] | tuple[str, ...] = (),
) -> dict[str, Any]:
    kind = str(kind).strip().lower()
    if kind not in VALID_SURFACES:
        raise ValueError(f"surface must be one of: {', '.join(sorted(VALID_SURFACES))}")
    state = load_state(repo_root, context_key)
    state["surface"] = {
        "kind": kind,
        "source": source,
        "evidence": [str(item) for item in evidence],
    }
    save_state(repo_root, state)
    return state


def set_dispatch(
    repo_root: Path,
    context_key: str,
    mode: str,
    *,
    workspace_id: str | None = None,
    executor_pane_id: str | None = None,
    reference: str | None = None,
) -> dict[str, Any]:
    """Backward-compatible wrapper for the pre-v2 dispatch API."""
    if mode not in VALID_DISPATCH_MODES:
        raise ValueError(f"unsupported dispatch mode: {mode}")
    if mode in {"inline", "codex"}:
        return set_target(
            repo_root,
            context_key,
            "executor",
            "codex",
            "current",
            label="Codex",
            metadata={"mode": "inline"},
        )
    if mode == "dsh":
        return set_target(
            repo_root,
            context_key,
            "executor",
            "dsh",
            reference or "provider-managed",
            label="MCP DSH",
            metadata={"mode": "dsh"},
        )
    if mode == "herdr":
        if not workspace_id or not executor_pane_id:
            raise ValueError("herdr mode requires workspace and executor pane")
        return set_target(
            repo_root,
            context_key,
            "executor",
            "herdr",
            executor_pane_id,
            label=f"Herdr {workspace_id}:{executor_pane_id}",
            metadata={
                "mode": "herdr",
                "workspace_id": workspace_id,
                "executor_pane_id": executor_pane_id,
            },
        )
    raise ValueError("auto and ask are policies, not concrete executor targets")


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
    """Backward-compatible wrapper for selecting a ChatGPT reviewer."""
    return set_target(
        repo_root,
        context_key,
        "reviewer",
        "chatgpt",
        conversation_id,
        label=conversation_title or conversation_id,
        metadata={
            "conversation_id": conversation_id,
            "url": url,
            "conversation_title": conversation_title,
            "project_id": project_id,
            "project_title": project_title,
        },
    )


def invalidate(state: dict[str, Any], part: str) -> dict[str, Any]:
    """Clear only the named routing slot; ``dispatch`` is a legacy alias."""
    if part == "dispatch":
        part = "executor"
    if part not in {"executor", "reviewer"}:
        raise ValueError("part must be executor or reviewer")
    state[part] = None
    return state


@dataclass(frozen=True)
class SurfaceEvidence:
    kind: str
    source: str
    evidence: tuple[str, ...]
    reason: str = ""

    def as_dict(self) -> dict[str, Any]:
        value: dict[str, Any] = {
            "kind": self.kind,
            "source": self.source,
            "evidence": list(self.evidence),
        }
        if self.reason:
            value["reason"] = self.reason
        return value


def _surface_value(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    normalized = value.strip().lower().replace("_", "-")
    aliases = {
        "app": "desktop",
        "codex-app": "desktop",
        "codex-desktop": "desktop",
        "codex-cli": "cli",
    }
    normalized = aliases.get(normalized, normalized)
    return normalized if normalized in VALID_SURFACES else None


def _truthy_marker(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    return str(value).strip().lower() not in {"", "0", "false", "no", "off", "none"}


def detect_surface(
    environ: dict[str, Any] | None = None,
    *,
    explicit: str | None = None,
) -> SurfaceEvidence:
    """Detect CLI/Desktop using explicit values and strong, name-only markers."""
    env = dict(os.environ if environ is None else environ)
    if explicit is not None:
        kind = _surface_value(explicit)
        if kind:
            return SurfaceEvidence(kind, "explicit_override", ("explicit_surface",))
        return SurfaceEvidence("unknown", "explicit_override", ("explicit_surface",), "invalid surface override")

    for marker in ("CODEX_SURFACE", "CODEX_HOST_SURFACE"):
        if marker in env:
            kind = _surface_value(env[marker])
            if kind:
                return SurfaceEvidence(kind, "env_surface", (marker,))
            return SurfaceEvidence("unknown", "env_surface", (marker,), "invalid host surface value")

    app_markers = tuple(
        marker
        for marker in ("CODEX_APP_TOOLS_PIPE_PATH", "CODEX_APP_TOOLS_PIPE")
        if _truthy_marker(env.get(marker))
    )
    cli_markers = tuple(
        marker
        for marker in ("CODEX_CLI_SURFACE", "CODEX_CLI")
        if _truthy_marker(env.get(marker))
    )
    evidence = app_markers + cli_markers
    if app_markers and cli_markers:
        return SurfaceEvidence("unknown", "env_marker", evidence, "conflicting host markers")
    if app_markers:
        return SurfaceEvidence("desktop", "env_marker", app_markers)
    if cli_markers:
        return SurfaceEvidence("cli", "env_marker", cli_markers)
    return SurfaceEvidence("unknown", "none", (), "no strong host marker")


def _surface_kind(surface: SurfaceEvidence | dict[str, Any] | str | None) -> str:
    if isinstance(surface, SurfaceEvidence):
        return surface.kind
    if isinstance(surface, dict):
        return str(surface.get("kind", "unknown"))
    if isinstance(surface, str):
        return _surface_value(surface) or "unknown"
    return "unknown"


def resolve_codex_provider(
    repo_root: Path,
    surface: SurfaceEvidence | dict[str, Any] | str | None,
    state: dict[str, Any] | None = None,
) -> str:
    """Resolve a provider class; never select a concrete target/resource.

    A valid conversation-scoped executor override has precedence over policy.
    An unsupported explicit provider returns ``unsupported`` rather than
    silently falling through to a host default.
    """
    if isinstance(state, dict):
        target = state.get("executor")
        if isinstance(target, dict) and _nonempty_string(target.get("provider")) and _nonempty_string(
            target.get("reference")
        ):
            provider = target["provider"].strip().lower()
            if provider in SUPPORTED_EXECUTOR_PROVIDERS:
                return provider
            return "unsupported"
    mode = get_codex_dispatch_mode(repo_root)
    if mode in {"codex", "inline"}:
        return "codex"
    if mode in {"herdr", "dsh", "ask"}:
        return mode
    if mode == "auto":
        return get_codex_host_routes(repo_root).get(_surface_kind(surface), "ask")
    return "ask"


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
        if item.get("workspace_id") == workspace and item.get("pane_id") != current.get("pane_id")
    ]
    return HerdrInventory(current, candidates)


def discover_herdr(command: tuple[str, ...] = ("herdr", "agent", "list")) -> HerdrInventory:
    try:
        completed = subprocess.run(command, check=True, capture_output=True, text=True, timeout=10)
        payload = json.loads(completed.stdout)
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError) as exc:
        return HerdrInventory(None, [], f"Herdr discovery failed: {exc}")
    return parse_herdr_inventory(payload, os.environ.get("HERDR_PANE_ID"))


def _executor_target(state: dict[str, Any]) -> dict[str, Any] | None:
    target = state.get("executor")
    if isinstance(target, dict):
        return target
    # Permit callers holding a pre-migration in-memory state to validate it.
    dispatch = state.get("dispatch")
    if isinstance(dispatch, dict):
        migrated = _migrate_v1(
            {
                "version": 1,
                "platform": "codex",
                "context_key": state.get("context_key", ""),
                "dispatch": dispatch,
                "reviewer": state.get("reviewer"),
            },
            str(state.get("context_key", "")),
        )
        return migrated.get("executor")
    return None


def executor_validity(inventory: HerdrInventory, state: dict[str, Any]) -> tuple[bool, str]:
    target = _executor_target(state)
    if target is None:
        return False, "executor selection is missing"
    provider = target.get("provider")
    reference = target.get("reference")
    if provider == "codex" and reference in {"current", "codex/current"}:
        return True, "current Codex was explicitly selected"
    if provider == "dsh" and _nonempty_string(reference):
        return True, f"MCP DSH target {reference} is selected"
    if provider != "herdr":
        return False, f"unsupported executor provider: {provider}"
    if inventory.error or inventory.current is None:
        return False, inventory.error or "current Herdr pane is unresolved"
    pane_id = target.get("executor_pane_id") or reference
    workspace_id = target.get("workspace_id")
    if not _nonempty_string(pane_id) or not _nonempty_string(workspace_id):
        return False, "selected Herdr target has no workspace or pane identity"
    match = next((item for item in inventory.candidates if item.get("pane_id") == pane_id), None)
    if match is None:
        return False, "selected executor pane is unavailable"
    if match.get("workspace_id") != workspace_id:
        return False, "selected executor moved to another workspace"
    return True, candidate_summary(match)


def reviewer_validity(state: dict[str, Any]) -> tuple[bool, str]:
    reviewer = state.get("reviewer")
    if reviewer is None:
        return False, "reviewer selection is missing"
    if not _valid_target(reviewer):
        return False, "reviewer target is malformed"
    if reviewer.get("provider") == "codex" and reviewer.get("reference") in {"current", "codex/current"}:
        return True, "current Codex was explicitly selected for review"
    if reviewer.get("provider") == "chatgpt" and reviewer.get("provider") in SUPPORTED_REVIEWER_PROVIDERS:
        return True, f"ChatGPT reviewer {reviewer.get('reference')} is selected"
    return False, f"unsupported reviewer provider: {reviewer.get('provider')}"


def routing_missing_slots(state: dict[str, Any]) -> list[str]:
    """Return missing or unsupported slots without selecting replacements."""
    missing: list[str] = []
    executor = _executor_target(state)
    executor_valid = False
    if _valid_target(executor):
        if executor is not None and executor.get("provider") == "codex":
            executor_valid = executor.get("reference") in {"current", "codex/current"}
        elif executor is not None and executor.get("provider") == "dsh":
            executor_valid = _nonempty_string(executor.get("reference"))
        elif executor is not None and executor.get("provider") == "herdr":
            executor_valid = _nonempty_string(executor.get("workspace_id")) and _nonempty_string(
                executor.get("executor_pane_id") or executor.get("reference")
            )
    if not executor_valid:
        missing.append("executor")
    reviewer = state.get("reviewer")
    reviewer_valid = False
    if _valid_target(reviewer):
        if reviewer is not None and reviewer.get("provider") == "codex":
            reviewer_valid = reviewer.get("reference") in {"current", "codex/current"}
        elif reviewer is not None and reviewer.get("provider") == "chatgpt":
            reviewer_valid = _nonempty_string(reviewer.get("reference"))
    if not reviewer_valid:
        missing.append("reviewer")
    return missing


def candidate_summary(item: dict[str, Any]) -> str:
    return " | ".join(
        str(item.get(key) or "unknown")
        for key in ("pane_id", "agent", "agent_status", "foreground_cwd", "terminal_title_stripped")
    )


def target_summary(target: dict[str, Any] | None) -> str:
    if not isinstance(target, dict):
        return "missing"
    return f"{target.get('provider')}:{target.get('reference')}"


def selection_prompt(
    inventory: HerdrInventory,
    state: dict[str, Any],
    *,
    surface: SurfaceEvidence | dict[str, Any] | str | None = None,
    provider: str | None = None,
) -> str:
    parts: list[str] = []
    selected_surface = surface or state.get("surface")
    surface_kind = _surface_kind(selected_surface)
    if "executor" in routing_missing_slots(state):
        resolved_provider = provider or "ask"
        parts.append(
            f"Executor selection is required for surface={surface_kind} (provider policy={resolved_provider}). "
            "Explicitly choose executor=codex for this conversation, or provide a provider/reference target."
        )
        if resolved_provider == "herdr":
            if inventory.error:
                parts.append(f"Herdr executor unresolved ({inventory.error}); choose inline or wait for Herdr.")
            elif not inventory.candidates:
                parts.append("No other pane exists in this Herdr workspace; choose inline or wait for another pane.")
            else:
                listed = "\n".join(f"- {candidate_summary(candidate)}" for candidate in inventory.candidates)
                parts.append("Choose one Herdr executor pane (no candidate is auto-selected):\n" + listed)
        elif resolved_provider == "dsh":
            parts.append("MCP DSH is the host default; choose its provider-managed reference explicitly. No worker is auto-selected.")
        elif resolved_provider == "ask":
            parts.append("Surface/policy is unresolved; choose the executor provider explicitly. No fallback is allowed.")
    if "reviewer" in routing_missing_slots(state):
        parts.append(
            "Choose one ChatGPT reviewer or explicitly set reviewer=codex for self-review: "
            "provide a provider/reference target, an existing conversation URL, a project chat, "
            "or a newly created user-selected chat."
        )
    return "\n\n".join(parts)


def resolve_codex_context_key() -> str:
    key = resolve_context_key(platform="codex")
    if not key:
        raise RuntimeError("cannot resolve the current Codex conversation identity")
    return key
