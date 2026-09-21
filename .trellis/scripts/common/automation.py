"""Conversation-scoped automation context and legacy-state migration.

This module deliberately owns only the small context shared by the current
conversation: an optional explicit executor override and a generic reviewer
target. Task lifecycle, authorization, and active review-run state belong to
separate layers and are not represented here.
"""

from __future__ import annotations

import copy
import hashlib
import json
import os
import re
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


AUTOMATION_VERSION = 1
CURRENT_EXECUTOR = "current"
SUPPORTED_MIGRATED_EXECUTORS = {"herdr", "dsh-web"}
_IDENTITY_FIELDS = {"provider", "reference", "label", "selected_by", "metadata"}
_SAFE_CONTEXT_KEY = re.compile(r"[^A-Za-z0-9._-]+")


def _utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _safe_context_key(context_key: str) -> str:
    safe = _SAFE_CONTEXT_KEY.sub("_", str(context_key)).strip("._-")
    if not safe:
        raise ValueError("empty automation context key")
    return safe


def automation_dir(repo_root: Path) -> Path:
    return Path(repo_root) / ".trellis" / ".runtime" / "automation"


def context_path(repo_root: Path, context_key: str) -> Path:
    return automation_dir(repo_root) / f"{_safe_context_key(context_key)}.json"


def legacy_routing_path(repo_root: Path, context_key: str) -> Path:
    return Path(repo_root) / ".trellis" / ".runtime" / "routing" / f"{_safe_context_key(context_key)}.json"


