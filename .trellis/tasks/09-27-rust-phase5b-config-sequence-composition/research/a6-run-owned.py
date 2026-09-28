#!/usr/bin/env python3
"""Run one isolated Rust test binary and record process/socket cleanup."""

from __future__ import annotations

import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path


def process_identity(pid: int) -> dict[str, int | str] | None:
    proc = Path(f"/proc/{pid}")
    try:
        stat = (proc / "stat").read_text()
        fields = stat[stat.rfind(")") + 2 :].split()
        return {
            "ppid": int(fields[1]),
            "process_group": int(fields[2]),
            "starttime_ticks": int(fields[19]),
            "executable": os.path.realpath(proc / "exe"),
        }
    except (FileNotFoundError, IndexError, OSError, ValueError):
        return None


def listener_snapshot() -> str:
    result = subprocess.run(
        ["ss", "-Hlntup"], check=True, capture_output=True, text=True
    )
    return "\n".join(sorted(result.stdout.splitlines())) + "\n"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def verified_signal(pid: int, expected: dict[str, int | str], signum: int) -> None:
    current = process_identity(pid)
    if current != expected or current["process_group"] != pid:
        raise RuntimeError(f"refusing signal: owned process identity changed: {current}")
    os.killpg(pid, signum)


def main() -> int:
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} TEMP_ROOT TEST_BINARY", file=sys.stderr)
        return 64
    root = Path(sys.argv[1]).resolve()
    binary = Path(sys.argv[2]).resolve()
    if not root.is_dir() or not os.access(binary, os.X_OK):
        print("temporary root or test binary is unavailable", file=sys.stderr)
        return 66

    before = listener_snapshot()
    started_at = time.time()
    process = subprocess.Popen(
        [str(binary), "--test-threads=1", "--nocapture"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        start_new_session=True,
    )
    identity = process_identity(process.pid)
    if identity is None or identity["process_group"] != process.pid:
        # Do not signal an identity that cannot be established.
        output, _ = process.communicate(timeout=10)
        (root / "a6-exact-rule-test.log").write_text(output)
        (root / "a6-exact-rule-run.json").write_text(
            json.dumps(
                {"pid": process.pid, "identity": identity, "exit_code": process.returncode},
                indent=2,
            )
            + "\n"
        )
        return 125

    timed_out = False
    cleanup_signals: list[str] = []
    try:
        output, _ = process.communicate(timeout=180)
    except subprocess.TimeoutExpired:
        timed_out = True
        verified_signal(process.pid, identity, signal.SIGTERM)
        cleanup_signals.append("SIGTERM")
        try:
            output, _ = process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            verified_signal(process.pid, identity, signal.SIGKILL)
            cleanup_signals.append("SIGKILL")
            output, _ = process.communicate()

    (root / "a6-exact-rule-test.log").write_text(output)
    return_code = process.returncode
    final_identity = process_identity(process.pid)
    after = listener_snapshot()
    marker_lines = re.findall(r"A6_EXACT_ENDPOINTS [^\r\n]+", output)
    ports: list[int] = []
    for line in marker_lines:
        ports.extend(int(port) for port in re.findall(r"127\.0\.0\.1:(\d+)", line))
    leaked_ports = [
        port
        for port in sorted(set(ports))
        if any(re.search(rf":{port}(?!\d)", line) for line in after.splitlines())
    ]
    fixture_dirs = sorted(str(path) for path in Path("/tmp").glob(f"phase5b-exact-rule-*-{process.pid}"))
    result = {
        "pid": process.pid,
        "identity": identity,
        "final_identity": final_identity,
        "exit_code": return_code,
        "timed_out": timed_out,
        "cleanup_signals": cleanup_signals,
        "duration_seconds": round(time.time() - started_at, 3),
        "endpoints": marker_lines,
        "leaked_ports": leaked_ports,
        "fixture_dirs_remaining": fixture_dirs,
        "listener_snapshot_before_sha256": sha256(before.encode()),
        "listener_snapshot_after_sha256": sha256(after.encode()),
    }
    (root / "a6-exact-rule-run.json").write_text(json.dumps(result, indent=2) + "\n")
    if return_code != 0 or timed_out or final_identity == identity or leaked_ports or fixture_dirs:
        print(json.dumps(result, indent=2), file=sys.stderr)
        return 1 if return_code == 0 else return_code
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
