//! Compact, bounded descriptions of durable project edits. No project roots or media data.
use crate::{Clip, Project, Track};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use uuid::Uuid;

const MOST_EVENTS: usize = 128;
const MOST_OBJECTS: usize = 256;
const MOST_CLIP_COMPARISONS: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObjectChange {
    pub kind: String,
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<Uuid>,
    /// added, removed or updated.
    pub change: String,
    /// Changed fields only. Scalar values are the new value; null marks a complex change.
    pub fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangeSummary {
    pub label: String,
    pub objects: Vec<ObjectChange>,
    /// Some object details may be omitted by the bounded diff; inspect for remaining details.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RevisionChange {
    pub revision: u64,
    #[serde(flatten)]
    pub summary: ChangeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangesSince {
    pub since_revision: u64,
    pub revision: u64,
    /// Changes after this revision remain available in the transient journal.
    pub oldest_revision: u64,
    /// False after reload/recovery, or when the 128-event journal evicted requested changes.
    pub complete: bool,
    pub entries: Vec<RevisionChange>,
}

#[derive(Default)]
pub(crate) struct ChangeJournal(VecDeque<RevisionChange>);

impl ChangeJournal {
    pub fn record(&mut self, revision: u64, label: &str, before: &Project, after: &Project) {
        self.0.push_back(RevisionChange {
            revision,
            summary: summarize_changes(label, before, after),
        });
        while self.0.len() > MOST_EVENTS {
            self.0.pop_front();
        }
    }

    pub fn since(&self, since: u64, revision: u64) -> ChangesSince {
        let oldest = self
            .0
            .front()
            .map_or(revision, |entry| entry.revision.saturating_sub(1));
        ChangesSince {
            since_revision: since,
            revision,
            oldest_revision: oldest,
            complete: since >= oldest && since <= revision,
            entries: self
                .0
                .iter()
                .filter(|entry| entry.revision > since)
                .cloned()
                .collect(),
        }
    }
}

/// Summarize changed objects without serializing full clips, animation keys or asset metadata.
/// Shared tracks and unchanged entities are skipped; the result is capped at 256 objects.
pub fn summarize_changes(label: &str, before: &Project, after: &Project) -> ChangeSummary {
    let mut summary = ChangeSummary {
        label: short(label),
        objects: Vec::new(),
        truncated: false,
    };
    let mut fields = BTreeMap::new();
    if before.name != after.name {
        fields.insert("name".into(), json!(short(&after.name)));
    }
    if before.settings != after.settings {
        fields.insert("settings".into(), Value::Null);
    }
    if !before
        .tracks
        .iter()
        .map(|track| track.id)
        .eq(after.tracks.iter().map(|track| track.id))
    {
        fields.insert("track_order".into(), Value::Null);
    }
    if !fields.is_empty() {
        push(&mut summary, "project", after.id, None, "updated", fields);
    }
    let previous: BTreeMap<_, _> = before
        .tracks
        .iter()
        .map(|track| (track.id, track))
        .collect();
    let current: BTreeMap<_, _> = after.tracks.iter().map(|track| (track.id, track)).collect();
    for id in previous
        .keys()
        .chain(current.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        if summary.truncated {
            break;
        }
        match (previous.get(&id), current.get(&id)) {
            (Some(old), Some(new)) => {
                if std::sync::Arc::ptr_eq(old, new) {
                    continue;
                }
                let fields = track_fields(old, new);
                if !fields.is_empty() {
                    push(&mut summary, "track", id, None, "updated", fields);
                }
                if old.clips.len().saturating_add(new.clips.len()) > MOST_CLIP_COMPARISONS {
                    push(
                        &mut summary,
                        "track",
                        id,
                        None,
                        "updated",
                        BTreeMap::from([("clips".into(), Value::Null)]),
                    );
                    summary.truncated = true;
                    continue;
                }
                if old.clips == new.clips {
                    continue;
                }
                diff_clips(&mut summary, id, &old.clips, &new.clips);
            }
            (Some(track), None) | (None, Some(track)) => {
                let change = if current.contains_key(&id) {
                    "added"
                } else {
                    "removed"
                };
                push(
                    &mut summary,
                    "track",
                    id,
                    None,
                    change,
                    BTreeMap::from([("name".into(), json!(short(&track.name)))]),
                );
                for clip in &track.clips {
                    if summary.truncated {
                        break;
                    }
                    push(
                        &mut summary,
                        "clip",
                        clip.id,
                        Some(id),
                        change,
                        BTreeMap::new(),
                    );
                }
            }
            _ => {}
        }
    }
    if !std::sync::Arc::ptr_eq(&before.assets, &after.assets) {
        diff_map(&mut summary, "asset", &before.assets, &after.assets);
    }
    if !std::sync::Arc::ptr_eq(&before.folders, &after.folders) {
        diff_map(&mut summary, "folder", &before.folders, &after.folders);
    }
    if !std::sync::Arc::ptr_eq(&before.trackers, &after.trackers) {
        diff_map(&mut summary, "tracker", &before.trackers, &after.trackers);
    }
    if !std::sync::Arc::ptr_eq(&before.markers, &after.markers) {
        let old_markers = before
            .markers
            .iter()
            .map(|marker| (marker.id, marker))
            .collect();
        let new_markers = after
            .markers
            .iter()
            .map(|marker| (marker.id, marker))
            .collect();
        diff_map(&mut summary, "marker", &old_markers, &new_markers);
    }
    summary
}

fn diff_clips(summary: &mut ChangeSummary, track_id: Uuid, old: &[Clip], new: &[Clip]) {
    // Most edits keep IDs in the same order. Only mismatched positions need an
    // identity lookup; retain at most 512 candidates, then scan both vectors once.
    // This bounds auxiliary memory even for a cut moving across 100,000 clips.
    let mut candidates = BTreeSet::new();
    let mut incomplete = false;
    for index in 0..old.len().max(new.len()) {
        match (old.get(index), new.get(index)) {
            (Some(before), Some(after)) if before.id == after.id => {
                if before != after {
                    push(
                        summary,
                        "clip",
                        after.id,
                        Some(track_id),
                        "updated",
                        clip_fields(before, after),
                    );
                    if summary.truncated {
                        return;
                    }
                }
            }
            (before, after) => {
                for clip in before.into_iter().chain(after) {
                    if candidates.len() < MOST_OBJECTS * 2 {
                        candidates.insert(clip.id);
                    } else if !candidates.contains(&clip.id) {
                        incomplete = true;
                    }
                }
            }
        }
    }
    if candidates.is_empty() {
        return;
    }
    let mut previous = BTreeMap::new();
    let mut current = BTreeMap::new();
    for clip in old {
        if candidates.contains(&clip.id) {
            previous.insert(clip.id, clip);
        }
    }
    for clip in new {
        if candidates.contains(&clip.id) {
            current.insert(clip.id, clip);
        }
    }
    for id in candidates {
        match (previous.get(&id), current.get(&id)) {
            (Some(before), Some(after)) if before != after => {
                push(
                    summary,
                    "clip",
                    id,
                    Some(track_id),
                    "updated",
                    clip_fields(before, after),
                );
            }
            (Some(_), None) => push(
                summary,
                "clip",
                id,
                Some(track_id),
                "removed",
                BTreeMap::new(),
            ),
            (None, Some(_)) => push(
                summary,
                "clip",
                id,
                Some(track_id),
                "added",
                BTreeMap::new(),
            ),
            _ => {}
        }
        if summary.truncated {
            return;
        }
    }
    summary.truncated |= incomplete;
}

fn diff_map<T: PartialEq>(
    summary: &mut ChangeSummary,
    kind: &str,
    old: &BTreeMap<Uuid, T>,
    new: &BTreeMap<Uuid, T>,
) {
    for (id, previous) in old {
        if summary.truncated {
            return;
        }
        let change = match new.get(id) {
            Some(current) if previous != current => "updated",
            None => "removed",
            _ => continue,
        };
        push(summary, kind, *id, None, change, BTreeMap::new());
    }
    for id in new.keys() {
        if summary.truncated {
            return;
        }
        if !old.contains_key(id) {
            push(summary, kind, *id, None, "added", BTreeMap::new());
        }
    }
}

fn push(
    summary: &mut ChangeSummary,
    kind: &str,
    id: Uuid,
    track_id: Option<Uuid>,
    change: &str,
    fields: BTreeMap<String, Value>,
) {
    if summary.objects.len() == MOST_OBJECTS {
        summary.truncated = true;
        return;
    }
    summary.objects.push(ObjectChange {
        kind: kind.into(),
        id,
        track_id,
        change: change.into(),
        fields,
    });
}

fn short(text: &str) -> String {
    let mut chars = text.chars();
    let mut shortened: String = chars.by_ref().take(256).collect();
    if chars.next().is_some() {
        shortened.push('…');
    }
    shortened
}

fn track_fields(before: &Track, after: &Track) -> BTreeMap<String, Value> {
    let mut fields = BTreeMap::new();
    macro_rules! changed { ($($field:ident),* $(,)?) => { $(if before.$field != after.$field { fields.insert(stringify!($field).into(), json!(after.$field)); })* }; }
    if before.name != after.name {
        fields.insert("name".into(), json!(short(&after.name)));
    }
    if before.language != after.language {
        fields.insert(
            "language".into(),
            json!(after.language.as_deref().map(short)),
        );
    }
    changed!(kind, locked, hidden, muted, gain, solo);
    fields
}

fn clip_fields(before: &Clip, after: &Clip) -> BTreeMap<String, Value> {
    let mut fields = BTreeMap::new();
    macro_rules! scalar { ($($field:ident),* $(,)?) => { $(if before.$field != after.$field { fields.insert(stringify!($field).into(), json!(after.$field)); })* }; }
    if before.name != after.name {
        fields.insert("name".into(), json!(short(&after.name)));
    }
    scalar!(
        start,
        duration,
        source_in,
        playback_rate,
        linked_id,
        group_id,
        gain,
        blend,
        fade_in,
        fade_out
    );
    macro_rules! complex { ($($field:ident),* $(,)?) => { $(if before.$field != after.$field { fields.insert(stringify!($field).into(), Value::Null); })* }; }
    complex!(
        content,
        transform,
        effects,
        masks,
        animation,
        retime,
        transition_in,
        transition_out,
        follow
    );
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Editor, Time, TrackKind};
    use std::sync::Arc;

    #[test]
    fn journal_names_track_state_changes_and_tracks_undo_and_redo() {
        let mut project = Project::new("Journal");
        let track = Track::new("Presenter", TrackKind::Video);
        let id = track.id;
        project.tracks.push(Arc::new(track));
        let mut editor = Editor::new(project).unwrap();
        editor
            .dispatch(Command::SetTrackState {
                track_id: id,
                locked: None,
                hidden: Some(true),
                muted: Some(true),
                name: None,
                gain: None,
                solo: None,
            })
            .unwrap();
        let changed = editor.changes_since(0);
        assert!(changed.complete);
        let track = changed.entries[0]
            .summary
            .objects
            .iter()
            .find(|object| object.id == id)
            .unwrap();
        assert_eq!(track.fields["hidden"], true);
        assert_eq!(track.fields["muted"], true);
        assert!(editor.undo());
        assert!(editor.redo());
        let changed = editor.changes_since(1);
        assert_eq!(
            changed
                .entries
                .iter()
                .map(|entry| entry.revision)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(
            changed.entries[0].summary.objects[0].fields["hidden"],
            false
        );
        assert_eq!(changed.entries[1].summary.objects[0].fields["hidden"], true);
        assert!(!editor.changes_since(4).complete);
    }

    #[test]
    fn journal_bounds_retention_and_reports_reload_gaps() {
        let mut editor = Editor::new(Project::new("Initial")).unwrap();
        for index in 0..130 {
            editor
                .dispatch(Command::RenameProject {
                    name: format!("Edit {index}"),
                })
                .unwrap();
        }
        let changed = editor.changes_since(0);
        assert!(!changed.complete);
        assert_eq!(changed.oldest_revision, 2);
        assert_eq!(changed.entries.len(), 128);
        assert!(editor.changes_since(2).complete);
        let restored = Editor::from_session(editor.session_state()).unwrap();
        assert!(!restored.changes_since(0).complete);
        assert!(restored.changes_since(130).complete);
    }

    #[test]
    fn large_edits_return_explicitly_truncated_object_changes() {
        let before = Project::new("Before");
        let mut after = before.clone();
        let mut track = Track::new("Captions", TrackKind::Text);
        for index in 0..300 {
            track
                .clips
                .push(Clip::text("Word", Time(index * 1_000_000), Time(1_000_000)));
        }
        after.tracks.push(Arc::new(track));
        let summary = summarize_changes("Many captions", &before, &after);
        assert_eq!(summary.objects.len(), 256);
        assert!(summary.truncated);
    }
    #[test]
    fn giant_tracks_emit_a_bounded_inspection_hint_and_shared_tracks_are_skipped() {
        let mut before = Project::new("Large");
        let mut track = Track::new("Many captions", crate::TrackKind::Text);
        for index in 0..3000 {
            track.clips.push(Clip::text(
                "Caption",
                Time(index * 1_000_000),
                Time(1_000_000),
            ));
        }
        let id = track.id;
        before.tracks.push(Arc::new(track));
        let mut after = before.clone();
        Arc::make_mut(after.tracks.last_mut().unwrap()).clips[2999].name = "Changed".into();
        let summary = summarize_changes("One distant edit", &before, &after);
        assert!(summary.truncated);
        assert_eq!(summary.objects.len(), 1);
        assert_eq!(summary.objects[0].id, id);
        assert!(summary.objects[0].fields.contains_key("clips"));
        let shared = summarize_changes("No changes", &before, &before.clone());
        assert!(shared.objects.is_empty());
        assert!(!shared.truncated);
    }
}
