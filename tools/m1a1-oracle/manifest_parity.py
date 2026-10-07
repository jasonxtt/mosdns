#!/usr/bin/env python3
"""Manifest-decoding parity between the Go providers and the native ones.

For each manifest variant a tiny tree is built (one SRS file, one provider, one
mapper or IP set) and both oracles are asked what the provider ended up with:
the number of rules for `sd_set_light`, whether an address matches for
`si_set`. Usage: manifest_parity.py <repo src dir> <work dir> <go oracle> <native example> <fixture srs dir>
"""
import json
import os
import shutil
import subprocess
import sys

SRC, WORK, GO, NATIVE, SRSDIR = sys.argv[1:6]

SD = [
    ("plain", '[{"name":"a","files":"srs/domains.srs","enabled":true}]'),
    ("disabled", '[{"name":"a","files":"srs/domains.srs","enabled":false}]'),
    ("upper keys", '[{"Name":"a","Files":"srs/domains.srs","ENABLED":true}]'),
    ("later spelling wins off", '[{"name":"a","files":"srs/domains.srs","enabled":true,"Enabled":false}]'),
    ("later spelling wins on", '[{"name":"a","files":"srs/domains.srs","Enabled":false,"enabled":true}]'),
    ("null scalars", '[{"name":"a","files":"srs/domains.srs","enabled":true,"url":null,"rule_count":null,"last_updated":null}]'),
    ("null before good", '[{"name":"b","files":null,"enabled":null},{"name":"a","files":"srs/domains.srs","enabled":true}]'),
    ("bad enabled type", '[{"name":"a","files":"srs/domains.srs","enabled":"true"}]'),
    ("repeated key bad then good", '[{"name":"a","files":"srs/domains.srs","enabled":"x","enabled":true}]'),
    ("bad time", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"not-a-time"}]'),
    ("good time", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02.956038327+08:00"}]'),
    ("zero time", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"0001-01-01T00:00:00Z"}]'),
    ("time without zone", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02"}]'),
    ("lowercase t", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13t23:56:02Z"}]'),
    ("lowercase z", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02z"}]'),
    ("comma fraction", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02,5Z"}]'),
    ("offset hour 24", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02+24:00"}]'),
    ("offset minute 60", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02+23:60"}]'),
    ("leap second", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:60Z"}]'),
    ("empty fraction", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02.Z"}]'),
    ("offset without colon", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02+0800"}]'),
    ("unused huge id", '[{"id":1e1000,"name":"a","files":"srs/domains.srs","enabled":true}]'),
    ("time one-digit hour", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T3:56:02Z"}]'),
    ("time one-digit hour fraction", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T3:56:02.5Z"}]'),
    ("time one-digit hour bad offset", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T3:56:02+25:00"}]'),
    ("time offset 24:00", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02+24:00"}]'),
    ("time offset 25:00", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02+25:00"}]'),
    ("time offset 23:60", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02+23:60"}]'),
    ("time offset 23:61", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02-23:61"}]'),
    ("time one-digit minute", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:5:02Z"}]'),
    ("escaped T in time", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13\\u005423:56:02Z"}]'),
    ("escaped Z in time", '[{"name":"a","files":"srs/domains.srs","enabled":true,"last_updated":"2026-05-13T23:56:02\\u005a"}]'),
    ("negative zero count", '[{"name":"a","files":"srs/domains.srs","enabled":true,"rule_count":-0}]'),
    ("integral float count", '[{"name":"a","files":"srs/domains.srs","enabled":true,"update_interval_hours":1.0}]'),
    ("exponent count", '[{"name":"a","files":"srs/domains.srs","enabled":true,"rule_count":1e2}]'),
    ("long s key", '[{"name":"a","fileſ":"srs/domains.srs","enabled":true}]'),
    ("kelvin key", '[{"name":"a","files":"srs/domains.srs","enabled":true,"auto_update":true,"\u212aey":1}]'),
    ("long s in enabled", '[{"name":"a","files":"srs/domains.srs","ENABLED":true,"Fileſ":"srs/domains.srs"}]'),
    ("huge count", '[{"name":"a","files":"srs/domains.srs","enabled":true,"rule_count":1e1000}]'),
    ("float count", '[{"name":"a","files":"srs/domains.srs","enabled":true,"rule_count":1.5}]'),
    ("huge count", '[{"name":"a","files":"srs/domains.srs","enabled":true,"rule_count":18446744073709551615}]'),
    ("negative interval", '[{"name":"a","files":"srs/domains.srs","enabled":true,"update_interval_hours":-3}]'),
    ("id wrong type (not a sd field)", '[{"id":5,"name":"a","files":"srs/domains.srs","enabled":true}]'),
    ("duplicate names", '[{"name":"a","files":"srs/domains.srs","enabled":false},{"name":"a","files":"srs/domains.srs","enabled":true}]'),
    ("not an array", '{"name":"a"}'),
    ("trailing data", '[] []'),
    ("empty file", ''),
    ("empty array", '[]'),
]
SI = [
    ("plain", '[{"name":"a","files":"srs/ips.srs","enabled":true}]'),
    ("enable_regexp wrong type ignored", '[{"name":"a","files":"srs/ips.srs","enabled":true,"enable_regexp":"x"}]'),
    ("id wrong type ignored", '[{"id":5,"name":"a","files":"srs/ips.srs","enabled":true}]'),
    ("bad time", '[{"name":"a","files":"srs/ips.srs","enabled":true,"last_updated":"2026-02-30T00:00:00Z"}]'),
    ("upper keys", '[{"NAME":"a","FILES":"srs/ips.srs","Enabled":true}]'),
]


def run(cmd):
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def tree(name, kind, manifest):
    root = os.path.join(WORK, name)
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(os.path.join(root, "srs"))
    for f in ("domains.srs", "ips.srs"):
        shutil.copy(os.path.join(SRSDIR, f), os.path.join(root, "srs", f))
    with open(os.path.join(root, "srs/m.json"), "w") as handle:
        handle.write(manifest)
    plugins = [
        '  - tag: fwd\n    type: forward\n    args: {upstreams: [{addr: "udp://127.0.0.1:15453"}]}\n',
        f'  - tag: p\n    type: {kind}\n    args: {{local_config: "srs/m.json"}}\n',
    ]
    if kind == "sd_set_light":
        plugins.append("  - tag: m\n    type: domain_mapper\n    args: {rules: [{tag: p, mark: 1}]}\n")
    plugins.append("  - tag: e\n    type: sequence\n    args: [{exec: $fwd}]\n  - tag: l\n    type: udp_server\n    args: {entry: e, listen: '127.0.0.1:15353'}\n")
    with open(os.path.join(root, "config.yaml"), "w") as handle:
        handle.write("log: {level: error}\nplugins:\n" + "".join(plugins))
    return root


def ask(root, tool, requests):
    inp, out = os.path.join(root, "in.jsonl"), os.path.join(root, f"out-{tool}.jsonl")
    with open(inp, "w") as handle:
        for r in requests:
            handle.write(json.dumps(r) + "\n")
    if tool == "go":
        cmd = [GO, "-root", root, "-config", "config.yaml", "-in", inp, "-out", out]
    else:
        cmd = [NATIVE, "--root", root, "--config", os.path.join(root, "config.yaml"), "--in", inp, "--out", out]
    run(cmd)
    with open(out) as handle:
        return [json.loads(line) for line in handle]


def main():
    bad = 0
    for label, manifest in SD:
        root = tree("sd", "sd_set_light", manifest)
        # Go: unique rules the provider exports; native: unique rules of the mapper.
        go_rules = len(set(ask(root, "go", [{"op": "rules", "tag": "p"}])[0].get("rules") or []))
        native_rules = ask(root, "native", [{"op": "count", "tag": "m"}])[0]["rules"]
        ok = go_rules == native_rules
        bad += not ok
        print("OK  " if ok else "FAIL", "sd_set_light", label, go_rules, native_rules)
    for label, manifest in SI:
        root = tree("si", "si_set", manifest)
        reqs = [{"op": "ip", "tag": "p", "addr": a} for a in ("10.0.0.1", "192.0.2.6", "8.8.8.8", "2001:db8::1")]
        go, native = ask(root, "go", reqs), ask(root, "native", reqs)
        ok = go == native
        bad += not ok
        print("OK  " if ok else "FAIL", "si_set", label, [a["match"] for a in go], [a["match"] for a in native])
    sys.exit(1 if bad else 0)


main()
