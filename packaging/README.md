# Packaging assets

Frontend-owned assets for the Linux portable bundle. The packager
(`scripts/package-release.py --frontend-assets packaging/linux`) copies them to
the bundle root beside `bin/`, `licenses/`, `manifest.json` and
`build-record.json`. `--media-tools` puts the FFmpeg that
`scripts/fetch-media-tools.py` fetched in `bin/` beside `aster`, with its
license in `notices/ffmpeg/`, so the bundle needs no FFmpeg on the host.

Status: prepared from source reading. The launcher has not been run, the desktop
entry has not been validated or installed, and the icon has not been rasterized
or viewed in a desktop shell. Bundles remain local-trial and unpublished while
the project has no license.

## `linux/AppRun`

POSIX `sh` launcher; mode `0755` must survive packaging and extraction.

- Resolves its own directory through symlinks (up to 32 hops, needs `readlink`),
  then `exec`s `<bundle>/bin/aster` with every argument unchanged.
- Leaves the working directory, `PATH`, `ASTER_FFMPEG` and `ASTER_FFPROBE`
  untouched, so relative path arguments resolve as typed and codec discovery is
  Aster's own: explicit override first, then the `ffmpeg` and `ffprobe` in
  `bin/` beside `aster`, then `PATH`. It adds nothing to `PATH`, reads no
  settings and installs or downloads nothing.
- Silent on success. Exits `127` with `AppRun: missing <path>` when `bin/aster`
  is absent, `126` with `AppRun: not executable: <path>` when its mode is wrong,
  and `1` when the bundle directory cannot be resolved.

Not handled: a bundle directory whose name ends in a newline.

Root checks worth running on the extracted bundle: a path with spaces and
non-ASCII characters, a symlink to `AppRun` from another directory, a relative
symlink, `sh AppRun` from inside the bundle, arguments containing spaces, `--`
and empty strings, and the override, bundled and `PATH` codec routes.

## `linux/Aster.desktop.in`

A template, not a desktop entry. A portable tar has no stable absolute path and
the Desktop Entry Specification gives `Exec` no bundle-relative form, so the tar
ships no working `.desktop` file and claims no desktop integration. Starting the
portable bundle means running `AppRun`.

A later local install step generates the entry by replacing three tokens:

| Token | Value | Escaping |
| --- | --- | --- |
| `@ASTER_APP_RUN@` | Absolute path of `AppRun` | Sits inside the template's double quotes. Apply in order: `\` → `\\\\`, `"` → `\\"`, `` ` `` → ``\\` ``, `$` → `\\$`, `%` → `%%` |
| `@ASTER_APP_RUN_PATH@` | Same path, for `TryExec` | `\` → `\\` only |
| `@ASTER_ICON@` | Absolute path of `aster.svg`, or `aster` once the icon is installed in an icon theme | `\` → `\\` only |

Reject paths containing a newline rather than escaping them. Write the result as
`aster.desktop`: the window's Wayland app ID is `aster` (`src/main.rs`), and
shells match windows to entries by that file name. `StartupWMClass=aster` covers
X11. `TryExec` hides the entry if the bundle is moved or deleted.

`%F` passes opened files as arguments; Aster accepts a project and media paths
at launch. The entry declares no `MimeType` because no `.aster` MIME type is
registered, so file managers will not offer Aster for project files yet.
`StartupNotify` is omitted because startup notification is unverified.

`desktop-file-validate` on the generated entry is a root check.

## `linux/aster.svg`

Hand-authored application icon on a 256 grid: a rounded tile in the default
accent blue (`0x0a84ff`, `src/theme.rs`), eight white ray florets and a play
shape at the centre. No text, filters, `<use>` or external references, so
simple SVG renderers can draw it.

- The tile is inset 16 px with a 50 px corner radius; the corners are
  transparent. Export square PNGs straight from the file with no extra mask or
  padding.
- Sizes to export if rasters are needed: 16, 24, 32, 48, 64, 128, 256, 512. At
  16 px each ray is about one pixel wide; review that size by eye, and draw a
  simplified small variant if it reads as noise.
- The same file can seed the Windows `.ico` and macOS `.icns` after root
  rasterizes it. macOS expects its own tile margins, so that export needs a
  separate look.
