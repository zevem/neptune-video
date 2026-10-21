//! Keyframes in the timeline: a lane of diamonds under a clip's track, and a
//! graph of value over time that takes the tracks' place.
//!
//! Both share the timeline's time axis. Each key in view is a widget with a
//! name, so it is reached by the keyboard as by the pointer. A held drag is a
//! draft drawn here and on the canvas; it becomes one command when it is let
//! go and nothing when it is given up.
//!
//! A clip with sound and no keyframe yet has a lane too: one empty row for
//! its volume, which the clip menu and the volume line on its tile key.
use super::{
    Action, EASINGS, Selection, controls, fades, inspector, kit,
    timeline::{Geometry, TimelineInput},
    widgets::{self, menu_item, menu_style},
};
use crate::{
    icons::{self, Icon},
    rules,
    theme::{self, Palette},
    timefmt,
};
use editor_core::{
    AnimTarget, AnimTrack, Clip, ClipId, ClipOverride, Command, Ease, FrameRate, Keyframe,
    ParamKey, Project, Time, Track, gain_at, look, sound,
};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Response, Sense, Shape,
    Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::sync::Arc;
use uuid::Uuid;

/// The height of one parameter's row in the lane. A lane is as many rows as
/// the clip has animated parameters, and scrolls with the tracks.
pub const ROW: f32 = 18.0;
const PAD: f32 = 3.0;
/// Half the width of a key's diamond in the lane: 9 points corner to corner,
/// and 11 under the pointer.
const DIAMOND: f32 = 4.5;
const DIAMOND_HOVER: f32 = 5.5;
/// Half the width of a key's diamond in the graph, where the curve is the
/// subject: 7 points corner to corner, and 9 under the pointer.
const POINT: f32 = 3.5;
const POINT_HOVER: f32 = 4.5;
const HANDLE: f32 = 3.5;
/// The height of a row of the graph's legend: its 22-point toggles with a
/// little room between them, so a column of pressed ones stays apart.
const LEGEND_ROW: f32 = 24.0;
/// A legend row's toggles, as a track head's cells: 22 square, 23 apart.
const LEGEND_CELL: f32 = 22.0;
const LEGEND_STEP: f32 = 23.0;
/// Where a head card's glyph stands, and its name starts, from its left.
const HEAD_GLYPH: f32 = 16.0;
const HEAD_NAME: f32 = 28.0;
const SNAP_POINTS: f32 = 8.0;
/// How far the pointer travels before a press is a drag.
const TRAVEL: f32 = 3.0;
/// The graph draws its value labels from this height up; under it only the
/// curves and their keys fit.
pub const GRAPH_LABELLED: f32 = 64.0;
/// Points between samples of a curve.
pub(super) const SAMPLE: f32 = 2.0;
/// A step of an easing handle moved with the arrows.
const HANDLE_STEP: f32 = 0.05;

/// One animated parameter of a clip: the effect or mask it belongs to, none
/// for the clip itself, and the value.
pub type Lane = (Option<Uuid>, ParamKey);

/// One key of the clip being edited, by where it is stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyRef {
    pub target: Option<Uuid>,
    pub key: ParamKey,
    /// From the clip's start.
    pub at: Time,
}

impl KeyRef {
    fn lane(&self) -> Lane {
        (self.target, self.key)
    }
    fn of(track: &AnimTrack, key: &Keyframe) -> Self {
        Self {
            target: track.target,
            key: track.key,
            at: key.at,
        }
    }
}

/// A copied key: where it was from the first of those copied with it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Copied {
    lane: Lane,
    offset: Time,
    value: f32,
    ease: Ease,
}

#[derive(Clone, Debug)]
enum Drag {
    /// A press that holds nothing: on a key of a locked track, or one that
    /// let a key go. Its release is not a second click.
    Press,
    Keys {
        grabbed: KeyRef,
        /// The value the grabbed key is stored with.
        value: f32,
        from: Pos2,
        delta: Time,
        lift: f32,
        /// A press that never travels is a click and moves nothing.
        moved: bool,
        /// The pointer has travelled up or down far enough to change the
        /// value: a key moved along in time keeps the value it has.
        lifted: bool,
        extend: bool,
    },
    /// One of the two handles of the segment leaving `at`.
    Handle {
        at: KeyRef,
        second: bool,
        ease: Ease,
        from: Pos2,
        moved: bool,
    },
    Marquee {
        from: Pos2,
        /// What was selected before, kept while a modifier is held.
        base: Vec<KeyRef>,
        travelled: bool,
    },
}

/// What of the keyframe editor had the keyboard in the last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Focused {
    #[default]
    Nothing,
    Key,
    Handle,
}

/// The graph's curves as they were last sampled, and what they were sampled
/// from. A frame that changes none of it draws them again as they are.
struct Cache {
    track: Arc<Track>,
    clip: ClipId,
    shown: u64,
    /// The values the visible curves take, with room around them.
    fit: (f32, f32),
    rect: Rect,
    zoom: f32,
    scroll: f32,
    view: (f32, f32),
    draft: u64,
    /// The index of a visible track and its curve in screen points.
    lines: Vec<(usize, Vec<Pos2>)>,
    /// The value axis: where a line is drawn and what it says.
    ticks: Vec<(f32, String)>,
}

pub struct KeyframesState {
    /// Whether the graph has the tracks' place.
    pub graph: bool,
    /// The clip whose lane is open.
    open: Option<ClipId>,
    /// The lane was opened and has not been brought into view yet.
    reveal: bool,
    /// The clip the selected keys are of.
    clip: Option<ClipId>,
    selected: Vec<KeyRef>,
    drag: Option<Drag>,
    /// The tracks a held drag would write.
    draft: Vec<AnimTrack>,
    draft_serial: u64,
    copied: Vec<Copied>,
    /// What the system's clipboard was told when they were copied: a paste
    /// is of them while it still holds this.
    copied_text: String,
    /// The clips that were selected when the keys were last drawn. Keys stay
    /// selected only while these do.
    chosen: Vec<ClipId>,
    hidden: Vec<Lane>,
    solo: Option<Lane>,
    shown_serial: u64,
    /// The values the graph shows when it is not fitted to its curves.
    view: Option<(f32, f32)>,
    /// The values the graph showed last.
    showing: (f32, f32),
    cache: Option<Cache>,
    names: Option<(Arc<Track>, ClipId, Arc<[Named]>)>,
    focus: Focused,
    focus_now: Focused,
    /// Where the keys drawn this frame are, for a marquee.
    spots: Vec<(KeyRef, Pos2)>,
    marquee: Option<Rect>,
    /// How often the graph's curves were sampled.
    samplings: u64,
}

impl Default for KeyframesState {
    fn default() -> Self {
        Self {
            graph: false,
            open: None,
            reveal: false,
            clip: None,
            selected: Vec::new(),
            drag: None,
            draft: Vec::new(),
            draft_serial: 0,
            copied: Vec::new(),
            copied_text: String::new(),
            chosen: Vec::new(),
            hidden: Vec::new(),
            solo: None,
            shown_serial: 0,
            view: None,
            showing: (0.0, 1.0),
            cache: None,
            names: None,
            focus: Focused::Nothing,
            focus_now: Focused::Nothing,
            spots: Vec::new(),
            marquee: None,
            samplings: 0,
        }
    }
}

// ---- Edits: pure, and what the commands are made of ------------------------

/// What an animation track of `clip` addresses, for its range and its name.
pub fn target_of(clip: &Clip, target: Option<Uuid>) -> Option<AnimTarget> {
    let Some(id) = target else {
        return Some(AnimTarget::Clip);
    };
    if let Some(effect) = clip.effects.iter().find(|effect| effect.id == id) {
        return Some(AnimTarget::Effect(effect.kind));
    }
    (clip.masks.iter().any(|mask| mask.id == id)).then_some(AnimTarget::Mask)
}

/// The values a parameter takes, from the tables.
fn range(clip: &Clip, lane: Lane) -> Option<(f32, f32)> {
    let spec = look::param_spec(target_of(clip, lane.0)?, lane.1)?;
    Some((spec.min, spec.max))
}

fn track_of(clip: &Clip, lane: Lane) -> Option<&AnimTrack> {
    (clip.animation.iter()).find(|track| (track.target, track.key) == lane)
}

fn exists(clip: &Clip, key: &KeyRef) -> bool {
    track_of(clip, key.lane()).is_some_and(|track| {
        track
            .keys
            .binary_search_by_key(&key.at, |key| key.at)
            .is_ok()
    })
}

/// As much of `delta` as leaves every picked key inside the clip.
pub fn reach(clip: &Clip, picked: &[KeyRef], delta: Time) -> Time {
    let first = picked.iter().map(|key| key.at.0).min().unwrap_or(0);
    let last = (picked.iter().map(|key| key.at.0).max()).unwrap_or(clip.duration.0);
    Time(delta.0.clamp(-first, (clip.duration.0 - last).max(-first)))
}

/// The tracks that change when the picked keys move by `delta` and their
/// values by `lift`, each held to its parameter's range. A key a moved one
/// lands on gives way to it.
pub fn moved(clip: &Clip, picked: &[KeyRef], delta: Time, lift: f32) -> Vec<AnimTrack> {
    let mut changed = Vec::new();
    for track in &clip.animation {
        let lane = (track.target, track.key);
        if !picked.iter().any(|key| key.lane() == lane) {
            continue;
        }
        let (low, high) = range(clip, lane).unwrap_or((f32::MIN, f32::MAX));
        let is_picked = |key: &Keyframe| picked.contains(&KeyRef::of(track, key));
        let carried: Vec<Keyframe> = (track.keys.iter())
            .filter(|key| is_picked(key))
            .map(|key| Keyframe {
                at: key.at + delta,
                value: if lift == 0.0 {
                    key.value
                } else {
                    (key.value + lift).clamp(low, high)
                },
                ease: key.ease,
            })
            .collect();
        let mut keys: Vec<Keyframe> = (track.keys.iter())
            .filter(|key| !is_picked(key) && !carried.iter().any(|moved| moved.at == key.at))
            .copied()
            .collect();
        keys.extend(carried);
        keys.sort_by_key(|key| key.at);
        changed.push(AnimTrack {
            target: track.target,
            key: track.key,
            keys,
        });
    }
    changed
}

