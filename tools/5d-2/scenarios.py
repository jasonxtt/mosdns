"""5D-2 scenario definitions (F faults, H hot updates), their runner and checks.

A scenario is a plain dict: traffic (unique-name group streams routed through
reserved suffixes, or the weighted production corpus), a timeline of events
(fault injection / management requests), analysis windows and a `checks`
function. `run_scenario` starts one isolated host, runs the sender, analyzes
and writes result.json. Private names never leave the run directory.
"""
import json
import math
import pathlib
import shutil
import subprocess
import sys
import urllib.parse

import yaml

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / 'e2e-1'))
sys.path.insert(0, str(HERE.parent / '5d-1'))
import analyze  # noqa: E402
from dnswire import answer, decode, question  # noqa: E402
from harness import Env  # noqa: E402
from load import semantic  # noqa: E402
from mock import GROUPS, MARKERS, validate_faults  # noqa: E402

SUFFIX = '5d2-{}.example.com'
DEFAULT_STREAMS = list(GROUPS)
FB_STREAMS = ['domestic', 'foreign', 'foreignecs', 'cnfake', 'fb']
DOWNLOAD_NAME = 'geosite_cn'  # an sd_set_light provider and its source name


# --------------------------------------------------------------------- inputs


def reserved_routes(root):
    """Routes `*.5d2-<group>.example.com` straight to a group (through one cache) and
    `*.5d2-fb.example.com` to the production `prefer_foreign_nocache` fallback."""
    path = root / 'config_custom.yaml'
    config = yaml.safe_load(path.read_text())
    # First in the file: the Go host needs a plugin defined before the sequence that uses it.
    config['plugins'].insert(0, dict(tag='cache_5d2', type='cache', args=dict(size=200000)))
    entry = next(p for p in config['plugins'] if p['tag'] == 'sequence_6666')
    position = next(i for i, step in enumerate(entry['args']) if step.get('exec') == '$top_domains') + 1
    steps = [dict(matches='qname domain:' + SUFFIX.format(g), exec=['$cache_5d2', '$' + g, 'exit']) for g in GROUPS]
    steps.append(dict(matches='qname domain:' + SUFFIX.format('fb'), exec=['$prefer_foreign_nocache', 'exit']))
    entry['args'][position:position] = steps
    path.write_text(yaml.safe_dump(config, allow_unicode=True, sort_keys=False))


def local_downloads(root):
    """Points every rule source at the mock file server and serves the current files."""
    downloads = root / 'downloads'
    downloads.mkdir(exist_ok=True)
    for manifest in (root / 'srs').glob('*.json'):
        entries = json.loads(manifest.read_text())
        changed = False
        for entry in entries:
            source = root / entry.get('files', '')
            if entry.get('url') and source.is_file():
                shutil.copy2(source, downloads / source.name)
                entry['url'] = 'http://127.0.0.1:18080/' + urllib.parse.quote(source.name)
                changed = True
        if changed:
            manifest.write_text(json.dumps(entries, ensure_ascii=False, indent=2))


def fast_requery(root):
    """The shipped requery runs 10 qps (about 17 minutes for the snapshot's names);
    the scenario copy runs 200 qps so the task completes inside the observation."""
    path = root / 'webinfo/requeryconfig.json'
    config = json.loads(path.read_text())
    config['execution_settings']['queries_per_second'] = 200
    path.write_text(json.dumps(config, ensure_ascii=False, indent=2))


def stream_semantic(group, name, qtype):
    key = 'foreign' if group == 'fb' else group
    return semantic(decode(answer(question(name, qtype, 0), *MARKERS[key])))


def stream_corpus(streams, rate, duration, revisit=None):
    """Unique-name rows interleaved across `streams`, 20 qps each at 100 qps. A row's
    planned time is index/rate. `revisit` re-asks names first asked in a fault
    window: dict(groups, source=(lo, hi), window=(lo, hi))."""
    count = round(rate * duration)
    rows, oracle, issued, serial = [], [], {g: [] for g in streams}, {g: 0 for g in streams}
    for i in range(count):
        group = streams[i % len(streams)]
        t = i / rate
        name = qtype = None
        if revisit and group in revisit['groups'] and revisit['window'][0] <= t < revisit['window'][1]:
            pool = [r for r in issued[group] if revisit['source'][0] <= r[2] < revisit['source'][1]]
            if pool:
                # Spread the revisits evenly over the whole pool so every fault segment is covered.
                slots = (revisit['window'][1] - revisit['window'][0]) * rate / len(streams)
                name, qtype, _ = pool[int(serial[group] * len(pool) / slots) % len(pool)]
                serial[group] += 1
        if name is None:
            k = len(issued[group])
            name, qtype = f'u{k:06d}.{SUFFIX.format(group)}', 1 if k % 10 < 7 else 28
        issued[group].append((name, qtype, t))
        rows.append(dict(name=name, type=qtype, group=group, weight=1))
        entry = dict(index=i, semantic=stream_semantic(group, name, qtype))
        if group == 'fb':
            # The configured fallback may legitimately answer from `domestic`.
            entry['alt'] = [stream_semantic('domestic', name, qtype)]
        oracle.append(entry)
    return rows, oracle


