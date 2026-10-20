"""Task-owned systemd deployment/control. Reboot restores guards, never replay."""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import time

from isolation_probe import counters, main as isolation_probe
from longrun_report import generate
from longrun_store import atomic
from network import setup
from unattended import boot_id, load

PREFIX = 'shadow1-48h'
EVENT_TIMERS = [f'{PREFIX}-event{i}.timer' for i in range(1, 5)]


def command(*args):
    return subprocess.check_output(list(args), text=True)


def network(root):
    current_boot = boot_id()
    work = root / 'network'
    runtime = load(root / 'runtime.json')
    assert runtime and runtime.get('target'), 'private runtime input must exist before creating any guard'
    present = 'shadow1-replay' in command('ip', 'netns', 'list').split()
    if present:
        assert load(work / 'boot.json')['boot_id'] == current_boot
        command('nft', 'list', 'table', 'netdev', 'shadow1_guard')
        command('nft', 'list', 'table', 'inet', 'shadow1')
        assert len(counters()) == 3
        return
    tables = command('nft', 'list', 'tables')
    assert 'shadow1' not in tables, 'partial isolation state; refuse to reset counters'
    if work.exists():
        work.rename(root / ('network-before-' + str(int(time.time()))))
    work.mkdir(mode=0o700)
    iface = json.loads(command('ip', '-j', 'route', 'show', 'default'))[0]['dev']
    assert iface.replace('-', '').replace('_', '').isalnum()
    guard = f'''table netdev shadow1_guard {{
 chain physical_out {{
  type filter hook egress device "{iface}" priority 500; policy accept;
  udp sport 53 counter drop
  tcp sport 53 counter drop
 }}
}}
'''
    (work / 'guard.nft').write_text(guard)
    command('nft', '-f', str(work / 'guard.nft'))
    setup(work, runtime['target'])
    atomic(work / 'boot.json', {'boot_id': current_boot, 'created_wall': time.time(),
                               'counter_reset_reason': 'new boot; previous volatile guards were lost'})
    assert counters() == [0, 0, 0]


def install(root):
    assert root.is_absolute() and ' ' not in str(root)
    scripts = pathlib.Path(__file__).parent.resolve()
    units = pathlib.Path('/etc/systemd/system')
    ctl = f'/usr/bin/python3 {scripts / "longrun_control.py"} {root}'
    common = 'StartLimitIntervalSec=172900\nStartLimitBurst=3\n'
    files = {
        'shadow1-isolation.service': f'''[Unit]
Description=Shadow1 persistent veth and physical egress guards
After=network-online.target
Wants=network-online.target
Before=mosdns.service {PREFIX}.service
{common}
[Service]
Type=oneshot
ExecStart={ctl} network
RemainAfterExit=yes
Restart=on-failure
RestartSec=5
UMask=0077
[Install]
WantedBy=multi-user.target
''',
        f'{PREFIX}.service': f'''[Unit]
Description=Shadow1 unattended 48h capture replay collector
Requires=shadow1-isolation.service
After=shadow1-isolation.service mosdns.service
OnFailure={PREFIX}-failed.service
{common}
[Service]
Type=exec
ExecStart=/usr/bin/python3 -u {scripts / 'unattended.py'} {root}
ExecStopPost={ctl} segment-exit
Restart=on-failure
RestartSec=10
KillMode=mixed
TimeoutStopSec=40
UMask=0077
''',
        f'{PREFIX}-end.service': f'''[Unit]
Description=Shadow1 stop capture/replay and produce final numerical report
{common}
[Service]
Type=oneshot
ExecStart={ctl} finish
Restart=on-failure
RestartSec=10
TimeoutStartSec=10min
UMask=0077
''',
        f'{PREFIX}-end.timer': f'''[Unit]
Description=Shadow1 48h deadline plus initial baseline and final drain
[Timer]
OnActiveSec=172885s
AccuracySec=1s
Unit={PREFIX}-end.service
''',
        f'{PREFIX}-failed.service': f'''[Unit]
Description=Shadow1 bounded tool restarts exhausted
[Service]
Type=oneshot
ExecStart={ctl} failed
UMask=0077
''',
        f'{PREFIX}-boot.service': f'''[Unit]
Description=Shadow1 record interrupted run after reboot without replay resume
After=mosdns.service shadow1-isolation.service
Requires=shadow1-isolation.service
[Service]
Type=oneshot
ExecStart={ctl} boot
UMask=0077
[Install]
WantedBy=multi-user.target
'''}
    for i, hour in enumerate((6, 18, 30, 42), 1):
        files[f'{PREFIX}-event{i}.service'] = f'''[Unit]
Description=Shadow1 once-only event {i}
After=shadow1-isolation.service
{common}
[Service]
Type=oneshot
ExecStart=/usr/bin/python3 {scripts / 'longrun_events.py'} {root} {i}
ExecStopPost=/usr/bin/python3 {scripts / 'longrun_events.py'} {root} {i} --recover
Restart=on-failure
RestartSec=10
TimeoutStartSec=5min
UMask=0077
'''
        files[f'{PREFIX}-event{i}.timer'] = f'''[Unit]
Description=Shadow1 event {i} at approximately hour {hour}
[Timer]
OnActiveSec={hour}h
AccuracySec=1s
Unit={PREFIX}-event{i}.service
'''
    for name, source in files.items():
        target = units / name
        if target.exists() and not (root / 'units.json').exists():
            raise RuntimeError('refuse to overwrite preexisting unit: ' + name)
        target.write_text(source)
    atomic(root / 'units.json', sorted(files))
    command('systemctl', 'daemon-reload')
    command('systemctl', 'enable', 'shadow1-isolation.service', f'{PREFIX}-boot.service')
    command('systemctl', 'start', 'shadow1-isolation.service')


