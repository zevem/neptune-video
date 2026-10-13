---
status: accepted
---

# Native Rust editor with an independent domain core

The user requires a high-performance Rust desktop editor on Linux, macOS and
Windows and asks to reuse Neptune's stack where it applies. Keep project and
editing rules in a GUI-independent Rust core, with native `egui`/`wgpu` desktop
composition and separate media/persistence ownership; this preserves one command
contract for people and agents while isolating expensive I/O and replaceable
codec implementations from interactive frames. A browser-hosted editor would
introduce a second frontend runtime, and embedding media or filesystem state in
the core would make deterministic edits, recovery and headless verification
depend on the desktop lifecycle.
