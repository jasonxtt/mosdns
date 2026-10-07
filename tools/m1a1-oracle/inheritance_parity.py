#!/usr/bin/env python3
"""Inheritance parity between the Go mapper and the native one for `full:`
rules written with upper-case letters.

The native host matches `full:` rules case-insensitively (approved deviation)
but a rule must still inherit exactly as the reference does, from the
`domain:` rules named by its own spelling. For each case a tiny tree is built
(one `domain_set_light` per rule, one mapper), Go is asked about the *exact
spelling* of the full rule and the native host about that spelling and
differently cased ones; the answers must be identical.
Usage: inheritance_parity.py <work dir> <go oracle> <native example>
"""
import json
import os
import shutil
import subprocess
import sys

WORK, GO, NATIVE = sys.argv[1:4]

# (label, rules in mapper order, exact-case query names)
CASES = [
    ("codex example", ["full:X.example", "domain:x.example", "domain:X.example"], ["X.example."]),
    ("parent spelled differently", ["full:A.B.example", "domain:b.example", "domain:B.example", "domain:example"], ["A.B.example."]),
    ("only lower domain", ["full:Q.example", "domain:q.example"], ["Q.example."]),
    ("only upper domain", ["full:Q.example", "domain:Q.example"], ["Q.example."]),
    ("deep tails", ["full:W.X.Y.example", "domain:y.example", "domain:Y.example", "domain:x.y.example", "domain:X.Y.example"], ["W.X.Y.example."]),
    ("unrelated case", ["full:Mixed.Example", "domain:mixed.example", "domain:example"], ["Mixed.Example."]),
]


def variants(name):
    return [name, name.lower(), name.upper(), name.swapcase()]


def build(label, rules):
    root = os.path.join(WORK, "inh")
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    plugins = ['  - tag: fwd\n    type: forward\n    args: {upstreams: [{addr: "udp://127.0.0.1:15453"}]}\n']
    mapper = []
    for i, rule in enumerate(rules, start=1):
        plugins.append(f'  - tag: p{i}\n    type: domain_set_light\n    args: {{exps: ["{rule}"]}}\n')
        mapper.append(f'{{tag: p{i}, mark: {i}, output_tag: "t{i}"}}')
    plugins.append(f'  - tag: m\n    type: domain_mapper\n    args: {{rules: [{", ".join(mapper)}]}}\n')
    plugins.append("  - tag: e\n    type: sequence\n    args: [{exec: $fwd}]\n  - tag: l\n    type: udp_server\n    args: {entry: e, listen: '127.0.0.1:15353'}\n")
    with open(os.path.join(root, "config.yaml"), "w") as handle:
        handle.write("log: {level: error}\nplugins:\n" + "".join(plugins))
    return root


def ask(root, tool, names):
    inp, out = os.path.join(root, "in.jsonl"), os.path.join(root, f"out-{tool}.jsonl")
    with open(inp, "w") as handle:
        for n in names:
            handle.write(json.dumps({"op": "map", "tag": "m", "name": n}) + "\n")
    if tool == "go":
        cmd = [GO, "-root", root, "-config", "config.yaml", "-in", inp, "-out", out]
    else:
        cmd = [NATIVE, "--root", root, "--config", os.path.join(root, "config.yaml"), "--in", inp, "--out", out]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    with open(out) as handle:
        return [json.loads(line) for line in handle]


bad = 0
for label, rules, names in CASES:
    root = build(label, rules)
    for name in names:
        go_answers = ask(root, "go", [name])
        got = ask(root, "native", variants(name))
        # A truncated answer stream must not pass.
        complete = len(go_answers) == 1 and len(got) == len(variants(name))
        want = go_answers[0] if go_answers else None
        ok = complete and all(answer == want for answer in got)
        bad += not ok
        print("OK  " if ok else "FAIL", label, name, want if ok else {"go": want, "native": got})
sys.exit(1 if bad else 0)