/// The tracks without the picked keys. One left with no key is removed.
pub fn removed(clip: &Clip, picked: &[KeyRef]) -> Vec<AnimTrack> {
    (clip.animation.iter())
        .filter(|track| {
            picked
                .iter()
                .any(|key| key.lane() == (track.target, track.key))
        })
        .map(|track| AnimTrack {
            target: track.target,
            key: track.key,
            keys: (track.keys.iter())
                .filter(|key| !picked.contains(&KeyRef::of(track, key)))
                .copied()
                .collect(),
        })
        .collect()
}

/// Whether a segment leaves `key`: a track's last key has none, and an
/// easing set on it would shape nothing.
pub fn leaves_segment(clip: &Clip, key: &KeyRef) -> bool {
    track_of(clip, key.lane())
        .and_then(|track| track.keys.last())
        .is_some_and(|last| key.at < last.at)
}

/// The tracks with `ease` shaping the segments that leave the picked keys.
/// A track whose only picked key is its last is left out: nothing of it
/// would change.
pub fn eased(clip: &Clip, picked: &[KeyRef], ease: Ease) -> Vec<AnimTrack> {
    (clip.animation.iter())
        .filter_map(|track| {
            let mut track = track.clone();
            let (target, key) = (track.target, track.key);
            let mut any = false;
            let last = track.keys.len().saturating_sub(1);
            for held in &mut track.keys[..last] {
                let at = held.at;
                if picked.contains(&KeyRef { target, key, at }) {
                    held.ease = ease;
                    any = true;
                }
            }
            any.then_some(track)
        })
        .collect()
}

/// The tracks with the copied keys written from `at` on, and those keys.
/// Keys of a parameter the clip does not have, or past its end, are left out.
fn pasted(clip: &Clip, copied: &[Copied], at: Time) -> (Vec<AnimTrack>, Vec<KeyRef>) {
    let mut changed: Vec<AnimTrack> = Vec::new();
    let mut written = Vec::new();
    for key in copied {
        let time = at + key.offset;
        let Some((low, high)) = range(clip, key.lane) else {
            continue;
        };
        if time < Time::ZERO || time > clip.duration {
            continue;
        }
        let index = match (changed.iter()).position(|t| (t.target, t.key) == key.lane) {
            Some(index) => index,
            None => {
                changed.push(AnimTrack {
                    target: key.lane.0,
                    key: key.lane.1,
                    keys: track_of(clip, key.lane).map_or_else(Vec::new, |t| t.keys.clone()),
                });
                changed.len() - 1
            }
        };
        let keys = &mut changed[index].keys;
        let place = keys.partition_point(|held| held.at < time);
        let new = Keyframe {
            at: time,
            value: key.value.clamp(low, high),
            ease: key.ease,
        };
        match keys.get_mut(place).filter(|held| held.at == time) {
            Some(held) => *held = new,
            None => keys.insert(place, new),
        }
        written.push(KeyRef {
            target: key.lane.0,
            key: key.lane.1,
            at: time,
        });
    }
    (changed, written)
}

/// Whether the clip's volume is keyed where the clip is: its media has sound
/// and no linked audio clip owns that sound. A clip that is only muted,
/// soloed out or at volume 0 is, as it keeps its volume row in the inspector.
pub fn takes_volume(project: &Project, clip: &Clip) -> bool {
    fades::has_sound(project, clip) && !sound::owned(project, clip)
}

/// The row a lane has for a clip with no keyframe yet: its volume.
static VOLUME_ROW: AnimTrack = AnimTrack {
    target: None,
    key: ParamKey::Gain,
    keys: Vec::new(),
};

/// The rows of a clip's lane: one for each animated parameter, and an empty
/// one for the volume of a clip that has none and whose volume can be keyed.
fn lane_tracks<'a>(project: &Project, clip: &'a Clip) -> &'a [AnimTrack] {
    if clip.animation.is_empty() && takes_volume(project, clip) {
        std::slice::from_ref(&VOLUME_ROW)
    } else {
        &clip.animation
    }
}

/// The one command that keys the clip's volume `local` after its start at
/// the volume it has there, so that nothing heard changes: a key that lands
/// between two others divides their stretch as a split does, and a held or
/// eased stretch keeps its shape. None when a key is there already, or the
/// stretch cannot be carried by keys.
pub fn key_volume(clip: &Clip, local: Time) -> Option<Command> {
    let lane = (None, ParamKey::Gain);
    let track = match track_of(clip, lane) {
        Some(stored) => {
            let mut track = stored.clone();
            track.insert(local, range(clip, lane)?).then_some(track)?
        }
        None => AnimTrack {
            target: lane.0,
            key: lane.1,
            keys: vec![Keyframe {
                at: local,
                value: gain_at(clip, clip.start + local),
                ease: Ease::Linear,
            }],
        },
    };
    command(clip, "Key volume", vec![track])
}

/// The command "Key volume" asks for: a volume key of `clip` at `playhead`.
/// Says why when there is none to write.
pub fn volume_key(project: &Project, clip: ClipId, playhead: Time) -> Result<Command, String> {
    let (track, clip) = project.clip(clip).ok_or("The clip no longer exists.")?;
    if !fades::has_sound(project, clip) {
        return Err("That clip has no sound to key the volume of.".to_owned());
    }
    if sound::owned(project, clip) {
        return Err(format!("{}.", fades::OWNED));
    }
    if track.locked {
        return Err("Unlock the clip's track to key its volume.".to_owned());
    }
    let local = playhead - clip.start;
    if local < Time::ZERO || local > clip.duration {
        return Err("Put the playhead on the clip to key its volume.".to_owned());
    }
    let keyed = track_of(clip, (None, ParamKey::Gain))
        .is_some_and(|track| track.keys.iter().any(|key| key.at == local));
    if keyed {
        return Err("The volume is keyed there already.".to_owned());
    }
    key_volume(clip, local).ok_or_else(|| "The volume's curve cannot take a key there.".to_owned())
}

/// The one command that writes `tracks` into the clip: a `SetKeyframes` for
/// each track that differs from the stored one, in a batch named for the
/// gesture, so that it is one undo step and says what it was.
pub fn command(clip: &Clip, label: &str, tracks: Vec<AnimTrack>) -> Option<Command> {
    let commands: Vec<Command> = (tracks.into_iter())
        .filter(|track| {
            let stored = track_of(clip, (track.target, track.key));
            stored.map_or(!track.keys.is_empty(), |stored| stored.keys != track.keys)
        })
        .map(|track| Command::SetKeyframes {
            clip_id: clip.id,
            target: track.target,
            key: track.key,
            keys: track.keys,
        })
        .collect();
    (!commands.is_empty()).then(|| Command::Batch {
        label: label.into(),
        commands,
    })
}

/// The two control points of a segment's ease. A straight segment has them a
/// third of the way along it.
fn controls(ease: Ease) -> Option<[f32; 4]> {
    match ease {
        Ease::Hold => None,
        Ease::Linear => Some([1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0]),
        Ease::Bezier { x1, y1, x2, y2 } => Some([x1, y1, x2, y2]),
    }
}

fn with_control(ease: Ease, second: bool, x: f32, y: f32) -> Ease {
    let [x1, y1, x2, y2] = controls(ease).unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let round = |value: f32| (value * 1000.0).round() / 1000.0;
    let (x, y) = (round(x.clamp(0.0, 1.0)), round(y.clamp(-4.0, 4.0)));
    if second {
        Ease::Bezier {
            x1,
            y1,
            x2: x,
            y2: y,
        }
    } else {
        Ease::Bezier {
            x1: x,
            y1: y,
            x2,
            y2,
        }
    }
}

/// The tracks of `project` as shared, to tell one revision of a track from
/// the next without comparing what is in it.
fn shared(project: &Project, track: &Track) -> Option<Arc<Track>> {
    (project.tracks.iter())
        .find(|held| held.id == track.id)
        .cloned()
}

fn lane_name(clip: &Clip, track: &AnimTrack) -> String {
    let Some(target) = target_of(clip, track.target) else {
        return "Keyframes".to_owned();
    };
    let label = inspector::param_name(target, track.key);
    match target {
        AnimTarget::Clip => label.to_owned(),
        AnimTarget::Effect(kind) => format!("{} {}", inspector::effect_title(kind), label),
        AnimTarget::Mask => {
            let index = (clip.masks.iter()).position(|mask| Some(mask.id) == track.target);
            format!("Mask {} {}", index.map_or(1, |index| index + 1), label)
        }
    }
}

/// A step of the value axis that reads well and leaves `room` points between
/// its lines.
fn tick(span: f32, room: f32) -> f32 {
    let rough = span / (room / 34.0).max(1.0);
    let magnitude = 10f32.powf(rough.log10().floor());
    let part = rough / magnitude;
    magnitude
        * match part {
            part if part <= 1.0 => 1.0,
            part if part <= 2.0 => 2.0,
            part if part <= 5.0 => 5.0,
            _ => 10.0,
        }
}

/// What an animated parameter is called, and its controls in the legend.
struct Named {
    name: String,
    show: String,
    solo: String,
}

/// What a gesture needs to know of the clip it edits and where it is drawn.
struct Sheet<'a> {
    p: &'a Palette,
    clip: &'a Clip,
    locked: bool,
    rate: FrameRate,
    playhead: Time,
    snapping: bool,
    geometry: Geometry,
    /// The graph's value axis; the lane has none.
    axis: Option<Axis>,
}

#[derive(Clone, Copy)]
struct Axis {
    bottom: f32,
    low: f32,
    /// Points to one unit of value.
    rise: f32,
}

impl Axis {
    fn y(&self, value: f32) -> f32 {
        self.bottom - (value - self.low) * self.rise
    }
    fn value(&self, y: f32) -> f32 {
        self.low + (self.bottom - y) / self.rise
    }
}

impl Sheet<'_> {
    fn x(&self, at: Time) -> f32 {
        self.geometry.x(self.clip.start + at)
    }
    fn timecode(&self, at: Time) -> String {
        timefmt::timecode(self.clip.start + at, self.rate)
    }
}

impl KeyframesState {
    /// Gives up a held drag, or else the selected keys. Says whether there
    /// was either: an Escape that did this does nothing more.
    pub fn cancel(&mut self) -> bool {
        if self.drag.take().is_some() {
            self.draft.clear();
            self.draft_serial += 1;
            self.marquee = None;
            return true;
        }
        let had = !self.selected.is_empty();
        self.selected.clear();
        had
    }

