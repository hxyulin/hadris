#!/usr/bin/env python3
"""Regression checks for incomplete inventories and false proof success."""

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("verify_fat", Path(__file__).with_name("verify-fat.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class ProofRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        path = "crates/block/hadris-fat-raw/src/verification.rs"
        source = self.root / path
        source.parent.mkdir(parents=True)
        source.write_text("#[kani::proof]\nfn example() {}\n")
        self.row = {"harness": "verification::example", "path": path,
                    "group": "entries", "scope": "All input bytes", "requirements": [], "covers": 0}
        self.document = {"schema_version": 1, "crate": "hadris-fat-raw",
                         "features": [], "verifier_version": "0.68.0", "harnesses": [self.row]}
        (self.root / "spec/proofs").mkdir(parents=True)
        (self.root / "spec/requirements").mkdir()
        (self.root / "spec/requirements/hadris-fat.json").write_text('{"requirements": []}')
        self.write_inventory()

    def write_inventory(self):
        (self.root / runner.MANIFEST).write_text(json.dumps(self.document))

    def test_complete_inventory_is_accepted(self):
        self.assertEqual(len(runner.inventory(self.root)["harnesses"]), 1)

    def test_missing_and_duplicate_harnesses_fail(self):
        for rows in ([], [self.row, self.row]):
            self.document["harnesses"] = rows
            self.write_inventory()
            with self.assertRaises(ValueError):
                runner.inventory(self.root)

    def test_new_unmapped_harness_fails(self):
        source = self.root / self.row["path"]
        source.write_text(source.read_text() + "#[kani::proof]\nfn unmapped() {}\n")
        with self.assertRaises(ValueError):
            runner.inventory(self.root)

    def test_harness_elsewhere_in_crate_cannot_be_silently_omitted(self):
        (self.root / self.row["path"]).with_name("other.rs").write_text("#[kani::proof]\nfn omitted() {}\n")
        with self.assertRaises(ValueError):
            runner.inventory(self.root)

    def test_invalid_requirement_or_path_fails(self):
        for key, value in (("requirements", ["missing"]), ("path", "elsewhere.rs")):
            with self.subTest(key=key):
                original = self.row[key]
                self.row[key] = value
                self.write_inventory()
                with self.assertRaises(ValueError):
                    runner.inventory(self.root)
                self.row[key] = original

    def test_catalog_citation_must_be_in_manifest(self):
        requirement = {"id": "spec:clause#claim", "tests": [{"kind": "proof",
                       "path": self.row["path"], "name": "example"}]}
        (self.root / "spec/requirements/hadris-fat.json").write_text(json.dumps({"requirements": [requirement]}))
        with self.assertRaises(ValueError):
            runner.inventory(self.root)
        self.row["requirements"] = [requirement["id"]]
        self.write_inventory()
        runner.inventory(self.root)

    def result(self, text, code=0):
        def fake_run(command, **kwargs):
            kwargs["stdout"].write(text)
            return subprocess.CompletedProcess(command, code)
        with patch.object(runner.subprocess, "run", side_effect=fake_run):
            return runner.run_harness(self.root, self.row, self.root, 1)

    def test_exactly_one_successful_proof_passes(self):
        result = self.result("VERIFICATION:- SUCCESSFUL\n1 successfully verified harnesses, 0 failures, 1 total")
        self.assertEqual(result["status"], "passed")
        self.assertIn("--exact", result["command"])
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
        with patch.object(runner.subprocess, "run", side_effect=subprocess.TimeoutExpired("cargo", 1)):
            result = runner.run_harness(self.root, self.row, self.root, 1)
        self.assertEqual(result["status"], "timeout")
        self.assertIsNone(result["returncode"])


if __name__ == "__main__":
    unittest.main()
