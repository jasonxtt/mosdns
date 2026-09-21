"""Provider-neutral reviewer transport and review-loop protocol helpers.

The repository owns only the narrow contract and durable review bookkeeping.
The host supplies the platform-native ChatGPT conversation capability through
an injected adapter; this module contains no HTTP client, browser automation,
or provider discovery.
"""

from __future__ import annotations

import copy
import re
from dataclasses import dataclass
from typing import Any, Protocol

from .automation import validate_target
from .automation_run import AutomationRun, AutomationRunError, save_run


class ReviewerTransportUnavailable(RuntimeError):
    """The host did not provide a usable reviewer transport adapter."""


class ReviewerTransport(Protocol):
    """Platform-native reviewer boundary supplied by the host controller."""

    def send(self, request: Any) -> Any:
        ...

    def wait_result(self, timeout: float) -> Any:
        ...

    def read(self) -> Any:
        ...


def _utc_now() -> str:
    from datetime import datetime, timezone

    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def verify_reviewer_transport(target: dict[str, Any] | None, transport: Any) -> dict[str, Any]:
    """Verify a selected target through an injected host-native adapter.

    A transport must expose ``verify_target`` so the controller cannot turn a
    mere object shape or a user-supplied boolean into durable proof. The
    returned envelope is suitable for Slice 2's pre-start snapshot.
    """

    if not isinstance(target, dict) or not validate_target(target):
        raise ReviewerTransportUnavailable("reviewer target is malformed")
    verifier = getattr(transport, "verify_target", None) if transport is not None else None
    if not callable(verifier):
        raise ReviewerTransportUnavailable("platform-native reviewer transport is unavailable")
    try:
        result = verifier(copy.deepcopy(target))
    except Exception as exc:  # host boundary: convert adapter failures to one stable error
        raise ReviewerTransportUnavailable(f"reviewer transport verification failed: {exc}") from exc
    if result is False or not isinstance(result, dict):
        raise ReviewerTransportUnavailable("reviewer transport target could not be verified")
    mechanism = result.get("mechanism")
    if not isinstance(mechanism, str) or not mechanism.strip():
        raise ReviewerTransportUnavailable("reviewer transport verification has no mechanism")
    evidence: dict[str, Any] = {
        "verified": True,
        "target": copy.deepcopy(target),
        "mechanism": mechanism.strip(),
        "verified_at": result.get("verified_at") or _utc_now(),
    }
    if result.get("probe_id"):
        evidence["probe_id"] = str(result["probe_id"])
    return evidence


def send_review_request(transport: ReviewerTransport, request: Any) -> Any:
    send = getattr(transport, "send", None)
    if not callable(send):
        raise ReviewerTransportUnavailable("reviewer transport does not implement send(request)")
    try:
        return send(request)
    except Exception as exc:
        raise ReviewerTransportUnavailable(f"reviewer send failed: {exc}") from exc


def read_review_round_trip(transport: ReviewerTransport, request: Any, *, timeout: float) -> Any:
    """Send, bounded-wait, then read one reviewer response."""

    send_review_request(transport, request)
    wait = getattr(transport, "wait_result", None)
    read = getattr(transport, "read", None)
    if not callable(wait) or not callable(read):
        raise ReviewerTransportUnavailable("reviewer transport must implement wait_result/read")
    try:
        readiness = wait(timeout)
        if readiness in (False, None, "pending", "idle"):
            return {"status": "pending"}
        return read()
    except Exception as exc:
        raise ReviewerTransportUnavailable(f"reviewer read failed: {exc}") from exc


_FINAL = re.compile(r"(?im)^\s*FINAL\s*:\s*(PASS|FAIL)\s*$")
_FINDING = re.compile(
    r"(?im)^\s*(P[0-3]-\d+)\b\s*(?:[-—:]\s*)?(.*?)(?:\s+\[(out[- ]of[- ]scope)\])?\s*$"
)


