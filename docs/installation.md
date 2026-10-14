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
It was built before titles, captions, speech recognition, export and proxies
were added, so it has none of them; build from source for those.

That trial bundle predates bundled media tools: it needs FFmpeg and ffprobe on
normal `PATH`, or selected with `ASTER_FFMPEG` and `ASTER_FFPROBE`. Bundles
packaged with `--media-tools` carry them in `bin/` (see
[media executables](#media-executables)). Native graphics/audio libraries and
desktop services remain host dependencies. This bundle was verified on the current Linux
host, including thirteen native editing checks through its extracted launcher;
it does not establish a minimum supported distribution or clean-machine release
acceptance. `Aster.desktop.in` is a template, not an installed menu entry.

The archive includes source/binary hashes, declared dependency licenses and
available notices. Missing dependency license texts remain recorded release work;
this is an unpublished local trial rather than an approved public installer.

To reproduce it from this checkout after a verified release build:

```sh
cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-unknown-linux-gnu > /absolute/path/cargo-metadata.json
python3 scripts/fetch-media-tools.py
python3 scripts/package-release.py --build-record /absolute/path/build-record.json \
  --cargo-metadata /absolute/path/cargo-metadata.json --target x86_64-unknown-linux-gnu \
  --output artifacts/packaging/new-trial --frontend-assets packaging/linux \
  --media-tools .tools/media/linux-x86_64
```

The packager checks the matching host, executable architecture, exact current
source/binary identity and notice inputs. It refuses existing output directories
and retains failed attempts. macOS and Windows layouts remain preparation work;
their host/header checks cannot establish native execution on Linux.

On Debian/Ubuntu, install the native window/audio development dependencies,
fetch the FFmpeg Aster ships, and build:

```sh
sudo apt-get install pkg-config libxkbcommon-dev libwayland-dev libasound2-dev \
  build-essential cmake libclang-dev
python3 scripts/fetch-media-tools.py
cargo build --release --locked --bins
./target/release/aster
```

`build-essential`, `cmake` and `libclang-dev` build the speech recognizer (see
[speech recognition](#speech-recognition)); the first build compiles it from
source and takes a few minutes longer.

Alternatively, use `./scripts/run-local.sh` once the optimized binary exists.
The script forwards project/media arguments and runs `fetch-media-tools.py`
first, which downloads only when `.tools/media/` lacks the pinned build;
explicit media-tool overrides skip it. It performs no rebuilds. Rebuild after source changes with
`cargo build --release --locked --bins`; when the binary is missing, the script
prints that build command and exits.

Native file dialogs use the desktop's file-picker/portal service. An isolated
Xvfb server without that service can run the editor and its input tests but
cannot establish file-dialog acceptance. Test normal desktop file dialogs and
real file-manager drops before declaring Linux release support.

## macOS and Windows

On macOS, install Rust, the Xcode command-line tools and CMake
(`brew install cmake`), run `python3 scripts/fetch-media-tools.py`, then build
with `cargo build --release --locked --bins`. The executable is
`target/release/aster`. Native Metal, audio,
Retina/IME, Intel/Apple Silicon support policy and notarized packaging remain
acceptance work.

On Windows, use Rust's MSVC toolchain with Visual Studio C++ build tools, their
CMake component, the Windows SDK and LLVM's libclang (set `LIBCLANG_PATH` to its
`bin` directory if it is not found), then run the same Cargo build. Executables are
`target/release/aster.exe` and `target/release/aster-agent.exe`. Run
`python scripts/fetch-media-tools.py` once for the FFmpeg the app runs. Native GPU/audio, Unicode paths, DPI, file dialogs,
clipboard and installer signing remain acceptance work.

## Media executables

The Rust application decodes PNG/JPEG/WebP/BMP/GIF natively. Video/audio probing
and software processing run `ffprobe` and `ffmpeg` as separate processes. Aster
ships them, so a person installing it installs no FFmpeg. Each is resolved, in
order, from:

1. `ASTER_FFMPEG` / `ASTER_FFPROBE`, an explicit executable path;
2. the copy in the same folder as the `aster` executable (symlinks to `aster`
   followed), where a package puts it;
3. for a build inside a source checkout (an executable below `target/`),
   `.tools/media/<os>-<arch>/` in that checkout;
4. `PATH`.

A tool found nowhere reports an error naming these places. Aster itself never
downloads FFmpeg.

[`scripts/fetch-media-tools.py`](../scripts/fetch-media-tools.py) fetches the
pinned build for a platform, checks it against a pinned SHA-256 before reading
it, and writes `ffmpeg`, `ffprobe`, the build's license and a provenance record
(`media-tools.json`) to `.tools/media/<os>-<arch>`. `package-release.py
--media-tools` copies those into the package's `bin/` and `notices/ffmpeg/`.
The pins are static GPL builds, because export uses libx264 and libx265:

| Platform | Build | Source |
| --- | --- | --- |
| `linux-x86_64`, `linux-aarch64`, `windows-x86_64`, `windows-aarch64` | FFmpeg 8.1.3-9-g29e619e767 | [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) `autobuild-2026-09-30-13-08`, a month-end build that project keeps |
| `macos-aarch64`, `macos-x86_64` | FFmpeg 9.0.2 | [martin-riedl.de](https://ffmpeg.martin-riedl.de/) release builds, with the GNU GPLv3 text |

The Linux builds need glibc 2.28 or later and link nothing else from the host.
Only `linux-x86_64` has been fetched, run and tested (on Ubuntu 26.04); the other
pins are verified against their published digests only. Shipping GPL FFmpeg
executables beside proprietary Aster also obliges the publisher to offer the
corresponding FFmpeg source; that, codec patent licensing, and macOS signing
and notarization of the bundled executables, remain release work.

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

### Export encoders

Export and proxies encode with the same FFmpeg. Which formats can be exported
depends on how that FFmpeg was built and, for hardware encoders, on the
graphics driver:

| To export | FFmpeg needs |
| --- | --- |
| MP4 or MOV with H.264 / H.265 | `libx264` / `libx265`, or a hardware encoder |
| AV1 | `libsvtav1` or `libaom-av1`, or a hardware encoder |
| WebM with VP9 and Opus | `libvpx-vp9` (or a hardware VP9 encoder) and `libopus` |
| MOV with ProRes | `prores_ks` (or `prores_videotoolbox` on macOS) |
| AAC sound (MP4, MOV, M4A) | `aac` (FFmpeg's own) or `aac_at` (macOS) |
| MP3 / FLAC / WAV sound | `libmp3lame` / `flac` / `pcm_s16le` and `pcm_s24le` |
| Proxies | `libx264`; without it proxies are MJPEG |

Hardware encoders are NVIDIA NVENC, AMD AMF, Intel Quick Sync, Apple
VideoToolbox, VA-API and Media Foundation, each only when FFmpeg was built
with it and the driver and device are present. VA-API is tried on each
`/dev/dri/renderD*` node.

Nothing is taken on trust: when the editor starts, it asks FFmpeg which
encoders and muxers it lists and has each known encoder make a short test
picture or a moment of silence. The export dialog offers only what passed and
shows the reason for what failed. Without a working FFmpeg nothing can be
exported, and each proxy asked for is marked failed in the library. After
installing another FFmpeg, press **Try again** in the dialog. Distribution builds of FFmpeg differ; Ubuntu
26.04's `ffmpeg` package, for example, links `libx264`, `libx265`, `libvpx`,
`libsvtav1`, `libaom`, `libopus` and `libmp3lame`. On the Linux host used for this work (FFmpeg
8.0.1, NVIDIA driver) the software encoders and NVENC passed, while Quick
Sync and VA-API failed to initialise.

H.264 and H.265 are patent-encumbered formats, and encoders such as `libx264`
and `libx265` are GPL-licensed. Aster bundles no encoder: these come from the
FFmpeg you install, under its terms. Codec licensing for a distributed
release is part of the open
[codec and distribution ticket](../.scratch/core-editor/issues/03-codec-and-distribution.md).

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

## Speech recognition

Captions from speech use [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
1.8.3, compiled into the binary through `whisper-rs` 0.16. Nothing else is
installed to run it: no Python, PyTorch, ONNX Runtime or GPU runtime.

- **Build**: CMake, a C and C++ compiler, and libclang for the generated
  bindings. `.cargo/config.toml` builds whisper.cpp for a CPU family rather
  than for the build machine: AVX2, FMA, F16C and BMI2 on x86-64 (processors
  from 2013 onwards), NEON on 64-bit ARM. On an older x86-64 processor the
  recognizer cannot run, and its static setup code may stop the binary from
  starting; no such processor was tried. GPU back-ends are not enabled.
- **Models**: none is bundled. The speech panel downloads the chosen model
  once, on request, from the whisper.cpp model repository on Hugging Face, and
  keeps it only when its size and SHA-256 sum match the built-in list. Sizes
  run from 33 MB (Tiny) to 575 MB (Large v3 Turbo), with the 0.9 MB speech
  detector. A computer without a network can use **Model file…** with any
  whisper.cpp `.bin` model.
- **Download tool**: the system's `curl`, which ships with current Linux
  distributions, macOS and Windows 10 1803 and later; `ASTER_CURL` names
  another executable. Without it the panel says so and a model file can still
  be chosen by hand. Aster links no TLS library of its own.
- **Translation** into languages other than English uses the native AI
  provider (see [native AI editing](native-ai.md)); no translation model is
  installed.

Only the Linux build was compiled and run with speech recognition; the macOS
and Windows builds, and `curl` on them, have not been tried.

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
