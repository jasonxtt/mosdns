"""Appearance and WebUI-port interop between the reference handlers and the native host.

    python3 tools/f1-interop/interop.py --native /path/mosdns --reference /path/f1-interop --work /tmp/dir

The same request sequence is applied to two fresh directories: one through the
reference handlers (`f1-interop`), one through the native host over HTTP.
Responses and the resulting state files must agree (upload ids, version stamps and
timestamps are normalised). Then each side reads the directory the *other* side
wrote and must answer exactly as the writer did. Only synthetic data is used.
"""
import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

PNG = b'\x89PNG\r\n\x1a\n' + bytes(range(256)) * 20
GIF = b'GIF89a' + b'\x01\x02\x03' * 300
JPG = b'\xff\xd8\xff\xe0' + b'\x10\x20' * 500
TEXT = b'this is not an image\n'
EMPTY = b''

STEPS = [
    ('GET', '/api/v1/appearance/panel-background', None, None),
    ('GET', '/api/v1/appearance/text-color', None, None),
    ('GET', '/api/v1/appearance/button-color', None, None),
    ('POST', '/api/v1/appearance/panel-background', {'mode': 'color', 'color': '#ABCDEF', 'opacity': 0.5, 'blur': 7}, None),
    ('POST', '/api/v1/appearance/panel-background', {'mode': 'url', 'url': '  https://example.test/x.png  ', 'light_color': '12', 'opacity': 7, 'blur': -3}, None),
    ('POST', '/api/v1/appearance/panel-background', {'mode': 'url', 'url': ' '}, None),
    ('POST', '/api/v1/appearance/panel-background', {'mode': 'upload'}, None),
    ('POST', '/api/v1/appearance/panel-background', {'mode': 'upload', 'upload_id': 'nothere1'}, None),
    ('POST', '/api/v1/appearance/panel-background', {'mode': 'weird', 'dark_color': '#00FF00'}, None),
    ('POST', '/api/v1/appearance/text-color', {'light': {'mode': 'custom', 'color': '#ABCDEF'}, 'dark': {'mode': 'custom', 'color': 'zzz'}}, None),
    ('POST', '/api/v1/appearance/button-color', {'light': {'mode': 'Custom', 'color': '#111'}, 'dark': {'mode': 'default', 'color': '#123456'}}, None),
    ('GET', '/api/v1/appearance/text-color', None, None),
    ('GET', '/api/v1/appearance/button-color', None, None),
    ('GET', '/api/v1/appearance/panel-background/history', None, None),
    ('GET', '/api/v1/appearance/panel-background/image', None, None),
    ('UPLOAD', '/api/v1/appearance/panel-background/upload', None, PNG),
    ('UPLOAD', '/api/v1/appearance/panel-background/upload', None, GIF),
    ('UPLOAD', '/api/v1/appearance/panel-background/upload', None, TEXT),
    ('UPLOAD', '/api/v1/appearance/panel-background/upload', None, EMPTY),
    ('UPLOAD', '/api/v1/appearance/panel-background/upload', None, JPG),
    ('GET', '/api/v1/appearance/panel-background/history', None, None),
    ('GET', '/api/v1/appearance/panel-background/image', None, None),
    ('POST', '/api/v1/appearance/panel-background', {'mode': 'upload', 'upload_id': '@1', 'opacity': 0.3, 'blur': 12}, None),
    ('GET', '/api/v1/appearance/panel-background', None, None),
    ('GET', '/api/v1/appearance/panel-background/history/@1', None, None),
    ('GET', '/api/v1/appearance/panel-background/history/abc', None, None),
    ('DELETE', '/api/v1/appearance/panel-background/history/@1', None, None),
    ('GET', '/api/v1/appearance/panel-background', None, None),
    ('DELETE', '/api/v1/appearance/panel-background/history/@1', None, None),
    ('POST', '/api/v1/appearance/panel-background', {'mode': 'upload'}, None),
    ('DELETE', '/api/v1/appearance/panel-background/history', None, None),
    ('GET', '/api/v1/appearance/panel-background', None, None),
    ('GET', '/api/v1/appearance/panel-background/history', None, None),
    ('DELETE', '/api/v1/appearance/panel-background/history', None, None),
]
STATE_FILES = ['webinfo/appearance_settings.json', 'webinfo/appearance_text_settings.json', 'webinfo/appearance_button_settings.json']


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