# ------------------------------------------------------------------ timelines


def fault_event(at, table):
    return dict(at=at, action='faults', table=table)


def group_fault(groups, mode, protocols=None, **extra):
    spec = dict(mode=mode, **extra)
    if protocols:
        spec['protocols'] = protocols
    return {'groups': {g: dict(spec) for g in groups}}


def f_standard(name, title, table, affected, controls=None, streams=None, duration=75, revisit=None,
               extra_events=None, deadline_s=12, checks=None, notes=''):
    """Fault at 15 s, cleared at 45 s, observation to the end (duration)."""
    return dict(name=name, title=title, kind='F', rate=100, duration=duration, deadline_s=deadline_s,
                streams=streams or DEFAULT_STREAMS, revisit=revisit, prepare=[reserved_routes],
                fault_at=15, clear_at=45, affected=affected, controls=controls,
                events=[fault_event(15, table), fault_event(45, {})] + (extra_events or []),
                checks=checks or [], notes=notes)


def build_scenarios():
    s = {}
    s['F1'] = f_standard('F1', 'domestic group completely silent', group_fault(['domestic'], 'blackhole'),
                         affected=['domestic'])
    for mode, label in (('refuse', 'F2a'), ('reset', 'F2b'), ('handshake_reset', 'F2c')):
        s[label] = f_standard(label, f'foreign DoH upstream: {mode}',
                              group_fault(['foreign', 'foreignecs'], mode, ['https']),
                              affected=['foreign', 'foreignecs'], checks=['pool_rebuild'])
    revisit = dict(groups=['domestic', 'foreign'], source=(15, 55), window=(65, 80))
    s['F3'] = dict(f_standard('F3', 'wrong / truncated / mismatched upstream answers', {}, affected=['domestic', 'foreign'],
                              duration=85, revisit=revisit, checks=['no_cache_pollution']),
                   events=[fault_event(15, group_fault(['domestic', 'foreign'], 'bad_id')),
                           fault_event(25, group_fault(['domestic', 'foreign'], 'truncated')),
                           fault_event(35, group_fault(['domestic', 'foreign'], 'wrong_question')),
                           fault_event(45, group_fault(['domestic', 'foreign'], 'servfail')),
                           fault_event(55, {})], clear_at=55)
    s['F4a'] = f_standard('F4a', 'SOCKS5 proxy down', {'socks': {'mode': 'down'}}, affected=['foreign', 'foreignecs'],
                          checks=['no_silent_direct'])
    s['F4b'] = f_standard('F4b', 'SOCKS5 proxy accepts but never answers', {'socks': {'mode': 'blackhole'}},
                          affected=['foreign', 'foreignecs'], checks=['no_silent_direct'])
    s['F5'] = f_standard('F5', 'slow foreign upstream (2 s) with the configured fallback',
                         group_fault(['foreign'], 'slow', ['https'], delay_ms=2000), affected=['foreign', 'fb'],
                         streams=FB_STREAMS, checks=['fallback_behavior'])
    s['F6'] = dict(name='F6', title='rule source download failures', kind='F', rate=100, duration=190, deadline_s=12,
                   streams=None, prepare=[local_downloads], fault_at=None, clear_at=None, affected=[], controls=None,
                   events=[], checks=['download_failures'], notes='weighted production corpus at 100 qps')
    # H: 500 qps weighted production corpus, two operations per run.
    base = dict(kind='H', rate=500, duration=75, deadline_s=5, streams=None, affected=[], controls=None,
                fault_at=None, clear_at=None, checks=['hot_update'])
    s['H1'] = dict(base, name='H1', title='settings API: upstream list and proxy', prepare=[reserved_routes],
                   ops=[25, 50])
    s['H2'] = dict(base, name='H2', title='rule source refresh rebuilds the mapper', prepare=[local_downloads],
                   ops=[25, 50])
    s['H3'] = dict(base, name='H3', title='special group save (with a dedicated port)', prepare=[], ops=[25, 50])
    s['H4'] = dict(base, name='H4', title='requery task', prepare=[fast_requery], ops=[25], duration=110,
                   wrong_allowed='the requery task runs its configured flush_rules (caches and the remembered '
                                 'name lists), so routing legitimately differs from the pre-task oracle until the '
                                 'lists are learned again; checked against Go and by timing (all differences start '
                                 'after the flush)')
    return s


