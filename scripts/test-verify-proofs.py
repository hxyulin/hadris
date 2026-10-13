#!/usr/bin/env python3
"""Regression checks for incomplete inventories and false proof success."""

import importlib.util
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("verify_proofs", Path(__file__).with_name("verify-proofs.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class ProofRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for crate in ("example-raw", "example"):
            (self.root / "crates" / crate / "src").mkdir(parents=True)
            (self.root / "crates" / crate / "Cargo.toml").write_text("")
        path = "crates/example-raw/src/verification.rs"
        (self.root / path).write_text("#[kani::proof]\nfn example() {}\n")
        self.row = {"harness": "verification::example", "crate": "example-raw", "path": path,
                    "group": "entries", "scope": "All input bytes", "requirements": [], "covers": 0}
        self.document = {
            "schema_version": 2, "verifier_version": "0.68.0",
            "catalog": "spec/requirements/example.json", "workflow": ".github/workflows/example-proofs.yml",
            "groups": ["entries"],
            "crates": {"example-raw": {"path": "crates/example-raw", "features": []}},
            "harnesses": [self.row],
        }
        (self.root / "spec/proofs").mkdir(parents=True)
        (self.root / "spec/requirements").mkdir()
        (self.root / "spec/requirements/example.json").write_text('{"requirements": []}')
        (self.root / ".github/workflows").mkdir(parents=True)
        self.write_workflow("entries")
        self.write_inventory()

    def write_inventory(self):
        (self.root / "spec/proofs/example.json").write_text(json.dumps(self.document))

    def write_workflow(self, groups):
        (self.root / self.document["workflow"]).write_text(
            f"jobs:\n  verify:\n    strategy:\n      matrix:\n        group: [{groups}]\n"
        )

    def inventory(self):
        return runner.inventory(self.root, "example")

    def test_complete_inventory_is_accepted(self):
        self.assertEqual(len(self.inventory()["harnesses"]), 1)
        self.assertEqual(runner.suites(self.root), ["example"])

    def test_missing_and_duplicate_harnesses_fail(self):
        for rows in ([], [self.row, self.row]):
            self.document["harnesses"] = rows
            self.write_inventory()
            with self.assertRaises(ValueError):
                self.inventory()

    def test_new_unmapped_harness_fails(self):
        source = self.root / self.row["path"]
        source.write_text(source.read_text() + "#[kani::proof]\nfn unmapped() {}\n")
        with self.assertRaises(ValueError):
            self.inventory()

    def test_harness_elsewhere_in_crate_cannot_be_silently_omitted(self):
        (self.root / self.row["path"]).with_name("other.rs").write_text("#[kani::proof]\nfn omitted() {}\n")
        with self.assertRaises(ValueError):
            self.inventory()

    def test_harnesses_in_every_listed_crate_are_required(self):
        path = "crates/example/src/verification.rs"
        (self.root / path).write_text("#[kani::proof]\nfn example() {}\n")
        self.document["crates"]["example"] = {"path": "crates/example", "features": ["alloc"]}
        self.write_inventory()
        with self.assertRaises(ValueError):
            self.inventory()
        self.document["harnesses"].append({**self.row, "crate": "example", "path": path})
        self.write_inventory()
        self.assertEqual(len(self.inventory()["harnesses"]), 2)
        command = runner.command_for(self.document["harnesses"][1], ["alloc"], 1)
        self.assertEqual(command[command.index("-p") + 1], "example")
        self.assertEqual(command[command.index("--features") + 1], "alloc")

    def test_wrong_crate_for_harness_fails(self):
        self.row["crate"] = "example"
        self.document["crates"]["example"] = {"path": "crates/example", "features": []}
        self.write_inventory()
        with self.assertRaises(ValueError):
            self.inventory()

    def test_invalid_requirement_or_path_fails(self):
        for key, value in (("requirements", ["missing"]), ("path", "elsewhere.rs"), ("group", "other")):
            with self.subTest(key=key):
                original = self.row[key]
                self.row[key] = value
                self.write_inventory()
                with self.assertRaises(ValueError):
                    self.inventory()
                self.row[key] = original

    def test_workflow_matrix_must_match_groups(self):
        for groups in ("", "entries, other", "other"):
            with self.subTest(groups=groups):
                self.write_workflow(groups)
                with self.assertRaises(ValueError):
                    self.inventory()
        self.write_workflow("entries")
        self.document["groups"] = ["entries", "unused"]
        self.write_inventory()
        self.write_workflow("entries, unused")
        with self.assertRaises(ValueError):
            self.inventory()

    def test_catalog_citation_must_be_in_manifest(self):
        requirement = {"id": "spec:clause#claim", "tests": [{"kind": "proof",
                       "path": self.row["path"], "name": "example"}]}
        (self.root / "spec/requirements/example.json").write_text(json.dumps({"requirements": [requirement]}))
        with self.assertRaises(ValueError):
            self.inventory()
        self.row["requirements"] = [requirement["id"]]
        self.write_inventory()
        self.inventory()

    def result(self, text, code=0):
        def fake_run(command, cwd, output, timeout):
            output.write(text)
            return code
        with patch.object(runner, "run_group", side_effect=fake_run):
            return runner.run_harness(self.root, self.row, [], self.root, 1)

    def test_exactly_one_successful_proof_passes(self):
        result = self.result("VERIFICATION:- SUCCESSFUL\n1 successfully verified harnesses, 0 failures, 1 total")
        self.assertEqual(result["status"], "passed")
        self.assertIn("--exact", result["command"])
        self.assertNotIn("--features", result["command"])
        self.assertEqual(result["command"][result["command"].index("--harness") + 1], "verification::example")

    def test_zero_proofs_or_unconfirmed_success_fails(self):
        for text in ("", "VERIFICATION:- SUCCESSFUL", "0 successfully verified harnesses, 0 failures, 0 total"):
            with self.subTest(text=text):
                self.assertEqual(self.result(text)["status"], "failed")

    def test_nonzero_exit_overrides_success_text(self):
        text = "VERIFICATION:- SUCCESSFUL\n1 successfully verified harnesses, 0 failures, 1 total"
        self.assertEqual(self.result(text, 1)["status"], "failed")

    def test_unreachable_or_missing_acceptance_cover_fails(self):
        self.row["covers"] = 1
        success = "VERIFICATION:- SUCCESSFUL\n1 successfully verified harnesses, 0 failures, 1 total"
        for covers in ("", "0 of 1 cover properties satisfied (1 unreachable)"):
            with self.subTest(covers=covers):
                self.assertEqual(self.result(success + "\n" + covers)["status"], "failed")
        self.assertEqual(self.result(success + "\n1 of 1 cover properties satisfied")["status"], "passed")

    def test_timeout_cannot_pass(self):
        with patch.object(runner, "run_group", return_value=None):
            result = runner.run_harness(self.root, self.row, [], self.root, 1)
        self.assertEqual(result["status"], "timeout")
        self.assertIsNone(result["returncode"])

    def test_timeout_kills_whole_process_group(self):
        marker = self.root / "survivor"
        command = ["sh", "-c", f"(sleep 2; touch '{marker}') & wait"]
        with (self.root / "log").open("w") as output:
            self.assertIsNone(runner.run_group(command, self.root, output, 0.5))
        time.sleep(2.5)
        self.assertFalse(marker.exists())

    def test_repository_inventories_are_complete(self):
        for suite in runner.suites(runner.ROOT):
            with self.subTest(suite=suite):
                runner.inventory(runner.ROOT, suite)


if __name__ == "__main__":
    unittest.main()
