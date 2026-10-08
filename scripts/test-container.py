#!/usr/bin/env python3
"""Smoke-test a local image without registry credentials or production data."""

import argparse
import contextlib
import json
import os
import re
import socket
import struct
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import zipfile
from pathlib import Path


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(base, path, payload=None, expected=200):
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request(base + path, data=data, headers={"Content-Type": "application/json"})
    try:
        response = urllib.request.urlopen(req, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read()
        assert response.code == expected, (path, response.code, body)
        return body


def wait_ready(base):
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        try:
            if json.loads(request(base, "/api/v1/system/health"))["ready"]:
                return
        except (OSError, AssertionError, ValueError):
            time.sleep(0.5)
    raise AssertionError("Container did not become ready: " + base)


def check_dns(port, tcp=False):
    labels = b"\x05smoke\x04test\x00"
    query = struct.pack("!6H", 1234, 0x100, 1, 0, 0, 0) + labels + struct.pack("!2H", 1, 1)
    kind = socket.SOCK_STREAM if tcp else socket.SOCK_DGRAM
    with socket.socket(socket.AF_INET, kind) as sock:
        sock.settimeout(5)
        sock.connect(("127.0.0.1", port))
        if tcp:
            sock.sendall(struct.pack("!H", len(query)) + query)
            header = sock.makefile("rb")
            size = struct.unpack("!H", header.read(2))[0]
            response = header.read(size)
        else:
            sock.send(query)
            response = sock.recv(4096)
    ident, flags, _, answers, _, _ = struct.unpack("!6H", response[:12])
    assert ident == 1234 and flags & 0x8000 and flags & 15 == 0 and answers == 1
    assert response.endswith(socket.inet_aton("192.0.2.10")), response


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image")
    parser.add_argument("--engine", choices=("docker", "container"), default="docker")
    parser.add_argument("--platform", default="linux/amd64")
    args = parser.parse_args()
    created = []

    def engine(*command, check=True):
        return subprocess.run([args.engine, *command], check=check, capture_output=True, text=True)

    def restore_permissions(name):
        if args.engine == "docker":
            # Only these test containers mount our disposable data directory.
            engine("exec", name, "chown", "-R", f"{os.getuid()}:{os.getgid()}", "/cus/mosdns", check=False)

    def remove(name):
        restore_permissions(name)
        engine("rm", "-f", name)
        created.remove(name)

    def cleanup_containers():
        for name in created[:]:
            logs = engine("logs", name, check=False)
            print(logs.stdout + logs.stderr)
            restore_permissions(name)
            engine("rm", "-f", name, check=False)
            created.remove(name)

    try:
        with tempfile.TemporaryDirectory(prefix="mosdns-container-test-") as scratch, contextlib.ExitStack() as cleanup:
            # Stop containers and restore ownership before TemporaryDirectory removes files.
            cleanup.callback(cleanup_containers)
            data = Path(scratch)
            webinfo = data / "webinfo"
            webinfo.mkdir()
            source = (Path(__file__).resolve().parent.parent / "coremain/config_update.go").read_text()
            schema = int(re.search(r'requiredConfigSchema\s*=\s*"(\d+)"', source)[1])
            (webinfo / "config_update_state.json").write_text(json.dumps({"format": 1, "applied_schema": schema, "status": "applied"}))
            (webinfo / "special_upstream_groups.json").write_text(json.dumps([{
                "slot": 50, "name": "smoke", "listen_port": 6053, "custom_port_only": True,
            }]))
            config = """log:
  level: error
api:
  http: ':9099'
plugins:
  - tag: smoke_hosts
    type: hosts
    args:
      entries:
        - 'smoke.test 192.0.2.10'
  - tag: smoke_sequence
    type: sequence
    args:
      - exec: $smoke_hosts
  - tag: smoke_udp
    type: udp_server
    args:
      entry: smoke_sequence
      listen: ':53'
  - tag: smoke_tcp
    type: tcp_server
    args:
      entry: smoke_sequence
      listen: ':53'
"""
            (data / "config_custom.yaml").write_text(config)
            (data / "keep.txt").write_text("existing operator data")

            # Serve a deterministic config package locally instead of downloading operator config.
            with zipfile.ZipFile(data / "config_all.zip", "w") as archive:
                for path in data.rglob("*"):
                    if path.is_file() and path.name != "config_all.zip":
                        archive.write(path, path.relative_to(data))
            server = "mosdns-init-source-" + data.name.rsplit("-", 1)[-1]
            server_command = ["run", "-d", "--name", server, "--platform", args.platform,
                              "--entrypoint", "python", "-v", str(data) + ":/fixtures:ro"]
            if args.engine == "container" and args.platform == "linux/amd64":
                server_command += ["--rosetta"]
            created.append(server)
            engine(*server_command, "python:3.13-alpine", "-m", "http.server", "8000", "--directory", "/fixtures")
            # Docker returns from a detached run before Python has opened its listening socket.
            engine("exec", server, "python", "-c", """
import time, urllib.request
for attempt in range(60):
    try:
        with urllib.request.urlopen('http://127.0.0.1:8000/config_all.zip', timeout=1):
            pass
        break
    except OSError:
        time.sleep(0.5)
else:
    raise SystemExit('Fixture config server did not become ready')
""")
            server_ip = engine("exec", server, "hostname", "-i").stdout.strip().split()[0]
            empty_data = data / "auto-init"
            empty_data.mkdir()
            http_port, dns_port = free_port(), free_port()
            init_name = "mosdns-init-test-" + data.name.rsplit("-", 1)[-1]
            init_command = ["run", "-d", "--name", init_name, "--platform", args.platform,
                            "-v", str(empty_data) + ":/cus/mosdns", "-e",
                            f"MOSDNS_CONFIG_INIT_URL=http://{server_ip}:8000/config_all.zip",
                            "-p", f"127.0.0.1:{http_port}:9099", "-p", f"127.0.0.1:{dns_port}:53/udp"]
            if args.engine == "container" and args.platform == "linux/amd64":
                init_command += ["--rosetta"]
            created.append(init_name)
            engine(*init_command, args.image)
            wait_ready(f"http://127.0.0.1:{http_port}")
            check_dns(dns_port)
            assert (empty_data / "config_custom.yaml").read_text() == config
            assert (empty_data / "keep.txt").read_text() == "existing operator data"
            remove(init_name)
            remove(server)
            print(f"{args.platform}: empty-volume config auto-init passed", flush=True)

            for mode in ("bridge", "host"):
                http_port, dns_port = free_port(), free_port()
                name = "mosdns-smoke-" + mode + "-" + data.name.rsplit("-", 1)[-1]
                command = ["run", "-d", "--name", name, "--platform", args.platform,
                           "-v", str(data) + ":/cus/mosdns", "-e", "MOSDNS_CONTAINER_NETWORK_MODE=" + mode,
                           "-p", f"127.0.0.1:{http_port}:9099", "-p", f"127.0.0.1:{dns_port}:53/udp",
                           "-p", f"127.0.0.1:{dns_port}:53/tcp"]
                # Apple uses a VM network; this exercises the same host-mode API capability flag.
                if args.engine == "docker" and mode == "host":
                    command = ["run", "-d", "--name", name, "--platform", args.platform,
                               "--network", "host", "-v", str(data) + ":/cus/mosdns",
                               "-e", "MOSDNS_CONTAINER_NETWORK_MODE=host"]
                    http_port, dns_port = free_port(), free_port()
                    host_config = config.replace(":9099", f":{http_port}").replace(":53", f":{dns_port}")
                    (data / "config_custom.yaml").write_text(host_config)
                if args.engine == "container" and args.platform == "linux/amd64":
                    command += ["--rosetta"]
                command.append(args.image)
                created.append(name)
                engine(*command)
                base = f"http://127.0.0.1:{http_port}"
                wait_ready(base)
                for path in ("/", "/log", "/assets/vue-log/app.js", "/assets/vue-log1/app.js"):
                    assert request(base, path), path
                port_status = json.loads(request(base, "/api/v1/system/webui-port"))
                assert port_status["change_supported"] == (mode == "host"), port_status
                group = json.loads(request(base, "/api/v1/special-groups/"))[0]
                assert group.get("port_mapping_required", False) == (mode == "bridge"), group
                if mode == "bridge":
                    assert "6053/tcp" in group["message"] and "6053/udp" in group["message"]
                    request(base, "/api/v1/system/webui-port", {"port": 9099}, expected=409)
                for path in ("/api/v1/update/apply", "/api/v1/config/export", "/api/v1/config/update_from_url"):
                    request(base, path, {}, expected=409)
                update = json.loads(request(base, "/api/v1/update/status"))
                assert update["apply_supported"] is False and not update.get("download_url"), update
                check_dns(dns_port)
                check_dns(dns_port, tcp=True)
                request(base, "/api/v1/special-groups/", {
                    "slot": 50, "name": "smoke-persisted", "listen_port": 6053, "custom_port_only": True,
                })
                request(base, "/api/v1/system/restart", {"delay_ms": 100})
                time.sleep(2)
                wait_ready(base)
                check_dns(dns_port)
                remove(name)
                created.append(name)
                engine(*command)
                wait_ready(base)
                assert json.loads(request(base, "/api/v1/special-groups/"))[0]["name"] == "smoke-persisted"
                assert (data / "keep.txt").read_text() == "existing operator data"
                assert (data / "config_custom.yaml").read_text() == (host_config if args.engine == "docker" and mode == "host" else config)
                remove(name)
                print(f"{args.platform}: {mode} DNS, WebUI, API restrictions, restart and volume reuse passed", flush=True)
    finally:
        cleanup_containers()


if __name__ == "__main__":
    main()
