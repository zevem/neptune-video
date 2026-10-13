#!/usr/bin/env python3
"""Build the owned provider fixture, then run locked Rust correctness tests."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("ci", "release", "dev"), default="ci")
    parser.add_argument("--target", help="Optional Cargo target; otherwise use Cargo's environment")
    parser.add_argument("--package", action="append", default=[])
    parser.add_argument("--filter", help="Optional Rust test-name filter")
    arguments = parser.parse_args()

    repository = Path(__file__).resolve().parent.parent
    cargo = shutil.which("cargo")
    if cargo is None:
        parser.error("cargo is unavailable; install the pinned Rust toolchain")

    common = ["--profile", arguments.profile, "--locked"]
    if arguments.target:
        common.extend(["--target", arguments.target])
    build = subprocess.run(
        [
            cargo, "build", *common, "-p", "editor-agent", "--example",
            "provider-fault-fixture", "--message-format=json",
        ],
        cwd=repository,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        check=False,
    )
    executable = None
    try:
        for line in build.stdout.splitlines():
            event = json.loads(line)
            if event.get("reason") == "compiler-message":
                rendered = event.get("message", {}).get("rendered")
                if rendered:
                    sys.stderr.write(rendered)
            elif (
                event.get("reason") == "compiler-artifact"
                and event.get("target", {}).get("name") == "provider-fault-fixture"
                and "example" in event.get("target", {}).get("kind", [])
            ):
                executable = event.get("executable")
        if build.returncode:
            return build.returncode
        if not executable:
            raise ValueError("Cargo did not report the example executable")
        fixture = Path(executable).resolve(strict=True)
        if not fixture.is_file():
            raise ValueError("provider fixture is not a file")
        digest = hashlib.sha256()
        with fixture.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except (OSError, ValueError, KeyError, TypeError) as error:
        sys.stderr.write(f"Cannot locate the built provider fixture: {error}\n")
        return 1

    print(f"Provider fault fixture: {fixture} (SHA-256 {digest.hexdigest()})", flush=True)
    environment = os.environ.copy()
    environment["ASTER_PROVIDER_TEST_FIXTURE"] = str(fixture)
    test = [cargo, "test", *common, "--all-targets"]
    if arguments.package:
        for package in arguments.package:
            test.extend(["-p", package])
    else:
        test.append("--workspace")
    if arguments.filter:
        test.append(arguments.filter)
    return subprocess.run(test, cwd=repository, env=environment, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
