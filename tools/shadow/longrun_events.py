"""Once-only, reversible rtdns event scripts. Never contacts the Go host."""
import argparse
import functools
import hashlib
import http.server
import json
import os
import pathlib
import shutil
import subprocess
import threading
import time
import urllib.error
import urllib.request

from longrun_store import atomic
from unattended import load

BASE = 'http://127.0.0.1'
DROP = pathlib.Path('/etc/systemd/system/mosdns.service.d/90-shadow1-48h-update.conf')


def request(path, value=None, raw=False, timeout=10):
    req = urllib.request.Request(BASE + path, data=None if value is None else json.dumps(value).encode(),
                                 headers={'Content-Type': 'application/json'})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(req, timeout=timeout) as response:
        body = response.read()
        return {'http_status': response.status, 'body': body.decode() if raw else json.loads(body)}


def service():
    return dict(line.split('=', 1) for line in subprocess.check_output(['systemctl', 'show', 'mosdns.service',
        '-p', 'MainPID', '-p', 'NRestarts', '-p', 'ActiveState'], text=True).splitlines())


def wait_ready(version, timeout=25):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            h = request('/api/v1/system/health')['body']
            if h.get('ready') and h.get('version') == version and h.get('update', {}).get('state') == 'idle':
                return h
        except (OSError, ValueError):
            pass
        time.sleep(.2)
    raise RuntimeError('ready/version/idle timeout')


def restart(root, event, version, record):
    now = time.time()
    intent = {'event': event, 'issued': now, 'until': now + 30, 'before': service()}
    atomic(root / 'restart-intent.json', intent)
    begin = time.monotonic()
    subprocess.run(['systemctl', 'restart', 'mosdns.service'], check=True, timeout=25)
    wait_ready(version)
    after = service()
    intent['after_pid'] = after['MainPID']
    # A fast restart may finish between sampler ticks. Keep this exact PID
    # authorization for its original bounded interval, then let it expire.
    atomic(root / 'restart-intent.json', intent)
    record.setdefault('planned_restarts', []).append({'wall': now, 'seconds': time.monotonic() - begin,
                                                     'after': after})


def hot_setting(root, record):
    before = request('/api/v1/overrides')['body']
    changed = {**before, 'ecs': '192.0.2.77' if before.get('ecs') != '192.0.2.77' else '198.51.100.77'}
    atomic(root / 'events/1-backup.private.json', before)
    try:
        record['apply'] = request('/api/v1/overrides', changed)
        assert request('/api/v1/overrides')['body']['ecs'] == changed['ecs']
        record['applied_wall'] = time.time()
        time.sleep(10)
    finally:
        record['revert'] = request('/api/v1/overrides', before)
        assert request('/api/v1/overrides')['body'] == before, 'typed setting revert mismatch'
        record['reverted_wall'] = time.time()


def local_rule(root, record):
    runtime = load(root / 'runtime.json')
    tag = runtime['event_rule_tag']
    path = '/plugins/' + tag
    original = request(path + '/show', raw=True)['body']
    before = original.splitlines()
    rule = 'full:shadow1-event.example 192.0.2.123'
    assert rule not in before
    atomic(root / 'events/2-backup.private.json', {'tag': tag, 'values': before})
    try:
        record['apply'] = request(path + '/post', {'values': before + [rule]}, raw=True)
        assert rule in request(path + '/show', raw=True)['body'].splitlines()
        record['applied_wall'] = time.time()
        time.sleep(10)
    finally:
        record['revert'] = request(path + '/post', {'values': before}, raw=True)
        assert request(path + '/show', raw=True)['body'].splitlines() == before, 'local rule revert mismatch'
        record['reverted_wall'] = time.time()


