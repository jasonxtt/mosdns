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
import time
from datetime import datetime
from pathlib import Path
from typing import Any, Callable, Protocol
from urllib.parse import urlsplit

from .automation import AutomationContext, make_target, validate_target
from .automation_review import build_review_request, is_blocked
from .automation_run import AutomationRun, AutomationRunError


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


MAX_REVIEW_ONLY_MESSAGE_BYTES = 4096
_FINAL_LINE = re.compile(r"(?im)^\s*FINAL\s*:\s*(PASS|FAIL)\s*$")
_STRICT_FINDING = re.compile(
    r"(?im)^\s*(P[0-3]-\d+)\s*(?:[-—:]\s*)?(.+?)\s+\[(open|closed)\]\s*$"
)
_FINDING_MARKER = re.compile(r"(?im)^\s*P[0-3]-\d+\b")
_FULL_SHA = re.compile(r"^[0-9a-fA-F]{40}$")
_IN_PROGRESS_STATES = {"active", "generating", "in_progress", "pending", "queued", "running", "streaming", "working"}


def _compact(value: Any, default: str = "(not supplied)") -> str:
    if value is None:
        return default
    if isinstance(value, (list, tuple, set)):
        compacted = [_compact(item, "") for item in value]
        rendered = ", ".join(item for item in compacted if item)
        return rendered or default
    if isinstance(value, dict):
        rendered = ", ".join(f"{key}={_compact(item, '')}" for key, item in value.items())
        return rendered or default
    return re.sub(r"\s+", " ", str(value).strip()) or default


def build_reviewer_only_request(
    run: AutomationRun,
    unit: str,
    evidence: dict[str, Any],
) -> dict[str, Any]:
    """Wrap Trellis evidence in the bounded external C2C review contract."""

    base_request = build_review_request(run, unit, evidence)
    if run.status == "blocked" or is_blocked(run, unit):
        raise AutomationRunError("review-only request is unavailable for a blocked run or finding limit")
    for field in ("base_sha", "head_sha"):
        value = evidence.get(field)
        if not isinstance(value, str) or _FULL_SHA.fullmatch(value.strip()) is None:
            raise ValueError(f"review-only request requires a full {field}")
    paths = evidence.get("changed_paths")
    if (
        not isinstance(paths, (list, tuple, set))
        or not paths
        or any(not isinstance(path, str) or not path.strip() for path in paths)
    ):
        raise ValueError("review-only request requires non-empty changed_paths")
    state = run.units[unit]
    submission = state.get("submission") if isinstance(state.get("submission"), dict) else {}
    if base_request["kind"] == "rereview":
        if (
            submission.get("request_kind") != "rereview"
            or state.get("review_result_recorded") is not False
            or not isinstance(state.get("result"), dict)
            or state["result"].get("status") not in {"pass", "fail"}
        ):
            raise AutomationRunError("re-review request requires an explicit prior result and submitted remediation")
        if (
            _FULL_SHA.fullmatch(str(submission.get("parent_sha", ""))) is None
            or _FULL_SHA.fullmatch(str(submission.get("head_sha", ""))) is None
        ):
            raise AutomationRunError("re-review request requires a complete submitted parent/head SHA pair")
    submitted_parent = submission.get("parent_sha")
    submitted_head = submission.get("head_sha")
    if bool(submitted_parent) != bool(submitted_head):
        raise AutomationRunError("review-only submission has an incomplete parent/head SHA pair")
    if submitted_parent and submitted_head:
        if (
            evidence["base_sha"].strip() != submitted_parent
            or evidence["head_sha"].strip() != submitted_head
        ):
            raise AutomationRunError("review-only evidence must match the submitted parent/head SHA pair")
    lines = [
        "[C2C]",
        "MODE: REVIEW_ONLY",
        "STATE: REVIEW",
        "CONTROLLER: TRELLIS",
        f"TASK_ID: {run.task}",
        f"UNIT: {unit}",
        f"REQUEST_KIND: {base_request['kind']}",
        f"BASE_SHA: {_compact(evidence['base_sha'])}",
        f"HEAD_SHA: {_compact(evidence['head_sha'])}",
        f"PATHS: {_compact(evidence.get('changed_paths'))}",
        f"VALIDATION: {_compact(evidence.get('validation'))}",
        f"ACCEPTANCE: {_compact(evidence.get('acceptance'))}",
        f"FORBIDDEN_SCOPE: {_compact(evidence.get('forbidden_scope'))}",
        "EVIDENCE: Use read-only git_compare(base_sha, head_sha, path, offset, max_bytes) for the exact committed range.",
        "INSTRUCTION: Review only this unit. Do not plan, execute, edit, create tasks, change Trellis state, or treat C2C DONE/PLAN/iteration limits as controller authority.",
        "OUTPUT: Return one final line, FINAL: PASS or FINAL: FAIL. PASS has no findings. FAIL uses P0/P1/P2/P3-N: root cause [open|closed]; keep the same ID for the same root cause on re-review.",
    ]
    previous = state.get("findings") or {}
    if base_request["kind"] == "rereview" and previous:
        lines.insert(-2, f"PREVIOUS_FINDINGS: {_compact(previous)}")
    text = "\n".join(lines)
    if len(text.encode("utf-8")) > MAX_REVIEW_ONLY_MESSAGE_BYTES:
        raise ValueError("review-only request exceeds the bounded message size")
    return {"kind": base_request["kind"], "task": run.task, "unit": unit, "text": text}


