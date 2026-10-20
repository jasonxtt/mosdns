"""Numerical 48h report, exact outcome counts and bounded-memory latency bins."""
import collections
import gzip
import json
import math
import pathlib
import struct
import tarfile
import time

from longrun_store import atomic
from report import distribution, trend


def json_rows(path):
    if not path.exists():
        return
    with path.open() as stream:
        for line in stream:
            if line.strip():
                yield json.loads(line)


def binary_rows(folder, name, fmt):
    raw = folder / name
    compressed = folder / (name + '.gz')
    path = raw if raw.exists() else compressed
    if not path.exists():
        return
    size = struct.calcsize(fmt)
    opener = gzip.open if path.suffix == '.gz' else open
    with opener(path, 'rb') as stream:
        while True:
            chunk = stream.read(size)
            if not chunk:
                break
            if len(chunk) != size:
                raise RuntimeError('incomplete numeric frame: ' + str(path))
            yield struct.unpack(fmt, chunk)


def latency_summary(hist):
    count = sum(hist.values())
    if not count:
        return {'samples': 0}
    ranks = {f'p{p}': math.ceil(count * p / 100) for p in (50, 95, 99)}
    result, total = {'samples': count, 'quantization_error_ms_at_most': .0005}, 0
    for value, n in sorted(hist.items()):
        total += n
        for name, rank in ranks.items():
            if total >= rank and name not in result:
                result[name] = value / 1000
    result.update(min=min(hist) / 1000, max=max(hist) / 1000)
    return result


def window_stats(folders, start, end):
    classifications, differences, latency = collections.Counter(), collections.Counter(), collections.Counter()
    names = ['semantic_equal', 'explained:learned-state', 'unexplained', 'incomplete']
    fields = ('rcode', 'has_answer', 'tc', 'types', 'class', 'address_kinds', 'cname_terminals', 'service_targets')
    for folder in folders:
        for wall, mask, code in binary_rows(folder, 'outcomes.bin', '!dHB'):
            if start <= wall < end:
                classifications[names[code]] += 1
                if mask:
                    differences['+'.join(sorted(f for index, f in enumerate(fields) if mask & (1 << index)))] += 1
        for wall, value in binary_rows(folder, 'latencies.bin', '!dd'):
            if start <= wall < end:
                latency[round(value * 1000)] += 1
    return {'classifications': dict(classifications), 'differences': dict(differences),
            'latency_ms': latency_summary(latency)}


