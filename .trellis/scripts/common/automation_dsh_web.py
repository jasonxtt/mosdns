"""Explicit DSH Web browser-transport adapter.

The adapter keeps endpoint discovery and reference normalization available for
an explicitly selected target. It does not invoke discovery from hooks or
choose an endpoint from the current host. Browser I/O is supplied by the host
controller through the narrow transport argument.
"""

from __future__ import annotations

import re
import shutil
import subprocess
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class DshWebInventory:
    candidates: list[dict[str, Any]]
    error: str | None = None


def reference_from_command(command: str) -> tuple[str, int]:
    """Extract a browser endpoint from a DSH Web process command."""
    port_match = re.search(r"(?:^|\s)--port\s+(\d+)(?:\s|$)", command)
    port = int(port_match.group(1)) if port_match else 3080
    trusted_hosts = re.findall(r"(?:^|\s)--trusted-host\s+([^\s]+)", command)
    public_host = next(
        (host for host in trusted_hosts if host not in {"localhost", "127.0.0.1", "0.0.0.0"}),
        None,
    )
    if public_host:
        return f"https://{public_host.rstrip('/')}/", port
    return f"http://127.0.0.1:{port}/", port


def normalize_reference(value: Any) -> str:
    """Normalize endpoint references for stable candidate matching."""
    return str(value).strip().rstrip("/")


def discover_dsh_web(
    command: tuple[str, ...] = ("ps", "-axo", "pid=,command="),
) -> DshWebInventory:
    """Discover running DSH Web browser endpoints on explicit request."""
    try:
        completed = subprocess.run(command, check=True, capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError) as exc:
        return DshWebInventory([], f"DSH Web process discovery failed: {exc}")

    candidates: list[dict[str, Any]] = []
    seen: set[str] = set()
    for raw_line in completed.stdout.splitlines():
        line = raw_line.strip()
        if not line or not re.search(r"\bdsh\s+web\b", line):
            continue
        match = re.match(r"(\d+)\s+(.*)$", line)
        pid = int(match.group(1)) if match else None
        command_text = match.group(2) if match else line
        reference, port = reference_from_command(command_text)
        if reference in seen:
            continue
        seen.add(reference)
        candidates.append(
            {
                "provider": "dsh-web",
                "reference": reference,
                "label": f"DSH Web ({reference})",
                "pid": pid,
                "port": port,
                "transport": "browser-ui",
            }
        )

    if candidates:
        return DshWebInventory(candidates)
    if shutil.which("dsh"):
        return DshWebInventory([], "dsh is installed but no dsh web process is running")
    return DshWebInventory([], "dsh executable is unavailable")


def available(target: dict[str, Any] | None, inventory: DshWebInventory | None = None) -> bool:
    """Return whether an explicit browser endpoint is usable or well-formed."""
    if not isinstance(target, dict) or str(target.get("provider", "")).strip().lower() != "dsh-web":
        return False
    reference = target.get("reference")
    if not isinstance(reference, str) or not reference.strip():
        return False
    if inventory is None:
        return True
    if inventory.error and not inventory.candidates:
        return False
    normalized = normalize_reference(reference)
    return any(normalize_reference(item.get("reference")) == normalized for item in inventory.candidates)


def _transport_call(transport: Any, method: str, *args: Any) -> Any:
    operation = getattr(transport, method, None) if transport is not None else None
    if not callable(operation):
        raise RuntimeError(f"DSH Web {method} transport is not available")
    return operation(*args)


def dispatch(unit_prompt: str, *, target: dict[str, Any], transport: Any = None) -> Any:
    """Dispatch one authorized unit through a supplied browser transport."""
    if not isinstance(unit_prompt, str) or not unit_prompt.strip():
        raise ValueError("DSH Web unit prompt must be non-empty")
    if not available(target):
        raise ValueError("DSH Web dispatch requires an explicit endpoint target")
    return _transport_call(transport, "dispatch", target, unit_prompt)


def collect(*, target: dict[str, Any], transport: Any = None) -> Any:
    """Collect a result through the supplied browser transport."""
    if not available(target):
        raise ValueError("DSH Web collection requires an explicit endpoint target")
    return _transport_call(transport, "collect", target)
