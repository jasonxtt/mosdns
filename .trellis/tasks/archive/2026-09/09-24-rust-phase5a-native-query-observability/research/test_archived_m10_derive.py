"""Regressions for the archived-task offline M10 re-derivation.

The task directory moves to ``.trellis/tasks/archive/<month>/`` when the task
closes, so both the current location of these scripts and the path recorded in
the measuring commit have to be resolved explicitly. The measured commits here
are fictional: this module redirects every child ``git`` process into a
throwaway repository whose single commit is the "measuring commit".
"""
import contextlib
import hashlib
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
TASK_DIR = HERE.parent.name
ARCHIVE_MONTH = '2026-09'
PREFLIGHT = 'm10-preflight/identity.json'
MEASURED_HEAD = '0' * 40
TOOLS = {'run-m10-w3.py': '#measured driver\n', 'm10-server-control.py': '#measured control\n',
         'run-m6-w1.py': '#measured base\n', 'm5-remote-tools.py': '#measured remote tools\n',
         'measurement-revision-v10.md': '#measured revision\n'}
CASES = (('domain-hit', 'domain-hit.test.', 'DOMAIN_HIT', ['route-a']),
         ('ip-rule-hit', 'ip-hit.test.', 'IP_RULE_HIT', ['route-b', 'route-a']),
         ('ip-rule-miss', 'ip-miss.test.', 'IP_RULE_MISS', ['route-b', 'route-c']))
ROLES = {'sut': (4100, '4101'), 'fixture': (4200, '4201'),
         'fixture_b': (4300, '4301'), 'fixture_c': (4400, '4401')}
REAL_CHECK_OUTPUT = subprocess.check_output
REAL_RUN = subprocess.run


def load(name, alias):
    spec = importlib.util.spec_from_file_location(alias, HERE / name)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def git(root, *arguments):
    return REAL_CHECK_OUTPUT(['git', *arguments], cwd=str(root), text=True, stderr=subprocess.STDOUT)


def in_history(root, head, path):
    return REAL_RUN(['git', 'cat-file', '-e', f'{head}:{path}'], cwd=str(root),
                    capture_output=True).returncode == 0


@contextlib.contextmanager
def history_in(repository, head, commands=None):
    """Serve the code under test from a fictional measuring commit.

    Commands are recorded as the code under test built them (still naming the
    published head) and only then translated to the throwaway repository.
    """
    def redirect(command, **kwargs):
        if isinstance(command, (list, tuple)) and command and command[0] == 'git':
            if commands is not None:
                commands.append([str(part) for part in command])
            command = [str(part).replace(MEASURED_HEAD, head) for part in command]
            kwargs['cwd'] = str(repository)
        return REAL_CHECK_OUTPUT(command, **kwargs)

    with patch.object(subprocess, 'check_output', side_effect=redirect):
        yield commands


def build_repository(root, omit=(), preflight=None):
    (root / 'rust').mkdir(parents=True)
    (root / 'go.mod').write_text('module mosdns\n')
    (root / 'rust/Cargo.toml').write_text('[package]\n')
    (root / 'tests/phase5a-baseline').mkdir(parents=True)
    research = root / '.trellis/tasks' / TASK_DIR / 'research'
    for name, body in TOOLS.items():
        if name not in omit:
            write(research / name, body)
    if preflight is not None:
        write(research / PREFLIGHT, json.dumps(preflight, indent=2) + '\n')
    return research


def commit(root):
    git(root, 'init', '-q')
    git(root, 'add', '-A')
    git(root, '-c', 'user.email=m10@example.invalid', '-c', 'user.name=m10', 'commit', '-qm', 'measure task')
    return git(root, 'rev-parse', 'HEAD').strip()


def archive_task(root):
    """Move the task directory, leaving the measuring commit on the old path."""
    (root / '.trellis/tasks/archive' / ARCHIVE_MONTH).mkdir(parents=True)
    git(root, 'mv', f'.trellis/tasks/{TASK_DIR}', f'.trellis/tasks/archive/{ARCHIVE_MONTH}/{TASK_DIR}')
    return root / f'.trellis/tasks/archive/{ARCHIVE_MONTH}/{TASK_DIR}/research'


def identity_for(driver, workload):
    return dict(local_tools={name: digest(body) for name, body in TOOLS.items()},
                server_inputs={driver.t.SERVER_INPUT + '/w3-off.yaml': '0' * 64,
                               driver.t.SERVER_INPUT + '/w3-on.yaml': '1' * 64,
                               driver.t.SERVER_INPUT + '/w3-on-b.yaml': '2' * 64,
                               driver.t.SERVER_INPUT + '/repo/tests/phase5a-baseline/workloads/routing.jsonl':
                                   hashlib.sha256(workload.encode()).hexdigest()},
                client_inputs={})