def restore_before_update(root, record):
    metadata = load(root / 'events/3-before-update.private.json')
    now = time.time()
    atomic(root / 'restart-intent.json', {'event': 3, 'issued': now, 'until': now + 30})
    try:
        subprocess.run(['systemctl', 'stop', 'mosdns.service'], check=True, timeout=25)
        target = pathlib.Path('/usr/local/bin/mosdns')
        temp = target.with_name('mosdns.shadow1-48h-restore')
        shutil.copy2(root / 'events/3-before-mosdns.private', temp)
        temp.replace(target)
        installed = pathlib.Path('/cus/mosdns/update/installed.json')
        if metadata['installed_existed']:
            shutil.copy2(root / 'events/3-before-installed.private.json', installed)
        else:
            installed.unlink(missing_ok=True)
        subprocess.run(['systemctl', 'start', 'mosdns.service'], check=True, timeout=25)
        wait_ready(metadata['version'])
        record['failure_restored_previous_binary'] = True
    finally:
        intent = load(root / 'restart-intent.json', {})
        intent['after_pid'] = service()['MainPID']
        atomic(root / 'restart-intent.json', intent)


def update(root, record):
    release = root / 'releases'
    version = 'rust-v0.1.3'
    expected = load(release / 'mosdns.manifest.json')['artifact']['sha256']
    assert hashlib.sha256((release / 'mosdns').read_bytes()).hexdigest() == expected
    assert not DROP.exists(), 'refuse to replace an existing override'
    before_version = request('/api/v1/system/health')['body']['version']
    assert before_version != version, 'event must install a newer candidate'
    backup = root / 'events/3-before-mosdns.private'
    shutil.copy2('/usr/local/bin/mosdns', backup)
    installed = pathlib.Path('/cus/mosdns/update/installed.json')
    previous_installed = installed.read_bytes() if installed.exists() else None
    if previous_installed is not None:
        (root / 'events/3-before-installed.private.json').write_bytes(previous_installed)
    atomic(root / 'events/3-before-update.private.json', {'version': before_version,
                                                       'installed_existed': previous_installed is not None})
    listing = [{'tag_name': version, 'draft': False, 'prerelease': False, 'published_at': '2026-10-19T10:00:00Z',
                'assets': [{'name': name, 'browser_download_url': 'http://127.0.0.1:18081/releases/' + name}
                           for name in ['mosdns-rust-linux-amd64.tar.gz', 'mosdns-rust-update-manifest.json']]}]
    atomic(release / 'releases.json', listing)
    class Handler(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *_):
            pass
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 18081), functools.partial(Handler, directory=str(release)))
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    confirmed = False
    try:
        DROP.parent.mkdir(exist_ok=True)
        DROP.write_text('[Service]\nEnvironment="MOSDNS_UPDATE_API_URL=http://127.0.0.1:18081/releases.json"\n')
        subprocess.run(['systemctl', 'daemon-reload'], check=True)
        restart(root, 3, before_version, record)
        record['check'] = request('/api/v1/update/check', {}, timeout=30)
        assert record['check']['body']['latest_version'] == version and record['check']['body']['update_available']
        start = time.monotonic()
        record['apply_started_wall'] = time.time()
        record['apply'] = request('/api/v1/update/apply', {}, timeout=60)
        record['apply_http_seconds'] = time.monotonic() - start
        assert record['apply']['body']['restart_required']
        observations = record['observations'] = []
        while time.monotonic() - start < 150:
            item = {'wall': time.time(), 'elapsed': time.monotonic() - start}
            try:
                health = request('/api/v1/system/health', timeout=2)['body']
                item.update(ready=health.get('ready'), version=health.get('version'), update=health.get('update'))
                if health.get('ready') and health.get('version') == version:
                    record.setdefault('first_candidate_ready_seconds', item['elapsed'])
                    if health.get('update', {}).get('state') == 'idle':
                        assert hashlib.sha256(pathlib.Path('/usr/local/bin/mosdns').read_bytes()).hexdigest() == expected
                        record['confirmation_seconds'] = time.monotonic() - start
                        confirmed = True
                        observations.append(item)
                        break
                if health.get('update', {}).get('state') == 'rolled_back':
                    raise RuntimeError('F-3a guard rolled back candidate')
            except (OSError, ValueError) as error:
                item['read_error'] = type(error).__name__
            observations.append(item)
            time.sleep(.2)
        assert confirmed, 'F-3a confirmation timeout'
        record['installed_sha256'] = expected
    finally:
        # Stop the temporary server even if restoring the service fails.
        try:
            if DROP.exists():
                DROP.unlink()
                subprocess.run(['systemctl', 'daemon-reload'], check=True)
                if not confirmed:
                    restore_before_update(root, record)
                else:
                    restart(root, 3, version, record)
                environment = subprocess.check_output(['systemctl', 'show', 'mosdns.service', '-p', 'Environment'], text=True)
                assert 'MOSDNS_UPDATE_API_URL' not in environment
                record['temporary_environment_removed'] = True
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
            record['release_server_stopped'] = not thread.is_alive()