class Reference:
    def __init__(self, binary, root):
        self.binary, self.root = binary, root

    def request(self, method, path, body=None, upload=None):
        argv = [self.binary, '-root', str(self.root), '-method', method, '-path', path]
        if body is not None:
            argv += ['-body', json.dumps(body)]
        if upload is not None:
            tmp = self.root / 'upload.tmp'
            tmp.write_bytes(upload)
            argv += ['-upload', str(tmp)]
        out = subprocess.run(argv, capture_output=True, check=True).stdout
        status, _, rest = out.partition(b'\n')
        return int(status), rest


class Native:
    def __init__(self, binary, root):
        self.root = root
        api, dns = free_port(), free_port()
        (root / 'config.yaml').write_text(
            f"log: {{level: error}}\napi: {{http: '127.0.0.1:{api}'}}\nplugins:\n  - tag: up\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:9'}}]}}\n  - tag: main\n    type: sequence\n    args: [{{exec: $up}}]\n  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:{dns}', entry: main, enable_audit: false}}\n")
        self.api = api
        self.process = subprocess.Popen([binary, 'start', '-d', str(root), '-c', str(root / 'config.yaml')],
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(200):
            try:
                urllib.request.urlopen(f'http://127.0.0.1:{api}/api/v1/system/health', timeout=1).read()
                return
            except Exception:
                time.sleep(0.05)
        raise RuntimeError('native host did not start')

    def request(self, method, path, body=None, upload=None):
        data, headers = None, {}
        if upload is not None:
            boundary = '----interop'
            data = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="u.bin"\r\n'
                    'Content-Type: application/octet-stream\r\n\r\n').encode() + upload + f'\r\n--{boundary}--\r\n'.encode()
            headers['Content-Type'] = f'multipart/form-data; boundary={boundary}'
        elif body is not None:
            data = json.dumps(body).encode()
        request = urllib.request.Request(f'http://127.0.0.1:{self.api}{path}', data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                return response.status, self.flatten(response)
        except urllib.error.HTTPError as error:
            return error.code, self.flatten(error)

    @staticmethod
    def flatten(response):
        body = response.read()
        content_type = response.headers.get('Content-Type', '')
        if content_type.startswith('image/') or content_type == 'application/octet-stream':
            return f"{content_type} {response.headers.get('Cache-Control', '')} sha256={hashlib.sha256(body).hexdigest()}\n".encode()
        return body

    def stop(self):
        self.process.terminate()
        self.process.wait(timeout=20)


def normalise(text, ids):
    """Upload ids, version stamps and timestamps differ between runs; everything else must not."""
    text = re.sub(r'\?v=\d+', '?v=<V>', text)
    text = re.sub(r'"modified_at":"[^"]+"', '"modified_at":"<T>"', text)
    for index, uid in enumerate(ids, 1):
        text = text.replace(uid, f'<ID{index}>')
    return text


def run_sequence(client, label, log):
    ids, results = [], []
    for method, path, body, upload in STEPS:
        real = 'UPLOAD' == method
        if '@1' in path:
            path = path.replace('@1', ids[0] if ids else 'missing')
        if isinstance(body, dict) and body.get('upload_id') == '@1':
            body = dict(body, upload_id=ids[0] if ids else 'missing')
        status, text = client.request('POST' if real else method, path, body, upload if real else None)
        text = text.decode('utf-8', 'replace')
        if real and status == 200:
            ids.append(json.loads(text)['upload_id'])
        results.append((method, path, status, text))
    log(f'{label}: {len(results)} requests, {len(ids)} uploads')
    return ids, results


def state_snapshot(root, ids):
    files = {name: (root / name).read_text() if (root / name).exists() else None for name in STATE_FILES}
    history = sorted(
        (normalise(p.name, ids), p.stat().st_size, hashlib.sha256(p.read_bytes()).hexdigest())
        for p in (root / 'webinfo/panel_background_history').glob('*')) if (root / 'webinfo/panel_background_history').exists() else []
    image = (root / 'webinfo/panel_background.bin')
    return files, history, hashlib.sha256(image.read_bytes()).hexdigest() if image.exists() else None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--native', required=True)
    parser.add_argument('--reference', required=True)
    parser.add_argument('--work', type=pathlib.Path, required=True)
    args = parser.parse_args()
    shutil.rmtree(args.work, ignore_errors=True)
    ref_root, nat_root = args.work / 'reference', args.work / 'native'
    for root in (ref_root, nat_root):
        (root / 'webinfo').mkdir(parents=True)
    problems = []
    log = lambda message: print(message, flush=True)
    reference_client = Reference(args.reference, ref_root)
    native = Native(args.native, nat_root)
    try:
        ref_ids, ref_results = run_sequence(reference_client, 'reference', log)
        nat_ids, nat_results = run_sequence(native, 'native', log)
        for a, b in zip(ref_results, nat_results):
            same = (a[2], normalise(a[3], ref_ids)) == (b[2], normalise(b[3], nat_ids))
            if not same:
                problems.append(f'{a[0]} {normalise(a[1], ref_ids)}: reference {a[2]} {normalise(a[3], ref_ids)!r} != native {b[2]} {normalise(b[3], nat_ids)!r}')
        # Final states after the same sequence: identical files.
        if state_snapshot(ref_root, ref_ids) != state_snapshot(nat_root, nat_ids):
            problems.append(f'state differs: {state_snapshot(ref_root, ref_ids)} vs {state_snapshot(nat_root, nat_ids)}')
        # Cross reads on directories left in a rich state: re-run the uploads and settings, then each side reads the other's files.
        for root in (ref_root, nat_root):
            shutil.rmtree(root / 'webinfo' / 'panel_background_history', ignore_errors=True)
        ref_ids, _ = run_sequence(reference_client, 'reference (second)', log)
        nat_ids, _ = run_sequence(native, 'native (second)', log)
        for method, path in [('GET', '/api/v1/appearance/panel-background'), ('GET', '/api/v1/appearance/text-color'),
                             ('GET', '/api/v1/appearance/button-color'), ('GET', '/api/v1/appearance/panel-background/history'),
                             ('GET', '/api/v1/appearance/panel-background/image')]:
            # The reference reads the directory the native host wrote ...
            written_by_native = Reference(args.reference, nat_root).request(method, path)
            seen_by_native = native.request(method, path)
            if (written_by_native[0], written_by_native[1].decode()) != (seen_by_native[0], seen_by_native[1].decode()):
                problems.append(f'reference reading native files differs for {path}: {written_by_native} vs {seen_by_native}')
        # ... and the native host reads the directory the reference wrote: start it on that directory.
        native.stop()
        native = Native(args.native, ref_root)
        for method, path in [('GET', '/api/v1/appearance/panel-background'), ('GET', '/api/v1/appearance/text-color'),
                             ('GET', '/api/v1/appearance/button-color'), ('GET', '/api/v1/appearance/panel-background/history'),
                             ('GET', '/api/v1/appearance/panel-background/image')]:
            by_reference = reference_client.request(method, path)
            by_native = native.request(method, path)
            if (by_reference[0], by_reference[1].decode()) != (by_native[0], by_native[1].decode()):
                problems.append(f'native reading reference files differs for {path}: {by_reference} vs {by_native}')
        # WebUI port: the file the native host saves is what the reference resolves at its start.
        port = free_port()
        status, text = native.request('POST', '/api/v1/system/webui-port', {'port': port})
        if status != 200:
            problems.append(f'webui-port save failed: {status} {text!r}')
        resolved = subprocess.run([args.reference, '-root', str(ref_root), '-webui-port-default', '9099'], capture_output=True, check=True).stdout.strip()
        if resolved != str(port).encode():
            problems.append(f'reference resolves port {resolved!r}, native saved {port}')
        # And the reverse: a reference-style file is read by the native host.
        (ref_root / 'webinfo/webui_port_settings.json').write_text('{\n  "port": 8123\n}')
        status, text = native.request('GET', '/api/v1/system/webui-port')
        if json.loads(text)['port'] != 8123:
            problems.append(f'native does not read the reference port file: {text!r}')
    finally:
        native.stop()
    print('INTEROP OK' if not problems else 'INTEROP FAILED')
    for problem in problems:
        print(' -', problem)
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
