"""Sanitised one-line-per-run summary of a 5D-2 `runs/` directory.

    python3 tools/5d-2/report.py runs [--json]

Reads only result.json / plan.json (no host logs, no corpus names), so the output is
safe to paste into the public validation record.
"""
import argparse
import json
import pathlib


def failed_checks(result):
    return [c['name'] for c in result['checks'] if not c['ok']]


def summarise(directory):
    result = json.loads((directory / 'result.json').read_text())
    resources = result['resources']
    peak_kib = max(window['rss_kib']['peak'] for window in resources.values() if window)
    drops = result.get('udp_drops_whole_run') or {}
    return dict(run=directory.name, scenario=result['scenario'], role=result['role'], rate=result['rate'],
                passed=result['pass'], failed_checks=failed_checks(result), statuses=result['statuses'],
                recovery_s=result.get('recovery_s'), rss_peak_mib=round(peak_kib / 1024, 1),
                udp_drops53=drops.get('Drops53'), rcvbuf_errors=drops.get('RcvbufErrors'))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=pathlib.Path)
    parser.add_argument('--json', action='store_true')
    args = parser.parse_args()
    rows = [summarise(d) for d in sorted(args.root.iterdir()) if (d / 'result.json').exists()]
    if args.json:
        print(json.dumps(rows, indent=1))
        return
    for row in rows:
        print(f"{row['run']:<18} {'PASS' if row['passed'] else 'FAIL'} rss_peak={row['rss_peak_mib']:>6} MiB "
              f"drops53={row['udp_drops53']} recovery={row['recovery_s']} statuses={row['statuses']}"
              + (f" failed={row['failed_checks']}" if row['failed_checks'] else ''))


if __name__ == '__main__':
    main()
