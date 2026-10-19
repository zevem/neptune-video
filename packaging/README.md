# Packaging assets

Frontend-owned assets for the Linux portable bundle. The packager
(`scripts/package-release.py --frontend-assets packaging/linux`) copies them to
the bundle root beside `bin/`, `licenses/`, `manifest.json` and
`build-record.json`. `--media-tools` puts the FFmpeg that
`scripts/fetch-media-tools.py` fetched in `bin/` beside `neptune-video`, with its
license in `notices/ffmpeg/`, so the bundle needs no FFmpeg on the host.

Status: prepared from source reading. The launcher has not been run, the desktop
entry has not been validated or installed, and the icon has not been viewed in
a desktop shell. Bundles remain unpublished local trials.

## `linux/AppRun`

POSIX `sh` launcher; mode `0755` must survive packaging and extraction.

- Resolves its own directory through symlinks (up to 32 hops, needs `readlink`),
  then `exec`s `<bundle>/bin/neptune-video` with every argument unchanged.
- Leaves the working directory, `PATH`, `NEPTUNE_VIDEO_FFMPEG` and `NEPTUNE_VIDEO_FFPROBE`
  untouched, so relative path arguments resolve as typed and codec discovery is
  Neptune Video's own: explicit override first, then the `ffmpeg` and `ffprobe` in
  `bin/` beside `neptune-video`, then `PATH`. It adds nothing to `PATH`, reads no
  settings and installs or downloads nothing.
- Silent on success. Exits `127` with `AppRun: missing <path>` when `bin/neptune-video`
  is absent, `126` with `AppRun: not executable: <path>` when its mode is wrong,
  and `1` when the bundle directory cannot be resolved.

Not handled: a bundle directory whose name ends in a newline.

Root checks worth running on the extracted bundle: a path with spaces and
non-ASCII characters, a symlink to `AppRun` from another directory, a relative
symlink, `sh AppRun` from inside the bundle, arguments containing spaces, `--`
and empty strings, and the override, bundled and `PATH` codec routes.

## `linux/neptune-video.desktop.in`

A template, not a desktop entry. A portable tar has no stable absolute path and
the Desktop Entry Specification gives `Exec` no bundle-relative form, so the tar
ships no working `.desktop` file and claims no desktop integration. Starting the
portable bundle means running `AppRun`.

A later local install step generates the entry by replacing three tokens:

| Token | Value | Escaping |
| --- | --- | --- |
| `@NEPTUNE_VIDEO_APP_RUN@` | Absolute path of `AppRun` | Sits inside the template's double quotes. Apply in order: `\` → `\\\\`, `"` → `\\"`, `` ` `` → ``\\` ``, `$` → `\\$`, `%` → `%%` |
| `@NEPTUNE_VIDEO_APP_RUN_PATH@` | Same path, for `TryExec` | `\` → `\\` only |
| `@NEPTUNE_VIDEO_ICON@` | Absolute path of the bundle's `neptune-video.png`, or `rs.neptune.video` once the icon is installed in an icon theme | `\` → `\\` only |

Reject paths containing a newline rather than escaping them. Write the result as
`rs.neptune.video.desktop`: the window's Wayland app ID is `rs.neptune.video` (`src/main.rs`), and
shells match windows to entries by that file name. `StartupWMClass=rs.neptune.video` covers
X11. `TryExec` hides the entry if the bundle is moved or deleted.

`%F` passes opened files as arguments; Neptune Video accepts a project and media paths
at launch. The entry declares no `MimeType` because no `.nvp` MIME type is
registered, so file managers will not offer Neptune Video for project files yet.
`StartupNotify` is omitted because startup notification is unverified.

`desktop-file-validate` on the generated entry is a root check.

## Icon

The packager puts `assets/icons/neptune-video-256.png`, exported from the
supplied logo, at the bundle root as `neptune-video.png`. An icon theme install
names the exported PNGs after the application ID, for example
`assets/icons/neptune-video-256.png` as
`~/.local/share/icons/hicolor/256x256/apps/rs.neptune.video.png`, with
`Icon=rs.neptune.video` in the entry. The Windows `.ico` and macOS `.icns` sit
beside the PNGs; see [assets](../assets/README.md).