def f6_events():
    events, at = [], 10
    for mode, spec in (('status 500', dict(mode='status', code=500)), ('status 503', dict(mode='status', code=503)),
                       ('oversize', dict(mode='oversize', bytes=70 * 1024 * 1024)), ('short', dict(mode='short')),
                       ('stall', dict(mode='stall')), ('ok', dict(mode='ok'))):
        events.append(fault_event(at, {'download': spec}))
        events.append(dict(at=at + 0.5, action='http', method='POST', path=f'/plugins/{DOWNLOAD_NAME}/update/{DOWNLOAD_NAME}',
                           label='download ' + mode))
        at += 70 if mode == 'stall' else 15
    return events


# ---------------------------------------------------------------- orchestration


def h_events(env, name, ops):
    """Management requests for the H scenarios (built after the host is up)."""
    events = []
    api = lambda method, path, body=None: dict(action='http', method=method, path=path, body=body, timeout=120)
    if name == 'H1':
        status, text, _ = env.api('GET', '/api/v1/upstream/config')
        original = json.loads(text)['foreign']
        changed = [dict(tag='google', enabled=True, protocol='tcp', addr='127.0.0.1:15010', use_socks_proxy=True)]
        events += [dict(at=ops[0], label='proxy to second SOCKS5 listener', **api('POST', '/api/v1/overrides', dict(socks5='127.0.0.1:17892', ecs='', replacements=[]))),
                   dict(at=ops[0] + 3, label='foreign list to plain TCP', **api('POST', '/api/v1/upstream/config', dict(plugin_tag='foreign', upstreams=changed))),
                   dict(at=ops[1], label='proxy back', **api('POST', '/api/v1/overrides', dict(socks5='127.0.0.1:17891', ecs='', replacements=[]))),
                   dict(at=ops[1] + 3, label='foreign list back', **api('POST', '/api/v1/upstream/config', dict(plugin_tag='foreign', upstreams=original)))]
        # Unique foreign names (reserved route, uncached) show which proxy and list a new query uses.
        for label, at in (('probe before', ops[0] - 4), ('probe changed', ops[0] + 8), ('probe changed late', ops[1] - 4),
                          ('probe reverted', ops[1] + 8)):
            events.append(dict(at=at, action='probe', port=53, name=f'{label.replace(" ", "-")}.{SUFFIX.format("foreign")}',
                               timeout=8, label=label))
    elif name == 'H2':
        for at in ops:
            events.append(dict(at=at, label='rule source update', **api('POST', f'/plugins/{DOWNLOAD_NAME}/update/{DOWNLOAD_NAME}')))
    elif name == 'H3':
        group = dict(name='h3', listen_port=3999, custom_port_only=True, upstream_sources=[dict(kind='group', plugin_tag='foreign')])
        status, text, _ = env.api('GET', '/api/v1/special-groups')
        if status != 200 or json.loads(text):
            raise RuntimeError('H3 expects no existing special groups')
        for at in ops:
            events += [dict(at=at, label='create group', **api('POST', '/api/v1/special-groups', group)),
                       dict(at=at + 4, action='probe', port=3999, name='h3-probe.example.com', label='probe dedicated port'),
                       dict(at=at + 8, label='delete group', **api('DELETE', '/api/v1/special-groups/50')),
                       dict(at=at + 12, action='probe', port=3999, name='h3-probe.example.com', timeout=2, label='probe after delete')]
    elif name == 'H4':
        events += [dict(at=ops[0], label='requery trigger', **api('POST', '/plugins/requery/trigger')),
                   dict(at=ops[0] + 1, action='poll', path='/plugins/requery/status', until='"task_state":"idle"', interval=2,
                        timeout=80, method='GET', label='requery status')]
    return events


