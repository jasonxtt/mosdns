"""48h collector adapter. Pairing, replay and semantic comparison stay in run.py."""
import argparse
import collections
import hashlib
import io
import json
import os
import pathlib
import shutil
import subprocess
import threading
import time
import traceback
import urllib.request

from isolation_probe import counters
from longrun_store import HourStore, atomic
from run import Experiment, api, api_opener, project_audit, project_trace, resources
from safety import SafetyError
from stream_projection import project_json


def load(path, fallback=None):
    try:
        return json.loads(path.read_text())
    except FileNotFoundError:
        return fallback


def boot_id():
    return pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def trace_projection():
    pool = {}
    def project(entry):
        projected = project_trace([entry])
        if not projected:
            return None
        row = projected[0]
        # Trace IDs and step names repeat for every step in the same query.
        # The pool belongs to one bounded API batch, never a process-global
        # intern table; values and the complete step order stay unchanged.
        return {key: pool.setdefault(value, value) if isinstance(value, str) else value
                for key, value in row.items()}
    return project


def projected_api(base, path, trace=False):
    opener = api_opener()
    if trace:
        project = trace_projection()
    else:
        def project(entry):
            return project_audit({'logs':[entry]})[0]
    with opener.open(base + path, timeout=3) as response:
        with io.TextIOWrapper(response, encoding='utf-8') as stream:
            return project_json(stream, project, member=None if trace else 'logs')


class LongExperiment(Experiment):
    def __init__(self, args, root, manifest):
        self.run_root, self.manifest = root, manifest
        self.state_lock = threading.Lock()
        super().__init__(args)
        self.pairs_file.close()
        (self.root / 'paired.jsonl').unlink()
        self.store = HourStore(self.root, manifest['capture_start_wall'])
        state = load(root / 'safety-state.json', {})
        for name in ('started', 'not_ready_since', 'bad_total', 'rolling_bad'):
            if name in state:
                setattr(self.safety, name, state[name])
        if state.get('identity'):
            self.safety.identity = tuple(state['identity'])
        for name in ('pairs',):
            if name in state:
                setattr(self.safety, name, collections.deque(state[name]))
        # The systemd cgroup CPU counter can reset when a tool unit restarts.
        # A new segment therefore needs a fresh full 300s CPU window. The
        # SERVFAIL window, total, native identity and count baselines persist.
        self.state_counts.previous = load(root / 'learned-baseline.json', {})
        original_sample = self.state_counts.sample

        def count_sample(wall):
            try:
                result = original_sample(wall)
            except RuntimeError as error:
                self.trigger_safety(str(error))
                raise
            atomic(root / 'learned-baseline.json', result['counts'])
            return result
        self.state_counts.sample = count_sample

    def persist_safety(self):
        with self.state_lock:
            state = {name: getattr(self.safety, name) for name in
                     ('started', 'identity', 'not_ready_since', 'bad_total', 'rolling_bad')}
            state.update(cpu=list(self.safety.cpu), pairs=list(self.safety.pairs), wall=time.time())
            atomic(self.run_root / 'safety-state.json', state)

    def record(self, row):
        self.store.record(row, time.time())
        if row.get('rust_servfail_go_noerror'):
            self.persist_safety()

    def trigger_safety(self, reason):
        super().trigger_safety(reason)
        # This latch is shared by every tool segment and all event units.
        atomic(self.run_root / 'stop-status.json', {'wall': time.time(), 'kind': 'safety', 'reason': reason,
                                                   'segment': self.root.name, 'rust_left_running': True})

    def sample(self, phase):
        super().sample(phase)
        self.store.checkpoint()
        self.persist_safety()

    def audit_loop(self):
        good_since = None
        while not self.stop.is_set():
            try:
                self.emit('audit', projected_api(self.config['api'], '/api/v2/audit/logs?page=1&limit=500'))
                good = True
                for filename, expected in self.policy['files'].items():
                    path = pathlib.Path(filename)
                    digest = hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None
                    good = good and digest == expected
                for tag, expected in self.policy['switches'].items():
                    good = good and api(self.config['api'], f'/plugins/{tag}/show', raw=True) == expected
                if not good:
                    self.emit('policy_changed', None)
                    good_since = None
                else:
                    good_since = good_since or time.monotonic()
                    if not self.policy_valid and time.monotonic() - good_since >= 20:
                        self.emit('policy_restored', None)
                self.emit('trace', projected_api(self.config['api'], '/api/v1/capture/logs', trace=True))
                if time.monotonic() - self.trace_renewed >= 540:
                    api(self.config['api'], '/api/v1/capture/start', {'duration_seconds': 600}, raw=True)
                    self.trace_renewed = time.monotonic()
                    self.counts['trace_windows'] += 1
            except (OSError, ValueError, KeyError):
                self.counts['audit_read_errors'] += 1
                self.emit('trace_gap', None)
            self.stop.wait(1)

    def health_loop(self):
        last_success = time.monotonic()
        while not self.stop.is_set():
            try:
                props = dict(line.split('=', 1) for line in subprocess.check_output(['systemctl', 'show',
                    'mosdns.service', '-p', 'MainPID', '-p', 'NRestarts', '-p', 'ActiveState'], text=True).splitlines())
                try:
                    health = api(self.config['api'], '/api/v1/system/health')
                    ready = health.get('ready') is True
                except (OSError, ValueError):
                    health, ready = {}, False
                    self.counts['health_read_errors'] += 1
                now, wall = time.monotonic(), time.time()
                roles = {}
                for name, pid in [('collector', os.getpid()), ('worker', self.child.pid if self.child else 0),
                                  ('capture_ssh', self.capture.pid if self.capture else 0), ('rust', int(props['MainPID']))]:
                    if pid:
                        try:
                            roles[name] = resources(pid)
                        except (OSError, ValueError):
                            pass
                rss = sum(value['rss_kib'] or 0 for name, value in roles.items() if name != 'rust')
                cpu = int(dict(line.split() for line in self.cgroup_cpu_file.read_text().splitlines())['usage_usec']) / 1e6
                guard, free = counters(), shutil.disk_usage(self.run_root).free
                intent = load(self.run_root / 'restart-intent.json', {})
                planned = (intent.get('event') in (3, 4) and intent.get('issued', 0) <= wall <= intent.get('until', 0)
                           and 0 < intent.get('until', 0) - intent.get('issued', 0) <= 35
                           and (self.run_root / 'events' / f"{intent.get('event')}.claim").exists())
                if intent.get('after_pid'):
                    planned = planned and props['MainPID'] in ('0', intent['after_pid'])
                data = {'wall': wall, 'monotonic': now, 'service': props, 'ready': ready,
                        'version': health.get('version'), 'planned_restart': planned, 'guard': guard,
                        'tool_cpu_seconds': cpu, 'tool_rss_kib': rss, 'resources': roles, 'disk_free_bytes': free}
                event_rss = 0
                for number in range(1, 5):
                    event_group = pathlib.Path(f'/sys/fs/cgroup/system.slice/shadow1-48h-event{number}.service')
                    if (event_group / 'cgroup.procs').exists():
                        for pid in (event_group / 'cgroup.procs').read_text().split():
                            try:
                                event_rss += resources(int(pid))['rss_kib'] or 0
                            except (OSError, ValueError):
                                pass
                data['event_actuator_rss_kib'] = event_rss
                if int(props['MainPID']):
                    group = next(line[3:].lstrip('/') for line in
                                 (pathlib.Path('/proc') / props['MainPID'] / 'cgroup').read_text().splitlines()
                                 if line.startswith('0::'))
                    group_root = pathlib.Path('/sys/fs/cgroup') / group
                    native = []
                    for pid in (group_root / 'cgroup.procs').read_text().split():
                        try:
                            native.append(resources(int(pid)))
                        except (OSError, ValueError):
                            pass
                    data['native_processes'] = native
                    data['native_tree_rss_kib'] = sum(p['rss_kib'] or 0 for p in native)
                    data['native_cgroup_cpu_seconds'] = int(dict(line.split() for line in
                        (group_root / 'cpu.stat').read_text().splitlines())['usage_usec']) / 1e6
                try:
                    self.safety.health(now, props, ready, planned=planned)
                    data['tool_cpu_5m_percent'] = self.safety.resources(now, cpu, rss, guard)
                    if free < 500_000_000:
                        raise SafetyError('disk free below 500 MB')
                except SafetyError as error:
                    data['safety_stop'] = str(error)
                    self.trigger_safety(str(error))
                with (self.root / 'health.jsonl').open('a') as stream:
                    stream.write(json.dumps(data) + '\n')
                self.persist_safety()
                last_success = now
            except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
                self.counts['health_sampler_errors'] += 1
                if time.monotonic() - last_success >= 30:
                    self.trigger_safety('health/safety sampler unavailable for 30 seconds: ' + type(error).__name__)
            self.stop.wait(5)