def parse_c2c_review_result(text: Any) -> dict[str, Any]:
    """Parse the strict reviewer-only PASS/FAIL and stable finding contract."""

    pending = {"status": "pending", "findings": [], "raw": text}
    if not isinstance(text, str) or not text.strip():
        return pending
    finals = list(_FINAL_LINE.finditer(text))
    if len(finals) != 1 or text.rstrip().splitlines()[-1].strip() != finals[0].group(0).strip():
        return pending
    finding_markers = list(_FINDING_MARKER.finditer(text))
    findings: list[dict[str, Any]] = []
    for match in _STRICT_FINDING.finditer(text):
        findings.append(
            {
                "id": match.group(1),
                "root_cause": match.group(2).strip(),
                "status": match.group(3).lower(),
                "out_of_scope": "out-of-scope" in match.group(2).lower(),
            }
        )
    if len(findings) != len(finding_markers):
        return pending
    if len({finding["id"] for finding in findings}) != len(findings):
        return pending
    status = finals[0].group(1).lower()
    if status == "pass" and findings:
        return pending
    if status == "fail" and not findings:
        return pending
    return {"status": status, "findings": findings, "raw": text}


class C2CWebHost(Protocol):
    """Platform-native ChatGPT capability injected by the host."""

    def verify_target(self, target: dict[str, Any]) -> dict[str, Any]:
        ...

    def send_message(self, target: dict[str, Any], message: str) -> Any:
        ...

    def read_thread(self, target: dict[str, Any], cursor: str | None = None) -> Any:
        ...


def _payload_text(payload: Any) -> str:
    if not isinstance(payload, dict):
        return payload if isinstance(payload, str) else ""
    latest = payload.get("latestAssistantMessage")
    if isinstance(latest, dict):
        for key in ("text", "message"):
            if isinstance(latest.get(key), str):
                return latest[key]
    content = payload.get("content")
    if isinstance(content, list):
        parts = [item.get("text") for item in content if isinstance(item, dict) and isinstance(item.get("text"), str)]
        return "\n".join(parts)
    for key in ("text", "message"):
        if isinstance(payload.get(key), str):
            return payload[key]
    return ""


def _assistant_message_id(payload: Any) -> str | None:
    if not isinstance(payload, dict):
        return None
    latest = payload.get("latestAssistantMessage")
    if isinstance(latest, dict):
        for key in ("id", "messageId", "message_id"):
            if isinstance(latest.get(key), str) and latest[key].strip():
                return latest[key].strip()
    for key in ("latestAssistantMessageId", "assistantMessageId", "assistant_message_id"):
        if isinstance(payload.get(key), str) and payload[key].strip():
            return payload[key].strip()
    return None


