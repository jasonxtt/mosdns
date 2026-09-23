#!/usr/bin/env python3
"""Summarize per-stage and per-role metrics without modifying official raw data."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import statistics
from datetime import datetime
from pathlib import Path


SCENARIOS = ("w1-udp", "w1-tcp", "w2", "w3")
STAGES = ("normal-reference", "common-load", "near-saturation", "overload", "recovery")
STAGE_FIELDS = (
    "scenario", "repetition", "position", "candidate", "stage", "target_qps", "duration_ms",
    "runner_exit", "stage_invalid_reason", "valid_pairs", "complete_pairs", "pair_valid",
    "correct_on_time", "scheduled", "sent", "received", "correct_late", "expected_negative_on_time",
    "wrong_response", "protocol_error", "transport_error", "timeout", "sender_shortfall",
    "latency_sample_count", "p50_us", "p95_us", "p99_us", "effective_throughput_qps",
)
RESOURCE_FIELDS = (
    "scenario", "repetition", "position", "candidate", "stage", "role", "pid", "sample_count",
    "sample_interval_s", "cpu_ticks", "user_system_cpu_ms", "cpu_percent_one_core", "cpu_resolution_percent_one_core",
    "cpu_us_per_correct_on_time", "cpu_resolution_us_per_correct_on_time",
    "rss_median_kib", "rss_peak_kib", "fd_median", "fd_peak", "stage_invalid_reason", "pair_valid",
)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def jsonl(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as stream:
        return [json.loads(line) for line in stream if line.strip()]


def read_tsv(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as stream:
        return list(csv.DictReader(stream, delimiter="\t"))


def parse_timestamp(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def group_key(scenario: str, stage: str) -> tuple[str, str]:
    return scenario, stage


def build_group_index(aggregation: dict) -> dict[tuple[str, str], dict]:
    result = {}
    for group in aggregation.get("groups", []):
        key = group_key(group["scenario"], group["stage"])
        if key in result:
            raise ValueError(f"duplicate paired group {key}")
        result[key] = group
    return result


def stage_paths(run_dir: Path, scenario: str, stage: str) -> tuple[Path, Path]:
    if scenario == "w2" and stage == "official-w2-cold":
        cold = run_dir / "w2-cold"
        return cold / "stages.jsonl", cold / "resource-samples.jsonl"
    if scenario == "w2":
        warm = run_dir / "w2-warm"
        return warm / "stages.jsonl", warm / "resource-samples.jsonl"
    return run_dir / "stages.jsonl", run_dir / "resource-samples.jsonl"


def write_tsv(path: Path, fields: tuple[str, ...], rows: list[dict]) -> None:
    with path.open("x", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def summarize(
    manifest_path: Path,
    manifest_sha: str,
    driver_path: Path,
    driver_sha: str,
    results_root: Path,
    status_map_path: Path,
    status_map_sha: str,
    aggregation_path: Path,
    aggregation_sha: str,
    stage_output: Path,
    resource_output: Path,
) -> tuple[int, int]:
    if digest(manifest_path) != manifest_sha:
        raise ValueError("manifest SHA-256 mismatch")
    if digest(driver_path) != driver_sha:
        raise ValueError("matrix driver SHA-256 mismatch")
    if digest(status_map_path) != status_map_sha:
        raise ValueError("attempt exit status map SHA-256 mismatch")
    if digest(aggregation_path) != aggregation_sha:
        raise ValueError("paired aggregation SHA-256 mismatch")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    aggregation = json.loads(aggregation_path.read_text(encoding="utf-8"))
    if aggregation.get("manifest_sha256") != manifest_sha:
        raise ValueError("paired aggregation is bound to a different manifest")
    if stage_output.exists() or resource_output.exists():
        raise FileExistsError("refusing to overwrite an existing summary")

    pair_groups = build_group_index(aggregation)
    status_rows = read_tsv(status_map_path)
    expected_attempts = 24
    if len(status_rows) != expected_attempts:
        raise ValueError(f"status map has {len(status_rows)} attempts, want {expected_attempts}")
    status_by_run = {}
    for row in status_rows:
        key = (row["scenario"], int(row["repetition"]), row["candidate"])
        if key in status_by_run:
            raise ValueError(f"duplicate attempt-status mapping {key}")
        if row["manifest_sha256"] != manifest_sha:
            raise ValueError(f"status map manifest mismatch for {key}")
        if row["matrix_driver_sha256"] != driver_sha:
            raise ValueError(f"status map matrix driver mismatch for {key}")
        if row["status_source_sha256"] != digest(results_root / "attempt-status.tsv"):
            raise ValueError(f"status map source checksum mismatch for {key}")
        status_by_run[key] = row

    matrix_rows = {
        (row["scenario"], int(row["repetition"]), row["candidate"]): row
        for row in read_tsv(results_root / "attempt-status.tsv")
    }
    if set(matrix_rows) != set(status_by_run):
        raise ValueError("matrix attempt-status.tsv does not match the derived status map")
    for key, row in status_by_run.items():
        source = matrix_rows[key]
        for field in ("runner_exit", "start_utc", "end_utc"):
            if row[field] != source[field]:
                raise ValueError(f"derived status map differs from matrix status for {key}/{field}")

    stage_rows: list[dict] = []
    resource_rows: list[dict] = []
    scenarios = manifest.get("scenarios", {})
    schedule = manifest.get("pair_schedule", [])
    expected_keys = {
        (scenario, pair["repetition"], candidate)
        for scenario in SCENARIOS
        for pair in schedule
        for candidate in pair["order"]
    }
    if set(status_by_run) != expected_keys:
        raise ValueError("status map attempts do not match frozen manifest schedule")

    for scenario in SCENARIOS:
        plan = scenarios[scenario]
        planned_stages = ("official-w2-cold", *STAGES) if scenario == "w2" else STAGES
        qps_by_stage = {
            "normal-reference": plan["normal_reference_qps"],
            "common-load": plan["common_load_qps"],
            "near-saturation": plan["near_saturation_qps"],
            "overload": plan["overload_qps"],
            "recovery": plan["normal_reference_qps"],
        }
        if scenario == "w2":
            qps_by_stage["official-w2-cold"] = plan["normal_reference_qps"]
        for pair in schedule:
            repetition = int(pair["repetition"])
            for position, candidate in enumerate(pair["order"], 1):
                run_key = (scenario, repetition, candidate)
                status = status_by_run[run_key]
                run_dir = results_root / scenario / f"repetition-{repetition}" / candidate
                for stage_name in planned_stages:
                    group = pair_groups.get(group_key(scenario, stage_name))
                    if group is None:
                        raise ValueError(f"missing paired aggregate group {scenario}/{stage_name}")
                    invalid_observations = {
                        (int(invalid["repetition"]), invalid["candidate"]): invalid["reason"]
                        for invalid in group.get("invalid_pairs", [])
                    }
                    valid_repetitions = {
                        scheduled_pair["repetition"]
                        for scheduled_pair in schedule
                        if all(
                            (scheduled_pair["repetition"], member) not in invalid_observations
                            for member in ("go", "rust")
                        )
                    }
                    if len(valid_repetitions) != group["valid_pairs"]:
                        raise ValueError(f"paired-valid count disagrees with invalid observations for {scenario}/{stage_name}")
                    pair_valid = repetition in valid_repetitions
                    reason = invalid_observations.get((repetition, candidate), "")

                    stage_file, resource_file = stage_paths(run_dir, scenario, stage_name)
                    rows = jsonl(stage_file) if stage_file.exists() else []
                    matching = [row for row in rows if row.get("stage") == stage_name]
                    if len(matching) > 1:
                        raise ValueError(f"duplicate stage {scenario}/{repetition}/{candidate}/{stage_name}")
                    stage = matching[0] if matching else {}
                    counters = stage.get("counters", {})
                    latency = stage.get("latency_samples_us", [])
                    stage_rows.append(
                        {
                            "scenario": scenario,
                            "repetition": repetition,
                            "position": position,
                            "candidate": candidate,
                            "stage": stage_name,
                            "target_qps": stage.get("target_qps", qps_by_stage[stage_name]),
                            "duration_ms": stage.get("duration_ms", plan["stage_duration_ms"]),
                            "runner_exit": status["runner_exit"],
                            "stage_invalid_reason": reason or ("missing stage result" if not stage else ""),
                            "valid_pairs": group["valid_pairs"],
                            "complete_pairs": group["complete_pairs"],
                            "pair_valid": str(pair_valid).lower(),
                            "correct_on_time": counters.get("correct_on_time", ""),
                            "scheduled": counters.get("scheduled", ""),
                            "sent": counters.get("sent", ""),
                            "received": counters.get("received", ""),
                            "correct_late": counters.get("correct_late", ""),
                            "expected_negative_on_time": counters.get("expected_negative_on_time", ""),
                            "wrong_response": counters.get("wrong_response", ""),
                            "protocol_error": counters.get("protocol_error", ""),
                            "transport_error": counters.get("transport_error", ""),
                            "timeout": counters.get("timeout", ""),
                            "sender_shortfall": counters.get("sender_shortfall", ""),
                            "latency_sample_count": len(latency),
                            "p50_us": stage.get("p50_us", ""),
                            "p95_us": stage.get("p95_us", ""),
                            "p99_us": stage.get("p99_us", ""),
                            "effective_throughput_qps": stage.get("effective_throughput_qps", ""),
                        }
                    )

                    if not stage:
                        continue
                    resource_samples = [
                        sample for sample in jsonl(resource_file)
                        if sample.get("stage_id") == stage_name
                    ] if resource_file.exists() else []
                    expected_counts = stage.get("resource_sample_counts", {})
                    for role, expected_count in expected_counts.items():
                        samples = sorted(
                            [sample for sample in resource_samples if sample.get("role") == role],
                            key=lambda sample: sample["timestamp"],
                        )
                        if len(samples) != expected_count or len(samples) < 2:
                            raise ValueError(
                                f"resource sample coverage mismatch for {scenario}/{repetition}/{candidate}/{stage_name}/{role}: "
                                f"got {len(samples)}, expected {expected_count}"
                            )
                        pids = {sample["pid"] for sample in samples}
                        clocks = {sample["clock_ticks_per_second"] for sample in samples}
                        if len(pids) != 1 or len(clocks) != 1:
                            raise ValueError(f"resource identity/tick-rate changed within {scenario}/{repetition}/{candidate}/{stage_name}/{role}")
                        first, last = samples[0], samples[-1]
                        interval = (parse_timestamp(last["timestamp"]) - parse_timestamp(first["timestamp"])).total_seconds()
                        clock = int(first["clock_ticks_per_second"])
                        cpu_ticks = (int(last["user_ticks"]) - int(first["user_ticks"])) + (int(last["system_ticks"]) - int(first["system_ticks"]))
                        cpu_ms = cpu_ticks * 1000 / clock
                        correct = int(counters.get("correct_on_time", 0))
                        resource_rows.append(
                            {
                                "scenario": scenario,
                                "repetition": repetition,
                                "position": position,
                                "candidate": candidate,
                                "stage": stage_name,
                                "role": role,
                                "pid": next(iter(pids)),
                                "sample_count": len(samples),
                                "sample_interval_s": round(interval, 6),
                                "cpu_ticks": cpu_ticks,
                                "user_system_cpu_ms": round(cpu_ms, 3),
                                "cpu_percent_one_core": round(100 * cpu_ms / (interval * 1000), 3) if interval > 0 else "",
                                "cpu_resolution_percent_one_core": round(100 * (1 / clock) / interval, 3) if interval > 0 else "",
                                "cpu_us_per_correct_on_time": round(cpu_ms * 1000 / correct, 3) if role == "sut" and correct > 0 and cpu_ticks > 0 else "",
                                "cpu_resolution_us_per_correct_on_time": round(1_000_000 / clock / correct, 3) if role == "sut" and correct > 0 else "",
                                "rss_median_kib": statistics.median(int(sample["rss_kib"]) for sample in samples),
                                "rss_peak_kib": max(int(sample["rss_kib"]) for sample in samples),
                                "fd_median": statistics.median(int(sample["fd_count"]) for sample in samples),
                                "fd_peak": max(int(sample["fd_count"]) for sample in samples),
                                "stage_invalid_reason": reason,
                                "pair_valid": str(pair_valid).lower(),
                            }
                        )

    write_tsv(stage_output, STAGE_FIELDS, stage_rows)
    write_tsv(resource_output, RESOURCE_FIELDS, resource_rows)
    return len(stage_rows), len(resource_rows)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--matrix-driver", type=Path, required=True)
    parser.add_argument("--matrix-driver-sha256", required=True)
    parser.add_argument("--results-root", type=Path, required=True)
    parser.add_argument("--status-map", type=Path, required=True)
    parser.add_argument("--status-map-sha256", required=True)
    parser.add_argument("--aggregation", type=Path, required=True)
    parser.add_argument("--aggregation-sha256", required=True)
    parser.add_argument("--stage-output", type=Path, required=True)
    parser.add_argument("--resource-output", type=Path, required=True)
    args = parser.parse_args()
    try:
        stage_count, resource_count = summarize(
            args.manifest, args.manifest_sha256, args.matrix_driver, args.matrix_driver_sha256,
            args.results_root, args.status_map, args.status_map_sha256, args.aggregation,
            args.aggregation_sha256, args.stage_output, args.resource_output,
        )
    except (OSError, KeyError, TypeError, ValueError) as error:
        parser.error(str(error))
    print(f"stage_observations={stage_count}")
    print(f"resource_observations={resource_count}")
    print("raw_results_modified=false")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
