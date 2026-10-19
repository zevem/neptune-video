# Neptune Video MCP productivity review

Reviewed and implemented on 2026-10-15. The original session/project was not
provided, so its exact payload sizes and HDR footage could not be replayed.
Findings below distinguish verified source behavior, generated-fixture
regressions, existing capabilities, and remaining limits.

## Point-by-point findings

1. **Large and incomplete inspection: confirmed, with an existing full-view
   escape hatch.** Summary included every asset; timeline exposed abbreviated
   clips, lacked filters beyond one track and truncated without continuation.
   Whole-project `full` already read unsaved authority state, so saving and
   reading `.nvp` was unnecessary, but selective rich reads were missing.
   Added slim summary/tracks, paged track/clip/media views, exact half-open
   time ranges, ID filters, top-level fields, and selected exact core Clip
   objects. Default pages contain 50 entries, maximum 500; responses remain
   capped at 4 MiB. A full clip is returned as `{track_id, clip}` and can be
   used as raw `add_clip.clip`. Omitting `fields` retains the complete object.
   The regression round-trips a live unsaved styled/animated clip without
   reading a project file. The original 87–100k-character figures remain
   unverified because the original project is unavailable.

2. **Delete/re-add for every edit: partly a discovery problem.** Existing
   `clip_edit` move/trim and core commands already preserve identity. Atomic
   partial retargeting was missing. Added `patch_clip` backed by a core command
   with stable targets, locks, validation, overlap checks and normal undo.
   Typed time/source/asset fields combine with recursive stored-field changes;
   arrays replace and null clears optional fields. Unknown nested fields and
   ID/link/group replacement are refused. Existing effect/keyframe tools edit
   individual array entries. Added signed `offset_clips` using the existing
   linked/grouped move command. Tests cover unchanged fields, identity,
   serialization, batch rollback, undo/redo and save/recovery round-trips.

3. **Unhelpful validation context: confirmed.** Invalid animation, word and
   text-animation errors omitted the owning clip. They now include clip IDs
   and relevant curve/key/word/effect/mask indices and bounds. Trim interpolation
   already existed in `TrimClip`; patch shortening now uses those rules by
   default. Existing timed data is fitted before explicit replacements, which
   must themselves validate. `trim_animation: preserve` opts into strict
   validation without fitting. The regression samples the old curve and
   checks its interpolated new endpoint. Source-aligned head trimming remains
   `clip_edit {op: trim}`; a patch's source/start assignment is direct.

4. **Asset picture requests snapped to project rate: confirmed.** The tools
   parsed picture times using the original project's rate and snapped seconds/
   microseconds. They now retain exact microseconds and use the subject's rate
   for frame/timecode input. A generated 60 fps source in a 30 fps project
   verifies raw source pixels match an off-project-grid clip in-point. This
   retains Neptune Video's compositor/decoder/tone-mapping path, rather than substituting
   a separate FFmpeg image command. It does not establish arbitrary VFR PTS
   selection or HDR-footage pixel acceptance; the existing decoder still
   selects source-grid frames.

5. **Cannot isolate a layer: confirmed.** Picture tools now accept
   `only_tracks`, `exclude_tracks` and `solo_clip`, projecting an immutable
   snapshot. Explicit inclusion can reveal hidden tracks only for that render;
   transforms, grade, effects and text styling remain. Source/effect resources
   remain available. PNG/JPEG sequence jobs support the same selectors.
   Tests verify hidden/excluded/solo states and unchanged editing revision and
   visibility. Native encoded-video exports still capture the authority's
   project and do not accept isolation selectors; isolated frame dumps cover
   the matte workflow without changing the person's visibility.

6. **Inline pixel cap and missing output folders: confirmed.** The cap is
   64 Mi pixels (67,108,864), plus 48 pictures, rather than a streaming API.
   Retained these memory limits and added `frames_export`: one bounded GPU
   lane, one-frame delivery buffer, up to 100,000 sample times, per-frame
   encoding/write and cancellation. It returns a pattern/count/sampling recipe
   instead of a huge image/path response. Rational frame steps avoid accumulated
   interval rounding. Existing outputs are checked before the dump begins;
   completed files remain on cancellation or later failure. Picture, sheet and
   encoded-video output parents are created through root-checked paths, with
   symlink/outside-root regression controls. A 60-frame job proves one call can
   exceed the inline frame limit.

