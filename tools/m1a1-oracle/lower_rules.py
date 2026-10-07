#!/usr/bin/env python3
"""Lowercase every `full:` line of the text rule files under a snapshot copy
(rule/ and gen/), for the case-insensitive reference run."""
import os
import sys

root = sys.argv[1]
changed = 0
for sub in ("rule", "gen"):
    base = os.path.join(root, sub)
    if not os.path.isdir(base):
        continue
    for name in os.listdir(base):
        path = os.path.join(base, name)
        if not name.endswith(".txt") or not os.path.isfile(path):
            continue
        with open(path, encoding="utf-8", errors="surrogateescape") as handle:
            lines = handle.read().split("\n")
        out = [line.lower() if line.strip().startswith("full:") else line for line in lines]
        if out != lines:
            changed += 1
            with open(path, "w", encoding="utf-8", errors="surrogateescape") as handle:
                handle.write("\n".join(out))
print("lowercased full rules in", changed, "files")