    /// Whether the arrows, Delete and the clipboard chords are the selected
    /// keys' and not the timeline's: there are keys selected, and the
    /// keyboard is with them or with nothing.
    ///
    /// They are theirs only while the clips selected are the ones that were
    /// when the keys were: a clip chosen since, on the canvas or with a
    /// chord, is what the keys are then for.
    pub fn owns_keys(&self, ctx: &egui::Context, selection: &Selection) -> bool {
        !self.selected.is_empty()
            && self.chosen == selection.clips
            && (self.focus != Focused::Nothing || ctx.memory(|memory| memory.focused().is_none()))
    }

    /// Whether a paste writes the copied keys: keys were the last thing
    /// copied and a clip's keyframes are in view.
    pub fn takes_paste(&self) -> bool {
        !self.copied.is_empty() && self.clip.is_some()
    }

    /// Whether a paste of `text` is of the copied keys: it is what the
    /// clipboard was told when they were copied. Anything else was copied
    /// since, here or in another window, and is not theirs.
    pub fn pastes(&self, text: &str) -> bool {
        self.takes_paste() && text.trim() == self.copied_text
    }

    /// What the system's clipboard holds for the copied keys.
    #[cfg(test)]
    pub fn copied_text(&self) -> &str {
        &self.copied_text
    }

    /// A press on the tracks lets the selected keys go, so that Delete and
    /// the arrows are the timeline's again.
    pub(super) fn let_go(&mut self) {
        self.selected.clear();
    }

    /// Clips were copied: the next paste is theirs.
    pub fn forget_copied(&mut self) {
        self.copied.clear();
        self.copied_text.clear();
    }

    /// The clip whose lane is open.
    pub fn lane_clip(&self) -> Option<ClipId> {
        self.open
    }

    pub(super) fn set_lane(&mut self, clip: Option<ClipId>) {
        self.open = clip;
        self.reveal = clip.is_some();
    }

