#!/usr/bin/env python3
"""Packaging boundary tests using synthetic, never-executed native headers."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("aster_packager", Path(__file__).with_name("package-release.py"))
PACKAGER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGER)
TARGET = "x86_64-unknown-linux-gnu"


def elf(machine=62):
    data = bytearray(128)
    data[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<HHI", data, 16, 3, machine, 1)
    struct.pack_into("<Q", data, 32, 64)
    struct.pack_into("<HHH", data, 52, 64, 56, 1)
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 5, 0, 0, 0, len(data), len(data), 4096)
    return bytes(data)


class PackageTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="aster-package-guards-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        for name in ("src", "crates", ".cargo", "target/release", "assets/fonts", "assets/icons/tabler", "artifacts", "registry/fixture-dep"):
            (self.root / name).mkdir(parents=True)
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n[package]\nname = "aster-editor"\nversion = "0.1.0"\n')
        (self.root / "Cargo.lock").write_text('version = 4\n[[package]]\nname = "aster-editor"\nversion = "0.1.0"\n[[package]]\nname = "fixture-dep"\nversion = "1.0.0"\nsource = "registry+https://example.invalid/index"\n')
        for name in ("src/main.rs", ".cargo/config.toml", "rust-toolchain.toml"):
            (self.root / name).write_text("owned source fixture\n")
        (self.root / "assets/fonts/Geist-LICENSE.txt").write_text("font notice fixture\n")
        self.tabler_notice = (Path(__file__).resolve().parents[1] / "assets/icons/tabler/LICENSE.txt").read_bytes()
        (self.root / "assets/icons/tabler/LICENSE.txt").write_bytes(self.tabler_notice)
        dependency = self.root / "registry/fixture-dep"
        (dependency / "Cargo.toml").write_text('[package]\nname = "fixture-dep"\nversion = "1.0.0"\nlicense = "MIT"\n')
        (dependency / "LICENSE-MIT").write_text("dependency notice fixture\n")
        self.record_path = self.root / "artifacts/build-record.json"
        self.metadata_path = self.root / "artifacts/cargo-metadata.json"
        self.output = self.root / "artifacts/package"
        self.record = {"profile": "release opt3", "target": TARGET, "source_files": {}, "binaries": {}}
        aggregate = hashlib.sha256()
        for name in sorted(("Cargo.toml", "Cargo.lock", "src/main.rs", ".cargo/config.toml", "rust-toolchain.toml")):
            data = (self.root / name).read_bytes()
            self.record["source_files"][name] = hashlib.sha256(data).hexdigest()
            aggregate.update(name.encode() + b"\0" + data + b"\0")
        self.record.update(source_sha256=aggregate.hexdigest(), source_files_count=5)
        for name in ("aster", "aster-agent", "aster-mcp"):
            binary = self.root / "target/release" / name
            binary.write_bytes(elf())
            self.record["binaries"][name] = {"path": str(binary), "bytes": binary.stat().st_size, "sha256": PACKAGER.digest(binary)}
        self.metadata = {"workspace_root": str(self.root), "packages": [
            {"id": "root", "name": "aster-editor", "version": "0.1.0", "source": None,
             "manifest_path": str(self.root / "Cargo.toml"), "license": None, "license_file": None},
            {"id": "dependency", "name": "fixture-dep", "version": "1.0.0", "source": "registry+https://example.invalid/index",
             "manifest_path": str(dependency / "Cargo.toml"), "license": "MIT", "license_file": None}],
            "resolve": {"root": "root", "nodes": [{"id": "root", "deps": [{"pkg": "dependency", "dep_kinds": [{"kind": None}]}]}, {"id": "dependency", "deps": []}]}}
        self.write_inputs()

    def write_inputs(self):
        self.record_path.write_text(json.dumps(self.record))
        self.metadata_path.write_text(json.dumps(self.metadata))

    def run_package(self, output=None, assets=None, media=None):
        with patch.object(PACKAGER.platform, "system", return_value="Linux"), patch.object(PACKAGER.platform, "machine", return_value="x86_64"):
            return PACKAGER.package(self.root, self.record_path, self.metadata_path, TARGET, output or self.output, assets, media)

    def media_tools(self, platform="linux-x86_64", machine=62):
        """A folder shaped as fetch-media-tools.py writes it."""
        folder = self.root / ".tools/media" / platform
        folder.mkdir(parents=True)
        (folder / "ffmpeg").write_bytes(elf(machine))
        (folder / "ffprobe").write_bytes(elf(machine) + b"probe")
        (folder / "LICENSE.txt").write_text("GPL fixture\n")
        record = {"schema": "aster-media-tools-v1", "platform": platform, "version": "8.1.3", "license": "GPL-3.0-or-later",
                  "source": "fixture", "downloads": [{"url": "https://example.invalid/ffmpeg.tar.xz", "sha256": "0" * 64}],
                  "files": {name: PACKAGER.digest(folder / name) for name in ("ffmpeg", "ffprobe", "LICENSE.txt")}}
        (folder / "media-tools.json").write_text(json.dumps(record))
        return folder

    def assert_failure(self, message, assets=None, media=None):
        with self.assertRaisesRegex(ValueError, message):
            self.run_package(assets=assets, media=media)
        report = json.loads((self.output / "package-report.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertIn(message.split("|")[0], report["failure"])

    def test_archive_is_reproducible_and_contains_only_owned_payload(self):
        (self.root / "personal-settings.json").write_text("must not be packaged")
        first = self.run_package()
        second = self.run_package(self.root / "artifacts/other-package")
        self.assertEqual(first["archive_sha256"], second["archive_sha256"])
        self.assertEqual(first["project_license"], "NOASSERTION")
        self.assertEqual(first["native_acceptance"], "not_run by this packager")
        with tarfile.open(self.output / first["archive"], "r:gz") as archive:
            members = archive.getmembers()
            self.assertTrue(all(m.isfile() or m.isdir() for m in members))
            self.assertTrue(all(m.uid == m.gid == m.mtime == 0 for m in members))
            binaries = [m for m in members if "/bin/" in m.name and m.isfile()]
            self.assertEqual({Path(m.name).name for m in binaries}, {"aster", "aster-agent", "aster-mcp"})
            self.assertTrue(all(m.mode == 0o755 for m in binaries))
            self.assertFalse(any("personal" in m.name or "ffmpeg" in m.name for m in members))
            notice = next(m for m in members if m.name.endswith("/notices/icons/Tabler-LICENSE.txt"))
            self.assertEqual(notice.mode, 0o644)
            self.assertEqual(archive.extractfile(notice).read(), self.tabler_notice)
            manifest = json.load(archive.extractfile(next(m for m in members if m.name.endswith("/manifest.json"))))
            self.assertIn("external ffmpeg", manifest["runtime_dependencies"]["codecs"])
            self.assertEqual(manifest["frontend_assets"], [])

    def test_fetched_media_tools_ship_beside_aster_with_their_license(self):
        result = self.run_package(media=self.media_tools())
        with tarfile.open(self.output / result["archive"], "r:gz") as archive:
            names = {m.name.split("/", 1)[1]: m for m in archive.getmembers() if "/" in m.name}
            for tool in ("ffmpeg", "ffprobe"):
                self.assertEqual(names[f"bin/{tool}"].mode, 0o755)
            self.assertEqual(names["notices/ffmpeg/LICENSE.txt"].mode, 0o644)
            self.assertIn("notices/ffmpeg/media-tools.json", names)
            manifest = json.load(archive.extractfile(names["manifest.json"]))
        self.assertIn("bundled FFmpeg 8.1.3", manifest["runtime_dependencies"]["codecs"])
        self.assertEqual(set(manifest["media_tools"]["executables"]), {"ffmpeg", "ffprobe"})
        self.assertEqual(result["media_tools"]["version"], "8.1.3")

    def test_media_tools_for_another_platform_are_refused(self):
        self.assert_failure("not the linux-x86_64 build", media=self.media_tools("macos-aarch64"))

    def test_media_tools_for_another_machine_are_refused(self):
        self.assert_failure("Wrong native ELF", media=self.media_tools(machine=183))

    def test_media_tool_changed_after_fetching_is_refused(self):
        folder = self.media_tools()
        (folder / "ffprobe").write_bytes(elf() + b"changed")
        self.assert_failure("Media tool changed", media=folder)

    def test_missing_embedded_tabler_notice_prevents_packaging(self):
        (self.root / "assets/icons/tabler/LICENSE.txt").unlink()
        self.assert_failure("Expected a regular file")

    def test_existing_output_is_preserved(self):
        self.output.mkdir()
        (self.output / "sentinel").write_text("keep")
        with self.assertRaises(FileExistsError):
            self.run_package()
        self.assertEqual((self.output / "sentinel").read_text(), "keep")

    def test_proprietary_project_notice_is_identified_and_copied_exactly(self):
        notice = "SPDX-License-Identifier: LicenseRef-Aster-Proprietary\nOwned private source notice.\n"
        (self.root / "LICENSE").write_text(notice)
        (self.root / "THIRD-PARTY-NOTICES.md").write_text("Separate third-party rights.\n")
        result = self.run_package()
        stage = next(path for path in self.output.iterdir() if path.is_dir())
        self.assertEqual(result["project_license"], "LicenseRef-Aster-Proprietary")
        self.assertEqual((stage / "notices/PROJECT-LICENSE").read_text(), notice)
        self.assertEqual((stage / "notices/THIRD-PARTY-NOTICES.md").read_text(), "Separate third-party rights.\n")

    def test_wrong_native_machine_is_refused_even_with_matching_hash(self):
        path = Path(self.record["binaries"]["aster"]["path"])
        path.write_bytes(elf(183))
        self.record["binaries"]["aster"]["sha256"] = PACKAGER.digest(path)
        self.write_inputs()
        self.assert_failure("Wrong native ELF")

    def test_build_binary_mutation_is_refused(self):
        Path(self.record["binaries"]["aster"]["path"]).write_bytes(elf() + b"changed")
        self.assert_failure("Verified binary changed")

    def test_new_source_file_invalidates_inventory(self):
        (self.root / "src/unrecorded.rs").write_text("unverified")
        self.assert_failure("Build source inventory")

    def test_traversing_source_path_is_refused(self):
        with self.assertRaisesRegex(ValueError, "Unsafe relative path"):
            PACKAGER.relative_name("../personal")
        with self.assertRaisesRegex(ValueError, "Unsafe relative path"):
            PACKAGER.relative_name("/absolute")

    def test_symlink_binary_is_refused(self):
        path = Path(self.record["binaries"]["aster"]["path"])
        real = path.with_name("original")
        path.rename(real)
        path.symlink_to(real)
        self.assert_failure("Symlink input/output")

    def test_unknown_dependency_is_not_packaged(self):
        self.metadata["packages"][1]["version"] = "2.0.0"
        self.write_inputs()
        self.assert_failure("Dependency is not in")

    def test_private_frontend_file_is_refused(self):
        assets = self.root / "frontend"
        assets.mkdir()
        (assets / ".env").write_text("private")
        self.assert_failure("Unexpected frontend asset", assets)

    def test_owned_frontend_assets_have_exact_permissions(self):
        assets = self.root / "frontend"
        (assets / "icons").mkdir(parents=True)
        (assets / "AppRun").write_text("owned launcher fixture; never executed")
        (assets / "Aster.desktop.in").write_text("owned desktop template fixture")
        (assets / "aster.svg").write_text("owned root icon fixture")
        (assets / "icons/aster.svg").write_text("owned icon fixture")
        result = self.run_package(assets=assets)
        stage = next(p for p in self.output.iterdir() if p.is_dir())
        self.assertEqual((stage / "AppRun").stat().st_mode & 0o777, 0o755)
        self.assertEqual((stage / "Aster.desktop.in").stat().st_mode & 0o777, 0o644)
        self.assertEqual(set(result["frontend_assets"]), {"AppRun", "Aster.desktop.in", "aster.svg", "icons/aster.svg"})

    def test_mutation_during_archive_retains_failure_instead_of_success(self):
        original = PACKAGER.archive

        def mutate(stage, destination):
            original(stage, destination)
            Path(self.record["binaries"]["aster"]["path"]).write_bytes(b"mutated after archive")

        with patch.object(PACKAGER, "archive", side_effect=mutate):
            self.assert_failure("Packaging input changed")
        self.assertTrue(any(self.output.glob("*.tar.gz")))

    def test_wrong_host_and_symlink_output_touch_no_files(self):
        with patch.object(PACKAGER.platform, "system", return_value="Windows"):
            with self.assertRaisesRegex(ValueError, "matching native host"):
                PACKAGER.package(self.root, self.record_path, self.metadata_path, TARGET, self.output)
        self.assertFalse(self.output.exists())
        alias = self.root / "artifacts/alias"
        alias.symlink_to(self.root / "artifacts", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "Symlink output"):
            self.run_package(alias / "out")
        self.assertFalse((self.root / "artifacts/out").exists())


if __name__ == "__main__":
    unittest.main()
