#!/usr/bin/env python3
"""VM-only private production-shape rehearsal; publishes summary.json, no inputs.

Input root contains snapshot/, the four local configuration zip files, built
mosdns-0 / mosdns-1 / mosdns-settings-migrate and go-bin/mosdns. Never accesses
public networking or the original system service except to compare its identity.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tarfile
import threading
import time
import urllib.request
import zipfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "e2e-1"))
from prepare import prepare  # noqa: E402


def call(argv, **kwargs):
    return subprocess.run(list(map(str, argv)), check=True, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, timeout=300, **kwargs).stdout


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n")


def wait_for(f, seconds=60):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        try:
            result = f()
            if result:
                return result
        except (OSError, ValueError, KeyError, subprocess.SubprocessError):
            pass
        time.sleep(.02)
    raise RuntimeError("timed out")


def props(unit):
    return dict(line.split("=", 1) for line in call(["systemctl", "show", unit,
        "-p", "MainPID", "-p", "ActiveState", "-p", "NRestarts"]).decode().splitlines())


class Probe:
    """Continuous 20 ms UDP query stream, max 200 ms outstanding wait."""
    def __init__(self, root):
        self.stop = threading.Event()
        self.samples = []
        self.root = root
        self.thread = threading.Thread(target=self.loop)

    def loop(self):
        wire = b"\x42\x42\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00"
        for part in "e2e1-rewrite-ip.example.com".split("."):
            wire += bytes([len(part)]) + part.encode()
        wire += b"\x00\x00\x01\x00\x01"
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(.2)
            while not self.stop.is_set():
                begin = time.monotonic()
                ok = False
                try:
                    sock.sendto(wire, ("127.0.0.1", 53))
                    result = sock.recv(4096)
                    ok = result[:2] == b"\x42\x42" and result[3] & 15 == 0 and result[-4:] == bytes([192, 0, 2, 77])
                except OSError:
                    pass
                self.samples.append((time.monotonic(), ok))
                self.stop.wait(max(0, .02 - (time.monotonic()-begin)))

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *args):
        self.stop.set()
        self.thread.join()

    def report(self):
        successes = [t for t, ok in self.samples if ok]
        return {"sent": len(self.samples), "failed": sum(not ok for _, ok in self.samples),
                "max_success_gap_seconds": round(max((b-a for a, b in zip(successes, successes[1:])), default=0), 4)}


def api(port, path, data=None):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    req = urllib.request.Request(f"http://127.0.0.1:{port}" + path,
                                 data=None if data is None else json.dumps(data).encode(),
                                 headers={"Content-Type": "application/json"})
    with opener.open(req, timeout=10) as response:
        return json.load(response)


def run(work):
    source = HERE.parent.parent
    original = props("mosdns")
    write(work / "service-before.json", original)
    inputs = work / "inputs"
    inputs.mkdir()
    shutil.copytree(work / "snapshot", inputs / "snapshot")
    for name in ("config_all", "config_rust"):
        with zipfile.ZipFile(work / (name + ".zip")) as archive:
            archive.extractall(inputs / name)
    prepared = work / "prepared"
    prepared.mkdir()
    prepare(prepared, inputs, work / "mosdns-settings-migrate", 1)
    root = work / "live"
    shutil.copytree(prepared / "go", root)
    # Management port is a disposable-copy override; original snapshot is unchanged.
    write(root / "webinfo/webui_port_settings.json", {"port": 9099})
    # Explicit schema-4 baseline, whose managed bytes are checked against the package.
    write(root / "webinfo/config_update_state.json", {"format": 1, "applied_schema": 4,
          "required_schema": 4, "package_id": "main-config-schema-4", "status": "success"})
    mock = work / "mock"
    mock.mkdir()
    call(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", mock / "ca-key.pem",
          "-out", mock / "ca.pem", "-days", "2", "-subj", "/CN=F3b-CA", "-addext", "basicConstraints=critical,CA:TRUE"])
    call(["openssl", "req", "-newkey", "rsa:2048", "-nodes", "-keyout", mock / "key.pem",
          "-out", mock / "leaf.csr", "-subj", "/CN=localhost"])
    (mock / "leaf.ext").write_text("subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n")
    call(["openssl", "x509", "-req", "-in", mock / "leaf.csr", "-CA", mock / "ca.pem", "-CAkey", mock / "ca-key.pem",
          "-CAcreateserial", "-out", mock / "cert.pem", "-days", "2", "-extfile", mock / "leaf.ext"])
    tmp = work / "private-tmp"
    tmp.mkdir()
    binary = work / "mosdns"
    shutil.copy2(work / "go-bin/mosdns", binary)
    binary.chmod(0o755)
    env_file = work / "environment"
    env_file.write_text("MOSDNS_CACHE_BACKEND=go\n")
    ns = "f3b-20261018"
    unit = "mosdns-f3b-20261018.service"
    unit_path = Path("/etc/systemd/system") / unit
    unit_path.write_text(f"[Unit]\nDescription=F3b isolated rehearsal\nConditionFileIsExecutable={binary}\n"
        f"[Service]\nExecStart={binary} start -d {root} -c {root}/config_custom.yaml\nRestart=always\nRestartSec=120\n"
        f"EnvironmentFile=-{env_file}\nEnvironment=SSL_CERT_FILE={mock}/ca.pem\nEnvironment=MOSDNS_UPDATE_API_URL=http://127.0.0.1:18081/releases\n"
        f"NetworkNamespacePath=/run/netns/{ns}\nBindPaths={tmp}:/tmp\n")
    call(["systemctl", "daemon-reload"])
    call(["systemctl", "start", unit])
    mock_log = (work / "mock-process.log").open("w")
    mock_process = subprocess.Popen(["python3", str(source / "tools/e2e-1/mock.py"), str(mock)],
                                    stdout=mock_log, stderr=subprocess.STDOUT)
    processes = [mock_process]
    release_process = None
    results = []
    txs = []
    try:
        wait_for(lambda: (mock / "mock-ready").exists())
        wait_for(lambda: api(9099, "/api/v1/system/health").get("ready"))
        args = ["bash", str(source / "scripts/cutover/go-to-rust.sh"), "--rust-binary", str(work / "mosdns-0"),
            "--migrate", str(work / "mosdns-settings-migrate"), "--config-package", str(work / "config_rust_up.zip"),
            "--go-reference", str(work / "config_all.zip"), "--go-package", str(work / "config_up.zip"),
            "--root", str(root), "--binary", str(binary), "--service", unit, "--environment-file", str(env_file),
            "--dns-name", "e2e1-rewrite-ip.example.com", "--expect-answer", "192.0.2.77", "--timeout", "8"]
        # Production-copy YAML deviations are private inputs; the test explicitly
        # authorizes replacement in this disposable root, never on a real host.
        with zipfile.ZipFile(work / "config_up.zip") as z:
            manifest = json.loads(z.read("manifest.json"))
        with zipfile.ZipFile(work / "config_all.zip") as z:
            for name in manifest["managed_files"]:
                if not (root / name).exists() or (root / name).read_bytes() != z.read(name):
                    args += ["--accept-diff", name]
        env = dict(os.environ, MOSDNS_CUTOVER_NETNS=ns)

        def transact(label, extra=(), expected=0, base=None):
            before = set(work.glob(".mosdns-cutover-*"))
            with Probe(root) as probe:
                time.sleep(.15)
                result = subprocess.run([*(base or args), *extra], env=env, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, timeout=300)
                time.sleep(.15)
            (work / (label + ".log")).write_bytes(result.stdout)
            assert result.returncode == expected, (label, result.returncode, "see private log")
            created = set(work.glob(".mosdns-cutover-*")) - before
            tx = next(iter(created)) if created else None
            if tx:
                txs.append(tx)
            result_row = {"case": label, "returncode": result.returncode, "dns": probe.report(),
                "health_version": api(9099, "/api/v1/system/health")["version"], "service": props(unit)}
            if tx:
                state = json.loads((tx / "state.json").read_text())
                result_row.update(phase=state["phase"], controlled_outage_seconds=state.get("outage_seconds"))
            results.append(result_row)
            write(work / "summary.json", results)
            return tx

        before_pid = props(unit)["MainPID"]
        dry = transact("01-dry-run", ["--dry-run"])
        assert props(unit)["MainPID"] == before_pid and binary.read_bytes() == (work / "go-bin/mosdns").read_bytes()
        tx = transact("02-cutover")
        assert api(9099, "/api/v1/system/health")["version"] == "rust-v0.1.0"

        # F-3a final-code binary+package update against a local fake release.
        releases = work / "releases"
        releases.mkdir()
        archive_path = releases / "mosdns-rust-linux-amd64.tar.gz"
        with tarfile.open(archive_path, "w:gz") as tar:
            tar.add(work / "mosdns-1", arcname="mosdns")
        tag = "rust-v0.1.1"
        names = [archive_path.name, "mosdns-rust-update-manifest.json"]
        sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
        write(releases / names[1], {"format": 1, "channel": "rust", "version": tag,
              "required_config_schema": 1, "config_package_id": "rust-config-schema-1",
              "artifacts": {names[0]: {"sha256": sha(archive_path)}},
              "config": {"url": "http://127.0.0.1:18081/config_rust_up.zip", "sha256": sha(work / "config_rust_up.zip")}})
        shutil.copy2(work / "config_rust_up.zip", releases / "config_rust_up.zip")
        write(releases / "releases", [{"tag_name": tag, "draft": False, "prerelease": False,
            "published_at": "2026-10-18T00:00:00Z", "assets": [{"name": name,
                "browser_download_url": "http://127.0.0.1:18081/"+name} for name in names]}])
        release_log = (work / "release.log").open("w")
        release_process = subprocess.Popen(["python3", "-m", "http.server", "18081", "--bind", "127.0.0.1",
                                           "--directory", str(releases)], stdout=release_log, stderr=subprocess.STDOUT)
        processes.append(release_process)
        wait_for(lambda: subprocess.run(["curl", "-fsS", "http://127.0.0.1:18081/releases"], stdout=subprocess.DEVNULL).returncode == 0)
        # Also reapply schema 1: proves the final updater handles all 17 files.
        state = json.loads((root / "webinfo/rust_config_update_state.json").read_text())
        state["applied_schema"] = 0
        write(root / "webinfo/rust_config_update_state.json", state)
        pid_before = props(unit)["MainPID"]
        with Probe(root) as probe:
            api(9099, "/api/v1/update/apply", {"force": True})
            wait_for(lambda: api(9099, "/api/v1/system/health").get("version") == tag, 120)
            wait_for(lambda: json.loads((root / "update/installed.json").read_text()).get("version") == tag, 120)
            time.sleep(.3)
        assert props(unit)["MainPID"] == pid_before
        results.append({"case": "03-online-update", "dns": probe.report(), "health_version": tag,
                        "service": props(unit), "same_main_pid": True})
        write(work / "summary.json", results)
        rollback = ["bash", str(source / "scripts/cutover/rust-to-go.sh"), "--transaction", str(tx), "--acknowledge-data-loss"]
        (root / "rule/post-rust.txt").write_text("full:post-rust.example.com\n")
        transact("04-rollback", base=rollback)
        assert not (root / "rule/post-rust.txt").exists()
        assert list(tx.glob("rust-root-*/rule/post-rust.txt"))

        # Candidate passes check but one listener is occupied by the local stub.
        import yaml
        bad = work / "bad.zip"
        with zipfile.ZipFile(work / "config_rust_up.zip") as z:
            files = {n: z.read(n) for n in z.namelist()}
        main = yaml.safe_load(files["config_custom.yaml"])
        entry = next(p["args"]["entry"] for p in main["plugins"] if p["type"] == "udp_server")
        main["plugins"].append({"tag": "f3b_occupied", "type": "udp_server",
                                "args": {"entry": entry, "listen": "127.0.0.1:15000"}})
        files["config_custom.yaml"] = yaml.safe_dump(main, allow_unicode=True, sort_keys=False).encode()
        m = json.loads(files["manifest.json"])
        m["sha256"]["config_custom.yaml"] = hashlib.sha256(files["config_custom.yaml"]).hexdigest()
        files["manifest.json"] = json.dumps(m).encode()
        with zipfile.ZipFile(bad, "w") as z:
            for name, content in files.items():
                z.writestr(name, content)
        badargs = args[:]
        badargs[badargs.index("--config-package")+1] = str(bad)
        failed_tx = transact("05-auto-rollback", expected=1, base=badargs)
        assert json.loads((failed_tx / "state.json").read_text())["phase"] == "rolled_back"

        # Full config lives on a deliberately tiny tmpfs. Never fill the real disk.
        small = work / "small"
        small.mkdir()
        # systemd isolates this controller's mount namespace as a consequence of
        # NetworkNamespacePath. Mount on the VM host so the temporary DNS unit
        # and the detached cutover worker see the same small filesystem.
        call(["nsenter", "-t", "1", "-m", "--", "mount", "-t", "tmpfs", "-o", "size=32m", "tmpfs", small])
        try:
            call(["systemctl", "stop", unit])
            shutil.copytree(root, small / "live")
            old = unit_path.read_text()
            unit_path.write_text(old.replace(str(root), str(small / "live")))
            call(["systemctl", "daemon-reload"])
            call(["systemctl", "start", unit])
            wait_for(lambda: api(9099, "/api/v1/system/health").get("ready"))
            smallargs = args[:]
            smallargs[smallargs.index("--root")+1] = str(small / "live")
            transact("06-space-refusal", expected=1, base=smallargs)
            assert not list(small.glob(".mosdns-cutover-*"))
            assert b"insufficient space" in (work / "06-space-refusal.log").read_bytes()
        finally:
            call(["systemctl", "stop", unit])
            unit_path.write_text(old)
            call(["systemctl", "daemon-reload"])
            call(["systemctl", "start", unit])
            call(["nsenter", "-t", "1", "-m", "--", "umount", small])
        wait_for(lambda: api(9099, "/api/v1/system/health").get("ready"))

        # Stop the independent worker at its committed directory-rename intent,
        # SIGKILL it, then recover using the public rollback entry point.
        before = set(work.glob(".mosdns-cutover-*"))
        log = (work / "07-interrupt.log").open("w")
        with Probe(root) as probe:
            monitor = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT)
            processes.append(monitor)
            tx7 = wait_for(lambda: next(iter(set(work.glob(".mosdns-cutover-*"))-before), None))
            end = time.monotonic()+120
            caught = None
            while time.monotonic() < end:
                try:
                    current = json.loads((tx7 / "state.json").read_text())
                    if current["phase"] in ("binary_installed", "root_moving", "root_moved", "root_installing"):
                        worker_unit = current["worker_unit"] + ".service"
                        call(["systemctl", "kill", "--signal=SIGSTOP", worker_unit])
                        caught = json.loads((tx7 / "state.json").read_text())["phase"]
                        call(["systemctl", "kill", "--signal=SIGKILL", worker_unit])
                        break
                except (OSError, ValueError, subprocess.SubprocessError):
                    pass
                time.sleep(.001)
            assert caught, "missed the interruption window"
            monitor.wait(timeout=10)
            txs.append(tx7)
            recover = rollback[:]
            recover[recover.index("--transaction")+1] = str(tx7)
            result = subprocess.run(recover, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
            (work / "07-recovery.log").write_bytes(result.stdout)
            assert result.returncode == 0
            time.sleep(.2)
        results.append({"case": "07-kill-and-recover", "killed_phase": caught, "dns": probe.report(),
                        "health_version": api(9099, "/api/v1/system/health")["version"], "service": props(unit)})
        write(work / "summary.json", results)
    finally:
        call(["systemctl", "stop", unit])
        for process in processes:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        for tx in work.glob(".mosdns-cutover-*"):
            if (tx / "state.json").exists():
                s = json.loads((tx / "state.json").read_text())
                for worker_unit in s.get("worker_units", [s.get("worker_unit")]):
                    if worker_unit:
                        subprocess.run(["systemctl", "stop", worker_unit+".service"], stdout=subprocess.DEVNULL)
                        subprocess.run(["systemctl", "reset-failed", worker_unit+".service"],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        unit_path.unlink()
        call(["systemctl", "daemon-reload"])
        subprocess.run(["systemctl", "reset-failed", unit], stdout=subprocess.DEVNULL)
        after = props("mosdns")
        write(work / "cleanup.json", {"original_service_unchanged": after == original,
              "original_service": after, "owned_processes_exited": all(p.poll() is not None for p in processes)})
        assert after == original


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    run(args.work.resolve())
