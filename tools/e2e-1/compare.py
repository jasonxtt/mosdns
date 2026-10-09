"""Semantic differential and path TTL checks; never an automatic acceptance."""
import argparse
import bisect
import collections
import json
import math
import pathlib

from mock import MARKERS

FIELDS = ("domain_set", "effective_tag", "final_sequence", "final_upstream_group")


def records(value):
    return sorted(tuple(row[:4]) for row in value.get("answers", []))


def marker(value):
    return sorted({group for row in value.get("answers", []) for group, addresses in MARKERS.items() if row[3] in addresses})


def upstream_group(audit, aliases):
    raw = (audit or {}).get("final_upstream") or ""
    if not raw:
        return ""
    candidates = aliases.get(raw, [])
    return candidates[0] if len(candidates) == 1 else "unmapped_or_ambiguous"


def ttl_result(row, cold=None):
    """Check non-OPT sections and warm aging against actual mock traffic.

    Rewrite is regenerated at TTL5. Mock answers are capped at 300, or 5 for
    the 3099 cnfake wrapper. A warm answer with no fresh mock invocation must
    age from the same cold records, allowing one second of integer rounding.
    FakeSOA has frozen TTL300; unknown local records fail closed.
    """
    dns = row["dns"]
    rr = [a for section in ("answers", "authority", "additional") for a in dns.get(section, [])]
    if not rr:
        return {"policy": "no_records", "valid": True, "ttls": []}
    audit = row.get("audit") or {}
    rewrite = row["input"].get("class") in ("rewrite-ip", "rewrite-name") or "重定向" in (audit.get("domain_set") or "")
    if rewrite:
        policy, maximum, exact = "rewrite_5", 5, True
    elif marker(dns):
        maximum = 5 if row["input"]["port"] == 3099 else 300
        policy, exact = "mock_max_" + str(maximum), False
    elif all(a[1] == 6 for a in rr):
        policy, maximum, exact = "fake_soa_300", 300, True
    else:
        # No hosts declarations in this fixture. Do not infer a contract from
        # an unexpected local address or accept an arbitrary 86400 cap.
        return {"policy": "unmapped_local_response", "valid": False, "ttls": [a[4] for a in rr]}
    valid = all(a[4] == maximum if exact else 0 <= a[4] <= maximum for a in rr)
    result = {"policy": policy, "valid": valid, "ttls": [a[4] for a in rr]}
    if cold is not None and not exact and not row.get("fresh_upstream", False) and records(dns) == records(cold["dns"]):
        elapsed = max(0, row["finished_s"] - cold["started_s"])
        originals = {tuple(a[:4]): a[4] for section in ("answers", "authority", "additional") for a in cold["dns"].get(section, [])}
        result["warm_elapsed_s"] = round(elapsed, 3)
        result["warm_aged"] = all(
            tuple(a[:4]) in originals and
            max(0, originals[tuple(a[:4])] - math.ceil(elapsed) - 1) <= a[4] <=
            max(0, originals[tuple(a[:4])] - max(0, math.floor(elapsed) - 1))
            for a in rr
        )
        result["valid"] &= result["warm_aged"]
    return result


def differences(left, right):
    changed = []
    for row in (left, right):
        if "error" in row["dns"]:
            changed.append("dns_error")
        if not row.get("ttl_policy", ttl_result(row))["valid"]:
            changed.append("ttl_policy")
        if row.get("audit_expected", row["input"]["port"] == 53) and row["audit"] is None:
            changed.append("audit_missing")
        if (row.get("audit") or {}).get("final_upstream_group") == "unmapped_or_ambiguous":
            changed.append("upstream_mapping")
    if left["dns"].get("rcode") != right["dns"].get("rcode"):
        changed.append("rcode")
    if records(left["dns"]) != records(right["dns"]):
        changed.append("answers")
    if marker(left["dns"]) != marker(right["dns"]):
        changed.append("marker_group")
    if left["audit"] is not None and right["audit"] is not None:
        for field in FIELDS:
            if (left["audit"].get(field) or "") != (right["audit"].get(field) or ""):
                changed.append(field)
    return sorted(set(changed))


def rows(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def compare(root):
    left, right = [rows(root / role / "replay.jsonl") for role in ("go", "native")]
    if len(left) != len(right) or not left:
        raise ValueError("incomplete replay")
    mapping = json.loads((root / "mapping.json").read_text())
    ttl_evidence, ttl_counts = [], {}
    for role, data in (("go", left), ("native", right)):
        calls = collections.defaultdict(list)
        for call in rows(root / role / "mock.jsonl"):
            if call["kind"] == "dns":
                q = call["question"][0]
                calls[(q[0], q[1])].append(call["monotonic_s"])
        cold, counts = {}, collections.Counter()
        for row in data:
            times = calls[(row["input"]["name"] + ".", row["input"]["type"])]
            pos = bisect.bisect_left(times, row["started_s"])
            row["fresh_upstream"] = pos < len(times) and times[pos] <= row["finished_s"]
            if row["audit"] is not None:
                row["audit"]["final_upstream_group"] = upstream_group(row["audit"], mapping["upstream_aliases"][role])
            result = ttl_result(row, cold.get(row["index"]) if row["pass"] == 2 else None)
            row["ttl_policy"] = result
            counts[result["policy"] + (":valid" if result["valid"] else ":invalid")] += 1
            ttl_evidence.append({"role": role, "pass": row["pass"], "index": row["index"], "class": row["input"]["class"], "fresh_upstream": row["fresh_upstream"], **result})
            if row["pass"] == 1:
                cold[row["index"]] = row
        ttl_counts[role] = dict(counts)
        # Private rows retain raw upstream labels beside normalized group identity.
        (root / role / "compared.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in data))
    mismatches, counts, paths = [], collections.Counter(), collections.Counter()
    raw_upstream_differences = 0
    for a, b in zip(left, right):
        if (a["pass"], a["index"], a["input"]) != (b["pass"], b["index"], b["input"]):
            raise ValueError("replay order/input mismatch")
        changed = differences(a, b)
        counts.update(changed)
        paths.update([str(a["input"]["port"]) + ":" + g for g in marker(b["dns"])])
        if a["audit"] is not None and b["audit"] is not None:
            raw_upstream_differences += (a["audit"].get("final_upstream") or "") != (b["audit"].get("final_upstream") or "")
        if changed:
            mismatches.append({"pass": a["pass"], "index": a["index"], "class": a["input"]["class"], "fields": changed, "classification": "pending"})
    latencies = {}
    for role, data in (("go", left), ("native", right)):
        latencies[role] = {}
        for pass_id in (1, 2):
            values = sorted(row["latency_ms"] for row in data if row["pass"] == pass_id)
            latencies[role][str(pass_id)] = {"p" + str(p): values[math.ceil(len(values) * p / 100) - 1] for p in (50, 95, 99)}
    summary = {"rows_per_runtime": len(left), "semantic_equal": len(left) - len(mismatches), "semantic_different": len(mismatches), "different_fields": dict(counts), "raw_final_upstream_differences_diagnostic_only": raw_upstream_differences, "ttl_policies": ttl_counts, "native_marker_paths": dict(paths), "latencies_ms": latencies, "resources": {role: json.loads((root / role / "resources.json").read_text()) for role in ("go", "native")}, "verdict": "classification_required"}
    (root / "ttl-policy.jsonl").write_text("".join(json.dumps(r) + "\n" for r in ttl_evidence))
    (root / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (root / "mismatches.json").write_text(json.dumps(mismatches, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=pathlib.Path)
    compare(parser.parse_args().root)
