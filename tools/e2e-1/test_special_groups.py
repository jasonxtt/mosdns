"""Guard the narrow renderer exception used by the differential."""
import unittest

import copy

from special_groups import compare_rows, without_upstreams


class RenderException(unittest.TestCase):
    def test_only_group_upstream_entries_may_differ(self):
        prefix = b"  - tag: special_upstream_50\n    type: aliapi\n    args:\n      concurrent: 2\n      upstreams:\n"
        suffix = b"\n  - tag: sequence_special_50\n    type: sequence\n"
        go = prefix + b'        - addr: "223.5.5.5"\n' + suffix
        native = prefix + b'        - {"addr":"udp://127.0.0.1:15401"}\n' + suffix
        self.assertEqual(without_upstreams(go), without_upstreams(native))
        self.assertNotEqual(without_upstreams(go), without_upstreams(native.replace(b"sequence_special_50", b"sequence_special_51")))
        self.assertNotEqual(without_upstreams(go), without_upstreams(native.replace(b"concurrent: 2", b"concurrent: 3")))
        self.assertEqual(without_upstreams(b"      upstreams:\n        - addr: foo\n"), b"      upstreams:\n        - addr: foo\n")

    def test_audit_classification_rejects_missing_labels_and_wrong_routes(self):
        go, native = [], []
        for run in [1, 2]:
            for port, name, slot in [(15353, "main.example.com", 50), (15353, "port.example.com", None), (15354, "port.example.com", 51)]:
                for qtype in [1, 28]:
                    row = {"run": run, "port": port, "name": name, "qtype": qtype, "answers": []}
                    label = "特殊上游50" if slot == 50 else "unmatched_rule"
                    sequence = f"sequence_special_{slot}" if slot else "main"
                    upstream = f"special_upstream_{slot}" if slot else "normal"
                    cached = slot is not None and run == 2
                    go.append({**row, "route": {"domain_set": label, "effective_tag": label,
                        "final_sequence": None if cached else sequence, "final_upstream": None if cached else upstream}})
                    native.append({**row, "route": {"domain_set": label, "effective_tag": label,
                        "final_sequence": sequence, "final_upstream": f"special_{slot}_0" if cached else upstream}})
        self.assertEqual(len(compare_rows(go, native)), 4)
        for key, value in [("domain_set", None), ("effective_tag", "unmatched_rule"), ("final_sequence", "sequence_special"), ("final_upstream", "unknown")]:
            broken = copy.deepcopy(native)
            broken[6]["route"][key] = value
            with self.assertRaises(AssertionError):
                compare_rows(go, broken)
        with self.assertRaises(AssertionError):
            compare_rows(go[:-1], native)


if __name__ == "__main__":
    unittest.main()