def run_scenario(spec, work, role, binary, output, seed=501):
    """One isolated run of `spec`; returns the analysis dict (also saved)."""
    output = pathlib.Path(output)

    def prepare(root):
        for step in spec['prepare']:
            step(root)
        if role == 'go' and spec['name'] == 'H4':
            # The shipped configuration names the API port 9099 (requery URLs, `domain_set_url`),
            # but the Go host listens on the port in webui_port_settings.json (80 in the snapshot),
            # so its own learning pushes and flushes would reach nothing. For this scenario's Go
            # reference the disposable copy uses 9099, which is what the native host serves.
            path = root / 'webinfo/webui_port_settings.json'
            path.write_text(json.dumps(dict(json.loads(path.read_text()), port=9099)))

    with Env(work, role, binary, output, prepare_root=prepare, seed=seed) as env:
        if spec['kind'] == 'F' and spec['streams']:
            rows, oracle = stream_corpus(spec['streams'], spec['rate'], spec['duration'], spec.get('revisit'))
            mode = 'sequential'
            events = list(spec['events'])
        else:
            rows = json.loads((work / 'weighted-corpus.json').read_text())
            oracle = json.loads((work / ('native-oracle.json' if role == 'native' else 'oracle.json')).read_text())
            mode = 'weighted'
            events = f6_events() if spec['name'] == 'F6' else h_events(env, spec['name'], spec['ops'])
        (output / 'corpus.json').write_text(json.dumps(rows))
        (output / 'oracle.json').write_text(json.dumps(oracle))
        job = dict(corpus=str(output / 'corpus.json'), oracle=str(output / 'oracle.json'), output=str(output / 'load'),
                   root=str(env.root), rate=spec['rate'], duration=spec['duration'], deadline_s=spec['deadline_s'],
                   mode=mode, seed=seed, pid=env.host.pid, mock_pid=env.mock.pid, api_port=env.api_port,
                   events=events, start_file=str(output / 'start.json'))
        (output / 'job.json').write_text(json.dumps(job))
        sender = env.spawn(1, [sys.executable, HERE / 'fload.py', output / 'job.json'], 'sender.log')
        if sender.wait() != 0:
            raise RuntimeError('sender failed; inspect sender.log')
        if env.host.poll() is not None:
            raise RuntimeError('host exited during the scenario')
        log = env.root / 'tmp/mosdns.log'
        (output / 'host-log-tail.txt').write_text(log.read_text(errors='replace')[-400000:] if log.exists() else '')
    result = analysis(spec, role, output, events)
    (output / 'result.json').write_text(json.dumps(result, indent=2))
    return result


# --------------------------------------------------------------------- analysis


