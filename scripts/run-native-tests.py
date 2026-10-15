#!/usr/bin/env python3
"""Prepare or run native backend checks, retaining logs and build identities."""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time


TARGETS = {
    "x86_64-unknown-linux-gnu": ("Linux", "x86_64"),
    "aarch64-apple-darwin": ("Darwin", "arm64"),
    "x86_64-apple-darwin": ("Darwin", "x86_64"),
    "x86_64-pc-windows-msvc": ("Windows", "x86_64"),
}


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def source_digest(root: Path) -> str:
    paths = {Path("Cargo.toml"), Path("Cargo.lock"), Path("rust-toolchain.toml"), Path(".cargo/config.toml")}
    paths.update(path.relative_to(root) for base in ("src", "crates") for path in (root / base).rglob("*.rs"))
    paths.update(path.relative_to(root) for path in (root / "crates").rglob("Cargo.toml"))
    value = hashlib.sha256()
    for path in sorted(paths, key=lambda item: item.as_posix()):
        value.update(path.as_posix().encode() + b"\0" + (root / path).read_bytes() + b"\0")
    return value.hexdigest()


def collect_results(output: Path) -> tuple[dict, dict]:
    results = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", (output / "tests.log").read_text(encoding="utf-8"))
    counts = {
        key: sum(int(values[index]) for values in results)
        for index, key in enumerate(("passed", "failed", "ignored"))
    }
    binaries = {}
    # Cargo's artifact event is authoritative even with a custom target-dir or
    # a native executable suffix. Example/test artifacts are not application bins.
    with (output / "build.log").open(encoding="utf-8") as stream:
        for line in stream:
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if not isinstance(event, dict) or event.get("reason") != "compiler-artifact":
                continue
            target = event.get("target", {})
            if not isinstance(target, dict) or "bin" not in target.get("kind", []):
                continue
            name, executable = target.get("name"), event.get("executable")
            if name in ("aster", "aster-agent", "aster-mcp") and executable:
                binary = Path(executable).resolve(strict=True)
                binaries[name] = {"path": str(binary), "bytes": binary.stat().st_size, "sha256": digest(binary)}
    if set(binaries) != {"aster", "aster-agent", "aster-mcp"} or not results or counts["passed"] == 0 or counts["failed"]:
        raise ValueError("Native build/test records are incomplete")
    return counts, binaries


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--plan", action="store_true", help="Write a not-run plan; launch no tools")
    mode.add_argument("--run", action="store_true", help="Run checks on the matching native host")
    parser.add_argument("--output", type=Path, required=True, help="Fresh workspace artifact directory")
    arguments = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    output = (root / arguments.output).resolve()
    if not output.is_relative_to(root) or output == root or output.exists():
        parser.error("--output must be a new directory inside this repository")

    expected_os, expected_arch = TARGETS[arguments.target]
    observed_arch = platform.machine().lower()
    observed_arch = {"amd64": "x86_64", "aarch64": "arm64"}.get(observed_arch, observed_arch)
    if arguments.run and (platform.system(), observed_arch) != (expected_os, expected_arch):
        parser.error("Native execution requires the target OS and architecture; use --plan here")

    python = "python" if expected_os == "Windows" else "python3"
    common = ["--profile", "ci", "--locked", "--target", arguments.target]
    commands = [
        ("architecture", [python, "scripts/check-architecture.py"]),
        ("media-tools", [python, "scripts/check-media-tools.py"]),
        ("tests", [python, "scripts/test-workspace.py", "--profile", "ci", "--target", arguments.target]),
        ("clippy", ["cargo", "clippy", *common, "--workspace", "--all-targets", "--", "-D", "warnings"]),
        ("build", ["cargo", "build", *common, "--bins", "--message-format=json"]),
    ]
    report = {
        "created_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "target": arguments.target,
        "host_os": platform.system(),
        "host_architecture": observed_arch,
        "source_sha256": source_digest(root),
        "source_digest_scope": "Sorted relative path, NUL, content, NUL: Rust/manifests/lock/toolchain/Cargo config",
        "runner_sha256": digest(Path(__file__)),
        "status": "not_run",
        "profile": "ci, correctness/link checks; not an optimized performance build",
        "steps": [{"name": name, "command": command, "status": "not_run"} for name, command in commands],
        "native_ui_gpu_audio_acceptance": "not_run; requires the separate native UI/device cases",
    }
    output.mkdir(parents=True)
    report_path = output / "report.json"

    def save() -> None:
        pending = output / "report.pending.json"
        pending.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        pending.replace(report_path)

    def fail(message: str, exit_code: int = 1) -> int:
        report.update(status="failed", failure=message)
        save()
        print(json.dumps({"status": "failed", "failure": message, "report": str(report_path.relative_to(root))}), flush=True)
        return exit_code

    save()
    if arguments.plan:
        print(json.dumps({"status": "not_run", "report": str(report_path.relative_to(root))}))
        return 0

    environment = os.environ.copy()
    cargo = shutil.which("cargo")
    rustc = shutil.which(environment.get("RUSTC", "rustc"))
    interpreter = shutil.which(python)
    if cargo is None or rustc is None or interpreter is None:
        return fail("Pinned Rust tools and a native Python interpreter are required")
    try:
        version = subprocess.run([rustc, "-vV"], cwd=root, capture_output=True, text=True, encoding="utf-8", timeout=15, check=False)
    except (OSError, subprocess.TimeoutExpired, KeyboardInterrupt) as error:
        return fail(f"Rust toolchain preflight: {type(error).__name__}: {error}", 130 if isinstance(error, KeyboardInterrupt) else 1)
    pinned = re.search(r'^channel\s*=\s*"([^"]+)"', (root / "rust-toolchain.toml").read_text(), re.MULTILINE)
    native_host = re.search(r"^host: (.+)$", version.stdout, re.MULTILINE)
    if version.returncode or not pinned or not native_host or native_host[1] != arguments.target or not version.stdout.startswith(f"rustc {pinned[1]} "):
        report["rustc_version"] = version.stdout
        return fail("Rust version/host differs from the pinned native target")
    report["rustc_version"] = version.stdout
    for step in report["steps"]:
        command = [interpreter if step["command"][0] == python else cargo, *step["command"][1:]]
        log = output / f'{step["name"]}.log'
        started = time.monotonic()
        step.update(status="running", log=str(log.relative_to(root)))
        report["status"] = "running"
        save()
        try:
            with log.open("w", encoding="utf-8") as stream:
                result = subprocess.run(command, cwd=root, env=environment, stdout=stream, stderr=subprocess.STDOUT, check=False)
        except (OSError, subprocess.TimeoutExpired, KeyboardInterrupt) as error:
            step.update(status="failed", elapsed_seconds=round(time.monotonic() - started, 3), failure=f"{type(error).__name__}: {error}")
            return fail(f'{step["name"]}: {step["failure"]}', 130 if isinstance(error, KeyboardInterrupt) else 1)
        step.update(status="passed" if result.returncode == 0 else "failed", exit_code=result.returncode, elapsed_seconds=round(time.monotonic() - started, 3), log=str(log.relative_to(root)))
        save()
        print(json.dumps(step), flush=True)
        if result.returncode:
            return fail(f'{step["name"]} exited with {result.returncode}', result.returncode if result.returncode > 0 else 1)
    try:
        report["test_counts"], report["binaries"] = collect_results(output)
        report["source_sha256_after"] = source_digest(root)
        runner_after = digest(Path(__file__))
    except (OSError, ValueError, TypeError) as error:
        return fail(f"Build/test evidence: {type(error).__name__}: {error}")
    if report["source_sha256_after"] != report["source_sha256"] or runner_after != report["runner_sha256"]:
        return fail("Source or runner changed during native checks")
    report["status"] = "passed"
    save()
    print(json.dumps({"status": "passed", "report": str(report_path.relative_to(root)), "tests": report["test_counts"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
