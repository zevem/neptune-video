# Assets

Project artwork, icons and bundled fonts live here.

The supplied originals have distinct uses:

| Source | Use |
| --- | --- |
| [`branding/neptune-video-logo.png`](branding/neptune-video-logo.png) | Linux and Windows app and taskbar icons, and the application mark in the toolbar |
| [`branding/neptune-video-macos-logo.png`](branding/neptune-video-macos-logo.png) | macOS Finder and Dock icon, with its supplied padding preserved |
| [`branding/neptune-video-icon.png`](branding/neptune-video-icon.png) | README, website and sharing |

- `icons/` contains Linux and Windows PNGs from 16 to 1024 pixels and a Windows
  ICO with sizes through 256 pixels, exported from `neptune-video-logo.png`. The
  macOS ICNS has Retina sizes through 1024 pixels, exported from
  `neptune-video-macos-logo.png` without adding padding.
- [`icons/tabler/`](icons/tabler/README.md) contains the pinned Tabler native UI
  font subsets, official SVGs, mapping and MIT notice. The supplied branding is
  not part of that set.
- `fonts/` contains the bundled Geist typefaces and their license.

Regenerate the icons from the repository root; the originals are never written:

```sh
python3 -m pip install Pillow
python3 scripts/export-icons.py
python3 scripts/test-export-icons.py
```

Linux and Windows windows load `icons/neptune-video-256.png`, which the toolbar's
application mark also draws; Windows builds embed `icons/neptune-video.ico` in
the executables. The running macOS Dock icon loads
`branding/neptune-video-macos-logo.png`, the same artwork and padding as
`icons/neptune-video.icns`, so opening the app does not enlarge the icon. The
Linux bundle carries `icons/neptune-video-256.png` as `neptune-video.png`; see
[packaging](../packaging/README.md).
