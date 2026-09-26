#!/usr/bin/env python3
"""Local controller for fixed two-host W1 controls; no credentials in files."""
import argparse
import csv
import hashlib
import json
import os
import shlex
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = next((parent for parent in HERE.parents if (parent / 'go.mod').exists()), HERE / 'repo')
SERVER_BASE = '/root/mosdns-rust-phase5a-native-query-observability-545ba29'
SERVER_INPUT = SERVER_BASE + '/measurement-v5'
SERVER_RESULTS = SERVER_BASE + '/results-m5-server'
CLIENT_BASE = '/root/mosdns-phase5a-m5-client'
CLIENT_INPUT = CLIENT_BASE + '/tools'
CLIENT_RESULTS = CLIENT_BASE + '/results'
FIELDS = ['scenario', 'repetition', 'variant', 'pair_position', 'result_dir', 'runner_exit']
HELPER_SHA = '28d5faf5f5129aa990aac51efd8752eba655b852f0bcb27619b216e572c0450e'
BASELINE_SHA = '370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa'
WORKLOAD_SHA = '32ae1a43cae5f4e85b0a058d389d2071a8d7cf844aad98b31137f95b57715aa2'
TOOLS = ('run-m5-w1.py', 'm5-remote-tools.py', 'm5-server-control.py', 'qualify-m5-w1.py',
         'qualify-m2-controls.py', 'summarize-slice3-pilot-v2.py', 'measurement-revision-v5.md')


def input_identity(args):
    hashes = {file: hashlib.sha256((HERE / file).read_bytes()).hexdigest() for file in TOOLS}
    server = {SERVER_INPUT + '/' + file: hashes[file] for file in ('m5-remote-tools.py', 'm5-server-control.py')}
    server.update({SERVER_INPUT + '/phase5a-baseline-helper-v10': HELPER_SHA,
                   SERVER_INPUT + '/repo/tests/phase5a-baseline/workloads/forward.jsonl': WORKLOAD_SHA,
                   SERVER_INPUT + '/forward-tcp-lan.yaml': 'bfd243afbbf26cf8d890fc99bbf24a6d5ba2087c728ed7d4692aced0aca0cb7e',
                   '/root/mosdns-rust-phase5a-first-native-performance-605c305/bin/official-v1/mosdns-rust': BASELINE_SHA})
    client = {CLIENT_INPUT + '/phase5a-baseline-helper-v10': HELPER_SHA, CLIENT_INPUT + '/forward.jsonl': WORKLOAD_SHA}
    source = {str(path.relative_to(REPO)): hashlib.sha256(path.read_bytes()).hexdigest()
              for path in (REPO / 'tests/phase5a-baseline/cmd/phase5a-baseline').glob('*.go')}
    server.update({SERVER_INPUT + '/repo/' + path: sha for path, sha in source.items()})
    for is_client, files in ((False, server), (True, client)):
        actual = remote(args, is_client, quoted(['sha256sum', *files])).stdout.splitlines()
        observed = {line.split(maxsplit=1)[1]: line.split()[0] for line in actual}
        if observed != files:
            raise ValueError('fixed remote input identity differs')
    return dict(helper_sha256=HELPER_SHA, baseline_sha256=BASELINE_SHA, local_tools=hashes, helper_source=source,
                server_inputs=server, client_inputs=client)


def make_plan(root):
    orders = [('before_off', 'after_off', 'after_on'), ('after_off', 'after_on', 'before_off'),
              ('after_on', 'before_off', 'after_off')]
    return {batch: [dict(scenario='w1-tcp', repetition=rep, variant=variant,
                         pair_position=1 if variant == 'before_off' else 2,
                         run_id=f'm5-{batch}-w1-tcp-r{rep}-{variant}',
                         result_dir=str(root / batch / f'w1-tcp-r{rep}-{variant}'))
                    for rep, order in enumerate(orders, 1) for variant in order]
            for batch in ('batch1', 'batch2')}


def remote(args, client, command, timeout=15, check=True):
    options = ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5']
    if client:
        options += ['-S', args.client_control]
    target = 'root@10.0.0.50' if client else 'mosdns-rust'
    return subprocess.run(['ssh', *options, target, command], capture_output=True,
                          text=True, timeout=timeout, check=check)


def transfer(args, client, source, destination, upload=False, recursive=False):
    options = ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5']
    if client:
        options += ['-o', 'ControlPath=' + args.client_control]
    target = 'root@10.0.0.50' if client else 'mosdns-rust'
    files = [source, f'{target}:{destination}'] if upload else [f'{target}:{source}', destination]
    subprocess.run(['scp', *options, *(['-r'] if recursive else []), *files], check=True, timeout=30, capture_output=True)


def quoted(parts):
    return shlex.join([str(part) for part in parts])


