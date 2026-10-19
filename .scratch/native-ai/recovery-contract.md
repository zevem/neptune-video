# Crash recovery document binding

Backend APIs are implemented; focused validation is pending the compilation
owner's grant. `editor-store` owns this change.

`load_recovery_with_document(path: &Path) -> Result<RecoveredSession, StoreError>`
loads `recovery_path(path)` on the loading worker. The named result has:

- `editor: Editor`: complete validated history, with a fresh live context.
- `document: Option<Arc<DocumentSession>>`: the exact canonical ordinary path
  and expected byte/hash baseline recorded in that archive, never today's disk
  bytes. Retain this session through subsequent protected ordinary saves.
- `document_changed: bool`: a valid ordinary document differs from the recorded
  baseline. Edits/history still restore; a later save
  rechecks the original baseline and refuses conflicts. Surface this state.

When `document` is `None`, restore **unsaved**, with no ordinary save destination.
This includes old archives without bindings, unsaved/private per-window recovery,
archives copied elsewhere and destinations not meaningful on this platform.
Damaged, future or inaccessible ordinary destinations also restore unbound:
archival JSON must not invent save authority over an unvalidated file.
The returned editor is dirty in these cases. Keep its recovery base and lease,
then offer Save As when the person saves. Do not infer authority from an offered
project path or call `DocumentSession::prepare` during restoration.

`SaveWorker::request_recovery(path: PathBuf, editor: &Editor,
lease: Arc<RecoveryLease>, document: Option<Arc<DocumentSession>>)` queues the
immutable project/history and document binding together, capturing the binding
at submission without filesystem I/O. For ordinary named recovery, pass the
session's canonical `path()` and `Some(session.clone())`. For unsaved/private
recovery, pass that independent recovery base and `None`. Passing a session
whose path differs returns an error; a private archive cannot bind an ordinary
file. Match existing completion context/project/revision checks as before.

The baseline publication lock spans memory access only. A manual save does not
hold it while serializing, syncing or replacing. A recovery queued during a
manual save can conservatively retain the earlier baseline: after a crash it
may require Save As even if this editor performed that save, but it cannot
silently authorize overwriting independently changed bytes. A newer recovery
submission after acknowledgement captures the advanced baseline normally.

Synchronous storage/startup-worker callers can use
`save_recovery_with_document(document: &DocumentSession, editor: &Editor)` for
the session's named sidecar. Existing `save_recovery`, `load_recovery`,
`request(..., recovery=true)` and `request_with_lease(..., recovery=true)` remain
compatibility/history-only APIs; they write unbound recovery and convey no
ordinary save authority.

The ordinary `.nvp` format remains version 1. Recovery adds the optional
`document_binding` field, with its own version 1, canonical destination and
missing/present byte+SHA-256 baseline. Existing archives decode without it.
Unsupported binding versions and malformed fingerprints are rejected so their
archive bytes remain protected. No source media or credentials are read.
