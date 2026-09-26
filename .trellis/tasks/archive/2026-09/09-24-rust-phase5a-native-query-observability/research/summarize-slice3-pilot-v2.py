#!/usr/bin/env python3
"""Build deterministic primary-stage and paired-guard summaries from pilot raws."""

from __future__ import annotations

import csv
import json
import statistics
import sys
from pathlib import Path


ATTEMPT_FIELDS = (
    "scenario",
    "phase",
    "repetition",
    "variant",
    "pair_position",
    "runner_exit",
    "stage",
    "qps",
    "valid",
    "scheduled",
    "sent",
    "received",
    "correct_on_time",
    "correct_late",
    "wrong_response",
    "protocol_error",
    "transport_error",
    "timeout",
    "sender_shortfall",
    "p50_us",
    "p95_us",
    "p99_us",
    "effective_throughput_qps",
    "cpu_ticks",
    "cpu_us_per_correct_query",
    "rss_peak_kib_sampled",
    "host_load_start",
    "host_load_end",
)

ASSESSMENT_FIELDS = (
    "scenario",
    "phase",
    "qps",
    "comparison",
    "metric",
    "valid_pairs",
    "control_values",
    "candidate_values",
    "paired_delta_pct",
    "control_relative_mad",
    "guard",
    "pairs_above_guard",
    "tick_resolution_limited_pairs",
    "verdict",
)


def read_jsonl(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line]


def loadavg(result_dir: Path) -> tuple[str, str]:
    lines = (result_dir / "loadavg.tsv").read_text().splitlines()
    values: dict[str, str] = {}
    for i, line in enumerate(lines):
        if line.startswith("loadavg_before_utc=") and len(lines) > i + 1:
            values["start"] = " ".join(lines[i + 1].split()[:3])
        elif line.startswith("loadavg_after_utc=") and len(lines) > i + 1:
            values["end"] = " ".join(lines[i + 1].split()[:3])
    return values.get("start", ""), values.get("end", "")


def stage_invalid(result_dir: Path, phase: str, stage: str) -> bool:
    path = result_dir / "invalid-stages.tsv"
    if not path.exists():
        return False
    for line in path.read_text().splitlines():
        key = line.split("\t", 1)[0]
        if key == stage or (phase in ("cold", "warm") and key == f"w2-{phase}"):
            return True
    return False


def oracle_invalid(result_dir: Path) -> bool:
    path = result_dir / "invalid-stages.tsv"
    if not path.exists():
        return False
    return any(line.split("\t", 1)[0] in ("events", "counters") for line in path.read_text().splitlines())


def metric_valid(counters: dict, explicitly_invalid: bool) -> bool:
    return (
        not explicitly_invalid
        and counters.get("scheduled", 0) == counters.get("sent", -1)
        and counters.get("scheduled", 0) == counters.get("received", -1)
        and counters.get("scheduled", 0) == counters.get("correct_on_time", -1)
        and all(
            counters.get(name, 0) == 0
            for name in (
                "correct_late",
                "wrong_response",
                "protocol_error",
                "transport_error",
                "timeout",
                "sender_shortfall",
            )
        )
    )


def fmt(values: list[float | int]) -> str:
    return ",".join(f"{value:.4f}" if isinstance(value, float) else str(value) for value in values)


def median_relative_mad(values: list[float]) -> float:
    center = statistics.median(values)
    if center == 0:
        return 0.0
    return statistics.median(abs(value - center) for value in values) / center


def read_attempts(root: Path) -> tuple[list[dict], dict[tuple, dict]]:
    measurements: list[dict] = []
    indexed: dict[tuple, dict] = {}
    for line in (root / "attempt-order.tsv").read_text().splitlines():
        if line.startswith("scenario\t"):
            continue
        scenario, repetition, variant, pair_position, raw_path, runner_exit = line.split("\t")
        result_dir = root / Path(raw_path).name
        start_load, end_load = loadavg(result_dir)
        stage_files = sorted(result_dir.rglob("stages.jsonl"))
        for stage_file in stage_files:
            relative = stage_file.relative_to(result_dir).as_posix()
            if "/prefill/" in f"/{relative}/":
                continue
            if scenario == "w2":
                phase = "cold" if relative.startswith("w2-cold/") else "warm"
            else:
                phase = "main"
            resources = read_jsonl(stage_file.parent / "resource-samples.jsonl")
            by_stage: dict[str, list[dict]] = {}
            for sample in resources:
                if sample.get("role") == "sut":
                    by_stage.setdefault(sample.get("stage_id", ""), []).append(sample)
            for stage_row in read_jsonl(stage_file):
                stage = stage_row["stage"]
                qps = int(stage_row["target_qps"])
                if qps not in (200, 400):
                    continue
                if qps == 200:
                    expected_stage = "pilot-w2-cold" if phase == "cold" else "normal-reference"
                else:
                    expected_stage = "overload"
                if stage != expected_stage:
                    continue
                counters = stage_row["counters"]
                samples = by_stage.get(stage, [])
                total_ticks = [x["user_ticks"] + x["system_ticks"] for x in samples]
                tick_delta = max(total_ticks) - min(total_ticks) if total_ticks else 0
                rss_peak = max((x["rss_kib"] for x in samples), default=0)
                correct = counters.get("correct_on_time", 0)
                valid = metric_valid(
                    counters,
                    stage_invalid(result_dir, phase, stage) or oracle_invalid(result_dir),
                )
                record = {
                    "scenario": scenario,
                    "phase": phase,
                    "repetition": int(repetition),
                    "variant": variant,
                    "pair_position": int(pair_position),
                    "runner_exit": int(runner_exit),
                    "stage": stage,
                    "qps": qps,
                    "valid": int(valid),
                    **{key: counters.get(key, 0) for key in (
                        "scheduled", "sent", "received", "correct_on_time", "correct_late",
                        "wrong_response", "protocol_error", "transport_error", "timeout", "sender_shortfall",
                    )},
                    "p50_us": stage_row["p50_us"],
                    "p95_us": stage_row["p95_us"],
                    "p99_us": stage_row["p99_us"],
                    "effective_throughput_qps": stage_row["effective_throughput_qps"],
                    "cpu_ticks": tick_delta,
                    "cpu_us_per_correct_query": (tick_delta * 10_000.0 / correct) if correct else 0.0,
                    "rss_peak_kib_sampled": rss_peak,
                    "host_load_start": start_load,
                    "host_load_end": end_load,
                }
                measurements.append(record)
                key = (scenario, phase, int(repetition), variant, qps)
                indexed[key] = record
    return measurements, indexed


