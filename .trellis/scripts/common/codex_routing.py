"""Deprecated compatibility shim for the removed Codex routing state machine.

New code must use :mod:`common.automation` for conversation context and the
explicit ``automation_herdr`` / ``automation_dsh_web`` adapter modules for
provider transport. This module keeps a narrow import surface for one release
without retaining surface detection, dispatch policy, or provider election.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from .automation import (
    AutomationContext,
    clear_executor,
    clear_reviewer,
    load_context,
    make_target,
    save_context,
    set_executor,
    set_reviewer as _set_automation_reviewer,
)
from .automation_dsh_web import (
    DshWebInventory,
    available as dsh_web_available,
    collect as dsh_web_collect,
    dispatch as dsh_web_dispatch,
    discover_dsh_web,
    normalize_reference,
)
from .automation_herdr import (
    HerdrInventory,
    available as herdr_available,
    collect as herdr_collect,
    dispatch as herdr_dispatch,
    discover_herdr,
    parse_herdr_inventory,
)
from .active_task import resolve_context_key


class RoutingDeprecatedError(RuntimeError):
    """Raised when a removed routing-policy operation is requested."""


VALID_SURFACES = frozenset({"cli", "desktop", "unknown"})


def _deprecated(operation: str) -> None:
    raise RoutingDeprecatedError(
        f"{operation} is deprecated; use common.automation for explicit conversation context "
        "and an explicit provider adapter when requested"
    )


def empty_state(context_key: str) -> dict[str, Any]:
    """Return the new context shape for callers that still import this name."""

    return AutomationContext(context_key=context_key).to_dict()


def routing_path(repo_root: Path, context_key: str) -> Path:
    """Point legacy path callers at the new automation context path."""

    from .automation import context_path

    return context_path(repo_root, context_key)


def load_state(repo_root: Path, context_key: str) -> dict[str, Any]:
    """Load the conversation automation context, including one-way migration."""

    return load_context(repo_root, context_key).to_dict()


def resolve_codex_context_key() -> str:
    """Resolve the current conversation key for legacy CLI callers."""

    key = resolve_context_key(platform="codex")
    if not key:
        raise RuntimeError("cannot resolve the current Codex conversation identity")
    return key


def _context_from_state(state: AutomationContext | dict[str, Any]) -> AutomationContext:
    if isinstance(state, AutomationContext):
        return state
    if not isinstance(state, dict):
        raise ValueError("automation context must be an object")
    return AutomationContext.from_dict(state, state.get("context_key"))


def save_state(repo_root: Path, state: AutomationContext | dict[str, Any]) -> None:
    """Persist the new automation context through the legacy function name."""

    save_context(repo_root, _context_from_state(state))


def validate_state(data: Any, context_key: str) -> bool:
    """Validate the new automation context through the legacy function name."""

    from .automation import validate_context

    return validate_context(data, context_key)


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
    """Forward explicit target writes to the new context API.

    ``selected_by`` is accepted only for source compatibility; new context
    state intentionally does not persist legacy provenance.
    """

    del selected_by
    context = load_context(repo_root, context_key)
    if role == "executor":
        set_executor(context, provider, reference, label=label, metadata=metadata)
    elif role == "reviewer":
        _set_automation_reviewer(context, provider, reference, label=label, metadata=metadata)
    else:
        raise ValueError("role must be executor or reviewer")
    save_context(repo_root, context)
    return context.to_dict()


def set_reviewer(
    repo_root: Path,
    context_key: str,
    *,
    conversation_id: str,
    url: str = "",
    conversation_title: str = "",
    project_id: str = "",
    project_title: str = "",
) -> dict[str, Any]:
    """Forward the old ChatGPT reviewer helper to the generic target API."""

    metadata = {
        key: value
        for key, value in {
            "conversation_id": conversation_id,
            "url": url,
            "conversation_title": conversation_title,
            "project_id": project_id,
            "project_title": project_title,
        }.items()
        if value
    }
    return set_target(
        repo_root,
        context_key,
        "reviewer",
        "chatgpt",
        conversation_id,
        label=conversation_title or conversation_id,
        metadata=metadata or None,
    )


def invalidate(state: dict[str, Any], part: str) -> dict[str, Any]:
    """Clear a context slot in memory through the legacy helper name."""

    if part == "dispatch":
        _deprecated("dispatch invalidation")
    context = _context_from_state(state)
    if part == "executor":
        clear_executor(context)
    elif part == "reviewer":
        clear_reviewer(context)
    else:
        raise ValueError("part must be executor or reviewer")
    return context.to_dict()


def target_summary(target: dict[str, Any] | None) -> str:
    if not isinstance(target, dict):
        return "missing"
    return f"{target.get('provider')}:{target.get('reference')}"


# Removed surface/policy APIs intentionally fail closed rather than selecting a
# provider from host evidence or inventing a replacement target.
def get_codex_host_routes(repo_root: Path | None = None) -> dict[str, str]:
    del repo_root
    _deprecated("host route policy")
    return {}


def detect_surface(*args: Any, **kwargs: Any) -> None:
    del args, kwargs
    _deprecated("surface detection")


def resolve_codex_provider(*args: Any, **kwargs: Any) -> None:
    del args, kwargs
    _deprecated("provider policy resolution")


def set_surface(*args: Any, **kwargs: Any) -> None:
    del args, kwargs
    _deprecated("surface persistence")


def set_dispatch(*args: Any, **kwargs: Any) -> None:
    del args, kwargs
    _deprecated("dispatch policy")


def executor_validity(*args: Any, **kwargs: Any) -> None:
    del args, kwargs
    _deprecated("provider validity policy")


def reviewer_validity(*args: Any, **kwargs: Any) -> None:
    del args, kwargs
    _deprecated("reviewer validity policy")


def routing_missing_slots(*args: Any, **kwargs: Any) -> None:
    del args, kwargs
    _deprecated("routing slot election")


def selection_prompt(*args: Any, **kwargs: Any) -> None:
    del args, kwargs
    _deprecated("routing selection prompt")


__all__ = [
    "DshWebInventory",
    "HerdrInventory",
    "RoutingDeprecatedError",
    "VALID_SURFACES",
    "dsh_web_available",
    "dsh_web_collect",
    "dsh_web_dispatch",
    "discover_dsh_web",
    "discover_herdr",
    "herdr_available",
    "herdr_collect",
    "herdr_dispatch",
    "load_state",
    "make_target",
    "normalize_reference",
    "parse_herdr_inventory",
    "resolve_codex_context_key",
    "routing_path",
    "save_state",
    "set_reviewer",
    "set_target",
    "target_summary",
]
