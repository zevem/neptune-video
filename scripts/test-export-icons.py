#!/usr/bin/env python3
"""The supplied artwork stays as supplied and the native icons come from it.

Requires Pillow. Run from anywhere: python3 scripts/test-export-icons.py
"""

import hashlib
import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("export_icons", Path(__file__).with_name("export-icons.py"))
export_icons = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(export_icons)

# The originals as supplied on 2026-10-19.
ORIGINALS = {
    "neptune-video-icon.png": "d5727d1574de290b6a58ed9f0e76421cb644b690e55a48847331deb896939767",
    "neptune-video-logo.png": "f3c8356fb4e7181c4b7cf8940375e1dbfc44f706009ee28c2876e24765778b54",
    "neptune-video-macos-logo.png": "f5b92262a7251eee7e47c6008c6480d66eee33acafa64ec2fe352d6358a4fe83",
}


class ExportIconsTests(unittest.TestCase):
    def test_supplied_artwork_is_unchanged(self):
        for name, expected in ORIGINALS.items():
            with self.subTest(name=name):
                source = ROOT / "assets/branding" / name
                self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), expected)

    def test_committed_icons_are_the_export_of_the_artwork(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copytree(ROOT / "assets/branding", root / "assets/branding")
            with patch.object(export_icons, "ROOT", root):
                export_icons.main()
            outputs = sorted((root / "assets/icons").iterdir())
            self.assertEqual(len(outputs), len(export_icons.SIZES) + 2)
            for path in outputs:
                relative = path.relative_to(root)
                with self.subTest(path=str(relative)):
                    self.assertEqual(path.read_bytes(), (ROOT / relative).read_bytes())
            for name in ORIGINALS:
                self.assertEqual((root / "assets/branding" / name).read_bytes(),
                                 (ROOT / "assets/branding" / name).read_bytes())


if __name__ == "__main__":
    unittest.main()
