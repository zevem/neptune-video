# Source installation

Aster currently has source, native Cargo binaries and a verified local Linux
portable trial bundle. Its original source is private/proprietary under
[the project notice](../LICENSE); third-party components retain their own terms.
Use the pinned Rust
1.97.1 toolchain, a graphical desktop and a working graphics driver. macOS and
Windows are architecture/CI targets with native acceptance still pending in
[verification](verification.md).

## Linux

The current [Linux portable trial](../artifacts/packaging/linux-attempt-02/Aster-0.1.0-x86_64-unknown-linux-gnu-local-trial.tar.gz)
contains the verified native binaries and a launcher. Extract it into a directory
you own and run `AppRun` from that directory. The launcher preserves project/media
arguments and the current working directory, and supports spaces, Unicode and
symlinks in the bundle path. It performs no installation, downloads or updates.

FFmpeg and ffprobe must be installed on normal `PATH`, or selected with
`ASTER_FFMPEG` and `ASTER_FFPROBE`. Native graphics/audio libraries and desktop
services remain host dependencies. This bundle was verified on the current Linux
host, including thirteen native editing checks through its extracted launcher;
it does not establish a minimum supported distribution or clean-machine release
acceptance. `Aster.desktop.in` is a template, not an installed menu entry.

The archive includes source/binary hashes, declared dependency licenses and
available notices. Missing dependency license texts remain recorded release work;
this is an unpublished local trial rather than an approved public installer.

To reproduce it from this checkout after a verified release build:

```sh
cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-unknown-linux-gnu > /absolute/path/cargo-metadata.json
python3 scripts/package-release.py --build-record /absolute/path/build-record.json \
  --cargo-metadata /absolute/path/cargo-metadata.json --target x86_64-unknown-linux-gnu \
  --output artifacts/packaging/new-trial --frontend-assets packaging/linux
```

The packager checks the matching host, executable architecture, exact current
source/binary identity and notice inputs. It refuses existing output directories
and retains failed attempts. macOS and Windows layouts remain preparation work;
their host/header checks cannot establish native execution on Linux.

On Debian/Ubuntu, install the native window/audio development dependencies and
the media tools:

```sh
sudo apt-get install pkg-config libxkbcommon-dev libwayland-dev libasound2-dev ffmpeg
cargo build --release --locked --bins
./target/release/aster
```

For this prepared Linux checkout, use `./scripts/run-local.sh` once the optimized
binary exists. The script forwards project/media arguments, preserves explicit
media-tool overrides and selects `.tools/ffmpeg/usr/bin` executables when they
are available; otherwise the app resolves normal `PATH`. It performs no
downloads or rebuilds. Rebuild after source changes with
`cargo build --release --locked --bins`; when the binary is missing, the script
prints that build command and exits.

Native file dialogs use the desktop's file-picker/portal service. An isolated
Xvfb server without that service can run the editor and its input tests but
cannot establish file-dialog acceptance. Test normal desktop file dialogs and
real file-manager drops before declaring Linux release support.

## macOS and Windows

On macOS, install Rust and Xcode command-line tools, then build with
`cargo build --release --locked --bins`. The executable is `target/release/aster`.
FFmpeg/ffprobe must be available to the app's environment. Native Metal, audio,
Retina/IME, Intel/Apple Silicon support policy and notarized packaging remain
acceptance work.

On Windows, use Rust's MSVC toolchain with Visual Studio C++ build tools and the
Windows SDK, then run the same Cargo build. Executables are
`target/release/aster.exe` and `target/release/aster-agent.exe`. Make FFmpeg/ffprobe
available to the launched app. Native GPU/audio, Unicode paths, DPI, file dialogs,
clipboard and installer signing remain acceptance work.

## Media executables

The Rust application decodes PNG/JPEG/WebP/BMP/GIF natively. Video/audio probing
and software processing use external `ffprobe` and `ffmpeg`, resolved from
`PATH` or `ASTER_FFPROBE` / `ASTER_FFMPEG` executable-path overrides. The verified
Linux fixture run used FFmpeg 8.0.1. Missing tools report errors; they are not
downloaded or installed automatically by Aster.

Codecs are therefore a runtime dependency, rather than a pure-Rust codec stack
or a bundled installer guarantee. Supported formats depend on the installed
FFmpeg build. Deployment, codec licensing and notices need a release policy.

