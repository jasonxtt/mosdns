"""Numerical budget evaluation, with explicit missing-observability limits."""
import argparse
import json
import pathlib


def evaluate(baseline, candidate, freeze):
    go={s['stage']:s for s in baseline['aggregates']}
    rows=[]
    for stage in candidate['aggregates']:
        name=stage['stage']
        values=[next(s for s in r['stages'] if s['stage']==name) for r in candidate['runs']]
        cap=freeze['latency_ms'].get('F' if freeze.get('scenario')=='F' and name=='q100' else name)
        median={k:stage['planned_latency_ms'][k]['median'] for k in ('p50','p95','p99')}
        correct=all(s['stable']['correct_rate']>=.999 for s in values)
        wrong_free=all(not sum(count for status,count in s['stable']['statuses'].items() if status in ('wrong','late_wrong')) for s in values)
        coverage=all(s['stable']['send_coverage']>=.999 for s in values)
        latency=None if cap is None else median['p95']<=cap['p95'] and median['p99']<=cap['p99']
        reference=go[name]['cpu_us_per_correct']['median']
        cpu=stage['cpu_us_per_correct']['median']
        common=freeze['capacity_latency_ms']
        sustainable=all(s['stable']['planned_latency_ms']['p95'] is not None
                        and s['stable']['planned_latency_ms']['p95']<=common['p95']
                        and s['stable']['planned_latency_ms']['p99']<=common['p99']
                        and s['stable']['correct_rate']>=.999 and s['stable']['send_coverage']>=.999 for s in values)
        normal=freeze.get('scenario')=='F' or cap is not None or sustainable
        memory=[s['resources']['rss_median_kib'] for s in values]
        rss_limit=min(128*1024,go[name]['rss_median_kib']['median'])
        rows.append(dict(stage=name,rate=stage['rate'],median_latency_ms=median,
                         latency_pass=latency,correctness_each_repeat_pass=correct,coverage_each_repeat_pass=coverage,
                         frozen_oracle_agreement_pass=wrong_free,
                         native_cpu_us_per_correct=cpu,go_cpu_us_per_correct=reference,cpu_ratio=cpu/reference,
                         profiling_required=normal and any(s['resources']['cpu_us_per_correct']>1.5*reference for s in values),
                         non_overload_for_cpu=normal,sustainable_each_repeat=sustainable,
                         native_steady_rss_kib=memory,go_steady_median_kib=go[name]['rss_median_kib']['median'],
                         steady_rss_pass=all(x<=rss_limit for x in memory)))
    peaks=[max(r['startup_peak_rss_kib'],r['processes']['sut']['peak_rss_kib']) for r in candidate['runs']]
    capacity=max((r['rate'] for r in rows if r['stage'].startswith('q') and r['sustainable_each_repeat']),default=0)
    go_capacity=max((s['rate'] for s in baseline['aggregates'] if s['stage'].startswith('q')
                     and all(next(v for v in r['stages'] if v['stage']==s['stage'])['stable']['correct_rate']>=.999
                             and next(v for v in r['stages'] if v['stage']==s['stage'])['stable']['send_coverage']>=.999
                             and next(v for v in r['stages'] if v['stage']==s['stage'])['stable']['planned_latency_ms']['p95']<=freeze['capacity_latency_ms']['p95']
                             and next(v for v in r['stages'] if v['stage']==s['stage'])['stable']['planned_latency_ms']['p99']<=freeze['capacity_latency_ms']['p99'] for r in baseline['runs'])),default=0)
    recovery=[]
    for run in candidate['runs']:
        tail=next(s for s in run['stages'] if s['stage']=='recovery')
        healthy=[]
        budget=freeze['latency_ms']['F' if freeze.get('scenario')=='F' else 'q100']
        for value in tail['seconds']:
            q=value['planned_latency_ms']
            healthy.append(value['correct_rate']>=.999 and value['send_coverage']>=.999
                           and q['p95'] is not None and q['p95']<=budget['p95'] and q['p99']<=budget['p99'])
        completion=next((i+5 for i in range(len(healthy)-4) if all(healthy[i:i+5])),None)
        recovery.append(dict(seed=run['seed'],five_healthy_bins_finish_s=completion,
                             latency_correctness_within_10s=None if freeze.get('scenario')=='F' else completion is not None and completion<=10,
                             context='continued unique load, no overload-recovery test' if freeze.get('scenario')=='F' else 'inspect service overload and FD traces before recovery conclusion',
                             fd_end=tail['resources']['fd_end'],threads_end=tail['resources']['threads_end']))
    return dict(stages=rows,peak_rss_kib=peaks,peak_rss_pass=all(p<=256*1024 for p in peaks),
                tested_sustainable_capacity=capacity,go_tested_sustainable_capacity=go_capacity,
                capacity_ratio_report_only=capacity/go_capacity if go_capacity else None,
                capacity_floor_pass=None if freeze.get('scenario') in ('F','extension') else capacity>=2000,recovery=recovery,
                oracle_agreement_definition='strict pre-frozen oracle plus TTL; disagreement alone does not prove a product-contract violation',
                query_in_flight_observability='not exposed by pinned native HTTP/metrics API; not proven')


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for name in ('baseline','candidate','freeze','output'):
        parser.add_argument('--'+name,type=pathlib.Path,required=True)
    parser.add_argument('--scenario-f',action='store_true')
    parser.add_argument('--extension',action='store_true')
    args=parser.parse_args()
    freeze=json.loads(args.freeze.read_text())
    if args.scenario_f: freeze['scenario']='F'
    if args.extension: freeze['scenario']='extension'
    args.output.write_text(json.dumps(evaluate(json.loads(args.baseline.read_text()),json.loads(args.candidate.read_text()),freeze),indent=2)+'\n')
