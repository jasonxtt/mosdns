#!/usr/bin/env python3
"""First Go -> Rust installation. Private, retained, recoverable transactions.

Both public commands detach their mutating phase into systemd. No live file is
deleted; rollback saves the current tree before restoring the Go installation.
"""
import argparse
import datetime
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import sys
import time
import urllib.request
import uuid
import zipfile


MIB = 1024 * 1024
GO_ID = "main-config-schema-4"
RUST_ID = "rust-config-schema-1"
TERMINAL = {"verified", "rolled_back", "recovery_required", "failed"}


def run(argv, timeout=120, **kwargs):
    return subprocess.run([str(a) for a in argv], check=True, timeout=timeout,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **kwargs).stdout


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic(path, data, mode=0o600, owner=None):
    path = Path(path)
    tmp = path.parent / ("." + path.name + ".cutover-" + uuid.uuid4().hex)
    try:
        with tmp.open("xb") as stream:
            os.chmod(tmp, mode)
            if owner:
                os.chown(tmp, *owner)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(tmp, path)
        sync_dir(path.parent)
    finally:
        if tmp.exists():
            tmp.unlink()  # Only our own temporary file.


def save(path, value):
    atomic(path, (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode())


def load(path):
    return json.loads(Path(path).read_bytes())


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(MIB), b""):
            h.update(block)
    return h.hexdigest()


def need(ok, message):
    if not ok:
        raise ValueError(message)


def safe_path(value):
    p = Path(value)
    need(p.is_absolute() and ".." not in p.parts, "use absolute paths without '..'")
    for q in (p, *p.parents):
        need(not q.is_symlink(), "symbolic link refused: " + str(q))
    return p


def safe_tree(root):
    safe_path(root)
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            p = Path(directory) / name
            mode = p.lstat().st_mode
            need(stat.S_ISREG(mode) or stat.S_ISDIR(mode), "non-regular input: " + str(p))
            need(not p.is_mount(), "nested mount refused: " + str(p))


def tree_size(root):
    block = os.statvfs(root).f_frsize
    return sum(max(block, math.ceil(p.stat().st_size / block) * block)
               for p in Path(root).rglob("*")) + block


def managed(name):
    return (name == "config_custom.yaml" or
            re.fullmatch(r"sub_config/[^/]+\.yaml", name) is not None) and name != "sub_config/special_groups.yaml"


def zip_entries(path):
    result = {}
    with zipfile.ZipFile(path) as archive:
        need(len(archive.infolist()) <= 2048, "too many zip entries")
        need(sum(e.file_size for e in archive.infolist()) <= 64 * MIB, "zip expands beyond 64 MiB")
        for entry in archive.infolist():
            name = entry.filename
            need(not name.startswith("/") and "\\" not in name and
                 all(p not in ("", ".", "..") for p in name.rstrip("/").split("/")), "unsafe zip path")
            need(not stat.S_ISLNK(entry.external_attr >> 16), "zip symlink refused")
            if entry.is_dir():
                continue
            need(name not in result and entry.file_size <= 64 * MIB, "duplicate/oversized zip entry")
            result[name] = archive.read(entry)
    # Full config_all sometimes has one enclosing directory.
    if "config_custom.yaml" not in result and "manifest.json" not in result:
        prefixes = {n.split("/", 1)[0] for n in result}
        if len(prefixes) == 1 and all("/" in n for n in result):
            result = {n.split("/", 1)[1]: data for n, data in result.items()}
    return result


def package(path, channel, schema, package_id):
    entries = zip_entries(path)
    m = json.loads(entries.pop("manifest.json"))
    need(m["format"] == 1 and m["channel"] == channel and
         m["config_schema"] == schema and m["package_id"] == package_id, "wrong package identity")
    names = m["managed_files"]
    need(names and len(set(names)) == len(names) and all(managed(n) for n in names), "invalid managed files")
    need(set(entries) == set(names) == set(m["sha256"]), "package files/hash keys differ from manifest")
    for name, data in entries.items():
        need(hashlib.sha256(data).hexdigest() == m["sha256"][name], "package hash mismatch: " + name)
    for name, value in m.get("create_if_missing", {}).items():
        need(re.fullmatch(r"rule/switch[0-9]+\.txt", name) and value in ("A", "B"), "invalid switch default")
    need(all(managed(n) and n.startswith("sub_config/") and n not in names
             for n in m.get("delete_files", [])), "invalid delete_files")
    return m, entries