def analysis(spec, role, output, events):
    summary = json.loads((output / 'load/summary.json').read_text())
    start = summary['start']
    rows = analyze.load_rows(output / 'load/requests.jsonl', start)
    samples = json.loads((output / 'load/resources.json').read_text())
    recorded = json.loads((output / 'load/events.json').read_text())
    duration = spec['duration']
    result = dict(scenario=spec['name'], title=spec['title'], role=role, rate=spec['rate'], duration=duration,
                  rows=len(rows), orphan_packets=summary['orphan_packets'], notes=spec.get('notes', ''))
    statuses = {}
    for row in rows:
        statuses[row['status']] = statuses.get(row['status'], 0) + 1
    result['statuses'] = statuses
    result['udp_drops_whole_run'] = analyze.udp_delta(samples, start, 0, duration + spec['deadline_s'] + 1)
    result['wrong_samples'] = [dict(group=r['group'], answer=r.get('answer')) for r in rows if r['status'] == 'wrong'][:5]
    groups = sorted({r['group'] for r in rows})
    result['events'] = [dict(label=e.get('label', e['action']), at=e['at'], started=round(e['started'] - start, 3),
                             result=e['result'] if not isinstance(e['result'], dict) else {k: e['result'][k] for k in e['result'] if k != 'body'} | ({'body': e['result']['body'][:160]} if 'body' in e['result'] else {}))
                        for e in recorded]
    stats_path = output / 'runtime/mock-stats.jsonl'
    checks = []

    def check(name, ok, detail):
        checks.append(dict(name=name, ok=bool(ok), detail=detail))

    if spec.get('wrong_allowed'):
        first_op = min(spec['ops'])
        early = [r for r in rows if r['status'] == 'wrong' and r['planned'] < first_op]
        result['wrong_allowed'] = spec['wrong_allowed']
        check('answers_differ_from_the_oracle_only_after_the_operation', not early,
              f"wrong={statuses.get('wrong', 0)} (all after t={first_op}s expected={not early})")
    else:
        check('no_wrong_answers', statuses.get('wrong', 0) == 0, f"wrong={statuses.get('wrong', 0)}")
    missed = statuses.get('send_error', 0) + statuses.get('unsent', 0)
    check('sender_coverage', missed <= 0.002 * len(rows),
          f"sender missed {missed} of {len(rows)} slots (send_error={statuses.get('send_error', 0)} unsent={statuses.get('unsent', 0)})")
    if spec['kind'] == 'F' and spec.get('fault_at') is not None:
        f0, f1 = spec['fault_at'], spec['clear_at']
        affected = spec['affected']
        controls = [g for g in groups if g not in affected]
        result['windows'] = {
            # Ends a second before the injection: queries in flight at that instant are the fault's, not the baseline's.
            'baseline': {g: analyze.window(rows, 5, f0 - 1, {g}) for g in groups},
            'fault': {g: analyze.window(rows, f0 + 1, f1, {g}) for g in groups},
            'after': {g: analyze.window(rows, f1 + 12, duration - 2, {g}) for g in groups}}
        check('baseline_healthy', all((v['healthy_rate_of_sent'] or 0) >= .999 for v in result['windows']['baseline'].values()),
              {g: v['healthy_rate_of_sent'] for g, v in result['windows']['baseline'].items()})
        check('controls_unaffected_during_fault',
              all((result['windows']['fault'][g]['healthy_rate_of_sent'] or 0) >= .999 for g in controls),
              {g: result['windows']['fault'][g]['healthy_rate_of_sent'] for g in controls})
        recovery = {g: analyze.recovery_seconds(rows, f1, {g}) for g in affected}
        result['recovery_s'] = recovery
        check('recovery_within_10s', all(v is not None and v <= 10 for v in recovery.values()), recovery)
        result['resources'] = {'baseline': analyze.resources(samples, start, 5, f0), 'fault': analyze.resources(samples, start, f0, f1),
                               'tail': analyze.resources(samples, start, duration - 10, duration)}
        base, tail = result['resources']['baseline'], result['resources']['tail']
        if base and tail:
            check('resources_return_to_baseline',
                  tail['fd']['last'] <= base['fd']['last'] + 16 and tail['threads']['last'] <= base['threads']['last'] + 4
                  and tail['rss_kib']['last'] <= base['rss_kib']['last'] * 1.5 + 30 * 1024,
                  dict(fd=[base['fd']['last'], tail['fd']['last']], threads=[base['threads']['last'], tail['threads']['last']],
                       rss_kib=[base['rss_kib']['last'], tail['rss_kib']['last']]))
        result['mock_counters'] = {'baseline': analyze.counter_delta(stats_path, start, 5, f0),
                                   'fault': analyze.counter_delta(stats_path, start, f0, f1),
                                   'after': analyze.counter_delta(stats_path, start, f1 + 12, duration - 2)}
        extra_checks(spec, result, rows, start, output, check, role)
    elif spec['name'] == 'F6':
        f6_checks(spec, result, rows, samples, start, output, recorded, check, role)
    else:
        h_checks(spec, result, rows, samples, start, output, recorded, check, role)
    result['checks'] = checks
    result['pass'] = all(c['ok'] for c in checks)
    return result


