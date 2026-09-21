"""Durable, fail-closed state for an authorized automation run.

This module owns task authorization and progress only. Conversation-scoped
executor/reviewer choices remain in :mod:`common.automation`, and task.json
remains owned by ``task.py``. The run file is deliberately small enough to
reload after a context compaction while retaining the authorization snapshot,
current unit, and review submission placeholder needed by later slices.
"""

from __future__ import annotations

import copy
import json
import os
import re
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable

from .automation import load_context, validate_target


RUN_VERSION = 1
SNAPSHOT_VERSION = 1
RUN_STATUSES = {"running", "blocked", "authorized_scope_complete"}
UNIT_PHASES = {
    "pending",
    "implementing",
    "ready_for_review",
    "awaiting_review",
    "remediating",
    "passed",
}
REQUEST_KINDS = {"bootstrap", "rereview"}
DEFAULT_MAX_SAME_FINDING_ROUNDS = 5
_UNIT_HEADING = re.compile(r"^\s*##\s+(Slice\s+(\d+))\b")
_SAFE_KEY = re.compile(r"[^A-Za-z0-9._-]+")


class AutomationRunError(RuntimeError):
    """Base error for invalid or unsafe run transitions."""


class ActivationError(AutomationRunError):
    """The planning/reviewer/task activation gate is not satisfied."""


@dataclass
class AuthorizationSnapshot:
    """The pre-start authorization frozen before ``task.py start``."""

    context_key: str
    task: str
    authorized_units: list[str]
    authorized_at: str
    reviewer: dict[str, Any]
    reviewer_transport_evidence: dict[str, Any]
    version: int = SNAPSHOT_VERSION

    def to_dict(self) -> dict[str, Any]:
        return {
            "version": SNAPSHOT_VERSION,
            "context_key": self.context_key,
            "task": self.task,
            "authorized_units": list(self.authorized_units),
            "authorized_at": self.authorized_at,
            "reviewer": copy.deepcopy(self.reviewer),
            "reviewer_transport_evidence": copy.deepcopy(self.reviewer_transport_evidence),
        }

    @classmethod
    def from_dict(cls, value: Any) -> "AuthorizationSnapshot":
        if not validate_snapshot(value):
            raise ValueError("invalid authorization snapshot")
        return cls(
            context_key=value["context_key"],
            task=value["task"],
            authorized_units=list(value["authorized_units"]),
            authorized_at=value["authorized_at"],
            reviewer=copy.deepcopy(value["reviewer"]),
            reviewer_transport_evidence=copy.deepcopy(value["reviewer_transport_evidence"]),
        )


@dataclass
class AutomationRun:
    context_key: str
    task: str
    authorized_units: list[str]
    authorized_at: str
    current_unit: str | None
    reviewer_bootstrap_sent: bool = False
    auto_advance: bool = True
    auto_remediate: bool = True
    max_same_finding_rounds: int = DEFAULT_MAX_SAME_FINDING_ROUNDS
    auto_finish: bool = False
    status: str = "running"
    units: dict[str, dict[str, Any]] = field(default_factory=dict)
    blocked_reason: str | None = None
    version: int = RUN_VERSION

    def to_dict(self) -> dict[str, Any]:
        value: dict[str, Any] = {
            "version": RUN_VERSION,
            "context_key": self.context_key,
            "task": self.task,
            "authorized_units": list(self.authorized_units),
            "authorized_at": self.authorized_at,
            "current_unit": self.current_unit,
            "reviewer_bootstrap_sent": self.reviewer_bootstrap_sent,
            "auto_advance": self.auto_advance,
            "auto_remediate": self.auto_remediate,
            "max_same_finding_rounds": self.max_same_finding_rounds,
            "auto_finish": False,
            "status": self.status,
            "units": copy.deepcopy(self.units),
        }
        if self.blocked_reason:
            value["blocked_reason"] = self.blocked_reason
        return value

    @classmethod
    def from_dict(cls, value: Any) -> "AutomationRun":
        if not validate_run(value):
            raise ValueError("invalid automation run")
        return cls(
            context_key=value["context_key"],
            task=value["task"],
            authorized_units=list(value["authorized_units"]),
            authorized_at=value["authorized_at"],
            current_unit=value["current_unit"],
            reviewer_bootstrap_sent=value["reviewer_bootstrap_sent"],
            auto_advance=value["auto_advance"],
            auto_remediate=value["auto_remediate"],
            max_same_finding_rounds=value["max_same_finding_rounds"],
            auto_finish=False,
            status=value["status"],
            units=copy.deepcopy(value["units"]),
            blocked_reason=value.get("blocked_reason"),
            version=RUN_VERSION,
        )