    /// Whether a lane was opened since this was last asked: the tracks are
    /// then scrolled to show it.
    pub(super) fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.reveal)
    }

    #[cfg(test)]
    pub fn selected(&self) -> &[KeyRef] {
        &self.selected
    }

    /// Opens the lane of the one selected clip, or closes the open one. A
    /// clip has a lane when it has keyframes, or sound whose volume could
    /// have: that one opens on an empty Volume row.
    pub fn toggle_lane(&mut self, project: &Project, selection: &Selection) {
        let laned = (selection.single()).filter(|id| {
            (project.clip(*id)).is_some_and(|(_, clip)| !lane_tracks(project, clip).is_empty())
        });
        self.set_lane(laned.filter(|id| self.open != Some(*id)));
    }

    /// Whether [`KeyframesState::toggle_lane`] would do anything.
    pub fn can_toggle_lane(&self, project: &Project, selection: &Selection) -> bool {
        self.open.is_some()
            || (selection.single())
                .and_then(|id| project.clip(id))
                .is_some_and(|(_, clip)| !lane_tracks(project, clip).is_empty())
    }

    /// What the canvas draws in the stored animation's place while keys or a
    /// handle are held.
    pub fn overriding(&self, project: &Project) -> Option<ClipOverride> {
        if self.draft.is_empty() {
            return None;
        }
        let (_, clip) = project.clip(self.clip?)?;
        let mut animation = clip.animation.clone();
        for draft in &self.draft {
            match (animation.iter_mut())
                .find(|track| (track.target, track.key) == (draft.target, draft.key))
            {
                Some(track) => track.keys.clone_from(&draft.keys),
                None => animation.push(draft.clone()),
            }
        }
        animation.retain(|track| !track.keys.is_empty());
        Some(ClipOverride {
            clip: clip.id,
            transform: None,
            effects: None,
            masks: None,
            blend: None,
            animation: Some(animation),
            reading: None,
            content: None,
        })
    }

    /// How often the graph's curves have been sampled.
    #[cfg(test)]
    pub fn samplings(&self) -> u64 {
        self.samplings
    }

    /// The curve the graph last drew for the `index`th animated parameter.
    #[cfg(test)]
    pub fn curve(&self, index: usize) -> Option<&[Pos2]> {
        let lines = &self.cache.as_ref()?.lines;
        let (_, points) = lines.iter().find(|(drawn, _)| *drawn == index)?;
        Some(points)
    }

    /// The number of rows the open lane has, and the track it is under.
    pub(super) fn lane_rows(&self, project: &Project) -> Option<(editor_core::TrackId, usize)> {
        if self.graph {
            return None;
        }
        let (track, clip) = project.clip(self.open?)?;
        let rows = lane_tracks(project, clip).len();
        (rows > 0).then_some((track.id, rows))
    }

    /// The height of a lane of `rows` parameters.
    pub(super) fn lane_height(rows: usize) -> f32 {
        rows as f32 * ROW + PAD * 2.0
    }

    /// Moves the graph's values by `points` of a graph `height` high.
    pub(super) fn pan(&mut self, points: f32, height: f32) {
        let (low, high) = self.showing;
        let by = points / height.max(1.0) * (high - low);
        self.view = Some((low + by, high + by));
    }

    /// Starts a frame: the clip being edited is the one selected in the
    /// graph and the open one in the lane, keys that are gone are no longer
    /// selected, and the keys the keyboard asked for are edited.
    pub(super) fn begin(
        &mut self,
        ui: &Ui,
        geometry: &Geometry,
        selection: &Selection,
        input: &TimelineInput,
        actions: &mut Vec<Action>,
    ) {
        self.focus = std::mem::take(&mut self.focus_now);
        self.spots.clear();
        let chosen: Arc<[ClipId]> = selection.clips.as_slice().into();
        ui.ctx()
            .data_mut(|data| data.insert_temp(chosen_id(), chosen));
        let project = input.project;
        let id = if self.graph {
            selection.single()
        } else {
            self.open
        };
        // The graph draws curves and has none for a clip without keyframes;
        // the lane has the empty row of a volume that could be keyed.
        let found = id.and_then(|id| project.clip(id)).filter(|(_, clip)| {
            !clip.animation.is_empty() || (!self.graph && takes_volume(project, clip))
        });
        if found.map(|(_, clip)| clip.id) != self.clip {
            self.clip = found.map(|(_, clip)| clip.id);
            self.selected.clear();
            self.cancel();
            self.hidden.clear();
            self.solo = None;
            self.view = None;
            self.shown_serial += 1;
        }
        let Some((track, clip)) = found else {
            return;
        };
        if self.drag.is_none() {
            self.selected.retain(|key| exists(clip, key));
            // Another clip was selected since, away from the keys: the
            // arrows and Delete are that clip's.
            if !self.selected.is_empty() && self.chosen != selection.clips {
                self.selected.clear();
            }
        }
        let sheet = self.sheet(input, geometry, track, clip, None);
        self.keyboard(ui, &sheet, selection, actions);
    }

    /// Ends the keys' part of a frame. Keys that are selected have their
    /// clip selected, and alone: what the inspector and the canvas show is
    /// the clip Delete and the arrows then edit the keys of. The clips
    /// selected now are the ones the selected keys go with.
    fn end(&mut self, clip: ClipId, selection: &mut Selection) {
        if !self.selected.is_empty() && selection.single() != Some(clip) {
            selection.only(clip);
        }
        if self.chosen != selection.clips {
            self.chosen.clone_from(&selection.clips);
        }
    }

    fn sheet<'a>(
        &self,
        input: &'a TimelineInput,
        geometry: &Geometry,
        track: &Track,
        clip: &'a Clip,
        axis: Option<Axis>,
    ) -> Sheet<'a> {
        Sheet {
            p: input.p,
            clip,
            locked: track.locked,
            rate: input.project.settings.frame_rate,
            playhead: input.playhead,
            snapping: input.snapping,
            geometry: *geometry,
            axis,
        }
    }

    /// The names of the clip's rows, made once for each revision of its
    /// track.
    fn names(
        &mut self,
        project: &Project,
        track: &Track,
        clip: &Clip,
        rows: &[AnimTrack],
    ) -> Arc<[Named]> {
        // Whether a clip has the empty row is not its own track's to say:
        // the names kept are those of as many rows.
        if let Some((held, id, names)) = &self.names
            && *id == clip.id
            && held.id == track.id
            && names.len() == rows.len()
            && (project.tracks.iter()).any(|shared| Arc::ptr_eq(shared, held))
        {
            return names.clone();
        }
        let names: Arc<[Named]> = (rows.iter())
            .map(|track| {
                let name = lane_name(clip, track);
                Named {
                    show: format!("Show {name}"),
                    solo: format!("Solo {name}"),
                    name,
                }
            })
            .collect();
        if let Some(shared) = shared(project, track) {
            self.names = Some((shared, clip.id, names.clone()));
        }
        names
    }

    fn visible(&self, lane: Lane) -> bool {
        match self.solo {
            Some(solo) => solo == lane,
            None => !self.hidden.contains(&lane),
        }
    }

    /// Where a stored key is drawn: where the held drag has it.
    fn placed(&self, sheet: &Sheet, at: &KeyRef, value: f32) -> (Time, f32) {
        if let Some(Drag::Keys {
            delta,
            lift,
            moved: true,
            ..
        }) = &self.drag
            && self.selected.contains(at)
        {
            let value = match range(sheet.clip, at.lane()) {
                Some((low, high)) if *lift != 0.0 => (value + lift).clamp(low, high),
                _ => value,
            };
            return (at.at + *delta, value);
        }
        (at.at, value)
    }

    /// The ease of the segment leaving a stored key, with a held handle's.
    fn ease_of(&self, at: &KeyRef, stored: Ease) -> Ease {
        match &self.drag {
            Some(Drag::Handle {
                at: held,
                ease,
                moved: true,
                ..
            }) if held == at => *ease,
            _ => stored,
        }
    }

    // ---- Keyboard ---------------------------------------------------------

    fn keyboard(
        &mut self,
        ui: &Ui,
        sheet: &Sheet,
        selection: &Selection,
        actions: &mut Vec<Action>,
    ) {
        let ctx = ui.ctx();
        let owned = self.owns_keys(ctx, selection);
        if !(owned || self.takes_paste())
            || self.drag.is_some()
            || ctx.text_edit_focused()
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        let (mut frames, mut lifts) = (0_i64, 0_i64);
        let (mut delete, mut copy, mut cut, mut paste) = (false, false, false, false);
        ui.input(|i| {
            for event in &i.events {
                match event {
                    // A key is read with the modifiers of its own event.
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.command && !modifiers.alt && !modifiers.ctrl => {
                        let step = if modifiers.shift { 10 } else { 1 };
                        match key {
                            egui::Key::ArrowLeft => frames -= step,
                            egui::Key::ArrowRight => frames += step,
                            egui::Key::ArrowUp => lifts += step,
                            egui::Key::ArrowDown => lifts -= step,
                            egui::Key::Delete | egui::Key::Backspace if !modifiers.shift => {
                                delete = true;
                            }
                            _ => {}
                        }
                    }
                    egui::Event::Copy => copy = true,
                    egui::Event::Cut => cut = true,
                    egui::Event::Paste(text) => paste |= self.pastes(text),
                    _ => {}
                }
            }
        });
        if owned && (copy || cut) {
            self.copy(ctx, sheet.clip);
            if !cut {
                actions.push(Action::Notify("Copied keyframes".to_owned()));
            }
        }
        if paste {
            self.paste(sheet, actions);
            return;
        }
        if !owned {
            return;
        }
        if delete || cut {
            self.delete(sheet, actions);
            return;
        }
        // A focused handle takes the arrows itself.
        if self.focus == Focused::Handle || (frames == 0 && (lifts == 0 || !self.graph)) {
            return;
        }
        let delta = reach(sheet.clip, &self.selected, sheet.rate.frame_time(frames));
        // A hundredth of what the graph shows, and of the narrowest range a
        // selected key has: a value of 0..=1 is not thrown to its limit
        // because it is drawn beside one in the hundreds.
        let lift = if self.graph {
            let narrowest = (self.selected.iter())
                .filter_map(|key| range(sheet.clip, key.lane()))
                .map(|(low, high)| high - low)
                .fold(self.showing.1 - self.showing.0, f32::min);
            lifts as f32 * narrowest / 100.0
        } else {
            0.0
        };
        let tracks = moved(sheet.clip, &self.selected, delta, lift);
        if let Some(action) = edit(sheet, "Move keyframes", tracks) {
            actions.push(action);
            // A locked track refuses the edit, and its keys stay put.
            if !sheet.locked {
                for key in &mut self.selected {
                    key.at = key.at + delta;
                }
            }
        }
    }

    fn copy(&mut self, ctx: &egui::Context, clip: &Clip) {
        let Some(first) = self.selected.iter().map(|key| key.at).min() else {
            return;
        };
        // The system's clipboard is told too: a paste arrives only when it
        // holds something, and what it held was copied before these.
        self.copied_text = format!("Neptune Video keyframes ({})", self.selected.len());
        ctx.copy_text(self.copied_text.clone());
        self.copied = (self.selected.iter())
            .filter_map(|key| {
                let track = track_of(clip, key.lane())?;
                let index = track.keys.binary_search_by_key(&key.at, |k| k.at).ok()?;
                Some(Copied {
                    lane: key.lane(),
                    offset: key.at - first,
                    value: track.keys[index].value,
                    ease: track.keys[index].ease,
                })
            })
            .collect();
    }

    fn paste(&mut self, sheet: &Sheet, actions: &mut Vec<Action>) {
        let at = sheet.playhead - sheet.clip.start;
        if at < Time::ZERO || at > sheet.clip.duration {
            let text = "Put the playhead on the clip to paste keyframes.";
            actions.push(Action::Notify(text.to_owned()));
            return;
        }
        let (tracks, written) = pasted(sheet.clip, &self.copied, at);
        if written.is_empty() {
            let text = "The copied keyframes do not fit this clip.";
            actions.push(Action::Notify(text.to_owned()));
            return;
        }
        if let Some(action) = edit(sheet, "Paste keyframes", tracks) {
            actions.push(action);
        }
        if !sheet.locked {
            self.selected = written;
        }
    }

    fn delete(&mut self, sheet: &Sheet, actions: &mut Vec<Action>) {
        let tracks = removed(sheet.clip, &self.selected);
        if let Some(action) = edit(sheet, "Delete keyframes", tracks) {
            actions.push(action);
        }
        if !sheet.locked {
            self.selected.clear();
        }
    }

    fn select_lane(&mut self, clip: &Clip, lane: Option<Lane>) {
        self.selected = (clip.animation.iter())
            .filter(|track| match lane {
                Some(lane) => (track.target, track.key) == lane,
                None => self.visible((track.target, track.key)),
            })
            .flat_map(|track| track.keys.iter().map(|key| KeyRef::of(track, key)))
            .collect();
    }

    // ---- Menu -------------------------------------------------------------

    /// What is done to the selected keys by name: the route to everything a
    /// pointer does on a key, and the only one to a named easing.
    fn menu(&mut self, ui: &mut Ui, sheet: &Sheet, actions: &mut Vec<Action>) {
        let p = sheet.p;
        menu_style(ui, 210.0);
        let some = !self.selected.is_empty();
        let common = (self.selected.first())
            .and_then(|key| self.stored_ease(sheet.clip, key))
            .filter(|ease| {
                (self.selected.iter()).all(|key| self.stored_ease(sheet.clip, key) == Some(*ease))
            });
        // The easing is of the stretch that leaves a keyframe.
        let leaves = (self.selected.iter()).any(|key| leaves_segment(sheet.clip, key));
        if some && !leaves {
            menu_note(ui, p, "No keyframe follows to ease to");
        }
        ui.add_enabled_ui(leaves && !sheet.locked, |ui| {
            for (name, ease) in EASINGS {
                let icon = (common == Some(ease)).then_some(Icon::Check);
                if menu_item(ui, p, icon, name, "").clicked() {
                    let tracks = eased(sheet.clip, &self.selected, ease);
                    actions.extend(edit(sheet, "Change easing", tracks));
                    ui.close();
                }
            }
        });
        widgets::menu_separator(ui, p);
        ui.add_enabled_ui(some, |ui| {
            let keys = super::shortcut("Cmd+C");
            if menu_item(ui, p, Some(Icon::Copy), "Copy keyframes", &keys).clicked() {
                self.copy(ui.ctx(), sheet.clip);
                ui.close();
            }
        });
        ui.add_enabled_ui(!self.copied.is_empty() && !sheet.locked, |ui| {
            let keys = super::shortcut("Cmd+V");
            if menu_item(ui, p, Some(Icon::Clipboard), "Paste at playhead", &keys).clicked() {
                self.paste(sheet, actions);
                ui.close();
            }
        });
        ui.add_enabled_ui(some && !sheet.locked, |ui| {
            if menu_item(ui, p, Some(Icon::Trash), "Delete keyframes", "Delete").clicked() {
                self.delete(sheet, actions);
                ui.close();
            }
        });
        widgets::menu_separator(ui, p);
        if menu_item(ui, p, None, "Select all keyframes", "").clicked() {
            self.select_lane(sheet.clip, None);
            ui.close();
        }
    }

    fn stored_ease(&self, clip: &Clip, key: &KeyRef) -> Option<Ease> {
        let track = track_of(clip, key.lane())?;
        let index = track.keys.binary_search_by_key(&key.at, |k| k.at).ok()?;
        Some(track.keys[index].ease)
    }

    // ---- One key ----------------------------------------------------------

    #[expect(
        clippy::too_many_arguments,
        reason = "one key needs its place, its name and what a press on it changes"
    )]
    fn key(
        &mut self,
        ui: &mut Ui,
        sheet: &Sheet,
        name: &str,
        index: usize,
        at: KeyRef,
        stored: &Keyframe,
        center: Pos2,
        selection: &mut Selection,
        actions: &mut Vec<Action>,
    ) {
        let p = sheet.p;
        self.spots.push((at, center));
        let rect = Rect::from_center_size(center, vec2(14.0, ROW - 2.0));
        let id = Id::new(("keyframe", sheet.clip.id, at.target, at.key, index));
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let mut picked = self.selected.contains(&at);
        response.widget_info(|| {
            let label = format!("{name} keyframe at {}", sheet.timecode(at.at));
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, picked, label)
        });
        let (pressed, extend, pointer) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.modifiers.shift || i.modifiers.command,
                i.pointer.interact_pos(),
            )
        });
        let toggle = |selected: &mut Vec<KeyRef>| match selected.iter().position(|key| *key == at) {
            Some(index) => {
                selected.remove(index);
                false
            }
            None => {
                selected.push(at);
                true
            }
        };
        if pressed && response.is_pointer_button_down_on() {
            if !selection.contains(sheet.clip.id) {
                selection.only(sheet.clip.id);
            }
            picked = if extend {
                toggle(&mut self.selected)
            } else {
                if !picked {
                    self.selected.clear();
                    self.selected.push(at);
                }
                true
            };
            response.request_focus();
            self.drag = Some(if picked && !sheet.locked {
                Drag::Keys {
                    grabbed: at,
                    value: stored.value,
                    from: pointer.unwrap_or(center),
                    delta: Time::ZERO,
                    lift: 0.0,
                    moved: false,
                    lifted: false,
                    extend,
                }
            } else {
                Drag::Press
            });
        } else if response.clicked() && self.drag.is_none() {
            // Enter or Space on the key that has the keyboard, or a press
            // and its release inside one frame.
            picked = if extend {
                toggle(&mut self.selected)
            } else {
                self.selected.clear();
                self.selected.push(at);
                true
            };
        }
        if response.secondary_clicked() && !picked {
            self.selected.clear();
            self.selected.push(at);
            picked = true;
        }
        response.context_menu(|ui| self.menu(ui, sheet, actions));
        if response.has_focus() {
            self.focus_now = Focused::Key;
            // The arrows move the keys and do not leave them.
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
        }
        if !ui.is_rect_visible(rect) {
            return;
        }
        let painter = ui.painter();
        // In ink, and in the accent only while selected; a locked track's
        // keys are quieter, as its tiles are. The graph's are smaller, so
        // the curves they sit on stay the subject.
        let fill = if sheet.locked {
            p.muted
        } else if picked {
            p.key_selected()
        } else {
            p.key()
        };
        let lit = !sheet.locked && response.hovered();
        let radius = match (sheet.axis.is_some(), lit) {
            (false, false) => DIAMOND,
            (false, true) => DIAMOND_HOVER,
            (true, false) => POINT,
            (true, true) => POINT_HOVER,
        };
        // A keyline of the ground keeps a key apart from the line or the
        // curve it sits on.
        let edge = Stroke::new(1.0, p.bg);
        if stored.ease == Ease::Hold {
            // A key that holds its value until the next is square.
            let body = Rect::from_center_size(center, Vec2::splat(radius * 1.6));
            painter.rect(body, CornerRadius::same(1), fill, edge, StrokeKind::Middle);
        } else {
            painter.add(Shape::convex_polygon(diamond(center, radius), fill, edge));
        }
        if response.has_focus() {
            painter.circle_stroke(center, radius + 3.0, ring(p));
        }
        if !sheet.locked && self.drag.is_none() && response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        response.on_hover_ui(|ui| {
            ui.label(format!("{name}, {}", sheet.timecode(at.at)));
        });
    }

    // ---- The held gesture -------------------------------------------------

    /// Follows the pointer with what is held, and ends it when the button
    /// comes up: keys and handles as one edit, a marquee as a selection, and
    /// a press on nothing as a move of the playhead.
    fn gesture(&mut self, ui: &Ui, sheet: &Sheet, ground: &Response, actions: &mut Vec<Action>) {
        let (pressed, released, down, extend, shift, pointer) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.primary_down(),
                i.modifiers.shift || i.modifiers.command,
                i.modifiers.shift,
                i.pointer.latest_pos(),
            )
        });
        if pressed
            && ground.is_pointer_button_down_on()
            && let Some(from) = pointer
        {
            self.drag = Some(Drag::Marquee {
                from,
                base: if extend {
                    self.selected.clone()
                } else {
                    Vec::new()
                },
                travelled: false,
            });
        } else if ground.clicked() && self.drag.is_none() {
            // A press and its release inside one frame.
            if !extend {
                self.selected.clear();
            }
            if let Some(pos) = ground.interact_pointer_pos() {
                let time = rules::quantize(sheet.geometry.time(pos.x), sheet.rate);
                actions.push(Action::Seek(time));
            }
        }
        self.marquee = None;
        let clip = sheet.clip;
        let mut redraft = None;
        match &mut self.drag {
            Some(Drag::Keys {
                grabbed,
                value,
                from,
                delta,
                lift,
                moved,
                lifted,
                ..
            }) => {
                if let Some(pos) = pointer {
                    let mut travel = pos - *from;
                    *moved |= travel.length() >= TRAVEL;
                    *lifted |= travel.y.abs() >= TRAVEL;
                    // Shift keeps a key in the graph on the axis it is
                    // moved along.
                    if shift && sheet.axis.is_some() {
                        if travel.x.abs() >= travel.y.abs() {
                            travel.y = 0.0;
                        } else {
                            travel.x = 0.0;
                        }
                    }
                    if *moved {
                        let held = clip.start + grabbed.at;
                        let wanted = held + sheet.geometry.span(travel.x);
                        let reach_of_snap = sheet.geometry.span(SNAP_POINTS).0;
                        let to = if sheet.snapping
                            && (sheet.playhead.0 - wanted.0).abs() <= reach_of_snap
                        {
                            sheet.playhead
                        } else if wanted < Time::ZERO {
                            Time::ZERO
                        } else {
                            rules::quantize(wanted, sheet.rate)
                        };
                        // A value changes only once the pointer has gone up
                        // or down, and lands on the finest round number a
                        // point of the graph's height tells apart: never one
                        // coarser than a hundredth of the range it has.
                        let lifting = *lifted && travel.y != 0.0;
                        let raised = (sheet.axis.filter(|_| lifting)).map_or(0.0, |axis| {
                            let unit = unit(axis.rise, range(clip, grabbed.lane()));
                            ((*value - travel.y / axis.rise) / unit).round() * unit - *value
                        });
                        let next = (reach(clip, &self.selected, to - held), raised);
                        if next != (*delta, *lift) {
                            (*delta, *lift) = next;
                            redraft = Some(moved_tracks(clip, &self.selected, next));
                        }
                    }
                }
            }
            Some(Drag::Handle {
                at,
                second,
                ease,
                from,
                moved,
            }) => {
                if let (Some(pos), Some(track)) = (pointer, track_of(clip, at.lane())) {
                    *moved |= (pos - *from).length() >= TRAVEL;
                    let index = track.keys.partition_point(|key| key.at < at.at);
                    if let (true, Some(k0), Some(k1)) =
                        (*moved, track.keys.get(index), track.keys.get(index + 1))
                    {
                        let (x0, x1) = (sheet.x(k0.at), sheet.x(k1.at));
                        let current = controls(*ease).unwrap_or([0.0, 0.0, 1.0, 1.0]);
                        let x = (pos.x - x0) / (x1 - x0).max(1e-3);
                        // A level segment has no height to pull a handle
                        // through: only when it eases is changed.
                        let y = match sheet.axis {
                            Some(axis) if k1.value != k0.value => {
                                (axis.value(pos.y) - k0.value) / (k1.value - k0.value)
                            }
                            _ => current[if *second { 3 } else { 1 }],
                        };
                        let next = with_control(*ease, *second, x, y);
                        if next != *ease {
                            *ease = next;
                            redraft = Some(eased(clip, std::slice::from_ref(at), next));
                        }
                    }
                }
            }
            Some(Drag::Marquee {
                from,
                base,
                travelled,
            }) => {
                if let Some(pos) = pointer {
                    *travelled |= pos.distance(*from) >= TRAVEL;
                    if *travelled {
                        let rect = Rect::from_two_pos(*from, pos);
                        let mut picked = base.clone();
                        for (key, spot) in &self.spots {
                            if rect.contains(*spot) && !picked.contains(key) {
                                picked.push(*key);
                            }
                        }
                        self.selected = picked;
                        self.marquee = Some(rect);
                    }
                }
            }
            Some(Drag::Press) | None => {}
        }
        if let Some(tracks) = redraft {
            self.draft = tracks;
            self.draft_serial += 1;
        }

        if !(released || !down) {
            return;
        }
        let Some(drag) = self.drag.take() else {
            return;
        };
        self.marquee = None;
        // What is written is made from the clip as it is now. The draft was
        // made from the clip as it was when the pointer last moved, and an
        // undo or an agent's edit may have changed its keys since.
        self.draft.clear();
        self.draft_serial += 1;
        match drag {
            Drag::Press => {}
            Drag::Keys {
                grabbed,
                delta,
                lift,
                moved,
                extend,
                ..
            } => {
                if moved && (delta != Time::ZERO || lift != 0.0) {
                    self.selected.retain(|key| exists(clip, key));
                    let delta = reach(clip, &self.selected, delta);
                    let tracks = moved_tracks(clip, &self.selected, (delta, lift));
                    if let Some(action) = edit(sheet, "Move keyframes", tracks) {
                        actions.push(action);
                        for key in &mut self.selected {
                            key.at = key.at + delta;
                        }
                    }
                } else if !moved && !extend {
                    // A plain click on one of several selected keys keeps
                    // only that one.
                    self.selected.clear();
                    self.selected.push(grabbed);
                }
            }
            Drag::Handle {
                at, ease, moved, ..
            } => {
                if moved && exists(clip, &at) {
                    let tracks = eased(clip, std::slice::from_ref(&at), ease);
                    actions.extend(edit(sheet, "Change easing", tracks));
                }
            }
            Drag::Marquee {
                from,
                travelled,
                base,
            } => {
                // A press that did not travel is a click on nothing: it
                // lets the keys go and moves the playhead there.
                if !travelled {
                    if base.is_empty() {
                        self.selected.clear();
                    }
                    let time = rules::quantize(sheet.geometry.time(from.x), sheet.rate);
                    actions.push(Action::Seek(time));
                }
            }
        }
    }

    fn paint_marquee(&self, ui: &Ui, p: &Palette, within: Rect) {
        if let Some(rect) = self.marquee {
            // As the tiles' marquee: a wash and a line of the accent.
            let painter = ui.painter_at(within);
            painter.rect(
                rect,
                CornerRadius::same(2),
                theme::tint(p.accent, 0.10),
                Stroke::new(1.0, theme::tint(p.accent, 0.6)),
                StrokeKind::Inside,
            );
        }
    }

    // ---- The lane ---------------------------------------------------------

    /// The open clip's lane in `band`, a strip of the lanes under its track:
    /// a row for each animated parameter, a diamond at each key. A clip with
    /// sound and no keyframe has one empty row, its volume's.
    pub(super) fn show_lane(
        &mut self,
        ui: &mut Ui,
        band: Rect,
        geometry: &Geometry,
        selection: &mut Selection,
        input: &TimelineInput,
        actions: &mut Vec<Action>,
    ) {
        let Some((track, clip)) = self.open.and_then(|id| input.project.clip(id)) else {
            return;
        };
        let p = input.p;
        let view = band.intersect(geometry.lanes);
        if !view.is_positive() {
            return;
        }
        let rows = lane_tracks(input.project, clip);
        let names = self.names(input.project, track, clip, rows);
        let sheet = self.sheet(input, geometry, track, clip, None);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(band));
        ui.set_clip_rect(view);
        let ui = &mut ui;
        let ground = ui.interact(
            band,
            Id::new(("keyframe-lane", clip.id)),
            Sense::click_and_drag(),
        );
        let painter = ui.painter().clone();
        // The lane is the timeline's ground under its track's row, and the
        // clip's own span, where its keys can be, is a band of a row carried
        // down from the clip.
        painter.rect_filled(band, CornerRadius::ZERO, p.bg);
        let span = Rect::from_min_max(
            pos2(sheet.x(Time::ZERO), band.top()),
            pos2(sheet.x(clip.duration), band.bottom()),
        );
        painter.rect_filled(span, CornerRadius::ZERO, p.lane());

        let held = matches!(self.drag, Some(Drag::Keys { moved: true, .. }));
        let (from, to) = (
            geometry.time(view.left() - 10.0) - clip.start,
            geometry.time(view.right() + 10.0) - clip.start,
        );
        for (row, track) in rows.iter().enumerate() {
            let y = band.top() + PAD + ROW * (row as f32 + 0.5);
            if y + ROW * 0.5 < view.top() || y - ROW * 0.5 > view.bottom() {
                continue;
            }
            // A hairline over each row: the first under the track's row,
            // the others between the parameters.
            let line = if row == 0 {
                band.top() + 0.5
            } else {
                y - ROW * 0.5
            };
            painter.hline(band.x_range(), line, p.hairline());
            if let (Some(first), Some(last)) = (track.keys.first(), track.keys.last()) {
                painter.hline(
                    sheet.x(first.at)..=sheet.x(last.at),
                    y,
                    Stroke::new(1.0, theme::tint(p.fg, 0.16)),
                );
            }
            // Keys are in time order, so those in view are one run of them.
            // Held ones are drawn where the drag has them, wherever that is.
            let run = if held {
                0..track.keys.len()
            } else {
                track.keys.partition_point(|key| key.at < from)
                    ..track.keys.partition_point(|key| key.at <= to)
            };
            for index in run {
                let key = &track.keys[index];
                let at = KeyRef::of(track, key);
                let (time, _) = self.placed(&sheet, &at, key.value);
                let center = pos2(sheet.x(time), y);
                self.key(
                    ui,
                    &sheet,
                    &names[row].name,
                    index,
                    at,
                    key,
                    center,
                    selection,
                    actions,
                );
            }
        }
        self.gesture(ui, &sheet, &ground, actions);
        self.end(clip.id, selection);
        self.paint_marquee(ui, p, view);
        ground.context_menu(|ui| self.menu(ui, &sheet, actions));
    }

    /// The names of the open lane's rows, under its track's head.
    pub(super) fn lane_heads(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        geometry: &Geometry,
        input: &TimelineInput,
        actions: &mut Vec<Action>,
    ) {
        let Some((track, clip)) = self.open.and_then(|id| input.project.clip(id)) else {
            return;
        };
        let p = input.p;
        let rows = lane_tracks(input.project, clip);
        let names = self.names(input.project, track, clip, rows);
        let sheet = self.sheet(input, geometry, track, clip, None);
        // The head's card runs on down beside the lane: a hairline parts the
        // track's head from the names of its clip's parameters, which stand
        // right-aligned against the lane they name.
        if ui.is_rect_visible(rect) {
            ui.painter().hline(
                (rect.left() + 8.0)..=(rect.right() - 8.0),
                rect.top() + 0.5,
                p.hairline(),
            );
        }
        for (row, animated) in rows.iter().enumerate() {
            let line = Rect::from_min_size(
                pos2(rect.left(), rect.top() + PAD + ROW * row as f32),
                vec2(rect.width(), ROW),
            );
            if !ui.is_rect_visible(line) {
                continue;
            }
            // The leading end of the first row is the lane's menu, under the
            // glyph of the track's kind.
            let label = Rect::from_min_max(
                pos2(line.left() + HEAD_NAME - 2.0, line.top() + 1.0),
                pos2(line.right() - 4.0, line.bottom() - 1.0),
            );
            let lane = (animated.target, animated.key);
            let id = Id::new(("keyframe-row", clip.id, lane));
            let response = ui.interact(label, id, Sense::click());
            let name = &names[row].name;
            response.widget_info(|| {
                let label = format!("Select {name} keyframes");
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
            });
            if response.clicked() {
                self.select_lane(clip, Some(lane));
            }
            let painter = ui.painter();
            let hover = kit::hover_amount(ui, &response);
            if hover > 0.0 {
                painter.rect_filled(label, CornerRadius::same(5), p.hover.gamma_multiply(hover));
            }
            if response.has_focus() {
                controls::focus_ring(painter, label, 5, *p);
            }
            let ink = if response.has_focus() {
                p.fg
            } else {
                theme::mix(p.secondary, p.fg, hover)
            };
            let galley =
                widgets::truncated(ui, name, theme::regular(11.0), ink, label.width() - 8.0);
            painter.galley(
                pos2(
                    line.right() - 8.0 - galley.size().x,
                    label.center().y - galley.size().y * 0.5,
                ),
                galley,
                ink,
            );
            response.on_hover_ui(|ui| {
                ui.label(format!("Select the {name} keyframes"));
            });
        }
        let more = Rect::from_center_size(
            pos2(rect.left() + HEAD_GLYPH, rect.top() + PAD + ROW * 0.5),
            Vec2::splat(ROW),
        );
        let response = options_button(
            ui,
            p,
            more,
            Id::new(("keyframe-options", clip.id)),
            "Keyframe options",
        );
        egui::Popup::menu(&response).show(|ui| self.menu(ui, &sheet, actions));
    }

    // ---- The graph --------------------------------------------------------

    /// The graph of the selected clip's animated values over the timeline's
    /// own time axis, in `area`, with its legend in `legend`.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call site; the graph needs what the lanes do"
    )]
    pub(super) fn show_graph(
        &mut self,
        ui: &mut Ui,
        legend: Rect,
        area: Rect,
        geometry: &Geometry,
        selection: &mut Selection,
        input: &TimelineInput,
        actions: &mut Vec<Action>,
    ) {
        let p = input.p;
        let project = input.project;
        // The plot is the timeline's ground, as the lanes' is.
        ui.painter().rect_filled(area, CornerRadius::ZERO, p.bg);
        let found = (selection.single())
            .and_then(|id| project.clip(id))
            .filter(|(_, clip)| !clip.animation.is_empty());
        let Some((track, clip)) = found else {
            let (title, hint) = if selection.single().is_some() {
                ("No animated values", "Add a keyframe in the inspector.")
            } else {
                ("No clip selected", "Select one clip to see its curves.")
            };
            // Said in the plot alone: the legend's column keeps its place.
            // A graph under a short timeline says the first line.
            let hint = if area.height() >= 96.0 { hint } else { "" };
            kit::empty_state(ui, p, area, Icon::ChartLine, title, hint, None);
            return;
        };
        let names = self.names(project, track, clip, &clip.animation);
        let Some(shared) = shared(project, track) else {
            return;
        };

        // ---- What is sampled, and whether it has changed --------------------
        let fresh = (self.cache.as_ref()).is_some_and(|cache| {
            Arc::ptr_eq(&cache.track, &shared)
                && cache.clip == clip.id
                && cache.shown == self.shown_serial
        });
        let fit = match (&self.cache, fresh) {
            (Some(cache), true) => cache.fit,
            _ => self.fit(clip),
        };
        let view = self.view.unwrap_or(fit);
        self.showing = view;
        let inner = area.shrink2(vec2(0.0, 12.0f32.min(area.height() * 0.2)));
        let axis = Axis {
            bottom: inner.bottom(),
            low: view.0,
            rise: inner.height().max(1.0) / (view.1 - view.0).max(f32::EPSILON),
        };
        let sheet = self.sheet(input, geometry, track, clip, Some(axis));
        let current = fresh
            && (self.cache.as_ref()).is_some_and(|cache| {
                cache.rect == area
                    && cache.zoom == geometry.zoom
                    && cache.scroll == geometry.scroll
                    && cache.view == view
                    && cache.draft == self.draft_serial
            });
        if !current {
            let (lines, ticks) = self.sample(&sheet, area, inner);
            self.cache = Some(Cache {
                track: shared,
                clip: clip.id,
                shown: self.shown_serial,
                fit,
                rect: area,
                zoom: geometry.zoom,
                scroll: geometry.scroll,
                view,
                draft: self.draft_serial,
                lines,
                ticks,
            });
        }

        self.legend(ui, legend, &sheet, &names);

        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(area));
        ui.set_clip_rect(area);
        let ui = &mut ui;
        let ground = ui.interact(
            area,
            Id::new(("keyframe-graph", clip.id)),
            Sense::click_and_drag(),
        );
        let painter = ui.painter().clone();
        // Outside the clip there is nothing to key: its span is a band of a
        // row, as in the lane.
        let (start, end) = (sheet.x(Time::ZERO), sheet.x(clip.duration));
        let span = Rect::from_min_max(pos2(start, area.top()), pos2(end, area.bottom()));
        painter.rect_filled(span, CornerRadius::ZERO, p.lane());
        // The time grid stands under the ruler's figures.
        let (step, _) = rules::ruler_step(geometry.zoom, sheet.rate.fps(), 92.0);
        let first = (f64::from(geometry.scroll) / f64::from(geometry.zoom) / step).floor() as i64;
        let last = (f64::from(geometry.scroll + area.width()) / f64::from(geometry.zoom) / step)
            .ceil() as i64;
        for index in first..=last {
            let x = geometry.x(Time::from_seconds(index as f64 * step));
            let x = painter.round_to_pixel_center(x);
            painter.vline(x, area.y_range(), p.hairline());
        }
        if let Some(cache) = &self.cache {
            for (y, text) in &cache.ticks {
                let y = painter.round_to_pixel_center(*y);
                painter.hline(area.x_range(), y, p.hairline());
                if area.height() >= GRAPH_LABELLED {
                    painter.text(
                        pos2(area.left() + 6.0, y - 2.0),
                        Align2::LEFT_BOTTOM,
                        text,
                        theme::mono(11.0),
                        p.muted,
                    );
                }
            }
            // The curve of the parameter whose keys are selected is drawn
            // strong and the others step back; with none selected, every
            // curve is drawn alike.
            let chosen = |index: usize| {
                let track = &clip.animation[index];
                (self.selected.iter()).any(|key| key.lane() == (track.target, track.key))
            };
            let any = (cache.lines.iter()).any(|(index, _)| chosen(*index));
            for (index, points) in &cache.lines {
                let colour = p.curve(*index);
                let stroke = match (chosen(*index), any) {
                    (true, _) => Stroke::new(2.0, colour),
                    (false, true) => Stroke::new(1.5, colour.gamma_multiply(0.6)),
                    (false, false) => Stroke::new(1.5, colour),
                };
                painter.add(Shape::line(points.clone(), stroke));
            }
        }

        for (index, track) in clip.animation.iter().enumerate() {
            let lane = (track.target, track.key);
            if !self.visible(lane) {
                continue;
            }
            let colour = p.curve(index);
            let held = matches!(self.drag, Some(Drag::Keys { moved: true, .. }));
            let reach = geometry.span(SNAP_POINTS + 4.0);
            let (from, to) = (
                geometry.time(area.left()) - clip.start - reach,
                geometry.time(area.right()) - clip.start + reach,
            );
            // The handles of the segments on either side of a selected key.
            for segment in 0..track.keys.len().saturating_sub(1) {
                let (k0, k1) = (&track.keys[segment], &track.keys[segment + 1]);
                let (r0, r1) = (KeyRef::of(track, k0), KeyRef::of(track, k1));
                if !(self.selected.contains(&r0) || self.selected.contains(&r1)) {
                    continue;
                }
                let Some([x1, y1, x2, y2]) = controls(self.ease_of(&r0, k0.ease)) else {
                    continue;
                };
                let (t0, v0) = self.placed(&sheet, &r0, k0.value);
                let (t1, v1) = self.placed(&sheet, &r1, k1.value);
                let (a, b) = (pos2(sheet.x(t0), axis.y(v0)), pos2(sheet.x(t1), axis.y(v1)));
                let at = |x: f32, y: f32| pos2(a.x + (b.x - a.x) * x, a.y + (b.y - a.y) * y);
                for (second, anchor, spot) in [(false, a, at(x1, y1)), (true, b, at(x2, y2))] {
                    self.handle(
                        ui,
                        &sheet,
                        &names[index].name,
                        (segment, second),
                        (r0, k0.ease, k1.at),
                        (anchor, spot),
                        colour,
                        actions,
                    );
                }
            }
            let run = if held {
                0..track.keys.len()
            } else {
                track.keys.partition_point(|key| key.at < from)
                    ..track.keys.partition_point(|key| key.at <= to)
            };
            for key_index in run {
                let key = &track.keys[key_index];
                let at = KeyRef::of(track, key);
                let (time, value) = self.placed(&sheet, &at, key.value);
                let center = pos2(sheet.x(time), axis.y(value));
                self.key(
                    ui,
                    &sheet,
                    &names[index].name,
                    key_index,
                    at,
                    key,
                    center,
                    selection,
                    actions,
                );
            }
        }
        self.gesture(ui, &sheet, &ground, actions);
        self.end(clip.id, selection);
        self.paint_marquee(ui, p, area);
        ground.context_menu(|ui| self.menu(ui, &sheet, actions));
        self.corner(ui, area, &sheet, actions);
    }

    /// The plot's own actions at its top-trailing corner: fitting the values
    /// to the curves again, and the menu of the selected keys. They stand on
    /// a patch of the ground, so no curve runs under them, and over the
    /// keys, so a press there is theirs.
    fn corner(&mut self, ui: &mut Ui, area: Rect, sheet: &Sheet, actions: &mut Vec<Action>) {
        let p = sheet.p;
        let size = vec2(28.0 * 2.0 + 2.0, 28.0);
        let rect = Rect::from_min_size(pos2(area.right() - 6.0 - size.x, area.top() + 4.0), size);
        ui.painter()
            .rect_filled(rect.expand(2.0), CornerRadius::same(9), p.bg);
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
        ui.add_enabled_ui(self.view.is_some(), |ui| {
            if icons::button(ui, Icon::FitValues, "Fit values").clicked() {
                self.view = None;
            }
        });
        let more = icons::button(&mut ui, Icon::Dots, "Keyframe options");
        egui::Popup::menu(&more).show(|ui| self.menu(ui, sheet, actions));
    }

    /// The span of values the visible curves take, with room around it.
    fn fit(&self, clip: &Clip) -> (f32, f32) {
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut take = |value: f32| {
            low = low.min(value);
            high = high.max(value);
        };
        for track in &clip.animation {
            if !self.visible((track.target, track.key)) {
                continue;
            }
            for pair in track.keys.windows(2) {
                take(pair[0].value);
                // An ease can carry a value past both of its keys.
                if matches!(pair[0].ease, Ease::Bezier { .. }) {
                    let span = pair[1].at.0 - pair[0].at.0;
                    for step in 1..16 {
                        take(track.sample(Time(pair[0].at.0 + span * step / 16)));
                    }
                }
            }
            if let Some(last) = track.keys.last() {
                take(last.value);
            }
        }
        if !(low.is_finite() && high.is_finite()) {
            return (0.0, 1.0);
        }
        let span = high - low;
        let margin = if span > f32::EPSILON * high.abs().max(1.0) {
            span * 0.12
        } else {
            (high.abs() * 0.1).max(0.1)
        };
        (low - margin, high + margin)
    }

    /// The visible curves over the part of the clip that is in view, read
    /// off the tracks' own evaluation, and the value axis beside them.
    #[expect(
        clippy::type_complexity,
        reason = "the two halves of the cache, made together"
    )]
    fn sample(
        &mut self,
        sheet: &Sheet,
        area: Rect,
        inner: Rect,
    ) -> (Vec<(usize, Vec<Pos2>)>, Vec<(f32, String)>) {
        self.samplings += 1;
        let Some(axis) = sheet.axis else {
            return (Vec::new(), Vec::new());
        };
        let clip = sheet.clip;
        let mut lines = Vec::new();
        let mut only = None;
        let mut count = 0;
        for (index, stored) in clip.animation.iter().enumerate() {
            let lane = (stored.target, stored.key);
            if !self.visible(lane) {
                continue;
            }
            count += 1;
            only = Some(lane);
            let track = (self.draft.iter())
                .find(|draft| (draft.target, draft.key) == lane)
                .unwrap_or(stored);
            if track.keys.is_empty() {
                continue;
            }
            let left = sheet.x(Time::ZERO).max(area.left() - SAMPLE);
            let right = sheet.x(clip.duration).min(area.right() + SAMPLE);
            let mut points = Vec::with_capacity(((right - left) / SAMPLE).max(0.0) as usize + 8);
            let local = |x: f32| sheet.geometry.time(x) - clip.start;
            // From one key to the next, so a corner and a held step are
            // drawn where they are and not between two samples.
            let mut stops = vec![(left, local(left))];
            stops.extend(
                (track.keys.iter())
                    .map(|key| (sheet.x(key.at), key.at))
                    .filter(|(x, _)| *x > left && *x < right),
            );
            stops.push((right, local(right)));
            for pair in stops.windows(2) {
                let ((from, start), (to, end)) = (pair[0], pair[1]);
                points.push(pos2(from, axis.y(track.sample(start))));
                let mut x = from + SAMPLE;
                while x < to {
                    points.push(pos2(x, axis.y(track.sample(local(x)))));
                    x += SAMPLE;
                }
                let before = Time((end.0 - 1).max(start.0));
                points.push(pos2(to, axis.y(track.sample(before))));
            }
            if let Some(&(x, end)) = stops.last() {
                points.push(pos2(x, axis.y(track.sample(end))));
            }
            lines.push((index, points));
        }

        // One curve alone is read in its own unit; several share bare numbers.
        let unit = (only.filter(|_| count == 1))
            .and_then(|lane| Some((target_of(clip, lane.0)?, lane.1)))
            .and_then(|(target, key)| {
                Some(inspector::param_unit(
                    target,
                    look::param_spec(target, key)?,
                ))
            });
        let (scale, suffix) = unit.unwrap_or((1.0, ""));
        let span = (axis.value(inner.top()) - axis.low) * scale;
        let step = tick(span.max(f32::EPSILON), inner.height());
        let decimals = (-step.log10().floor()).clamp(0.0, 4.0) as usize;
        let mut ticks = Vec::new();
        let (low, high) = (axis.value(area.bottom()), axis.value(area.top()));
        let mut value = (low * scale / step).ceil() * step;
        while value <= high * scale && ticks.len() < 64 {
            // No "-0" from a sum of steps.
            let shown = if value.abs() < step * 1e-3 {
                0.0
            } else {
                value
            };
            let text = format!("{shown:.decimals$}{suffix}");
            ticks.push((axis.y(value / scale), text));
            value += step;
        }
        (lines, ticks)
    }

    /// One easing handle of the segment leaving `leaving.0`.
    #[expect(
        clippy::too_many_arguments,
        reason = "a handle needs its segment, its place and its name"
    )]
    fn handle(
        &mut self,
        ui: &mut Ui,
        sheet: &Sheet,
        name: &str,
        (segment, second): (usize, bool),
        (at, stored, next): (KeyRef, Ease, Time),
        (anchor, spot): (Pos2, Pos2),
        colour: Color32,
        actions: &mut Vec<Action>,
    ) {
        let p = sheet.p;
        let rect = Rect::from_center_size(spot, Vec2::splat(13.0));
        let id = Id::new((
            "keyframe-ease",
            sheet.clip.id,
            at.target,
            at.key,
            segment,
            second,
        ));
        let sense = if sheet.locked {
            Sense::hover()
        } else {
            Sense::click_and_drag()
        };
        let response = ui.interact(rect, id, sense);
        response.widget_info(|| {
            let label = if second {
                format!("{name} ease into {}", sheet.timecode(next))
            } else {
                format!("{name} ease out of {}", sheet.timecode(at.at))
            };
            egui::WidgetInfo::labeled(egui::WidgetType::DragValue, !sheet.locked, label)
        });
        let (pressed, pointer) =
            ui.input(|i| (i.pointer.primary_pressed(), i.pointer.interact_pos()));
        if pressed && response.is_pointer_button_down_on() && !sheet.locked {
            response.request_focus();
            self.drag = Some(Drag::Handle {
                at,
                second,
                ease: stored,
                from: pointer.unwrap_or(spot),
                moved: false,
            });
        }
        if response.has_focus() {
            self.focus_now = Focused::Handle;
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
            // The arrows move the handle a step; each is an edit of its own.
            let step = ui.input(|i| {
                let mut step = Vec2::ZERO;
                for event in &i.events {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                        && !modifiers.command
                        && !modifiers.alt
                    {
                        match key {
                            egui::Key::ArrowLeft => step.x -= HANDLE_STEP,
                            egui::Key::ArrowRight => step.x += HANDLE_STEP,
                            egui::Key::ArrowUp => step.y += HANDLE_STEP,
                            egui::Key::ArrowDown => step.y -= HANDLE_STEP,
                            _ => {}
                        }
                    }
                }
                step
            });
            if step != Vec2::ZERO
                && self.drag.is_none()
                && let Some([x1, y1, x2, y2]) = controls(stored)
            {
                let (x, y) = if second { (x2, y2) } else { (x1, y1) };
                let ease = with_control(stored, second, x + step.x, y + step.y);
                let tracks = eased(sheet.clip, std::slice::from_ref(&at), ease);
                actions.extend(edit(sheet, "Change easing", tracks));
            }
        }
        let painter = ui.painter();
        let lit = response.hovered() || response.has_focus() || {
            matches!(&self.drag, Some(Drag::Handle { at: held, second: which, .. })
                if *held == at && *which == second)
        };
        // A quiet arm from the key, and a ring in the curve's colour that
        // fills while it is under the pointer, held or has the keyboard.
        painter.line_segment([anchor, spot], Stroke::new(1.0, theme::tint(p.fg, 0.5)));
        painter.circle(
            spot,
            HANDLE,
            if lit { colour } else { p.bg },
            Stroke::new(1.5, colour),
        );
        if response.has_focus() {
            painter.circle_stroke(spot, HANDLE + 3.0, ring(p));
        }
        if !sheet.locked && self.drag.is_none() && response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
    }

    /// The graph's legend, in the heads' column and in a head's card: a row
    /// for each parameter with its curve's colour and name, which selects
    /// its keys, and the toggles that show its curve or show it alone.
    fn legend(&mut self, ui: &mut Ui, rect: Rect, sheet: &Sheet, names: &[Named]) {
        let p = sheet.p;
        let clip = sheet.clip;
        // Inset as a track head's card is from its row, and as tall as its
        // rows; what does not fit scrolls inside it.
        let tall = clip.animation.len() as f32 * LEGEND_ROW + 8.0;
        let card = Rect::from_min_max(
            pos2(rect.left() + 6.0, rect.top() + 4.0),
            pos2(
                rect.right() - 4.0,
                (rect.top() + 4.0 + tall).min(rect.bottom() - 4.0),
            ),
        );
        if !card.is_positive() {
            return;
        }
        ui.painter()
            .rect_filled(card, CornerRadius::same(8), p.control);
        let inner = card.shrink(4.0);
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        ui.set_clip_rect(inner.expand(3.0).intersect(rect));
        let ui = &mut ui;
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        egui::ScrollArea::vertical()
            .id_salt(("keyframe-legend", clip.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, track) in clip.animation.iter().enumerate() {
                    let lane = (track.target, track.key);
                    let (row, _) = ui.allocate_exact_size(
                        vec2(ui.available_width(), LEGEND_ROW),
                        Sense::hover(),
                    );
                    self.legend_row(ui, p, row, clip, index, lane, &names[index]);
                }
            });
    }

    /// One parameter of the legend in `row`: its toggles at the trailing
    /// end, as a head's cells are, and before them its colour and name.
    #[expect(
        clippy::too_many_arguments,
        reason = "a row needs its place, its parameter and what it is called"
    )]
    fn legend_row(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        row: Rect,
        clip: &Clip,
        index: usize,
        lane: Lane,
        named: &Named,
    ) {
        let Named { name, show, solo } = named;
        let cell = |from_end: f32| {
            let right = row.right() - from_end * LEGEND_STEP;
            Rect::from_min_max(
                pos2(right - LEGEND_CELL, row.center().y - LEGEND_CELL * 0.5),
                pos2(right, row.center().y + LEGEND_CELL * 0.5),
            )
        };
        let (eye, alone_cell) = (cell(1.0), cell(0.0));
        let shown = !self.hidden.contains(&lane);
        let toggle = |ui: &mut Ui, at: Rect, icon: Icon, on: bool, label: &str| {
            let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(at));
            widgets::small_icon_toggle(&mut ui, p, icon, on, label).clicked()
        };
        if toggle(
            ui,
            eye,
            if shown { Icon::Eye } else { Icon::EyeOff },
            shown,
            show,
        ) {
            if shown {
                self.hidden.push(lane);
            } else {
                self.hidden.retain(|hidden| *hidden != lane);
            }
            self.shown_serial += 1;
        }
        let alone = self.solo == Some(lane);
        if toggle(ui, alone_cell, Icon::Solo, alone, solo) {
            self.solo = (!alone).then_some(lane);
            self.shown_serial += 1;
        }

        let label = Rect::from_min_max(
            pos2(row.left(), eye.top()),
            pos2(eye.left() - 2.0, eye.bottom()),
        );
        let response = ui.interact(
            label,
            Id::new(("keyframe-legend-row", clip.id, lane)),
            Sense::click(),
        );
        response.widget_info(|| {
            let label = format!("Select {name} keyframes");
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
        });
        if response.clicked() {
            self.select_lane(clip, Some(lane));
        }
        if !ui.is_rect_visible(label) {
            return;
        }
        let painter = ui.painter();
        let hover = kit::hover_amount(ui, &response);
        if hover > 0.0 {
            painter.rect_filled(label, CornerRadius::same(6), p.hover.gamma_multiply(hover));
        }
        if response.has_focus() {
            controls::focus_ring(painter, label, 6, *p);
        }
        // A curve that is not drawn keeps its row, quieter.
        let dim = !self.visible(lane);
        let colour = p.curve(index);
        let dot = pos2(row.left() + HEAD_GLYPH - 4.0, row.center().y);
        painter.circle_filled(
            dot,
            4.0,
            if dim {
                colour.gamma_multiply(0.35)
            } else {
                colour
            },
        );
        let ink = if dim { p.muted } else { p.fg };
        let left = row.left() + HEAD_NAME - 4.0;
        let galley = widgets::truncated(
            ui,
            name,
            theme::regular(11.5),
            ink,
            label.right() - 4.0 - left,
        );
        painter.galley(
            pos2(left, row.center().y - galley.size().y * 0.5),
            galley,
            ink,
        );
    }
}