def system_props(service):
    raw = run(["systemctl", "show", service, "-p", "FragmentPath", "-p", "DropInPaths",
               "-p", "ExecStart", "-p", "ActiveState", "-p", "MainPID", "-p", "NRestarts"])
    return dict(line.split("=", 1) for line in raw.decode().splitlines() if "=" in line)


def http_health(url):
    # Do not send local operations through HTTP(S)_PROXY.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(url, timeout=2) as response:
        need(response.status == 200, "health status")
        return json.loads(response.read(1024 * 1024))


def health(s, version, native, timeout=None):
    end = time.monotonic() + (timeout if timeout is not None else s["timeout"])
    consecutive = 0
    while time.monotonic() < end:
        try:
            h = http_health(s["health_url"])
            need(h.get("ready") is True and h.get("version") == version, "not ready/version mismatch")
            if native:
                need(h.get("runtime") == "rust" and h.get("applied_config_schema") == 1, "native schema/runtime mismatch")
            need(system_props(s["service"])["ActiveState"] == "active", "service not active")
            dns = run(["dig", "@" + s["dns_server"], "-p", s["dns_port"], s["dns_name"],
                       "A", "+time=1", "+tries=1", "+noall", "+comments", "+answer"], timeout=3).decode()
            need("status: NOERROR" in dns and re.search(r"ANSWER: [1-9][0-9]*", dns), "no DNS answer")
            if s.get("expect_answer"):
                need(any(line.split()[-1:] == [s["expect_answer"]] for line in dns.splitlines()
                         if not line.startswith(";")), "unexpected DNS answer")
            consecutive += 1
            if consecutive == 3:
                return h
        except (ValueError, OSError, subprocess.SubprocessError):
            consecutive = 0
        time.sleep(0.2)
    raise RuntimeError("health/DNS timed out for " + version)