7. **Only individual-clip loudness: confirmed.** Added timeline/mix scope to
   `loudness_start`, with exact range, selected tracks and optional per-track
   readings. The meter streams the same stereo PCM as export, after its master
   limiter, including actual gains/fades/effects/retimes/mute/solo and detached
   sound ownership. Media checks consider contributing audio only. Measurement
   and export no longer decode an unrequested final partial chunk. The tone
   fixture verifies two coherent voices add approximately 6 dB, mute changes
   the reading, silence has null figures, and analysis never edits. Per-track
   readings pass independently through the master stage and are not additive.
   Results identify project/context/revision, rate/layout/stage and omitted
   filters. These are pre-encoding PCM measurements; codecs can change peaks.

8. **Generic history labels conceal human changes: confirmed.** Kept the
   label-array format, bounded to latest 50 undo/next 50 redo entries, and added
   summaries with actual IDs, changed fields and simple new values. Added
   `history {op: changes_since, since_revision}` over a transient 128-event
   core journal, covering commands, undo and redo. Every summary is bounded;
   giant changed tracks return conservative truncated hints instead of scanning
   all clips on the interactive thread. Reload/eviction is explicit through
   `complete` and `oldest_revision`. Journal reads carry identity and enforce
   supplied project/context guards, including replacement races. This is not a
   persisted audit log. Tests cover human hide/mute values, undo/redo, retention,
   giant-track bounds and stale context/project reads.

9. **Ignored expected revision: not reproduced for current raw edit_apply;
   narrower gaps confirmed and fixed.** The existing raw tool already used the
   shared guarded apply path. New integration tests reject stale raw edits both
   with and without dry-run. Closed lower-level dry-run, duplicate-import,
   optional undo/redo and explicit save-revision gaps. Added opt-in
   `wait_if_busy` for shared editing, imports, undo/redo, restore and saves.
   Only definitely-unapplied busy refusals retry; the command and captured
   guards remain fixed. The retry-admission budget is five seconds plus normal
   transport request deadlines. Successful/indeterminate requests are never
   replayed. A real authenticated AgentHost control covers busy→success,
   busy→human edit→stale revision and no replay after indeterminate response.

10. **Paper cuts: confirmed.** `contact_sheet` had no `times` input and silently
    ignored the unknown property; it now consumes explicit ordered times and
    refuses conflicting even-sampling options. Asset-taking convenience tools
    accept a full UUID or unique prefix of at least four hexadecimal digits,
    with actionable ambiguous/malformed/not-found errors. Raw commands and
    multi-asset lists retain full UUIDs. Added paged `media_usage` returning
    source/effect references with full asset/track/clip/effect IDs. Regression
    controls cover prefix collisions, actual prefix-based calls, usage paging,
    explicit contact times and microsecond retention.

## Integration findings

The new end-to-end rendering test exposed an additional cache bug: changing a
clip's transform reused a same-ID decode-size cache across MCP snapshots.
Earlier source/off-grid equality passed, but an isolated dump after a differently
sized render produced different pixels from the earlier graded still. Clearing
snapshot-derived renderer caches once per MCP request retains GPU pipelines and
within-sequence decoder continuity. The strict pixel comparison is retained.

Validation also exposed a shutdown race hidden by the client transport's
discarded child exit status. Before the fix, two frame-only and two
frame-plus-export trials returned successful tool responses and then exited
with SIGSEGV; two export-only controls exited normally. A process trace
identified the frame worker. Joining it removed those crashes; removing an
incidental idle timeout then exposed the same detached teardown in export-only
trials (two SIGSEGV exits). The server now cancels source work and joins both
GPU-owning workers before returning to process exit. Idle waits remain
blocking; native GUI shutdown retains its bounded behavior. A standing
integration test retains the child handle and asserts actual exits for frame,
export and combined cases; a separate six-trial lifecycle matrix checks clean
exits. No C-library backtrace was captured, so the diagnosis rests on the
process trace, the worker lifetimes and the before/after controls.

The expanded tool catalog initially measured 50,534 bytes, exceeding the existing
40 KiB client budget. Input-schema compaction now removes redundant annotations
while retaining structural constraints, operation choices, stored shapes and
value/unit guidance. Shared usage guidance is in server instructions and
[the MCP guide](../../docs/mcp.md). The 43-tool catalog measures 37,342 bytes.
The native provider's narrower closed proposal schema excludes free-form patch
maps; MCP and raw command discovery expose them normally.

## Validation

Actual commands, host, results and limitations are recorded in
[the verification entry](../../docs/verification.md#mcp-productivity-follow-up).
Logs are under [artifacts/mcp-productivity](../../artifacts/mcp-productivity/).
Detailed source-review notes are in [clip edits](clip-edits.md),
[inspection/history](inspect-history.md), [rendering](rendering.md) and
[independent loudness review](loudness-review.md).
