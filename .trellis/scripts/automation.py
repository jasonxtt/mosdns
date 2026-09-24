#!/usr/bin/env python3
"""Manage the current conversation's small automation context."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from common.active_task import resolve_context_key
from common.automation import (
    clear_executor,
    clear_reviewer,
    load_context,
    migrate_legacy_routing,
    save_context,
    set_executor,
    set_reviewer,
)
from common.automation_c2c_web import resolve_reviewer_target
from common.automation_run import (
    ActivationError,
    AutomationRunError,
    activate,
    authorize,
    complete,
    load_run,
    record_fail,
    record_pass,
)
from common.automation_review import parse_review_result, persist_review_result


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Manage conversation-scoped Trellis automation context")
    sub = parser.add_subparsers(dest="command", required=True)

    for name in ("show", "migrate"):
        command = sub.add_parser(name)
        command.add_argument("--context", help="explicit context key (normally resolved from the current session)")

    executor = sub.add_parser("set-executor", help="set an explicit executor override")
    executor.add_argument("--context")
    executor.add_argument("--provider", required=True)
    executor.add_argument("--reference")
    executor.add_argument("--label", default="")
    executor.add_argument("--metadata", help="JSON object containing adapter metadata")

    reviewer = sub.add_parser("set-reviewer", help="set the generic reviewer target")
    reviewer.add_argument("--context")
    reviewer.add_argument("--provider", required=True)
    reviewer.add_argument("--reference", required=True)
    reviewer.add_argument("--label", default="")
    reviewer.add_argument("--metadata", help="JSON object containing adapter metadata")

    for name in ("clear-executor", "clear-reviewer"):
        command = sub.add_parser(name)
        command.add_argument("--context")

    authorize_command = sub.add_parser("authorize", help="snapshot authorized implementation units")
    authorize_command.add_argument("task")
    authorize_command.add_argument("--units", default="all")
    authorize_command.add_argument("--context")
    authorize_command.add_argument(
        "--reviewer-evidence",
        required=False,
        help="JSON evidence envelope returned by the host-level reviewer transport probe",
    )

    activate_command = sub.add_parser("activate", help="create a run from a pre-start authorization snapshot")
    activate_command.add_argument("task")
    activate_command.add_argument("--context")

    status = sub.add_parser("run-status", help="show the active automation run")
    status.add_argument("--context")

    for name in ("record-pass", "record-fail"):
        command = sub.add_parser(name)
        command.add_argument("--context")
        command.add_argument("--unit")
        command.add_argument("--result", help="JSON result payload")

    review = sub.add_parser("record-review", help="parse and persist one reviewer response")
    review.add_argument("--context")
    review.add_argument("--unit", required=True)
    review.add_argument("--text", required=True)

    complete_command = sub.add_parser("complete", help="advance after a recorded PASS")
    complete_command.add_argument("--context")

    return parser


def _root() -> Path:
    return Path(__file__).resolve().parents[2]


def _context_key(args: argparse.Namespace) -> str:
    explicit = getattr(args, "context", None)
    if isinstance(explicit, str) and explicit.strip():
        return explicit.strip()
    context_key = resolve_context_key(platform="codex")
    if not context_key:
        raise RuntimeError(
            "cannot resolve the current conversation identity; set TRELLIS_CONTEXT_ID or use --context"
        )
    return context_key


def _metadata(raw: str | None) -> dict[str, Any] | None:
    if raw is None:
        return None
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise ValueError(f"--metadata must be a JSON object: {exc}") from exc
    if not isinstance(value, dict):
        raise ValueError("--metadata must be a JSON object")
    return value


def _result(raw: str | None) -> Any:
    if raw is None:
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError as exc:
        raise ValueError(f"--result must be valid JSON: {exc}") from exc


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    root = _root()
    key = _context_key(args)

    if args.command == "authorize":
        context = authorize(
            root,
            args.task,
            args.units,
            context_key=key,
            reviewer_transport_evidence=_result(args.reviewer_evidence),
            reviewer_resolver=lambda current: resolve_reviewer_target(current, root)[0],
        )
    elif args.command == "activate":
        context = activate(root, args.task, context_key=key)
    elif args.command == "run-status":
        run = load_run(root, key)
        print(json.dumps(run.to_dict() if run is not None else {"status": "none"}, indent=2, ensure_ascii=False))
        return 0
    elif args.command == "record-pass":
        context = record_pass(root, key, unit=args.unit, result=_result(args.result))
    elif args.command == "record-fail":
        context = record_fail(root, key, unit=args.unit, result=_result(args.result))
    elif args.command == "record-review":
        context = persist_review_result(root, key, args.unit, parse_review_result(args.text))
    elif args.command == "complete":
        context = complete(root, key)
    elif args.command == "migrate":
        context = migrate_legacy_routing(root, key)
    else:
        context = load_context(root, key)

    if args.command == "set-executor":
        set_executor(
            context,
            args.provider,
            args.reference,
            label=args.label,
            metadata=_metadata(args.metadata),
        )
        save_context(root, context)
    elif args.command == "clear-executor":
        clear_executor(context)
        save_context(root, context)
    elif args.command == "set-reviewer":
        set_reviewer(
            context,
            args.provider,
            args.reference,
            label=args.label,
            metadata=_metadata(args.metadata),
        )
        save_context(root, context)
    elif args.command == "clear-reviewer":
        clear_reviewer(context)
        save_context(root, context)

    print(json.dumps(context.to_dict(), indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ActivationError, AutomationRunError, RuntimeError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(2)
