#!/usr/bin/env python3
"""Manage the current Codex conversation's Herdr/reviewer routing choice."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from common.codex_routing import (
    discover_herdr,
    executor_validity,
    invalidate,
    load_state,
    resolve_codex_context_key,
    save_state,
    selection_prompt,
    set_dispatch,
    set_reviewer,
)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("show")
    sub.add_parser("discover")
    sub.add_parser("validate")
    sub.add_parser("prompt")
    inline = sub.add_parser("set-inline")
    inline.set_defaults(mode="inline")
    executor = sub.add_parser("set-executor")
    executor.add_argument("workspace_id")
    executor.add_argument("pane_id")
    reviewer = sub.add_parser("set-reviewer")
    reviewer.add_argument("conversation_id")
    reviewer.add_argument("url")
    reviewer.add_argument("--title", default="")
    reviewer.add_argument("--project-id", default="")
    reviewer.add_argument("--project-title", default="")
    clear = sub.add_parser("invalidate")
    clear.add_argument("part", choices=("dispatch", "reviewer"))
    return parser


def main() -> int:
    args = _parser().parse_args()
    root = Path(__file__).resolve().parents[2]
    key = resolve_codex_context_key()
    state = load_state(root, key)
    if args.command == "show":
        output = state
    elif args.command == "discover":
        inventory = discover_herdr()
        output = {"current": inventory.current, "candidates": inventory.candidates, "error": inventory.error}
    elif args.command == "validate":
        valid, reason = executor_validity(discover_herdr(), state)
        output = {"executor_valid": valid, "reason": reason, "reviewer_selected": state.get("reviewer") is not None}
    elif args.command == "prompt":
        print(selection_prompt(discover_herdr(), state))
        return 0
    elif args.command == "set-inline":
        output = set_dispatch(root, key, "inline")
    elif args.command == "set-executor":
        output = set_dispatch(root, key, "herdr", workspace_id=args.workspace_id, executor_pane_id=args.pane_id)
    elif args.command == "set-reviewer":
        output = set_reviewer(
            root,
            key,
            conversation_id=args.conversation_id,
            url=args.url,
            conversation_title=args.title,
            project_id=args.project_id,
            project_title=args.project_title,
        )
    else:
        output = invalidate(state, args.part)
        save_state(root, output)
    print(json.dumps(output, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
