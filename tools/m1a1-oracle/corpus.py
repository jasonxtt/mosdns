#!/usr/bin/env python3
"""Generate the M1-a1 differential corpus.

Domain cases are derived from the rules the Go providers export (`--rules` is
the Go oracle's answer to `rules_request.jsonl`): for every rule its own name,
subdomains, parent, neighbouring names, case and trailing-dot variants, plus
keyword/regexp positives and random negatives. IP cases are the first/last
address of every CIDR range of every enabled `si_set` source (decoded here
independently of both implementations), the addresses just outside, and
random addresses. Every case line carries a `class` for the report.
"""
import argparse
import json
import os
import random
import re
import struct
import zlib

import extract


def uvarint(buf, pos):
    value = shift = 0
    while True:
        byte = buf[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            return value, pos
        shift += 7


def srs_ip_ranges(path):
    """Ranges of the item-6 (ip cidr) entries of one SRS file."""
    with open(path, "rb") as handle:
        data = handle.read()
    if data[:3] != b"SRS" or data[3] > 3:
        return []
    try:
        body = zlib.decompress(data[4:])
    except zlib.error:
        return []
    count, pos = uvarint(body, 0)
    ranges = []

    def rule(pos):
        mode = body[pos]
        pos += 1
        if mode == 0:
            while True:
                item = body[pos]
                pos += 1
                if item == 0xFF:
                    return pos
                if item != 6:
                    return pos
                assert body[pos] == 1
                pos += 1
                (length,) = struct.unpack(">Q", body[pos : pos + 8])
                pos += 8
                for _ in range(length):
                    n, pos = uvarint(body, pos)
                    start = body[pos : pos + n]
                    pos += n
                    n, pos = uvarint(body, pos)
                    end = body[pos : pos + n]
                    pos += n
                    ranges.append((start, end))
        if mode == 1:
            pos += 1
            n, pos = uvarint(body, pos)
            for _ in range(n):
                pos = rule(pos)
            return pos + 1
        return pos

    for _ in range(count):
        pos = rule(pos)
    return ranges


def to_addr(raw):
    import ipaddress

    if len(raw) == 4:
        return ipaddress.IPv4Address(raw)
    if len(raw) == 16:
        return ipaddress.IPv6Address(raw)
    return None


def parent(name):
    return name.split(".", 1)[1] if "." in name else None


def regex_sample(pattern):
    sample = pattern.strip()
    sample = sample.lstrip("^").rstrip("$")
    sample = sample.replace("\\.", ".").replace(".*", "abc").replace(".+", "abc")
    sample = re.sub(r"\[[^\]]*\][+*]?", "1", sample)
    sample = re.sub(r"\\d[+*]?", "1", sample)
    sample = re.sub(r"\([^)]*\)[?*+]?", "", sample)
    sample = re.sub(r"[\\^$?*+()\[\]{}|]", "", sample)
    try:
        if sample and re.search(pattern, sample):
            return sample
    except re.error:
        return None
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    parser.add_argument("--config", default="config_custom.yaml")
    parser.add_argument("--tags", required=True)
    parser.add_argument("--rules", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--stats", required=True)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    with open(args.tags, encoding="utf-8") as handle:
        tags = json.load(handle)

    rules = set()
    with open(args.rules, encoding="utf-8") as handle:
        for line in handle:
            rules.update(json.loads(line).get("rules") or [])

    names = {}  # name -> class
    regexp_without_positive = 0

    def add(name, klass):
        names.setdefault(name if name.endswith(".") else name + ".", klass)

    for rule in sorted(rules):
        kind, _, pattern = rule.partition(":")
        if not _:
            add(rule, "unprefixed")
            continue
        base = pattern.strip(".")
        if kind == "full" and base:
            add(base, "full-self")
            add("sub." + base, "full-sub")
            if parent(base):
                add(parent(base), "full-parent")
            add(base + "-x", "full-neighbor")
            add(base.upper(), "case")
            add("WwW." + base.title(), "case")
        elif kind == "domain" and base:
            add(base, "domain-self")
            add("www." + base, "domain-sub")
            add("a.b." + base, "domain-deep")
            if parent(base):
                add(parent(base), "domain-parent")
            add("z" + base, "domain-neighbor")
            add("www." + base.upper(), "case")
        elif kind == "keyword" and pattern:
            add(pattern + ".example.", "keyword")
            add("pre" + pattern + "post.test", "keyword")
            add(pattern.upper() + ".test", "case")
        elif kind == "regexp":
            sample = regex_sample(pattern)
            if not sample:
                regexp_without_positive += 1
            if sample:
                add(sample, "regexp")
                add("x" + sample + "y", "regexp")
            add("noregexmatch.invalid", "regexp-neg")
    # Random negatives and shape edge cases.
    letters = "abcdefghijklmnopqrstuvwxyz0123456789-"
    for _ in range(20000):
        labels = [
            "".join(rng.choice(letters) for _ in range(rng.randint(1, 12)))
            for _ in range(rng.randint(1, 4))
        ]
        add(".".join(labels) + rng.choice([".com", ".net", ".cn", ".org", ".xyz"]), "random")
    for name in [
        "localhost", "a", "xn--fiq228c.com", "_dmarc.example.com", "1.2.3.4", "_http._tcp.deb.debian.org",
        "-bad-.com", "very." * 30 + "long.test", "www.google.com", "google.com", "cn", "com",
        "baidu.com", "qq.com", "apple.com", "doubleclick.net", "WWW.GOOGLE.COM",
    ]:
        add(name, "edge")
    add("", "root")
    # Every mixed-case name also gets its lowercase twin, so a case-only
    # difference can be told apart from a real one.
    for name in list(names):
        if name != name.lower():
            names.setdefault(name.lower(), "case-twin")

    cases = []
    for mapper in tags["mappers"]:
        for name, klass in sorted(names.items()):
            cases.append({"op": "map", "tag": mapper, "name": name, "class": klass})
        cases.append({"op": "count", "tag": mapper, "class": "count"})

    ip_cases = 0
    if tags["ipsets"]:
        defs = {d["tag"]: d for d in extract.load(args.root, args.config)}
        for tag in tags["ipsets"]:
            local = defs[tag]["args"]["local_config"]
            with open(os.path.join(args.root, local), encoding="utf-8") as handle:
                manifest = json.load(handle)
            addrs = {}
            for source in manifest:
                if not source.get("enabled") or not source.get("files"):
                    continue
                path = os.path.join(args.root, source["files"])
                if not os.path.exists(path):
                    continue
                for start, end in srs_ip_ranges(path):
                    low, high = to_addr(start), to_addr(end)
                    if low is None or high is None:
                        continue
                    addrs.setdefault(str(low), "ip-first")
                    addrs.setdefault(str(high), "ip-last")
                    if int(low) > 0:
                        addrs.setdefault(str(type(low)(int(low) - 1)), "ip-before")
                    if int(high) < (1 << (32 if low.version == 4 else 128)) - 1:
                        addrs.setdefault(str(type(high)(int(high) + 1)), "ip-after")
            for _ in range(5000):
                addrs.setdefault(".".join(str(rng.randint(0, 255)) for _ in range(4)), "ip-random")
            for _ in range(2000):
                addrs.setdefault(
                    ":".join("%x" % rng.randint(0, 0xFFFF) for _ in range(8)), "ip-random6"
                )
            for fixed in ["0.0.0.0", "255.255.255.255", "127.0.0.1", "::", "::1", "2001:db8::1", "::ffff:1.2.3.4"]:
                addrs.setdefault(fixed, "ip-edge")
            for addr, klass in sorted(addrs.items()):
                cases.append({"op": "ip", "tag": tag, "addr": addr, "class": klass})
                ip_cases += 1

    with open(args.out, "w", encoding="utf-8") as handle:
        for case in cases:
            handle.write(json.dumps(case) + "\n")
    classes = {}
    for case in cases:
        classes[case["class"]] = classes.get(case["class"], 0) + 1
    with open(args.stats, "w", encoding="utf-8") as handle:
        json.dump(
            {
                "unique_rules": len(rules),
                "cases": len(cases),
                "ip_cases": ip_cases,
                "regexp_rules_without_a_derived_positive": regexp_without_positive,
                "classes": classes,
            },
            handle,
            indent=1,
            sort_keys=True,
        )
    print(len(cases), "cases")


if __name__ == "__main__":
    main()
