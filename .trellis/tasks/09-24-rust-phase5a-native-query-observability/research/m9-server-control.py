#!/usr/bin/env python3
"""Start/stop only owned cache/routing sessions; no benchmark traffic is generated here."""
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
INPUT = BASE / 'measurement-v9'
RESULTS = BASE / 'results-m9-server'
BEFORE = Path('/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/mosdns-rust')
BASELINE_SHA = '370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa'
SCENARIO = 'w2'
VARIANT = 'before_off'
CANDIDATE = BASE / 'candidate-m8/rust/target/release/mosdns'
CANDIDATE_SHA = '13785b388787fe87f370f608f0f69392288ec0631ba8210e410aa317441130ce'
HELPER_SHA = '1fceab7d2f26dbd40dab8b06e56026482fd42168ac7b8d13f792c3e6076ee2f3'


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
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(address)


def bound_pid(address, pid):
    rows = subprocess.check_output(['ss', '-H', '-lunp'], text=True)
    return any(f'{address[0]}:{address[1]}' in row and f'pid={pid},' in row for row in rows.splitlines())


def start(root):
    root.mkdir(parents=True, exist_ok=False)
    (root / 'owned.json').write_text('{}')
    try:
        start_session(root)
    except Exception as error:
        (root / 'startup-error.txt').write_text(str(error) + '\n')
        raise


def start_session(root):
    helper = INPUT / 'phase5a-baseline-helper-v11'
    config = INPUT / (SCENARIO + ('-on.yaml' if VARIANT == 'after_on' else '-off.yaml'))
    if socket.gethostname() != 'mosdns-rust' or int(subprocess.check_output(['getconf', '_NPROCESSORS_ONLN'])) != 2:
        raise ValueError('wrong fixed server host/topology')
    for path, expected in ((BEFORE, BASELINE_SHA), (helper, HELPER_SHA)):
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError('wrong fixed executable identity')
    name, port = ('cache', 15355) if SCENARIO == 'w2' else ('routing', 15356)
    original = (INPUT / f'repo/tests/phase5a-baseline/configs/{name}.yaml').read_bytes()
    expected_sha = '7f521ae011c28b4e102ff75437ebb943ba11f4bdc09aa656c41290ccde4b61c7' if SCENARIO == 'w2' else '66f358dbe2315df2aab1ad81668cec980b2f11aa68f863acad522c8e0c9fc651'
    if hashlib.sha256(original).hexdigest() != expected_sha:
        raise ValueError('wrong original UDP scenario config')
    expected = original.replace(f'127.0.0.1:{port}'.encode(), f'10.0.0.92:{port}'.encode())
    if VARIANT == 'after_on':
        expected = expected.replace(b'enable_audit: false', b'enable_audit: true')
    if config.read_bytes() != expected:
        raise ValueError('LAN overlay changes more than listener bind')
    specs = [('fixture', 'cache', 15455)] if SCENARIO == 'w2' else [('fixture', 'route_a', 15456), ('fixture_b', 'route_b', 15457), ('fixture_c', 'route_c', 15458)]
    for address in [('10.0.0.92', port)] + [('127.0.0.1', item[2]) for item in specs]:
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
        for role, upstream, fixture_port in specs:
            command = [str(helper), 'fixture', '--network', 'udp', '--addr', f'127.0.0.1:{fixture_port}',
                       '--upstream-id', upstream, '--counter', str(root / f'fixture-{upstream}.json')]
            if SCENARIO == 'w3':
                command += ['--event-journal', str(root / 'routing-events.jsonl')]
            fixture = launch(role, '1', command, env)
            deadline = time.monotonic() + 5
            while not bound_pid(('127.0.0.1', fixture_port), fixture.pid):
                if fixture.poll() is not None or time.monotonic() >= deadline:
                    raise ValueError('fixture startup failed')
                time.sleep(.05)
            actual_env = dict(item.split('=', 1) for item in Path(f'/proc/{fixture.pid}/environ').read_text().split('\0') if '=' in item)
            (root / f'{role}-profile.json').write_text(json.dumps({key: actual_env.get(key, '') for key in ('GOMAXPROCS', 'GOGC', 'GODEBUG', 'GOMEMLIMIT')}))
        sut = launch('sut', '0', [str(BEFORE), 'start', '-c', str(config)], os.environ.copy())
        deadline = time.monotonic() + 5
        while True:
            if sut.poll() is not None or time.monotonic() >= deadline:
                raise ValueError('SUT startup failed')
            if bound_pid(('10.0.0.92', port), sut.pid):
                break
            time.sleep(.05)
        for role in owned:
            subprocess.run([str(helper), 'verify-affinity', '--pid', str(owned[role]['pid']), '--expected', '0' if role == 'sut' else '1'], check=True)
        (root / 'sut.json').write_text(json.dumps({'sha256': BASELINE_SHA, 'path': str(BEFORE), 'variant': VARIANT, 'scenario': SCENARIO, 'audit_enabled': VARIANT == 'after_on', 'config_sha256': hashlib.sha256(config.read_bytes()).hexdigest()}))
        (root / 'ready').write_text(root.name + '\n')
    except Exception:
        for role in ['sut', 'fixture', 'fixture_b', 'fixture_c']:
            if role in owned:
                terminate_owned(owned[role])
        raise


def main():
    global BEFORE, BASELINE_SHA, VARIANT, SCENARIO
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=('start', 'stop'))
    parser.add_argument('--result', required=True)
    parser.add_argument('--scenario', choices=('w2', 'w3'))
    parser.add_argument('--variant', choices=('before_off', 'after_off', 'after_on'))
    args = parser.parse_args()
    root = Path(args.result).resolve()
    if root.parent != RESULTS or not re.fullmatch(r'm9-[a-z0-9_-]+', root.name):
        raise ValueError('result must be a fresh owned M9 server session')
    if args.command == 'start':
        if not args.variant or not args.scenario:
            raise ValueError('start requires actual binary/audit variant')
        VARIANT, SCENARIO = args.variant, args.scenario
        if VARIANT != 'before_off':
            BEFORE, BASELINE_SHA = CANDIDATE, CANDIDATE_SHA
        start(root)
    else:
        records = json.loads((root / 'owned.json').read_text())
        if not set(records) <= {'sut', 'fixture', 'fixture_b', 'fixture_c'}:
            raise ValueError('unexpected ownership record')
        for role in ['sut', 'fixture', 'fixture_b', 'fixture_c']:
            if role in records:
                terminate_owned(records[role])
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