def _finding_status(text: str) -> str:
    return "closed" if re.search(r"(?i)\bclosed\b", text) else "open"


def parse_review_result(text: Any) -> dict[str, Any]:
    """Parse only explicit final verdicts; pending text can never pass."""

    if not isinstance(text, str):
        return {"status": "pending", "findings": [], "raw": text}
    final = _FINAL.search(text)
    findings: list[dict[str, Any]] = []
    for match in _FINDING.finditer(text):
        root_cause = match.group(2).strip()
        findings.append(
            {
                "id": match.group(1),
                "root_cause": root_cause or match.group(1),
                "status": _finding_status(root_cause),
                "out_of_scope": bool(match.group(3)),
            }
        )
    if final is None:
        return {"status": "pending", "findings": findings, "raw": text}
    status = "pass" if final.group(1).upper() == "PASS" else "fail"
    return {"status": status, "findings": findings, "raw": text}


def _field(evidence: dict[str, Any], key: str, default: str = "(not supplied)") -> str:
    value = evidence.get(key, default)
    if isinstance(value, (list, tuple, set)):
        return "\n".join(f"- {item}" for item in value) or default
    if isinstance(value, dict):
        return "\n".join(f"- {name}: {item}" for name, item in value.items()) or default
    return str(value)


def _reviewer_label(target: Any) -> str:
    if not isinstance(target, dict):
        return "missing"
    return f"{target.get('provider')}:{target.get('reference')}"


def build_review_request(
    run: AutomationRun,
    unit: str,
    evidence: dict[str, Any],
) -> dict[str, Any]:
    """Build a self-contained bootstrap or compact re-review payload."""

    if unit not in run.authorized_units:
        raise AutomationRunError(f"review unit is not authorized: {unit}")
    if not isinstance(evidence, dict):
        raise ValueError("review evidence must be an object")
    state = run.units[unit]
    submission = state.get("submission", {})
    kind = "rereview" if run.reviewer_bootstrap_sent else "bootstrap"
    common = [
        f"Repository: {_field(evidence, 'repository')}",
        f"Branch: {_field(evidence, 'branch')}",
        f"Active Trellis task: {run.task}",
        f"Current authorized unit: {unit}",
        f"Reviewer target: {_reviewer_label(evidence.get('reviewer'))}",
        f"Base full SHA: {_field(evidence, 'base_sha')}",
        f"Head full SHA: {_field(evidence, 'head_sha')}",
        f"GitHub commit/tree reference: {_field(evidence, 'github_reference')}",
        "Exact changed paths:\n" + _field(evidence, "changed_paths"),
        "Validation evidence:\n" + _field(evidence, "validation"),
        "Current unit acceptance criteria:\n" + _field(evidence, "acceptance"),
        "Forbidden scope:\n" + _field(evidence, "forbidden_scope"),
    ]
    if kind == "bootstrap":
        text = "\n\n".join(
            [
                "You are the independent root reviewer for this Trellis task.",
                f"Task goal: {_field(evidence, 'task_goal')}",
                "Scope/contracts:\n" + _field(evidence, "scope_contracts"),
                "User-authorized unit range:\n" + ", ".join(run.authorized_units),
                "Automation contract: review only the submitted unit; scoped FAILs are automatically remediated and resubmitted; PASS authorizes the controller to enter only the next pre-authorized Slice; PASS does not authorize work beyond the original range; final Slice PASS does not authorize archive/finish, a new task, production wiring, deployment, or unrelated scope.",
                "Finding output contract: assign stable IDs such as P1-1 and P1-2; keep the same ID for the same substantive finding on every re-review; mark each finding explicitly open or closed; assign a new unused ID only to a genuinely new finding. Renaming an ID or rephrasing a root cause must not be used to evade the controller's same-root-cause remediation counter.",
                "\n\n".join(common),
                "Return an explicit FINAL: PASS or FINAL: FAIL. Pending, idle, silence, or partial responses are not PASS.",
            ]
        )
    else:
        previous = state.get("findings") or {}
        text = "\n\n".join(
            [
                "Compact re-review for the same Trellis task and unit.",
                f"Previous finding ledger:\n{_field({'findings': previous}, 'findings')}",
                f"Remediation parent/head: {submission.get('parent_sha')} -> {submission.get('head_sha')}",
                "Exact remediation diff scope:\n" + _field(evidence, "changed_paths"),
                "Validation evidence:\n" + _field(evidence, "validation"),
                "Review only this unit and the listed findings. Return an explicit FINAL: PASS or FINAL: FAIL; do not expand scope.",
            ]
        )
    return {"kind": kind, "task": run.task, "unit": unit, "text": text}


