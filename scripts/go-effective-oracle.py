#!/usr/bin/env python3
"""Prints what the reference (Go) host effectively does with each aliapi group,
computed independently from its override files (coremain/api_upstream.go rules),
in the shape of the native `check` EFFECTIVE lines. Used to validate the
settings migration; reads only the three webinfo override files it is given.

usage: go-effective-oracle.py <dir-with-webinfo> <yaml-default-groups.json> [native-check-output]
  yaml-default-groups.json: {"group": [{"tag":..,"addr":..}, ...]} for groups
  without a usable override list (the YAML list is then what Go ran).
  With a native `mosdns check` output the two are compared (exit 1 on a
  difference); the one intended rename `<tag>-<group>` is accepted.
"""
import json
import os
import sys

FALLBACK = {"foreign", "foreignecs"}
TCP = {"tcp", "tls", "https", "quic"}


def norm_protocol(p):
    p = (p or "").strip().lower()
    return {"dot": "tls", "doh": "https", "doq": "quic"}.get(p, p)


def runtime_addr(protocol, addr):
    raw = (addr or "").strip()
    if not raw or "://" in raw:
        return raw
    scheme = protocol if protocol in ("udp", "tcp", "tls", "https", "quic") else ""
    if not scheme:
        return raw
    if scheme != "https":
        return f"{scheme}://{raw}"
    return f"{scheme}://{raw}" if "/" in raw else f"{scheme}://{raw}/dns-query"


def find(base, name):
    for path in (os.path.join(base, "webinfo", name), os.path.join(base, name)):
        if os.path.isfile(path):
            return path
    return None


def go_effective(base, yaml_groups):
    cfg = json.load(open(find(base, "config_overrides.json")))
    overrides = json.load(open(find(base, "upstream_overrides.json")))
    global_socks5 = (cfg.get("socks5") or "").strip()
    result = {}
    for group, entries in sorted(overrides.items()):
        enabled = [e for e in entries if e.get("enabled")]
        if not enabled:
            result[group] = [(e["tag"], e["addr"], "yaml") for e in yaml_groups.get(group, [])]
            continue
        rows = []
        for e in enabled:
            protocol = norm_protocol(e.get("protocol"))
            literal = (e.get("socks5") or "").strip()
            explicit = e.get("use_socks_proxy")
            used = explicit if explicit is not None else (protocol in TCP and (bool(literal) or group in FALLBACK))
            addr = ""
            if used:
                addr = literal or (global_socks5 if group in FALLBACK else "")
            really = bool(addr) and protocol in ("tcp", "tls", "https")
            kind = "proxy-global" if really and addr == global_socks5 else ("proxy-own" if really else "direct")
            # The dial address decides which server is really contacted, so it is
            # part of what is compared (the native EFFECTIVE value prints it too).
            dial = (e.get("dial_addr") or "").strip()
            target = runtime_addr(protocol, e.get("addr")) + (f" dial={dial}" if dial else "")
            rows.append((e["tag"], target, kind))
        result[group] = rows
    return bool(global_socks5), result


def native_effective(path, global_set):
    result = {}
    for line in open(path, encoding="utf-8"):
        if not line.startswith("EFFECTIVE aliapi "):
            continue
        _, _, group, rest = line.rstrip("\n").split(" ", 3)
        value = rest.split(" source=")[0][len("upstreams="):]
        rows = []
        for part in value.split(", "):
            proxied = part.endswith(" [proxy]")
            part = part.removesuffix(" [proxy]")
            tag, _, addr = part.partition("=")
            rows.append((tag, addr, ("proxy-global" if global_set else "proxy-?") if proxied else "direct"))
        result[group] = rows
    return result


def main():
    base, yaml_groups = sys.argv[1], json.load(open(sys.argv[2]))
    global_set, expected = go_effective(base, yaml_groups)
    print(f"global socks5: {'SET' if global_set else 'EMPTY'}")
    for group, rows in expected.items():
        print(f"{group}:")
        for tag, addr, kind in rows:
            print(f"  {tag}={addr} {kind}")
    if len(sys.argv) > 3:
        actual = native_effective(sys.argv[3], global_set)
        # The native host needs unique upstream tags: a later group's repeated
        # tag becomes `<tag>-<group>` (the migration's documented rename).
        seen = set()
        for group in actual:
            renamed = []
            for tag, addr, kind in expected.get(group, []):
                if tag in seen:
                    tag = f"{tag}-{group}"
                seen.add(tag)
                renamed.append((tag, addr, kind))
            if group in expected:
                expected[group] = renamed
        bad = 0
        for group, rows in expected.items():
            if rows != actual.get(group, []):
                bad += 1
                print(f"DIFF {group}: go={rows} native={actual.get(group)}")
        print("EQUIVALENT" if not bad else f"{bad} group(s) differ")
        sys.exit(1 if bad else 0)


main()