def workload_text():
    return ''.join(json.dumps(dict(case_id=case_id, qname=qname, qtype='A', qclass=1,
                                   expected_route_class=route, weight=1)) + '\n'
                   for case_id, qname, route, _ in CASES)


def session_evidence(session, driver, row, identity, cleanup_error):
    run_id = row['run_id']
    requests = []
    events = []
    # The load is small but the shape is the measured one: one round-robin pass
    # over the three frozen cases, each repeated until the planned count.
    for index in range(1, row['planned'] + 1):
        case_id, qname, _, route = CASES[(index - 1) % len(CASES)]
        requests.append(dict(run_id=run_id, stage_id='normal-reference', request_seq=index, dns_id=900 + index,
                             case_id=case_id, qname=qname, qtype='A', qclass=1, sent=True,
                             outcome='correct_on_time', sent_at='2026-09-26T13:00:00Z',
                             finished_at='2026-09-26T13:00:00.000000500Z'))
        for upstream in route:
            events.append(dict(fixture_seq=len(events) + 1, dns_id=900 + index, qname=qname, qtype=1, qclass=1,
                               upstream=upstream, occurred_at='2026-09-26T13:00:01Z'))
    counters = dict(scheduled=row['planned'], sent=row['planned'], received=row['planned'],
                    correct_on_time=row['planned'], correct_late=0, expected_negative_on_time=0,
                    wrong_response=0, protocol_error=0, transport_error=0, timeout=0, sender_shortfall=0)
    scheduled = {case_id: row['planned'] // len(CASES) for case_id, _, _, _ in CASES}
    stage = dict(stage='normal-reference', run_id=run_id, fixture_session_id=run_id, scenario='w3', transport='udp',
                 target_qps=100, duration_ms=30000, request_deadline_ms=500, late_drain_ms=100,
                 counters=counters, case_scheduled=scheduled,
                 p50_us=600, p95_us=1200, p99_us=2100, fixture_seq_start=0, fixture_seq_end=0,
                 request_seq_start=1, request_seq_end=row['planned'])
    write(session / 'stages.jsonl', json.dumps(stage) + '\n')
    write(session / 'oracles.txt',
          'verify-stage\nfixture event loss in stage window: seq_start=0 seq_end=0 events=0\nexit=0\n'
          'verify-sender\nexit=0\nverify-routing-events\nexit=1\nverify-counters\nexit=0\n')
    if cleanup_error:
        write(session / 'cleanup-error.txt', 'stderr:\n[Errno 3] No such process\n')
    write(session / 'client/requests.jsonl', ''.join(json.dumps(r) + '\n' for r in requests))
    write(session / 'server/routing-events.jsonl', ''.join(json.dumps(e) + '\n' for e in events))
    write(session / 'server/owned.json', json.dumps({role: dict(pid=pid, start_identity=start)
                                                    for role, (pid, start) in ROLES.items()}) + '\n')
    variant = row['variant']
    write(session / 'server/sut.json', json.dumps(dict(
        variant=variant, audit_enabled=variant == 'after_on',
        sha256=driver.t.BASELINE_SHA if variant == 'before_off' else driver.CANDIDATE_SHA,
        config_sha256=identity['server_inputs'][
            driver.t.SERVER_INPUT + '/w3-' + ('on' if variant == 'after_on' else 'off') + '.yaml'])) + '\n')
    for host, hostname in (('client', 'Debian'), ('server', 'mosdns-rust')):
        directory = session / host
        files = {str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest()
                 for path in sorted(directory.rglob('*')) if path.is_file()}
        write(directory / 'source-manifest.json', json.dumps(dict(host=hostname, files=files)) + '\n')


def build_evidence(raw, driver, identity, cleanup_error_run=None):
    raw.mkdir(parents=True)
    plan = driver.plan()
    rows = [dict(slot, runner_exit=1, correct=slot['planned'], shortfall=0, errors=0,
                 p50_us=600, p95_us=1200, p99_us=2100) for slot in plan]
    write(raw / 'rows.json', json.dumps(rows, indent=2) + '\n')
    write(raw / 'identity.json', json.dumps(identity, indent=2) + '\n')
    write(raw / 'assessment.json', json.dumps(dict(passed=False, failures=['legacy route oracle failed'])) + '\n')
    write(raw / 'source-head.txt', MEASURED_HEAD + '\n')
    receipts = []
    for row in plan:
        session_evidence(raw / row['run_id'], driver, row, identity, row['run_id'] == cleanup_error_run)
        for role, (pid, start) in ROLES.items():
            receipts.append(dict(run_id=row['run_id'], role=role, pid=pid, start_identity=start,
                                 owned_process_active=False))
    write(raw / 'postbatch-proof/cleanup-verification.json', json.dumps(receipts, indent=2) + '\n')
    write(raw / 'postbatch-proof/identity.json', json.dumps(identity, indent=2) + '\n')


class ArchivedCommitPathTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.driver = load('run-m10-w3.py', 'm10_driver')

    def build(self, omit=(), preflight='{}'):
        research = build_repository(self.root, omit=omit, preflight=preflight)
        head = commit(self.root)
        return research, head

    def archive(self):
        research, head = self.build()
        self.assertFalse((self.root / f'.trellis/tasks/archive/{ARCHIVE_MONTH}').exists())
        archived = archive_task(self.root)
        self.assertFalse(research.exists())
        self.assertNotEqual(HERE, archived)
        return head, archived

    def test_committed_research_directory_is_resolved_where_that_commit_kept_it(self):
        head, archived = self.archive()
        self.assertTrue(in_history(self.root, head, f'.trellis/tasks/{TASK_DIR}/research/run-m10-w3.py'))
        self.assertFalse(in_history(self.root, head, str(archived.relative_to(self.root)) + '/run-m10-w3.py'))
        with patch.object(self.driver, 'HERE', archived), history_in(self.root, head):
            resolved = self.driver.committed_research_root(MEASURED_HEAD)
        self.assertEqual(resolved, Path(f'.trellis/tasks/{TASK_DIR}/research'))
        self.assertNotIn('archive', str(resolved))

    def test_ambiguous_incomplete_or_unknown_committed_task_directory_is_rejected(self):
        _, head = self.build()
        with history_in(self.root, head):
            self.assertEqual(self.driver.committed_research_root(MEASURED_HEAD),
                             Path(f'.trellis/tasks/{TASK_DIR}/research'))
            with self.assertRaisesRegex(ValueError, 'exactly one'):
                self.driver.committed_research_root(MEASURED_HEAD + '0123456789abcdef')
        duplicate = self.root / f'.trellis/tasks/archive/2026-08/{TASK_DIR}/research'
        duplicate.mkdir(parents=True)
        for name, body in TOOLS.items():
            write(duplicate / name, body)
        write(duplicate / PREFLIGHT, '{}\n')
        ambiguous = commit(self.root)
        with history_in(self.root, ambiguous):
            with self.assertRaisesRegex(ValueError, 'exactly one'):
                self.driver.committed_research_root(MEASURED_HEAD)
        with tempfile.TemporaryDirectory() as temporary:
            sparse = Path(temporary) / 'repo'
            build_repository(sparse, omit=(PREFLIGHT,))
            sparse_head = commit(sparse)
            with history_in(sparse, sparse_head):
                with self.assertRaisesRegex(ValueError, 'exactly one'):
                    self.driver.committed_research_root(MEASURED_HEAD)

    def test_committed_bytes_report_a_missing_history_path_instead_of_crashing(self):
        self.build()
        head = git(self.root, 'rev-parse', 'HEAD').strip()
        with history_in(self.root, head):
            present = self.driver.committed_bytes(MEASURED_HEAD, f'.trellis/tasks/{TASK_DIR}/research/run-m10-w3.py')
            self.assertEqual(present.decode(), TOOLS['run-m10-w3.py'])
            for path in (f'.trellis/tasks/archive/{ARCHIVE_MONTH}/{TASK_DIR}/research/run-m10-w3.py',
                         f'.trellis/tasks/{TASK_DIR}/research/absent.py'):
                with self.subTest(path=path), self.assertRaisesRegex(ValueError, 'missing in'):
                    self.driver.committed_bytes(MEASURED_HEAD, path)


class ArchivedDeriveTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.driver = load('run-m10-w3.py', 'm10_driver')
        self.oracle = load('m10-route-oracle.py', 'm10_oracle')

    def prepare(self, omit=(), duplicate=False, corrupt_tool=None):
        workload = workload_text()
        identity = identity_for(self.driver, workload)
        research = build_repository(self.root, omit=omit, preflight=identity)
        if duplicate:
            for path in sorted(research.rglob('*')):
                if path.is_file():
                    write(self.root / f'.trellis/tasks/archive/2026-08/{TASK_DIR}/research' /
                          path.relative_to(research), path.read_text())
        measured = commit(self.root)
        if corrupt_tool is not None:
            # Only the committed blob changes: the frozen preflight identity and
            # both raw identity copies keep the digest of the measured bytes, so
            # this can only be caught by hashing the historical Git object.
            write(research / corrupt_tool, '#tampered in the measuring commit\n')
            measured = commit(self.root)
        archived = archive_task(self.root)
        raw = self.root / 'raw'
        workload_path = self.root / 'routing.jsonl'
        build_evidence(raw, self.driver, identity, cleanup_error_run='m10-w3-r1-after_off')
        write(workload_path, workload)
        return dict(raw=raw, published=MEASURED_HEAD, measured=measured, workload=workload_path,
                    out=self.root / 'derived', identity=identity, archived=archived.relative_to(self.root))

    def derive(self, prepared, commands=None):
        with history_in(self.root, prepared['measured'], commands):
            return self.oracle.derive(str(prepared['raw']), str(prepared['out']), str(prepared['workload']),
                                      str(prepared['raw'] / 'postbatch-proof/cleanup-verification.json'),
                                      str(prepared['raw'] / 'postbatch-proof/identity.json'))

    def assert_history_is_measured(self, prepared, commands):
        """Every history read names the measuring commit at its measured path."""
        history = [command for command in commands if command[1:2] == ['show']]
        self.assertTrue(history)
        for command in history:
            self.assertTrue(command[2].startswith(
                f'{prepared["published"]}:.trellis/tasks/{TASK_DIR}/research/'), command)
            self.assertNotIn('archive', command[2])

    def test_full_derive_recomputes_the_proof_from_the_archived_checkout(self):
        prepared = self.prepare()
        commands = []
        assessment = self.derive(prepared, commands)
        self.assertTrue(assessment['passed'], assessment['failures'])
        self.assertEqual(len(assessment['rows']), 9)
        rows = json.loads((prepared['out'] / 'rows.json').read_text())
        self.assertTrue(all(row['offline_route_verified'] and row['runner_exit'] == 0
                            and row['original_runner_exit'] == 1 for row in rows))
        self.assertTrue(any(row['cleanup_exit_receipt_used'] for row in rows))
        planned = sum(slot['planned'] for slot in self.driver.plan())
        proofs = json.loads((prepared['out'] / 'route-proofs.json').read_text())
        self.assertEqual(sum(proof['requests_verified'] for proof in proofs.values()), planned)
        self.assertEqual(sum(proof['events_verified'] for proof in proofs.values()), int(planned * 5 / 3))
        provenance = json.loads((prepared['out'] / 'provenance.json').read_text())
        self.assertEqual(provenance['new_queries'], 0)
        self.assertEqual(provenance['cleanup_exit_receipts'], 1)
        self.assertTrue(provenance['original_verdict_unchanged'])
        self.assert_history_is_measured(prepared, commands)

    def test_derive_refuses_missing_ambiguous_or_unhashed_history(self):
        scenarios = (('report_hash', {}), ('deleted_tool', dict(omit=('m5-remote-tools.py',))),
                     ('ambiguous_task', dict(duplicate=True)))
        for change, options in scenarios:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                self.root = Path(temporary) / 'repo'
                prepared = self.prepare(**options)
                if change == 'report_hash':
                    identity = dict(prepared['identity'], local_tools=dict(
                        prepared['identity']['local_tools'], **{'run-m10-w3.py': 'f' * 64}))
                    write(prepared['raw'] / 'identity.json', json.dumps(identity, indent=2) + '\n')
                    write(prepared['raw'] / 'postbatch-proof/identity.json', json.dumps(identity, indent=2) + '\n')
                self.assertTrue((self.root / prepared['archived']).is_dir())
                with self.assertRaises(ValueError):
                    self.derive(prepared)

    def test_derive_rejects_a_tampered_historical_tool_object(self):
        for tool in ('m10-server-control.py', 'm5-remote-tools.py'):
            with self.subTest(tool=tool), tempfile.TemporaryDirectory() as temporary:
                self.root = Path(temporary) / 'repo'
                prepared = self.prepare(corrupt_tool=tool)
                # The identity chain still agrees with itself, so the only thing
                # left to reject the run is the historical object's own hash.
                self.assertIn(tool, prepared['identity']['local_tools'])
                self.assertNotEqual(hashlib.sha256(
                    (self.root / prepared['archived'] / tool).read_bytes()).hexdigest(),
                    prepared['identity']['local_tools'][tool])
                for name in ('identity.json', 'postbatch-proof/identity.json'):
                    self.assertEqual(json.loads((prepared['raw'] / name).read_text()), prepared['identity'])
                with self.assertRaisesRegex(ValueError, 'executed tool differs from reviewed preflight'):
                    self.derive(prepared)

    def test_current_copy_repair_does_not_replace_or_shadow_measured_history(self):
        prepared = self.prepare()
        # An offline-analysis repair, and a hostile copy where the task used to
        # live: both are in the worktree, neither may be read as measured bytes.
        write(self.root / prepared['archived'] / 'm10-server-control.py', '#repaired offline analysis\n')
        write(self.root / f'.trellis/tasks/{TASK_DIR}/research/run-m10-w3.py', '#hostile replacement\n')
        commands = []
        assessment = self.derive(prepared, commands)
        self.assertTrue(assessment['passed'], assessment['failures'])
        self.assert_history_is_measured(prepared, commands)


if __name__ == '__main__':
    unittest.main()