def make_target(
    provider: str,
    reference: str,
    *,
    label: str = "",
    metadata: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Create a generic provider/reference target.

    Adapter metadata is kept under its own key so it cannot overwrite the
    identity pair. Provenance is intentionally not part of the new target
    contract; it is used only while reading legacy state.
    """

    if not _nonempty_string(provider) or not _nonempty_string(reference):
        raise ValueError("target provider and reference must be non-empty")
    if not isinstance(label, str):
        raise ValueError("target label must be a string")
    if metadata is not None:
        if not isinstance(metadata, dict):
            raise ValueError("target metadata must be an object")
        if set(metadata).intersection(_IDENTITY_FIELDS):
            raise ValueError("target metadata cannot overwrite identity fields")

    target: dict[str, Any] = {
        "provider": provider.strip().lower(),
        "reference": reference.strip(),
    }
    if label.strip():
        target["label"] = label.strip()
    if metadata:
        target["metadata"] = copy.deepcopy(metadata)
    return target


def validate_target(value: Any) -> bool:
    """Return whether a value satisfies the generic target contract."""

    if value is None:
        return True
    if not isinstance(value, dict):
        return False
    if not _nonempty_string(value.get("provider")) or not _nonempty_string(value.get("reference")):
        return False
    if "label" in value and not isinstance(value["label"], str):
        return False
    if "selected_by" in value and not isinstance(value["selected_by"], str):
        return False
    metadata = value.get("metadata")
    if metadata is not None:
        if not isinstance(metadata, dict):
            return False
        if set(metadata).intersection(_IDENTITY_FIELDS):
            return False
    return True


def _valid_fingerprint(value: Any) -> bool:
    if not isinstance(value, dict):
        return False
    return _nonempty_string(value.get("path")) and isinstance(value.get("sha256"), str) and re.fullmatch(
        r"[0-9a-fA-F]{64}", value["sha256"]
    ) is not None


def validate_context(data: Any, context_key: str | None = None) -> bool:
    """Validate the persisted v1 conversation automation context."""

    if not isinstance(data, dict) or data.get("version") != AUTOMATION_VERSION:
        return False
    stored_key = data.get("context_key")
    if not _nonempty_string(stored_key):
        return False
    if context_key is not None and stored_key != context_key:
        return False
    if not validate_target(data.get("executor_override")):
        return False
    if not validate_target(data.get("reviewer")):
        return False
    if "updated_at" in data and not isinstance(data["updated_at"], str):
        return False
    if "migrated_from" in data and data["migrated_from"] is not None:
        if not _valid_fingerprint(data["migrated_from"]):
            return False
    allowed = {
        "version",
        "context_key",
        "executor_override",
        "reviewer",
        "updated_at",
        "migrated_from",
    }
    return not (set(data) - allowed)


@dataclass
class AutomationContext:
    """The conversation-scoped automation context persisted by Slice 0."""

    context_key: str
    executor_override: dict[str, Any] | None = None
    reviewer: dict[str, Any] | None = None
    updated_at: str = field(default_factory=_utc_now)
    migrated_from: dict[str, str] | None = None
    version: int = AUTOMATION_VERSION

    def __post_init__(self) -> None:
        _safe_context_key(self.context_key)

    @classmethod
    def from_dict(cls, data: dict[str, Any], context_key: str | None = None) -> "AutomationContext":
        if not validate_context(data, context_key):
            raise ValueError("invalid automation context")
        return cls(
            context_key=str(data["context_key"]),
            executor_override=copy.deepcopy(data.get("executor_override")),
            reviewer=copy.deepcopy(data.get("reviewer")),
            updated_at=data.get("updated_at") or _utc_now(),
            migrated_from=copy.deepcopy(data.get("migrated_from")),
            version=AUTOMATION_VERSION,
        )

    def to_dict(self) -> dict[str, Any]:
        value: dict[str, Any] = {
            "version": AUTOMATION_VERSION,
            "context_key": self.context_key,
            "executor_override": copy.deepcopy(self.executor_override),
            "reviewer": copy.deepcopy(self.reviewer),
            "updated_at": self.updated_at,
        }
        if self.migrated_from is not None:
            value["migrated_from"] = copy.deepcopy(self.migrated_from)
        return value

    @classmethod
    def load(cls, repo_root: Path, context_key: str) -> "AutomationContext":
        return load_context(repo_root, context_key)

    def save(self, repo_root: Path) -> None:
        save_context(repo_root, self)


def empty_context(context_key: str) -> AutomationContext:
    return AutomationContext(context_key=_safe_context_key(context_key))


def resolve_executor(context: AutomationContext | dict[str, Any]) -> str | dict[str, Any]:
    """Resolve the effective executor without persisting the current target."""

    override = context.executor_override if isinstance(context, AutomationContext) else context.get("executor_override")
    if isinstance(override, dict) and validate_target(override):
        return copy.deepcopy(override)
    return CURRENT_EXECUTOR


def set_executor(
    context: AutomationContext,
    provider: str,
    reference: str | None = None,
    *,
    label: str = "",
    metadata: dict[str, Any] | None = None,
) -> AutomationContext:
    """Set an explicit executor, or restore the implicit current executor."""

    normalized_provider = provider.strip().lower() if isinstance(provider, str) else ""
    normalized_reference = reference.strip() if isinstance(reference, str) else ""
    if normalized_provider in {"current", "codex"} and normalized_reference in {"", "current", "codex", "codex/current"}:
        context.executor_override = None
    else:
        context.executor_override = make_target(
            normalized_provider,
            normalized_reference,
            label=label,
            metadata=metadata,
        )
    return context


def clear_executor(context: AutomationContext) -> AutomationContext:
    context.executor_override = None
    return context


def set_reviewer(
    context: AutomationContext,
    provider: str,
    reference: str,
    *,
    label: str = "",
    metadata: dict[str, Any] | None = None,
) -> AutomationContext:
    context.reviewer = make_target(provider, reference, label=label, metadata=metadata)
    return context


def clear_reviewer(context: AutomationContext) -> AutomationContext:
    context.reviewer = None
    return context


def _read_json(path: Path) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, UnicodeDecodeError, json.JSONDecodeError):
        return None
    return value if isinstance(value, dict) else None


def save_context(repo_root: Path, context: AutomationContext) -> None:
    """Atomically save a validated context under the ignored runtime tree."""

    if not isinstance(context, AutomationContext) or not validate_context(context.to_dict(), context.context_key):
        raise ValueError("invalid automation context")
    context.updated_at = _utc_now()
    path = context_path(repo_root, context.context_key)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        temporary.write_text(
            json.dumps(context.to_dict(), indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def _load_existing_context(repo_root: Path, context_key: str) -> AutomationContext | None:
    path = context_path(repo_root, context_key)
    if not path.exists():
        return None
    data = _read_json(path)
    if data is None or not validate_context(data, context_key):
        return empty_context(context_key)
    return AutomationContext.from_dict(data, context_key)


def load_context(repo_root: Path, context_key: str) -> AutomationContext:
    """Load context, lazily migrating a legacy file only when no new file exists."""

    existing = _load_existing_context(repo_root, context_key)
    if existing is not None:
        return existing
    return migrate_legacy_routing(repo_root, context_key)


def _legacy_target_data(value: dict[str, Any], *, provider: str | None = None, reference: str | None = None) -> tuple[str, str, str, dict[str, Any]] | None:
    target_provider = provider if provider is not None else value.get("provider")
    target_reference = reference if reference is not None else value.get("reference")
    if not _nonempty_string(target_provider) or not _nonempty_string(target_reference):
        return None
    metadata: dict[str, Any] = {}
    raw_metadata = value.get("metadata")
    if isinstance(raw_metadata, dict):
        metadata.update(copy.deepcopy(raw_metadata))
    for key, item in value.items():
        if key not in _IDENTITY_FIELDS and key not in {"metadata", "conversation_id"}:
            metadata.setdefault(key, copy.deepcopy(item))
    label = value.get("label") if isinstance(value.get("label"), str) else ""
    if not label and isinstance(value.get("conversation_title"), str):
        label = value["conversation_title"]
    return str(target_provider), str(target_reference), label, metadata


def _migrate_reviewer(value: Any, *, legacy_version: int) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    selected_by = value.get("selected_by")
    if legacy_version == 1 and selected_by != "user":
        # A v1 reviewer without explicit provenance was only selected by the
        # old migration/policy path and must not silently become a binding.
        return None
    if selected_by != "user":
        return None
    reference = value.get("reference") or value.get("conversation_id")
    provider = value.get("provider") or "chatgpt"
    migrated = _legacy_target_data(value, provider=str(provider), reference=reference)
    if migrated is None or str(provider).strip().lower() == "dsh":
        return None
    target_provider, target_reference, label, metadata = migrated
    return make_target(target_provider, target_reference, label=label, metadata=metadata or None)


def _migrate_executor(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    if value.get("selected_by") != "user":
        return None
    migrated = _legacy_target_data(value)
    if migrated is None:
        return None
    provider, reference, label, metadata = migrated
    provider = provider.strip().lower()
    if provider in {"codex", "current"} and reference.strip().lower() in {"current", "codex/current", "codex"}:
        return None
    if provider not in SUPPORTED_MIGRATED_EXECUTORS:
        return None
    return make_target(provider, reference, label=label, metadata=metadata or None)


def _migrate_v1(data: dict[str, Any]) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    dispatch = data.get("dispatch")
    executor: dict[str, Any] | None = None
    if isinstance(dispatch, dict) and dispatch.get("selected_by") == "user":
        mode = str(dispatch.get("mode", "")).strip().lower()
        if mode == "herdr" and _nonempty_string(dispatch.get("workspace_id")) and _nonempty_string(
            dispatch.get("executor_pane_id")
        ):
            executor = make_target(
                "herdr",
                str(dispatch["executor_pane_id"]),
                label=f"Herdr {dispatch['workspace_id']}:{dispatch['executor_pane_id']}",
                metadata={
                    "workspace_id": dispatch["workspace_id"],
                    "executor_pane_id": dispatch["executor_pane_id"],
                },
            )
        elif mode == "dsh-web" and _nonempty_string(dispatch.get("reference")):
            executor = make_target("dsh-web", str(dispatch["reference"]), label="DSH Web")
    # v1 inline/codex is the implicit current executor; retired MCP dsh is
    # intentionally discarded.
    reviewer = _migrate_reviewer(data.get("reviewer"), legacy_version=1)
    return executor, reviewer


def migrate_legacy_routing(repo_root: Path, context_key: str) -> AutomationContext:
    """Migrate one legacy routing file without modifying or renaming it."""

    existing = _load_existing_context(repo_root, context_key)
    if existing is not None:
        return existing

    legacy_path = legacy_routing_path(repo_root, context_key)
    try:
        raw = legacy_path.read_bytes()
    except (FileNotFoundError, OSError):
        return empty_context(context_key)

    context = empty_context(context_key)
    context.migrated_from = {
        "path": legacy_path.relative_to(Path(repo_root)).as_posix(),
        "sha256": hashlib.sha256(raw).hexdigest(),
    }

    try:
        data = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        save_context(repo_root, context)
        return context

    if not isinstance(data, dict):
        save_context(repo_root, context)
        return context

    version = data.get("version")
    if version == 1:
        context.executor_override, context.reviewer = _migrate_v1(data)
    elif version == 2:
        context.executor_override = _migrate_executor(data.get("executor"))
        context.reviewer = _migrate_reviewer(data.get("reviewer"), legacy_version=2)

    save_context(repo_root, context)
    return context


# Explicit aliases make the storage boundary easy to discover without
# reintroducing the old routing module as a dependency.
load_automation_context = load_context
save_automation_context = save_context
