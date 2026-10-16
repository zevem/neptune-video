# Inspection, edit history and guards

Scope: report points 1, 8 and 9. Asset usage and UUID prefix handling are owned by the parent implementation.

## Verified against the original implementation

- `project_inspect` supported summary, timeline, media and full, plus one `track_id`. Summary serialized all assets. Timeline listed up to 2,000 compact clips, then set `truncated` without a cursor. The compact view omitted full animation, effect parameters, title style, transforms and other persisted fields. Bare full already read the authority's live project without saving, capped at 4 MiB. Reading the saved file was unnecessary but the useful selective full view was missing.
- `history` exposed only generic undo/redo labels. Core history already held before/after immutable project roots, so a compact actual difference can be derived at the owning boundary. Tracking a revision through undo/redo cannot be inferred from stack indices because those operations advance revisions and branch edits discard redo.
- Raw `edit_apply` already called `AsterServer::edit`, which checked the explicit expected revision before building and sent a guarded request. The report that this path ignored expected_revision was not reproduced against this checkout. Lower-level Workspace dry runs bypassed the explicit check; duplicate-only media imports returned before checking it; history undo/redo did not expose the optional explicit guard; save discarded an explicit revision at the protocol identity check. These gaps are addressed.
- Desktop `person_editing`, `desktop_busy`, host queue `busy`, and `timeout` explicitly mean nothing applied. Transport errors and `indeterminate` may already have applied, so retrying those automatically would risk duplicating edits.

## Implemented interface

`project_inspect` now offers:

- `scope: tracks`: paged slim track IDs, names, kinds, hidden/muted/solo/locked/gain state, counts and ends. Summary track lists use the same page; timeline track lists are bounded to the first page and report tracks_total/tracks_truncated (read scope tracks for more).
- Slim `summary`: settings, duration, track state, counts, and bounded markers. Media metadata moved to the existing media scope.
- Clip filtering by `track_id`, `track_ids`, `clip_ids`, `asset_ids` and exact half-open `time_range: {start: TimeSpec, end: TimeSpec}`.
- Clip/media pages: `limit` defaults to 50, maximum 500; `offset`, `total`, `next_offset`, `truncated`, and authority `context_id`/`revision`. Metadata is built only for the requested media page. Clip pagination retains only the returned page, not all matching clips.
- `scope: clips` or `detail: full` on timeline, and filtered `scope: full`, yield `{track_id, clip}`. `clip` is the exact serialized core object accepted by `edit_apply` `add_clip`, read from the current in-memory authority, without saving. Bare full keeps the existing whole-project response.
- `fields` projects top-level clip/media fields and retains IDs. Individual responses retain the 4 MiB cap.

`history {op: list}` keeps the label-array format (latest 50 past / next 50 future, names capped at 256 characters) and adds `past_entries`, `future_entries`, and `truncated`. Detailed entries cover the latest 50 past entries and first 50 future entries, with at most 256 changed objects per summary. Each object names its kind, stable ID, track ID for clips, added/removed/updated action, and changed fields. New scalar values show hide versus unhide, mute versus unmute, gain, rename and timing; complex fields such as content, animation and effects are marked null to request selective inspection rather than duplicate large blobs.

`history {op: changes_since, since_revision: N}` returns the current context's last 128 mutation events, including human edits, undo and redo. `oldest_revision` and `complete` explicitly report evicted/reloaded/recovered gaps. This is transient collaboration metadata, not a persisted audit log. History and journal protocol reads honor each supplied project/context identity guard; an MCP snapshot followed by a project replacement therefore cannot return another context's empty journal. Raw unguarded reads remain allowed. Changes results include explicit project_id/context_id (raw protocol siblings of changes; MCP flattened alongside journal fields). Each event and name string is bounded. At most 4,096 clip comparisons are allowed per changed track; larger tracks return a clips:null hint on the changed track with truncated:true for selective inspection. Equal shared tracks/assets/folders/trackers/markers are skipped; track-only edits skip clip map construction when clips compare equal.

Editing guards accept `wait_if_busy: true`: up to five seconds of retries for explicit, definitely-unapplied busy refusals only. The originally captured revision/project/context remain unchanged across retries; a human edit during the wait causes `stale_revision`. The edit command is built once. Successful or indeterminate edits are never replayed. Save, undo/redo, imports and checkpoint restore support the same bounded wait. Explicit save revisions are now honored by both authority implementations while omitted revisions retain the prior current-state save behavior.

## Focused evidence to run with the compilation owner

No cargo, rustfmt or native app process was started by this subagent.

- Protocol test `history_and_journal_reads_honor_optional_project_and_context_guards`: guarded reads reject a same-project reload or replacement, unguarded reads remain valid, either optional identity guard works alone, and result identities/nesting are explicit.

- Core unit tests `changes::tests`: hide/mute actual values; undo/redo revisions; eviction and recovery completeness; large edit truncation; giant-track comparison bound and shared-track skip.
- Desktop unit tests `mcp::tools::project::tests`: slim summary/tracks; pages and continuation; selected exact Clip/add_clip roundtrip from live unsaved state; ID/range/field composition; exact half-open edge; rejected unknown IDs and invalid page size.
- Desktop unit tests `mcp::workspace::tests`: stale dry runs leave state/history unchanged; stale undo/save; real authenticated AgentHost busy then success, busy then human edit (stale), and indeterminate without replay.

These tests establish headless behavior on the validation host. Native UI acceptance and performance measurements are separate; no responsiveness, cross-platform or original project-size measurement is claimed here.