def generate(root):
    manifest = json.loads((root / 'run.json').read_text())
    status = json.loads((root / 'stop-status.json').read_text()) if (root / 'stop-status.json').exists() else None
    segments = sorted((root / 'segments').glob('[0-9][0-9][0-9]'))
    folders = [folder for segment in segments for folder in sorted((segment / 'hours').glob('*'))]
    hours, counts = {}, collections.Counter()
    markers, resources, health, learned = [], [], [], []
    for segment in segments:
        marker = json.loads((segment / 'segment.json').read_text())
        if 'ended_wall' not in marker:
            marker['abrupt_end_or_checkpoint_gap'] = True
        markers.append(marker)
        if (segment / 'summary.json').exists():
            counts.update(json.loads((segment / 'summary.json').read_text())['counts'])
        resources.extend(json_rows(segment / 'resources.jsonl'))
        health.extend(json_rows(segment / 'health.jsonl'))
        learned.extend(json_rows(segment / 'learned-counts.jsonl'))
    for folder in folders:
        stats = json.loads((folder / 'stats.json').read_text())
        hours.setdefault(stats['hour'], collections.Counter()).update(stats['metrics'])
    series = [{'hour': hour, 'start_wall': manifest['capture_start_wall'] + hour * 3600,
               'metrics': dict(metrics)} for hour, metrics in sorted(hours.items())]
    measured = [r for r in health if manifest['capture_start_wall'] <= r['wall'] <= manifest['end_wall'] + 20]
    rss = [r['native_tree_rss_kib'] / 1024 for r in measured if 'native_tree_rss_kib' in r]
    slope_rows = [{'monotonic': r['monotonic'], 'rss_kib': r['native_tree_rss_kib']} for r in measured if 'native_tree_rss_kib' in r]
    cpu = []
    for previous, current in zip(measured, measured[1:]):
        a, b = previous.get('native_cgroup_cpu_seconds'), current.get('native_cgroup_cpu_seconds')
        elapsed = current['monotonic'] - previous['monotonic']
        if a is not None and b is not None and b >= a and 0 < elapsed <= 15:
            cpu.append((b - a) * 100 / elapsed)
    start = manifest['capture_start_wall']
    end = min(manifest['end_wall'] + 20, time.time(), status['wall'] if status and status['kind'] != 'completed' else math.inf)
    result = {'generated_wall': time.time(), 'planned_start_wall': start, 'planned_end_wall': manifest['end_wall'],
              'observed_end_wall': end, 'elapsed_wall_seconds': max(0, end - start), 'stop_status': status,
              'segments': markers, 'counts': dict(counts), 'hourly': series,
              'retention': 'full DNS wire/audit/trace only for non-equal rows; numerical outcomes and exact latencies for all rows',
              'guard_max': [max((r['guard'][i] for r in health), default=0) for i in range(3)],
              'health_samples': len(health), 'not_ready_samples': sum(not r['ready'] for r in health),
              'max_health_gap_seconds': max((b['wall'] - a['wall'] for a,b in zip(health,health[1:])), default=None),
              'disk_min_free_bytes': min((r['disk_free_bytes'] for r in health), default=None),
              'rust_tree_rss_mib': distribution(rss), 'rust_rss_slope_mib_per_hour': trend(slope_rows),
              'rust_cgroup_cpu_percent': distribution(cpu),
              'tool_rss_mib': distribution([r['tool_rss_kib'] / 1024 for r in health]),
              'tool_cpu_rolling_5m_percent': distribution([r['tool_cpu_5m_percent'] for r in health if r.get('tool_cpu_5m_percent') is not None]),
              'learned_count_samples': len(learned), 'learned_count_any_decrease': any(r['decreased'] for r in learned),
              'go_reference': {'state': 'Mac launchd samples are independent; merge coverage and RSS/CPU during Phase B'}}
    result['paired_rate'] = counts['paired'] / counts['replayed'] if counts['replayed'] else None
    result['replay_qps_over_elapsed_wall'] = counts['replayed'] / (end-start) if end > start else None
    result['tool_restart_count'] = sum(marker.get('kind') == 'tool_restart' for marker in markers)
    result['uncheckpointed_tool_failure_segments'] = [m['segment'] for m in markers
        if m.get('exit') == 'tool_failure' or m.get('abrupt_end_or_checkpoint_gap')]
    result['event_actuator_rss_mib'] = distribution([r.get('event_actuator_rss_kib',0)/1024 for r in health])
    result.update(window_stats(folders, start, end + .001))
    result['thresholds'] = {'steady_limit_mib': 128, 'peak_limit_mib': 256,
        'rss_p95_le_128': sorted(rss)[math.ceil(len(rss)*.95)-1] <= 128 if rss else None,
        'all_observed_rss_le_128': max(rss) <= 128 if rss else None,
        'peak_le_256': max(rss) <= 256 if rss else None, 'steady_not_above_go': 'pending Mac reference merge'}
    effects = []
    for path in sorted((root / 'events').glob('[1-4].json')):
        event = json.loads(path.read_text())
        window = event['effect_window']
        post = [r for r in measured if window['start_wall'] <= r['wall'] < window['end_wall']]
        item = {key: event[key] for key in ('event','started_wall','ended_wall','success','effect_window',
                'error','skipped','confirmation_seconds','apply_http_seconds','first_candidate_ready_seconds',
                'planned_restarts','temporary_environment_removed','release_server_stopped','installed_sha256') if key in event}
        item['http_results'] = {key: value['http_status'] for key, value in event.items()
                                if isinstance(value,dict) and 'http_status' in value}
        item['post_10m'] = window_stats(folders, window['start_wall'], window['end_wall'])
        item['post_10m'].update(health_samples=len(post), not_ready_samples=sum(not r['ready'] for r in post),
             rust_rss_mib=distribution([r['native_tree_rss_kib']/1024 for r in post if 'native_tree_rss_kib' in r]),
             tool_rss_mib=distribution([r['tool_rss_kib']/1024 for r in post]))
        during = [r for r in measured if event['started_wall'] <= r['wall'] <= event['ended_wall']]
        item['during_operation'] = {'health_samples':len(during), 'not_ready_samples':sum(not r['ready'] for r in during),
            'rust_rss_mib':distribution([r['native_tree_rss_kib']/1024 for r in during if 'native_tree_rss_kib' in r]),
            'event_actuator_rss_mib':distribution([r.get('event_actuator_rss_kib',0)/1024 for r in during]),
            **window_stats(folders,event['started_wall'],event['ended_wall'])}
        effects.append(item)
    result['event_effects'] = effects
    atomic(root / 'hourly-series.json', series)
    atomic(root / 'event-effects.json', effects)
    atomic(root / 'final-summary.json', result)
    with tarfile.open(root / 'numeric-evidence.tar.gz', 'w:gz') as archive:
        for name in ('run.json','stop-status.json','hourly-series.json','event-effects.json','final-summary.json'):
            if (root/name).exists(): archive.add(root/name, arcname=name)
        for segment in segments:
            for name in ('segment.json','summary.json','health.jsonl','resources.jsonl','learned-counts.jsonl','tool-hashes.json'):
                if (segment/name).exists(): archive.add(segment/name, arcname=str(segment.relative_to(root)/name))
    return result