def _utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _safe_key(value: str) -> str:
    key = _SAFE_KEY.sub("_", str(value)).strip("._-")
    if not key:
        raise ValueError("empty automation run context key")
    return key


def run_dir(repo_root: Path) -> Path:
    return Path(repo_root) / ".trellis" / ".runtime" / "automation" / "runs"


def run_path(repo_root: Path, context_key: str) -> Path:
    return run_dir(repo_root) / f"{_safe_key(context_key)}.json"


def authorization_dir(repo_root: Path) -> Path:
    return Path(repo_root) / ".trellis" / ".runtime" / "automation" / "authorizations"


def authorization_path(repo_root: Path, context_key: str) -> Path:
    return authorization_dir(repo_root) / f"{_safe_key(context_key)}.json"


def parse_implementation_units(task_dir: Path) -> list[str]:
    """Return numeric implementation Slice headings in their document order.

    Slice G is a pre-implementation feasibility gate and is intentionally not
    an executable authorization unit. Numeric headings are kept verbatim as
    the stable unit labels stored in the run snapshot.
    """

    path = Path(task_dir) / "implement.md"
    try:
        text = path.read_text(encoding="utf-8")
    except (FileNotFoundError, OSError, UnicodeDecodeError) as exc:
        raise ActivationError(f"cannot read implementation plan: {path}") from exc
    units: list[str] = []
    for line in text.splitlines():
        match = _UNIT_HEADING.match(line)
        if match and match.group(1) not in units:
            units.append(match.group(1))
    if not units:
        raise ActivationError(f"implementation plan has no numeric Slice headings: {path}")
    return units


def _select_units(available: list[str], selection: str | Iterable[str]) -> list[str]:
    if isinstance(selection, str):
        raw = selection.strip()
        if raw.lower() == "all":
            return list(available)
        range_match = re.fullmatch(r"slice\s+(\d+)\s*-\s*(?:slice\s*)?(\d+)", raw, re.IGNORECASE)
        if range_match:
            start, end = (int(item) for item in range_match.groups())
            if start > end:
                raise ActivationError("authorized Slice range must be ascending")
            requested = [f"Slice {number}" for number in range(start, end + 1)]
        else:
            requested = [item for item in re.split(r"\s*,\s*", raw) if item]
    else:
        requested = [str(item).strip() for item in selection if str(item).strip()]

    normalized: list[str] = []
    for item in requested:
        match = re.fullmatch(r"slice\s+(\d+)", item, re.IGNORECASE)
        canonical = f"Slice {match.group(1)}" if match else item
        if canonical not in available:
            raise ActivationError(f"requested authorization unit is not in implement.md: {item}")
        if canonical not in normalized:
            normalized.append(canonical)
    if not normalized:
        raise ActivationError("at least one authorization unit is required")
    return normalized


def _default_unit_state(phase: str = "pending") -> dict[str, Any]:
    return {
        "phase": phase,
        "submission": {
            "parent_sha": None,
            "head_sha": None,
            "review_round": 0,
            "request_kind": None,
            "submitted_to": None,
        },
        "findings": {},
        "result": None,
    }


