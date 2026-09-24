"""Dedicated C2C reviewer binding resolution for Trellis.

This module owns only the local binding boundary. It does not send messages,
inspect ChatGPT, or reuse the ordinary C2C planning session URL. The host
transport verifies the normalized target separately before authorization.
"""

from __future__ import annotations

import copy
import json
import re
import subprocess
from datetime import datetime
from pathlib import Path
from typing import Any, Protocol
from urllib.parse import urlsplit

from .automation import AutomationContext, make_target, validate_target


class C2CReviewerBindingError(ValueError):
    """The dedicated reviewer binding is missing, malformed, or mismatched."""


class C2CReviewerBindingSource(Protocol):
    """Read-only source for the external C2C reviewer binding."""

    def read(self, repo_root: Path) -> Any:
        ...


class CommandC2CReviewerBindingSource:
    """Read a binding through the installed C2C CLI without using a shell."""

    def __init__(self, command: tuple[str, ...] = ("c2c",), *, timeout: float = 10.0):
        self.command = command
        self.timeout = timeout

    def read(self, repo_root: Path) -> Any:
        try:
            completed = subprocess.run(
                (*self.command, "reviewer", "get", "-w", str(repo_root), "--json"),
                cwd=str(repo_root),
                check=False,
                capture_output=True,
                text=True,
                timeout=self.timeout,
            )
        except (OSError, subprocess.SubprocessError) as exc:
            raise C2CReviewerBindingError(f"C2C reviewer binding query failed: {exc}") from exc
        if completed.returncode != 0:
            detail = completed.stderr.strip() or "c2c reviewer get returned a failure"
            raise C2CReviewerBindingError(detail)
        try:
            return json.loads(completed.stdout)
        except json.JSONDecodeError as exc:
            raise C2CReviewerBindingError("c2c reviewer get returned invalid JSON") from exc


def _normalize_project_url(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    try:
        parsed = urlsplit(value.strip())
    except ValueError:
        return None
    if parsed.scheme != "https" or parsed.hostname not in {"chatgpt.com", "www.chatgpt.com"}:
        return None
    match = re.fullmatch(r"/g/(g-p-[A-Za-z0-9]+)/project/?", parsed.path)
    return f"https://chatgpt.com/g/{match.group(1)}/project" if match else None


def _normalize_chat_url(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    try:
        parsed = urlsplit(value.strip())
    except ValueError:
        return None
    if parsed.scheme != "https" or parsed.hostname not in {"chatgpt.com", "www.chatgpt.com"}:
        return None
    match = re.fullmatch(r"/c/([A-Za-z0-9_-]+)/?", parsed.path)
    return f"https://chatgpt.com/c/{match.group(1)}" if match else None


def _valid_timestamp(value: Any) -> bool:
    if not isinstance(value, str) or not value.strip():
        return False
    try:
        datetime.fromisoformat(value.strip().replace("Z", "+00:00"))
    except ValueError:
        return False
    return True


def normalize_c2c_binding(
    payload: Any,
    *,
    expected_project_url: str | None = None,
    expected_connector_name: str | None = None,
) -> dict[str, Any]:
    """Normalize the external CLI response into a provider-neutral target."""

    if not isinstance(payload, dict):
        raise C2CReviewerBindingError("C2C reviewer binding response must be an object")
    if payload.get("ok") is False:
        raise C2CReviewerBindingError("C2C reviewer binding query was not successful")
    binding = payload.get("binding") if "binding" in payload else payload
    if not isinstance(binding, dict):
        raise C2CReviewerBindingError("C2C reviewer binding is missing")

    project_url = _normalize_project_url(binding.get("projectUrl"))
    chat_url = _normalize_chat_url(binding.get("chatUrl"))
    connector_name = binding.get("connectorName")
    if not project_url:
        raise C2CReviewerBindingError("C2C reviewer project identity is invalid")
    if not chat_url:
        raise C2CReviewerBindingError("C2C reviewer chat identity is invalid")
    if not isinstance(connector_name, str) or not connector_name.strip() or len(connector_name.strip()) > 200:
        raise C2CReviewerBindingError("C2C reviewer connector identity is invalid")
    if not _valid_timestamp(binding.get("boundAt")):
        raise C2CReviewerBindingError("C2C reviewer binding timestamp is invalid")

    if expected_project_url is not None:
        expected_project = _normalize_project_url(expected_project_url)
        if expected_project is None or expected_project != project_url:
            raise C2CReviewerBindingError("C2C reviewer project identity does not match the workspace")
    if expected_connector_name is not None and connector_name.strip() != expected_connector_name.strip():
        raise C2CReviewerBindingError("C2C reviewer connector identity does not match the workspace")

    title = binding.get("title")
    if title is not None and (not isinstance(title, str) or len(title.strip()) > 200):
        raise C2CReviewerBindingError("C2C reviewer title is invalid")
    label = title.strip() if isinstance(title, str) and title.strip() else "C2C web reviewer"
    return make_target(
        "c2c-web",
        chat_url,
        label=label,
        metadata={
            "project_url": project_url,
            "chat_url": chat_url,
            "connector_name": connector_name.strip(),
            "binding_source": "c2c-reviewer",
        },
    )


def resolve_reviewer_target(
    context: AutomationContext,
    repo_root: Path,
    *,
    current_turn: dict[str, Any] | None = None,
    binding_source: C2CReviewerBindingSource | None = None,
    expected_project_url: str | None = None,
    expected_connector_name: str | None = None,
) -> tuple[dict[str, Any], str]:
    """Resolve reviewer precedence without consulting the planning session URL."""

    if current_turn is not None:
        if not validate_target(current_turn):
            raise C2CReviewerBindingError("current-turn reviewer target is invalid")
        return copy.deepcopy(current_turn), "current-turn-explicit"
    if context.reviewer is not None:
        if not validate_target(context.reviewer):
            raise C2CReviewerBindingError("persisted reviewer target is invalid")
        return copy.deepcopy(context.reviewer), "persisted-explicit"

    source = binding_source or CommandC2CReviewerBindingSource()
    try:
        payload = source.read(Path(repo_root))
    except C2CReviewerBindingError:
        raise
    except Exception as exc:
        raise C2CReviewerBindingError(f"C2C reviewer binding source failed: {exc}") from exc
    return (
        normalize_c2c_binding(
            payload,
            expected_project_url=expected_project_url,
            expected_connector_name=expected_connector_name,
        ),
        "c2c-reviewer-binding",
    )
