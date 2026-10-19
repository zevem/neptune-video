# Native AI and document sessions: desktop side

Scope `src/**` except `src/bin/**`.
No backend crate, manifest, script or doc is edited from here.

## Status

- 2026-10-12, written against the contract before the grant.
- 2026-10-12, after the frontend Cargo grant (Linux, pinned 1.97.1):
  `cargo clippy --profile ci --bin neptune-video --all-targets --locked -- -D warnings`
  clean and `cargo test --profile ci --bin neptune-video --locked` 81 passed at the
  backend state of about 03:50. After that the desktop only changed
  `src/ui/sheets.rs` (settings sheet height); tests were rerun, 81 passed.
  A later clippy run stopped inside backend files that were being edited
  (`editor-agent/src/context.rs` dead code, `editor-store`), so the last
  clean clippy does not cover that one sheet change against the newest backend.
- Release build verified natively on task-owned Xvfb `:96`/`:97`; record and
  captures in `artifacts/native/ai-round6/` (`report.json`). The provider in
  those runs is `fake_provider.py`, a deterministic stand-in set through
  `ai.claude_executable`. **No real provider request was sent from the desktop.**
- `scripts/native-smoke.py` rerun on the new build: 13 of 13.
- **Cargo window returned to root.** Both Xvfb servers and every Neptune Video process
  started here were stopped.

- 2026-10-12, round 2, **source only, not compiled**: bound recovery from
  `recovery-contract.md` is wired in `src/app.rs` (restore, the periodic copy,
  the copy written at exit) with two new tests. `backend-contract.md` carried
  no Cargo grant when it was checked (04:16), so no `cargo` command was run in
  this round: the change is unbuilt and its tests have not run. Only
  `rustfmt --edition 2024 --check src/app.rs` was run, and it is clean. No
  native process, display, portal session or provider request was started.

- 2026-10-12, round 3, **final outcome of this integration on Linux**. The
  round-2 source compiled as written; no file under `src/` changed in this
  round. Pinned 1.97.1, Linux x86_64, record in
  `artifacts/native/ai-round7/report.json`:
  - `cargo clippy --profile ci --bin neptune-video --all-targets --locked -- -D warnings`
    clean; `cargo test --profile ci --bin neptune-video --locked` 83 passed.
    `cargo build --release --bins --locked` built `neptune-video`
    `bca32395…6f043b`. `scripts/native-smoke.py` on that binary: 13 of 13,
    fixture and media hashes unchanged.
  - **Real file dialogs** (rfd → xdg-desktop-portal → GTK FileChooser v4, on
    the private session from `run-private.py`, typed and clicked with XTest):
    Save As cancelled, to a new file (a space and `%` in its name), and onto
    an existing file after GTK's own Replace; a save refused after an
    independent process rewrote the file; that stale file chosen again in the
    dialog and refused with its bytes unchanged; saved elsewhere; Duplicate
    refused onto an existing file and onto the open file, bytes unchanged;
    Duplicate to a new file with its own project id and the same media paths,
    the window moving to the copy; Open cancelled and chosen. 14 steps, all
    passed (`portal-report.json`). Not driven: the import dialogs, non-ASCII
    names, and a destination that changes between choosing and writing.
  - **Real providers from the Ask field**, default command lines and their own
    sign-in, on an owned synthetic project. Claude (`claude-haiku-5-5` on the
    card): 2 requests, 2 proposals; a value held in the Name field was
    committed at send; a marker added by hand made the open proposal stale
    and Apply did nothing; the second proposal applied as one revision, one
    undo restored the project exactly and redo the applied one. Codex
    (`gpt-6.1-sol`): 5 requests, 2 proposals; the first was closed by
    reopening the file, the last applied with the same exact undo and redo.
    Pending, proposal, stale, applied at 1440×900 and 760×560 in `ai-shots/`.
  - **Recovery** (`recovery-report.json`): SIGKILL with two unsaved edits,
    another process rewrites the file, restore: "Changed on disk" and the
    notice are up before anything is written, both history steps undo and
    redo, the first save is refused, the copy written after the restore still
    records the file as it was opened, Save As to a new file leaves the other
    writer's bytes alone. A copy with its binding removed, and a second
    window's private copy, both come back with no file; Save asks where and
    the original file is never written. A first window closing removes only
    its own copy. A window that closes with unsaved work before any periodic
    copy leaves a bound copy written at exit, which restores and saves to its
    own file.
  - **Cargo is returned to root.** No Cargo, Neptune Video, Xvfb, DBus or portal
    process started here is running; no watcher and no child task is left.
  - Not established here: macOS, Windows, Wayland, sound, playback rate,
    codecs. This is not a claim that the product is finished.