def validate_run(value: Any) -> bool:
    if not isinstance(value, dict):
        return False
    if value.get("version") != RUN_VERSION:
        return False
    if not isinstance(value.get("context_key"), str) or not value["context_key"].strip():
        return False
    if not isinstance(value.get("task"), str) or not value["task"].strip():
        return False
    authorized = value.get("authorized_units")
    if not isinstance(authorized, list) or not authorized or any(not isinstance(item, str) or not item for item in authorized):
        return False
    if len(set(authorized)) != len(authorized):
        return False
    if not isinstance(value.get("authorized_at"), str) or not value["authorized_at"].strip():
        return False
    current = value.get("current_unit")
    if current is not None and current not in authorized:
        return False
    for key in ("reviewer_bootstrap_sent", "auto_advance", "auto_remediate"):
        if not isinstance(value.get(key), bool):
            return False
    if value.get("auto_finish") is not False:
        return False
    if value.get("status") not in RUN_STATUSES:
        return False
    if not isinstance(value.get("max_same_finding_rounds"), int) or value["max_same_finding_rounds"] < 1:
        return False
    units = value.get("units")
    if not isinstance(units, dict) or set(units) != set(authorized):
        return False
    for state in units.values():
        if not isinstance(state, dict) or state.get("phase") not in UNIT_PHASES:
            return False
        submission = state.get("submission")
        if not isinstance(submission, dict):
            return False
        if not isinstance(submission.get("review_round"), int) or submission["review_round"] < 0:
            return False
        if submission.get("request_kind") not in ({None} | REQUEST_KINDS):
            return False
        if submission.get("submitted_to") is not None and not (
            isinstance(submission["submitted_to"], str) or validate_target(submission["submitted_to"])
        ):
            return False
        if not isinstance(state.get("findings"), dict):
            return False
    if value["status"] == "authorized_scope_complete" and current is not None:
        return False
    if value.get("blocked_reason") is not None and not isinstance(value["blocked_reason"], str):
        return False
    allowed = {
        "version",
        "context_key",
        "task",
        "authorized_units",
        "authorized_at",
        "current_unit",
        "reviewer_bootstrap_sent",
        "auto_advance",
        "auto_remediate",
        "max_same_finding_rounds",
        "auto_finish",
        "status",
        "units",
        "blocked_reason",
    }
    return not (set(value) - allowed)


def validate_snapshot(value: Any) -> bool:
    if not isinstance(value, dict) or value.get("version") != SNAPSHOT_VERSION:
        return False
    if not isinstance(value.get("context_key"), str) or not value["context_key"].strip():
        return False
    if not isinstance(value.get("task"), str) or not value["task"].strip():
        return False
    units = value.get("authorized_units")
    if not isinstance(units, list) or not units or any(not isinstance(item, str) or not item for item in units):
        return False
    if len(set(units)) != len(units):
        return False
    if not isinstance(value.get("authorized_at"), str) or not value["authorized_at"].strip():
        return False
    if not validate_target(value.get("reviewer")) or value.get("reviewer") is None:
        return False
    evidence = value.get("reviewer_transport_evidence")
    if not isinstance(evidence, dict) or evidence.get("verified") is not True:
        return False
    if evidence.get("target") != value.get("reviewer"):
        return False
    if not isinstance(evidence.get("mechanism"), str) or not evidence["mechanism"].strip():
        return False
    if not isinstance(evidence.get("verified_at"), str) or not evidence["verified_at"].strip():
        return False
    allowed = {
        "version",
        "context_key",
        "task",
        "authorized_units",
        "authorized_at",
        "reviewer",
        "reviewer_transport_evidence",
    }
    return not (set(value) - allowed)


def _blocked_run(context_key: str, reason: str, task: str = "unknown") -> AutomationRun:
    return AutomationRun(
        context_key=context_key,
        task=task or "unknown",
        authorized_units=[],
        authorized_at=_utc_now(),
        current_unit=None,
        status="blocked",
        units={},
        blocked_reason=reason,
    )


def load_run(repo_root: Path, context_key: str) -> AutomationRun | None:
    """Load a run; malformed existing state becomes a blocked run.

    A missing file means no run exists. An existing but malformed file is not
    treated as absent, because doing so could silently bypass authorization or
    remediation limits after a crash or partial write.
    """

    path = run_path(repo_root, context_key)
    if not path.exists():
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        return _blocked_run(context_key, f"corrupt automation run: {exc}")
    try:
        return AutomationRun.from_dict(value)
    except ValueError:
        task = value.get("task") if isinstance(value, dict) and isinstance(value.get("task"), str) else "unknown"
        return _blocked_run(context_key, "corrupt automation run: invalid schema", task)


def save_run(repo_root: Path, run: AutomationRun) -> None:
    value = run.to_dict()
    if not validate_run(value):
        raise ValueError("invalid automation run")
    path = run_path(repo_root, run.context_key)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def load_authorization(repo_root: Path, context_key: str) -> AuthorizationSnapshot | None:
    """Load the pre-start snapshot; corruption fails closed."""

    path = authorization_path(repo_root, context_key)
    if not path.exists():
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
        return AuthorizationSnapshot.from_dict(value)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, ValueError) as exc:
        raise ActivationError(f"corrupt authorization snapshot: {exc}") from exc