def inventory(args, client):
    command = "hostname; uname -srmo; getconf _NPROCESSORS_ONLN; getconf CLK_TCK; awk '/MemAvailable:/ {print $2}' /proc/meminfo; df -PT /root; ip route get " + ('10.0.0.92' if client else '10.0.0.50')
    text = remote(args, client, command).stdout
    rows = text.splitlines()
    if (rows[0] != ('Debian' if client else 'mosdns-rust') or int(rows[2]) != (1 if client else 2)
            or int(rows[3]) != 100 or int(rows[4]) < (786432 if client else 2097152)
            or ' ext4 ' not in text or ('src 10.0.0.50' if client else 'src 10.0.0.92') not in text):
        raise ValueError('fixed host/network/memory/filesystem preflight failed')
    helper = (CLIENT_INPUT if client else SERVER_INPUT) + '/phase5a-baseline-helper-v10'
    sha = remote(args, client, quoted(['sha256sum', helper])).stdout.split()[0]
    if sha != HELPER_SHA or remote(args, client, quoted([helper, 'version'])).stdout.strip() != 'phase5a-baseline-helper/v10':
        raise ValueError('fixed helper identity failed')
    return text


def wait_file(args, path):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if remote(args, False, quoted(['test', '-f', path]), check=False).returncode == 0:
            return
        time.sleep(.1)
    raise ValueError('server readiness barrier failed')