def submit_review(
    run: AutomationRun,
    unit: str,
    *,
    parent_sha: str,
    head_sha: str,
    submitted_to: dict[str, Any],
    request_kind: str | None = None,
) -> AutomationRun:
    """Pin one review request to exact Git SHAs and reviewer identity."""

    if unit not in run.authorized_units or run.current_unit != unit:
        raise AutomationRunError(f"review unit is not current and authorized: {unit}")
    if not parent_sha or not head_sha or not validate_target(submitted_to):
        raise ValueError("review submission requires parent/head SHA and reviewer target")
    expected_kind = "bootstrap" if not run.reviewer_bootstrap_sent else "rereview"
    kind = request_kind or expected_kind
    if kind not in {"bootstrap", "rereview"}:
        raise ValueError("request kind must be bootstrap or rereview")
    if kind != expected_kind:
        raise AutomationRunError(f"review request kind must be {expected_kind}")
    previous = run.units[unit]["submission"]
    if kind == "rereview":
        if not previous.get("head_sha") or parent_sha != previous["head_sha"]:
            raise AutomationRunError("rereview parent must equal the previous review head")
        open_findings = _open_findings(run.units[unit])
        if not all(
            _remediation_ready_for_previous_submission(finding, previous)
            and finding["remediation"].get("parent_sha") == parent_sha
            and finding["remediation"].get("head_sha") == head_sha
            for finding in open_findings
        ):
            raise AutomationRunError(
                "rereview requires recorded remediation evidence for every open finding"
            )
    review_round = 0 if kind == "bootstrap" else int(previous.get("review_round", 0)) + 1
    run.units[unit]["submission"] = {
        "parent_sha": parent_sha,
        "head_sha": head_sha,
        "review_round": review_round,
        "request_kind": kind,
        "submitted_to": copy.deepcopy(submitted_to),
    }
    run.units[unit]["phase"] = "awaiting_review"
    run.units[unit]["review_result_recorded"] = False
    if kind == "bootstrap":
        run.reviewer_bootstrap_sent = True
    return run


def _normalize_root_cause(value: Any) -> str:
    text = re.sub(r"\s+", " ", str(value or "").strip().lower())
    return re.sub(r"^p[0-3]-\d+\s*[-:—]?\s*", "", text)


def _root_cause_key(finding: dict[str, Any]) -> str | None:
    value = finding.get("root_cause_key") or finding.get("semantic_root_cause")
    if value is None or not str(value).strip():
        return None
    return str(value).strip()


def _find_existing(ledger: dict[str, Any], finding: dict[str, Any]) -> tuple[str | None, dict[str, Any] | None]:
    finding_id = str(finding.get("id") or "")
    semantic_key = _root_cause_key(finding)
    if finding_id in ledger:
        current = ledger[finding_id]
        current_key = _root_cause_key(current)
        if current_key is None and semantic_key is None:
            return finding_id, current
        if semantic_key is not None and (current_key is None or semantic_key == current_key):
            return finding_id, current
        # A stable reviewer ID is not enough to conflate two controller-judged
        # semantic roots. Let the caller allocate a distinct ledger key.
        return None, None
    if semantic_key is not None:
        for key, current in ledger.items():
            if _root_cause_key(current) == semantic_key:
                return key, current
    return None, None