Sound editing runs FFmpeg filters as well: `aresample`, `asetrate` and
`atempo` for speed and pitch; `highpass`, `equalizer`, `acompressor`,
`alimiter`, `afftdn`, and `aformat` with `pan`, for the audio effects;
`ebur128` for Normalize loudness; and the `lavfi` source `anullsrc` for the
probe. No minimum FFmpeg version is set. Once a session, before the first
sound, Aster runs a control (`anull`) and then each audio effect's own filter
string at its defaults over 50 ms of silence; a kind whose run fails is left
out of the sound, said once, and shown as "Not available in this FFmpeg
build" in the inspector. A project that holds such an effect still opens and
saves it. The probe proves the defaults only: a build that refuses another
value stops the preview's sound with FFmpeg's message. Every filter here is
in Homebrew FFmpeg 8.1.2, the only build the sound features were tested with;
FFmpeg's `rubberband` filter is not used.

Waveform peaks are kept in Aster's user cache directory (`peaks`, beside the
`stabilization` cache), or in a `peaks` folder beside the settings file when
`--settings` is given; the cache is bounded and can be deleted at any time.
Voiceover uses the system's default input device through CPAL. No packaged
build exists, so the microphone usage description a packaged macOS app must
declare has not been set up, and no microphone permission prompt has been
tried on any platform.

## Stem separator

"Separate stems" runs an external stem separator the person installs, as
FFmpeg is: Aster neither bundles nor downloads a separator, its model or its
runtime. The supported tool is the Demucs command line, version 4
(`adefossez/demucs`, the maintained fork of the archived
`facebookresearch/demucs`; 4.1.0 on PyPI, 11 July 2026), which needs Python
3.10 or later:

```sh
uv tool install demucs --with numpy     # or: python3 -m pip install -U demucs numpy
```

Demucs 4.1.0 imports NumPy without installing it: on macOS 26.5.2 with
Python 3.12, `uvx demucs --help` ended with `ModuleNotFoundError: No module
named 'numpy'` until NumPy was installed beside it. Aster then says once that
Separate stems is unavailable, with that last line.

Aster runs `demucs` from `PATH`, or the executable that `ASTER_SEPARATOR`
names, for instance a script that runs `python3 -m demucs "$@"`. Once a
session it runs the tool with `--help`, which must end well within a minute
and name `--two-stems`; otherwise "Separate stems" stays unavailable. A
separation runs

```text
demucs --two-stems vocals -n htdemucs -o "<document> media/<media> (stems).part" --filename {stem}.{ext} <media file>
```

where `<document> media` is the project's media folder beside the saved
document and `<media>` is the media's name without its extension. It takes
the `vocals.wav` and `no_vocals.wav` the tool writes, 16-bit stereo at
44.1 kHz, puts them in the media folder as `<media> (Vocals).wav` and
`<media> (Instruments).wav` (with ` 2`, ` 3`… when a name is taken; no file
is replaced), and removes the `.part` folder. Demucs fetches its pretrained
model itself the first time it runs; that network request is the tool's,
not Aster's.

The Demucs code is MIT licensed. The licence of its pretrained weights is not
settled: the author wrote in 2023 that the weights "are provided for research
purpose" (`facebookresearch/demucs` issue 508). The text of the Hugging Face
model card that now hosts them names no licence; an unanswered discussion
there says its metadata is tagged MIT, which was not checked.
Whether a given use of the weights is allowed is for the person who installs
the tool to judge; Aster recommends Demucs as a tool to install, not its
weights for any particular use.

## Launch options

The current native binary accepts:

```text
aster [OPTIONS] [PROJECT] [MEDIA]...
--agent-session PATH       Publish this instance's agent descriptor
--settings PATH            Read/write preferences at this path
--size WxH                 Initial logical window dimensions
--screenshot PATH          Capture the native window and exit
--screenshot-delay S       Capture delay, default 2.5 seconds
--help                     Print current help
--version                  Print version
```

The first `.aster` argument opens a project; remaining source-file/folder
arguments import media. Use task-owned project, settings, session and capture
paths for verification. A screenshot is native visual evidence, not proof that
file dialogs, audible output or every OS input route was exercised.
