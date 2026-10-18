#!/usr/bin/env python3
"""Run a fixed 30-invocation batch per test binary, two default-thread lanes.

Only a test helper: never builds, touches service state, or replaces failed runs.
Use a fresh output directory for each source revision. Logs are retained.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import hashlib
from pathlib import Path
import subprocess
import time

FILES = ("f1_system", "f2_diagnostics", "f3a_update", "m1_0_config_loading")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--deps", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(mode=0o700)
    binaries = {}
    for name in FILES:
        matches = [path for path in a.deps.glob(name + "-*") if path.is_file() and not path.suffix]
        if len(matches) != 1:
            raise RuntimeError(f"expected exactly one binary for {name}: {matches}")
        binaries[name] = matches[0].resolve()
    identities = {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in binaries.items()}
    (a.output / "binary-sha256.json").write_text(json.dumps(identities, indent=2)+"\n")

    def lane(index):
        rows = []
        for name in FILES:
            for number in range(1, 16):
                label = f"{name}-lane{index}-{number:02d}"
                start = time.monotonic()
                with (a.output / (label + ".log")).open("wb") as log:
                    try:
                        result = subprocess.run([str(binaries[name])], stdout=log,
                                                stderr=subprocess.STDOUT, timeout=300,
                                                cwd=binaries[name].parents[3] / "native-host")
                        status = result.returncode
                    except subprocess.TimeoutExpired:
                        status = "timeout"
                row = {"file": name, "lane": index, "iteration": number,
                       "status": status, "seconds": round(time.monotonic()-start, 3)}
                rows.append(row)
                (a.output / (label + ".json")).write_text(json.dumps(row)+"\n")
        return rows

    with ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(lane, i) for i in (1, 2)]
        rows = [row for future in futures for row in future.result()]
    summary = {name: {"runs": sum(r["file"] == name for r in rows),
                      "failed": sum(r["file"] == name and r["status"] != 0 for r in rows)}
               for name in FILES}
    if identities != {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in binaries.items()}:
        raise RuntimeError("test binaries changed during the batch")
    (a.output / "summary.json").write_text(json.dumps(summary, indent=2)+"\n")
    print(json.dumps(summary), flush=True)
    return int(any(r["status"] != 0 for r in rows))


if __name__ == "__main__":
    raise SystemExit(main())
