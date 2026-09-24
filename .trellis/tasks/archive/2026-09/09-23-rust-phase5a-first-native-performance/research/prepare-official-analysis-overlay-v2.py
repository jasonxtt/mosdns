#!/usr/bin/env python3
"""Create a read-only raw-results overlay with driver exit status sidecars."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import re
import shutil
import tempfile
from datetime import datetime
from pathlib import Path


SCENARIOS = ("w1-udp", "w1-tcp", "w2", "w3")
STATUS_FIELDS = (
    "scenario",
    "repetition",
    "position",
    "candidate",
    "start_utc",
    "end_utc",
    "runner_exit",
    "loadavg_before",
    "loadavg_after",
)
MAP_FIELDS = (
    "scenario",
    "repetition",
    "position",
    "candidate",
    "runner_exit",
    "start_utc",
    "end_utc",
    "raw_run_dir",
    "overlay_run_dir",
    "status_source_sha256",
    "manifest_sha256",
    "matrix_driver_sha256",
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_kv(path: Path) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition("=")
        if not separator or not key or key in fields:
            raise ValueError(f"invalid or duplicate key in {path}: {line!r}")
        fields[key] = value
    return fields


def expected_attempts(manifest: dict) -> list[tuple[str, int, int, str]]:
    pairs = manifest.get("pair_schedule")
    scenarios = manifest.get("scenarios")
    if not isinstance(pairs, list) or len(pairs) != 3 or not isinstance(scenarios, dict):
        raise ValueError("manifest must contain three paired repetitions and scenarios")
    expected: list[tuple[str, int, int, str]] = []
    for scenario in SCENARIOS:
        if scenario not in scenarios:
            raise ValueError(f"manifest is missing scenario {scenario}")
        for pair in pairs:
            repetition = pair.get("repetition")
            order = pair.get("order")
            if not isinstance(repetition, int) or order not in (["go", "rust"], ["rust", "go"]):
                raise ValueError(f"invalid paired schedule row: {pair!r}")
            for position, candidate in enumerate(order, 1):
                expected.append((scenario, repetition, position, candidate))
    if len(expected) != 24 or len(set(expected)) != 24:
        raise ValueError("manifest does not define exactly 24 unique matrix attempts")
    return expected


def load_attempt_status(path: Path, expected: list[tuple[str, int, int, str]]) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as stream:
        reader = csv.DictReader(stream, delimiter="\t")
        if tuple(reader.fieldnames or ()) != STATUS_FIELDS:
            raise ValueError(f"unexpected attempt-status.tsv header: {reader.fieldnames!r}")
        rows = list(reader)
    actual: list[tuple[str, int, int, str]] = []
    for row in rows:
        try:
            identity = (row["scenario"], int(row["repetition"]), int(row["position"]), row["candidate"])
            runner_exit = int(row["runner_exit"])
            started = datetime.fromisoformat(row["start_utc"].replace("Z", "+00:00"))
            ended = datetime.fromisoformat(row["end_utc"].replace("Z", "+00:00"))
        except (KeyError, ValueError) as error:
            raise ValueError(f"invalid attempt status row: {row!r}: {error}") from error
        if runner_exit < 0 or ended < started:
            raise ValueError(f"invalid exit code or time interval in attempt status row: {row!r}")
        actual.append(identity)
    if actual != expected:
        raise ValueError("attempt-status.tsv does not contain the exact frozen 24-row schedule in order")
    return rows


def prepare_overlay(
    manifest_path: Path,
    manifest_digest: str,
    driver_path: Path,
    driver_digest: str,
    status_path: Path,
    results_root: Path,
    overlay_root: Path,
    mapping_path: Path,
) -> int:
    if not re.fullmatch(r"[0-9a-f]{64}", manifest_digest) or sha256(manifest_path) != manifest_digest:
        raise ValueError("manifest does not match the reviewed SHA-256")
    if not re.fullmatch(r"[0-9a-f]{64}", driver_digest) or sha256(driver_path) != driver_digest:
        raise ValueError("matrix driver does not match the reviewed SHA-256")
    sidecar = manifest_path.with_suffix(".sha256")
    sidecar_fields = sidecar.read_text(encoding="utf-8").split()
    if sidecar_fields != [manifest_digest, manifest_path.name]:
        raise ValueError("manifest sidecar does not exactly match the reviewed manifest")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1 or manifest.get("official_frozen") is not True:
        raise ValueError("analysis requires the frozen v1 official manifest")
    expected = expected_attempts(manifest)
    rows = load_attempt_status(status_path, expected)
    status_digest = sha256(status_path)

    if not results_root.is_dir():
        raise ValueError(f"missing official results root: {results_root}")
    if overlay_root.exists() or mapping_path.exists():
        raise FileExistsError("refusing to replace an existing analysis overlay or status map")
    overlay_root.parent.mkdir(parents=True, exist_ok=True)
    mapping_path.parent.mkdir(parents=True, exist_ok=True)
    overlay_temp = Path(tempfile.mkdtemp(prefix=f".{overlay_root.name}.tmp-", dir=overlay_root.parent))
    mapping_temp: Path | None = None
    try:
        fd, temporary_name = tempfile.mkstemp(prefix=f".{mapping_path.name}.tmp-", dir=mapping_path.parent)
        mapping_temp = Path(temporary_name)
        with os.fdopen(fd, "w", encoding="utf-8", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=MAP_FIELDS, delimiter="\t", lineterminator="\n")
            writer.writeheader()
            for row in rows:
                scenario = row["scenario"]
                repetition = int(row["repetition"])
                position = int(row["position"])
                candidate = row["candidate"]
                raw_dir = results_root / scenario / f"repetition-{repetition}" / candidate
                metadata = read_kv(raw_dir / "run-metadata.txt")
                expected_metadata = {
                    "scenario": scenario,
                    "candidate": candidate,
                    "repetition": str(repetition),
                    "pair_position": str(position),
                    "run_mode": "official",
                    "manifest_sha256": manifest_digest,
                }
                for key, value in expected_metadata.items():
                    if metadata.get(key) != value:
                        raise ValueError(f"{raw_dir}: {key}={metadata.get(key)!r}, expected {value!r}")
                raw_status = raw_dir / "attempt-exit-status.txt"
                if raw_status.exists():
                    raise ValueError(f"raw attempt already has an exit sidecar: {raw_status}")

                relative_dir = Path(scenario) / f"repetition-{repetition}" / candidate
                overlay_dir = overlay_temp / relative_dir
                final_overlay_dir = overlay_root / relative_dir
                overlay_dir.mkdir(parents=True)
                for entry in raw_dir.iterdir():
                    if entry.is_symlink():
                        raise ValueError(f"raw attempt contains an unexpected symlink: {entry}")
                    (overlay_dir / entry.name).symlink_to(entry.resolve(), target_is_directory=entry.is_dir())
                (overlay_dir / "attempt-exit-status.txt").write_text(
                    f"exit={int(row['runner_exit'])}\n", encoding="utf-8"
                )
                writer.writerow(
                    {
                        "scenario": scenario,
                        "repetition": repetition,
                        "position": position,
                        "candidate": candidate,
                        "runner_exit": row["runner_exit"],
                        "start_utc": row["start_utc"],
                        "end_utc": row["end_utc"],
                        "raw_run_dir": str(raw_dir),
                        "overlay_run_dir": str(final_overlay_dir),
                        "status_source_sha256": status_digest,
                        "manifest_sha256": manifest_digest,
                        "matrix_driver_sha256": driver_digest,
                    }
                )
        os.rename(overlay_temp, overlay_root)
        os.rename(mapping_temp, mapping_path)
    except BaseException:
        if overlay_temp.exists():
            shutil.rmtree(overlay_temp)
        if mapping_temp is not None and mapping_temp.exists():
            mapping_temp.unlink()
        raise
    return len(rows)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--matrix-driver", type=Path, required=True)
    parser.add_argument("--matrix-driver-sha256", required=True)
    parser.add_argument("--attempt-status", type=Path, required=True)
    parser.add_argument("--results-root", type=Path, required=True)
    parser.add_argument("--overlay-root", type=Path, required=True)
    parser.add_argument("--status-map", type=Path, required=True)
    args = parser.parse_args()
    try:
        count = prepare_overlay(
            args.manifest,
            args.manifest_sha256,
            args.matrix_driver,
            args.matrix_driver_sha256,
            args.attempt_status,
            args.results_root,
            args.overlay_root,
            args.status_map,
        )
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(f"analysis_overlay_attempts={count}")
    print(f"overlay_root={args.overlay_root}")
    print(f"status_map={args.status_map}")
    print("raw_results_modified=false")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
