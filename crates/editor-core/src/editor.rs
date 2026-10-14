use crate::{retime::scale, *};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub label: String,
    pub before: Arc<Project>,
    pub after: Arc<Project>,
}

/// An owned immutable capture for autosave. Workers never borrow the live editor.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub current: Arc<Project>,
    pub saved: Option<Arc<Project>>,
    pub past: Vec<HistoryEntry>,
    pub future: Vec<HistoryEntry>,
    pub revision: u64,
}

#[derive(Debug, Clone)]
pub struct EditOutcome {
    pub revision: u64,
    pub label: String,
    pub changed: bool,
}

/// A command reduced and validated against an immutable captured editor root.
///
/// The result can be reviewed off-thread and committed without reducing it
/// again. Its private roots and editing identity prevent callers from replacing
/// validated state through a mutable preview or a serialized project.
#[derive(Debug, Clone)]
pub struct PreparedEdit {
    before: Arc<Project>,
    after: Arc<Project>,
    label: String,
    context_id: Uuid,
    revision: u64,
}

impl PreparedEdit {
    /// Reduce a command against a snapshot captured from the target editor.
    ///
    /// Capture `Editor::snapshot`, `Editor::instance_id` and `Editor::revision`
    /// together. Only that same unchanged editor can commit the result; an
    /// equal-valued copy of the snapshot does not confer commit authority.
    pub fn from_snapshot(
        before: Arc<Project>,
        context_id: Uuid,
        revision: u64,
        command: Command,
    ) -> Result<Self, EditError> {
        let label = command.label().to_owned();
        let mut next = (*before).clone();
        apply(&mut next, command, 0)?;
        for track in &mut next.tracks {
            if !track.clips.windows(2).all(|c| c[0].start <= c[1].start) {
                Arc::make_mut(track).clips.sort_by_key(|c| c.start);
            }
        }
        next.validate()?;
        let after = if next == *before {
            before.clone()
        } else {
            Arc::new(next)
        };
        Ok(Self {
            before,
            after,
            label,
            context_id,
            revision,
        })
    }

    /// Original captured root. Mutating a returned Arc detaches that copy.
    pub fn before(&self) -> Arc<Project> {
        self.before.clone()
    }

    /// Exact validated root that a successful commit installs.
    pub fn after(&self) -> Arc<Project> {
        self.after.clone()
    }

    /// The label captured before reducing the command.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Whether the validated command changes the captured project.
    pub fn changed(&self) -> bool {
        !Arc::ptr_eq(&self.before, &self.after)
    }

    /// All identity guards required for a commit, without reducing or cloning.
    pub fn is_current(&self, editor: &Editor) -> bool {
        self.context_id == editor.instance_id
            && self.before.id == editor.current.id
            && self.revision == editor.revision
            && Arc::ptr_eq(&self.before, &editor.current)
    }
}

