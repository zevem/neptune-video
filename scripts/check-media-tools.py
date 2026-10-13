#!/usr/bin/env python3
"""Require working media executables before claiming FFmpeg fixture coverage."""

from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


TIMEOUT_SECONDS = 10
MAX_VERSION_LINE_BYTES = 4096
TOOLS = (("ffmpeg", "ASTER_FFMPEG"), ("ffprobe", "ASTER_FFPROBE"))


def check(tool: str, variable: str) -> None:
    requested = os.environ.get(variable, tool)
    if not requested or "\0" in requested:
        raise RuntimeError(f"{variable} must name an executable")
    executable = shutil.which(requested)
    if executable is None:
        source = variable if variable in os.environ else "PATH"
        raise RuntimeError(f"{tool} executable from {source} was not found or is not executable")
    executable = str(Path(executable).absolute())
    # Version output goes to an owned temporary file, keeping capture memory
    # bounded even if a broken executable prints unexpectedly large output.
    with tempfile.TemporaryFile() as output:
        subprocess.run(
            [executable, "-version"],
            stdin=subprocess.DEVNULL,
            stdout=output,
            stderr=subprocess.STDOUT,
            check=True,
            timeout=TIMEOUT_SECONDS,
        )
        output.seek(0)
        line = output.readline(MAX_VERSION_LINE_BYTES + 1)
    if len(line) > MAX_VERSION_LINE_BYTES:
        raise RuntimeError(f"{tool} version line exceeds {MAX_VERSION_LINE_BYTES} bytes")
    version = line.decode("utf-8", errors="replace").strip()
    if not version.startswith(f"{tool} version "):
        raise RuntimeError(f"{tool} executable did not report a valid version banner")
    print(f"{tool} executable: {executable}")
    print(version)


def main() -> int:
    failed = False
    for tool, variable in TOOLS:
        try:
            check(tool, variable)
        except subprocess.TimeoutExpired:
            failed = True
            print(f"ERROR: {tool} -version exceeded {TIMEOUT_SECONDS} seconds", file=sys.stderr)
        except subprocess.CalledProcessError as error:
            failed = True
            print(f"ERROR: {tool} -version failed with exit status {error.returncode}", file=sys.stderr)
        except (OSError, ValueError, RuntimeError) as error:
            failed = True
            print(f"ERROR: {error}", file=sys.stderr)
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
