# Neptune packaging reference for Aster

Read-only source audit, 2026-10-12. Neptune provides useful release primitives,
not proof that Aster's packages or platforms work. Aster currently documents
source installation and pending native macOS/Windows acceptance; its distribution
ticket remains open. [Aster installation:3–6][A-install]
[Distribution ticket:3–19][A-ticket]

Scope: inspected Neptune's agent guidance, release/development/verification
documents, hidden GitHub workflows, packaging scripts/resources and relevant
native entry points; inspected Aster's manifests, CI, launcher, media process
adapter and verification record. No compilation, network, installation,
credential inspection, release or existing-file mutation was performed. The
findings below describe checked-in mechanics; configured runners are not
successful runs or native acceptance. Both repositories require that distinction.
[Neptune AGENTS:17–20][N-agents] [Aster AGENTS:15–16][A-agents]

## Reusable mechanics and required adaptation

| Mechanism owned by Neptune | Aster application |
| --- | --- |
| Deliberate `v*` tags; exact source commit on `main` with successful push CI; refuse published-release reruns. [Release workflow:3–57][N-workflow] | Implement an Aster release workflow separately from PR CI. Keep tag/publication actions outside this audit. Neptune's release authorization and RC policy are Neptune decisions, not automatically new Aster policy. |
| Strict SemVer including Windows numeric bounds, useful dated notes, package/lock consistency; preparation validates notes before writing and creates no tag. [release.py:20–95][N-version] | Adapt package identity and version resolution. Aster's root and all four crates inherit `workspace.package.version`; Neptune's direct root `package.version` reads and desktop-only lock replacement cannot be copied unchanged. [Aster Cargo:5–30][A-cargo] [Core:1–5][A-core] [Media:1–5][A-media] [Store:1–5][A-store] [Agent:1–5][A-agent] |
| Native optimized jobs explicitly name Linux x64, macOS ARM64/Intel and Windows MSVC x64. Intel macOS gets a separate library test job. [Release workflow:59–101,157–169][N-matrix] | Declare Aster's supported OS/architecture and minimum-host policy, then run those actual targets. Aster currently uses moving `*-latest` runner names without an explicit architecture matrix. [Aster CI:33–70][A-ci] |
| Linux builds stage a desktop entry, icon sizes, notices and DEB; map prereleases to Debian `~`; AppImage adds executable `AppRun`, icon and host-library policy. [package-linux.py:87–136][N-linux] | Reuse staging, argument forwarding, prerelease ordering and desktop validation. Replace product identity/category and derive dependencies from Aster's own executable plus chosen codecs/audio stack. Do not inherit Neptune's DEB dependency list or glibc minimum as an Aster guarantee. |
| AppImage follows linked/transitive and runtime-loaded window libraries, fails on missing libraries/notices, handles merged `/usr` aliases, and leaves glibc/graphics/GBM/Wayland libraries host-owned. [package-linux.py:22–84][N-linux-libs] | Review the closure for Aster's CPAL/audio and codec deployment. The traversal is reusable; its CEF/NSS queue and exclusions need deliberate adaptation and clean-host proof. Aster's media crate uses CPAL. [Media Cargo:8–14][A-media-deps] |
| AppImage tools/runtime are version/hash pinned; CI validates desktop metadata, inspects DEB content and invokes AppImage `--version`. [Release workflow:128–149][N-linux-ci] | Pin Aster packaging inputs and inspect output. `--version` is packaging smoke only: Neptune explicitly requires a downloaded executable-permission check and a real native window. [Release guide:164–174][N-linux-accept] |
| macOS creates a fresh `.app`, executable permissions, icon, numeric bundle version and high-resolution metadata. [package-macos.py:15–63][N-macos] | Use a unique Aster bundle ID/name/category/icon and an explicit codec resource location; include `aster-agent` if it is shipped. Adapt inherited Cargo version handling. Browser helper bundles are unnecessary. |
| Developer ID hardened-runtime signing; app ZIP notarization/stapling/Gatekeeper check; Applications link; signed/notarized/stapled DMG; temporary keychain restoration in `finally`; no unsigned fallback. [sign-macos.py:28–107][N-sign] | Reuse the sequence after Aster credentials/policy exist. Sign the chosen Aster binaries and any deployed native helpers individually; omit CEF/JIT entitlements and browser framework traversal. This audit did not inspect credentials or establish signing availability. |
| Windows `winresource` embeds the app icon/product description; Inno Setup installs per-user with shortcuts, uninstall identity and explicit x64/minimum-Windows settings. [build.rs:1–11][N-build] [windows.iss:9–48][N-windows] | Add Aster resource identity and installer definitions, choose supported Windows minimum and signing policy, and package both intended binaries/codecs/notices. Neptune's installer is explicitly unsigned; that limitation is not an Aster signing decision. [Release workflow:115–127][N-windows-ci] |
| License payload collects locked Cargo metadata and dependency license/notice files, with bundled fonts and theme provenance. [package-common.py:57–83][N-notices] | Collect notices for Aster's shipped dependencies/fonts and chosen FFmpeg distribution. Omit Neptune's config example and iTerm2 theme notices; retain required provenance for the actual payload. Choose Aster's project license instead of copying Neptune's MIT declaration. [Neptune Cargo:7–14][N-cargo-license] [Aster Cargo:16–44][A-root-deps] |
| Final aggregation requires an exact artifact set, sizes/hashes/source SHA; Ed25519 signs exact manifest bytes and must match the public trust anchor; SHA256SUMS covers final files; provenance attests them. [release.py:103–135][N-seal] [Release workflow:171–206][N-draft-job] | Adapt names, repository, platforms, payload limits and trust anchor. Manifest signing is relevant if Aster implements authenticated update/discovery; installer signing remains its own platform operation. |
| Draft staging fails closed on ambiguous/read failures, refuses public asset mutation, marks uploads `INCOMPLETE`, verifies uploaded names and never publishes. Tests cover malformed versions, incomplete sets, exact bytes, failed uploads and published reruns. [release.py:138–177][N-stage] [test_release.py:148–264][N-release-tests] | Reuse these failure contracts with Aster fixtures before attaching real release credentials. Avoid copying Neptune's one-time public-history reset or release-note correction exceptions. [Release guide:40–63][N-reset] |

