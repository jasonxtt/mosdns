#!/usr/bin/env python3
"""Qualify the two fixed M2 control batches; never award candidate PASS."""
import csv
import json
import math
import statistics
import sys
from pathlib import Path

GROUPS = (('w1-tcp', 'main', 200), ('w1-tcp', 'main', 400),
          ('w2', 'cold', 200), ('w2', 'warm', 200), ('w2', 'warm', 400),
          ('w3', 'main', 200), ('w3', 'main', 400))
VARIANTS = ('before_off', 'after_off', 'after_on')
T5 = 2.01504837333302
MARGIN = math.log(1.10)


def interval(values):
    """Two-sided 90% interval on six independent whole-attempt log ratios."""
    if len(values) != 6 or not all(math.isfinite(x) for x in values):
        raise ValueError('exactly six finite paired log ratios are required')
    center = statistics.mean(values)
    half = T5 * statistics.stdev(values) / math.sqrt(6)
    return center - half, center + half


def qualify(root, groups=GROUPS, profile='m2'):
    failures, batches, intervals = [], [], []
    expected = {(s, p, q, v, r) for s, p, q in groups
                for v in VARIANTS for r in range(1, 4)}
    for batch in ('batch1', 'batch2'):
        directory = root / batch
        with (directory / 'derived-primary-measurements.tsv').open() as f:
            rows = list(csv.DictReader(f, delimiter='\t'))
        indexed = {}
        for row in rows:
            key = (row['scenario'], row['phase'], int(row['qps']),
                   row['variant'], int(row['repetition']))
            if key in indexed:
                failures.append(f'{batch}: duplicate primary key {key}')
            indexed[key] = row
            if (row['valid'] != '1' or row['runner_exit'] != '0'
                    or not (row['scheduled'] == row['sent'] == row['received'] == row['correct_on_time'])
                    or int(row['scheduled']) <= 0
                    or any(int(row[k]) != 0 for k in ('correct_late', 'wrong_response',
                                                     'protocol_error', 'transport_error',
                                                     'timeout', 'sender_shortfall'))):
                failures.append(f'{batch}: invalid primary row {key}')
        if len(rows) != len(expected) or set(indexed) != expected:
            failures.append(f'{batch}: primary matrix is incomplete or unexpected')
        for scenario in sorted({g[0] for g in groups}):
            for repetition in range(1, 4):
                for variant in VARIANTS:
                    attempt = directory / f'{scenario}-r{repetition}-{variant}'
                    evidence = dict(line.split('=', 1) for line in (attempt / 'environment.txt').read_text().splitlines() if '=' in line)
                    if evidence.get('measurement_profile') != profile or evidence.get('gomaxprocs_environment') != '1':
                        failures.append(f'{batch}: missing/wrong runtime profile in {attempt.name}')
                    if profile == 'm3':
                        metadata = dict(line.split('=', 1) for line in (attempt / 'run-metadata.txt').read_text().splitlines() if '=' in line)
                        settings = {'stage_duration_ms': '25000', 'normal_reference_qps': '200', 'overload_qps': '400', 'request_deadline_ms': '500', 'late_drain_ms': '100'}
                        if scenario == 'w2':
                            settings['w2_warm_lifecycle'] = 'independent-prefilled'
                        if any(metadata.get(k) != v for k, v in settings.items()):
                            failures.append(f'{batch}: wrong M3 measurement settings in {attempt.name}')
                    binary = json.loads((attempt / 'sut.json').read_text())
                    if binary.get('sha256') != '370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa':
                        failures.append(f'{batch}: wrong identical-baseline binary in {attempt.name}')
        if profile == 'm3':
            for key, row in indexed.items():
                if int(row['scheduled']) != key[2] * 25:
                    failures.append(f'{batch}: wrong M3 request count {key}')
        with (directory / 'derived-paired-assessments.tsv').open() as f:
            pairs = list(csv.DictReader(f, delimiter='\t'))
        expected_pairs = {(s, p, q, comp, metric) for s, p, q in groups
                          for comp in ('after_off_vs_before_off', 'after_on_vs_after_off')
                          for metric in ('p95_us', 'p99_us', 'cpu_us_per_correct_query', 'rss_peak_kib_sampled')}
        actual_pairs = set()
        for pair in pairs:
            actual_pairs.add((pair['scenario'], pair['phase'], int(pair['qps']), pair['comparison'], pair['metric']))
            if pair['valid_pairs'] != '3':
                failures.append(f'{batch}: paired comparison lacks three valid pairs')
            if pair['metric'] in ('p95_us', 'p99_us') and int(pair['pairs_above_guard']) > 0:
                failures.append(f"{batch}: individual original latency guard crosses: {pair['scenario']} {pair['phase']} {pair['qps']} {pair['comparison']} {pair['metric']}")
        if len(pairs) != len(expected_pairs) or actual_pairs != expected_pairs:
            failures.append(f'{batch}: original paired assessment matrix is incomplete or unexpected')
        batches.append(indexed)
    if any(set(batch) != expected for batch in batches):
        return {'qualified': False, 'failures': failures, 'intervals': []}
    for scenario, phase, qps in groups:
        for control, candidate in (('before_off', 'after_off'), ('after_off', 'after_on')):
            for metric in ('p95_us', 'p99_us'):
                values = []
                for batch in batches:
                    for repetition in range(1, 4):
                        a = float(batch[(scenario, phase, qps, control, repetition)][metric])
                        b = float(batch[(scenario, phase, qps, candidate, repetition)][metric])
                        if not (math.isfinite(a) and math.isfinite(b) and a > 0 and b > 0):
                            failures.append('nonpositive/nonfinite latency value')
                            continue
                        values.append(math.log(b / a))
                if len(values) != 6:
                    continue
                low, high = interval(values)
                qualified = low > -MARGIN and high < MARGIN
                row = {'scenario': scenario, 'phase': phase, 'qps': qps,
                       'comparison': f'{candidate}_vs_{control}', 'metric': metric,
                       'paired_log_ratios': values,
                       'ci90_low_pct': math.expm1(low) * 100,
                       'ci90_high_pct': math.expm1(high) * 100,
                       'qualified': qualified}
                intervals.append(row)
                if not qualified:
                    failures.append(f'{scenario} {phase} {qps} {candidate}_vs_{control} {metric}: equivalence interval outside margin')
    return {'qualified': not failures, 'scope': f'identical-baseline {profile.upper()} calibration only',
            'failures': failures, 'intervals': intervals}


if __name__ == '__main__':
    try:
        result = qualify(Path(sys.argv[1]))
    except (OSError, ValueError, KeyError, IndexError) as error:
        result = {'qualified': False, 'failures': [str(error)], 'intervals': []}
    print(json.dumps(result, indent=2, allow_nan=False))
    sys.exit(0 if result['qualified'] else 2)
