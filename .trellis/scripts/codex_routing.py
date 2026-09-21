#!/usr/bin/env python3
"""Deprecated CLI shim for the conversation automation context.

The old command name remains for one release so existing callers receive a
clear migration path. It forwards only explicit context operations; surface,
discovery, and provider-policy commands are intentionally rejected.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from common.active_task import resolve_context_key
from common.automation import (
    clear_executor,
    clear_reviewer,
    load_context,
    save_context,
    set_executor,
    set_reviewer,
)
from common.codex_routing import RoutingDeprecatedError


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Deprecated automation context compatibility shim")
    sub = parser.add_subparsers(dest="command", required=True)

    for name in ("show", "migrate"):
        sub.add_parser(name)

    inline = sub.add_parser("set-inline", help="restore the implicit current executor")
    inline.set_defaults(provider="current", reference="current")

    executor = sub.add_parser("set-executor", help="set an explicit executor target")
    executor.add_argument("--provider", required=True)
    executor.add_argument("--reference", required=False)
    executor.add_argument("--label", default="")
    executor.add_argument("--metadata", default=None)

    reviewer = sub.add_parser("set-reviewer", help="set an explicit reviewer target")
    reviewer.add_argument("--provider", required=True)
    reviewer.add_argument("--reference", required=True)
    reviewer.add_argument("--label", default="")
    reviewer.add_argument("--metadata", default=None)

    for name, role in (("clear-executor", "executor"), ("clear-reviewer", "reviewer")):
        command = sub.add_parser(name)
        command.set_defaults(clear_role=role)

    invalidate = sub.add_parser("invalidate")
    invalidate.add_argument("part", choices=("executor", "reviewer", "dispatch"))

    for name in ("discover", "validate", "prompt", "set-surface"):
        sub.add_parser(name)
    return parser


def _metadata(raw: str | None) -> dict | None:
    if not raw:
        return None
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise ValueError("--metadata must be a JSON object")
    return value


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    if args.command in {"discover", "validate", "prompt", "set-surface"}:
        raise RoutingDeprecatedError(
            f"{args.command} is deprecated; use explicit common.automation context and provider adapters"
        )

    root = Path(__file__).resolve().parents[2]
    context_key = resolve_context_key(platform="codex")
    if not context_key:
        raise RuntimeError("cannot resolve the current conversation identity")
    context = load_context(root, context_key)

    if args.command == "show" or args.command == "migrate":
        pass
    elif args.command == "set-inline":
        set_executor(context, "current", "current")
        save_context(root, context)
    elif args.command == "set-executor":
        reference = args.reference
        if reference is None and args.provider.strip().lower() in {"current", "codex"}:
            reference = "current"
        if reference is None:
            raise ValueError("set-executor requires --reference unless provider=current")
        set_executor(
            context,
            args.provider,
            reference,
            label=args.label,
            metadata=_metadata(args.metadata),
        )
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
    elif args.command in {"clear-executor", "clear-reviewer"}:
        (clear_executor if args.clear_role == "executor" else clear_reviewer)(context)
        save_context(root, context)
    elif args.command == "invalidate":
        if args.part == "dispatch":
            raise RoutingDeprecatedError(
                "dispatch invalidation is deprecated; clear an explicit executor target by name"
            )
        if args.part == "executor":
            clear_executor(context)
        else:
            clear_reviewer(context)
        save_context(root, context)
    print(json.dumps(context.to_dict(), indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RoutingDeprecatedError, RuntimeError, ValueError, json.JSONDecodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(2)