## Native integration worth preserving

Neptune matches its Linux desktop `StartupWMClass` to the runtime app ID and
provides Windows resource, runtime and macOS bundle icons. The runtime selects
padded macOS artwork because eframe replaces the Dock icon at launch. Aster
currently sets app ID `aster` and has no runtime icon in its native options.
Packaging should coordinate these identities and artwork with the UI.
[Neptune desktop:1–10][N-desktop]
[Neptune main:156–200][N-main] [Aster main:104–125][A-main]

Neptune removes AppImage launcher variables and bundled `LD_LIBRARY_PATH`
entries before any thread starts, so host child processes do not load private
installer libraries; it retains the original AppImage path for update purposes.
Aster constructs FFmpeg/ffprobe commands from overrides or `PATH` without a child
environment policy. If Aster adopts an AppImage with private libraries, define
separate environments for host tools and bundled codec helpers and test both.
This is a source-derived packaging risk, not an observed Aster failure.
[Neptune host_env:1–65][N-env] [Neptune main:4–9][N-bootstrap]
[Aster media process:211–218][A-tools]

The shared Rust GUI stack, Windows GUI subsystem and quiet helper creation are
already present in Aster. They establish implementation paths, not native
Windows acceptance. [Neptune Cargo:31][N-gui] [Aster Cargo:37][A-gui]
[Aster main:1–2][A-subsystem] [Aster media process:63–79][A-quiet]

