import importlib.util
import tempfile
import unittest
from pathlib import Path

import repo_paths

HERE = Path(__file__).resolve().parent
MARKERS = ('go.mod', 'rust/Cargo.toml', 'tests/phase5a-baseline')


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_').replace('.py', ''), HERE / name)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ArchivedTaskRootTests(unittest.TestCase):
    def test_archived_research_directory_still_locates_the_repository(self):
        root = repo_paths.repo_root()
        self.assertEqual(root, repo_paths.repo_root(HERE))
        self.assertEqual(root, repo_paths.repo_root(HERE / 'test_archived_task_paths.py'))
        self.assertNotEqual(root, HERE)
        # No fixed depth: the markers are what identify the repository, so this
        # holds wherever the task directory sits.
        self.assertIn(str(root), str(HERE))
        for marker in MARKERS:
            self.assertTrue((root / marker).exists(), marker)

    def test_repository_without_markers_is_rejected_instead_of_silently_guessed(self):
        with tempfile.TemporaryDirectory() as temporary:
            nested = Path(temporary) / 'not-a-repository/deeper'
            nested.mkdir(parents=True)
            for start in (temporary, nested):
                with self.assertRaisesRegex(ValueError, 'repository root'):
                    repo_paths.repo_root(start)
        for missing in (MARKERS[:1], MARKERS[1:]):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for marker in MARKERS:
                    if marker in missing:
                        continue
                    (root / marker).mkdir(parents=True)
                with self.assertRaisesRegex(ValueError, 'repository root'):
                    repo_paths.repo_root(root)

    def test_frozen_measurement_drivers_agree_with_the_marker_search(self):
        root = repo_paths.repo_root()
        drivers = {name: load(name) for name in ('run-m5-w1.py', 'run-m6-w1.py')}
        for name, driver in drivers.items():
            with self.subTest(driver=name):
                self.assertEqual(driver.REPO, root)
                self.assertTrue((driver.REPO / 'go.mod').exists())

    def test_history_reads_use_the_marker_search_not_the_frozen_driver_fallback(self):
        """The M10 history helper must not inherit the frozen drivers' fallback.

        ``run-m5-w1.py`` and ``run-m6-w1.py`` fall back to ``HERE/'repo'``,
        which does not exist here, so using their ``REPO`` would fail or
        silently address a tree that has none of the repository markers. The
        module-level assertion pins the source; the sandboxed one is asserted
        against a copy of the repository moved off the marker path.
        """
        source = (HERE / 'run-m10-w3.py').read_text()
        self.assertIn('REPO=repo_paths.repo_root(HERE)', source)
        self.assertNotIn('t.REPO', source.split('def committed_research_root')[1].split('def verify_reviewed_tools')[0])
        driver = load('run-m10-w3.py')
        self.assertEqual(driver.REPO, repo_paths.repo_root())
        with tempfile.TemporaryDirectory() as temporary:
            relocated = Path(temporary) / 'moved-repository'
            for marker in MARKERS:
                (relocated / marker).parent.mkdir(parents=True, exist_ok=True)
                (relocated / marker).mkdir(exist_ok=True)
            fallback = next((parent for parent in
                             (relocated / '.trellis/tasks' / driver.TASK_DIR / 'research').parents
                             if (parent / 'go.mod').exists()), relocated / 'repo')
            self.assertEqual(fallback, relocated)
            self.assertTrue(all((fallback / marker).exists() for marker in MARKERS))
            elsewhere = Path(temporary) / 'plain-checkout'
            elsewhere.mkdir()
            with self.assertRaisesRegex(ValueError, 'repository root'):
                repo_paths.repo_root(elsewhere)

    def test_paths_used_by_the_suites_resolve_from_the_archived_directory(self):
        root = repo_paths.repo_root()
        self.assertTrue((root / 'scripts/run-phase5a-baseline.sh').is_file())
        self.assertTrue((root / 'tests/phase5a-baseline/cmd/phase5a-baseline/main.go').is_file())


if __name__ == '__main__':
    unittest.main()