def main(root):
    os.umask(0o077)
    manifest = load(root / 'run.json')
    if manifest['boot_id'] != boot_id():
        raise RuntimeError('reboot ended the run; automatic replay resume is forbidden')
    if (root / 'stop-status.json').exists():
        return
    remaining = manifest['end_wall'] - time.time()
    if remaining <= 0:
        return
    segments = root / 'segments'
    segments.mkdir(exist_ok=True)
    number = 1 + max([int(p.name) for p in segments.iterdir() if p.is_dir()] or [0])
    segment = segments / f'{number:03d}'
    segment.mkdir(mode=0o700)
    marker = {'segment': number, 'kind': 'initial' if number == 1 else 'tool_restart',
              'started_wall': time.time(), 'boot_id': boot_id(), 'pid': os.getpid()}
    atomic(segment / 'segment.json', marker)
    idle = max(0, manifest['capture_start_wall'] - time.time())
    args = argparse.Namespace(root=segment, config=root / 'runtime.json', network=root / 'network',
                             seconds=max(1, remaining - idle), idle_seconds=idle, qps=100)
    experiment = None
    try:
        experiment = LongExperiment(args, root, manifest)
        experiment.run()
        marker['exit'] = 'safety' if experiment.safety_stop_reason else 'normal_or_signal'
    except BaseException as error:
        marker.update(exit='tool_failure', error=type(error).__name__)
        (segment / 'error.txt').write_text(traceback.format_exc())
        raise
    finally:
        if experiment:
            experiment.store.close()
            experiment.persist_safety()
        marker['ended_wall'] = time.time()
        atomic(segment / 'segment.json', marker)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=pathlib.Path)
    main(parser.parse_args().root)