def attempt(args, row):
    root = Path(row['result_dir'])
    root.mkdir()
    run_id = row['run_id']
    server = SERVER_RESULTS + '/' + run_id
    client = CLIENT_RESULTS + '/' + run_id
    helper = SERVER_INPUT + '/phase5a-baseline-helper-v10'
    started = False
    code = 0
    (root / 'loadavg.tsv').write_text('loadavg_before_utc=' + datetime.now(timezone.utc).isoformat() + '\n' + remote(args, False, 'cat /proc/loadavg').stdout)
    try:
        (root / 'input-identity.json').write_text(json.dumps(input_identity(args), indent=2) + '\n')
        remote(args, True, quoted(['mkdir', client]))
        remote(args, False, quoted(['python3', SERVER_INPUT + '/m5-server-control.py', 'start', '--result', server]))
        started = True
        owned = json.loads(remote(args, False, quoted(['cat', server + '/owned.json'])).stdout)
        for stage, qps in (('normal-reference', 200), ('overload', 400)):
            sampler_root = server + '/server-' + stage
            sampler = ['nohup', 'taskset', '-c', '1', 'python3', SERVER_INPUT + '/m5-remote-tools.py', 'sample-server',
                       '--sut-pid', owned['sut']['pid'], '--fixture-pid', owned['fixture']['pid'],
                       '--run-id', run_id, '--stage', stage, '--result', sampler_root]
            launch = quoted(sampler) + ' </dev/null >' + shlex.quote(server + '/sampler-' + stage + '.stdout') + ' 2>' + shlex.quote(server + '/sampler-' + stage + '.stderr') + ' &'
            # stdin/output detached; sampler has its own 35-second termination bound.
            remote(args, False, launch)
            wait_file(args, sampler_root + '/ready')
            client_stage = client + '/' + stage
            run = ['env', '-u', 'GOMEMLIMIT', 'GOMAXPROCS=1', 'GOGC=off', 'GODEBUG=gctrace=1',
                   'taskset', '-c', '0', CLIENT_INPUT + '/phase5a-baseline-helper-v10', 'run', '--sample-self',
                   '--workload', CLIENT_INPUT + '/forward.jsonl', '--scenario', 'w1', '--transport', 'tcp',
                   '--addr', '10.0.0.92:15354', '--stage', stage, '--qps', qps, '--duration', '25000ms',
                   '--deadline', '500ms', '--late-drain', '100ms', '--run-id', run_id, '--fixture-session-id', run_id,
                   '--result', client_stage, '--request-ledger', client + '/requests.jsonl', '--fail-on-error']
            try:
                response = remote(args, True, quoted(run) + ' >' + shlex.quote(client + '/' + stage + '.stdout') + ' 2>' + shlex.quote(client + '/' + stage + '.stderr'), timeout=32, check=False)
                code = max(code, response.returncode)
                remote(args, True, quoted(['mv', client_stage + '/stages.jsonl', client_stage + '/stages.client.jsonl']))
            finally:
                remote(args, False, quoted(['touch', sampler_root + '/stop']))
            wait_file(args, sampler_root + '/metadata.json')
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        code = 1
        (root / 'controller-error.txt').write_text(str(error) + '\n')
    finally:
        if started:
            try:
                remote(args, False, quoted(['python3', SERVER_INPUT + '/m5-server-control.py', 'stop', '--result', server]))
            except (ValueError, OSError, subprocess.SubprocessError) as error:
                code = 1
                (root / 'cleanup-error.txt').write_text(str(error) + '\n')
    try:
        transfer(args, True, client, str(root / 'client'), recursive=True)
        transfer(args, False, server, str(root / 'server'), recursive=True)
        for stage in ('normal-reference', 'overload'):
            subprocess.run(['python3', str(HERE / 'm5-remote-tools.py'), 'merge-stage', '--client', str(root / 'client' / stage),
                            '--server', str(root / 'server' / ('server-' + stage)), '--result', str(root), '--client-host', 'Debian'], check=True, capture_output=True)
        transfer(args, False, str(root / 'stages.jsonl'), server + '/stages.merged.jsonl', upload=True)
        checks = []
        for stage in ('normal-reference', 'overload'):
            checks += [[helper, command, '--stage-result', server + '/stages.merged.jsonl', '--stage', stage]
                       for command in ('verify-stage', 'verify-sender')]
            checks += [[helper, 'verify-samples', '--stage-result', server + '/stages.merged.jsonl', '--stage', stage, '--expected-fixtures', '1']]
        checks += [[helper, 'verify-session-counters', '--scenario', 'w1', '--workload', SERVER_INPUT + '/repo/tests/phase5a-baseline/workloads/forward.jsonl',
                    '--counter', server + '/fixture-forward.json', '--stage-result', server + '/stages.merged.jsonl', '--run-id', run_id]]
        for command in checks:
            response = remote(args, False, quoted(command), check=False)
            with (root / 'oracle-checks.txt').open('a') as stream:
                stream.write(quoted(command) + '\n' + response.stdout + response.stderr + f'exit={response.returncode}\n')
            code = max(code, response.returncode)
        (root / 'sut.json').write_bytes((root / 'server/sut.json').read_bytes())
        if json.loads((root / 'input-identity.json').read_text()) != input_identity(args):
            raise ValueError('input identity changed during attempt')
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        code = 1
        (root / 'evidence-error.txt').write_text(str(error) + '\n')
    (root / 'environment.txt').write_text('measurement_profile=m5\ngomaxprocs_environment=1\nclient_host=Debian\nserver_host=mosdns-rust\ngogc_environment=off\ngodebug_environment=gctrace=1\ngomemlimit_environment=unset\n')
    (root / 'run-metadata.txt').write_text('stage_duration_ms=25000\nnormal_reference_qps=200\noverload_qps=400\nrequest_deadline_ms=500\nlate_drain_ms=100\n')
    with (root / 'loadavg.tsv').open('a') as stream:
        stream.write('loadavg_after_utc=' + datetime.now(timezone.utc).isoformat() + '\n' + remote(args, False, 'cat /proc/loadavg').stdout)
    if code:
        (root / 'invalid-stages.tsv').write_text('counters\tdistributed attempt failed; see retained errors and oracles\n')
    return code


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--mode', choices=('plan', 'preflight', 'run'), required=True)
    parser.add_argument('--result-root', required=True)
    parser.add_argument('--client-control')
    parser.add_argument('--reviewed-head', help='exact approved source commit; required for measured run')
    args = parser.parse_args()
    root = Path(args.result_root).resolve()
    plans = make_plan(root)
    if args.mode == 'plan':
        print(json.dumps(plans, indent=2))
        return 0
    if not args.client_control or not Path(args.client_control).exists():
        raise ValueError('existing authenticated local client SSH multiplex socket required')
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    if args.mode == 'run' and args.reviewed_head != head:
        raise ValueError('measured run requires exact reviewed HEAD')
    root.mkdir(parents=True, exist_ok=False)
    (root / 'client-inventory.txt').write_text(inventory(args, True))
    (root / 'server-inventory.txt').write_text(inventory(args, False))
    (root / 'input-identity.json').write_text(json.dumps(input_identity(args), indent=2) + '\n')
    for batch, rows in plans.items():
        directory = root / batch
        directory.mkdir()
        with (directory / 'attempt-order-plan.tsv').open('x') as stream:
            for row in rows:
                stream.write('\t'.join(str(row[k]) for k in FIELDS[:-1]) + '\n')
        (directory / 'attempt-order.tsv').write_text('\t'.join(FIELDS) + '\n')
    (root / 'run-audit.txt').write_text('start_utc=' + datetime.now(timezone.utc).isoformat() + '\nmode=identical-baseline W1 calibration; every slot audit off\n')
    (root / 'source-head.txt').write_text(head + '\n')
    if args.mode == 'preflight':
        return 0
    for batch, rows in plans.items():
        for row in rows:
            code = attempt(args, row)
            with (root / batch / 'attempt-order.tsv').open('a') as stream:
                stream.write('\t'.join(str(row[k]) for k in FIELDS[:-1]) + '\t' + str(code) + '\n')
            print(f"{batch} r{row['repetition']} {row['variant']} runner_exit={code}", flush=True)
        response = subprocess.run(['python3', str(HERE / 'summarize-slice3-pilot-v2.py'), str(root / batch)], capture_output=True, text=True, check=True)
        (root / batch / 'analysis-summary.txt').write_text(response.stdout)
    with (root / 'run-audit.txt').open('a') as stream:
        stream.write('finished_utc=' + datetime.now(timezone.utc).isoformat() + '\n')
    response = subprocess.run([sys.executable, str(HERE / 'qualify-m5-w1.py'), str(root)], capture_output=True, text=True)
    (root / 'qualification.json').write_text(response.stdout)
    if response.stderr:
        (root / 'qualification.stderr').write_text(response.stderr)
    manifest = {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in sorted(root.rglob('*')) if path.is_file()}
    (root / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return response.returncode


if __name__ == '__main__':
    sys.exit(main())
