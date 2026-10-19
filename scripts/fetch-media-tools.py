#!/usr/bin/env python3
"""Fetch the pinned FFmpeg build Neptune Video ships for one platform.

Every download is checked against a pinned SHA-256 digest before anything is
read from it, and only the named files are taken out. A download that times
out or loses its connection is tried again, up to three times; one that
arrives with another digest is not. The result -- `ffmpeg`,
`ffprobe`, the build's license and `media-tools.json` recording where they
came from -- goes to `.tools/media/<os>-<arch>`, where a Neptune Video built in this
checkout finds it, or to `--output`. `package-release.py --media-tools` copies
that folder into a package, so an installed Neptune Video needs no FFmpeg of its own.

The builds are static GPL builds: export needs libx264 and libx265.
"""

from __future__ import annotations

import argparse
from collections.abc import Iterator
import contextlib
import hashlib
import http.client
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import zipfile


ROOT = Path(__file__).resolve().parents[1]
BTBN = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-30-13-08"
BTBN_BUILD = "ffmpeg-n8.1.3-9-g29e619e767"
RIEDL = "https://ffmpeg.martin-riedl.de/download/macos"
GPL3 = {"url": "https://www.gnu.org/licenses/gpl-3.0.txt",
        "sha256": "3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986"}


def btbn(platform_name: str, archive_suffix: str, sha256: str, exe: str = "") -> dict:
    stem = f"{BTBN_BUILD}-{platform_name}-gpl-8.1"
    return {
        "version": "8.1.3-9-g29e619e767",
        "license": "GPL-3.0-or-later",
        "source": "https://git.ffmpeg.org/ffmpeg.git (commit 29e619e767); build recipe https://github.com/BtbN/FFmpeg-Builds",
        "downloads": [{
            "url": f"{BTBN}/{stem}.{archive_suffix}", "sha256": sha256,
            "members": {f"ffmpeg{exe}": f"{stem}/bin/ffmpeg{exe}",
                        f"ffprobe{exe}": f"{stem}/bin/ffprobe{exe}",
                        "LICENSE.txt": f"{stem}/LICENSE.txt"},
        }],
    }


def riedl(arch: str, build: str, ffmpeg: str, ffprobe: str) -> dict:
    return {
        "version": "9.0.2",
        "license": "GPL-3.0-or-later",
        "source": "https://git.ffmpeg.org/ffmpeg.git (tag n9.0.2); build recipe https://gitlab.com/martinr92/ffmpeg",
        "downloads": [
            {"url": f"{RIEDL}/{arch}/{build}/ffmpeg.zip", "sha256": ffmpeg, "members": {"ffmpeg": "ffmpeg"}},
            {"url": f"{RIEDL}/{arch}/{build}/ffprobe.zip", "sha256": ffprobe, "members": {"ffprobe": "ffprobe"}},
            {**GPL3, "members": {"LICENSE.txt": None}},
        ],
    }


# Keyed `<os>-<arch>` as Rust's `std::env::consts` name them, which is the
# folder the application looks in.
PINS = {
    "linux-x86_64": btbn("linux64", "tar.xz", "97ce978979194b5cf7e06a5e68020dbdaa7a4f3294c5452b6a1bc347100dbb79"),
    "linux-aarch64": btbn("linuxarm64", "tar.xz", "10523d1e0be6b3ce62a9708cc50815c4b5eb5f36ae147fbe2afeecda6f2d6845"),
    "windows-x86_64": btbn("win64", "zip", "7b801cdd3a1a0bb54ae6f572187e68b4ed54f52086cfee133fab2180bfb429fa", ".exe"),
    "windows-aarch64": btbn("winarm64", "zip", "061a770557c30bbf8e0b6ca3810288a60f72b85088378b84be03277746f34f17", ".exe"),
    "macos-aarch64": riedl("arm64", "1789931890_9.0.2",
                           "c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924",
                           "fcbe839537485eaee7a7a8bc5cbc0f90d53617e80943e8a5b2e31cb851197ea6"),
    "macos-x86_64": riedl("amd64", "1789931006_9.0.2",
                          "7c6b4125b191cbf773832dc51f424cf2b6bb7da43007d1e066f95909e47cacd4",
                          "2322438ed2f6319a691291b247d09c69dcaa3a982460d1f269a7e1af335cfdfd"),
}
RECORD = "media-tools.json"
CHUNK = 1 << 20


def host_platform() -> str:
    system = {"Linux": "linux", "Darwin": "macos", "Windows": "windows"}.get(platform.system(), platform.system().lower())
    machine = platform.machine().lower()
    machine = {"amd64": "x86_64", "x64": "x86_64", "arm64": "aarch64"}.get(machine, machine)
    return f"{system}-{machine}"


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def download(url: str, expected: str, destination: Path, attempts: int = 3, pause: float = 10) -> None:
    request = urllib.request.Request(url, headers={"User-Agent": "neptune-video-fetch-media-tools"})
    for attempt in range(1, attempts + 1):
        hasher = hashlib.sha256()
        try:
            # The file is made before connecting, so what an attempt leaves
            # is its own, and something already there is never replaced.
            with destination.open("xb") as output, urllib.request.urlopen(request, timeout=60) as response:
                while chunk := response.read(CHUNK):
                    hasher.update(chunk)
                    output.write(chunk)
            break
        except (urllib.error.URLError, TimeoutError, ConnectionError, http.client.HTTPException) as error:
            destination.unlink()
            if attempt == attempts:
                raise
            print(f"{url}: {error}; trying again", file=sys.stderr)
            time.sleep(pause * attempt)
    if hasher.hexdigest() != expected:
        raise ValueError(f"{url} has SHA-256 {hasher.hexdigest()}, expected {expected}")