def start(root):
    assert not (root / 'run.json').exists(), 'a new run needs a fresh private directory'
    assert command('systemctl', 'show', f'{PREFIX}.service', '-p', 'ActiveState', '--value').strip() != 'active'
    # Prior, explicitly archived preflight windows used the same unit name.
    # Reset its start limiter only here, before creating a fresh run manifest;
    # ordinary automatic restarts never reset this budget.
    command('systemctl', 'reset-failed', f'{PREFIX}.service')
    assert command('systemctl', 'is-active', 'mosdns.service').strip() == 'active'
    assert command('systemctl', 'is-enabled', 'mosdns.service').strip() == 'enabled'
    isolation_probe(root / 'network')
    probe = load(root / 'network/isolation-probe.json')
    assert probe['answer']['rcode'] == 0 and probe['answer']['has_answer']
    assert probe['physical_observed_udp53'] == 0 and counters() == [0, 0, 0]
    stamp = time.time()
    manifest = {'activated_wall': stamp, 'capture_start_wall': stamp + 60, 'end_wall': stamp + 60 + 172800,
                'boot_id': boot_id(), 'duration_seconds': 172800,
                'source_head': '5cc3bdb5612727ea1f711a4d09669be3c098661a',
                'event_hours': [6,18,30,42], 'start_guard': counters(),
                'rust_baseline': command('systemctl','show','mosdns.service','-p','MainPID','-p','NRestarts','-p','ActiveState').splitlines()}
    atomic(root / 'run.json', manifest)
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in pathlib.Path(__file__).parent.glob('*.py')}
    atomic(root / 'tool-hashes.json', hashes)
    command('systemctl', 'start', f'{PREFIX}.service', *EVENT_TIMERS, f'{PREFIX}-end.timer')
    print(json.dumps(manifest))


def finish(root):
    # Disable all future task events first; never stop mosdns.service.
    command('systemctl', 'stop', *EVENT_TIMERS, f'{PREFIX}-end.timer')
    command('systemctl', 'stop', f'{PREFIX}.service')
    if not (root / 'stop-status.json').exists():
        manifest = load(root / 'run.json')
        done = time.time() >= manifest['end_wall']
        atomic(root / 'stop-status.json', {'wall': time.time(), 'kind': 'completed' if done else 'operator_stop',
            'reason': '48h deadline' if done else 'safe manual stop', 'rust_left_running': True})
    generate(root)
    print(json.dumps({'report': str(root/'final-summary.json'), 'guard': counters(),
                      'namespace_pids': command('ip','netns','pids','shadow1-replay').split(),
                      'rust_active': command('systemctl','is-active','mosdns.service').strip()}))


def segment_exit(root):
    status = load(root / 'stop-status.json')
    manifest = load(root / 'run.json')
    if status or time.time() >= manifest['end_wall']:
        command('systemctl', 'start', '--no-block', f'{PREFIX}-end.service')


def failed(root):
    if not (root / 'stop-status.json').exists():
        atomic(root / 'stop-status.json', {'wall': time.time(), 'kind': 'tool_failure',
            'reason': 'bounded systemd collector restart budget exhausted', 'rust_left_running': True})
    command('systemctl', 'start', '--no-block', f'{PREFIX}-end.service')


def boot(root):
    manifest = load(root / 'run.json')
    if manifest and manifest['boot_id'] != boot_id() and not (root/'stop-status.json').exists():
        atomic(root / 'stop-status.json', {'wall': time.time(), 'kind': 'reboot_interruption',
            'reason': 'host reboot ended the segment; replay was not resumed', 'rust_left_running': True})
        from longrun_events import recover
        for number in range(1, 5):
            try:
                recover(root, number)
            except Exception:
                pass  # Each failed recovery is retained in its event record.
        generate(root)


def status(root):
    segments = sorted((root / 'segments').glob('[0-9][0-9][0-9]'))
    last = segments[-1] if segments else None
    summary = load(last/'summary.json') if last else None
    print(json.dumps({'run': load(root/'run.json'), 'stop': load(root/'stop-status.json'),
        'last_segment': str(last), 'summary': summary, 'guard': counters(),
        'units': command('systemctl','list-units','--all',PREFIX+'*','shadow1-isolation.service','--no-pager'),
        'timers': command('systemctl','list-timers','--all',PREFIX+'*','--no-pager')}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=pathlib.Path)
    parser.add_argument('action', choices=('network','install','start','finish','segment-exit','failed','boot','status'))
    args = parser.parse_args()
    os.umask(0o077)
    {'network':network,'install':install,'start':start,'finish':finish,'segment-exit':segment_exit,
     'failed':failed,'boot':boot,'status':status}[args.action](args.root)
