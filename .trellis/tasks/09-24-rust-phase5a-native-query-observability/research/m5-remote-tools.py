#!/usr/bin/env python3
"""Host-local resource brackets for the prospective distributed W1 probe."""
import argparse
import hashlib
import json
import os
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path


def read_process(proc, pid, hz, role, run_id, stage, host):
    directory = proc / str(pid)
    stat = (directory / 'stat').read_text()
    fields = stat[stat.rindex(')') + 2:].split()
    user, system, start = int(fields[11]), int(fields[12]), fields[19]
    status = dict(line.split(':', 1) for line in (directory / 'status').read_text().splitlines() if ':' in line)
    rss = int(status['VmRSS'].split()[0])
    cpus = status['Cpus_allowed_list'].strip()
    if rss <= 0 or int(start) <= 0 or not cpus:
        raise ValueError('invalid process resource/identity evidence')
    sample = dict(run_id=run_id, stage_id=stage, role=role, host=host, pid=pid,
                  timestamp=datetime.now(timezone.utc).isoformat(), user_ticks=user,
                  system_ticks=system, clock_ticks_per_second=hz,
                  user_seconds=user / hz, system_seconds=system / hz,
                  rss_kib=rss, fd_count=len(list((directory / 'fd').iterdir())))
    return sample, start, cpus


