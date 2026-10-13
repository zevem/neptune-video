# Aster

## Product contract

- Native Rust desktop editing on Linux, macOS and Windows. The project and
  timeline are the product; chrome stays quiet and makes editing discoverable.
- Interactive frames remain independent of media probing, decoding, directory
  traversal, persistence and agent requests. Bound worker queues, decoded media
  and GPU caches; paint the visible timeline and sleep while idle.
- Edits are validated, reversible commands with stable targets. Human actions
  and agent actions enter through the same command boundary. Playback position,
  selection and worker state stay separate from durable project state.
- Protect the person's work. Failed edits leave the project unchanged; future
  or damaged project files require a protected recovery path before replacement.
- Describe measured behavior precisely. Native dependencies and CI builds do
  not establish platform acceptance, real-time playback or production readiness.

## Change paths

Read guidance when its branch applies:

| Change | Guidance |
| --- | --- |
| Domain, command ownership or dependencies | [GLOSSARY.md](GLOSSARY.md), [architecture](docs/architecture.md) and relevant [ADRs](docs/adr/) |
| Timeline, project, media or history behavior | [core editor spec](.scratch/core-editor/spec.md) |
| Rust implementation or validation | [development](docs/development.md) |
| Workers, rendering, decoding or resource policy | [performance](docs/performance.md) |
| Frontend, icons, input or motion | [interface design](docs/design.md) |
| Native verification, support or release claims | [verification](docs/verification.md) |

Keep `desktop → editor-core`, `desktop → editor-media`, `desktop → editor-store`
and `desktop → editor-agent`; the domain core is independent of GUI, codecs,
filesystem and platform services. Workers publish owned results with project,
media revision and request identity; stale completions cannot update a
replacement project or newer seek.

Trace behavior through shortcuts, menus, timeline gestures, inspectors and
serialized agent commands. Cover validation, reversal, save/load and failure at
the owning seam. Capture stable asset, track and clip IDs when creating actions;
later focus changes must not retarget them.

## Working here

Use the repository's pinned toolchain and lockfile. Coordinate one compilation
owner during multi-agent work and respect `.cargo/config.toml`. Run the focused
checks that prove the changed behavior; documentation changes need reference
review. Whole-workspace validation belongs to CI or the designated validation
owner. Report the actual host, build and unverified behavior.

Native test runs use fresh task-owned projects and storage. Track the processes
started by the task and clean up only those. Preserve unrelated files and user
projects. UI acceptance requires captures of the running final native build,
including narrow windows and changed states; name any unavailable capture.

Reference Neptune for ownership and native performance discipline; give this
editor its own visual identity. Neptune is a read-only reference project.

## Agent skills

### Issue tracker

Issues and specs live under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the five default triage roles as status strings. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `GLOSSARY.md` and `docs/adr/`. See `docs/agents/domain.md`.
