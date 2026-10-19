# Development

Use `rustup` with the repository's `rust-toolchain.toml` (Rust 1.97.1, edition
2024). The desktop uses Neptune Terminal's native `eframe`/`egui` 0.36.2 stack with `wgpu`,
X11/Wayland and accessibility integration. The application runs on a graphical
desktop with a working graphics driver.

## Build and run

Follow [source installation](installation.md) for Linux, macOS and Windows
prerequisites and FFmpeg/ffprobe configuration, then run:

```sh
cargo run --release --locked --bin neptune-video
```

Rust controls the editor and worker adapters; FFmpeg supplies native codecs.
Platform prerequisites and configured CI remain distinct from installer or
native runtime acceptance.

## Focused verification

Choose the owning package and relevant test filter:

```sh
cargo test -p editor-core --locked
cargo test -p editor-store --locked
cargo test -p editor-media --locked
python3 scripts/test-workspace.py --package editor-agent
cargo clippy -p editor-core --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
```

The pure core tests need no window, media tools or audio device. Persistence
tests use temporary directories, never personal projects. Media integration
proof uses generated/owned fixtures and actual FFmpeg executables when the test
requires them; skipped tests must be reported. Keep Rust compilation sequential
within an agent and coordinate one validation owner across shared work.

The designated validation owner or CI runs the workspace checks:

```sh
cargo fmt --all -- --check
python3 scripts/test-workspace.py --profile ci
cargo clippy --profile ci --workspace --all-targets --locked -- -D warnings
cargo build --profile ci --locked --bins
```

Use `--locked`; investigate a toolchain/dependency error before changing the
pin. Full CI profile validation establishes compilation and headless behavior.
Native design and media acceptance are separate in
[verification](verification.md).

[test-workspace.py](../scripts/test-workspace.py) builds the owned native
provider-fault example first, takes its absolute executable path from Cargo's
artifact event, and supplies `NEPTUNE_VIDEO_PROVIDER_TEST_FIXTURE` for the subsequent
locked tests. Missing fixture configuration fails explicitly. `--package` and
`--filter` select focused checks; `--target` and Cargo's environment retain
normal target configuration. No fixture ships in the application binaries.
The private process tests exercise blocked-input cancellation, deadlines,
fragmented/invalid/bounded output and full-queue teardown on each native target;
Windows/Linux additionally verify a retained descendant terminates after its
immediate parent exits. Linux uses pidfds for that test, while Windows keeps an
exact process handle. Native target execution is separate from a library/test
typecheck.

## Isolation and evidence

Launch native verification with owned fixtures/projects and explicit
`--settings`, `--agent-session` and capture paths. The settings parent contains
that launch's unsaved recovery directory, so isolating the settings file also
isolates unsaved recovery. Use a private X11/Xvfb display for OS injection where
possible; real desktop portals, file drops and sound devices need their own
native acceptance. Track child processes and clean up their exact handles.

```sh
target/release/neptune-video --settings /absolute/run/settings.json \
  --agent-session /absolute/run/session.json /absolute/run/film.nvp
```

Retain screenshots, test logs and measurements under `artifacts/` or another
task-owned destination. Reports identify host, build profile/features, graphics
backend, media fixture and limitations. Use release builds for performance;
keep headless correctness checks on the reusable CI profile.

## Continuous integration

CI runs on every pull request and on pushes to `main`. A preflight job
classifies the change: documentation, `.scratch/` and `artifacts/` changes skip
compilation but still report every job. The `main` ruleset requires a pull
request and the aggregate `Check` status, which passes only when every job
passed.

[CI](../.github/workflows/ci.yml) declares explicit Linux x86-64, macOS ARM64,
macOS Intel and Windows x86-64 MSVC Rust build/test/lint jobs plus formatting
and dependency-boundary validation. The versioned runner labels follow
[GitHub's runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners);
they identify CI environments, not minimum supported OS versions. The Linux
job installs window/audio build dependencies and FFmpeg. Other jobs install
FFmpeg for media tests.

Before headless correctness, every native CI job runs
[check-media-tools.py](../scripts/check-media-tools.py). It requires both
`ffmpeg` and `ffprobe` to complete `-version` successfully within ten seconds
each, and records their resolved executable paths and actual version banners.
Explicit `NEPTUNE_VIDEO_FFMPEG` / `NEPTUNE_VIDEO_FFPROBE` overrides select those executables;
otherwise it takes the build `scripts/fetch-media-tools.py` placed in
`.tools/media/<os>-<arch>`, as a build in the checkout does, and then `PATH`. Missing, broken or empty overrides fail the
preflight instead of falling back to another tool. Run the same preflight
before claiming local FFmpeg integration coverage. Some local media fixtures
return early when tools are unavailable, so a successful test count with zero
ignored tests alone does not establish that those fixtures executed.

Configured jobs are pending evidence until their actual runs succeed; they do
not replace real device/audio, input or installer tests.

## Native test preparation

[run-native-tests.py](../scripts/run-native-tests.py) is the shared backend
entry point for the four CI targets. A plan launches no tools and labels every
check `not_run`; give it a fresh artifact directory inside the repository:

```sh
python3 scripts/run-native-tests.py --plan --target aarch64-apple-darwin \
  --output artifacts/native/backend-checks/macos-plan
```

On the corresponding native host, use `--run` with a fresh output directory.
The runner checks the OS, process architecture and pinned Rust host, then runs
the architecture check, mandatory media-tool preflight, locked workspace tests,
denied-warning Clippy and native application linking in sequence. It records
logs, the source digest before and after, test counts, and application paths and
hashes from Cargo's artifact events. A failed check stops later checks and
retains its evidence. These use the CI profile, not a performance build.

The workflow uploads the native backend reports for 14 days. [test-native-test-runner.py](../scripts/test-native-test-runner.py)
checks reporting and refusal with fake tools and temporary files; it never
launches Rust, media, devices or the editor. Its checks do not establish native
acceptance.

The separate [macOS and Windows UI pack](native-platform-tests.md) prepares
real input, device and operator acceptance. As requested on 2026-10-12, those
platform runs are deferred; the tests are prepared here on Linux. No hosted CI
run or native macOS/Windows acceptance is recorded yet.
