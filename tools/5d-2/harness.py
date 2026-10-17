"""Root-only VM harness for 5D-2: one loopback-only netns, the fault-capable mock
and one host (Go or native) pinned like 5D-1 (SUT CPU0, mock/sender CPU1).

Never installs, builds or deploys. Inputs and outputs are private VM paths.
"""
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
E2E = HERE.parent / 'e2e-1'
FIVE_D_1 = HERE.parent / '5d-1'
sys.path.insert(0, str(E2E))
sys.path.insert(0, str(FIVE_D_1))
from load import resource  # noqa: E402


def command(argv, **kwargs):
    return subprocess.run(list(map(str, argv)), check=True, **kwargs)


def service_state():
    return subprocess.check_output(['systemctl', 'show', 'mosdns', '-p', 'MainPID', '-p', 'ActiveState'])


class Env:
    """Context manager: `with Env(...) as env:` leaves nothing behind."""

    def __init__(self, work, role, binary, output, prepare_root=None, seed=501, saved_port_applies=None):
        # The Go host and the native host since F-1 start the management listener on the port saved
        # in `webinfo/webui_port_settings.json` (the production snapshot has one). Default: every
        # role applies it; pass False only for a native build older than F-1.
        self.saved_port_applies = True if saved_port_applies is None else saved_port_applies
        if os.geteuid() != 0:
            raise ValueError('VM root required')
        self.work, self.role, self.binary, self.output = work, role, pathlib.Path(binary), pathlib.Path(output)
        self.prepare_root = prepare_root
        self.seed = seed
        self.owned, self.logs, self.created = [], [], False
        self.ns = f'5d2-{os.getpid()}'
        self.root = self.output / 'runtime'
        self.api_port = 9099
        self.host = self.mock = None

    # ----- lifecycle -------------------------------------------------------

    def __enter__(self):
        signal.signal(signal.SIGTERM, self._interrupted)
        signal.signal(signal.SIGINT, self._interrupted)
        self.output.mkdir(mode=0o700, parents=True)
        shutil.copytree(self.work / self.role, self.root)
        for path in (self.root / 'cache').glob('*.dump'):
            path.unlink()
        (self.root / 'tmp').mkdir()
        (self.root / 'empty-certs').mkdir(exist_ok=True)
        (self.root / 'faults.json').write_text('{}')
        self.before = service_state()
        (self.output / 'service-before.txt').write_bytes(self.before)
        self.env = dict(os.environ, SSL_CERT_FILE=str(self.root / 'ca.pem'), SSL_CERT_DIR=str(self.root / 'empty-certs'),
                        GOMAXPROCS='1', PYTHONDONTWRITEBYTECODE='1')
        for key in ('GOGC', 'GOMEMLIMIT', 'GODEBUG', 'HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy',
                    'https_proxy', 'all_proxy'):
            self.env.pop(key, None)
        try:
            if self.prepare_root:
                self.prepare_root(self.root)  # scenario-specific disposable edits
            command(['ip', 'netns', 'add', self.ns])
            self.created = True
            command(['ip', '-n', self.ns, 'link', 'set', 'lo', 'up'])
            self.mock = self.spawn(1, ['python3', E2E / 'mock.py', self.root], 'mock.log')
            end = time.monotonic() + 15
            while not (self.root / 'mock-ready').exists():
                if self.mock.poll() is not None or time.monotonic() > end:
                    raise RuntimeError('mock not ready')
                time.sleep(.1)
            self.host = self.spawn(0, ['unshare', '--mount', '--propagation', 'private', 'sh', '-c',
                                       'mount --bind "$1" /tmp; shift; exec "$@"', '5d2', self.root / 'tmp',
                                       self.binary, 'start', '-d', self.root, '-c', 'config_custom.yaml'], 'host.log')
            if self.saved_port_applies:
                self.api_port = json.loads((self.root / 'webinfo/webui_port_settings.json').read_text())['port']
            self.startup = []
            end = time.monotonic() + 90
            while True:
                self.startup.append(dict(time=time.monotonic(), **resource(self.host.pid)))
                listening = subprocess.check_output(['ip', 'netns', 'exec', self.ns, 'sh', '-c', 'cat /proc/net/udp /proc/net/udp6'])
                if b':0035 ' in listening:
                    break
                if self.host.poll() is not None or time.monotonic() > end:
                    raise RuntimeError('host not ready')
                time.sleep(.25)
            (self.output / 'startup.json').write_text(json.dumps(self.startup))
            self.api('POST', '/api/v1/audit/start', {})
            (self.output / 'run.json').write_text(json.dumps(dict(
                role=self.role, binary=str(self.binary), sut_pid=self.host.pid, mock_pid=self.mock.pid,
                seed=self.seed, ns=self.ns, api_port=self.api_port, audit='on', GOMAXPROCS='1')))
        except BaseException:
            self.__exit__(None, None, None)
            raise
        return self

    @staticmethod
    def _interrupted(signum, frame):
        raise InterruptedError('interrupted')

    def spawn(self, cpu, argv, name):
        stream = (self.output / name).open('w')
        self.logs.append(stream)
        process = subprocess.Popen(['ip', 'netns', 'exec', self.ns, 'taskset', '-c', str(cpu), *map(str, argv)],
                                   stdout=stream, stderr=subprocess.STDOUT, env=self.env, start_new_session=True)
        self.owned.append(process)
        return process

    def __exit__(self, *exc):
        for process in reversed(self.owned):
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
        remaining = b''
        deleted = False
        if self.created:
            # Report leftovers, but always try to delete the namespace and write the receipt first.
            remaining = subprocess.check_output(['ip', 'netns', 'pids', self.ns]).strip()
            if not remaining:
                command(['ip', 'netns', 'delete', self.ns])
                deleted = True
        for stream in self.logs:
            stream.close()
        after = service_state()
        value = dict(all_owned_processes_exited=all(p.poll() is not None for p in self.owned),
                     namespace_deleted=deleted if self.created else None, original_service_unchanged=after == self.before)
        (self.output / 'cleanup.json').write_text(json.dumps(value))
        if remaining:
            raise RuntimeError('namespace contains remaining processes')
        if after != self.before:
            raise RuntimeError('original service changed')
        return False

    # ----- helpers ---------------------------------------------------------

    def api(self, method, path, body=None, timeout=60):
        """One management request from inside the namespace; returns (status, text, seconds)."""
        code = ('import json,sys,time,urllib.request,urllib.error\n'
                'method,url,body=sys.argv[1:4]\n'
                'data=None if body=="-" else body.encode()\n'
                'request=urllib.request.Request(url,data=data,method=method,headers={"Content-Type":"application/json"})\n'
                'began=time.monotonic()\n'
                'try:\n'
                '    with urllib.request.urlopen(request,timeout=%d) as response: status,text=response.status,response.read().decode()\n'
                'except urllib.error.HTTPError as error: status,text=error.code,error.read().decode()\n'
                'print(json.dumps([status,text,time.monotonic()-began]))\n' % timeout)
        out = subprocess.check_output(['ip', 'netns', 'exec', self.ns, 'python3', '-c', code, method,
                                       f'http://127.0.0.1:{self.api_port}{path}',
                                       '-' if body is None else (body if isinstance(body, str) else json.dumps(body))])
        status, text, seconds = json.loads(out)
        return status, text, seconds

    def inject(self, table):
        """Atomically replaces faults.json (the mock applies it within ~50 ms)."""
        path = self.root / 'faults.json'
        temporary = self.root / 'faults.json.tmp'
        temporary.write_text(json.dumps(table))
        os.replace(temporary, path)

    def tcp_connections(self):
        """ESTABLISHED/other TCP states of the host process, by remote port."""
        counts = {}
        for name in ('tcp', 'tcp6'):
            try:
                lines = pathlib.Path(f'/proc/{self.host.pid}/net/{name}').read_text().splitlines()[1:]
            except OSError:
                continue
            for line in lines:
                fields = line.split()
                state, remote = fields[3], fields[2]
                port = int(remote.rsplit(':', 1)[1], 16)
                key = f'{state}:{port}'
                counts[key] = counts.get(key, 0) + 1
        return counts