def paired_assessments(indexed: dict[tuple, dict]) -> list[dict]:
    groups = sorted({(key[0], key[1], key[4]) for key in indexed})
    output: list[dict] = []
    for scenario, phase, qps in groups:
        for comparison, control_name, candidate_name in (
            ("after_off_vs_before_off", "before_off", "after_off"),
            ("after_on_vs_after_off", "after_off", "after_on"),
        ):
            pairs = []
            for rep in (1, 2, 3):
                control = indexed.get((scenario, phase, rep, control_name, qps))
                candidate = indexed.get((scenario, phase, rep, candidate_name, qps))
                if control and candidate and control["valid"] and candidate["valid"]:
                    pairs.append((control, candidate))
            for metric in ("p95_us", "p99_us", "cpu_us_per_correct_query", "rss_peak_kib_sampled"):
                control_values = [float(pair[0][metric]) for pair in pairs]
                candidate_values = [float(pair[1][metric]) for pair in pairs]
                deltas_pct = [100.0 * (new / old - 1.0) for old, new in zip(control_values, candidate_values) if old]
                relative_mad = median_relative_mad(control_values) if control_values else 0.0
                below_tick_resolution = sum(abs(pair[1]["cpu_ticks"] - pair[0]["cpu_ticks"]) < 2 for pair in pairs)
                if metric in ("p95_us", "p99_us"):
                    guard = max(10.0, 200.0 * relative_mad)
                    above = sum(delta > guard for delta in deltas_pct)
                    if len(pairs) < 3:
                        verdict = "inconclusive: fewer than 3 valid pairs"
                    elif statistics.median(deltas_pct) > guard and above >= 2:
                        verdict = "repeatable regression"
                    else:
                        verdict = "no repeatable regression under frozen guard"
                    guard_text = f"{guard:.2f}%"
                elif metric == "cpu_us_per_correct_query":
                    guard = max(25.0, 200.0 * relative_mad)
                    above = sum(delta > guard for delta in deltas_pct)
                    if len(pairs) < 3:
                        verdict = "inconclusive: fewer than 3 valid pairs"
                    elif below_tick_resolution:
                        verdict = "inconclusive: 100-Hz ticks differ by <2 in one or more pairs"
                    elif statistics.median(deltas_pct) > guard and above >= 2:
                        verdict = "repeatable regression"
                    else:
                        verdict = "no repeatable regression under frozen guard"
                    guard_text = f"{guard:.2f}%"
                else:
                    if comparison == "after_off_vs_before_off" and control_values:
                        guard = max(8192.0, 0.25 * statistics.median(control_values), max(control_values) - min(control_values))
                    else:
                        guard = 32768.0
                    absolute_deltas = [new - old for old, new in zip(control_values, candidate_values)]
                    above = sum(delta > guard for delta in absolute_deltas)
                    if len(pairs) < 3:
                        verdict = "inconclusive: fewer than 3 valid pairs"
                    elif statistics.median(absolute_deltas) > guard and above >= 2:
                        verdict = "repeatable regression"
                    else:
                        verdict = "no repeatable regression under frozen guard"
                    guard_text = f"{guard:.0f} KiB"
                output.append({
                    "scenario": scenario,
                    "phase": phase,
                    "qps": qps,
                    "comparison": comparison,
                    "metric": metric,
                    "valid_pairs": len(pairs),
                    "control_values": fmt(control_values),
                    "candidate_values": fmt(candidate_values),
                    "paired_delta_pct": fmt(deltas_pct),
                    "control_relative_mad": f"{relative_mad:.4f}",
                    "guard": guard_text,
                    "pairs_above_guard": above,
                    "tick_resolution_limited_pairs": below_tick_resolution if metric == "cpu_us_per_correct_query" else "",
                    "verdict": verdict,
                })
    return output


def write_tsv(path: Path, fields: tuple[str, ...], rows: list[dict]) -> None:
    with path.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {Path(sys.argv[0]).name} RESULTS_DIR", file=sys.stderr)
        return 2
    root = Path(sys.argv[1])
    measurements, indexed = read_attempts(root)
    assessments = paired_assessments(indexed)
    write_tsv(root / "derived-primary-measurements.tsv", ATTEMPT_FIELDS, measurements)
    write_tsv(root / "derived-paired-assessments.tsv", ASSESSMENT_FIELDS, assessments)
    print(f"primary rows: {len(measurements)}")
    print(f"paired assessments: {len(assessments)}")
    print(f"valid primary rows: {sum(row['valid'] for row in measurements)}/{len(measurements)}")
    for row in assessments:
        print(
            f"{row['scenario']} {row['phase']} {row['qps']} {row['comparison']} "
            f"{row['metric']} n={row['valid_pairs']} deltas=[{row['paired_delta_pct']}] "
            f"guard={row['guard']} above={row['pairs_above_guard']} {row['verdict']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
