#!/usr/bin/env python3
"""Compare the native harness against the Go oracle.

Two Go answer sets are used because the native mapper intentionally matches
`full:` rules case-insensitively (user decision 2026-10-07; the reference
mapper matches them case-sensitively):

  --go        Go on the snapshot as is, queries as generated.
  --go-lower  Go on a snapshot whose `full:` text rules are lowercased, queried
              with lowercased names. That is exactly the reference behavior
              with case-insensitive `full:` rules.

The native answer must equal `--go-lower` for every case (this is the
acceptance check). The number of cases where `--go` and `--go-lower` differ is
the footprint of the deviation and is reported separately.
"""
import argparse
import json
import sys


def joined(value):
    return sorted(part for part in (value or "").split("|") if part)


def norm(answer):
    if "domain_set" in answer:
        # Tags are joined in configuration order (deterministic); only the order
        # of source names can differ (the reference iterates a Go map).
        return dict(answer, source=joined(answer.get("source")))
    return answer


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", required=True)
    parser.add_argument("--go", required=True)
    parser.add_argument("--go-lower", required=True)
    parser.add_argument("--native", required=True)
    parser.add_argument("--report", required=True)
    parser.add_argument("--samples", type=int, default=25)
    args = parser.parse_args()
    for path in (args.go, args.go_lower, args.native):
        with open(path) as handle, open(args.cases) as cases:
            if sum(1 for _ in handle) != sum(1 for _ in cases):
                sys.exit(f"{path} does not have one answer per case")
    total = identical = join_order = diff = footprint = 0
    by_class = {}
    samples = []
    with open(args.cases) as c, open(args.go) as g, open(args.go_lower) as gl, open(args.native) as n:
        for case_line, go_line, lower_line, native_line in zip(c, g, gl, n):
            case = json.loads(case_line)
            if case["op"] == "count":
                continue
            go, lower, native = json.loads(go_line), json.loads(lower_line), json.loads(native_line)
            total += 1
            stats = by_class.setdefault(case["class"], {"total": 0, "native_equals_reference": 0, "deviation_footprint": 0})
            stats["total"] += 1
            if norm(go) != norm(lower):
                footprint += 1
                stats["deviation_footprint"] += 1
            if native == lower:
                identical += 1
                stats["native_equals_reference"] += 1
            elif norm(native) == norm(lower):
                join_order += 1
                stats["native_equals_reference"] += 1
            else:
                diff += 1
                if len(samples) < args.samples:
                    samples.append({"case": case, "go_lower": lower, "native": native})
    report = {
        "total": total,
        "native_identical_to_reference": identical,
        "native_same_modulo_join_order": join_order,
        "native_different": diff,
        "cases_where_case_insensitive_full_changes_the_reference_answer": footprint,
        "by_class": by_class,
        "samples": samples,
    }
    with open(args.report, "w") as handle:
        json.dump(report, handle, indent=1, sort_keys=True, ensure_ascii=False)
    print(json.dumps({k: v for k, v in report.items() if k not in ("by_class", "samples")}))
    if diff:
        sys.exit(1)


if __name__ == "__main__":
    main()
