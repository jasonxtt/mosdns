#!/usr/bin/env python3
"""Start/stop only owned W1 sessions; no benchmark traffic is generated here."""
import argparse
import hashlib
import json
import os
import re
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

BASE = Path('/root/mosdns-rust-phase5a-native-query-observability-545ba29')
INPUT = BASE / 'measurement-v7'
RESULTS = BASE / 'results-m7-server'
BEFORE = Path('/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/mosdns-rust')
BASELINE_SHA = '370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa'
VARIANT = 'before_off'
CANDIDATE = BASE / 'candidate-v12/rust/target/release/mosdns'
CANDIDATE_SHA = '8d3e9dc7f5f1ae46365dda4d46e72b2e24dc2ca2b94b8e8c91f70e4e8e9edd0d'
HELPER_SHA = '28d5faf5f5129aa990aac51efd8752eba655b852f0bcb27619b216e572c0450e'


def process_start(pid):
    text = Path(f'/proc/{pid}/stat').read_text()
    return text[text.rindex(')') + 2:].split()[19]


def terminate_owned(record):
    pid = record['pid']
    try:
        identity = process_start(pid)
    except FileNotFoundError:
        return
    if identity != record['start_identity']:
        raise ValueError('process ownership changed; refusing signal')
    try:
        descriptor = os.pidfd_open(pid)
    except ProcessLookupError:
        return
    try:
        if process_start(pid) != identity:
            raise ValueError('process ownership changed before signal')
        signal.pidfd_send_signal(descriptor, signal.SIGTERM)
    except (FileNotFoundError, ProcessLookupError):
        return
    finally:
        os.close(descriptor)
    deadline = time.monotonic() + 5
    while Path(f'/proc/{pid}/stat').exists():
        try:
            text = Path(f'/proc/{pid}/stat').read_text()
        except FileNotFoundError:
            return
        if text[text.rindex(')') + 2:].split()[0] == 'Z':
            return
        if time.monotonic() >= deadline:
            raise ValueError('owned process did not stop within bound')
        time.sleep(.05)


def probe_available(address):
    with socket.socket() as probe:
        # Match the listener's normal bind behavior without SO_REUSEPORT.
        # Closed-session TIME_WAIT is allowed; an active listener still fails.
        probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        probe.bind(address)


def start(root):
    root.mkdir(parents=True, exist_ok=False)
    (root / 'owned.json').write_text('{}')
    try:
        start_session(root)
    except Exception as error:
        (root / 'startup-error.txt').write_text(str(error) + '\n')
        raise


def start_session(root):
    helper = INPUT / 'phase5a-baseline-helper-v10'
    config = INPUT / ('forward-on.yaml' if VARIANT == 'after_on' else 'forward-off.yaml')
    if socket.gethostname() != 'mosdns-rust' or int(subprocess.check_output(['getconf', '_NPROCESSORS_ONLN'])) != 2:
        raise ValueError('wrong fixed server host/topology')
    for path, expected in ((BEFORE, BASELINE_SHA), (helper, HELPER_SHA)):
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError('wrong fixed executable identity')
    original = (INPUT / 'repo/tests/phase5a-baseline/configs/forward-tcp.yaml').read_bytes()
    if hashlib.sha256(original).hexdigest() != '1a2280f44ad07ec2111eeb09f6d1904e66b65d52a20f2df1bb14fff22458b8a1':
        raise ValueError('wrong original TCP config')
    expected = original.replace(b'listen: "127.0.0.1:15354"', b'listen: "10.0.0.92:15354"')
    if VARIANT == 'after_on':
        expected = expected.replace(b'enable_audit: false', b'enable_audit: true')
    if config.read_bytes() != expected:
        raise ValueError('LAN overlay changes more than listener bind')
    for address in (('10.0.0.92', 15354), ('127.0.0.1', 15454)):
        probe_available(address)
    env = dict(os.environ, GOMAXPROCS='1', GOGC='off', GODEBUG='gctrace=1')
    env.pop('GOMEMLIMIT', None)
    owned = {}
    try:
        def launch(role, cpus, command, environment):
            with (root / f'{role}.stdout').open('x') as stdout, (root / f'{role}.stderr').open('x') as stderr:
                child = subprocess.Popen(['taskset', '-c', cpus, *command], env=environment,
                                         stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, start_new_session=True)
            owned[role] = dict(pid=child.pid, start_identity=process_start(child.pid))
            (root / 'owned.json').write_text(json.dumps(owned, indent=2))
            return child
        fixture = launch('fixture', '1', [str(helper), 'fixture', '--network', 'tcp', '--addr', '127.0.0.1:15454',
                         '--upstream-id', 'forward', '--counter', str(root / 'fixture-forward.json')], env)
        deadline = time.monotonic() + 5
        while not (root / 'fixture-forward.json').exists():
            if fixture.poll() is not None or time.monotonic() >= deadline:
                raise ValueError('fixture startup failed')
            time.sleep(.05)
        sut = launch('sut', '0', [str(BEFORE), 'start', '-c', str(config)], os.environ.copy())
        deadline = time.monotonic() + 5
        while True:
            if sut.poll() is not None or time.monotonic() >= deadline:
                raise ValueError('SUT startup failed')
            try:
                with socket.create_connection(('10.0.0.92', 15354), timeout=.2):
                    break
            except OSError:
                time.sleep(.05)
        for role, cpus in (('sut', '0'), ('fixture', '1')):
            subprocess.run([str(helper), 'verify-affinity', '--pid', str(owned[role]['pid']), '--expected', cpus], check=True)
        actual_env = dict(item.split('=', 1) for item in Path(f'/proc/{fixture.pid}/environ').read_text().split('\0') if '=' in item)
        (root / 'fixture-profile.json').write_text(json.dumps({key: actual_env.get(key, '') for key in ('GOMAXPROCS', 'GOGC', 'GODEBUG', 'GOMEMLIMIT')}))
        (root / 'sut.json').write_text(json.dumps({'sha256': BASELINE_SHA, 'path': str(BEFORE), 'variant': VARIANT, 'audit_enabled': VARIANT == 'after_on', 'config_sha256': hashlib.sha256(config.read_bytes()).hexdigest()}))
        (root / 'ready').write_text(root.name + '\n')
    except Exception:
        for role in ('sut', 'fixture'):
            if role in owned:
                terminate_owned(owned[role])
        raise


def main():
    global BEFORE, BASELINE_SHA, VARIANT
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=('start', 'stop'))
    parser.add_argument('--result', required=True)
    parser.add_argument('--variant', choices=('before_off', 'after_off', 'after_on'))
    args = parser.parse_args()
    root = Path(args.result).resolve()
    if root.parent != RESULTS or not re.fullmatch(r'm7-[a-z0-9_-]+', root.name):
        raise ValueError('result must be a fresh owned M7 server session')
    if args.command == 'start':
        if not args.variant:
            raise ValueError('start requires actual binary/audit variant')
        VARIANT = args.variant
        if VARIANT != 'before_off':
            BEFORE, BASELINE_SHA = CANDIDATE, CANDIDATE_SHA
        start(root)
    else:
        records = json.loads((root / 'owned.json').read_text())
        if not set(records) <= {'sut', 'fixture'}:
            raise ValueError('unexpected ownership record')
        for role in ('sut', 'fixture'):
            if role in records:
                terminate_owned(records[role])
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