Do not import terminal/PTY/ConPTY acceptance criteria, shell restoration, terminal
font/theme policies, CEF/browser processes, cookie stores, browser sandbox/NSS
modules or browser JIT entitlements. Neptune owns those domain/runtime choices;
Aster owns project/history and media workers. Replace the acceptance workload
with editing, import, codec/audio, recovery and agent commands while retaining
Neptune's fresh storage, exact process ownership, sequential OS input and
release-build evidence discipline. [Neptune development:3][N-dev-stack]
[package-common.py:11–54][N-cef] [Aster architecture:11–24][A-ownership]
[Neptune scripts guidance:18–41,128–137][N-script-rules]

## Aster's present evidence gap

- CI configures headless tests and `ci`-profile binary builds on Linux/macOS/
  Windows, installs host FFmpeg, and runs Clippy on Linux. It has no installer,
  signing, release-artifact upload or native GUI acceptance steps. The current
  verification record reports no hosted CI run. [Aster CI:33–70][A-ci]
  [Verification:35–40][A-ci-proof]
- `scripts/run-local.sh` launches an existing Linux release binary, optionally
  selects task-local FFmpeg/ffprobe, forwards arguments and never downloads or
  rebuilds. It is a checkout convenience, not a distributable runtime.
  [Launcher:1–21][A-launcher] [Installation:19–25][A-launcher-guide]
- Codecs currently depend on installed tools/overrides and the installed FFmpeg
  build; clean-machine availability, supported-format matrix and deployment
  notices are unresolved. No automatic installation is promised.
  [Installation:46–56][A-codecs] [Distribution ticket:9–19][A-ticket-task]
- Linux has final optimized X11 smoke/recovery, Weston URI-drop and GNOME
  launch/render/teardown evidence. The GNOME run did not exercise input/drop/
  chooser. macOS/Windows have source/configuration evidence only, with native
  graphics/audio/input/storage/package acceptance pending.
  [Verification:64–99][A-linux-proof] [Platform matrix:184–208][A-platforms]

A read-only `rg --files --hidden` inventory, excluding `.git`, `target`,
`artifacts`, `.tools`, `node_modules` and `__pycache__`, found only
`.github/workflows/ci.yml` for workflow/package-resource patterns and
`assets/fonts/Geist-LICENSE.txt` for license/notice patterns. No root `build.rs`,
packaging directory, release/package script, changelog, root license, desktop
entry, ICNS/ICO, Inno definition or plist was found in this checkout. This is an
inventory observation, not a claim about externally prepared files; Aster's own
verification explicitly disclaims an installer or release pipeline.
[Verification:203–208][A-distribution]

## Concrete implementation order

This is a proposed sequence, not a new release authorization or adopted policy.
It addresses the existing distribution ticket and acceptance gates.
[Distribution ticket:9–19][A-ticket-task]
[Verification:193–208][A-gates]

1. Specify supported OS/architectures/minimum versions, project license, binary
   payload (`aster`/`aster-agent`), FFmpeg source/build/format/licensing policy,
   install locations and update channels. Choose the release-candidate and
   publication policy explicitly; Neptune's accepted-RC/exact-installer rule is
   a useful model. [Neptune release policy:12–38][N-policy]
2. Adapt version/notes/artifact validation and notice generation for Aster's
   workspace inheritance. Add focused tests for malformed versions, incomplete
   sets, payload hashes, licensing inputs and failure-preserving draft staging.
   Keep offline packaging validation separate from publication credentials.
   [release.py:53–95,103–177][N-primitives]
3. Obtain actual pinned native build/test logs for each selected architecture,
   then create fresh Linux AppImage/DEB, macOS app/DMG and Windows installer
   stages. Align native icons/IDs and project-opening integration; prove media
   discovery and child environments when launched outside a developer shell.
   Use Aster-owned fixtures/storage. [Aster installation:27–56][A-platform-install]
   [Aster development:54–71][A-isolation]
4. Add explicit release build/package jobs, signing/notarization policy and
   complete-artifact checksum/provenance validation. Configure least-privilege
   release environments only when needed, without copying Neptune credentials
   or trust material. [Neptune release environment:222–231][N-security]