def _assistant_in_progress(payload: Any) -> bool:
    if not isinstance(payload, dict):
        return False
    statuses: list[Any] = [payload.get("status"), payload.get("assistantStatus"), payload.get("assistant_status")]
    latest = payload.get("latestAssistantMessage")
    if isinstance(latest, dict):
        statuses.extend((latest.get("status"), latest.get("phase")))
    return any(isinstance(status, str) and status.strip().lower() in _IN_PROGRESS_STATES for status in statuses)


class C2CWebReviewerTransport:
    """Exactly-once, bounded send/read transport for a C2C reviewer target."""

    def __init__(
        self,
        host: C2CWebHost,
        target: dict[str, Any],
        *,
        poll_interval: float = 1.0,
        clock: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
    ):
        self.host = host
        self.target = copy.deepcopy(target)
        self.poll_interval = max(0.0, float(poll_interval))
        self.clock = clock
        self.sleep = sleep
        self._sent = False
        self._send_attempted = False
        self._attempted_message: str | None = None
        self._verified_target: dict[str, Any] | None = None
        self._cursor: str | None = None
        self._baseline_ready = False
        self._baseline_assistant_id: str | None = None
        self._post_send_response_seen = False
        self._candidate_assistant_id: str | None = None
        self._latest_text = ""
        self._latest_payload: Any = None
        self._final_candidate: str | None = None
        self._final_candidate_polls = 0

    def _validate_target(self, target: dict[str, Any]) -> None:
        if not validate_target(target) or target.get("provider") != "c2c-web":
            raise C2CReviewerBindingError("C2C transport requires a c2c-web target")
        metadata = target.get("metadata")
        if not isinstance(metadata, dict):
            raise C2CReviewerBindingError("C2C target metadata is missing")
        if metadata.get("chat_url") != target.get("reference"):
            raise C2CReviewerBindingError("C2C target chat identity is inconsistent")
        for key in ("project_url", "connector_name", "binding_source"):
            if not isinstance(metadata.get(key), str) or not metadata[key].strip():
                raise C2CReviewerBindingError(f"C2C target metadata is missing {key}")

    def verify_target(self, target: dict[str, Any]) -> dict[str, Any]:
        self._validate_target(target)
        if target != self.target:
            raise C2CReviewerBindingError("C2C reviewer verification target differs from the transport target")
        result = self.host.verify_target(copy.deepcopy(target))
        if result is False or not isinstance(result, dict):
            raise C2CReviewerBindingError("C2C reviewer target could not be verified")
        if any(result.get(key) is False for key in ("verified", "valid", "available", "ok")):
            raise C2CReviewerBindingError("C2C reviewer target was not verified")
        if not any(result.get(key) is True for key in ("verified", "valid", "available", "ok")):
            raise C2CReviewerBindingError("C2C reviewer target verification was not affirmative")
        if result.get("target") is not None and result.get("target") != target:
            raise C2CReviewerBindingError("C2C reviewer verification returned a different target")
        verified = copy.deepcopy(result)
        verified.setdefault("target", copy.deepcopy(target))
        verified.setdefault("mechanism", "chatgpt-platform-native")
        self._verified_target = copy.deepcopy(target)
        return verified

    def send(self, request: Any) -> Any:
        if self._sent:
            raise C2CReviewerBindingError("C2C review request was already sent")
        message = request.get("text") if isinstance(request, dict) else request
        if not isinstance(message, str) or not message.startswith("[C2C]"):
            raise C2CReviewerBindingError("C2C review request must be a structured message")
        if len(message.encode("utf-8")) > MAX_REVIEW_ONLY_MESSAGE_BYTES:
            raise C2CReviewerBindingError("C2C review request exceeds the bounded message size")
        if self._send_attempted and message != self._attempted_message:
            raise C2CReviewerBindingError("a failed C2C send may only be retried with the exact same message")
        if self._verified_target != self.target:
            raise C2CReviewerBindingError("C2C review target must be verified before sending")
        self._validate_target(self.target)
        if not self._baseline_ready:
            baseline = self.host.read_thread(copy.deepcopy(self.target), None)
            if not isinstance(baseline, dict) or not isinstance(baseline.get("cursor"), str) or not baseline["cursor"]:
                raise C2CReviewerBindingError("C2C reviewer transport requires a pre-send message cursor")
            baseline_status = str(baseline.get("status", "")).lower()
            if baseline_status in {"error", "failed", "dead", "not_found"} or baseline.get("error"):
                raise C2CReviewerBindingError("C2C reviewer transport failed while reading the pre-send cursor")
            self._cursor = baseline["cursor"]
            self._baseline_assistant_id = _assistant_message_id(baseline)
            if self._baseline_assistant_id is None:
                raise C2CReviewerBindingError("C2C reviewer transport requires a pre-send assistant message identity")
            self._baseline_ready = True
        self._send_attempted = True
        self._attempted_message = message
        result = self.host.send_message(copy.deepcopy(self.target), message)
        self._sent = True
        return result

    def wait_result(self, timeout: float) -> Any:
        if not self._sent:
            raise C2CReviewerBindingError("C2C reviewer result cannot be read before the request is sent")
        deadline = self.clock() + max(0.0, float(timeout))
        while True:
            payload = self.host.read_thread(copy.deepcopy(self.target), self._cursor)
            self._latest_payload = payload
            if not isinstance(payload, dict) or not isinstance(payload.get("cursor"), str) or not payload["cursor"]:
                raise C2CReviewerBindingError("C2C reviewer transport response has no message cursor")
            returned_cursor = payload["cursor"]
            status = str(payload.get("status", "")).lower() if isinstance(payload, dict) else ""
            if status in {"error", "failed", "dead", "not_found"} or (
                isinstance(payload, dict) and payload.get("error")
            ):
                raise C2CReviewerBindingError("C2C reviewer transport failed while waiting")
            cursor_changed = returned_cursor != self._cursor
            if cursor_changed:
                self._cursor = returned_cursor
            assistant_id = _assistant_message_id(payload)
            if not self._post_send_response_seen:
                if (
                    not cursor_changed
                    or _assistant_in_progress(payload)
                    or assistant_id is None
                    or (self._baseline_assistant_id is not None and assistant_id == self._baseline_assistant_id)
                ):
                    self._latest_text = ""
                    self._final_candidate = None
                    self._final_candidate_polls = 0
                    remaining = deadline - self.clock()
                    if remaining <= 0:
                        return False
                    if self.poll_interval <= 0:
                        return False
                    self.sleep(min(self.poll_interval, remaining))
                    continue
                self._post_send_response_seen = True
                self._candidate_assistant_id = assistant_id
                self._latest_text = _payload_text(payload)
            else:
                if (
                    _assistant_in_progress(payload)
                    or assistant_id is None
                    or (self._baseline_assistant_id is not None and assistant_id == self._baseline_assistant_id)
                ):
                    self._latest_text = ""
                    self._final_candidate = None
                    self._final_candidate_polls = 0
                    remaining = deadline - self.clock()
                    if remaining <= 0:
                        return False
                    if self.poll_interval <= 0:
                        return False
                    self.sleep(min(self.poll_interval, remaining))
                    continue
                if self._candidate_assistant_id != assistant_id:
                    self._candidate_assistant_id = assistant_id
                    self._final_candidate = None
                    self._final_candidate_polls = 0
                self._latest_text = _payload_text(payload)
            parsed = parse_c2c_review_result(self._latest_text)
            if parsed["status"] in {"pass", "fail"}:
                if self._latest_text == self._final_candidate:
                    self._final_candidate_polls += 1
                else:
                    self._final_candidate = self._latest_text
                    self._final_candidate_polls = 1
                if self._final_candidate_polls >= 2:
                    return "ready"
            else:
                self._final_candidate = None
                self._final_candidate_polls = 0
            remaining = deadline - self.clock()
            if remaining <= 0:
                return False
            if self.poll_interval <= 0:
                return False
            self.sleep(min(self.poll_interval, remaining))

    def read(self) -> str:
        return self._latest_text