def _new_finding_key(ledger: dict[str, Any], requested: str) -> str:
    base = requested or f"finding-{len(ledger) + 1}"
    candidate = base
    suffix = 2
    while candidate in ledger:
        candidate = f"{base}#{suffix}"
        suffix += 1
    return candidate


def _open_findings(state: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        finding
        for finding in state.get("findings", {}).values()
        if isinstance(finding, dict) and finding.get("status") == "open"
    ]


def _remediation_ready_for_previous_submission(
    finding: dict[str, Any],
    previous_submission: dict[str, Any],
) -> bool:
    remediation = finding.get("remediation")
    previous_head = previous_submission.get("head_sha")
    return (
        isinstance(remediation, dict)
        and remediation.get("executed") is True
        and remediation.get("consumed") is False
        and remediation.get("parent_sha") == previous_head
        and isinstance(remediation.get("head_sha"), str)
        and remediation["head_sha"] != previous_head
    )


def _remediation_ready_for_current_submission(
    finding: dict[str, Any],
    submission: dict[str, Any],
) -> bool:
    remediation = finding.get("remediation")
    return (
        isinstance(remediation, dict)
        and remediation.get("executed") is True
        and remediation.get("consumed") is False
        and remediation.get("head_sha") == submission.get("head_sha")
        and remediation.get("parent_sha") == submission.get("parent_sha")
    )


def record_remediation(
    run: AutomationRun,
    unit: str,
    *,
    parent_sha: str,
    head_sha: str,
    finding_id: str | None = None,
    root_cause_key: str | None = None,
    summary: str = "",
) -> AutomationRun:
    """Record controller evidence for a new remediation commit.

    The evidence is attached to one open semantic finding and must be based on
    the previous review head. A later rereview consumes it exactly once.
    """

    if unit not in run.authorized_units or run.current_unit != unit:
        raise AutomationRunError(f"review unit is not current and authorized: {unit}")
    if not parent_sha or not head_sha or parent_sha == head_sha:
        raise AutomationRunError("remediation evidence requires distinct parent/head SHA")
    state = run.units[unit]
    previous_head = state.get("submission", {}).get("head_sha")
    if not previous_head or parent_sha != previous_head:
        raise AutomationRunError("remediation parent must equal the previous review head")
    query: dict[str, Any] = {}
    if finding_id:
        query["id"] = finding_id
    if root_cause_key:
        query["root_cause_key"] = root_cause_key
    key, finding = _find_existing(state.get("findings", {}), query)
    if finding is None or finding.get("status") != "open":
        raise AutomationRunError("remediation evidence must target one open finding")
    existing = finding.get("remediation")
    if isinstance(existing, dict) and existing.get("head_sha") == head_sha and not existing.get("consumed", False):
        raise AutomationRunError("remediation evidence is already recorded for this finding and head")
    finding["remediation"] = {
        "executed": True,
        "parent_sha": parent_sha,
        "head_sha": head_sha,
        "root_cause_key": _root_cause_key(finding),
        "summary": str(summary),
        "consumed": False,
    }
    state["phase"] = "remediating"
    return run