5. Test downloaded candidate installers on clean target machines: real window,
   media/audio, import/drop/chooser, clipboard/IME/DPI/accessibility, recovery and
   failed storage, install/upgrade/uninstall and source-media preservation.
   Retain exact source SHA, artifact hashes, hardware/backend and captures; keep
   unsupported targets explicitly pending. [Aster gates:193–208][A-gates]
   [Neptune verification gates:563–580][N-gates]
6. Add authenticated update discovery/handoff only with a defined product
   policy and trusted manifest protocol, then test exact downloaded bytes and
   failure recovery per platform. Neptune distinguishes replaceable macOS app/
   Linux AppImage from package-managed/source/Windows manual handoff.
   [Neptune platform updater:16–56][N-update-targets]

[N-agents]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/AGENTS.md:17
[A-agents]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/AGENTS.md:15
[A-install]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/installation.md:3
[A-ticket]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/.scratch/core-editor/issues/03-codec-and-distribution.md:3
[N-workflow]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/.github/workflows/release.yml:3
[N-version]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/release.py:20
[A-cargo]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/Cargo.toml:5
[A-core]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/crates/editor-core/Cargo.toml:1
[A-media]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/crates/editor-media/Cargo.toml:1
[A-store]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/crates/editor-store/Cargo.toml:1
[A-agent]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/crates/editor-agent/Cargo.toml:1
[N-matrix]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/.github/workflows/release.yml:59
[A-ci]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/.github/workflows/ci.yml:33
[N-linux]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/package-linux.py:87
[N-linux-libs]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/package-linux.py:22
[A-media-deps]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/crates/editor-media/Cargo.toml:8
[N-linux-ci]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/.github/workflows/release.yml:128
[N-linux-accept]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/docs/releases.md:164
[N-macos]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/package-macos.py:15
[N-sign]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/sign-macos.py:28
[N-build]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/build.rs:1
[N-windows]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/packaging/windows.iss:9
[N-windows-ci]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/.github/workflows/release.yml:115
[N-notices]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/package-common.py:57
[N-cargo-license]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/Cargo.toml:7
[A-root-deps]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/Cargo.toml:16
[N-seal]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/release.py:103
[N-draft-job]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/.github/workflows/release.yml:171
[N-stage]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/release.py:138
[N-release-tests]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/test_release.py:148
[N-reset]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/docs/releases.md:40
[N-desktop]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/packaging/neptune.desktop:1
[N-main]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/main.rs:156
[A-main]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/src/main.rs:104
[N-env]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/platform/host_env.rs:1
[N-bootstrap]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/main.rs:4
[A-tools]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/crates/editor-media/src/process.rs:211
[N-gui]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/Cargo.toml:31
[A-gui]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/Cargo.toml:37
[A-subsystem]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/src/main.rs:1
[A-quiet]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/crates/editor-media/src/process.rs:63
[N-dev-stack]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/docs/development.md:3
[N-cef]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/package-common.py:11
[A-ownership]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/architecture.md:11
[N-script-rules]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/AGENTS.md:18
[A-ci-proof]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/verification.md:35
[A-launcher]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/scripts/run-local.sh:1
[A-launcher-guide]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/installation.md:19
[A-codecs]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/installation.md:46
[A-ticket-task]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/.scratch/core-editor/issues/03-codec-and-distribution.md:9
[A-linux-proof]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/verification.md:64
[A-platforms]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/verification.md:184
[A-distribution]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/verification.md:203
[A-gates]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/verification.md:193
[N-policy]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/docs/releases.md:12
[N-primitives]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/scripts/release.py:53
[A-platform-install]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/installation.md:27
[A-isolation]: /home/biggabo/Documents/Projects/ai-vid-editor/ai-vid-editor/docs/development.md:54
[N-security]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/docs/releases.md:222
[N-gates]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/docs/verification.md:563
[N-update-targets]: /home/biggabo/Documents/Projects/rust-terminal/rust-terminal/src/platform/updates.rs:16
