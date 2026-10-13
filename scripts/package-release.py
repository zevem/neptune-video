#!/usr/bin/env python3
"""Create an unpublished, codec-external portable bundle from verified native bytes.

No executables are launched. Cargo metadata must be captured separately with
`cargo metadata --locked --offline --filter-platform TARGET --format-version 1`.
This is packaging evidence, not a license-compliance or native-acceptance claim.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import stat
import struct
import tarfile
import tomllib


ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "x86_64-unknown-linux-gnu": ("Linux", "x86_64", "elf", 62),
    "aarch64-apple-darwin": ("Darwin", "arm64", "macho", 0x100000C),
    "x86_64-apple-darwin": ("Darwin", "x86_64", "macho", 0x1000007),
    "x86_64-pc-windows-msvc": ("Windows", "x86_64", "pe", 0x8664),
}
NOTICE_PATTERNS = ("LICENSE*", "LICENCE*", "COPYING*", "NOTICE*", "COPYRIGHT*")


def regular(path: Path) -> Path:
    """Reject symlinks at every component, including the requested filename."""
    path = Path(os.path.abspath(path))
    for component in (path, *path.parents):
        if component.is_symlink():
            raise ValueError(f"Symlink input/output is not allowed: {component}")
    if not path.is_file() or not stat.S_ISREG(path.stat().st_mode):
        raise ValueError(f"Expected a regular file: {path}")
    return path


def digest(path: Path) -> str:
    with regular(path).open("rb") as stream:
        result = hashlib.file_digest(stream, "sha256")
    return result.hexdigest()


def relative_name(name: str) -> str:
    parsed = PurePosixPath(name)
    if not name or parsed.is_absolute() or ".." in parsed.parts or "\\" in name \
            or str(parsed) != name or any(ord(c) < 32 for c in name):
        raise ValueError(f"Unsafe relative path: {name!r}")
    return name


def source_snapshot(root: Path, record: dict) -> dict:
    expected = record["source_files"]
    inventory = {str(p.relative_to(root)) for parent in (root / "src", root / "crates")
                 for p in parent.rglob("*.rs")}
    inventory.update(str(p.relative_to(root)) for p in (root / "crates").rglob("Cargo.toml"))
    inventory.update(("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml"))
    if set(expected) != inventory or record.get("source_files_count") != len(inventory):
        raise ValueError("Build source inventory does not match this checkout")
    result, aggregate = {}, hashlib.sha256()
    for name in sorted(expected):
        data = regular(root / relative_name(name)).read_bytes()
        result[name] = hashlib.sha256(data).hexdigest()
        aggregate.update(name.encode() + b"\0" + data + b"\0")
    if result != expected or aggregate.hexdigest() != record["source_sha256"]:
        raise ValueError("Source bytes differ from the verified build record")
    return {"source_sha256": aggregate.hexdigest(), "source_files": result}


def native_identity(path: Path, target: str) -> dict:
    """Inspect headers without asking a loader to execute untrusted binaries."""
    data = regular(path).read_bytes()
    kind, machine = TARGETS[target][2:]
    if kind == "elf":
        if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01" \
                or struct.unpack_from("<H", data, 18)[0] != machine \
                or struct.unpack_from("<H", data, 16)[0] not in (2, 3):
            raise ValueError(f"Wrong native ELF executable: {path.name}")
        phoff, phsize, phnum = struct.unpack_from("<Q", data, 32)[0], \
            struct.unpack_from("<H", data, 54)[0], struct.unpack_from("<H", data, 56)[0]
        if phnum == 0 or phnum > 4096 or phsize != 56 or phoff + phnum * phsize > len(data):
            raise ValueError("Malformed ELF program headers")
        loads, dynamic, interpreter = [], None, None
        for index in range(phnum):
            values = struct.unpack_from("<IIQQQQQQ", data, phoff + index * phsize)
            tag, _, offset, address, _, size, _, _ = values
            if offset + size > len(data):
                raise ValueError("ELF segment exceeds executable")
            if tag == 1:
                loads.append((address, offset, size))
            elif tag == 2:
                dynamic = (offset, size)
            elif tag == 3:
                interpreter = data[offset:offset + size].rstrip(b"\0").decode("utf-8")
        needed = []
        if dynamic:
            offset, size = dynamic
            if size % 16 or size > 131072:
                raise ValueError("Malformed ELF dynamic table")
            entries = []
            for cursor in range(offset, offset + size, 16):
                tag, value = struct.unpack_from("<qQ", data, cursor)
                if tag == 0:
                    break
                entries.append((tag, value))
            address = next((v for t, v in entries if t == 5), None)
            strsize = next((v for t, v in entries if t == 10), 0)
            string_offset = next((o + address - a for a, o, s in loads
                                  if address is not None and a <= address < a + s), None)
            if string_offset is None or string_offset + strsize > len(data):
                raise ValueError("Malformed ELF dynamic strings")
            strings = data[string_offset:string_offset + strsize]
            for tag, value in entries:
                if tag == 1:
                    end = strings.find(b"\0", value)
                    if value >= len(strings) or end < 0:
                        raise ValueError("Malformed ELF dependency name")
                    needed.append(strings[value:end].decode("utf-8"))
        return {"format": kind, "machine": machine, "interpreter": interpreter,
                "linked_sonames": sorted(set(needed)),
                "runtime_loaded_libraries": "not enumerated; window/audio/graphics services remain host-owned"}
    if kind == "pe":
        if len(data) < 64 or data[:2] != b"MZ":
            raise ValueError("Wrong native PE executable")
        offset = struct.unpack_from("<I", data, 60)[0]
        if offset + 26 > len(data) or data[offset:offset + 4] != b"PE\0\0" \
                or struct.unpack_from("<H", data, offset + 4)[0] != machine \
                or struct.unpack_from("<H", data, offset + 24)[0] != 0x20B:
            raise ValueError("Wrong native PE architecture")
    elif len(data) < 32 or data[:4] != b"\xcf\xfa\xed\xfe" \
            or struct.unpack_from("<I", data, 4)[0] != machine \
            or struct.unpack_from("<I", data, 12)[0] != 2:
        raise ValueError("Wrong native Mach-O executable")
    return {"format": kind, "machine": machine, "linked_dependencies": "not enumerated"}


def put(source: Path, destination: Path, inputs: dict, executable: bool = False) -> None:
    source = regular(source)
    expected = digest(source)
    inputs[source] = expected
    destination.parent.mkdir(parents=True, exist_ok=True)
    with source.open("rb") as incoming, destination.open("xb") as outgoing:
        shutil.copyfileobj(incoming, outgoing)
    destination.chmod(0o755 if executable else 0o644)
    if digest(destination) != expected:
        raise ValueError(f"Copied input changed: {source}")


def save(path: Path, value: dict | list) -> None:
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")
    path.chmod(0o644)


def dependencies(root: Path, metadata: dict, stage: Path, inputs: dict) -> list:
    if Path(metadata["workspace_root"]) != root:
        raise ValueError("Cargo metadata belongs to another workspace")
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {p["id"]: p for p in metadata["resolve"]["nodes"]}
    root_id = metadata["resolve"]["root"]
    if Path(packages[root_id]["manifest_path"]) != root / "Cargo.toml":
        raise ValueError("Cargo metadata has the wrong root package")
    locked = tomllib.loads(regular(root / "Cargo.lock").read_text())["package"]
    locked_ids = {(p["name"], p["version"], p.get("source")) for p in locked}
    pending, selected = [root_id], set()
    while pending:
        package_id = pending.pop()
        if package_id in selected:
            continue
        selected.add(package_id)
        for dep in nodes[package_id]["deps"]:
            if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]):
                pending.append(dep["pkg"])
    result = []
    for package_id in sorted(selected, key=lambda key: (packages[key]["name"], packages[key]["version"])):
        package = packages[package_id]
        name, version = package["name"], package["version"]
        if not re.fullmatch(r"[A-Za-z0-9_.+-]+", name + version):
            raise ValueError("Unsafe dependency name/version")
        if (name, version, package.get("source")) not in locked_ids:
            raise ValueError(f"Dependency is not in the verified Cargo.lock: {name} {version}")
        manifest = regular(Path(package["manifest_path"]))
        inputs[manifest] = digest(manifest)
        package_root = manifest.parent
        paths = {p for pattern in NOTICE_PATTERNS for p in package_root.glob(pattern) if p.is_file()}
        if package.get("license_file"):
            paths.add(package_root / relative_name(package["license_file"]))
        notices = []
        for path in sorted(paths):
            filename = relative_name(path.name)
            destination = stage / "notices" / "cargo" / f"{name}-{version}" / filename
            put(path, destination, inputs)
            notices.append(str(destination.relative_to(stage)))
        result.append({"name": name, "version": version,
                       "declared_license_expression": package.get("license") or "NOASSERTION",
                       "license_files": notices,
                       "notice_status": "texts collected, compliance unreviewed" if notices else "no packaged license text found"})
    return result


def archive(stage: Path, destination: Path) -> None:
    """Fixed ordering, permissions, owner and gzip/tar times; no host metadata."""
    with destination.open("xb") as output, gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as stream:
            for path in (stage, *sorted(stage.rglob("*"))):
                name = str(path.relative_to(stage.parent))
                relative_name(name)
                info = tarfile.TarInfo(name)
                info.mode = 0o755 if path.is_dir() or path.stat().st_mode & 0o111 else 0o644
                if path.is_dir():
                    info.type = tarfile.DIRTYPE
                    stream.addfile(info)
                else:
                    regular(path)
                    info.size = path.stat().st_size
                    with path.open("rb") as data:
                        stream.addfile(info, data)


def verify_archive(stage: Path, destination: Path) -> None:
    expected = {str(p.relative_to(stage.parent)): p for p in (stage, *stage.rglob("*"))}
    seen = set()
    with tarfile.open(destination, "r:gz") as stream:
        for member in stream:
            relative_name(member.name)
            if member.name in seen or member.name not in expected or not (member.isfile() or member.isdir()):
                raise ValueError("Unexpected or unsafe archive member")
            seen.add(member.name)
            path = expected[member.name]
            mode = 0o755 if path.is_dir() or path.stat().st_mode & 0o111 else 0o644
            if member.mode != mode or member.uid != 0 or member.gid != 0 or member.mtime != 0:
                raise ValueError("Unexpected archive permissions or metadata")
            if member.isdir() != path.is_dir():
                raise ValueError("Archive member type changed")
            if member.isfile():
                with stream.extractfile(member) as data:
                    if hashlib.file_digest(data, "sha256").hexdigest() != digest(path):
                        raise ValueError("Archived bytes differ from staged package")
    if seen != set(expected):
        raise ValueError("Archive is missing package content")


def package(root: Path, build_record: Path, cargo_metadata: Path, target: str,
            output: Path, frontend_assets: Path | None = None) -> dict:
    root, output = Path(os.path.abspath(root)), Path(os.path.abspath(output))
    if not any(output.is_relative_to(root / parent) and output != root / parent for parent in ("artifacts", "dist")):
        raise ValueError("Output must be a fresh directory below artifacts/ or dist/")
    for path in (output, *output.parents):
        if path.is_symlink():
            raise ValueError("Symlink output is not allowed")
    expected_os, expected_arch = TARGETS[target][:2]
    host_arch = {"amd64": "x86_64", "aarch64": "arm64"}.get(platform.machine().lower(), platform.machine().lower())
    if (platform.system(), host_arch) != (expected_os, expected_arch):
        raise ValueError("Packaging requires the matching native host; cross-target acceptance is not inferred")
    output.mkdir(parents=True, exist_ok=False)
    report = {"status": "failed", "target": target, "publication": "local trial; unpublished",
              "native_acceptance": "not_run by this packager"}
    try:
        inputs = {regular(build_record): digest(build_record), regular(cargo_metadata): digest(cargo_metadata),
                  regular(Path(__file__)): digest(Path(__file__))}
        record = json.loads(build_record.read_text())
        before = source_snapshot(root, record)
        if set(record["binaries"]) != {"aster", "aster-agent"} or not str(record.get("profile", "")).startswith("release"):
            raise ValueError("A verified release record with both application binaries is required")
        if record.get("target", target) != target:
            raise ValueError("Build record target differs from package target")
        version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.+-]+)?", version):
            raise ValueError("Unsafe package version")
        stage = output / f"Aster-{version}-{target}-local-trial"
        stage.mkdir()
        binaries = {}
        for name, entry in sorted(record["binaries"].items()):
            source = regular(Path(entry["path"]))
            if not source.is_relative_to(root / "target") or digest(source) != entry["sha256"] or source.stat().st_size != entry["bytes"]:
                raise ValueError(f"Verified binary changed or is outside target/: {name}")
            identity = native_identity(source, target)
            filename = name + (".exe" if expected_os == "Windows" else "")
            put(source, stage / "bin" / filename, inputs, executable=True)
            binaries[name] = {"path": f"bin/{filename}", "sha256": entry["sha256"], "bytes": entry["bytes"], **identity}
        put(build_record, stage / "build-record.json", inputs)
        dependency_records = dependencies(root, json.loads(cargo_metadata.read_text()), stage, inputs)
        save(stage / "dependencies.json", {"scope": "target-filtered Cargo normal/build graph; includes build-time dependencies, not an exact linked-code SBOM",
                                            "packages": dependency_records})
        put(root / "assets/fonts/Geist-LICENSE.txt", stage / "notices/fonts/Geist-LICENSE.txt", inputs)
        project_license = "NOASSERTION"
        if (root / "LICENSE").exists():
            put(root / "LICENSE", stage / "notices/PROJECT-LICENSE", inputs)
            project_license = "see notices/PROJECT-LICENSE; license identification unreviewed"
            identifier = re.search(r"^SPDX-License-Identifier: (LicenseRef-[A-Za-z0-9.-]+)$", (root / "LICENSE").read_text(), re.MULTILINE)
            if identifier:
                project_license = identifier.group(1)
        if (root / "THIRD-PARTY-NOTICES.md").exists():
            put(root / "THIRD-PARTY-NOTICES.md", stage / "notices/THIRD-PARTY-NOTICES.md", inputs)
        frontend_files = []
        if frontend_assets is not None:
            frontend_assets = Path(os.path.abspath(frontend_assets))
            if not frontend_assets.is_dir() or frontend_assets.is_symlink():
                raise ValueError("Frontend assets must be a regular directory")
            paths = sorted(frontend_assets.rglob("*"))
            if any(p.is_symlink() for p in paths):
                raise ValueError("Symlink frontend assets are not allowed")
            for path in paths:
                if path.is_dir():
                    continue
                name = relative_name(str(path.relative_to(frontend_assets)))
                allowed = name in {"AppRun", "Aster.desktop", "Aster.desktop.in", "aster.svg", "README.md", "install-user.sh", "uninstall-user.sh"} \
                    or (name.startswith("icons/") and path.suffix in (".png", ".svg") and not any(part.startswith(".") for part in Path(name).parts))
                if not allowed:
                    raise ValueError(f"Unexpected frontend asset path: {name}")
                put(path, stage / name, inputs, executable=name in {"AppRun", "install-user.sh", "uninstall-user.sh"})
                frontend_files.append(name)
        files = {str(p.relative_to(stage)): {"sha256": digest(p), "bytes": p.stat().st_size,
                 "mode": oct(stat.S_IMODE(p.stat().st_mode))} for p in sorted(stage.rglob("*")) if p.is_file()}
        manifest = {"schema": "aster-portable-package-v1", "version": version, "target": target,
                    "publication": "local trial; unpublished; no public license-compliance claim",
                    "project_license": project_license, "source": before,
                    "inputs": {"build_record_sha256": digest(build_record), "cargo_metadata_sha256": digest(cargo_metadata),
                               "packager_sha256": digest(Path(__file__))},
                    "binaries": binaries, "files": files, "frontend_assets": frontend_files,
                    "desktop_integration": "portable assets only; no desktop installation or native acceptance" if frontend_files else "absent; launch bin/aster directly",
                    "runtime_dependencies": {"codecs": "external ffmpeg and ffprobe through PATH or ASTER_FFMPEG/ASTER_FFPROBE; no codecs bundled",
                                             "graphics_audio_desktop": "host-owned; linked dependencies above exclude dynamically loaded services"},
                    "update_policy": "manual side-by-side extraction; no automatic updates or changes to user storage",
                    "native_acceptance": "not_run by this packager"}
        save(stage / "manifest.json", manifest)
        destination = output / f"{stage.name}.tar.gz"
        archive(stage, destination)
        if source_snapshot(root, record) != before or any(digest(path) != value for path, value in inputs.items()):
            raise ValueError("Packaging input changed during staging/archive creation")
        # Re-read staged bytes too: a concurrent mutation must never produce a successful report.
        for name, entry in files.items():
            if digest(stage / name) != entry["sha256"]:
                raise ValueError("Staged package changed during archive creation")
        verify_archive(stage, destination)
        report.update(status="packaged", archive=destination.name, archive_sha256=digest(destination),
                      archive_bytes=destination.stat().st_size, manifest_sha256=digest(stage / "manifest.json"),
                      source_sha256=before["source_sha256"], binaries=binaries,
                      license_texts_missing=sum(not p["license_files"] for p in dependency_records),
                      project_license=project_license, frontend_assets=frontend_files)
    except (OSError, ValueError, KeyError, TypeError, struct.error) as error:
        report["failure"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        save(output / "package-report.json", report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-record", type=Path, required=True)
    parser.add_argument("--cargo-metadata", type=Path, required=True)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--frontend-assets", type=Path)
    arguments = parser.parse_args()
    try:
        report = package(ROOT, arguments.build_record, arguments.cargo_metadata,
                         arguments.target, arguments.output, arguments.frontend_assets)
    except (OSError, ValueError, KeyError, TypeError, struct.error) as error:
        print(json.dumps({"status": "failed", "failure": f"{type(error).__name__}: {error}"}))
        return 1
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
