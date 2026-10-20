"""Produce numerical-only statistics from private evidence; never emit names/IPs."""
import argparse
import collections
import json
import math
import pathlib
import statistics



def rows(path):
    with path.open() as stream:
        for line in stream:
            if line.strip():
                yield json.loads(line)


def distribution(values):
    values = sorted(values)
    if not values:
        return {"samples": 0}
    return {"samples": len(values), "min": values[0], "median": statistics.median(values), "max": values[-1],
            **{f"p{p}": values[math.ceil(len(values) * p / 100) - 1] for p in (50, 95, 99)}}


def cpu_intervals(samples):
    values = []
    gaps = changed = 0
    for previous, current in zip(samples, samples[1:]):
        if current.get("sample_error") or previous.get("sample_error"):
            gaps += 1
            continue
        if current.get("start_ticks") != previous.get("start_ticks") or current.get("pid") != previous.get("pid"):
            changed += 1
            continue
        elapsed = current["monotonic"] - previous["monotonic"]
        if elapsed <= 0:
            gaps += 1
            continue
        values.append((current["cpu_seconds"] - previous["cpu_seconds"]) * 100 / elapsed)
    return {"percent": distribution(values), "sample_gaps": gaps, "identity_changes": changed}


def trend(samples):
    pairs = [(r["monotonic"], r["rss_kib"]) for r in samples if r.get("rss_kib") is not None]
    if len(pairs) < 2:
        return None
    # Center timestamps first: avoid loss of precision at long machine uptimes.
    start = pairs[0][0]
    xs, ys = [t - start for t, _ in pairs], [rss / 1024 for _, rss in pairs]
    mean_x, mean_y = statistics.mean(xs), statistics.mean(ys)
    denominator = sum((x - mean_x) ** 2 for x in xs)
    return sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys)) / denominator * 3600 if denominator else None


def summarize(root, reference):
    resources = list(rows(root / "resources.jsonl"))
    before = next(r for r in resources if r["phase"] == "idle_collection_baseline_end")
    end = next(r for r in resources if r["phase"] == "capture_finished")
    measured = [r for r in resources if before["monotonic"] <= r["monotonic"] <= end["monotonic"]]
    result = {"capture_window_seconds": end["monotonic"] - before["monotonic"],
              "start_wall": before["wall"], "end_wall": end["wall"], "minute_samples": len(measured),
              "guard_max": [max(r["guard"][index] for r in resources) for index in range(3)],
              "counts": json.loads((root / "summary.json").read_text())["counts"], "processes": {}}
    for role in ("rust", "replay_collector", "worker", "capture_ssh"):
        entries = [{"monotonic": r["monotonic"], **r["resources"][role]} for r in measured
                   if role in r["resources"] and not r["resources"][role].get("unavailable")]
        result["processes"][role] = {"rss_mib": distribution([r["rss_kib"] / 1024 for r in entries]),
            "kernel_hwm_mib": max((r["hwm_kib"] or 0) / 1024 for r in entries) if entries else None,
            "rss_slope_mib_per_hour": trend(entries), "cpu": cpu_intervals(entries),
            "threads": distribution([r["threads"] for r in entries]), "fds": distribution([r["fd_count"] for r in entries])}
    totals = [sum(r["resources"][role]["rss_kib"] for role in ("replay_collector", "worker", "capture_ssh")) / 1024
              for r in measured if all(role in r["resources"] and "rss_kib" in r["resources"][role]
                                        for role in ("replay_collector", "worker", "capture_ssh"))]
    result["tool_total_rss_mib"] = distribution(totals)
    reference_rows = [r for r in rows(reference) if result["start_wall"] <= r.get("wall", r.get("controller_start", 0)) <= result["end_wall"]]
    good_reference = [r for r in reference_rows if not r.get("sample_error")]
    result["go_reference"] = {"samples": len(reference_rows), "failed_samples": len(reference_rows) - len(good_reference),
        "rss_mib": distribution([r["rss_kib"] / 1024 for r in good_reference]), "cpu": cpu_intervals(reference_rows),
        "service_pid_values": sorted({r["pid"] for r in good_reference}),
        "service_restart_values": sorted({r["service"]["NRestarts"] for r in good_reference})}
    # Do not average minute percentiles. Reconstruct the full private per-query
    # sample distribution when retained detailed rows cover the whole run.
    files = list(reversed(sorted(root.glob("paired.jsonl.[123]")))) + [root / "paired.jsonl"]
    latency, incomplete, differences, routes = [], collections.Counter(), collections.Counter(), collections.Counter()
    detail_rows = 0
    classifications, unexplained = collections.Counter(), collections.Counter()
    tc_counts = collections.Counter()
    for file in files:
        if not file.exists():
            continue
        for row in rows(file):
            detail_rows += 1
            classifications[row["classification"]] += 1
            if "latency_ms" in row:
                latency.append(row["latency_ms"])
            if row["classification"] == "incomplete":
                incomplete["go_missing" if "go" not in row else "rust_missing"] += 1
            if row["different_fields"]:
                differences["+".join(sorted(row["different_fields"]))] += 1
                if row["classification"] == "unexplained":
                    unexplained["+".join(sorted(row["different_fields"]))] += 1
            if "go" in row and "rust" in row:
                left, right = row["go"]["tc"], row["rust"]["tc"]
                tc_counts["paired"] += 1
                tc_counts["go_tc"] += left
                tc_counts["rust_tc"] += right
                tc_counts["go_only_tc"] += left and not right
                tc_counts["rust_only_tc"] += right and not left
                if bytes.fromhex(row["query_wire"])[10:12] == b"\0\0":
                    tc_counts["query_ar_zero"] += 1
                    go_size = len(bytes.fromhex(row["go_wire"]))
                    rust_size = len(bytes.fromhex(row["rust_wire"]))
                    tc_counts["no_edns_rust_over_512"] += rust_size > 512
                    tc_counts["no_edns_over_512_without_tc"] += rust_size > 512 and not right
                    if left or right:
                        tc_counts["no_edns_tc_pairs"] += 1
                        tc_counts["no_edns_tc_length_equal"] += go_size == rust_size
                        tc_counts["no_edns_tc_length_different"] += go_size != rust_size
            audit = row.get("audit", {})
            routes["unmapped" if audit.get("final_upstream_group") == "unmapped_or_ambiguous" else "mapped_or_no_forward" if audit else "audit_missing"] += 1
    result.update(detail_rows=detail_rows, detail_complete=detail_rows == result["counts"].get("replayed", 0),
        latency_ms=distribution(latency), incomplete_categories=dict(incomplete), difference_categories=dict(differences),
        audit_group_mapping=dict(routes), tc_counts=dict(tc_counts))
    result.update(classifications=dict(classifications), unexplained_categories=dict(unexplained))
    count_file = root / "learned-counts.jsonl"
    if count_file.exists():
        samples = list(rows(count_file))
        result["learned_count_monitor"] = {"samples": len(samples),
            "any_decrease": any(s["decreased"] for s in samples),
            "initial": samples[0] if samples else None, "final": samples[-1] if samples else None}
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=pathlib.Path)
    parser.add_argument("--reference", required=True, type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()
    result = summarize(args.root, args.reference)
    text = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(text)
    print(text)