def main(root, number):
    os.umask(0o077)
    directory = root / 'events'
    directory.mkdir(exist_ok=True)
    claim = directory / f'{number}.claim'
    try:
        with claim.open('x') as stream:
            stream.write(str(time.time()) + '\n')
            stream.flush()
            os.fsync(stream.fileno())
    except FileExistsError:
        recover(root, number)
        return
    record = {'event': number, 'started_wall': time.time(), 'before_service': service(), 'success': False}
    try:
        manifest = load(root / 'run.json')
        if (root / 'stop-status.json').exists() or time.time() >= manifest['end_wall']:
            record['skipped'] = 'run stopped or deadline passed'
            return
        assert request('/api/v1/system/health')['body']['ready']
        if number == 1:
            hot_setting(root, record)
        elif number == 2:
            local_rule(root, record)
        elif number == 3:
            update(root, record)
        elif number == 4:
            restart(root, 4, request('/api/v1/system/health')['body']['version'], record)
        record['success'] = True
    except Exception as error:
        record['error'] = type(error).__name__ + ': ' + str(error)
    finally:
        record.update(ended_wall=time.time(), after_service=service())
        record['effect_window'] = {'start_wall': record['ended_wall'], 'end_wall': record['ended_wall'] + 600}
        atomic(directory / f'{number}.json', record)


def recover(root, number):
    """A killed event is never applied twice; only its retained revert may run."""
    directory = root / 'events'
    if not (directory / f'{number}.claim').exists():
        return
    record = load(directory / f'{number}.json', {'event': number,
        'started_wall': float((directory / f'{number}.claim').read_text()), 'success': False,
        'error': 'event process interrupted; recovery only'})
    if record.get('success') or record.get('reverted_wall'):
        return
    try:
        if number == 1 and (directory / '1-backup.private.json').exists():
            before = load(directory / '1-backup.private.json')
            record['recovery_revert'] = request('/api/v1/overrides', before)
            assert request('/api/v1/overrides')['body'] == before
        elif number == 2 and (directory / '2-backup.private.json').exists():
            before = load(directory / '2-backup.private.json')
            path = '/plugins/' + before['tag']
            record['recovery_revert'] = request(path + '/post', {'values': before['values']}, raw=True)
            assert request(path + '/show', raw=True)['body'].splitlines() == before['values']
        elif number == 3:
            if DROP.exists():
                DROP.unlink()
                subprocess.run(['systemctl', 'daemon-reload'], check=True)
                record['temporary_environment_removed'] = True
            if (directory / '3-before-update.private.json').exists():
                restore_before_update(root, record)
        record['reverted_wall'] = time.time()
    except Exception as error:
        record['recovery_error'] = type(error).__name__ + ': ' + str(error)
        raise
    finally:
        record['ended_wall'] = time.time()
        record['effect_window'] = {'start_wall': record['ended_wall'], 'end_wall': record['ended_wall'] + 600}
        atomic(directory / f'{number}.json', record)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('root', type=pathlib.Path)
    parser.add_argument('event', type=int, choices=(1, 2, 3, 4))
    parser.add_argument('--recover', action='store_true')
    args = parser.parse_args()
    (recover if args.recover else main)(args.root, args.event)