def save_authorization(repo_root: Path, snapshot: AuthorizationSnapshot) -> None:
    value = snapshot.to_dict()
    if not validate_snapshot(value):
        raise ValueError("invalid authorization snapshot")
    path = authorization_path(repo_root, snapshot.context_key)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def reviewer_transport_evidence(
    target: dict[str, Any],
    *,
    mechanism: str,
    verified_at: str | None = None,
    probe_id: str | None = None,
) -> dict[str, Any]:
    """Build the durable evidence envelope supplied by a host-level probe."""

    if not validate_target(target) or target is None:
        raise ValueError("reviewer transport evidence requires a valid reviewer target")
    if not isinstance(mechanism, str) or not mechanism.strip():
        raise ValueError("reviewer transport evidence requires a mechanism")
    evidence: dict[str, Any] = {
        "verified": True,
        "target": copy.deepcopy(target),
        "mechanism": mechanism.strip(),
        "verified_at": verified_at or _utc_now(),
    }
    if probe_id:
        evidence["probe_id"] = probe_id
    return evidence


def _task_path(repo_root: Path, task_dir: Path | str) -> tuple[Path, str]:
    candidate = Path(task_dir)
    if not candidate.is_absolute():
        candidate = Path(repo_root) / candidate
    candidate = candidate.resolve()
    try:
        relative = candidate.relative_to(Path(repo_root).resolve()).as_posix()
    except ValueError as exc:
        raise ActivationError("task directory must be inside the repository") from exc
    return candidate, relative


def _task_status(task_dir: Path) -> str | None:
    try:
        value = json.loads((task_dir / "task.json").read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, UnicodeDecodeError, json.JSONDecodeError):
        return None
    return value.get("status") if isinstance(value, dict) else None


def authorize(
    repo_root: Path,
    task_dir: Path | str,
    units: str | Iterable[str],
    *,
    context_key: str,
    reviewer_transport_evidence: dict[str, Any] | None = None,
    transport_verifier: Callable[[dict[str, Any]], dict[str, Any]] | None = None,
) -> AuthorizationSnapshot | AutomationRun:
    """Freeze authorization before start, or activate it after start.

    The planning branch writes only an authorization snapshot. The
    post-start branch consumes that exact snapshot and never re-reads the
    current implementation plan or accepts a fresh verification assertion.
    """

    existing = load_run(repo_root, context_key)
    if existing is not None:
        if existing.status == "blocked":
            raise ActivationError(existing.blocked_reason or "automation run is blocked")
        _, task_relative = _task_path(repo_root, task_dir)
        if existing.task != task_relative:
            raise ActivationError("another automation run already exists for this context")
        return existing

    task_path, task_relative = _task_path(repo_root, task_dir)
    context = load_context(repo_root, context_key)
    if context.reviewer is None:
        raise ActivationError("reviewer target must be resolved before automation authorization")
    status = _task_status(task_path)
    if status == "planning":
        existing_snapshot = load_authorization(repo_root, context_key)
        if existing_snapshot is not None:
            if existing_snapshot.task != task_relative:
                raise ActivationError("another authorization snapshot already exists for this context")
            return existing_snapshot

        evidence = reviewer_transport_evidence
        if transport_verifier is not None:
            evidence = transport_verifier(context.reviewer)
        if not _valid_reviewer_evidence(evidence, context.reviewer):
            raise ActivationError("reviewer transport evidence must be verified before task.py start")
        authorized_units = _select_units(parse_implementation_units(task_path), units)
        snapshot = AuthorizationSnapshot(
            context_key=context_key,
            task=task_relative,
            authorized_units=authorized_units,
            authorized_at=_utc_now(),
            reviewer=copy.deepcopy(context.reviewer),
            reviewer_transport_evidence=copy.deepcopy(evidence),
        )
        save_authorization(repo_root, snapshot)
        return snapshot

    if status == "in_progress":
        return activate(repo_root, task_dir, context_key=context_key)
    raise ActivationError("task must be planning for authorization or in_progress for activation")


def _valid_reviewer_evidence(evidence: Any, target: dict[str, Any]) -> bool:
    if not isinstance(evidence, dict) or evidence.get("verified") is not True:
        return False
    if evidence.get("target") != target:
        return False
    return (
        isinstance(evidence.get("mechanism"), str)
        and bool(evidence["mechanism"].strip())
        and isinstance(evidence.get("verified_at"), str)
        and bool(evidence["verified_at"].strip())
    )


