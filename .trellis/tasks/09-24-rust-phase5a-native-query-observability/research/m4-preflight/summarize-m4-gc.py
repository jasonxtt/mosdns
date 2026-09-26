#!/usr/bin/env python3
"""Post-run evidence only; never executes during a measured stage."""
import hashlib
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
scenario = sys.argv[2]
upstreams = {'w1-tcp': ('forward',), 'w2': ('cache',), 'w3': ('route-a', 'route-b', 'route-c')}[scenario]
logs = [root / 'pilot.stderr.log']
temporary = list(root.glob('.run.*'))
if len(temporary) != 1:
    print(json.dumps({'qualified': False, 'error': 'exactly one runner temporary directory required'}))
    sys.exit(0)
logs += [temporary[0] / f'{upstream}.stderr' for upstream in upstreams]
records = []
for log in logs:
    if not log.exists():
        records.append({'path': log.relative_to(root).as_posix(), 'sha256': 'missing', 'gc_trace_lines': -1})
        continue
    content = log.read_bytes()
    records.append({'path': log.relative_to(root).as_posix(),
                    'sha256': hashlib.sha256(content).hexdigest(),
                    'gc_trace_lines': len(re.findall(rb'^gc \d+ @', content, re.MULTILINE))})
peaks = {}
for path in root.rglob('resource-samples.jsonl'):
    for line in path.read_text().splitlines():
        sample = json.loads(line)
        if sample['role'] != 'sut':
            peaks[sample['role']] = max(peaks.get(sample['role'], 0), sample['rss_kib'])
expected_roles = {'load-generator'} | {f'fixture-{i}' for i in range(1, len(upstreams) + 1)}
result = {'logs': records, 'gc_trace_lines': sum(r['gc_trace_lines'] for r in records),
          'sampled_rss_peaks_kib': peaks,
          'qualified': set(peaks) == expected_roles and all(0 < v <= 262144 for v in peaks.values()) and all(r['gc_trace_lines'] == 0 for r in records)}
print(json.dumps(result, indent=2))
