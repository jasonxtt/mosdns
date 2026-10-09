"""Sanitized numerical summaries; never copy private names or metric labels."""
import argparse
import collections
import json
import pathlib
import re
import statistics


def cache(text):
    counts={}
    for line in text.splitlines():
        match=re.match(r'(mosdns_cache_(?:hit|lazy_hit|query)_total)(?:\{[^}]*\})?\s+(\S+)',line)
        if match: counts[match[1]]=counts.get(match[1],0)+float(match[2])
    return counts


def primary_cache(text):
    # cache_all is the outer production cache. Summed nested cache lookups are
    # a different denominator and must not be called a request hit rate.
    counts={}
    for line in text.splitlines():
        match=re.match(r'(mosdns_cache_(?:hit|lazy_hit|query)_total)\{([^}]*)\}\s+(\S+)',line)
        if match and re.search(r'(?:^|,)tag="cache_all"(?:,|$)',match[2]):
            counts[match[1]]=float(match[3])
    return counts


def summarize(roots):
    runs=[]
    for root in roots:
        summary=json.loads((root/'load/summary.json').read_text())
        meta=json.loads((root/'run.json').read_text())
        startup=json.loads((root/'startup.json').read_text())
        samples=json.loads((root/'load/resources.json').read_text())
        metrics=json.loads((root/'load/metrics.json').read_text())
        start,end=cache(metrics['start']),cache(metrics['end'])
        delta={k:end.get(k,0)-start.get(k,0) for k in end}
        query=delta.get('mosdns_cache_query_total',0)
        hit=delta.get('mosdns_cache_hit_total',0)
        outer_start,outer_end=primary_cache(metrics['start']),primary_cache(metrics['end'])
        outer_delta={k:outer_end.get(k,0)-outer_start.get(k,0) for k in outer_end}
        outer_query=outer_delta.get('mosdns_cache_query_total',0)
        outer_hit=outer_delta.get('mosdns_cache_hit_total',0)
        mock_counts=collections.Counter()
        for line in (root/'runtime/mock.jsonl').open():
            event=json.loads(line)
            if event['kind']=='dns':
                if event['group'] not in ('domestic','foreign','foreignecs','cnfake','nocnfake'):
                    raise ValueError('unknown mock group')
                mock_counts[event['group']+':'+event['protocol']]+=1
            elif event['kind']=='socks': mock_counts['socks_allowed' if event['allowed'] else 'socks_denied']+=1
        cpus={}
        for cpu in ('cpu0','cpu1'):
            first,last=samples[0]['cpu'][cpu],samples[-1]['cpu'][cpu]
            diff=[b-a for a,b in zip(first,last)]
            # Guest times already included in user/nice on Linux.
            total=sum(diff[:8]); idle=sum(diff[3:5])
            cpus[cpu]=dict(busy_fraction=(total-idle)/total,steal_fraction=diff[7]/total)
        processes={}
        for role in ('sut','mock','sender'):
            values=[s['processes'][role] for s in samples if not s['processes'][role].get('missing')]
            processes[role]=dict(cpu_s=values[-1]['cpu_s']-values[0]['cpu_s'],peak_rss_kib=max(s['rss_kib'] for s in values),
                                affinity=values[0]['affinity'],fd_start=values[0]['fd'],fd_end=values[-1]['fd'])
        offset=0
        for stage in summary['stages']:
            lo=summary['started']+offset
            hi=lo+stage['duration']; offset+=stage['duration']
            window=[s for s in samples if lo<=s['time']<=hi]
            if len(window)<2: continue
            elapsed=window[-1]['time']-window[0]['time']
            stage['sampling_cpu_fraction']={}
            for role in ('sut','mock','sender'):
                first,last=[s['processes'][role] for s in (window[0],window[-1])]
                if not first.get('missing') and not last.get('missing'):
                    stage['sampling_cpu_fraction'][role]=(last['cpu_s']-first['cpu_s'])/elapsed
            stage['cpu_steal_fraction']={}
            for cpu in ('cpu0','cpu1'):
                diff=[b-a for a,b in zip(window[0]['cpu'][cpu],window[-1]['cpu'][cpu])]
                stage['cpu_steal_fraction'][cpu]=diff[7]/max(1,sum(diff[:8]))
        runs.append(dict(role=meta['role'],seed=summary['seed'],corpus_sha256=summary['corpus_sha256'],oracle_sha256=summary['oracle_sha256'],
                         startup_rss_kib=[r['rss_kib'] for r in startup if 'rss_kib' in r],
                         startup_peak_rss_kib=max(r.get('rss_kib',0) for r in startup),
                         cache_counters=delta,cache_lookup_hit_fraction=hit/query if query else None,
                         outer_cache_counters=outer_delta,outer_cache_hit_fraction=outer_hit/outer_query if outer_query else None,
                         mock_counts=dict(mock_counts),
                         cpus=cpus,processes=processes,orphan_or_late_packets=summary['orphan_or_late_packets'],
                         stages=summary['stages'],cleanup=json.loads((root/'cleanup.json').read_text())))
    aggregates=[]
    for stage in runs[0]['stages']:
        values=[next(s for s in r['stages'] if s['stage']==stage['stage']) for r in runs]
        def spread(values):
            values=[v for v in values if v is not None]
            return dict(median=statistics.median(values),min=min(values),max=max(values)) if values else None
        aggregates.append(dict(stage=stage['stage'],rate=stage['rate'],
                               planned_latency_ms={k:spread([s['stable']['planned_latency_ms'].get(k) for s in values]) for k in ('p50','p95','p99')},
                               correct_rate=spread([s['stable']['correct_rate'] for s in values]),
                               send_coverage=spread([s['stable']['send_coverage'] for s in values]),
                               cpu_us_per_correct=spread([s.get('resources',{}).get('cpu_us_per_correct') for s in values]),
                               rss_median_kib=spread([s.get('resources',{}).get('rss_median_kib') for s in values]),
                               rss_peak_kib=spread([s.get('resources',{}).get('rss_peak_kib') for s in values])))
    return dict(runs=runs,aggregates=aggregates)


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('roots',type=pathlib.Path,nargs='+')
    parser.add_argument('--output',type=pathlib.Path,required=True)
    args=parser.parse_args()
    args.output.write_text(json.dumps(summarize(args.roots),indent=2)+'\n')