def sample_server(args):
    output = Path(args.result)
    output.mkdir(parents=True, exist_ok=False)
    host = socket.gethostname()
    metadata = dict(valid=False, host=host, run_id=args.run_id, stage=args.stage,
                    proc_root=args.proc_root, counts={}, processes={})
    began = time.monotonic()
    try:
        targets = [('sut', args.sut_pid, args.sut_cpu)] + [(f'fixture-{i}', pid, args.fixture_cpu) for i, pid in enumerate(args.fixture_pid, 1)]
        expected_starts = {'sut': args.sut_start, 'fixture-1': args.fixture_start}
        if len(args.fixture_pid) != 1:
            raise ValueError('exactly one owned W1 fixture required')
        if args.max_seconds <= 0 or args.max_seconds > 60 or any(pid <= 0 for _, pid, _ in targets) or len({pid for _, pid, _ in targets}) != len(targets):
            raise ValueError('invalid bounded sampler settings or duplicate process')
        hz = int(subprocess.check_output(['getconf', 'CLK_TCK'], text=True).strip())
        if hz <= 0:
            raise ValueError('invalid CLK_TCK')
        metadata['clock_ticks_per_second'] = hz
        identities = {}
        with (output / 'resource-samples.jsonl').open('x') as stream:
            def sample_all():
                for role, pid, expected_cpu in targets:
                    sample, start, cpus = read_process(Path(args.proc_root), pid, hz, role, args.run_id, args.stage, host)
                    if start != expected_starts[role]:
                        raise ValueError('sampled PID differs from owned start identity')
                    if cpus != expected_cpu or (role in identities and identities[role] != start):
                        raise ValueError('process affinity or start identity changed')
                    identities[role] = start
                    metadata['processes'][role] = dict(pid=pid, start_identity=start, cpus=cpus)
                    stream.write(json.dumps(sample) + '\n')
                    metadata['counts'][role] = metadata['counts'].get(role, 0) + 1
                stream.flush()
            sample_all()
            (output / 'ready').write_text(args.run_id + '\n')
            deadline = time.monotonic() + args.max_seconds
            next_sample = time.monotonic() + 1
            while not (output / 'stop').exists():
                if time.monotonic() >= deadline:
                    raise ValueError('bounded server resource bracket timed out')
                if time.monotonic() >= next_sample:
                    sample_all()
                    next_sample += 1
                time.sleep(.02)
            sample_all()
        metadata['valid'] = True
    except (OSError, ValueError, KeyError, IndexError, subprocess.SubprocessError) as error:
        metadata['error'] = str(error)
    metadata['bracket_seconds'] = time.monotonic() - began
    (output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    return 0 if metadata['valid'] else 1


def merge_stage(args):
    client, server, output = Path(args.client), Path(args.server), Path(args.result)
    stages = [json.loads(line) for line in (client / 'stages.client.jsonl').read_text().splitlines()]
    if len(stages) != 1:
        raise ValueError('exactly one original client stage required')
    stage = stages[0]
    metadata = json.loads((server / 'metadata.json').read_text())
    if (metadata['valid'] is not True or metadata['host'] == args.client_host
            or metadata['proc_root'] != '/proc' or metadata['run_id'] != stage['run_id']
            or metadata['stage'] != stage['stage'] or stage['sut_pid'] != 0
            or stage['harness_cpu_set'] != '0' or stage['harness_host'] != args.client_host):
        raise ValueError('mismatched remote host/stage/PID namespace evidence')
    client_samples = [json.loads(line) for line in (client / 'resource-samples.jsonl').read_text().splitlines()]
    server_samples = [json.loads(line) for line in (server / 'resource-samples.jsonl').read_text().splitlines()]
    counts = {}
    for sample in client_samples:
        if sample['role'] != 'load-generator' or sample['pid'] != stage['harness_pid']:
            raise ValueError('client samples must contain only its own process')
        sample['host'] = args.client_host
    for sample in server_samples:
        role = sample['role']
        process = metadata['processes'][role]
        if sample['host'] != metadata['host'] or sample['pid'] != process['pid']:
            raise ValueError('server resource identity mismatch')
        if process['cpus'] != ('0' if role == 'sut' else '1') or int(process['start_identity']) <= 0:
            raise ValueError('invalid server process affinity/start identity')
    for sample in client_samples + server_samples:
        if (sample['run_id'] != stage['run_id'] or sample['stage_id'] != stage['stage']
                or sample['clock_ticks_per_second'] != 100 or sample['rss_kib'] <= 0):
            raise ValueError('invalid resource stage/clock/value')
        counts[sample['role']] = counts.get(sample['role'], 0) + 1
    if (set(counts) != {'sut', 'load-generator', 'fixture-1'}
            or any(v < 2 for v in counts.values())
            or {k: v for k, v in counts.items() if k != 'load-generator'} != metadata['counts']
            or stage['resource_sample_counts'] != {'load-generator': counts['load-generator']}):
        raise ValueError('incomplete or inconsistent resource coverage')
    output.mkdir(parents=True, exist_ok=True)
    if (output / 'stages.jsonl').exists() and any(json.loads(line)['stage'] == stage['stage'] for line in (output / 'stages.jsonl').read_text().splitlines()):
        raise ValueError('duplicate merged stage')
    stage['resource_sample_counts'] = counts
    stage['resource_sample_count'] = counts['sut']
    stage['client_host'] = args.client_host
    stage['server_resources'] = metadata
    with (output / 'stages.jsonl').open('a') as stream:
        stream.write(json.dumps(stage) + '\n')
    with (output / 'resource-samples.jsonl').open('a') as stream:
        for sample in client_samples + server_samples:
            stream.write(json.dumps(sample) + '\n')
    return 0


def hash_tree(args):
    root = Path(args.result)
    manifest = root / 'source-manifest.json'
    if manifest.exists():
        raise ValueError('source manifest already exists')
    files = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError('raw evidence symlink forbidden')
        if path.is_file():
            before = path.stat()
            files[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
            after = path.stat()
            if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
                raise ValueError('raw evidence changed during hashing')
    if not files:
        raise ValueError('empty raw evidence')
    manifest.write_text(json.dumps(dict(host=socket.gethostname(), files=files), indent=2) + '\n')
    return 0


def main():
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest='command', required=True)
    sampler = commands.add_parser('sample-server')
    sampler.add_argument('--sut-pid', type=int, required=True)
    sampler.add_argument('--sut-start', required=True)
    sampler.add_argument('--fixture-start', required=True)
    sampler.add_argument('--fixture-pid', type=int, action='append', default=[])
    sampler.add_argument('--sut-cpu', default='0')
    sampler.add_argument('--fixture-cpu', default='1')
    sampler.add_argument('--run-id', required=True)
    sampler.add_argument('--stage', required=True)
    sampler.add_argument('--result', required=True)
    sampler.add_argument('--proc-root', default='/proc')
    sampler.add_argument('--max-seconds', type=float, default=35)
    merge = commands.add_parser('merge-stage')
    for name in ('client', 'server', 'result', 'client-host'):
        merge.add_argument('--' + name, required=True)
    manifest = commands.add_parser('hash-tree')
    manifest.add_argument('--result', required=True)
    args = parser.parse_args()
    return {'sample-server': sample_server, 'merge-stage': merge_stage, 'hash-tree': hash_tree}[args.command](args)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, IndexError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