pub struct Editor {
    instance_id: Uuid,
    current: Arc<Project>,
    saved: Option<Arc<Project>>,
    past: Vec<HistoryEntry>,
    future: Vec<HistoryEntry>,
    revision: u64,
    // Acknowledgements name the submitted document, even if the user edits or
    // undoes while the atomic write is in progress.
    submitted_saves: BTreeMap<u64, Arc<Project>>,
}
impl Default for Editor {
    fn default() -> Self {
        let project = Arc::new(Project::default());
        Self {
            instance_id: Uuid::new_v4(),
            current: project,
            saved: None,
            past: Vec::new(),
            future: Vec::new(),
            revision: 0,
            submitted_saves: BTreeMap::new(),
        }
    }
}
impl Editor {
    pub fn new(project: Project) -> Result<Self, EditError> {
        project.validate()?;
        let current = Arc::new(project);
        Ok(Self {
            instance_id: Uuid::new_v4(),
            saved: Some(current.clone()),
            current,
            past: Vec::new(),
            future: Vec::new(),
            revision: 0,
            submitted_saves: BTreeMap::new(),
        })
    }
    pub fn project(&self) -> &Project {
        &self.current
    }
    pub fn snapshot(&self) -> Arc<Project> {
        self.current.clone()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Transient editing context. Opening even the same document creates a new
    /// context, so plans from a previous load cannot target a reset revision.
    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }
    pub fn is_dirty(&self) -> bool {
        self.saved
            .as_ref()
            .is_none_or(|s| !Arc::ptr_eq(s, &self.current))
    }
    pub fn register_save(&mut self) -> u64 {
        self.submitted_saves
            .insert(self.revision, self.current.clone());
        self.revision
    }
    pub fn mark_saved(&mut self, revision: u64) {
        if let Some(snapshot) = self.submitted_saves.remove(&revision) {
            self.saved = Some(snapshot);
        } else if self.revision == revision {
            self.saved = Some(self.current.clone());
        }
        self.submitted_saves.retain(|r, _| *r > revision);
    }
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.past.last().map(|h| h.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.future.last().map(|h| h.label.as_str())
    }
    pub fn history(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.past.iter()
    }
    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.past.pop() else {
            return false;
        };
        self.current = entry.before.clone();
        self.future.push(entry);
        self.revision = self.revision.saturating_add(1);
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.future.pop() else {
            return false;
        };
        self.current = entry.after.clone();
        self.past.push(entry);
        self.revision = self.revision.saturating_add(1);
        true
    }
    pub fn session_state(&self) -> SessionState {
        SessionState {
            current: self.current.clone(),
            saved: self.saved.clone(),
            past: self.past.clone(),
            future: self.future.clone(),
            revision: self.revision,
        }
    }
    pub fn from_session(state: SessionState) -> Result<Self, EditError> {
        // Validate every distinct root once. Recovery can never bypass invariants.
        let mut seen = BTreeSet::new();
        for project in std::iter::once(&state.current)
            .chain(state.saved.iter())
            .chain(state.past.iter().flat_map(|h| [&h.before, &h.after]))
            .chain(state.future.iter().flat_map(|h| [&h.before, &h.after]))
        {
            if project.id != state.current.id {
                return Err(EditError::Invalid(
                    "recovery roots belong to different projects".into(),
                ));
            }
            if seen.insert(Arc::as_ptr(project) as usize) {
                project.validate()?;
            }
        }
        let mut cursor = state.current.clone();
        for entry in state.past.iter().rev() {
            if cursor.as_ref() != entry.after.as_ref() {
                return Err(EditError::Invalid("recovery undo chain".into()));
            }
            cursor = entry.before.clone();
        }
        cursor = state.current.clone();
        for entry in state.future.iter().rev() {
            if cursor.as_ref() != entry.before.as_ref() {
                return Err(EditError::Invalid("recovery redo chain".into()));
            }
            cursor = entry.after.clone();
        }
        Ok(Self {
            instance_id: Uuid::new_v4(),
            current: state.current,
            saved: state.saved,
            past: state.past,
            future: state.future,
            revision: state.revision,
            submitted_saves: BTreeMap::new(),
        })
    }
    pub fn dispatch_at(
        &mut self,
        expected_revision: u64,
        command: Command,
    ) -> Result<EditOutcome, EditError> {
        if expected_revision != self.revision {
            return Err(EditError::StaleRevision {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        self.dispatch(command)
    }
    pub fn dispatch(&mut self, command: Command) -> Result<EditOutcome, EditError> {
        let prepared = self.prepare(command)?;
        self.commit_prepared(&prepared)
    }

    /// Validate a command without changing the document, revision or history.
    pub fn prepare(&self, command: Command) -> Result<PreparedEdit, EditError> {
        PreparedEdit::from_snapshot(
            self.current.clone(),
            self.instance_id,
            self.revision,
            command,
        )
    }

    /// Install the exact reviewed root as one reversible edit.
    ///
    /// A stale, reopened or replaced editor rejects the entire edit. A no-op
    /// retains the current root, saved boundary, revision and redo history.
    pub fn commit_prepared(&mut self, prepared: &PreparedEdit) -> Result<EditOutcome, EditError> {
        if prepared.context_id != self.instance_id || prepared.before.id != self.current.id {
            return Err(EditError::Invalid("prepared editing context".into()));
        }
        if prepared.revision != self.revision {
            return Err(EditError::StaleRevision {
                expected: prepared.revision,
                actual: self.revision,
            });
        }
        if !Arc::ptr_eq(&prepared.before, &self.current) {
            return Err(EditError::Invalid("prepared source snapshot".into()));
        }
        if !prepared.changed() {
            return Ok(EditOutcome {
                revision: self.revision,
                label: prepared.label.clone(),
                changed: false,
            });
        }
        self.past.push(HistoryEntry {
            label: prepared.label.clone(),
            before: prepared.before.clone(),
            after: prepared.after.clone(),
        });
        self.current = prepared.after.clone();
        self.future.clear();
        self.revision = self.revision.saturating_add(1);
        Ok(EditOutcome {
            revision: self.revision,
            label: prepared.label.clone(),
            changed: true,
        })
    }
    pub fn copy(&self, clip_ids: &[ClipId]) -> Result<Clipboard, EditError> {
        let selected = expand(self.project(), clip_ids, true)?;
        let mut clips = Vec::new();
        for (index, track) in self.project().tracks.iter().enumerate() {
            for clip in &track.clips {
                if selected.contains(&clip.id) {
                    clips.push(ClipboardClip {
                        track_index: index,
                        track_kind: track.kind,
                        clip: clip.clone(),
                    });
                }
            }
        }
        let origin = clips
            .iter()
            .map(|c| c.clip.start)
            .min()
            .unwrap_or(Time::ZERO);
        Ok(Clipboard { clips, origin })
    }
    pub fn paste(
        &mut self,
        clipboard: &Clipboard,
        at: Time,
        target_track: Option<TrackId>,
    ) -> Result<EditOutcome, EditError> {
        if clipboard.clips.is_empty() {
            return Err(EditError::Invalid("empty clipboard".into()));
        }
        let origin_index = clipboard
            .clips
            .iter()
            .map(|c| c.track_index)
            .min()
            .unwrap_or(0);
        let base = if let Some(id) = target_track {
            self.project()
                .tracks
                .iter()
                .position(|t| t.id == id)
                .ok_or(EditError::Missing(id))?
        } else {
            origin_index
        };
        let mut links = BTreeMap::new();
        let mut groups = BTreeMap::new();
        let mut commands = Vec::new();
        let mut tracks: BTreeMap<(usize, u8), TrackId> = BTreeMap::new();
        for item in &clipboard.clips {
            let index = base + item.track_index - origin_index;
            let key = (index, item.track_kind as u8);
            let track_id = if let Some(id) = tracks.get(&key) {
                *id
            } else if let Some(track) = self
                .project()
                .tracks
                .get(index)
                .filter(|t| t.kind == item.track_kind && !t.locked)
            {
                tracks.insert(key, track.id);
                track.id
            } else {
                if index == base && target_track.is_some() {
                    return Err(EditError::IncompatibleTrack);
                }
                let track = Track::new("Pasted track", item.track_kind);
                let id = track.id;
                commands.push(Command::AddTrack { track });
                tracks.insert(key, id);
                id
            };
            let mut clip = item.clip.clone();
            clip.id = Uuid::new_v4();
            clip.start = at + (clip.start - clipboard.origin);
            clip.linked_id = clip
                .linked_id
                .map(|id| *links.entry(id).or_insert_with(Uuid::new_v4));
            clip.group_id = clip
                .group_id
                .map(|id| *groups.entry(id).or_insert_with(Uuid::new_v4));
            renew_look_ids(&mut clip);
            if clip
                .follow
                .as_ref()
                .is_some_and(|f| !self.project().trackers.contains_key(&f.tracker_id))
            {
                clip.follow = None;
            }
            commands.push(Command::AddClip { track_id, clip });
        }
        self.dispatch(Command::Batch {
            label: "Paste clips".into(),
            commands,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Clipboard {
    pub clips: Vec<ClipboardClip>,
    pub origin: Time,
}
#[derive(Debug, Clone)]
pub struct ClipboardClip {
    pub track_index: usize,
    pub track_kind: TrackKind,
    pub clip: Clip,
}

fn track_mut(
    project: &mut Project,
    id: TrackId,
    require_unlocked: bool,
) -> Result<&mut Track, EditError> {
    let track = project
        .tracks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or(EditError::Missing(id))?;
    if require_unlocked && track.locked {
        return Err(EditError::Locked);
    }
    Ok(Arc::make_mut(track))
}
fn clip_mut(project: &mut Project, id: ClipId) -> Result<&mut Clip, EditError> {
    let track_id = project.clip(id).ok_or(EditError::Missing(id))?.0.id;
    track_mut(project, track_id, true)?
        .clips
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or(EditError::Missing(id))
}
fn expand(project: &Project, ids: &[ClipId], groups: bool) -> Result<BTreeSet<ClipId>, EditError> {
    let mut selected: BTreeSet<_> = ids.iter().copied().collect();
    for id in ids {
        project.clip(*id).ok_or(EditError::Missing(*id))?;
    }
    // Transitive closure handles overlapping link/group relationships.
    loop {
        let mut links = BTreeSet::new();
        let mut group_ids = BTreeSet::new();
        for id in &selected {
            if let Some((_, c)) = project.clip(*id) {
                if let Some(l) = c.linked_id {
                    links.insert(l);
                }
                if groups && let Some(g) = c.group_id {
                    group_ids.insert(g);
                }
            }
        }
        let before = selected.len();
        for clip in project.tracks.iter().flat_map(|t| &t.clips) {
            if clip.linked_id.is_some_and(|l| links.contains(&l))
                || (groups && clip.group_id.is_some_and(|g| group_ids.contains(&g)))
            {
                selected.insert(clip.id);
            }
        }
        if selected.len() == before {
            return Ok(selected);
        }
    }
}
// Fresh effect and mask IDs, with the clip's animation targets following them.
fn renew_look_ids(clip: &mut Clip) {
    let mut fresh = BTreeMap::new();
    let ids = clip
        .effects
        .iter_mut()
        .map(|e| &mut e.id)
        .chain(clip.masks.iter_mut().map(|m| &mut m.id));
    for id in ids {
        let new = Uuid::new_v4();
        fresh.insert(*id, new);
        *id = new;
    }
    for track in &mut clip.animation {
        if let Some(new) = track.target.and_then(|id| fresh.get(&id)) {
            track.target = Some(*new);
        }
    }
}
fn prune_animation(clip: &mut Clip) {
    let (effects, masks) = (&clip.effects, &clip.masks);
    clip.animation.retain(|track| {
        track
            .target
            .is_none_or(|id| effects.iter().any(|e| e.id == id) || masks.iter().any(|m| m.id == id))
    });
}
// Inclusive range of the parameter an animation track drives.
fn anim_range(clip: &Clip, track: &AnimTrack) -> (f32, f32) {
    let target = match track.target {
        None => Some(AnimTarget::Clip),
        Some(id) => clip
            .effects
            .iter()
            .find(|e| e.id == id)
            .map(|e| AnimTarget::Effect(e.kind))
            .or_else(|| {
                clip.masks
                    .iter()
                    .any(|m| m.id == id)
                    .then_some(AnimTarget::Mask)
            }),
    };
    target
        .and_then(|target| look::param_spec(target, track.key))
        .map_or((f32::MIN, f32::MAX), |spec| (spec.min, spec.max))
}
// Keeps `from..to` of the animation and time map, measured from the clip's
// start, and rebases them to `from`. The caller sets `start` and `duration`.
fn crop_local(clip: &mut Clip, from: Time, to: Time) -> Result<(), EditError> {
    if from == Time::ZERO && to == clip.duration {
        return Ok(());
    }
    let ranges: Vec<_> = clip
        .animation
        .iter()
        .map(|track| anim_range(clip, track))
        .collect();
    for (track, range) in clip.animation.iter_mut().zip(ranges) {
        if !track.crop(from, to, range) {
            return Err(EditError::Invalid("animation keys".into()));
        }
    }
    if let Some(retime) = &clip.retime {
        let (map, shift) = retime
            .crop(from, to)
            .ok_or_else(|| EditError::Invalid("time map".into()))?;
        clip.source_in = clip.source_in + shift;
        clip.retime = Some(map);
    }
    // A title keeps every word: one cut off is held at the edge it crossed.
    if let ClipContent::Text { words, .. } = &mut clip.content {
        let length = to - from;
        for word in words {
            word.at = (word.at - from).clamp(Time::ZERO, length);
            word.end = (word.end - from).clamp(word.at, length);
        }
    }
    Ok(())
}
// A transition never outlasts its clip; one shorter than a frame is dropped.
// A title's or a graphic's animations are held inside the clip too, at a
// frame or more, and its word times inside the clip.
fn fit_transitions(clip: &mut Clip, frame: Time) {
    let duration = clip.duration;
    for slot in [&mut clip.transition_in, &mut clip.transition_out] {
        if let Some(transition) = slot {
            transition.duration = transition.duration.min(duration);
            if transition.duration < frame || transition.duration <= Time::ZERO {
                *slot = None;
            }
        }
    }
    match &mut clip.content {
        ClipContent::Text {
            enter, exit, words, ..
        } => {
            (*enter, *exit) = title::fit_animations(*enter, *exit, frame, duration);
            for word in words {
                word.at = word.at.clamp(Time::ZERO, duration);
                word.end = word.end.clamp(word.at, duration);
            }
        }
        ClipContent::Graphic { elements, .. } => {
            for element in elements {
                (element.enter, element.exit) =
                    title::fit_animations(element.enter, element.exit, frame, duration);
            }
        }
        ClipContent::Media { .. } => {}
    }
}
// Whether a title's or graphic's animation is shorter than `frame`.
fn short_animation(clip: &Clip, frame: Time) -> bool {
    let short = |animation: &Option<TextAnimation>| {
        animation.is_some_and(|animation| animation.duration < frame)
    };
    match &clip.content {
        ClipContent::Text { enter, exit, .. } => short(enter) || short(exit),
        ClipContent::Graphic { elements, .. } => elements
            .iter()
            .any(|element| short(&element.enter) || short(&element.exit)),
        ClipContent::Media { .. } => false,
    }
}
// A fade never outlasts its clip: each is cut to the clip, then the fade out
// to what the fade in leaves.
fn fit_fades(clip: &mut Clip) {
    clip.fade_in = clip.fade_in.min(clip.duration);
    clip.fade_out = clip.fade_out.min(clip.duration - clip.fade_in);
}
// A transition lasts at least one frame of the canvas it is now played on.
fn lengthen_transitions(clip: &mut Clip, frame: Time) {
    for transition in [&mut clip.transition_in, &mut clip.transition_out]
        .into_iter()
        .flatten()
    {
        transition.duration = transition.duration.max(frame);
    }
    fit_transitions(clip, frame);
}
// Stretches animation key times after the clip's duration changed from `old`,
// and a title's word times and text animations with them.
fn rescale_local(clip: &mut Clip, old: Time, frame: Time) {
    let duration = clip.duration;
    for track in &mut clip.animation {
        track.rescale(old, duration);
    }
    if old > Time::ZERO && old != duration {
        let factor = duration.0 as f64 / old.0 as f64;
        let stretch = |time: Time| Time(scale(time.0, duration.0, old.0));
        match &mut clip.content {
            ClipContent::Text {
                enter, exit, words, ..
            } => {
                for word in words {
                    (word.at, word.end) = (stretch(word.at), stretch(word.end));
                }
                for animation in [enter, exit].into_iter().flatten() {
                    *animation = animation.scaled(factor, frame);
                }
            }
            ClipContent::Graphic { elements, .. } => {
                for element in elements {
                    for animation in [&mut element.enter, &mut element.exit]
                        .into_iter()
                        .flatten()
                    {
                        *animation = animation.scaled(factor, frame);
                    }
                }
            }
            ClipContent::Media { .. } => {}
        }
    }
    fit_transitions(clip, frame);
}
// The segment an easing shapes, played from its other end.
fn mirrored(ease: Ease) -> Ease {
    match ease {
        Ease::Bezier { x1, y1, x2, y2 } => Ease::Bezier {
            x1: 1.0 - x2,
            y1: (1.0 - y2).clamp(-4.0, 4.0),
            x2: 1.0 - x1,
            y2: (1.0 - y1).clamp(-4.0, 4.0),
        },
        ease => ease,
    }
}
// Moves animation keys after the clip's time map changed from `old` to `new`,
// so each key stays on the source moment it was set on.
//
// - A key goes to the time the new map shows its source moment. Shown more
//   than once, it takes the showing nearest its old time.
// - Where the new map holds that moment, a key that was already in a hold
//   keeps its distance into the hold (stopping at the hold's end); any other
//   key takes the hold's nearest instant, so a key on the frame a freeze
//   holds stays at the freeze's start and later keys move by its length.
// - A moment the new map never shows goes to the map key nearest it in source.
// - Keys that land on one time leave the one nearest its own moment, then
//   the later one.
// - Where a stretch now plays backwards its keys change order: each segment
//   keeps its easing, mirrored.
fn follow_source(clip: &mut Clip, old: &Retime, new: &Retime) {
    for track in &mut clip.animation {
        let mut placed: Vec<(Time, u64, usize)> = track
            .keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let source = old.source_at(key.at);
                let held = old.held_for(key.at);
                let near = |time: Time| time.0.abs_diff(key.at.0);
                let shown = new
                    .shows(source)
                    .into_iter()
                    .map(|(from, to)| match held {
                        Some(held) if from < to => (from + held).min(to),
                        _ => key.at.clamp(from, to),
                    })
                    .min_by_key(|time| near(*time));
                match shown {
                    Some(time) => (time, 0, index),
                    None => {
                        let (time, miss) = new
                            .keys
                            .iter()
                            .map(|k| (k.at, k.source.0.abs_diff(source.0)))
                            .min_by_key(|(time, miss)| (*miss, near(*time)))
                            .unwrap_or((key.at, 0));
                        (time, miss, index)
                    }
                }
            })
            .collect();
        placed.sort_by_key(|(time, miss, index)| (*time, *miss, std::cmp::Reverse(*index)));
        placed.dedup_by_key(|(time, ..)| *time);
        let keys: Vec<Keyframe> = placed
            .iter()
            .enumerate()
            .map(|(place, (at, _, index))| {
                let key = track.keys[*index];
                let ease = match placed.get(place + 1) {
                    Some((_, _, next)) if next < index => mirrored(track.keys[*next].ease),
                    _ => key.ease,
                };
                Keyframe {
                    at: *at,
                    ease,
                    ..key
                }
            })
            .collect();
        track.keys = keys;
    }
}
fn shift_after(
    project: &mut Project,
    edge: Time,
    delta: Time,
    exclude: &BTreeSet<ClipId>,
) -> Result<(), EditError> {
    for track in &mut project.tracks {
        if track
            .clips
            .iter()
            .any(|c| c.start >= edge && !exclude.contains(&c.id))
        {
            if track.locked && delta != Time::ZERO {
                return Err(EditError::Locked);
            }
            if delta != Time::ZERO {
                for clip in &mut Arc::make_mut(track).clips {
                    if clip.start >= edge && !exclude.contains(&clip.id) {
                        clip.start = clip.start + delta;
                    }
                }
            }
        }
    }
    Ok(())
}

fn apply(project: &mut Project, command: Command, depth: usize) -> Result<(), EditError> {
    if depth > 32 {
        return Err(EditError::Invalid("transaction nesting".into()));
    }
    let frame = project.settings.frame_rate.frame_time(1);
    match command {
        Command::Batch { commands, .. } => {
            if commands.len() > 100_000 {
                return Err(EditError::Invalid("transaction size".into()));
            }
            for command in commands {
                apply(project, command, depth + 1)?;
            }
        }
        Command::AddAssets { assets } => {
            let map = Arc::make_mut(&mut project.assets);
            for asset in assets {
                if map.contains_key(&asset.id) {
                    return Err(EditError::Invalid("duplicate asset ID".into()));
                }
                map.insert(asset.id, asset);
            }
        }
        Command::RemoveAssets { asset_ids } => {
            if project.tracks.iter().flat_map(|t| &t.clips).any(|c| {
                c.asset_id()
                    .into_iter()
                    .chain(c.effects.iter().filter_map(|e| e.asset_id))
                    .any(|id| asset_ids.contains(&id))
            }) {
                return Err(EditError::Invalid(
                    "media is still used on the timeline".into(),
                ));
            }
            if project
                .trackers
                .values()
                .any(|t| asset_ids.contains(&t.asset_id))
            {
                return Err(EditError::Invalid(
                    "media is still used by a tracker".into(),
                ));
            }
            for id in asset_ids {
                Arc::make_mut(&mut project.assets)
                    .remove(&id)
                    .ok_or(EditError::Missing(id))?;
            }
        }
        Command::AddFolder { folder } => {
            if Arc::make_mut(&mut project.folders)
                .insert(folder.id, folder)
                .is_some()
            {
                return Err(EditError::Invalid("duplicate folder ID".into()));
            }
        }
        Command::MoveAssetToFolder {
            asset_id,
            folder_id,
        } => {
            Arc::make_mut(&mut project.assets)
                .get_mut(&asset_id)
                .ok_or(EditError::Missing(asset_id))?
                .folder_id = folder_id;
        }
        Command::AddTrack { track } => project.tracks.push(Arc::new(track)),
        Command::RemoveTrack { track_id } => {
            let track = project
                .track(track_id)
                .ok_or(EditError::Missing(track_id))?;
            if track.locked {
                return Err(EditError::Locked);
            }
            project.tracks.retain(|t| t.id != track_id);
        }
        Command::ReorderTrack { track_id, index } => {
            let old = project
                .tracks
                .iter()
                .position(|t| t.id == track_id)
                .ok_or(EditError::Missing(track_id))?;
            if index >= project.tracks.len() {
                return Err(EditError::Invalid("track index".into()));
            }
            if project.tracks[old].locked {
                return Err(EditError::Locked);
            }
            let track = project.tracks.remove(old);
            project.tracks.insert(index, track);
        }
        Command::SetTrackState {
            track_id,
            locked,
            hidden,
            muted,
            name,
            gain,
            solo,
        } => {
            let track = track_mut(project, track_id, false)?;
            if let Some(v) = locked {
                track.locked = v;
            }
            if let Some(v) = hidden {
                track.hidden = v;
            }
            if let Some(v) = muted {
                track.muted = v;
            }
            if let Some(v) = name {
                track.name = v;
            }
            if let Some(v) = gain {
                track.gain = v;
            }
            if let Some(v) = solo {
                track.solo = v;
            }
        }
        Command::AddClip { track_id, clip } => track_mut(project, track_id, true)?.clips.push(clip),
        Command::DeleteClips { clip_ids, ripple } => {
            let selected = expand(project, &clip_ids, true)?;
            let mut intervals: Vec<_> = selected
                .iter()
                .filter_map(|id| project.clip(*id))
                .map(|(_, c)| (c.start, c.end()))
                .collect();
            intervals.sort_unstable();
            let mut merged: Vec<(Time, Time)> = Vec::new();
            for (start, end) in intervals {
                if let Some(last) = merged.last_mut().filter(|last| last.1 >= start) {
                    last.1 = last.1.max(end);
                } else {
                    merged.push((start, end));
                }
            }
            for track in &mut project.tracks {
                if track.clips.iter().any(|c| selected.contains(&c.id)) {
                    if track.locked {
                        return Err(EditError::Locked);
                    }
                    Arc::make_mut(track)
                        .clips
                        .retain(|c| !selected.contains(&c.id));
                }
            }
            if ripple {
                for track in &mut project.tracks {
                    let shifts: Vec<_> = track
                        .clips
                        .iter()
                        .map(|c| {
                            merged
                                .iter()
                                .map(|(a, b)| Time((c.start.0 - a.0).clamp(0, b.0 - a.0)))
                                .fold(Time::ZERO, |a, b| a + b)
                        })
                        .collect();
                    if shifts.iter().any(|s| *s != Time::ZERO) {
                        if track.locked {
                            return Err(EditError::Locked);
                        }
                        for (clip, shift) in Arc::make_mut(track).clips.iter_mut().zip(shifts) {
                            clip.start = clip.start - shift;
                        }
                    }
                }
                for marker in Arc::make_mut(&mut project.markers) {
                    let shift = merged
                        .iter()
                        .map(|(a, b)| Time((marker.time.0 - a.0).clamp(0, b.0 - a.0)))
                        .fold(Time::ZERO, |a, b| a + b);
                    marker.time = marker.time - shift;
                }
            }
        }
        Command::MoveClips {
            clip_ids,
            delta,
            target_track,
        } => {
            let selected = expand(project, &clip_ids, true)?;
            for id in &selected {
                let clip = clip_mut(project, *id)?;
                clip.start = clip.start + delta;
            }
            if let Some(target) = target_track {
                track_mut(project, target, true)?;
                let mut moved = Vec::new();
                for track in &mut project.tracks {
                    if track.id != target && track.clips.iter().any(|c| clip_ids.contains(&c.id)) {
                        let clips = &mut Arc::make_mut(track).clips;
                        let mut i = 0;
                        while i < clips.len() {
                            if clip_ids.contains(&clips[i].id) {
                                moved.push(clips.remove(i));
                            } else {
                                i += 1;
                            }
                        }
                    }
                }
                track_mut(project, target, true)?.clips.extend(moved);
            }
        }
        Command::TrimClip {
            clip_id,
            source_in,
            duration,
            ripple,
        } => {
            let original = project
                .clip(clip_id)
                .ok_or(EditError::Missing(clip_id))?
                .1
                .clone();
            let selected = expand(project, &[clip_id], false)?;
            // The start edge moves by this much timeline time on every linked clip.
            let head = (source_in - original.source_in).scaled(1.0 / original.playback_rate);
            for id in &selected {
                // A title or a still has no source before zero to run out of.
                let unbound = project.clip(*id).is_some_and(|(_, clip)| {
                    clip.asset_id().is_none_or(|asset| {
                        project
                            .assets
                            .get(&asset)
                            .is_some_and(|asset| asset.kind == MediaKind::Image)
                    })
                });
                let clip = clip_mut(project, *id)?;
                let length = clip.duration + (duration - original.duration);
                // A time map is cropped instead, which also moves `source_in`.
                if clip.retime.is_none() {
                    let source_delta = (source_in - original.source_in)
                        .scaled(clip.playback_rate / original.playback_rate);
                    clip.source_in = clip.source_in + source_delta;
                    if unbound {
                        clip.source_in = clip.source_in.max(Time::ZERO);
                    }
                }
                crop_local(clip, head, head + length)?;
                if !ripple {
                    clip.start = clip.start + head;
                }
                clip.duration = length;
                fit_transitions(clip, frame);
                fit_fades(clip);
            }
            if ripple {
                let new_end = project
                    .clip(clip_id)
                    .ok_or(EditError::Missing(clip_id))?
                    .1
                    .end();
                shift_after(project, original.end(), new_end - original.end(), &selected)?;
                for marker in Arc::make_mut(&mut project.markers) {
                    if marker.time >= original.end() {
                        marker.time = marker.time + (new_end - original.end());
                    }
                }
            }
        }
        Command::SplitClip { clip_id, at } => {
            let selected = expand(project, &[clip_id], false)?;
            let new_link = Uuid::new_v4();
            for id in &selected {
                let (track, clip) = project.clip(*id).ok_or(EditError::Missing(*id))?;
                if at <= clip.start || at >= clip.end() {
                    return Err(EditError::Invalid(
                        "split must be inside all linked clips".into(),
                    ));
                }
                let track_id = track.id;
                let (local, total) = (at - clip.start, clip.duration);
                let mut right = clip.clone();
                right.id = Uuid::new_v4();
                if right.retime.is_none() {
                    right.source_in = clip.source_time(at);
                }
                // A timed title gives the words said before the cut, and
                // their text, to the left and the rest to the right.
                let parted = parted_words(&clip.content, local);
                if let Some((_, half)) = &parted {
                    half.give(&mut right);
                }
                split_animations(&mut right.content, false);
                crop_local(&mut right, local, total)?;
                right.start = at;
                right.duration = clip.end() - at;
                right.transition_in = None;
                fit_transitions(&mut right, frame);
                // The cut is no edge to fade at: each half keeps its outer fade.
                right.fade_in = Time::ZERO;
                fit_fades(&mut right);
                renew_look_ids(&mut right);
                if selected.len() > 1 {
                    right.linked_id = Some(new_link);
                }
                let left = clip_mut(project, *id)?;
                if let Some((half, _)) = &parted {
                    half.give(left);
                }
                split_animations(&mut left.content, true);
                crop_local(left, Time::ZERO, local)?;
                left.duration = local;
                left.transition_out = None;
                fit_transitions(left, frame);
                left.fade_out = Time::ZERO;
                fit_fades(left);
                track_mut(project, track_id, true)?.clips.push(right);
            }
        }
        Command::SetTransform { clip_id, transform } => {
            clip_mut(project, clip_id)?.transform = transform
        }
        Command::SetPlaybackRate { clip_id, rate } => {
            if !rate.is_finite() || !(0.05..=16.0).contains(&rate) {
                return Err(EditError::Invalid("clip speed".into()));
            }
            let original_rate = project
                .clip(clip_id)
                .ok_or(EditError::Missing(clip_id))?
                .1
                .playback_rate;
            let factor = rate / original_rate;
            let selected = expand(project, &[clip_id], false)?;
            for id in selected {
                let clip = clip_mut(project, id)?;
                let old = clip.duration;
                clip.duration = clip.duration.scaled(1.0 / factor);
                rescale_local(clip, old, frame);
                // A fade is a timeline length: it is kept, not stretched.
                fit_fades(clip);
                // A time map keeps rate 1.0 and plays the same span over the new duration.
                match &mut clip.retime {
                    Some(retime) => retime.rescale(old, clip.duration),
                    None => clip.playback_rate *= factor,
                }
            }
        }
        Command::SetGain { clip_id, gain } => clip_mut(project, clip_id)?.gain = gain,
        Command::SetFades {
            clip_id,
            fade_in,
            fade_out,
        } => {
            let clip = clip_mut(project, clip_id)?;
            clip.fade_in = fade_in;
            clip.fade_out = fade_out;
        }
        Command::SetText {
            clip_id,
            text,
            font_size,
            color,
        } => {
            let clip = clip_mut(project, clip_id)?;
            let duration = clip.duration;
            let ClipContent::Text {
                text: stored,
                font_size: stored_size,
                color: stored_color,
                words,
                ..
            } = &mut clip.content
            else {
                return Err(EditError::Invalid("clip is not text".into()));
            };
            *words = title::realign(stored, words, &text, duration);
            *stored = text.clone();
            *stored_size = font_size;
            *stored_color = color;
            clip.name = text;
        }
        Command::SetTextStyle { clip_id, style } => {
            let clip = clip_mut(project, clip_id)?;
            match &mut clip.content {
                ClipContent::Text { style: stored, .. } => *stored = style,
                _ => return Err(EditError::Invalid("clip is not text".into())),
            }
        }
        Command::SetTextAnimation {
            clip_id,
            edge,
            animation,
        } => {
            let clip = clip_mut(project, clip_id)?;
            let ClipContent::Text { enter, exit, .. } = &mut clip.content else {
                return Err(EditError::Invalid("clip is not text".into()));
            };
            match edge {
                TransitionEdge::In => *enter = animation,
                TransitionEdge::Out => *exit = animation,
            }
        }
        Command::SetWords { clip_id, words } => {
            let clip = clip_mut(project, clip_id)?;
            match &mut clip.content {
                ClipContent::Text { words: stored, .. } => *stored = words,
                _ => return Err(EditError::Invalid("clip is not text".into())),
            }
        }
        Command::SetGraphic {
            clip_id,
            size,
            elements,
        } => {
            let clip = clip_mut(project, clip_id)?;
            if !matches!(clip.content, ClipContent::Graphic { .. }) {
                return Err(EditError::Invalid("clip is not a graphic".into()));
            }
            clip.content = ClipContent::Graphic { size, elements };
        }
        Command::SetTrackLanguage { track_id, language } => {
            track_mut(project, track_id, false)?.language = language;
        }
        Command::CopyCaptions {
            from_track_id,
            track_id,
            name,
            language,
            texts,
        } => {
            let index = project
                .tracks
                .iter()
                .position(|track| track.id == from_track_id)
                .ok_or(EditError::Missing(from_track_id))?;
            let source = &project.tracks[index];
            if source.kind != TrackKind::Text || texts.len() != source.clips.len() {
                return Err(EditError::Invalid(
                    "captions need one text for each title of the track".into(),
                ));
            }
            let mut track = Track::new(name, TrackKind::Text);
            track.id = track_id;
            track.language = Some(language);
            for (clip, text) in source.clips.iter().zip(texts) {
                let ClipContent::Text { words, .. } = &clip.content else {
                    return Err(EditError::Invalid(
                        "captions need a track of titles only".into(),
                    ));
                };
                let timed = (words.first().zip(words.last()))
                    .map(|(first, last)| title::spread(&text, first.at, last.end));
                let mut copy = clip.clone();
                copy.id = Uuid::new_v4();
                copy.linked_id = None;
                copy.group_id = None;
                TitleHalf {
                    text,
                    words: timed.unwrap_or_default(),
                }
                .give(&mut copy);
                track.clips.push(copy);
            }
            // One language shows at a time: the copy takes the source's place.
            Arc::make_mut(&mut project.tracks[index]).hidden = true;
            project.tracks.insert(index + 1, Arc::new(track));
        }
        Command::LinkClips { clip_ids } => {
            set_relationship(project, &clip_ids, true, Some(Uuid::new_v4()))?
        }
        Command::UnlinkClips { clip_ids } => set_relationship(project, &clip_ids, true, None)?,
        Command::GroupClips { clip_ids } => {
            set_relationship(project, &clip_ids, false, Some(Uuid::new_v4()))?
        }
        Command::UngroupClips { clip_ids } => set_relationship(project, &clip_ids, false, None)?,
        Command::AddMarker { marker } => {
            let markers = Arc::make_mut(&mut project.markers);
            markers.push(marker);
            markers.sort_by_key(|m| m.time);
        }
        Command::RemoveMarker { marker_id } => {
            let markers = Arc::make_mut(&mut project.markers);
            let index = markers
                .iter()
                .position(|m| m.id == marker_id)
                .ok_or(EditError::Missing(marker_id))?;
            markers.remove(index);
        }
        Command::SetEffects { clip_id, effects } => {
            let clip = clip_mut(project, clip_id)?;
            clip.effects = effects;
            prune_animation(clip);
        }
        Command::SetMasks { clip_id, masks } => {
            let clip = clip_mut(project, clip_id)?;
            clip.masks = masks;
            prune_animation(clip);
        }
        Command::SetBlendMode { clip_id, mode } => clip_mut(project, clip_id)?.blend = mode,
        Command::SetKeyframes {
            clip_id,
            target,
            key,
            keys,
        } => {
            let animation = &mut clip_mut(project, clip_id)?.animation;
            let index = animation
                .iter()
                .position(|t| t.target == target && t.key == key);
            match index {
                Some(index) if keys.is_empty() => {
                    animation.remove(index);
                }
                Some(index) => animation[index].keys = keys,
                None if keys.is_empty() => {}
                None => animation.push(AnimTrack { target, key, keys }),
            }
        }
        Command::SetRetime {
            clip_id,
            retime,
            duration,
        } => {
            let named = project
                .clip(clip_id)
                .ok_or(EditError::Missing(clip_id))?
                .1
                .duration;
            let selected = expand(project, &[clip_id], false)?;
            for id in selected {
                let clip = clip_mut(project, id)?;
                let old = clip.duration;
                // A linked clip of another length keeps its proportion, as a
                // rate change would leave it.
                let fit = |time: Time| match old == named {
                    true => time,
                    false => Time(scale(time.0, old.0, named.0)),
                };
                // What the clip played before, as a map.
                let before = clip
                    .retime
                    .clone()
                    .unwrap_or_else(|| Retime::constant(old, clip.playback_rate));
                clip.duration = fit(duration);
                // A linked title cannot carry a map; it only changes length.
                let plain = id != clip_id && clip.content.is_generated();
                clip.retime = retime.clone().filter(|_| !plain);
                if let Some(map) = &mut clip.retime {
                    // The map arrives in the clip's own rate; bake it into the sources.
                    for key in &mut map.keys {
                        key.at = fit(key.at);
                        key.source = fit(key.source).scaled(clip.playback_rate);
                    }
                    // Rounding must not leave the scaled map short of the clip.
                    if let Some(last) = map.keys.last_mut().filter(|_| old != named) {
                        last.at = clip.duration;
                    }
                    clip.playback_rate = 1.0;
                }
                if plain {
                    // A title has no source to stay on: its keys stretch.
                    rescale_local(clip, old, frame);
                } else {
                    let after = clip
                        .retime
                        .clone()
                        .unwrap_or_else(|| Retime::constant(clip.duration, clip.playback_rate));
                    follow_source(clip, &before, &after);
                    fit_transitions(clip, frame);
                }
                fit_fades(clip);
            }
        }
        Command::SetTransition {
            clip_id,
            edge,
            transition,
        } => {
            let clip = clip_mut(project, clip_id)?;
            match edge {
                TransitionEdge::In => clip.transition_in = transition,
                TransitionEdge::Out => clip.transition_out = transition,
            }
        }
        Command::ApplyPreset {
            clip_ids,
            preset,
            strength,
        } => {
            if !(0.0..=1.0).contains(&strength) {
                return Err(EditError::Invalid("look strength".into()));
            }
            for id in clip_ids.into_iter().collect::<BTreeSet<_>>() {
                let clip = clip_mut(project, id)?;
                if clip.content.is_generated() {
                    return Err(EditError::Invalid("text clip look".into()));
                }
                // One look to a clip: it takes the place of the one held.
                clip.effects = look::with_look(&clip.effects, Some((preset, strength)));
                prune_animation(clip);
            }
        }
        Command::SetTracker { tracker } => {
            Arc::make_mut(&mut project.trackers).insert(tracker.id, Arc::new(tracker));
        }
        Command::RemoveTracker { tracker_id } => {
            Arc::make_mut(&mut project.trackers)
                .remove(&tracker_id)
                .ok_or(EditError::Missing(tracker_id))?;
            let follows = |c: &Clip| {
                c.follow
                    .as_ref()
                    .is_some_and(|f| f.tracker_id == tracker_id)
            };
            for track in &mut project.tracks {
                if track.clips.iter().any(follows) {
                    if track.locked {
                        return Err(EditError::Locked);
                    }
                    for clip in &mut Arc::make_mut(track).clips {
                        if follows(clip) {
                            clip.follow = None;
                        }
                    }
                }
            }
        }
        Command::SetFollow { clip_id, follow } => clip_mut(project, clip_id)?.follow = follow,
        Command::RenameProject { name } => project.name = name,
        Command::SetSettings { settings } => {
            let valid = settings.validate();
            project.settings = settings;
            let frame = project.settings.frame_rate.frame_time(1);
            let short = |clip: &Clip| {
                [&clip.transition_in, &clip.transition_out]
                    .into_iter()
                    .flatten()
                    .any(|transition| transition.duration < frame)
                    || short_animation(clip, frame)
            };
            // A lower frame rate must not strand a transition under one frame.
            for track in project.tracks.iter_mut().filter(|_| valid) {
                if track.clips.iter().any(short) {
                    for clip in &mut Arc::make_mut(track).clips {
                        lengthen_transitions(clip, frame);
                    }
                }
            }
        }
        Command::RestoreProject { project: restored } => {
            if restored.id != project.id {
                return Err(EditError::Invalid(
                    "checkpoint belongs to another project".into(),
                ));
            }
            *project = *restored;
        }
    }
    Ok(())
}
// A title's text and word times, given to a clip of a split or a copy.
struct TitleHalf {
    text: String,
    words: Vec<WordTime>,
}
impl TitleHalf {
    fn give(&self, clip: &mut Clip) {
        if let ClipContent::Text { text, words, .. } = &mut clip.content {
            text.clone_from(&self.text);
            words.clone_from(&self.words);
            clip.name.clone_from(&self.text);
        }
    }
}
// A timed title parted at clip time `local`: the words said before it, with
// their text, and the rest. `None` for anything else, or when every word
// falls to one side, so both halves keep all of it.
fn parted_words(content: &ClipContent, local: Time) -> Option<(TitleHalf, TitleHalf)> {
    let ClipContent::Text { text, words, .. } = content else {
        return None;
    };
    let before = words.partition_point(|word| word.at < local);
    if before == 0 || before == words.len() {
        return None;
    }
    let cut = title::word_ranges(text).get(before)?.start;
    Some((
        TitleHalf {
            text: text[..cut].trim_end().to_owned(),
            words: words[..before].to_vec(),
        },
        TitleHalf {
            text: text[cut..].trim_start().to_owned(),
            words: words[before..].to_vec(),
        },
    ))
}
// What each half of a split keeps of a title's or graphic's animations: the
// left half its entrances, the right half its exits.
fn split_animations(content: &mut ClipContent, left: bool) {
    match content {
        ClipContent::Text { enter, exit, .. } => {
            if left {
                *exit = None;
            } else {
                *enter = None;
            }
        }
        ClipContent::Graphic { elements, .. } => {
            for element in elements {
                if left {
                    element.exit = None;
                } else {
                    element.enter = None;
                }
            }
        }
        ClipContent::Media { .. } => {}
    }
}
fn set_relationship(
    project: &mut Project,
    ids: &[ClipId],
    linked: bool,
    value: Option<Uuid>,
) -> Result<(), EditError> {
    let mut selected: BTreeSet<_> = ids.iter().copied().collect();
    if value.is_some() && selected.len() < 2 {
        return Err(EditError::Invalid(
            "select at least two distinct clips".into(),
        ));
    }
    let previous: BTreeSet<_> = ids
        .iter()
        .map(|id| {
            project
                .clip(*id)
                .map(|(_, c)| if linked { c.linked_id } else { c.group_id })
                .ok_or(EditError::Missing(*id))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    for clip in project.tracks.iter().flat_map(|t| &t.clips) {
        if (if linked {
            clip.linked_id
        } else {
            clip.group_id
        })
        .is_some_and(|id| previous.contains(&id))
        {
            selected.insert(clip.id);
        }
    }
    for id in selected {
        let clip = clip_mut(project, id)?;
        if linked {
            clip.linked_id = value;
        } else {
            clip.group_id = value;
        }
    }
    Ok(())
}