fn edit(sheet: &Sheet, label: &str, tracks: Vec<AnimTrack>) -> Option<Action> {
    command(sheet.clip, label, tracks).map(Action::Edit)
}

/// The step a dragged value lands on: the finest power of ten that a point
/// of a graph `rise` points high for each unit tells apart, and no coarser
/// than a hundredth of the `range` the value has.
fn unit(rise: f32, range: Option<(f32, f32)>) -> f32 {
    let seen = 10f32.powf((1.0 / rise).log10().floor());
    let Some((low, high)) = range.filter(|(low, high)| high > low) else {
        return seen;
    };
    seen.min(10f32.powf(((high - low) / 100.0).log10().floor()))
}

fn moved_tracks(clip: &Clip, picked: &[KeyRef], (delta, lift): (Time, f32)) -> Vec<AnimTrack> {
    moved(clip, picked, delta, lift)
}

/// The clips selected this frame, as the tiles' chevrons read them: a
/// chevron takes the ink of the name it stands before.
fn chosen_id() -> Id {
    Id::new("keyframe-lane-chosen")
}

/// The chevron on a clip's tile that opens and closes its lane.
pub(super) fn disclosure(ui: &mut Ui, p: &Palette, rect: Rect, clip: &Clip, open: bool) -> bool {
    let response = ui.interact(
        rect,
        Id::new(("keyframe-lane-toggle", clip.id)),
        Sense::click(),
    );
    response.widget_info(|| {
        let label = format!("Keyframes of {}", clip.name);
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, label);
        info.selected = Some(open);
        info
    });
    // The name's ink: on the accent while the clip is selected; white on a
    // dark tile's band and ink on a light one.
    let selected = ui.ctx().data(|data| {
        data.get_temp::<Arc<[ClipId]>>(chosen_id())
            .is_some_and(|chosen| chosen.contains(&clip.id))
    });
    let ink = if selected {
        p.on_accent
    } else if p.dark {
        Color32::WHITE
    } else {
        p.fg
    };
    let painter = ui.painter();
    let hover = kit::hover_amount(ui, &response);
    if hover > 0.0 {
        painter.rect_filled(rect, CornerRadius::same(4), theme::tint(ink, 0.2 * hover));
    }
    icons::paint(
        painter,
        Rect::from_center_size(rect.center(), Vec2::splat(11.0)),
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        ink.gamma_multiply(0.8 + 0.2 * hover),
    );
    if response.has_focus() {
        controls::focus_ring(painter, rect, 4, *p);
    }
    let response = response.on_hover_cursor(CursorIcon::Default);
    let name = if open {
        "Hide keyframes"
    } else {
        "Show keyframes"
    };
    icons::hint(response, name, "U").clicked()
}