def extra_checks(spec, result, rows, start, output, check, role):
    f0, f1 = spec['fault_at'], spec['clear_at']
    journal = output / 'runtime/mock.jsonl'
    wanted = spec['checks']
    injected_events = sorted((e for e in spec['events'] if e['action'] == 'faults'), key=lambda e: e['at'])
    stats_path = output / 'runtime/mock-stats.jsonl'
    observed, applied = {}, True
    for number, event in enumerate(injected_events):
        if not event['table']:
            continue
        table = validate_faults(event['table'])
        end = injected_events[number + 1]['at'] if number + 1 < len(injected_events) else f1
        # Counters are snapshotted once a second, so start two seconds in to exclude pre-injection traffic.
        window = analyze.counter_delta(stats_path, start, event['at'] + 2, end)
        for part, seen in analyze.injected(table, window, result['mock_counters']['baseline']).items():
            observed[f"t={event['at']}s {part}"] = seen
        # The mock must also have journaled this exact table (it applied the file, not just read it).
        applied &= any(e['table'] == table for e in analyze.journal_events(journal, start, event['at'] - 1, event['at'] + 3, 'faults'))
    # None = a silence-based proof without baseline traffic; the Go reference (persistent connections) may
    # lack it, the native host must not.
    check('fault_took_effect_as_seen_by_the_mock',
          observed and applied and all(v for v in observed.values() if v is not None)
          and (role == 'go' or None not in observed.values()),
          dict(observed=observed, applied_by_the_mock=applied))
    if 'pool_rebuild' in wanted:
        def ratio(window):
            accepts = sum(v for k, v in window.items() if k.endswith(':https:accept'))
            queries = sum(v for k, v in window.items() if k.endswith(':https:query'))
            return accepts, queries, (accepts / queries if queries else None)
        before, after = ratio(result['mock_counters']['baseline']), ratio(result['mock_counters']['after'])
        # The mock speaks HTTP/1.1, and the two entries of `foreign` race one request: the loser's
        # in-flight connection is discarded, so even healthy traffic opens ~0.5 connections per
        # query. A rebuild storm would show as a clearly higher ratio than that baseline.
        check('doh_connection_rate_returns_to_the_healthy_baseline',
              after[2] is not None and before[2] is not None and after[2] <= before[2] * 1.5 + 0.02,
              dict(healthy=dict(accepts=before[0], queries=before[1], per_query=before[2]),
                   recovered=dict(accepts=after[0], queries=after[1], per_query=after[2])))
    if 'no_cache_pollution' in wanted:
        revisit = [r for r in rows if r['group'] in ('domestic', 'foreign') and spec['revisit']['window'][0] <= r['planned'] < spec['revisit']['window'][1]]
        bad = [r for r in revisit if r['status'] != 'correct']
        check('revisited_names_answer_correctly_after_faults', revisit and not bad,
              dict(revisited=len(revisit), not_correct=len(bad), statuses={r['status']: 1 for r in bad[:3]}))
        # Every fault segment must be represented among the revisited names (by when each name was first asked).
        corpus = json.loads((output / 'corpus.json').read_text())
        first = {}
        for r in sorted(rows, key=lambda r: r['planned']):
            first.setdefault((corpus[r['index']]['name'], corpus[r['index']]['type']), r['planned'])
        bounds = [e['at'] for e in injected_events if e['table']] + [f1]
        coverage = {}
        for lo, hi in zip(bounds, bounds[1:]):
            for group in ('domestic', 'foreign'):
                names = {(corpus[r['index']]['name'], corpus[r['index']]['type']) for r in revisit if r['group'] == group}
                coverage[f'{group} {lo}-{hi}s'] = sum(1 for n in names if lo <= first[n] < hi)
        check('revisits_cover_every_fault_segment', all(v >= 50 for v in coverage.values()), coverage)
    if 'no_silent_direct' in wanted:
        direct = [e for e in analyze.journal_events(journal, start, f0 + 1, f1, 'dns') if e['group'] in ('foreign', 'foreignecs')]
        check('proxied_upstreams_never_connect_directly_while_the_proxy_is_down', not direct,
              f'upstream queries that reached the foreign mock without a working proxy: {len(direct)}')
    if 'fallback_behavior' in wanted:
        fb = analyze.window(rows, f0 + 1, f1, {'fb'})
        foreign = analyze.window(rows, f0 + 1, f1, {'foreign'})
        result['fallback'] = dict(fb=fb, foreign=foreign)
        median = (fb['correct_latency_ms'] or {}).get('p50')
        check('fallback_answers_after_the_configured_threshold',
              median is not None and 900 <= median <= 1500 and fb['statuses'].get('timeout', 0) == 0,
              dict(p50_ms=median, statuses=fb['statuses']))


def source_file_replaced(output):
    """True when the rule file the mock served was rewritten after the run began
    (the host's successful download replaces it; failed downloads leave it alone)."""
    targets = {e['target'] for e in analyze.journal_events(output / 'runtime/mock.jsonl', 0, float('-inf'), float('inf'), 'download')}
    began = (output / 'job.json').stat().st_mtime
    for manifest in (output / 'runtime/srs').glob('*.json'):
        for entry in json.loads(manifest.read_text()):
            if entry.get('url') and any(entry['url'].endswith(t) for t in targets):
                path = output / 'runtime' / entry.get('files', '')
                if path.is_file() and path.stat().st_mtime > began:
                    return True
    return False


