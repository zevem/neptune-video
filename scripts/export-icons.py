#!/usr/bin/env python3
"""Export Neptune Video's native icons from the supplied artwork.

Requires Pillow (python3 -m pip install Pillow). The logo supplies the Linux
and Windows PNGs and the Windows ICO; the macOS logo supplies the ICNS with its
own padding, adding none. The originals in assets/branding are never written.
"""

from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)


def main() -> None:
    branding = ROOT / "assets/branding"
    icons = ROOT / "assets/icons"
    icons.mkdir(parents=True, exist_ok=True)
    with Image.open(branding / "neptune-video-logo.png") as source:
        logo = source.convert("RGBA")
    with Image.open(branding / "neptune-video-macos-logo.png") as source:
        macos = source.convert("RGBA")
    for size in SIZES:
        logo.resize((size, size), Image.Resampling.LANCZOS).save(
            icons / f"neptune-video-{size}.png", optimize=True
        )
    logo.save(icons / "neptune-video.ico", sizes=[(size, size) for size in SIZES if size <= 256])
    macos.save(icons / "neptune-video.icns")


if __name__ == "__main__":
    main()
