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


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    root = _root()
    key = _context_key(args)

    if args.command == "migrate":
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
    except (RuntimeError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(2)