def f6_checks(spec, result, rows, samples, start, output, recorded, check, role):
    log = (output / 'host-log-tail.txt').read_text()
    result['download_log_lines'] = [line[-220:] for line in log.splitlines() if 'update' in line.lower()][:20]
    triggers = [e for e in recorded if e.get('label', '').startswith('download ') and e['action'] == 'http']
    accepted = [e for e in triggers if isinstance(e['result'], dict) and e['result'].get('status') in (200, 202)]
    check('update_requests_accepted', len(accepted) == len(triggers) and triggers,
          f'{len(accepted)}/{len(triggers)} accepted')
    window = analyze.window(rows, 5, spec['duration'] - 2)
    result['windows'] = {'all': window}
    check('dns_correct_throughout', window['statuses'].get('wrong', 0) == 0 and (window['healthy_rate_of_sent'] or 0) >= .999,
          dict(healthy_rate_of_sent=window['healthy_rate_of_sent'], statuses=window['statuses']))
    served = [e['mode'] for e in analyze.journal_events(output / 'runtime/mock.jsonl', start, 0, spec['duration'] + 60, 'download')]
    check('mock_served_each_injected_download_mode_in_order', served == ['status', 'status', 'oversize', 'short', 'stall', 'ok'], served)
    if role == 'native':
        # Each failure leaves its own error line, in order, and nothing fails after the final good download.
        errors = [line for line in log.splitlines() if 'failed to update' in line]
        expected = ['bad status: 500', 'bad status: 503', 'body exceeds', 'read body', 'timed out']
        check('each_failed_download_is_in_the_log_with_its_own_reason',
              len(errors) == len(expected) and all(text in line for text, line in zip(expected, errors)),
              [line.split('failed to update', 1)[1][-60:] for line in errors])
        check('final_good_download_replaced_the_source_file', source_file_replaced(output), 'file modified after the run began')
    result['resources'] = {'all': analyze.resources(samples, start, 0, spec['duration'])}
    peak = result['resources']['all']['rss_kib']['peak'] if result['resources']['all'] else None
    check('rss_peak_below_256_mib', peak is not None and peak <= 256 * 1024, f'peak {peak} KiB')


def h_checks(spec, result, rows, samples, start, output, recorded, check, role):
    ops = spec['ops']
    lost = sum(1 for r in rows if r['status'] in ('timeout', 'send_error', 'unsent'))
    check('no_lost_queries', lost == 0, f'lost={lost}')
    baseline = analyze.window(rows, 15, ops[0])
    result['windows'] = {'baseline': baseline}
    base_p99 = (baseline['answer_latency_ms'] or {}).get('p99') or 0
    result['operations'] = []
    poll = next((e for e in recorded if e['action'] == 'poll'), None)
    for index, at in enumerate(ops):
        # The window is 15 s; for the requery task it lasts until the task reports idle.
        hi = at + 15
        if spec['name'] == 'H4' and poll:
            hi = max(hi, poll['finished'] - start + 1)
        window = analyze.window(rows, at, hi)
        slow = [r for r in rows if at <= r['planned'] < hi and r['finished'] is not None
                and (r['finished'] - r['planned']) * 1000 > max(50, 3 * base_p99)]
        result['operations'].append(dict(at=at, until=round(hi, 1), window=window, slow_rows=len(slow), udp_drops=analyze.udp_delta(samples, start, at - 1, hi),
                                         lost=sum(1 for r in rows if at <= r['planned'] < hi and r['status'] in ('timeout', 'send_error', 'unsent')),
                                         disturbance_s=(max(r['planned'] for r in slow) - min(r['planned'] for r in slow)) if slow else 0))
    scheduled = [e for e in json.loads((output / 'job.json').read_text())['events'] if e['action'] == 'http']
    done = [e for e in recorded if e['action'] == 'http']
    check('operations_ok', len(done) == len(scheduled) and all(isinstance(e['result'], dict) and e['result'].get('status') in (200, 201, 202, 204) for e in done),
          dict(scheduled=len(scheduled), done=len(done), statuses=[e['result'].get('status') if isinstance(e['result'], dict) else str(e['result'])[:60] for e in done]))
    if spec['name'] == 'H2':
        served = [e['mode'] for e in analyze.journal_events(output / 'runtime/mock.jsonl', 0, float('-inf'), float('inf'), 'download')]
        check('each_refresh_downloaded_the_source', served == ['ok'] * len(ops), served)
        if role == 'native':
            check('source_file_replaced_by_the_refresh', source_file_replaced(output), 'file modified after the run began')
    if spec.get('wrong_allowed'):
        # The differences must be well-formed routing changes: the right question, NOERROR, and only
        # addresses the mock hands out (no corrupt, truncated or foreign data).
        corpus = json.loads((output / 'corpus.json').read_text())
        markers = {address for pair in MARKERS.values() for address in pair}
        odd = []
        for r in rows:
            if r['status'] != 'wrong':
                continue
            rcode, questions, answers = r['answer']
            wanted = (corpus[r['index']]['name'].lower().rstrip('.'), corpus[r['index']]['type'])
            if rcode != 0 or [(q[0].lower().rstrip('.'), q[1]) for q in questions] != [wanted] or not {a[3] for a in answers} <= markers:
                odd.append(r['seq'])
        check('differences_are_well_formed_routing_changes', not odd, f'{len(odd)} malformed of {result["statuses"].get("wrong", 0)}')
        # A prompt SERVFAIL is not a routing change: the task must not make the host fail queries.
        check('no_servfail_or_fallback_answers_during_the_task', not result['statuses'].get('servfail', 0) and not result['statuses'].get('fallback', 0),
              {k: v for k, v in result['statuses'].items() if k not in ('correct', 'wrong')})
    key = 'answered_on_time_of_sent' if spec.get('wrong_allowed') else 'healthy_rate_of_sent'
    check('every_window_healthy' if key == 'healthy_rate_of_sent' else 'every_window_answered_on_time',
          all((o['window'][key] or 0) >= .999 for o in result['operations']), [o['window'][key] for o in result['operations']])
    result['resources'] = {'all': analyze.resources(samples, start, 0, spec['duration'])}
    peak = result['resources']['all']['rss_kib']['peak'] if result['resources']['all'] else None
    check('rss_peak_below_256_mib', peak is not None and peak <= 256 * 1024, f'peak {peak} KiB')
    journal = output / 'runtime/mock.jsonl'
    if spec['name'] == 'H1' and role == 'native':
        expectations = {'probe before': (17891, 'https'), 'probe changed': (17892, 'tcp'),
                        'probe changed late': (17892, 'tcp'), 'probe reverted': (17891, 'https')}
        observed = {}
        for event in (e for e in recorded if e['action'] == 'probe'):
            t = event['started'] - start
            socks = analyze.journal_events(journal, start, t - 0.2, t + 3, 'socks')
            dns = [e for e in analyze.journal_events(journal, start, t - 0.2, t + 3, 'dns') if e['group'] == 'foreign']
            observed[event['label']] = dict(answered=event['result'].get('answered'), via=sorted({e['via'] for e in socks}),
                                            protocols=sorted({e['protocol'] for e in dns}))
        check('new_queries_use_the_new_proxy_and_list',
              all(label in observed and observed[label]['answered'] and observed[label]['via'] == [via] and observed[label]['protocols'] == [protocol]
                  for label, (via, protocol) in expectations.items()), observed)
    if spec['name'] == 'H3':
        probes = [e for e in recorded if e['action'] == 'probe']
        check('dedicated_listener_follows_the_group',
              all(e['result'].get('answered') == (e['label'] == 'probe dedicated port') for e in probes), [(e['label'], e['result']) for e in probes])
    if spec['name'] == 'H4':
        poll = next((e for e in recorded if e['action'] == 'poll'), None)
        check('requery_task_completed', bool(poll and isinstance(poll['result'], dict) and poll['result'].get('reached')), poll['result'] if poll else None)


