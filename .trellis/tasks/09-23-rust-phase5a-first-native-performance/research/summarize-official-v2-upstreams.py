#!/usr/bin/env python3
"""Summarize frozen W1/W2/W3 upstream evidence without changing raw results."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
from pathlib import Path
from typing import Any


INDEX_SHA256 = "7dd7597fc175efa9f124ed62fccf4fee32aeaf365d90afe3684b1feb3b017ea8"
SCENARIOS = ("w1-udp", "w1-tcp", "w2", "w3")
STAGES = ("normal-reference", "common-load", "near-saturation", "overload", "recovery")
EXPECTED_PATHS = {
    "domain-hit": ("route-a",),
    "ip-rule-hit": ("route-b", "route-a"),
    "ip-rule-miss": ("route-b", "route-c"),
}
EXPECTED_QNAMES = {
    "domain-hit": "domain-hit.test.",
    "ip-rule-hit": "ip-hit.test.",
    "ip-rule-miss": "ip-miss.test.",
}
FIELDS = (
    "scenario", "repetition", "candidate", "run_id", "stage", "target_qps",
    "runner_exit", "stage_invalid_reason", "scheduled", "sent", "received",
    "correct_on_time", "w1_positive_forward_delta", "w1_negative_forward_delta",
    "w2_cache_a_miss_delta", "w2_cache_b_miss_delta", "w3_domain_hit_sent",
    "w3_ip_rule_hit_sent", "w3_ip_rule_miss_sent", "w3_route_a_legs_expected",
    "w3_route_a_legs_observed", "w3_route_b_legs_expected", "w3_route_b_legs_observed",
    "w3_route_c_legs_expected", "w3_route_c_legs_observed", "w3_route_legs_expected",
    "w3_route_legs_observed", "w3_route_legs_match", "source_evidence_sha256",
    "source_files",
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_index(root: Path, index: Path) -> list[tuple[str, str]]:
    if sha256(index) != INDEX_SHA256:
        raise ValueError("official v2 raw-result index digest mismatch")
    entries: list[tuple[str, str]] = []
    expected_paths: set[str] = set()
    for number, line in enumerate(index.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        expected, relative = line.split(maxsplit=1)
        relative = relative.removeprefix("*").removeprefix("./")
        if relative in expected_paths:
            raise ValueError(f"duplicate raw-result index path on line {number}: {relative}")
        expected_paths.add(relative)
        path = root / relative
        if not path.is_file() or sha256(path) != expected:
            raise ValueError(f"raw-result index mismatch: {relative}")
        entries.append((relative, expected))
    actual_paths = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file()
    }
    if actual_paths != expected_paths:
        missing = sorted(expected_paths - actual_paths)
        extra = sorted(actual_paths - expected_paths)
        raise ValueError(f"raw-result file set differs from index: missing={missing[:3]} extra={extra[:3]}")
    if len(entries) != 718:
        raise ValueError(f"raw-result index has {len(entries)} files, expected 718")
    return entries


def jsonl(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def read_counter(path: Path) -> dict[str, int]:
    value = json.loads(path.read_text(encoding="utf-8"))
    counts = value.get("counts")
    if not isinstance(counts, dict):
        raise ValueError(f"counter file has no counts object: {path}")
    return {str(key): int(count) for key, count in counts.items()}


def deltas(before: dict[str, int], after: dict[str, int]) -> dict[str, int]:
    keys = set(before) | set(after)
    result = {key: after.get(key, 0) - before.get(key, 0) for key in keys}
    if any(value < 0 for value in result.values()):
        raise ValueError("fixture counter decreased within a session")
    return result


def find_run_file(run_dir: Path, name: str) -> Path:
    matches = list(run_dir.glob(f".run.*/{name}"))
    if len(matches) != 1:
        raise ValueError(f"expected one .run file {name} in {run_dir}, found {len(matches)}")
    return matches[0]


def invalid_reasons(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    result = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        stage, reason = line.split("\t", 1)
        result[stage] = reason
    return result


def source_digest(root: Path, files: set[Path]) -> tuple[str, str]:
    rows = []
    for path in sorted(files):
        relative = path.relative_to(root).as_posix()
        rows.append((relative, sha256(path)))
    payload = "".join(f"{relative}\0{digest}\n" for relative, digest in rows).encode()
    return hashlib.sha256(payload).hexdigest(), ";".join(relative for relative, _ in rows)


def base_row(scenario: str, repetition: int, candidate: str, stage: dict[str, Any], status: dict[str, str], reason: str) -> dict[str, Any]:
    counters = stage.get("counters", {})
    return {
        "scenario": scenario,
        "repetition": repetition,
        "candidate": candidate,
        "run_id": stage.get("run_id", ""),
        "stage": stage["stage"],
        "target_qps": stage.get("target_qps", ""),
        "runner_exit": status["runner_exit"],
        "stage_invalid_reason": reason,
        "scheduled": counters.get("scheduled", ""),
        "sent": counters.get("sent", ""),
        "received": counters.get("received", ""),
        "correct_on_time": counters.get("correct_on_time", ""),
        "w1_positive_forward_delta": "",
        "w1_negative_forward_delta": "",
        "w2_cache_a_miss_delta": "",
        "w2_cache_b_miss_delta": "",
        "w3_domain_hit_sent": "",
        "w3_ip_rule_hit_sent": "",
        "w3_ip_rule_miss_sent": "",
        "w3_route_a_legs_expected": "",
        "w3_route_a_legs_observed": "",
        "w3_route_b_legs_expected": "",
        "w3_route_b_legs_observed": "",
        "w3_route_c_legs_expected": "",
        "w3_route_c_legs_observed": "",
        "w3_route_legs_expected": "",
        "w3_route_legs_observed": "",
        "w3_route_legs_match": "",
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--results-root", type=Path, required=True)
    parser.add_argument("--result-index", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = args.results_root.resolve()
    index = args.result_index.resolve()
    output = args.output.resolve()
    if output.exists() or root in output.parents:
        raise FileExistsError("output must be new and outside the frozen raw-result tree")
    indexed = verify_index(root, index)
    status_rows = list(csv.DictReader((root / "attempt-status.tsv").open(encoding="utf-8", newline=""), delimiter="\t"))
    if len(status_rows) != 24:
        raise ValueError(f"attempt-status.tsv has {len(status_rows)} rows, expected 24")
    status_by_run = {
        (row["scenario"], int(row["repetition"]), row["candidate"]): row
        for row in status_rows
    }
    if len(status_by_run) != 24:
        raise ValueError("duplicate key in attempt-status.tsv")

    rows: list[dict[str, Any]] = []
    for scenario in SCENARIOS:
        for repetition in range(1, 4):
            for candidate in ("go", "rust"):
                status = status_by_run[(scenario, repetition, candidate)]
                run_dir = root / scenario / f"repetition-{repetition}" / candidate
                reasons = invalid_reasons(run_dir / "invalid-stages.tsv")
                stage_path = run_dir / "stages.jsonl"
                run_stages = jsonl(stage_path) if stage_path.exists() else []
                common_sources = {root / "attempt-status.tsv"}
                if stage_path.exists():
                    common_sources.add(stage_path)
                if (run_dir / "invalid-stages.tsv").exists():
                    common_sources.add(run_dir / "invalid-stages.tsv")

                if scenario in ("w1-udp", "w1-tcp"):
                    by_name = {stage["stage"]: stage for stage in run_stages}
                    if set(by_name) != set(STAGES):
                        raise ValueError(f"incomplete W1 stages: {scenario}/{repetition}/{candidate}")
                    before = {name: find_run_file(run_dir, f"counter-before-{name}.json") for name in STAGES}
                    final_counter = run_dir / "fixture-forward.json"
                    if not final_counter.exists():
                        raise FileNotFoundError(final_counter)
                    for index_in_run, name in enumerate(STAGES):
                        after_path = before[STAGES[index_in_run + 1]] if index_in_run + 1 < len(STAGES) else final_counter
                        delta = deltas(read_counter(before[name]), read_counter(after_path))
                        positive = sum(value for key, value in delta.items() if key.startswith("ok."))
                        negative = sum(value for key, value in delta.items() if key.startswith("negative."))
                        stage = by_name[name]
                        if positive + negative != int(stage["counters"]["sent"]):
                            raise ValueError(f"W1 counter delta != sent: {scenario}/{repetition}/{candidate}/{name}")
                        row = base_row(scenario, repetition, candidate, stage, status, reasons.get(name, ""))
                        row["w1_positive_forward_delta"] = positive
                        row["w1_negative_forward_delta"] = negative
                        files = common_sources | {before[name], after_path, final_counter}
                        row["source_evidence_sha256"], row["source_files"] = source_digest(root, files)
                        rows.append(row)

                elif scenario == "w2":
                    cold_path = run_dir / "w2-cold" / "stages.jsonl"
                    warm_path = run_dir / "w2-warm" / "stages.jsonl"
                    prefill_path = run_dir / "w2-warm" / "prefill" / "stages.jsonl"
                    cold_stage = jsonl(cold_path)
                    warm_stages = jsonl(warm_path)
                    prefill_stages = jsonl(prefill_path)
                    if len(cold_stage) != 1 or len(prefill_stages) != 1 or {x["stage"] for x in warm_stages} != set(STAGES):
                        raise ValueError(f"incomplete W2 lifecycle: repetition-{repetition}/{candidate}")

                    cold_before = find_run_file(run_dir, "counter-before-official-w2-cold.json")
                    cold_after = run_dir / "w2-cold" / "fixture-cache.json"
                    cold_delta = deltas(read_counter(cold_before), read_counter(cold_after))
                    cold = cold_stage[0]
                    row = base_row(scenario, repetition, candidate, cold, status, "")
                    row["w2_cache_a_miss_delta"] = cold_delta.get("cache-a.test.|A", 0)
                    row["w2_cache_b_miss_delta"] = cold_delta.get("cache-b.test.|A", 0)
                    if (row["w2_cache_a_miss_delta"], row["w2_cache_b_miss_delta"]) != (1, 1):
                        raise ValueError(f"W2 cold miss count mismatch: repetition-{repetition}/{candidate}")
                    files = common_sources | {cold_path, cold_before, cold_after}
                    row["source_evidence_sha256"], row["source_files"] = source_digest(root, files)
                    rows.append(row)

                    prefill_before = find_run_file(run_dir, "w2-prefill-before-counter.json")
                    prefill_after = find_run_file(run_dir, "w2-prefill-counter.json")
                    prefill = prefill_stages[0]
                    prefill_delta = deltas(read_counter(prefill_before), read_counter(prefill_after))
                    row = base_row(scenario, repetition, candidate, prefill, status, "")
                    row["stage"] = "warm-prefill"
                    row["w2_cache_a_miss_delta"] = prefill_delta.get("cache-a.test.|A", 0)
                    row["w2_cache_b_miss_delta"] = prefill_delta.get("cache-b.test.|A", 0)
                    if (row["w2_cache_a_miss_delta"], row["w2_cache_b_miss_delta"]) != (1, 1):
                        raise ValueError(f"W2 prefill miss count mismatch: repetition-{repetition}/{candidate}")
                    files = common_sources | {prefill_path, prefill_before, prefill_after}
                    row["source_evidence_sha256"], row["source_files"] = source_digest(root, files)
                    rows.append(row)

                    by_name = {stage["stage"]: stage for stage in warm_stages}
                    warm_before = {name: find_run_file(run_dir, f"counter-before-{name}.json") for name in STAGES}
                    warm_after_final = run_dir / "w2-warm" / "fixture-cache.json"
                    for index_in_run, name in enumerate(STAGES):
                        after_path = warm_before[STAGES[index_in_run + 1]] if index_in_run + 1 < len(STAGES) else warm_after_final
                        delta = deltas(read_counter(warm_before[name]), read_counter(after_path))
                        a_delta = delta.get("cache-a.test.|A", 0)
                        b_delta = delta.get("cache-b.test.|A", 0)
                        if a_delta or b_delta:
                            raise ValueError(f"W2 warm stage missed upstream: repetition-{repetition}/{candidate}/{name}")
                        stage = by_name[name]
                        row = base_row(scenario, repetition, candidate, stage, status, reasons.get(name, ""))
                        row["w2_cache_a_miss_delta"] = a_delta
                        row["w2_cache_b_miss_delta"] = b_delta
                        files = common_sources | {warm_path, warm_before[name], after_path, warm_after_final}
                        row["source_evidence_sha256"], row["source_files"] = source_digest(root, files)
                        rows.append(row)

                else:
                    by_name = {stage["stage"]: stage for stage in run_stages}
                    if set(by_name) != set(STAGES):
                        raise ValueError(f"incomplete W3 stages: repetition-{repetition}/{candidate}")
                    request_path = run_dir / "requests.jsonl"
                    requests = jsonl(request_path)
                    for name in STAGES:
                        stage = by_name[name]
                        start, end = int(stage["fixture_seq_start"]), int(stage["fixture_seq_end"])
                        event_path = Path(stage["event_journal_path"])
                        events = [event for event in jsonl(event_path) if start < int(event["fixture_seq"]) <= end]
                        sent_requests = [request for request in requests if request["stage_id"] == name and request["sent"]]
                        sent_by_case = {case: sum(request["case_id"] == case for request in sent_requests) for case in EXPECTED_PATHS}
                        expected_by_upstream = {"route-a": 0, "route-b": 0, "route-c": 0}
                        expected_by_qname_upstream: dict[tuple[str, str], int] = {}
                        for case, count in sent_by_case.items():
                            for upstream in EXPECTED_PATHS[case]:
                                expected_by_upstream[upstream] += count
                                event_key = (EXPECTED_QNAMES[case], upstream)
                                expected_by_qname_upstream[event_key] = count
                        observed_by_upstream = {upstream: sum(event["upstream"] == upstream for event in events) for upstream in expected_by_upstream}
                        observed_by_qname_upstream: dict[tuple[str, str], int] = {}
                        for event in events:
                            event_key = (event["qname"], event["upstream"])
                            observed_by_qname_upstream[event_key] = observed_by_qname_upstream.get(event_key, 0) + 1
                        if observed_by_upstream != expected_by_upstream or observed_by_qname_upstream != expected_by_qname_upstream:
                            raise ValueError(f"W3 route-leg counts differ from sent requests: repetition-{repetition}/{candidate}/{name}")
                        if len(events) != sum(expected_by_upstream.values()):
                            raise ValueError(f"W3 route-event total differs from sent requests: repetition-{repetition}/{candidate}/{name}")
                        if len(sent_requests) != int(stage["counters"]["sent"]):
                            raise ValueError(f"W3 request ledger sent count differs: repetition-{repetition}/{candidate}/{name}")
                        row = base_row(scenario, repetition, candidate, stage, status, reasons.get(name, ""))
                        row["w3_domain_hit_sent"] = sent_by_case["domain-hit"]
                        row["w3_ip_rule_hit_sent"] = sent_by_case["ip-rule-hit"]
                        row["w3_ip_rule_miss_sent"] = sent_by_case["ip-rule-miss"]
                        row["w3_route_a_legs_expected"] = expected_by_upstream["route-a"]
                        row["w3_route_a_legs_observed"] = observed_by_upstream["route-a"]
                        row["w3_route_b_legs_expected"] = expected_by_upstream["route-b"]
                        row["w3_route_b_legs_observed"] = observed_by_upstream["route-b"]
                        row["w3_route_c_legs_expected"] = expected_by_upstream["route-c"]
                        row["w3_route_c_legs_observed"] = observed_by_upstream["route-c"]
                        row["w3_route_legs_expected"] = sum(expected_by_upstream.values())
                        row["w3_route_legs_observed"] = len(events)
                        row["w3_route_legs_match"] = str(len(events) == row["w3_route_legs_expected"]).lower()
                        files = common_sources | {request_path, event_path}
                        row["source_evidence_sha256"], row["source_files"] = source_digest(root, files)
                        rows.append(row)

    if len(rows) != 132:
        raise ValueError(f"produced {len(rows)} upstream rows, expected 132")
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=FIELDS, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    print(f"raw_index_sha256={sha256(index)}")
    print(f"indexed_raw_files={len(indexed)}")
    print(f"upstream_rows={len(rows)}")
    print(f"output_sha256={sha256(output)}")
    print(f"output={output}")


if __name__ == "__main__":
    main()