def activate(repo_root: Path, task_dir: Path | str, *, context_key: str) -> AutomationRun:
    """Create a run from the pre-start snapshot after task.py confirms start."""

    existing = load_run(repo_root, context_key)
    if existing is not None:
        return existing
    task_path, task_relative = _task_path(repo_root, task_dir)
    if _task_status(task_path) != "in_progress":
        raise ActivationError("task.py must confirm in_progress before run activation")
    snapshot = load_authorization(repo_root, context_key)
    if snapshot is None:
        raise ActivationError("pre-start authorization snapshot is missing")
    if snapshot.task != task_relative:
        raise ActivationError("authorization snapshot belongs to another task")
    current_reviewer = load_context(repo_root, context_key).reviewer
    if current_reviewer != snapshot.reviewer:
        raise ActivationError("reviewer target changed after authorization; explicit re-authorization is required")
    run = AutomationRun(
        context_key=context_key,
        task=snapshot.task,
        authorized_units=list(snapshot.authorized_units),
        authorized_at=snapshot.authorized_at,
        current_unit=snapshot.authorized_units[0],
        units={
            unit: _default_unit_state("implementing" if index == 0 else "pending")
            for index, unit in enumerate(snapshot.authorized_units)
        },
    )
    save_run(repo_root, run)
    return run


def _require_current(run: AutomationRun, unit: str | None) -> str:
    selected = unit or run.current_unit
    if selected is None or selected not in run.authorized_units:
        raise AutomationRunError("run has no selectable current unit")
    if run.current_unit != selected:
        raise AutomationRunError(f"unit is not current: {selected}")
    return selected


def advance(run: AutomationRun) -> AutomationRun:
    """Advance only across the snapshotted units, never from the plan live."""

    if run.status == "blocked":
        return run
    if run.status == "authorized_scope_complete":
        return run
    current = _require_current(run, None)
    if run.units[current]["phase"] != "passed":
        raise AutomationRunError("current unit must be passed before advancing")
    current_index = run.authorized_units.index(current)
    for unit in run.authorized_units[current_index + 1 :]:
        if run.units[unit]["phase"] == "pending":
            run.current_unit = unit
            run.units[unit]["phase"] = "implementing"
            return run
    if all(run.units[unit]["phase"] == "passed" for unit in run.authorized_units):
        run.current_unit = None
        run.status = "authorized_scope_complete"
    return run


def record_unit_result(
    run: AutomationRun,
    unit: str | None,
    passed: bool,
    *,
    result: Any = None,
) -> AutomationRun:
    """Record a unit result and auto-advance only after a PASS."""

    selected = _require_current(run, unit)
    run.units[selected]["phase"] = "passed" if passed else "remediating"
    run.units[selected]["result"] = copy.deepcopy(result if result is not None else {"status": "pass" if passed else "fail"})
    if passed and run.auto_advance:
        advance(run)
    return run


def record_pass(
    repo_root: Path,
    context_key: str,
    *,
    unit: str | None = None,
    result: Any = None,
) -> AutomationRun:
    run = load_run(repo_root, context_key)
    if run is None:
        raise AutomationRunError("no automation run exists")
    if run.status == "blocked":
        raise AutomationRunError(run.blocked_reason or "automation run is blocked")
    record_unit_result(run, unit, True, result=result)
    save_run(repo_root, run)
    return run


def record_fail(
    repo_root: Path,
    context_key: str,
    *,
    unit: str | None = None,
    result: Any = None,
) -> AutomationRun:
    run = load_run(repo_root, context_key)
    if run is None:
        raise AutomationRunError("no automation run exists")
    if run.status == "blocked":
        raise AutomationRunError(run.blocked_reason or "automation run is blocked")
    record_unit_result(run, unit, False, result=result)
    save_run(repo_root, run)
    return run


def complete(repo_root: Path, context_key: str) -> AutomationRun:
    """Persist the next transition after a previously recorded unit PASS."""

    run = load_run(repo_root, context_key)
    if run is None:
        raise AutomationRunError("no automation run exists")
    if run.status == "blocked":
        raise AutomationRunError(run.blocked_reason or "automation run is blocked")
    if run.status != "authorized_scope_complete":
        advance(run)
        save_run(repo_root, run)
    return run
