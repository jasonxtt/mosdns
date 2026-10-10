"""Numerical fix-A summaries and gates, derived from every planned run."""
import argparse
import json
import pathlib
import statistics

from summarize import summarize


def report(root, output, freeze):
    output.mkdir(exist_ok=True)
    limits = json.loads(freeze.read_text())
    manifest = json.loads((root/'fix-a-matrix.json').read_text())
    if manifest['plan'] != [{k: row[k] for k in ('name', 'role', 'scenario', 'seed')}
                            for row in manifest['attempts']]:
        raise ValueError('every planned attempt required')
    comparisons = []
    cpu_checks = []
    for scenario in ('f', 'normal'):
        values = {}
        for role in ('go', 'native'):
            values[role] = summarize([root/f'{role}-{scenario}-r{repeat}' for repeat in (1, 2, 3)])
            (output/f'{role}-{scenario}.json').write_text(json.dumps(values[role], indent=2)+'\n')
        for go_stage in values['go']['aggregates']:
            stage = go_stage['stage']
            reference = go_stage['cpu_us_per_correct']['median']
            native = [next(s for s in run['stages'] if s['stage'] == stage)
                      ['resources']['cpu_us_per_correct'] for run in values['native']['runs']]
            cpu_checks.append(dict(scenario=scenario, stage=stage,
                                   go_median_us=reference, native_us=native,
                                   triggered_repetitions=[i for i, cpu in enumerate(native, 1)
                                                          if cpu > reference*1.5]))
        wanted = ('q100',) if scenario == 'f' else ('q20', 'q100', 'q500')
        for stage in wanted:
            rows = {role: [next(s for s in run['stages'] if s['stage'] == stage)
                           for run in values[role]['runs']] for role in values}
            ceiling = limits['latency_ms']['F' if scenario == 'f' else stage]
            latency = {q: statistics.median(s['stable']['planned_latency_ms'][q]
                                          for s in rows['native']) for q in ('p95', 'p99')}
            cpu_go = statistics.median(s['resources']['cpu_us_per_correct'] for s in rows['go'])
            cpu_native = [s['resources']['cpu_us_per_correct'] for s in rows['native']]
            correctness = all(s[window]['correct_rate'] >= limits['correct_on_time_each_repeat']
                              for s in rows['native'] for window in ('all', 'stable'))
            coverage = all(s[window]['send_coverage'] >= limits['sender_coverage_each_repeat']
                          for s in rows['native'] for window in ('all', 'stable'))
            wrong = sum(count for s in rows['native'] for status, count in s['all']['statuses'].items()
                        if 'wrong' in status)
            comparisons.append(dict(scenario=scenario, stage=stage, native_median_ms=latency,
                                    ceilings_ms=ceiling,
                                    latency_pass={q: latency[q] <= ceiling[q] for q in latency},
                                    correctness_each_repeat_pass=correctness,
                                    coverage_each_repeat_pass=coverage, wrong_dns=wrong,
                                    cpu_go_median_us=cpu_go, cpu_native_us=cpu_native,
                                    profiling_triggered_repetitions=[i for i, cpu in enumerate(cpu_native, 1)
                                                                    if cpu > cpu_go*1.5],
                                    rss_native_kib=[s['resources']['rss_median_kib'] for s in rows['native']],
                                    rss_go_kib=[s['resources']['rss_median_kib'] for s in rows['go']]))
    (output/'comparison.json').write_text(json.dumps(comparisons, indent=2)+'\n')
    (output/'cpu-trigger-checks.json').write_text(json.dumps(cpu_checks, indent=2)+'\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    for key in ('root', 'output', 'freeze-record'):
        parser.add_argument('--'+key, type=pathlib.Path, required=True)
    args = parser.parse_args()
    report(args.root, args.output, args.freeze_record)
