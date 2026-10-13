#!/usr/bin/env python3
"""Check backend evidence/refusal behavior with fake tools; no native acceptance."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("run-native-tests.py")
SPEC = importlib.util.spec_from_file_location("native_test_runner", SCRIPT)
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)
TARGET = "x86_64-unknown-linux-gnu"


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="aster-backend-runner-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for directory in ("scripts", ".cargo", "src", "crates", "build output"):
            (self.root / directory).mkdir()
        self.script = self.root / "scripts" / SCRIPT.name
        self.script.write_bytes(SCRIPT.read_bytes())
        for name in ("Cargo.toml", "Cargo.lock", ".cargo/config.toml", "src/main.rs"):
            (self.root / name).write_text("fixture\n", encoding="utf-8")
        (self.root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.97.1"\n', encoding="utf-8")
        self.output = self.root / "evidence"
        self.calls = []

    def invoke(self, mode="--run", target=TARGET, behavior=None):
        def fake_run(command, **arguments):
            self.calls.append(command)
            if behavior:
                outcome = behavior(command, arguments)
                if outcome is not None:
                    return outcome
            if command[0] == "/fake/rustc":
                return subprocess.CompletedProcess(command, 0, f"rustc 1.97.1 (fixture)\nhost: {TARGET}\n", "")
            stream = arguments["stdout"]
            if "scripts/test-workspace.py" in command:
                stream.write("test result: ok. 2 passed; 0 failed; 0 ignored;\n")
            if "build" in command:
                for name in ("aster", "aster-agent"):
                    binary = self.root / "build output" / f"{name}.exe"
                    binary.write_bytes(b"fake executable; never launched")
                    stream.write(json.dumps({"reason": "compiler-artifact", "target": {"name": name, "kind": ["bin"]}, "executable": str(binary)}) + "\n")
            return subprocess.CompletedProcess(command, 0)

        with (
            patch.object(RUNNER, "__file__", str(self.script)),
            patch.object(sys, "argv", [str(self.script), mode, "--target", target, "--output", "evidence"]),
            patch.object(RUNNER.platform, "system", return_value="Linux"),
            patch.object(RUNNER.platform, "machine", return_value="x86_64"),
            patch.object(RUNNER.shutil, "which", side_effect=lambda name: f"/fake/{name}"),
            patch.object(RUNNER.subprocess, "run", side_effect=fake_run),
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            return RUNNER.main()

    def report(self):
        return json.loads((self.output / "report.json").read_text(encoding="utf-8"))

    def test_plan_launches_nothing_and_cannot_pass(self):
        self.assertEqual(self.invoke("--plan", "x86_64-pc-windows-msvc"), 0)
        self.assertEqual(self.calls, [])
        report = self.report()
        self.assertEqual(report["status"], "not_run")
        self.assertTrue(all(step["status"] == "not_run" for step in report["steps"]))

    def test_wrong_host_refuses_before_tools_or_output(self):
        with self.assertRaises(SystemExit) as refusal:
            self.invoke(target="x86_64-pc-windows-msvc")
        self.assertEqual(refusal.exception.code, 2)
        self.assertEqual(self.calls, [])
        self.assertFalse(self.output.exists())

    def test_toolchain_timeout_retains_failed_report(self):
        def timeout(command, arguments):
            raise subprocess.TimeoutExpired(command, 15)
        self.assertEqual(self.invoke(behavior=timeout), 1)
        report = self.report()
        self.assertEqual(report["status"], "failed")
        self.assertIn("TimeoutExpired", report["failure"])
        self.assertTrue(all(step["status"] == "not_run" for step in report["steps"]))

    def test_missing_executable_retains_failure_and_stops_later_checks(self):
        def missing(command, arguments):
            if "scripts/check-media-tools.py" in command:
                raise FileNotFoundError("fake interpreter disappeared")
        self.assertEqual(self.invoke(behavior=missing), 1)
        report = self.report()
        self.assertEqual(report["status"], "failed")
        self.assertEqual([step["status"] for step in report["steps"]], ["passed", "failed", "not_run", "not_run", "not_run"])
        self.assertTrue((self.output / "media-tools.log").exists())

    def test_nonzero_media_preflight_retains_exit_and_never_runs_tests(self):
        def failed(command, arguments):
            if "scripts/check-media-tools.py" in command:
                arguments["stdout"].write("fake missing ffprobe\n")
                return subprocess.CompletedProcess(command, 7)
        self.assertEqual(self.invoke(behavior=failed), 7)
        report = self.report()
        self.assertEqual(report["steps"][1]["exit_code"], 7)
        self.assertEqual(report["steps"][2]["status"], "not_run")
        self.assertIn("missing ffprobe", (self.output / "media-tools.log").read_text())

    def test_missing_artifact_cannot_be_reported_as_passed(self):
        def incomplete(command, arguments):
            if "build" in command:
                arguments["stdout"].write('{"reason":"build-finished","success":true}\n')
                return subprocess.CompletedProcess(command, 0)
        self.assertEqual(self.invoke(behavior=incomplete), 1)
        self.assertEqual(self.report()["status"], "failed")
        self.assertIn("incomplete", self.report()["failure"])

    def test_source_mutation_invalidates_an_otherwise_successful_run(self):
        def mutation(command, arguments):
            if "build" in command:
                (self.root / "src/main.rs").write_text("changed during fake run\n", encoding="utf-8")
        self.assertEqual(self.invoke(behavior=mutation), 1)
        report = self.report()
        self.assertEqual(report["status"], "failed")
        self.assertNotEqual(report["source_sha256"], report["source_sha256_after"])

    def test_artifact_paths_are_taken_from_cargo_events_with_spaces_and_suffixes(self):
        self.assertEqual(self.invoke(), 0)
        report = self.report()
        self.assertEqual(report["test_counts"], {"passed": 2, "failed": 0, "ignored": 0})
        self.assertEqual(set(report["binaries"]), {"aster", "aster-agent"})
        for name, record in report["binaries"].items():
            self.assertEqual(record["path"], str(self.root / "build output" / f"{name}.exe"))
        self.assertTrue(report["native_ui_gpu_audio_acceptance"].startswith("not_run"))


if __name__ == "__main__":
    unittest.main()
