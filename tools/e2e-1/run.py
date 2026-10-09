"""VM-only experiment. Owns namespaces/processes, including failed starts."""
import argparse
import json
import os
import pathlib
import signal
import subprocess
import time

from prepare import prepare


def command(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def run(args):
    if os.geteuid() != 0:
        raise RuntimeError("run as root on the test VM")
    def interrupted(signum, frame):
        raise InterruptedError("experiment interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    args.work.mkdir(mode=0o700)
    service = subprocess.check_output(["systemctl", "show", "mosdns", "-p", "MainPID", "-p", "ActiveState"])
    (args.work / "service-before.txt").write_bytes(service)
    prepare(args.work, args.inputs, args.migrate, args.names)
    if args.corpus:
        pinned = args.corpus.read_bytes()
        if json.loads(pinned) != json.loads((args.work / "corpus.json").read_bytes()):
            raise ValueError("prepared corpus differs from pinned round-1 corpus")
        (args.work / "corpus.json").write_bytes(pinned)
    script = pathlib.Path(__file__).resolve().parent
    command([args.native, "check", "-d", args.work / "native", "-c", "config_custom.yaml"], stdout=(args.work / "check.txt").open("w"), stderr=subprocess.STDOUT)
    owned, namespaces, logs = [], [], []
    try:
        for role, binary in (("go", args.go), ("native", args.native)):
            root = args.work / role
            (root / "tmp").mkdir()
            # A CA anchor plus a real server leaf (CA-as-leaf is rejected by rustls).
            quiet = {"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}
            command(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", root / "ca-key.pem", "-out", root / "ca.pem", "-days", "2", "-subj", "/CN=E2E1-local-CA", "-addext", "basicConstraints=critical,CA:TRUE"], **quiet)
            command(["openssl", "req", "-newkey", "rsa:2048", "-nodes", "-keyout", root / "key.pem", "-out", root / "leaf.csr", "-subj", "/CN=localhost"], **quiet)
            (root / "leaf.ext").write_text("subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n")
            command(["openssl", "x509", "-req", "-in", root / "leaf.csr", "-CA", root / "ca.pem", "-CAkey", root / "ca-key.pem", "-CAcreateserial", "-out", root / "cert.pem", "-days", "2", "-extfile", root / "leaf.ext"], **quiet)
            ns = f"e2e1-{os.getpid()}-{role}"
            command(["ip", "netns", "add", ns])
            namespaces.append(ns)
            command(["ip", "-n", ns, "link", "set", "lo", "up"])
            (root / "isolation.txt").write_bytes(subprocess.check_output(["ip", "-n", ns, "route", "show"]))
            def spawn(argv, filename, env=None):
                stream = (root / filename).open("w")
                logs.append(stream)
                process = subprocess.Popen(["ip", "netns", "exec", ns, *map(str, argv)], stdout=stream, stderr=subprocess.STDOUT, env=env, start_new_session=True)
                owned.append(process)
                return process
            mock = spawn(["python3", script / "mock.py", root], "mock.log")
            deadline = time.monotonic() + 10
            while not (root / "mock-ready").exists():
                if mock.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("mock failed readiness")
                time.sleep(.1)
            env = dict(os.environ, SSL_CERT_FILE=str(root / "ca.pem"), SSL_CERT_DIR=str(root / "empty-certs"), GOMAXPROCS="1", PYTHONDONTWRITEBYTECODE="1")
            for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
                env.pop(key, None)
            host = spawn(["unshare", "--mount", "--propagation", "private", "sh", "-c", 'mount --bind "$1" /tmp; shift; exec "$@"', "e2e1", root / "tmp", binary, "start", "-d", root, "-c", "config_custom.yaml"], "host.log", env)
            # unshare/sh exec preserves the PID used for resource attribution.
            api_port = json.loads((root / "webinfo/webui_port_settings.json").read_text())["port"] if role == "go" else 9099
            command(["ip", "netns", "exec", ns, "python3", script / "replay.py", root, args.work / "corpus.json", host.pid, "--api-port", api_port])
            if host.poll() is not None:
                raise RuntimeError(role + " host exited during replay")
            # Stop before the next role, identical serial load and no simultaneous build.
            for process in (host, mock):
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=30)
        command(["python3", script / "compare.py", args.work])
    finally:
        for process in reversed(owned):
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
        for ns in reversed(namespaces):
            remaining = subprocess.check_output(["ip", "netns", "pids", ns]).strip()
            if remaining:
                raise RuntimeError("owned namespace still has processes")
            command(["ip", "netns", "delete", ns])
        for stream in logs:
            stream.close()
        after = subprocess.check_output(["systemctl", "show", "mosdns", "-p", "MainPID", "-p", "ActiveState"])
        (args.work / "cleanup.json").write_text(json.dumps({"processes_exited": all(p.poll() is not None for p in owned), "namespaces_deleted": namespaces, "original_service_unchanged": service == after}, indent=2))
        if after != service:
            raise RuntimeError("original service identity changed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    for flag in ("work", "inputs", "go", "native", "migrate"):
        parser.add_argument("--" + flag, required=True, type=lambda s: pathlib.Path(s).resolve())
    parser.add_argument("--names", type=int, default=2000)
    parser.add_argument("--corpus", type=lambda s: pathlib.Path(s).resolve())
    run(parser.parse_args())