@contextlib.contextmanager
def take(archive: Path, member: str) -> Iterator[io.BufferedIOBase]:
    """Open one regular file, by its exact name, inside a verified archive.

    The archive closes with the file, so Windows can delete it afterwards.
    """
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as bundle:
            info = bundle.getinfo(member)
            if info.is_dir():
                raise ValueError(f"{member} is not a file in {archive.name}")
            with bundle.open(info) as source:
                yield source
        return
    with tarfile.open(archive, "r:*") as bundle:
        info = bundle.getmember(member)
        if not info.isfile():
            raise ValueError(f"{member} is not a regular file in {archive.name}")
        with bundle.extractfile(info) as source:
            yield source


def fetch(key: str, output: Path) -> dict:
    pin = PINS[key]
    names = [name for download_pin in pin["downloads"] for name in download_pin["members"]]
    if any(PurePosixPath(name).name != name for name in names):
        raise ValueError("Pinned output names must be plain file names")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".media-tools-", dir=output.parent) as scratch:
        scratch = Path(scratch)
        stage = scratch / "stage"
        stage.mkdir()
        files = {}
        for index, download_pin in enumerate(pin["downloads"]):
            filename = download_pin["url"].rsplit("/", 1)[1]
            archive = scratch / f"{index}-{filename}"
            print(f"Downloading {download_pin['url']}", file=sys.stderr)
            download(download_pin["url"], download_pin["sha256"], archive)
            for name, member in download_pin["members"].items():
                destination = stage / name
                if member is None:
                    shutil.copyfile(archive, destination)
                else:
                    with take(archive, member) as source, destination.open("xb") as target:
                        shutil.copyfileobj(source, target, CHUNK)
                destination.chmod(0o644 if name == "LICENSE.txt" else 0o755)
                files[name] = digest(destination)
        record = {"schema": "neptune-video-media-tools-v1", "platform": key, "version": pin["version"],
                  "license": pin["license"], "source": pin["source"],
                  "downloads": [{"url": d["url"], "sha256": d["sha256"]} for d in pin["downloads"]],
                  "files": files}
        with (stage / RECORD).open("x", encoding="utf-8") as stream:
            json.dump(record, stream, indent=2, sort_keys=True)
            stream.write("\n")
        (stage / RECORD).chmod(0o644)
        # Replace an earlier fetch only once this one is whole.
        if output.exists():
            if not (output / RECORD).is_file():
                raise ValueError(f"{output} exists and was not made by this script; refusing to replace it")
            previous = scratch / "previous"
            output.rename(previous)
        stage.rename(output)
    return record


def current(output: Path, key: str) -> bool:
    """Whether `output` already holds exactly this pin's files."""
    try:
        record = json.loads((output / RECORD).read_text())
    except (OSError, ValueError):
        return False
    pin = PINS[key]
    return record.get("platform") == key and record.get("version") == pin["version"] \
        and [d["sha256"] for d in record.get("downloads", [])] == [d["sha256"] for d in pin["downloads"]] \
        and all((output / name).is_file() and digest(output / name) == value
                for name, value in record.get("files", {}).items())


def check(output: Path, key: str) -> None:
    """Run both tools once when they are built for this computer."""
    if key != host_platform():
        print(f"Not running {key} tools on {host_platform()}", file=sys.stderr)
        return
    suffix = ".exe" if key.startswith("windows") else ""
    for tool in ("ffmpeg", "ffprobe"):
        result = subprocess.run([str(output / f"{tool}{suffix}"), "-version"], stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=10, check=True)
        banner = result.stdout.decode("utf-8", "replace").splitlines()[0]
        if not banner.startswith(f"{tool} version "):
            raise ValueError(f"{tool} did not report a version banner")
        print(banner)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--platform", choices=sorted(PINS), default=host_platform(),
                        help="<os>-<arch> to fetch for (default: this computer)")
    parser.add_argument("--output", type=Path, help="folder to write (default: .tools/media/<platform>)")
    arguments = parser.parse_args()
    if arguments.platform not in PINS:
        print(f"ERROR: no FFmpeg build is pinned for {arguments.platform}", file=sys.stderr)
        return 1
    output = Path(os.path.abspath(arguments.output or ROOT / ".tools" / "media" / arguments.platform))
    try:
        if current(output, arguments.platform):
            print(f"{output} already holds the pinned FFmpeg {PINS[arguments.platform]['version']}", file=sys.stderr)
        else:
            fetch(arguments.platform, output)
            print(f"Wrote FFmpeg {PINS[arguments.platform]['version']} to {output}", file=sys.stderr)
        check(output, arguments.platform)
    except (OSError, ValueError, KeyError, tarfile.TarError, zipfile.BadZipFile, subprocess.SubprocessError) as error:
        print(f"ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
