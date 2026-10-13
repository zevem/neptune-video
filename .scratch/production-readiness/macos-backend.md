# macOS ARM64 backend cross-target proof

Status: ready-for-agent
Progress: resolved
Type: task

Date: 2026-10-12. Scope: prepare and type-check `editor-core`, `editor-agent`
and `editor-store` for `aarch64-apple-darwin` on Linux. This is headless
cross-target compilation; actual macOS execution, GUI/media, SDK linking,
packaging, Metal/CoreAudio and Apple Silicon acceptance remain separate gates.

## Prerequisites prepared

The [Windows setup](windows-backend.md) established the official rustup argv0
override without replacing user tools. The same pinned manager invocation
successfully added only macOS ARM64 std to Rust 1.97.1:

```sh
RUSTUP_FORCE_ARG0=rustup /home/biggabo/.cargo/bin/rustup target add --toolchain 1.97.1-x86_64-unknown-linux-gnu aarch64-apple-darwin
RUSTUP_FORCE_ARG0=rustup /home/biggabo/.cargo/bin/rustup target list --toolchain 1.97.1-x86_64-unknown-linux-gnu --installed
```

The installed target list, component manifest and target rustlib files confirm
the installation. The download URL and expected SHA-256 are from the pinned
toolchain's own release manifest; no toolchain version/default or user tool
links changed. No macOS SDK or linker was installed for this type-check scope.

Raw records: [prerequisites](../../artifacts/production-readiness/macos/prerequisites.json),
[install](../../artifacts/production-readiness/macos/target-install.log),
[targets](../../artifacts/production-readiness/macos/installed-targets.log) and
[compiler](../../artifacts/production-readiness/macos/rustc-version.log).

## Cross-target result

After root granted the shared compilation slot, this focused check passed with
exit code 0, no warnings/errors and 2.24 seconds reported by Cargo:

```sh
RUSTC=/home/biggabo/.rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc RUSTDOC=/home/biggabo/.rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustdoc /home/biggabo/.rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/cargo check --profile ci --locked --target aarch64-apple-darwin -p editor-core -p editor-agent -p editor-store --lib
```

The [diagnostic log](../../artifacts/production-readiness/macos/check.log) and
[command/config/source record](../../artifacts/production-readiness/macos/check-record.json)
retain the proof. Targeted source/manifests/lock/toolchain/config digests remained
unchanged; UI and binaries were outside that snapshot. No source fix or SDK/
linker install was needed. No other package/GUI Cargo command ran, and the slot
was returned immediately after Windows GNU and macOS ARM64 checks.

This proves the headless Rust backend libraries type-check for Apple Silicon,
including the Unix provider process boundary. It does not establish Mach-O
linking, native process behavior, Intel Macs, Metal/CoreAudio, filesystem or UI
acceptance, or signed/notarized distribution.