def record_review_result(
    run: AutomationRun,
    unit: str,
    result: dict[str, Any],
) -> AutomationRun:
    """Apply a parsed reviewer result and enforce per-finding limits."""

    if unit not in run.authorized_units or run.current_unit != unit:
        raise AutomationRunError(f"review unit is not current and authorized: {unit}")
    state = run.units[unit]
    if state.get("review_result_recorded"):
        return run
    status = result.get("status") if isinstance(result, dict) else "pending"
    if status == "pending":
        return run
    findings = result.get("findings", []) if isinstance(result, dict) else []
    if not isinstance(findings, list):
        raise ValueError("review findings must be a list")
    ledger = state.setdefault("findings", {})
    request_kind = state.get("submission", {}).get("request_kind")
    review_round = state.get("submission", {}).get("review_round", 0)
    out_of_scope = any(bool(item.get("out_of_scope")) for item in findings if isinstance(item, dict))
    if out_of_scope:
        run.status = "blocked"
        run.blocked_reason = "reviewer requested out-of-scope changes"
    for raw in findings:
        if not isinstance(raw, dict):
            continue
        key, existing = _find_existing(ledger, raw)
        was_existing = existing is not None
        if existing is None:
            key = _new_finding_key(ledger, str(raw.get("id") or ""))
            existing = {
                "finding_id": key,
                "root_cause": str(raw.get("root_cause") or key),
                "normalized_root_cause": _normalize_root_cause(raw.get("root_cause") or key),
                "root_cause_key": _root_cause_key(raw),
                "failed_remediation_rounds": 0,
                "status": "open",
                "aliases": [],
            }
            ledger[key] = existing
        elif str(raw.get("id") or key) != key and raw.get("id") not in existing.setdefault("aliases", []):
            existing["aliases"].append(str(raw["id"]))
        if (
            was_existing
            and status == "fail"
            and raw.get("status") != "closed"
            and request_kind == "rereview"
            and review_round > 0
            and existing.get("status") == "open"
        ):
            if _remediation_ready_for_current_submission(existing, state["submission"]):
                existing["failed_remediation_rounds"] = int(existing.get("failed_remediation_rounds", 0)) + 1
                existing["remediation"]["consumed"] = True
        existing["root_cause"] = str(raw.get("root_cause") or existing.get("root_cause"))
        existing["normalized_root_cause"] = _normalize_root_cause(existing["root_cause"])
        semantic_key = _root_cause_key(raw)
        if semantic_key is not None:
            existing["root_cause_key"] = semantic_key
        existing["status"] = "closed" if raw.get("status") == "closed" else "open"
        if existing["failed_remediation_rounds"] >= run.max_same_finding_rounds and existing["status"] == "open":
            run.status = "blocked"
            run.blocked_reason = f"finding {existing['finding_id']} reached {run.max_same_finding_rounds} remediation rounds"
    if status == "pass":
        open_findings = _open_findings(state)
        if open_findings:
            run.status = "blocked"
            run.blocked_reason = "reviewer returned PASS with an open finding"
            state["phase"] = "remediating"
        else:
            state["phase"] = "passed"
    elif status == "fail":
        state["phase"] = "remediating"
    state["result"] = copy.deepcopy(result)
    state["review_result_recorded"] = True
    if status == "pass" and run.status == "running" and run.auto_advance:
        from .automation_run import advance

        advance(run)
    return run


def is_blocked(run: AutomationRun, unit: str | None = None) -> bool:
    if run.status == "blocked":
        return True
    units = [unit] if unit else run.authorized_units
    return any(
        int(finding.get("failed_remediation_rounds", 0)) >= run.max_same_finding_rounds
        for selected in units
        if selected in run.units
        for finding in run.units[selected].get("findings", {}).values()
        if finding.get("status") == "open"
    )


def persist_review_result(repo_root: Any, context_key: str, unit: str, result: dict[str, Any]) -> AutomationRun:
    """Apply and persist a result for controller/CLI callers."""

    from .automation_run import load_run

    run = load_run(repo_root, context_key)
    if run is None:
        raise AutomationRunError("no automation run exists")
    record_review_result(run, unit, result)
    save_run(repo_root, run)
    return run


def persist_submission(
    repo_root: Any,
    context_key: str,
    unit: str,
    *,
    parent_sha: str,
    head_sha: str,
    submitted_to: dict[str, Any],
    request_kind: str | None = None,
) -> AutomationRun:
    """Pin and persist a review submission for compaction-safe resumption."""

    from .automation_run import load_run

    run = load_run(repo_root, context_key)
    if run is None:
        raise AutomationRunError("no automation run exists")
    submit_review(
        run,
        unit,
        parent_sha=parent_sha,
        head_sha=head_sha,
        submitted_to=submitted_to,
        request_kind=request_kind,
    )
    save_run(repo_root, run)
    return run
