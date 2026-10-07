#!/usr/bin/env python3
"""Regexp parity between Go's regexp package (the reference mapper/adguard
engine) and the native matcher: writes a request file for both oracles from a
pattern list and compares the answers.

  regexp_parity.py gen   <patterns.txt> <names.txt> <out requests.jsonl>
  regexp_parity.py check <patterns.txt> <go answers> <native answers> <report.json>

Rules checked per pattern:
  Go valid,   native ok          -> match vectors must be identical
  Go valid,   native unsupported -> allowed (the host refuses the rule)
  Go valid,   native invalid     -> VIOLATION (a valid Go rule would be dropped)
  Go invalid, native ok          -> VIOLATION (a rule Go skips would be applied)
  Go invalid, native invalid/unsupported -> fine
"""
import json
import sys


def lines(path):
    with open(path, encoding="utf-8") as handle:
        return [line.rstrip("\n") for line in handle if line.strip() and not line.startswith("##")]


def main():
    mode = sys.argv[1]
    if mode == "gen":
        patterns, names = lines(sys.argv[2]), lines(sys.argv[3])
        with open(sys.argv[4], "w", encoding="utf-8") as out:
            for pattern in patterns:
                out.write(json.dumps({"op": "regexp", "pattern": pattern, "names": names}) + "\n")
        return
    patterns = lines(sys.argv[2])
    go = [json.loads(l) for l in open(sys.argv[3], encoding="utf-8")]
    native = [json.loads(l) for l in open(sys.argv[4], encoding="utf-8")]
    if not (len(patterns) == len(go) == len(native)):
        sys.exit("answer files do not have one answer per pattern")
    counts = {"both_ok_identical": 0, "refused_unsupported": 0, "both_invalid": 0, "violations": 0}
    violations = []
    for pattern, g, n in zip(patterns, go, native):
        if g["valid"] and n["class"] == "ok":
            if g["matches"] == n["matches"]:
                counts["both_ok_identical"] += 1
            else:
                counts["violations"] += 1
                violations.append({"pattern": pattern, "why": "match vectors differ"})
        elif g["valid"] and n["class"] == "unsupported":
            counts["refused_unsupported"] += 1
        elif g["valid"]:
            counts["violations"] += 1
            violations.append({"pattern": pattern, "why": "valid in Go, rejected as invalid natively"})
        elif n["class"] == "ok":
            counts["violations"] += 1
            violations.append({"pattern": pattern, "why": "invalid in Go, accepted natively"})
        else:
            counts["both_invalid"] += 1
    report = {"patterns": len(patterns), "names": len(go[0]["matches"]) if go and go[0]["matches"] else 0, **counts, "violations_detail": violations}
    with open(sys.argv[5], "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=1, ensure_ascii=False)
    print(json.dumps({k: v for k, v in report.items() if k != "violations_detail"}))
    for v in violations:
        print("VIOLATION", json.dumps(v, ensure_ascii=False))
    sys.exit(1 if violations else 0)


if __name__ == "__main__":
    main()
