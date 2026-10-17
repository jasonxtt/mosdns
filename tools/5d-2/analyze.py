"""Pure analysis of a 5D-2 run: windows, recovery, resources, mock counters.

Everything here works on the rows/samples fload.py writes, so it is unit-tested
on synthetic data. Times are CLOCK_MONOTONIC seconds relative to the run start.
"""
import json
import math
import pathlib

NORMAL_DEADLINE_S = 0.5  # a "healthy" answer: correct and within 500 ms of its planned time


def quantile(values, fraction):
    values = sorted(values)
    if not values:
        return None
    return round(values[min(len(values) - 1, math.ceil(len(values) * fraction) - 1)], 6)


def load_rows(path, start):
    rows = [json.loads(line) for line in pathlib.Path(path).read_text().splitlines()]
    for row in rows:
        row['planned'] -= start
        for key in ('sent', 'finished'):
            if row[key] is not None:
                row[key] -= start
    return rows


def healthy(row):
    return (row['status'] in ('correct', 'fallback') and row['finished'] is not None
            and row['finished'] - row['planned'] <= NORMAL_DEADLINE_S)


def window(rows, lo, hi, groups=None):
    """Counts and latency of the rows PLANNED in [lo, hi) (optionally one group set)."""
    chosen = [r for r in rows if lo <= r['planned'] < hi and (groups is None or r['group'] in groups)]
    counts = {}
    for row in chosen:
        counts[row['status']] = counts.get(row['status'], 0) + 1
    answered = [(r['finished'] - r['planned']) * 1000 for r in chosen if r['finished'] is not None]
    good = [(r['finished'] - r['planned']) * 1000 for r in chosen if r['status'] in ('correct', 'fallback')]
    sent = [r for r in chosen if r['status'] not in ('unsent', 'send_error')]
    return dict(planned=len(chosen), sent=len(sent), statuses=counts, healthy=sum(healthy(r) for r in chosen),
                healthy_rate=(sum(healthy(r) for r in chosen) / len(chosen)) if chosen else None,
                # The sender's own misses (`unsent`) are reported as coverage, not blamed on the host.
                healthy_rate_of_sent=(sum(healthy(r) for r in sent) / len(sent)) if sent else None,
                send_coverage=(len(sent) / len(chosen)) if chosen else None,
                # Any answer (right or not) within the normal deadline: the timing view of a window.
                answered_on_time_of_sent=(sum(r['finished'] is not None and r['finished'] - r['planned'] <= NORMAL_DEADLINE_S for r in sent) / len(sent)) if sent else None,
                correct_latency_ms={k: quantile(good, f) for k, f in (('p50', .5), ('p95', .95), ('p99', .99))} | {'max': round(max(good), 3) if good else None},
                answer_latency_ms={k: quantile(answered, f) for k, f in (('p50', .5), ('p99', .99))} | {'max': round(max(answered), 3) if answered else None})


def recovery_seconds(rows, clear_at, groups, bins=5, limit=120):
    """Seconds from `clear_at` until `bins` consecutive one-second bins (by planned
    time, starting at or after the clear) are entirely healthy; None if never."""
    first = math.ceil(clear_at)
    candidates = []
    for second in range(first, int(limit) + first):
        members = [r for r in rows if second <= r['planned'] < second + 1 and r['group'] in groups
                   and r['status'] not in ('unsent', 'send_error')]
        if not members:
            break
        candidates.append(all(healthy(r) for r in members))
    for index in range(len(candidates) - bins + 1):
        if all(candidates[index:index + bins]):
            return first + index + bins - clear_at
    return None