def stage(s, name, **values):
    s.update(values, phase=name, updated_at=datetime.datetime.now(datetime.timezone.utc).isoformat())
    save(Path(s["tx"]) / "state.json", s)
    with (Path(s["tx"]) / "events.jsonl").open("a") as stream:
        stream.write(json.dumps({"phase": name, "at": s["updated_at"]}) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def copy_tree(source, dest):
    safe_tree(source)
    shutil.copytree(source, dest, copy_function=shutil.copy2)
    # copy2 preserves mode/time, not ownership; keep data owners too.
    for p in [Path(source), *Path(source).rglob("*")]:
        q = Path(dest) / p.relative_to(source)
        st = p.stat()
        os.chown(q, st.st_uid, st.st_gid)


def fsync_tree(root):
    for p in Path(root).rglob("*"):
        if p.is_file():
            with p.open("rb") as stream:
                os.fsync(stream.fileno())
    for directory, _, _ in os.walk(root, topdown=False):
        sync_dir(directory)


def hashes(root):
    return {str(p.relative_to(root)): digest(p) for p in Path(root).rglob("*") if p.is_file()}


def verify_hashes(root, expected):
    need(hashes(root) == expected, "backup contents/hash mismatch: " + str(root))


def space(requirements):
    grouped = {}
    for path, amount, inodes in requirements:
        dev = Path(path).stat().st_dev
        old = grouped.setdefault(dev, [Path(path), 0, 0])
        old[1] += amount
        old[2] += inodes
    facts = []
    for path, amount, inodes in grouped.values():
        st = os.statvfs(path)
        required = math.ceil(amount * 1.2) + 32 * MIB
        available = st.f_bavail * st.f_frsize
        facts.append({"filesystem_at": str(path), "required_bytes": required,
                      "available_bytes": available, "required_inodes": inodes})
        need(available >= required and (st.f_favail == 0 and st.f_files == 0 or st.f_favail >= inodes),
             f"insufficient space/inodes at {path}: need {required} bytes/{inodes} inodes, available {available}/{st.f_favail}; no files cleaned")
    return facts


def changed_managed(root, reference, names):
    return sorted(n for n in names if not (root / n).is_file() or
                  (root / n).read_bytes() != reference[n])


def metadata_paths(s):
    props = system_props(s["service"])
    # systemd quotes spaces in these fields; reject instead of misparsing them.
    paths = [props["FragmentPath"], *props["DropInPaths"].split()]
    need(paths[0] and all("\\" not in p for p in paths), "unsupported unit path escaping")
    if Path(s["environment_file"]).exists():
        paths.append(s["environment_file"])
    return paths


def preflight(a):
    need(os.geteuid() == 0 and platform.system() == "Linux" and platform.machine() == "x86_64",
         "requires Linux amd64 root")
    for tool in ("systemctl", "systemd-run", "dig"):
        need(shutil.which(tool), "missing dependency: " + tool)
    need(re.fullmatch(r"[A-Za-z0-9_.@-]+\.service", a.service) is not None, "invalid service name")
    root, binary = safe_path(a.root), safe_path(a.binary)
    need(root.is_dir() and binary.is_file() and not root.is_mount(), "root/binary missing or config root is a mount point")
    need(binary != root and root not in binary.parents, "binary must be outside config root")
    safe_tree(root)
    inputs = {k: str(safe_path(getattr(a, k))) for k in
              ("rust_binary", "migrate", "config_package", "go_reference", "go_package")}
    for path in inputs.values():
        need(Path(path).is_file(), "missing input: " + path)
    for k in ("rust_binary", "migrate"):
        header = Path(inputs[k]).read_bytes()[:20]
        need(header[:5] == b"\x7fELF\x02" and header[18:20] == b"\x3e\x00" and os.access(inputs[k], os.X_OK),
             "input must be executable Linux amd64 ELF: " + k)
    if a.sha256_manifest:
        expected = load(safe_path(a.sha256_manifest))
        need(set(expected) == set(inputs), "sha256 manifest must cover all five input keys")
        for key, path in inputs.items():
            need(digest(path) == expected[key], "input sha256 mismatch: " + key)
    m, files = package(inputs["config_package"], "rust", 1, RUST_ID)
    gm, _ = package(inputs["go_package"], "main", 4, GO_ID)
    reference = zip_entries(inputs["go_reference"])
    need(all(n in reference and hashlib.sha256(reference[n]).hexdigest() == gm["sha256"][n]
             for n in gm["managed_files"]), "Go reference does not match Go package manifest")
    state = load(root / "webinfo/config_update_state.json")
    need(state.get("applied_schema") == 4 and state.get("package_id") == GO_ID,
         "expected installed Go schema 4 / main-config-schema-4")
    for name in ("settings.json", "rust_config_update_state.json"):
        need(not (root / "webinfo" / name).exists(), "already has native state: " + name)
    diffs = changed_managed(root, reference, gm["managed_files"])
    need(set(a.accept_diff) <= set(gm["managed_files"]), "accept-diff must name a Go managed file")
    print("managed differences (take Rust package if explicitly accepted): " + json.dumps(diffs), flush=True)
    need(set(diffs) <= set(a.accept_diff), "managed files differ; explicitly choose --accept-diff PATH for each, or stop")
    unmanaged = sorted(str(p.relative_to(root)) for p in root.rglob("*")
                       if p.is_file() and str(p.relative_to(root)) not in gm["managed_files"])
    print("unmanaged files preserved: " + json.dumps(unmanaged), flush=True)
    props = system_props(a.service)
    need(props["ActiveState"] == "active", "Go service must be running")
    need(str(binary) in props["ExecStart"] and str(root) in props["ExecStart"],
         "service ExecStart must use the supplied binary and root")
    from urllib.parse import urlsplit
    url = urlsplit(a.health_url)
    need(url.scheme == "http" and url.hostname in ("127.0.0.1", "localhost", "::1") and
         url.path == "/api/v1/system/health" and not url.username, "health URL must be local HTTP health")
    need(a.dns_server in ("127.0.0.1", "::1") and 0 < a.dns_port < 65536, "DNS probe must be local")
    version = run([inputs["rust_binary"], "version"], timeout=10).decode().strip()
    need(re.fullmatch(r"rust-v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?", version), "invalid native version")
    s = dict(vars(a), root=str(root), binary=str(binary), version=version, inputs=inputs,
             old_version=http_health(a.health_url).get("version"), initial_service=props,
             accepted_differences=diffs, unmanaged_files=unmanaged)
    need(isinstance(s["old_version"], str) and not s["old_version"].startswith("rust-"), "expected Go health version")
    health(s, s["old_version"], False)
    meta = metadata_paths(s)
    for name in meta:
        need(safe_path(name).is_file(), "missing unit/env: " + name)
    s["metadata_hashes"] = {p: digest(p) for p in meta}
    s["old_binary_hash"] = digest(binary)
    s["input_hashes"] = {k: digest(p) for k, p in inputs.items()}
    r = tree_size(root)
    b = binary.stat().st_size + sum(Path(inputs[k]).stat().st_size for k in ("rust_binary", "migrate"))
    z = sum(Path(p).stat().st_size for p in inputs.values()) + Path(__file__).stat().st_size
    u = sum(map(len, files.values()))
    count = sum(1 for _ in root.rglob("*"))
    s["space"] = space([(root.parent, 4*r + 3*b + z + 2*u, 4*count + 200),
                         (binary.parent, 2*max(binary.stat().st_size, Path(inputs["rust_binary"]).stat().st_size), 4)])
    return s


def prepare_root(s, source, dest, label):
    tx = Path(s["tx"])
    copy_tree(source, dest)
    m, files = package(s["inputs"]["config_package"], "rust", 1, RUST_ID)
    gm, _ = package(s["inputs"]["go_package"], "main", 4, GO_ID)
    for name in set(gm["managed_files"]) | set(m.get("delete_files", [])):
        if name not in files and (dest / name).exists():
            (dest / name).unlink()  # The script-created staging copy only.
    for name, data in files.items():
        (dest / name).parent.mkdir(parents=True, exist_ok=True)
        atomic(dest / name, data, 0o644)
    for name, value in m.get("create_if_missing", {}).items():
        if not (dest / name).exists():
            (dest / name).parent.mkdir(parents=True, exist_ok=True)
            atomic(dest / name, value.encode(), 0o644)
    output = tx / ("migration-" + label)
    result = subprocess.run([s["inputs"]["migrate"], "--from", str(source), "-d", str(dest),
                             "-c", "config_custom.yaml", "--out", str(output)],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
    atomic(tx / ("migration-" + label + ".log"), result.stdout)
    need(result.returncode == 0, f"migration exit {result.returncode}; STOP: see {output}/migration-report.txt (private)")
    (dest / "webinfo").mkdir(exist_ok=True)
    shutil.copy2(output / "settings.json", dest / "webinfo/settings.json")
    save(dest / "webinfo/rust_config_update_state.json", {
        "format": 1, "applied_schema": 1, "required_schema": 1, "status": "success",
        "package_id": RUST_ID, "package_sha256": digest(s["inputs"]["config_package"]),
        "binary_version": s["version"], "files_updated": len(files),
        "backup_dir": str(tx / "backup"), "message": "first Go to Rust cutover",
        "updated_at": datetime.datetime.now(datetime.timezone.utc).isoformat()})
    checked = run([s["inputs"]["rust_binary"], "check", "-d", dest, "-c", "config_custom.yaml"])
    atomic(tx / ("check-" + label + ".log"), checked)
    need(re.search(rb"SUMMARY gaps=0 cascades=0 globals=0\b", checked), "native check has gaps; see private check log")
    fsync_tree(dest)


def install_binary(source, target):
    target = safe_path(target)
    st = target.stat()
    atomic(target, Path(source).read_bytes(), stat.S_IMODE(st.st_mode), (st.st_uid, st.st_gid))


def backup_metadata(s):
    tx = Path(s["tx"])
    records = []
    for index, name in enumerate(s["metadata_hashes"]):
        p = Path(name)
        st = p.stat()
        dest = tx / "backup/metadata" / str(index)
        shutil.copy2(p, dest)
        records.append({"path": name, "copy": str(dest), "sha256": digest(dest),
                        "mode": stat.S_IMODE(st.st_mode), "uid": st.st_uid, "gid": st.st_gid})
    s["metadata"] = records


def verify_backup(s):
    tx = Path(s["tx"])
    need(digest(tx / "backup/binary") == s["old_binary_hash"], "Go program backup hash mismatch")
    verify_hashes(tx / "backup/config", s["config_hashes"])
    if "final_hashes" in s:
        verify_hashes(tx / "backup/final-config", s["final_hashes"])
    for record in s["metadata"]:
        need(digest(record["copy"]) == record["sha256"], "unit/env backup mismatch")


def restore(s, reason, automatic=False):
    tx, root = Path(s["tx"]), Path(s["root"])
    outage_start = s.get("stop_time", time.time()) if automatic else time.time()
    verify_backup(s)
    # Refuse unexpected new drop-ins, never delete an administrator's files.
    need(set(metadata_paths(s)) == set(s["metadata_hashes"]), "unit/drop-in set changed; manual recovery required")
    safe_tree(root) if root.exists() else None
    if root.exists():
        space([(root.parent, tree_size(root) + tree_size(tx / "backup/config"), 200)])
    stage(s, "rolling_back", rollback_reason=reason)
    run(["systemctl", "stop", s["service"]])
    if (tx / "go-root").exists():
        verify_hashes(tx / "go-root", s.get("final_hashes", s["config_hashes"]))
        # Intent paths survive a kill between either rename and its state write.
        if root.exists():
            preserved = tx / ("rust-root-" + uuid.uuid4().hex)
            stage(s, "rolling_back", preserved_rust_root=str(preserved))
            os.rename(root, preserved)
            sync_dir(root.parent)
            sync_dir(tx)
        os.rename(tx / "go-root", root)
        sync_dir(root.parent)
        sync_dir(tx)
    elif s.get("preserved_rust_root") and not root.exists():
        # A repeated rollback after the second rename needs no extra move.
        source = tx / "backup" / ("final-config" if "final_hashes" in s else "config")
        copy_tree(source, root)
        fsync_tree(root)
    # If go-root was never created, the live root is still the original Go root.
    need(root.exists(), "Go root missing; refusing a mixed restart")
    saved_program = tx / ("program-before-rollback-" + uuid.uuid4().hex)
    shutil.copy2(s["binary"], saved_program)
    install_binary(tx / "backup/binary", s["binary"])
    for index, record in enumerate(s["metadata"]):
        p = safe_path(record["path"])
        if p.exists():
            shutil.copy2(p, tx / ("metadata-before-rollback-" + uuid.uuid4().hex + "-" + str(index)))
        atomic(p, Path(record["copy"]).read_bytes(), record["mode"], (record["uid"], record["gid"]))
    run(["systemctl", "daemon-reload"])
    # An inactive unit may have been garbage-collected after daemon-reload;
    # reset-failed then returns 1. The following explicit start is authoritative.
    subprocess.run(["systemctl", "reset-failed", s["service"]],
                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
    run(["systemctl", "start", s["service"]])
    health(s, s["old_version"], False)
    stage(s, "rolled_back", service_after=system_props(s["service"]),
          outage_seconds=time.time() - outage_start)


def worker(s, action):
    tx, root = Path(s["tx"]), Path(s["root"])
    with (tx / "worker.lock").open("a") as lock, (root.parent / ".mosdns-cutover.lock").open("a") as installation_lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fcntl.flock(installation_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        s = load(tx / "state.json")
        if action == "rollback":
            if s.get("launch_from_phase") == "rolled_back":
                health(s, s["old_version"], False)
                stage(s, "rolled_back")
                return
            need(s.get("backup_complete"), "no complete backup; no live changes were authorized")
            try:
                restore(s, "operator rollback; post-cutover Rust data retained separately")
            except Exception as error:
                stage(s, "recovery_required", error=str(error))
                raise
            return
        try:
            # Do all lengthy preflight copies while the Go service is still live.
            prepare_root(s, root, tx / "prepared", "initial")
            stage(s, "prepared")
            (tx / "backup/metadata").mkdir(parents=True)
            copy_tree(root, tx / "backup/config")
            shutil.copy2(s["binary"], tx / "backup/binary")
            backup_metadata(s)
            s["config_hashes"] = hashes(tx / "backup/config")
            fsync_tree(tx / "backup")
            save(tx / "backup/sha256.json", {"config": s["config_hashes"],
                 "binary": s["old_binary_hash"], "metadata": s["metadata"]})
            stage(s, "backed_up", backup_complete=True)
            verify_backup(s)
            need(digest(s["binary"]) == s["old_binary_hash"] and
                 all(digest(p) == h for p, h in s["metadata_hashes"].items()), "installation changed during preparation")
            gm, _ = package(s["inputs"]["go_package"], "main", 4, GO_ID)
            reference = zip_entries(s["inputs"]["go_reference"])
            need(set(changed_managed(root, reference, gm["managed_files"])) <= set(s["accept_diff"]),
                 "managed configuration changed during preparation")
            if s["dry_run"]:
                stage(s, "verified", dry_run=True, service_after=system_props(s["service"]))
                return
            count = sum(1 for _ in root.rglob("*"))
            space([(root.parent, 2 * tree_size(root), 2*count + 100)])
            stage(s, "stopping", stop_time=time.time())
            run(["systemctl", "stop", s["service"]])
            stage(s, "stopped")
            copy_tree(root, tx / "backup/final-config")
            s["final_hashes"] = hashes(tx / "backup/final-config")
            fsync_tree(tx / "backup/final-config")
            save(tx / "backup/final-sha256.json", s["final_hashes"])
            stage(s, "stopped")
            prepare_root(s, tx / "backup/final-config", tx / "final-prepared", "final")
            stage(s, "binary_installing")
            install_binary(s["inputs"]["rust_binary"], s["binary"])
            stage(s, "binary_installed")
            stage(s, "root_moving")
            os.rename(root, tx / "go-root")
            sync_dir(root.parent)
            sync_dir(tx)
            stage(s, "root_moved")
            stage(s, "root_installing")
            os.rename(tx / "final-prepared", root)
            sync_dir(root.parent)
            sync_dir(tx)
            stage(s, "testing")
            run(["systemctl", "start", s["service"]])
            health(s, s["version"], True)
            stage(s, "verified", outage_seconds=time.time()-s["stop_time"],
                  service_after=system_props(s["service"]))
        except Exception as error:
            if "stop_time" in s:
                try:
                    restore(s, str(error), automatic=True)
                except Exception as rollback_error:
                    stage(s, "recovery_required", error=str(error), rollback_error=str(rollback_error))
                    raise
            else:
                stage(s, "failed", error=str(error))
            raise


def launch(s, action):
    tx = Path(s["tx"])
    unit = "mosdns-cutover-" + uuid.uuid4().hex
    command = ["systemd-run", "--quiet", "--unit=" + unit, "--property=Type=exec",
               "--property=UMask=0077", "--property=StandardOutput=append:" + str(tx / "worker.log"),
               "--property=StandardError=inherit", "--property=TimeoutStartSec=infinity",
               sys.executable, str(tx / "engine.py"), "worker", "--transaction", str(tx), "--action", action]
    # Public caller may be inside the isolated VM test namespace.
    netns = os.environ.get("MOSDNS_CUTOVER_NETNS")
    if netns:
        need(re.fullmatch(r"[A-Za-z0-9_-]+", netns), "invalid test netns")
        command.insert(5, "--property=NetworkNamespacePath=/run/netns/" + netns)
    stage(s, "launching", launch_from_phase=s["phase"], worker_unit=unit,
          worker_units=s.get("worker_units", []) + [unit], pending_action=action)
    run(command)
    print("transaction: " + str(tx), flush=True)
    print("worker: " + unit + "; disconnect/Ctrl-C only detaches this monitor", flush=True)
    try:
        while True:
            current = load(tx / "state.json")
            if current["phase"] in TERMINAL:
                print(json.dumps({k: current.get(k) for k in
                      ("phase", "dry_run", "outage_seconds", "rollback_reason", "error", "rollback_error")}), flush=True)
                return 0 if current["phase"] == "verified" or action == "rollback" and current["phase"] == "rolled_back" else 1
            unit_state = system_props(unit + ".service")["ActiveState"]
            if unit_state in ("failed", "inactive"):
                raise RuntimeError("worker stopped before terminal result; use rust-to-go.sh --transaction " + str(tx))
            time.sleep(0.3)
    except KeyboardInterrupt:
        print("monitor detached; worker still runs; state: " + str(tx / "state.json"))
        return 0


def main():
    os.umask(0o077)
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="command", required=True)
    c = sub.add_parser("cutover")
    for flag in ("rust-binary", "migrate", "config-package", "go-reference", "go-package"):
        c.add_argument("--" + flag, required=True)
    c.add_argument("--binary", default="/usr/local/bin/mosdns")
    c.add_argument("--root", default="/cus/mosdns")
    c.add_argument("--service", default="mosdns.service")
    c.add_argument("--environment-file", default="/etc/sysconfig/mosdns")
    c.add_argument("--health-url", default="http://127.0.0.1:9099/api/v1/system/health")
    c.add_argument("--timeout", type=int, default=60)
    c.add_argument("--dns-server", default="127.0.0.1")
    c.add_argument("--dns-port", type=int, default=53)
    c.add_argument("--dns-name", default="example.com")
    c.add_argument("--expect-answer")
    c.add_argument("--sha256-manifest")
    c.add_argument("--accept-diff", action="append", default=[])
    c.add_argument("--dry-run", action="store_true")
    r = sub.add_parser("rollback")
    r.add_argument("--transaction", required=True)
    r.add_argument("--acknowledge-data-loss", action="store_true",
                   help="restore old Go data; save, but do not merge, subsequent Rust changes")
    w = sub.add_parser("worker")
    w.add_argument("--transaction", required=True)
    w.add_argument("--action", choices=("cutover", "rollback"), required=True)
    a = p.parse_args()
    if a.command == "cutover":
        need(1 <= a.timeout <= 600, "timeout must be 1..600 seconds")
        s = preflight(a)
        tx = Path(a.root).parent / (".mosdns-cutover-" + datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:12])
        tx.mkdir(mode=0o700)
        s["tx"] = str(tx)
        shutil.copy2(__file__, tx / "engine.py")
        (tx / "inputs").mkdir()
        for key, source in s["inputs"].items():
            target = tx / "inputs" / key
            shutil.copy2(source, target)
            need(digest(target) == s["input_hashes"][key], "input changed while copied: " + key)
            s["inputs"][key] = str(target)
        fsync_tree(tx)
        stage(s, "preparing")
        return launch(s, "cutover")
    tx = safe_path(a.transaction)
    st = tx.stat()
    need(os.geteuid() == 0 and st.st_uid == 0 and stat.S_IMODE(st.st_mode) == 0o700,
         "transaction must be a root-owned 0700 directory")
    s = load(tx / "state.json")
    need(s["tx"] == str(tx), "transaction path changed")
    if a.command == "worker":
        worker(s, a.action)
        return 0
    need(a.acknowledge_data_loss, "rollback discards subsequent Rust changes from the active installation; they are saved separately. Pass --acknowledge-data-loss")
    need(s.get("backup_complete") and not s.get("dry_run"), "not a cutover with a complete backup")
    # Do not overwrite a live worker's state (including after SSH disconnection).
    with (tx / "worker.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    return launch(s, "rollback")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print("STOP: " + str(error), file=sys.stderr)
        sys.exit(1)