## What the desktop uses (root: tell me here if any of it moves)

`editor_agent`: `AiRuntime::{new(wake), request(config, context, prompt) -> Result<Uuid, AiError>, poll() -> Option<AiEvent>, cancel()}`,
`AiEvent::{Started{id}, Proposal{id, proposal: Box<AiProposal>}, Failed{id, message}, Cancelled{id}}`,
`AiConfig { provider, executable, model, effort, timeout_seconds }` with `Default`,
`AiContext::capture(&Editor, &[ClipId], Time)`, `AiProvider::{Claude, Codex}`,
`AiProposal { summary, commands, provider, model, before, after }` as public
fields, `is_current(&Editor)`, `apply(&mut Editor) -> Result<EditOutcome, AiError>`
(round 6: `EditOutcome.changed` is read; nothing new is asked of the backend).
All of it is touched in one file, `src/ai.rs`.

`editor_store`: `DocumentSession::{prepare, create, path}`,
`load_project_with_session`, `save_project_with_session` (duplicate only, on
its worker), `SaveWorker::{request_document, request_recovery}`,
`load_recovery_with_document` / `RecoveredSession`, `save_recovery_with_document`
(exit only, when the worker took no request and is idle),
`StoreError::{DocumentConflict, DocumentBusy, RecoveryInUse}`.

No new dependency is needed.

- 2026-10-12, round 4, **source only, not compiled**: restored work without a
  file takes a new file only (see "Restored work without a file" below).
  `src/app.rs` alone changed; pinned `rustfmt --edition 2024 --check` clean.
  No Cargo, provider or native run this round. Pending the next grant:
  `cargo test --profile ci --bin neptune-video --locked` (83 before, 86 expected) and
  clippy; then a native run of unbound restore, Save As onto an existing
  project (refused, bytes equal), Save As to a new file, Save again.

- 2026-10-12, round 6, **source only, not compiled, tests not run**: an
  applied proposal that left the project as it was is no longer called
  applied (see "Apply says what the editor reported" below). Changed:
  `src/ai.rs`, `src/app.rs`, `src/ui/agent.rs`. Pinned
  `rustfmt --edition 2024 --check` on those three is clean. No Cargo, native
  process, provider, display or binary was touched this round. Pending for
  root: `cargo test --profile ci --bin neptune-video --locked` (86 before, 90
  expected: three in `ai::tests`, one in `app::tests`) and clippy. The three
  `ai::tests` are `cfg(unix)` and start an owned `/bin/sh` script from a
  fresh temp directory as the provider, through `AiRuntime`; no real provider
  is called. Not captured: the "No changes" card in the running build.

