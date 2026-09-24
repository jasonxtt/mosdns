import csv
import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("prepare-official-analysis-overlay-v2.py")
SPEC = importlib.util.spec_from_file_location("phase5a_analysis_overlay", SCRIPT)
OVERLAY = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(OVERLAY)


class PrepareOfficialAnalysisOverlayTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.manifest = self.root / "official-manifest-v2.json"
        self.driver = self.root / "run-official-matrix-v2.sh"
        self.status = self.root / "attempt-status.tsv"
        self.results = self.root / "results"
        self.overlay = self.root / "analysis-overlay"
        self.status_map = self.root / "attempt-exit-status-map.tsv"
        self.manifest_data = {
            "schema_version": 1,
            "official_frozen": True,
            "pair_schedule": [
                {"repetition": 1, "order": ["go", "rust"]},
                {"repetition": 2, "order": ["rust", "go"]},
                {"repetition": 3, "order": ["go", "rust"]},
            ],
            "scenarios": {name: {} for name in OVERLAY.SCENARIOS},
        }
        self.manifest.write_text(json.dumps(self.manifest_data), encoding="utf-8")
        self.manifest_sha = hashlib.sha256(self.manifest.read_bytes()).hexdigest()
        self.manifest.with_suffix(".sha256").write_text(
            f"{self.manifest_sha}  {self.manifest.name}\n", encoding="utf-8"
        )
        self.driver.write_text("reviewed matrix driver\n", encoding="utf-8")
        self.driver_sha = hashlib.sha256(self.driver.read_bytes()).hexdigest()
        self.results.mkdir()
        self.make_status_table()

    def tearDown(self):
        self.temp.cleanup()

    def make_status_table(self):
        expected = OVERLAY.expected_attempts(self.manifest_data)
        with self.status.open("w", encoding="utf-8", newline="") as stream:
            writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
            writer.writerow(OVERLAY.STATUS_FIELDS)
            for number, (scenario, repetition, position, candidate) in enumerate(expected):
                run_dir = self.results / scenario / f"repetition-{repetition}" / candidate
                run_dir.mkdir(parents=True)
                metadata = {
                    "scenario": scenario,
                    "candidate": candidate,
                    "repetition": str(repetition),
                    "pair_position": str(position),
                    "run_mode": "official",
                    "manifest_sha256": self.manifest_sha,
                }
                (run_dir / "run-metadata.txt").write_text(
                    "".join(f"{key}={value}\n" for key, value in metadata.items()),
                    encoding="utf-8",
                )
                (run_dir / "stage-evidence.txt").write_text("immutable", encoding="utf-8")
                writer.writerow(
                    [
                        scenario,
                        repetition,
                        position,
                        candidate,
                        "2026-09-24T00:00:00Z",
                        "2026-09-24T00:00:01Z",
                        number % 2,
                        "0.10,0.10,0.10",
                        "0.10,0.10,0.10",
                    ]
                )

    def invoke(self, driver_sha=None):
        return OVERLAY.prepare_overlay(
            self.manifest,
            self.manifest_sha,
            self.driver,
            driver_sha or self.driver_sha,
            self.status,
            self.results,
            self.overlay,
            self.status_map,
        )

    def test_maps_all_24_statuses_without_mutating_raw_attempts(self):
        self.assertEqual(self.invoke(), 24)
        with self.status_map.open(encoding="utf-8", newline="") as stream:
            mapped = list(csv.DictReader(stream, delimiter="\t"))
        self.assertEqual(len(mapped), 24)
        self.assertEqual({row["runner_exit"] for row in mapped}, {"0", "1"})
        self.assertTrue(all(row["status_source_sha256"] == hashlib.sha256(self.status.read_bytes()).hexdigest() for row in mapped))
        for row in mapped:
            raw_dir = Path(row["raw_run_dir"])
            overlay_dir = Path(row["overlay_run_dir"])
            self.assertFalse((raw_dir / "attempt-exit-status.txt").exists())
            self.assertEqual((overlay_dir / "attempt-exit-status.txt").read_text(), f"exit={row['runner_exit']}\n")
            self.assertTrue((overlay_dir / "stage-evidence.txt").is_symlink())

    def test_rejects_unreviewed_driver_before_creating_overlay(self):
        with self.assertRaisesRegex(ValueError, "matrix driver does not match"):
            self.invoke("0" * 64)
        self.assertFalse(self.overlay.exists())
        self.assertFalse(self.status_map.exists())

    def test_rejects_schedule_mismatch_before_creating_overlay(self):
        rows = self.status.read_text(encoding="utf-8").splitlines()
        fields = rows[1].split("\t")
        fields[3] = "rust"
        rows[1] = "\t".join(fields)
        self.status.write_text("\n".join(rows) + "\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "exact frozen 24-row schedule"):
            self.invoke()
        self.assertFalse(self.overlay.exists())
        self.assertFalse(self.status_map.exists())


if __name__ == "__main__":
    unittest.main()
