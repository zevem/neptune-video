# Windows backend cross-target proof

Status: ready-for-agent
Progress: resolved
Type: task

Date: 2026-10-12. Scope: prepare and type-check `editor-agent`, `editor-core`
and `editor-store` for `x86_64-pc-windows-gnu` on the Linux host. Native Windows
execution, GUI/media, MSVC linking and installer acceptance are separate gates.

## Prerequisites prepared

The T3 launcher's `unknown proxy name: t3_code_alpha` error is resolved with
the official rustup [`RUSTUP_FORCE_ARG0` override](https://rust-lang.github.io/rustup/dev-guide/tips-and-tricks.html#rustup_force_arg0).
The existing rustup executable and user tool links were preserved; no task-local
replacement or toolchain upgrade was needed.

```sh
RUSTUP_FORCE_ARG0=rustup /home/biggabo/.cargo/bin/rustup --version
RUSTUP_FORCE_ARG0=rustup /home/biggabo/.cargo/bin/rustup target add --toolchain 1.97.1-x86_64-unknown-linux-gnu x86_64-pc-windows-gnu
RUSTUP_FORCE_ARG0=rustup /home/biggabo/.cargo/bin/rustup target list --toolchain 1.97.1-x86_64-unknown-linux-gnu --installed
```

The target installation succeeded. The installed target list and rustlib
component manifest confirm Windows GNU std alongside the Linux host std.
Only that target component was added to pinned Rust 1.97.1. Download URL and
expected SHA-256 came from the pinned toolchain's own release manifest; rustup
performed its component installation/checksum validation.

Raw evidence: [prerequisites](../../artifacts/production-readiness/windows/prerequisites.json),
[installation](../../artifacts/production-readiness/windows/target-install.log),
[targets](../../artifacts/production-readiness/windows/installed-targets.log),
[rustup](../../artifacts/production-readiness/windows/rustup-version.log) and
[compiler](../../artifacts/production-readiness/windows/rustc-version.log).

No MinGW linker or Wine executable was found. This does not prevent a Rust
`cargo check --lib` cross-target proof; it does prevent claiming linked/native
Windows execution from these prerequisites.

## Cross-target result

After root granted the shared compilation slot, the focused check passed with
exit code 0, no warnings/errors and 2.12 seconds reported by Cargo:

```sh
RUSTC=/home/biggabo/.rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc RUSTDOC=/home/biggabo/.rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustdoc /home/biggabo/.rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/cargo check --profile ci --locked --target x86_64-pc-windows-gnu -p editor-core -p editor-agent -p editor-store --lib
```

The [diagnostic log](../../artifacts/production-readiness/windows/check.log) and
[command/config/source record](../../artifacts/production-readiness/windows/check-record.json)
retain the proof. Targeted source/manifests/lock/toolchain/config digests were
unchanged across the checks; UI and binary source were excluded from that
snapshot. No source fix was needed, no other package/GUI Cargo command ran,
and the compilation slot was returned immediately after the two target checks.

This verifies the Windows-conditional backend source, including stable job/
suspended-thread launch APIs and enabled windows-sys features, can type-check
for Windows GNU. It does not link an executable or run on Windows; job/process
behavior, file replacement, provider/media/desktop behavior, MSVC and installer
acceptance still require their own actual target proof.