- 2026-10-12, native round 5 (test sub-agent), **Linux acceptance of the
  existing release `neptune-video` `9f721c8b…17131c`** (source `db30b8bd…373016`,
  record `artifacts/production-readiness/final/db30-native-round5-build-record.json`).
  No Cargo and no source edit here. Record and captures:
  `artifacts/native/ai-round8/report.json`.
  - **Restored work without a file, real GTK Save dialogs** (private portal
    session, XTest): a copy with its binding removed, and a second window's
    private copy, each restore dirty with both history steps. Choosing the
    original existing project, then another valid project, and answering
    GTK's Replace is refused with a notice; every owned file, recovery copy
    included, is byte-identical before and after, the document and revision
    do not move, undo and redo still run in full. A new destination saves;
    an edit and Save then write that same file with no dialog; the original
    bytes never change. After that adoption an ordinary Save As onto a
    confirmed file replaces it, and Duplicate onto an existing file is refused.
  - **Real providers from the Ask field**, "split the text clip at 2 seconds
    on the timeline", pending and proposal at 1440×900 and 760×560. Claude
    (`claude-haiku-5-5` on the card, the command line's default here): 3
    requests, 1 cancelled by the stop button (no proposal or edit in 30 s),
    2 proposals; a marker added by hand made the first stale and Apply did
    nothing; the second applied as one revision, exactly the split, one undo
    and one redo exact with the same new clip id. Codex (`gpt-6.1-sol`): 3
    requests, 3 proposals; the first closed by reopening the file; **the
    second was a bad plan** (summary says split, the one step is "Transform
    clip", no change line), read on the card and not applied; the retry was
    a split and applied with the same exact undo and redo.
  - `scripts/native-smoke.py` unchanged: 13 of 13; owned media and fixture
    hashes unchanged.
  - Not shown here: the card's status after applying a no-op plan (root's
    round-6 finding; `src/ai.rs`, `src/app.rs`, `src/ui/agent.rs` on disk
    already differ from this build and need their own build and native run),
    a cancel racing an answer in flight, a destination appearing between
    choosing and writing, macOS, Windows, Wayland, sound, playback rate.
  - Every Neptune Video, Xvfb, DBus and portal process started here is stopped.

## Decisions taken here

- **Apply says what the editor reported** (round 6). `Planner::apply` returns
  `Applied::Changed` or `Applied::Unchanged` from `EditOutcome.changed`.
  Changed: the card reads "Applied. Undo takes it back in one step.", the
  status says "Applied", and the window marks the document changed. Unchanged
  (valid edits whose net result is the same project): the card reads "No
  changes. The project already matched these edits." in the quiet colour, the
  status says "No changes", and nothing is marked changed; revision, dirty
  state, root and redo are the editor's, untouched. It is not a failure: no
  warning, no "Not applied". Stale, refused, discarded and cancelled are as
  before. Either way the proposal is closed. Before Apply such a proposal
  still looks like any other with steps (its list of changes is empty); the
  card does not predict the outcome.

- **Ask and Command are separate modes of one field.** `Ask` sends the request
  to the model; `Command` runs the existing exact language and asks no model.
  `Cmd+K` opens the command line (as before, so `scripts/native-smoke.py`'s
  command-panel check is unchanged); `Shift+Cmd+K` opens Ask. The mode is kept
  in settings (`agent_ask`, default Ask).
- **Settings hold no credential.** `ai: { provider, effort, claude_model,
  codex_model, claude_executable, codex_executable }`. The executables are
  file-only overrides; provider, model and effort are in the panel's menu and
  the Settings sheet. Effort is Default/Low/Medium/High (`None`, `low`,
  `medium`, `high`); `xhigh`/`max` are not offered.
- **One request, one open proposal.** A new request sets an unapplied proposal
  aside. Replacing the project cancels the request and closes the proposal.
  A proposal with no commands is shown as the model's answer with no Apply.
- **Apply** commits held field values first (they are edits), then calls
  `AiProposal::apply`; whatever it returns is what the card says. A proposal
  that is no longer current shows that and cannot be applied.
- **Save As matches the document, not the revision.** The prepared session is
  dropped when the project or editing context changed while the destination
  was looked at. When it still matches, the save takes the document as it is
  at that moment (an edit made during the look is part of what is saved); the
  acknowledgement is per revision as before. A destination that is the open
  file reuses the open session, so Save As onto one's own conflicted file
  stays a conflict.
- **Duplicate** uses `DocumentSession::create` + `save_project_with_session`
  on the duplicate's own worker: an existing destination is refused, including
  one that appears after the check. (The OS save dialog may already have asked
  "replace?"; the answer is still no.)
- **Recovery** keeps its own lease. A copy written beside the open file
  (base equal to the session's canonical `path()`) is queued with that session;
  a private or unsaved copy is queued with `None`. Restoring adopts the session
  the copy recorded and never prepares a new one: when the file changed since,
  the toolbar says "Changed on disk" at once, a notice says so, and the first
  save is refused into the conflict sheet. A copy that recorded no file (old,
  private, moved, or a file that cannot be validated) is restored as a project
  with no file even when it was offered while opening one; it keeps its
  recovery base and lease, and Save asks where. Opening a file this window
  still holds a copy for removes that copy: its work was just given up.
- **Restored work without a file** (round 4): the window keeps an `Origin`,
  `Ordinary` or `Restored`. Only a restore whose copy came back unbound sets
  `Restored`; `replace_editor` (new, open, duplicate taking over, any other
  restore) and a save that landed set `Ordinary`. Save As looks at its
  destination with `Origin::prepare`: `DocumentSession::prepare` for
  `Ordinary`, `DocumentSession::create` for `Restored`, so a path that is
  taken is refused with a notice before anything is written and a file that
  arrives after the look is refused by the save (conflict sheet). The work
  stays dirty with its history, recovery base and lease. The origin travels
  in `Job::Prepared` and is matched with epoch and editing context when the
  answer comes back. The OS dialog may already have asked "replace?"; the
  answer is still no, as for Duplicate. A new project that was never saved,
  an opened one and bound restored work replace a chosen project as before.
- **Conflict/busy** bring up a sheet: Save as…, Discard and reopen (conflict on
  the open file only), Try again (busy only), Not now. The toolbar says
  "Changed on disk" until the document is saved elsewhere or reopened. Nothing
  refreshes the baseline.

- **Second window of one project** (contract update): when the lease beside
  the file cannot be taken, the window switches to
  `<settings dir>/recovery/<editor instance>.nvp` for its recovery copy and
  takes that lease instead. Its save destination and `DocumentSession` do not
  change. It never writes or removes the copy beside the file. After a crash
  that private copy is offered at the next launch without a project, as a
  project with no file (Save asks where): it is not re-attached to the file
  the other window owns.

- **Typed contention and removal** (contract update): the private recovery
  home is chosen only for `StoreError::RecoveryInUse`; any other lease failure
  is shown and retried as before. Every asynchronous removal of a recovery
  copy carries the window's `Arc<RecoveryLease>` into its job, or takes the
  lease inside the job and leaves the copy when it is in use.

## Questions for root

1. The batch label of an applied proposal is the whole summary (up to 2048
   bytes). The toolbar now cuts history labels to 60 characters in its
   tooltips; a short `label` field from the model would be better.
2. Closed in source by `recovery-contract.md` (see Recovery above); to be
   proven by the compiled tests and the native crash, other writer, restore run.
3. Is `AiRuntime` cancelled and joined on drop? The desktop relies on drop at
   exit and does not wait for a provider process.

4. **Codex failed 3 of 5 real requests** from the desktop in round 3 (same
   prompt, same project; requests 2 to 4, one of them in a new process),
   each shown as "Codex could not complete the planning turn" with nothing
   edited. `codex exec` answered meanwhile, and `codex-probe.py` classified
   an app-server error as schema / oneOf / not permitted. `provider/codex.rs`
   returns one fixed sentence for every terminal `error`, so the panel cannot
   tell a rejected schema from a limit or an outage. A short, non-private
   reason in `AiEvent::Failed` would let it. I did not edit the adapter.
5. Empty `.lock` and `.write-lock` files stay beside a project after a clean
   exit. Nothing later was blocked by them. Intended?

## Round 7 native acceptance returned (ai-round9, 2026-10-12)

Binary 2c596cb0…, source 28259254… (hashed before and after, unchanged; no
Cargo, no source edits). Report: `artifacts/native/ai-round9/report.json`.
Owned no-op fixture via Ask → Apply shows the quiet no-op card with revision,
dirty and history unchanged and redo intact; real Claude and Codex each applied
one exact split on the first request; 13/13 smoke; legacy-unbound restore spot
and native Import files/folder passed on the private GTK portal. No defect
needs a source round. For root: a clicked Apply shows no status toast (neither
"Applied" nor "No changes"); the card is the only place the result is said.
All owned processes stopped.
