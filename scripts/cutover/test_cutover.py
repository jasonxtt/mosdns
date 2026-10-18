"""Transaction guards. Run on the Linux VM with unittest; no live service used."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import subprocess
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location("cutover", Path(__file__).with_name("cutover.py"))
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)


class Guards(unittest.TestCase):
    def test_zip_rejects_traversal_duplicates_and_symlinks(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "test.zip"
            for name in ("../outside", "/outside", "sub_config/../outside", "sub_config\\x.yaml"):
                with zipfile.ZipFile(path, "w") as z:
                    z.writestr(name, b"x")
                with self.assertRaises(ValueError):
                    c.zip_entries(path)
            entry = zipfile.ZipInfo("sub_config/x.yaml")
            entry.external_attr = 0o120777 << 16
            with zipfile.ZipFile(path, "w") as z:
                z.writestr(entry, "/etc/passwd")
            with self.assertRaises(ValueError):
                c.zip_entries(path)

    def test_package_cannot_manage_user_data_or_generated_groups(self):
        with tempfile.TemporaryDirectory() as tmp:
            for name in ("webinfo/settings.json", "rule/whitelist.txt", "sub_config/special_groups.yaml"):
                path = Path(tmp) / "pkg.zip"
                with zipfile.ZipFile(path, "w") as z:
                    z.writestr("manifest.json", json.dumps({"format": 1, "channel": "rust",
                        "config_schema": 1, "package_id": c.RUST_ID, "managed_files": [name],
                        "sha256": {name: c.hashlib.sha256(b"x").hexdigest()}}))
                    z.writestr(name, b"x")
                with self.assertRaises(ValueError):
                    c.package(path, "rust", 1, c.RUST_ID)

    def test_missing_and_modified_managed_files_are_differences(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "config_custom.yaml").write_bytes(b"modified")
            (root / "unknown.bak").write_bytes(b"preserve")
            self.assertEqual(c.changed_managed(root,
                {"config_custom.yaml": b"original", "sub_config/x.yaml": b"x"},
                ["config_custom.yaml", "sub_config/x.yaml"]), ["config_custom.yaml", "sub_config/x.yaml"])
            self.assertEqual((root / "unknown.bak").read_bytes(), b"preserve")

    def test_space_refusal_does_not_write_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(ValueError):
                c.space([(root, 10**20, 1)])
            self.assertEqual(list(root.iterdir()), [])

    def test_tampered_backup_refuses_before_service_stop(self):
        with tempfile.TemporaryDirectory() as tmp:
            tx = Path(tmp)
            (tx / "backup").mkdir()
            (tx / "backup/binary").write_bytes(b"corrupt")
            s = {"tx": tmp, "root": tmp, "old_binary_hash": "0"*64}
            with patch.object(c, "run") as command:
                with self.assertRaises(ValueError):
                    c.restore(s, "test")
                command.assert_not_called()

    def test_atomic_state_and_hash_manifest_detect_data_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            c.save(root / "state.json", {"phase": "root_moving"})
            expected = c.hashes(root)
            c.verify_hashes(root, expected)
            (root / "state.json").write_text("corrupt")
            with self.assertRaises(ValueError):
                c.verify_hashes(root, expected)
            self.assertEqual(len(list(root.iterdir())), 1)

    def test_unloaded_unit_reset_failure_does_not_prevent_go_restart(self):
        with tempfile.TemporaryDirectory() as tmp:
            tx = Path(tmp)
            root = tx / "live"
            root.mkdir()
            (tx / "backup").mkdir()
            (tx / "backup/binary").write_bytes(b"Go")
            binary = tx / "mosdns"
            binary.write_bytes(b"Rust")
            s = {"tx": tmp, "root": str(root), "binary": str(binary), "metadata": [],
                 "metadata_hashes": {}, "service": "test.service", "old_version": "Go"}
            def commands(argv, **kwargs):
                if argv[1] == "reset-failed":
                    raise subprocess.CalledProcessError(1, argv)
                return b""
            with patch.object(c, "verify_backup"), patch.object(c, "metadata_paths", return_value=[]), \
                 patch.object(c, "space"), patch.object(c, "tree_size", return_value=1), \
                 patch.object(c, "run", side_effect=commands) as command, \
                 patch.object(c.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)), \
                 patch.object(c, "health"), patch.object(c, "system_props", return_value={"ActiveState": "active"}):
                c.restore(s, "test")
                self.assertIn(unittest.mock.call(["systemctl", "start", "test.service"]), command.call_args_list)
            self.assertEqual(c.load(tx / "state.json")["phase"], "rolled_back")
            self.assertEqual(binary.read_bytes(), b"Go")

    def test_recovery_after_either_config_rename_keeps_go_and_rust_data(self):
        for rust_installed in (False, True):
            with self.subTest(rust_installed=rust_installed), tempfile.TemporaryDirectory() as tmp:
                tx = Path(tmp)
                root = tx / "live"
                old = tx / "go-root"
                old.mkdir()
                (old / "user-rule.txt").write_bytes(b"Go user data")
                before = c.hashes(old)
                if rust_installed:
                    root.mkdir()
                    (root / "rust-rule.txt").write_bytes(b"Rust user data")
                (tx / "backup").mkdir()
                (tx / "backup/binary").write_bytes(b"Go")
                binary = tx / "mosdns"
                binary.write_bytes(b"Rust")
                s = {"tx": tmp, "root": str(root), "binary": str(binary), "metadata": [],
                     "metadata_hashes": {}, "service": "test.service", "old_version": "Go",
                     "config_hashes": before, "final_hashes": before}
                with patch.object(c, "verify_backup"), patch.object(c, "metadata_paths", return_value=[]), \
                     patch.object(c, "space"), patch.object(c, "tree_size", return_value=1), \
                     patch.object(c, "run", return_value=b""), patch.object(c.subprocess, "run"), \
                     patch.object(c, "health"), patch.object(c, "system_props", return_value={"ActiveState": "active"}):
                    c.restore(s, "interrupted rename")
                self.assertEqual((root / "user-rule.txt").read_bytes(), b"Go user data")
                self.assertEqual(binary.read_bytes(), b"Go")
                if rust_installed:
                    self.assertEqual((Path(s["preserved_rust_root"]) / "rust-rule.txt").read_bytes(), b"Rust user data")


if __name__ == "__main__":
    unittest.main()