def resources(samples, start, lo, hi):
    """First/last/peak of the SUT's RSS (KiB), FDs, threads and established TCP in [lo, hi)."""
    chosen = [s for s in samples if lo <= s['time'] - start < hi and not s['processes']['sut'].get('missing')]
    if not chosen:
        return None
    sut = [s['processes']['sut'] for s in chosen]
    established = [sum(v for k, v in s['connections'].items() if k.startswith('01:')) for s in chosen if 'connections' in s] or [0]
    return dict(rss_kib=dict(first=sut[0]['rss_kib'], last=sut[-1]['rss_kib'], peak=max(x['rss_kib'] for x in sut)),
                fd=dict(first=sut[0]['fd'], last=sut[-1]['fd'], peak=max(x['fd'] for x in sut)),
                threads=dict(first=sut[0]['threads'], last=sut[-1]['threads'], peak=max(x['threads'] for x in sut)),
                established=dict(first=established[0], last=established[-1], peak=max(established)),
                samples=len(chosen))


def udp_delta(samples, start, lo, hi):
    """Kernel UDP drops between two times: `Drops53` is the host's DNS socket(s)
    alone, `RcvbufErrors`/`InErrors` count the whole namespace."""
    points = [(s['time'] - start, s['udp']) for s in samples if s.get('udp')]
    if len(points) < 2:
        return None

    def at(moment, default):
        value = default
        for t, counters in points:
            if t <= moment:
                value = counters
        return value
    first, last = at(lo, points[0][1]), at(hi, points[-1][1])
    return {k: last[k] - first[k] for k in ('Drops53', 'RcvbufErrors', 'InErrors', 'InDatagrams') if k in last and k in first}


def counter_delta(stats_path, start, lo, hi):
    """Mock counters accumulated between two times (relative to run start)."""
    snapshots = []
    path = pathlib.Path(stats_path)
    if path.exists():
        for line in path.read_text().splitlines():
            value = json.loads(line)
            snapshots.append((value['t'] - start, value['counters']))
    def at(moment):
        best = {}
        for t, counters in snapshots:
            if t <= moment:
                best = counters
        return best
    first, last = at(lo), at(hi)
    return {k: last.get(k, 0) - first.get(k, 0) for k in last if last.get(k, 0) != first.get(k, 0)}


def injected(table, window, baseline):
    """Did the MOCK itself see each part of a (normalized) fault table take effect in `window`?
    `window`/`baseline` are mock counter deltas. Modes that answer wrongly or late leave a
    `<group>:<protocol>:fault:<mode>` counter; a refused listener or a SOCKS5 that is down
    leaves silence where the baseline had traffic. Returns {part: True/False, or None when
    silence cannot be judged because the baseline had no traffic}."""
    def traffic(counters, group, protocols):
        return sum(v for k, v in counters.items() if k.startswith(group + ':') and k.split(':')[1] in protocols
                   and k.endswith((':accept', ':query')))

    seen = {}
    for group, spec in table.get('groups', {}).items():
        mode = spec['mode']
        protocols = spec.get('protocols') or ['udp', 'tcp', 'tls', 'https']
        if mode == 'refuse':
            # None: the baseline had no new connections/queries (a host that reuses persistent
            # connections), so silence proves nothing.
            seen[f'{group}:{mode}'] = (traffic(window, group, protocols) == 0) if traffic(baseline, group, protocols) > 0 else None
        elif mode == 'handshake_reset':
            seen[f'{group}:{mode}'] = window.get('handshake_reset', 0) > 0
        else:
            seen[f'{group}:{mode}'] = any(k.startswith(group + ':') and k.endswith(':fault:' + mode) for k in window)
    socks = table.get('socks')
    if socks:
        mode = socks['mode']
        if mode == 'down':
            seen['socks:down'] = (window.get('socks:accept', 0) == 0) if baseline.get('socks:accept', 0) > 0 else None
        elif mode == 'blackhole':
            seen['socks:blackhole'] = window.get('socks:fault:blackhole', 0) > 0
        else:
            seen['socks:' + mode] = window.get('socks:fault:' + mode, 0) > 0
    return seen


def journal_events(path, start, lo, hi, kind=None):
    """Mock journal lines in [lo, hi) (relative seconds), optionally of one kind."""
    result = []
    with pathlib.Path(path).open() as handle:
        for line in handle:
            event = json.loads(line)
            t = event['monotonic_s'] - start
            if lo <= t < hi and (kind is None or event['kind'] == kind):
                result.append(event)
    return result
