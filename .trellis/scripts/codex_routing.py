#!/usr/bin/env python3
"""Inspect and update conversation-scoped Codex automation routing."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from common.codex_routing import (
    VALID_SURFACES,
    detect_surface,
    discover_dsh_web,
    discover_herdr,
    executor_validity,
    invalidate,
    load_state,
    resolve_codex_context_key,
    resolve_codex_provider,
    reviewer_validity,
    routing_missing_slots,
    save_state,
    selection_prompt,
    set_dispatch,
    set_surface,
    set_target,
)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("show")
    sub.add_parser("discover")
    sub.add_parser("validate")
    sub.add_parser("prompt")

    inline = sub.add_parser("set-inline", help="select the current Codex as executor")
    inline.set_defaults(mode="inline")

    surface = sub.add_parser("set-surface", help="persist a CLI/Desktop/unknown override")
    surface.add_argument("kind", choices=sorted(VALID_SURFACES))

    executor = sub.add_parser("set-executor", help="select an executor provider/reference")
    executor.add_argument("legacy", nargs="*", help="legacy Herdr workspace and pane arguments")
    executor.add_argument("--provider")
    executor.add_argument("--reference")
    executor.add_argument("--label", default="")

    reviewer = sub.add_parser("set-reviewer", help="select a reviewer provider/reference")
    reviewer.add_argument("legacy", nargs="*", help="legacy ChatGPT conversation and URL arguments")
    reviewer.add_argument("--provider")
    reviewer.add_argument("--reference")
    reviewer.add_argument("--label", default="")
    reviewer.add_argument("--title", default="")
    reviewer.add_argument("--url", default="")
    reviewer.add_argument("--project-id", default="")
    reviewer.add_argument("--project-title", default="")

    for name, role in (("clear-executor", "executor"), ("clear-reviewer", "reviewer")):
        clear = sub.add_parser(name, help=f"clear the conversation's {role} target")
        clear.set_defaults(clear_role=role)

    clear = sub.add_parser("invalidate", help="clear one routing slot")
    clear.add_argument("part", choices=("dispatch", "executor", "reviewer"))
    return parser


def _effective_surface(state: dict) -> object:
    return state.get("surface") or detect_surface()


def _set_executor(root: Path, key: str, args: argparse.Namespace) -> dict:
    provider = args.provider
    reference = args.reference
    metadata: dict[str, str] = {}
    label = args.label
    if provider is None and reference is None and len(args.legacy) == 2:
        provider = "herdr"
        metadata = {"workspace_id": args.legacy[0], "executor_pane_id": args.legacy[1], "mode": "herdr"}
        reference = args.legacy[1]
        label = label or f"Herdr {args.legacy[0]}:{args.legacy[1]}"
    elif provider is None and reference is None and len(args.legacy) == 0:
        raise ValueError("set-executor requires --provider/--reference or legacy Herdr workspace/pane")
    elif args.legacy:
        raise ValueError("do not mix legacy positional arguments with --provider/--reference")

    if provider is None:
        raise ValueError("set-executor requires --provider")
    provider = provider.strip().lower()
    if reference is None:
        if provider == "codex":
            reference = "current"
        elif provider == "dsh":
            raise ValueError("MCP DSH executor is disabled; use --provider dsh-web with the browser URL")
        else:
            raise ValueError("set-executor requires --reference for this provider")
    if provider == "codex" and reference == "codex":
        reference = "current"
    return set_target(
        root,
        key,
        "executor",
        provider,
        reference,
        label=label or provider,
        metadata=metadata or None,
    )


def _set_reviewer(root: Path, key: str, args: argparse.Namespace) -> dict:
    provider = args.provider
    reference = args.reference
    metadata: dict[str, str] = {}
    label = args.label or args.title
    if provider is None and reference is None and len(args.legacy) in (1, 2):
        provider = "chatgpt"
        reference = args.legacy[0]
        if len(args.legacy) == 2:
            metadata["url"] = args.legacy[1]
    elif provider is None and reference is None and len(args.legacy) == 0:
        raise ValueError("set-reviewer requires --provider/--reference or a legacy ChatGPT conversation")
    elif args.legacy:
        raise ValueError("do not mix legacy positional arguments with --provider/--reference")

    if provider is None:
        raise ValueError("set-reviewer requires --provider")
    provider = provider.strip().lower()
    if reference is None:
        if provider == "codex":
            reference = "current"
        else:
            raise ValueError("set-reviewer requires --reference for this provider")
    if provider == "codex" and reference == "codex":
        reference = "current"
    metadata.update(
        {
            key: value
            for key, value in {
                "conversation_id": reference if provider == "chatgpt" else "",
                "url": args.url,
                "conversation_title": args.title,
                "project_id": args.project_id,
                "project_title": args.project_title,
            }.items()
            if value
        }
    )
    return set_target(
        root,
        key,
        "reviewer",
        provider,
        reference,
        label=label or reference,
        metadata=metadata or None,
    )


def main() -> int:
    args = _parser().parse_args()
    root = Path(__file__).resolve().parents[2]
    key = resolve_codex_context_key()
    state = load_state(root, key)
    surface = _effective_surface(state)
    provider = resolve_codex_provider(root, surface, state)

    if args.command == "show":
        output = state
    elif args.command == "discover":
        inventory = discover_herdr()
        dsh_web = discover_dsh_web()
        missing = routing_missing_slots(state)
        recommendations = []
        if dsh_web.candidates:
            candidate = dsh_web.candidates[0]
            recommendations.append(
                {
                    "role": "executor",
                    "provider": "dsh-web",
                    "reference": candidate["reference"],
                    "label": candidate.get("label", "DSH Web"),
                    "reason": "running browser-backed DSH Web endpoint; MCP DSH remains disabled",
                }
            )
        output = {
            "surface": surface.as_dict() if hasattr(surface, "as_dict") else surface,
            "policy_provider": provider,
            "current": inventory.current,
            "candidates": inventory.candidates,
            "error": inventory.error,
            "dsh_web": {
                "candidates": dsh_web.candidates,
                "error": dsh_web.error,
            },
            "recommendations": recommendations,
            "executor": state.get("executor"),
            "reviewer": state.get("reviewer"),
            "missing": missing,
        }
    elif args.command == "validate":
        inventory = discover_herdr()
        dsh_web = discover_dsh_web()
        executor_ok, executor_reason = executor_validity(inventory, state, dsh_web)
        reviewer_ok, reviewer_reason = reviewer_validity(state)
        invalidated = []
        if state.get("executor") is not None and not executor_ok:
            invalidate(state, "executor")
            invalidated.append("executor")
        if state.get("reviewer") is not None and not reviewer_ok:
            invalidate(state, "reviewer")
            invalidated.append("reviewer")
        if invalidated:
            save_state(root, state)
        output = {
            "surface": surface.as_dict() if hasattr(surface, "as_dict") else surface,
            "policy_provider": provider,
            "executor_valid": executor_ok,
            "executor_reason": executor_reason,
            "reviewer_valid": reviewer_ok,
            "reviewer_reason": reviewer_reason,
            "routing_valid": executor_ok and reviewer_ok,
            "invalidated": invalidated,
        }
    elif args.command == "prompt":
        inventory = discover_herdr()
        dsh_web = discover_dsh_web()
        print(selection_prompt(inventory, state, surface=surface, provider=provider, dsh_web=dsh_web))
        return 0
    elif args.command == "set-inline":
        output = set_dispatch(root, key, "inline")
    elif args.command == "set-surface":
        output = set_surface(root, key, args.kind)
    elif args.command == "set-executor":
        output = _set_executor(root, key, args)
    elif args.command == "set-reviewer":
        output = _set_reviewer(root, key, args)
    elif args.command in {"clear-executor", "clear-reviewer"}:
        output = invalidate(state, args.clear_role)
        save_state(root, output)
    else:
        output = invalidate(state, args.part)
        save_state(root, output)
    print(json.dumps(output, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(2)
