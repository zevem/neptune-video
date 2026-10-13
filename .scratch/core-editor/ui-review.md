# UI integration review

Current native binary compiles; root is preparing real native screenshots and input tests. Please address these integration findings before finishing or in the next delegated round.

1. AudioService exposes `position()->Option<Time>` from actual output device; current App.logic uses only core wall clock. Use audio as master clock after preload and fallback while None, preserving end/loop/frame scheduling. Avoid seek() resetting anchors every tick need method `Playback::follow_clock(Time,duration)` if needed (root can provide). Call sync only on user/project transport changes.
2. Autosave currently requires self.path Some, so new unsaved projects never recover. Use unique per-editor recovery destination under settings_dir/recovery and retain/recover on startup with concrete UI affordance. Never fabricate sample media on launch.
3. App.take_saves currently proceeds with after_save even when newer edits made during asynchronous save remain dirty. Requeue current snapshot or prompt/guard again; do not silently discard those edits after an older save acknowledgement.
4. Store SaveResult now includes context_id (Editor.instance_id()) and project_id. Check them before applying completions, not just path. New Editor load resets revision, so old save completion for same path must not mark it saved or discard its recovery. Check recovery errors against context too.
5. After an explicit save, removing .recovery unconditionally can delete an autosave containing NEWER unsaved edits. Preserve/rewrite the latest sidecar, or delete only when captured/current document proven identical and no recovery write race. Full history currently only persists in recovery.
6. App::load async result can arrive after fresh edits/agent changes while file dialog/load was underway and silently replace them. Capture context/revision at load start and guard newly dirty work before accepting result. Apply latest file-dialog intent and protect newer current edits, including save-before-open flow.
7. Root backend fixes: Agent requests now require expected_revision, expected_project_id, expected_context_id. Inspect response carries project + context_id. Host.execute has atomic acceptance/cancellation semantics; no UI request field construction needed. Store recovery validates same project identity across all history roots.
8. Root native build observed unused icons/settings.forget/Action::{AgentClear,Problem}/Hit::Lane payload warnings. Please remove genuinely unused items, then run UI-focused Clippy when root compilation window is clear. Avoid broad dead-code allow.

Root will send exact native findings in a distinct delegated follow-up once first UI task finishes; don't start duplicate work elsewhere.
