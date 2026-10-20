"""Hourly private differences; equal traffic becomes numerical aggregates only."""
import collections
import gzip
import json
import os
import pathlib
import shutil
import struct


def atomic(path, value):
    temporary = path.with_suffix(path.suffix + '.tmp')
    with temporary.open('w') as stream:
        stream.write(json.dumps(value, separators=(',', ':')) + '\n')
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


class HourStore:
    def __init__(self, root, start):
        self.root, self.start = root, start
        self.hour = self.minute = None
        self.metrics, self.minute_metrics = collections.Counter(), collections.Counter()
        self.streams = {}
        self.folder = None

    def stream(self, name, binary=False):
        if name not in self.streams:
            self.streams[name] = (self.folder / name).open('ab' if binary else 'a', buffering=0 if binary else 1)
        return self.streams[name]

    def flush_minute(self):
        if self.minute is not None and self.minute_metrics:
            self.stream('minutes.jsonl').write(json.dumps({'minute': self.minute,
                'start_wall': self.start + self.minute * 60, 'metrics': dict(self.minute_metrics)}) + '\n')
        self.minute_metrics.clear()

    def checkpoint(self):
        if self.folder:
            atomic(self.folder / 'stats.json', {'hour': self.hour, 'start_wall': self.start + self.hour * 3600,
                                               'metrics': dict(self.metrics)})

    def close_hour(self):
        self.flush_minute()
        self.checkpoint()
        for stream in self.streams.values():
            stream.close()
        self.streams.clear()
        if self.folder:
            for name in ('differences.jsonl', 'latencies.bin', 'outcomes.bin', 'minutes.jsonl'):
                source = self.folder / name
                if source.exists():
                    with source.open('rb') as src, gzip.open(str(source) + '.gz', 'wb', compresslevel=3) as dst:
                        shutil.copyfileobj(src, dst, length=65536)
                    source.unlink()

    def record(self, row, wall):
        hour = max(0, int((wall - self.start) // 3600))
        minute = max(0, int((wall - self.start) // 60))
        if hour != self.hour:
            self.close_hour()
            self.hour, self.minute = hour, None
            self.metrics.clear()
            self.folder = self.root / 'hours' / f'{hour:03d}'
            self.folder.mkdir(parents=True, exist_ok=True)
        if minute != self.minute:
            self.flush_minute()
            self.minute = minute
        values = collections.Counter({'rows': 1, 'class:' + row['classification']: 1})
        if row.get('different_fields'):
            category = '+'.join(sorted(row['different_fields']))
            values['difference:' + category] += 1
            if row['classification'] == 'unexplained':
                values['unexplained:' + category] += 1
        values['submitted'] += 'sent_monotonic' in row
        values['audit_missing'] += 'audit' not in row
        audit = row.get('audit', {})
        mapping = audit.get('final_upstream_group')
        values['audit_unmapped'] += mapping == 'unmapped_or_ambiguous'
        diagnostics = audit.get('upstream_diagnostics') or {}
        for attempt in diagnostics.get('attempts', []):
            if isinstance(attempt, dict):
                # Only fixed categories are exported, never a target/address.
                result = str(attempt.get('result', attempt.get('status', 'unknown')))
                values['upstream_attempts'] += 1
                values['upstream_attempt_cancelled'] += 'cancel' in result
                values['upstream_attempt_failed'] += 'fail' in result or 'error' in result
        if 'go' in row and 'rust' in row:
            left, right = row['go']['tc'], row['rust']['tc']
            values.update({'paired': 1, 'go_tc': int(left), 'rust_tc': int(right),
                'go_only_tc': int(left and not right), 'rust_only_tc': int(right and not left),
                'rust_servfail_go_noerror': int(row.get('rust_servfail_go_noerror', False))})
            if bytes.fromhex(row['query_wire'])[10:12] == b'\0\0':
                go_size, rust_size = len(row['go_wire']) // 2, len(row['rust_wire']) // 2
                values['query_ar_zero'] += 1
                values['no_edns_rust_over_512'] += rust_size > 512
                values['no_edns_over_512_without_tc'] += rust_size > 512 and not right
                if left or right:
                    values['no_edns_tc_pairs'] += 1
                    values['no_edns_tc_length_equal'] += go_size == rust_size
                    values['no_edns_tc_length_different'] += go_size != rust_size
        else:
            values['incomplete_go_missing'] += 'go' not in row
            values['incomplete_rust_missing'] += 'rust' not in row
        if 'latency_ms' in row:
            self.stream('latencies.bin', binary=True).write(struct.pack('!dd', wall, row['latency_ms']))
            values['latency_samples'] += 1
        self.metrics.update(values)
        self.minute_metrics.update(values)
        fields = ('rcode', 'has_answer', 'tc', 'types', 'class', 'address_kinds', 'cname_terminals', 'service_targets')
        mask = sum(1 << index for index, field in enumerate(fields) if field in row.get('different_fields', []))
        code = {'semantic_equal': 0, 'explained:learned-state': 1, 'unexplained': 2, 'incomplete': 3}[row['classification']]
        self.stream('outcomes.bin', binary=True).write(struct.pack('!dHB', wall, mask, code))
        if row['classification'] != 'semantic_equal':
            self.stream('differences.jsonl').write(json.dumps(row, ensure_ascii=True, separators=(',', ':')) + '\n')

    def close(self):
        self.close_hour()