def reanalyze(directory):
    """Re-evaluates a finished run's retained data with the current checks. The previous
    result.json is kept as result-v1.json (the first re-analysis only)."""
    directory = pathlib.Path(directory).resolve()
    old = json.loads((directory / 'result.json').read_text())
    spec = build_scenarios()[old['scenario']]
    if old['rate'] != spec['rate']:
        spec = dict(spec, rate=old['rate'])
    events = json.loads((directory / 'job.json').read_text())['events']
    result = analysis(spec, old['role'], directory, events)
    if not (directory / 'result-v1.json').exists():
        (directory / 'result-v1.json').write_text(json.dumps(old, indent=2))
    (directory / 'result.json').write_text(json.dumps(result, indent=2))
    return result


def main(argv):
    import argparse
    if argv[:1] == ['--reanalyze']:
        for directory in argv[1:]:
            result = reanalyze(directory)
            print(json.dumps(dict(run=pathlib.Path(directory).name, passed=result['pass'],
                                  failed=[c['name'] for c in result['checks'] if not c['ok']])))
        return 0
    parser = argparse.ArgumentParser()
    parser.add_argument('scenario', choices=sorted(build_scenarios()))
    parser.add_argument('--work', type=pathlib.Path, required=True)
    parser.add_argument('--role', choices=('go', 'native'), default='native')
    parser.add_argument('--binary', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--seed', type=int, default=501)
    parser.add_argument('--rate', type=int, help='H scenarios only: override the load (qps) to see how an effect scales')
    args = parser.parse_args(argv)
    spec = build_scenarios()[args.scenario]
    if args.rate:
        if spec['kind'] != 'H':
            parser.error('--rate applies to H scenarios')
        spec = dict(spec, rate=args.rate)
    result = run_scenario(spec, args.work.resolve(), args.role, args.binary.resolve(), args.output.resolve(), args.seed)
    print(json.dumps(dict(scenario=result['scenario'], role=result['role'], passed=result['pass'],
                          failed=[c['name'] for c in result['checks'] if not c['ok']])))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