/// The lane's menu, at the head of its first row: a chevron that rests
/// without a surface, as a quiet pop-up does, and is lit under the pointer
/// and pressed while its menu is open.
fn options_button(ui: &mut Ui, p: &Palette, rect: Rect, id: Id, label: &str) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hover = kit::hover_amount(ui, &response);
        if open || response.is_pointer_button_down_on() {
            painter.rect_filled(rect, CornerRadius::same(6), p.pressed);
        } else if hover > 0.0 {
            painter.rect_filled(rect, CornerRadius::same(6), p.hover.gamma_multiply(hover));
        }
        if response.has_focus() {
            controls::focus_ring(painter, rect, 6, *p);
        }
        let ink = if open || response.has_focus() {
            p.fg
        } else {
            theme::mix(p.muted, p.fg, hover)
        };
        icons::paint(
            painter,
            Rect::from_center_size(rect.center() + vec2(0.0, 0.5), Vec2::splat(10.0)),
            Icon::ChevronDown,
            ink,
        );
    }
    icons::hint(
        response.on_hover_cursor(CursorIcon::PointingHand),
        label,
        "",
    )
}

/// The four corners of a key's diamond, `radius` from its centre.
fn diamond(center: Pos2, radius: f32) -> Vec<Pos2> {
    vec![
        center - vec2(radius, 0.0),
        center - vec2(0.0, radius),
        center + vec2(radius, 0.0),
        center + vec2(0.0, radius),
    ]
}

/// The ring round a key or a handle that has the keyboard: the focus ring
/// the controls wear, drawn round a point.
fn ring(p: &Palette) -> Stroke {
    Stroke::new(2.0, theme::tint(p.accent, 0.55))
}

/// A line of a menu that says why its rows below cannot be used, where a
/// heading would stand, and is read as one.
fn menu_note(ui: &mut Ui, p: &Palette, text: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    ui.painter().text(
        pos2(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        theme::medium(11.5),
        p.secondary,
    );
}

#[cfg(test)]
#[path = "keyframes_tests.rs"]
mod tests;
