//! The timeline: a tool strip, track heads, a ruler and lanes of clips.
//!
//! Lanes are hit-tested by hand from one response rather than made of a
//! widget per clip, so a long project costs only what is in view. A held drag
//! is drawn as a preview and becomes one command when it is released.
//!
//! A clip's keyframes are edited here too, in a lane under its track or in
//! the graph that takes the tracks' place: see [`super::keyframes`].
//!
//! A transition is a marking on the edge of its clip's tile, as wide as it
//! is long. Its inner end is a handle that changes how long it lasts, a cut
//! or a free edge under the pointer offers to add one, and a tile of the
//! library let go over a clip is given to that clip.
//!
//! A tile on an audio lane draws its media's sound as a waveform once the
//! peaks of that media are held: see [`crate::peaks`]. Only the tiles drawn
//! ask for them.
//!
//! The fades of a clip on an audio lane are drawn over its sound as the
//! curve they follow. A selected tile, or the one under the pointer, has a
//! handle at each top corner of its sound that sets how long that edge
//! fades: see [`super::fades`].
//!
//! A tile on an audio lane also carries a line at its clip's volume, bent by
//! the volume's keyframes. On a selected tile the line of a clip without
//! such keyframes is dragged up and down to set the volume, and Alt-click on
//! a line keys the volume there and opens the keyframe lane.
//!
//! A tile on an audio lane whose media's beats are shown carries a tick at
//! each beat where it is heard, and while snapping is on the beats of the
//! tiles drawn are places an edge, a trim, a fade or the playhead lands on:
//! see [`crate::peaks::HeldBeats`].
use super::{
    Action, Selection,
    browser::{self, Entry, EntryDrag},
    controls,
    fades::{self, FadeEdge},
    icon_of_track,
    keyframes::{self, KeyframesState},
    kit, shortcut,
    text_tab::{self, LookDrag},
    tone_of_track, tracks, transitions,
    widgets::{self, icon_button, icon_toggle, menu_item, menu_style},
};
use crate::{
    duck,
    icons::{self, Icon},
    normalize,
    peaks::{self, HeldBeats, HeldPeaks},
    rules::{self, Edge},
    speed,
    textures::Textures,
    theme::{self, Palette, metrics},
    timefmt,
};
use editor_core::{
    Asset, AssetId, Clip, ClipContent, ClipId, Command, Marker, ParamKey, Project, Time, Track,
    TrackId, TrackKind, Transition, TransitionEdge, gain_at, look, snap_time, sound,
};
use editor_media::{BeatGrid, MediaService, Peaks};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};

const STRIP: f32 = metrics::PANE_HEADER;
const RULER: f32 = 24.0;
const HEAD_WIDTH: f32 = 156.0;
/// The side of a toggle in a track head, and how far one stands from the
/// next.
const HEAD_CELL: f32 = 22.0;
const HEAD_CELL_STEP: f32 = 23.0;
/// The most toggles a head has side by side in one row. A fourth would leave
/// the name of a 156-point head twenty points.
const HEAD_ROW: usize = 3;
/// Where a track's name starts in its head, after the icon of its kind.
const HEAD_NAME: f32 = 28.0;
/// The lowest head that says its track's volume under the name.
const HEAD_FIGURE: f32 = 40.0;
const LANE_GAP: f32 = 4.0;
const EDGE_GRIP: f32 = 7.0;
const SNAP_POINTS: f32 = 8.0;
/// How near two beats may be drawn: one closer than this to another is
/// neither drawn nor snapped to.
const BEAT_GAP: f32 = 4.0;
/// How tall a beat's tick is, from the foot of its tile's sound.
const BEAT_TICK: f32 = 7.0;
/// The least a transition's marking is drawn wide, however short it is at
/// the zoom in view.
const MARK_MIN: f32 = 6.0;
/// The width of the handle at a marking's inner end.
const MARK_HANDLE: f32 = 10.0;
/// The narrowest tile of a sound lane that carries the handles of its fades:
/// room for both between the grips that trim it, and some of the tile to
/// take hold of beside them.
const FADE_TILE: f32 = 40.0;
/// How tall the handle of a fade is.
const FADE_HANDLE: f32 = 12.0;
/// How far apart the points of a fade's curve are drawn.
const FADE_STEP: f32 = 2.0;
/// How much of the height of a tile's sound a volume of 100% stands at, and
/// the volume at its top. A higher volume is drawn at the top.
const VOLUME_WHOLE: f32 = 2.0 / 3.0;
const VOLUME_TOP: f32 = 2.0;
/// How far above or under the volume line a press takes hold of it.
const VOLUME_REACH: f32 = 4.0;
/// How far the pointer travels before a press on the line changes the
/// volume.
const VOLUME_TRAVEL: f32 = 3.0;
/// How far the percent of a held volume line stands from the point it was
/// taken hold of, and from a fade, a fade's handle or a grip it keeps clear
/// of.
const READOUT_GAP: f32 = 6.0;
/// How near an edge the pointer offers to add a transition there, and the
/// narrowest tile it is offered on.
const ADD_REACH: f32 = 14.0;
const ADD_TILE: f32 = 36.0;
const ADD_BADGE: f32 = 16.0;
/// The height of the row a tall tile writes its clip's name in.
const NAME_ROW: f32 = 18.0;
/// How much further than eight points in a tile's name starts with no
/// chevron before it: clear of the trim bar at the tile's start, with room
/// between them for a selected name's capsule.
const NAME_STEP: f32 = 4.0;
/// How far the pointer travels before a press on the badge is a drag of the
/// clip under it.
const OFFER_TRAVEL: f32 = 3.0;
/// Lanes taller than this carry a poster at the head of their clips, under
/// the name.
const POSTER_LANE: f32 = 40.0;
pub const MIN_ZOOM: f32 = 0.5;
pub const MAX_ZOOM: f32 = 640.0;

/// The payload of a library item dragged toward the timeline.
#[derive(Clone, Copy, Debug)]
pub struct AssetDrag(pub AssetId);

#[derive(Clone, Debug)]
enum Drag {
    Scrub,
    Move {
        grabbed: ClipId,
        /// Time under the pointer when the clip was taken.
        origin: Time,
        delta: Time,
        target: Option<TrackId>,
        /// Whether the pointer has travelled: a press that never moves is a
        /// click and changes nothing.
        moved: bool,
    },
    Trim {
        clip: ClipId,
        edge: Edge,
        delta: Time,
    },
    /// The inner end of a transition's marking, and how long the transition
    /// would be where it is held.
    Transition {
        clip: ClipId,
        edge: TransitionEdge,
        duration: Time,
    },
    /// The handle of a fade of a clip's sound, and how long the fade would
    /// be where it is held.
    Fade {
        clip: ClipId,
        edge: FadeEdge,
        duration: Time,
        /// How far past the end of its fade the handle was taken hold of.
        /// A handle stands clear of the grips, so that of a short fade is
        /// not where the fade ends: the fade follows the travel.
        grab: Time,
    },
    /// The volume line of a clip whose volume has no keyframes, and the
    /// volume the clip would have where the line is held.
    Volume {
        clip: ClipId,
        gain: f32,
        /// The volume the clip had when the line was taken hold of.
        stored: f32,
        /// How high the line stood in the tile's sound when it was taken
        /// hold of, and how high that sound is drawn.
        level: f32,
        height: f32,
        /// How high the pointer was then: the line follows the travel.
        from: f32,
        /// The time of the project the line was taken hold of at: the held
        /// percent is said beside it.
        at: Time,
        /// A press that never travels is a click and changes nothing.
        moved: bool,
    },
    Marquee {
        from: Pos2,
        /// What was selected before, kept while a modifier is held.
        base: Vec<ClipId>,
    },
}

pub struct TimelineState {
    /// Points per second.
    pub zoom: f32,
    pub scroll: Vec2,
    drag: Option<Drag>,
    /// The time a held drag is resting against, drawn as a guide.
    snap_guide: Option<Time>,
    /// Fit the whole project in view on the next frame.
    pub fit: bool,
    /// The clip a context menu was opened on.
    menu_clip: Option<ClipId>,
    menu_time: Time,
    lane_width: f32,
    /// The keyframe lane and the graph: which is open, and the keys held.
    pub keys: KeyframesState,
    /// The transition whose marking is selected: the inspector shows it.
    transition: Option<(ClipId, TransitionEdge)>,
    /// The edge whose badge was pressed, and where: let go there, it is
    /// given a transition.
    offer: Option<(ClipId, TransitionEdge, Pos2)>,
    /// The badge the last frame drew, and whose edge it offered for: a press
    /// is the badge's only where one was there to be seen.
    badge: Option<(ClipId, TransitionEdge, Rect)>,
    /// The clip the keyboard selected last: it carries the focus ring until
    /// the pointer presses the tracks or the clip is let go.
    keyboard: Option<ClipId>,
    /// A clip to bring into view when the tracks are next laid out.
    reveal: Option<ClipId>,
    /// The beats the tiles of this frame show, by clip, where they fall on
    /// the timeline: what snapping takes besides edges and markers.
    beats: Vec<(ClipId, Time)>,
}

/// The clip a key asks for, from the one selected and the playhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// The topmost clip under the playhead; asked again, the next one down.
    AtPlayhead,
    /// The clip before or after the selected one on its track.
    Previous,
    Next,
    /// The clip nearest the playhead on the next track up or down that has
    /// one, in the order the tracks are shown.
    Above,
    Below,
}

/// The clip `reach` asks for, or `None` when there is none that way. `from`
/// is the clip selected last; without one every reach is of the playhead.
pub fn reach(
    project: &Project,
    from: Option<ClipId>,
    playhead: Time,
    reach: Reach,
) -> Option<ClipId> {
    let order = display_order(project);
    let under = |track: &Track| {
        track
            .clips
            .iter()
            .find(|clip| clip.start <= playhead && playhead < clip.end())
            .map(|clip| clip.id)
    };
    let stacked: Vec<ClipId> = order.iter().filter_map(|track| under(track)).collect();
    let at_playhead = |after: Option<ClipId>| {
        let next = after
            .and_then(|id| stacked.iter().position(|other| *other == id))
            .map_or(0, |index| (index + 1) % stacked.len().max(1));
        stacked.get(next).copied()
    };
    let held = from.and_then(|id| {
        let row = order
            .iter()
            .position(|track| track.clips.iter().any(|clip| clip.id == id))?;
        let clip = order[row].clips.iter().find(|clip| clip.id == id)?;
        Some((row, clip))
    });
    let Some((row, clip)) = held else {
        // Nothing to start from: what is under the playhead, or failing
        // that the clip that starts nearest it in the direction asked for.
        let starts = || order.iter().flat_map(|track| &track.clips);
        return at_playhead(None).or_else(|| match reach {
            Reach::Previous => starts()
                .filter(|clip| clip.start <= playhead)
                .max_by_key(|clip| clip.start)
                .map(|clip| clip.id),
            Reach::Next => starts()
                .filter(|clip| clip.start >= playhead)
                .min_by_key(|clip| clip.start)
                .map(|clip| clip.id),
            _ => None,
        });
    };
    match reach {
        Reach::AtPlayhead => at_playhead(from),
        Reach::Previous => (order[row].clips.iter())
            .filter(|other| other.start < clip.start)
            .max_by_key(|other| other.start)
            .map(|other| other.id),
        Reach::Next => (order[row].clips.iter())
            .filter(|other| other.start > clip.start)
            .min_by_key(|other| other.start)
            .map(|other| other.id),
        Reach::Above | Reach::Below => {
            // Measured from the playhead while it is on the clip, so a
            // column of clips is walked without drifting sideways.
            let on = clip.start <= playhead && playhead < clip.end();
            let at = if on { playhead } else { clip.start };
            let away = |other: &Clip| {
                // A clip that only touches the time is not on it.
                if other.start <= at && at < other.end() {
                    0
                } else {
                    1 + other
                        .start
                        .0
                        .abs_diff(at.0)
                        .min(other.end().0.abs_diff(at.0))
                }
            };
            let nearest = |track: &&Track| {
                (track.clips.iter())
                    .min_by_key(|other| (away(other), other.start))
                    .map(|other| other.id)
            };
            if reach == Reach::Above {
                order[..row].iter().rev().find_map(nearest)
            } else {
                order[row + 1..].iter().find_map(nearest)
            }
        }
    }
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            zoom: 80.0,
            scroll: Vec2::ZERO,
            drag: None,
            snap_guide: None,
            fit: false,
            menu_clip: None,
            menu_time: Time::ZERO,
            lane_width: 800.0,
            keys: KeyframesState::default(),
            transition: None,
            offer: None,
            badge: None,
            keyboard: None,
            reveal: None,
            beats: Vec::new(),
        }
    }
}

impl TimelineState {
    pub fn cancel_drag(&mut self) -> bool {
        self.snap_guide = None;
        self.offer = None;
        // Not `||`: a held clip and held keys are both let go.
        self.drag.take().is_some() | self.keys.cancel()
    }

    /// Marks `clip` as the one the keyboard selected: it is brought into
    /// view and carries the focus ring.
    pub fn point_at(&mut self, clip: ClipId) {
        self.keyboard = Some(clip);
        self.reveal = Some(clip);
    }

    /// Lets go of the selected marking of a transition, and says whether
    /// one was selected: what an Escape does when nothing is held.
    pub fn unmark(&mut self) -> bool {
        self.transition.take().is_some()
    }

    /// The transition whose marking is selected, while its clip is the one
    /// selected and still has it.
    pub fn marked(
        &self,
        project: &Project,
        selection: &Selection,
    ) -> Option<(ClipId, TransitionEdge)> {
        let (id, edge) = self.transition?;
        let (_, clip) = project.clip(id)?;
        (selection.single() == Some(id) && transitions::of(clip, edge).is_some())
            .then_some((id, edge))
    }

    /// Selects the marking of a transition, and with it the clip.
    pub fn mark(&mut self, selection: &mut Selection, clip: ClipId, edge: TransitionEdge) {
        selection.only(clip);
        self.transition = Some((clip, edge));
        self.keys.let_go();
    }

    /// Zooms by `factor`, keeping the time at `anchor` points from the lanes'
    /// leading edge where it is.
    pub fn zoom_about(&mut self, factor: f32, anchor: f32) {
        let seconds = (self.scroll.x + anchor) / self.zoom;
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.scroll.x = (seconds * self.zoom - anchor).max(0.0);
    }

    pub fn zoom_centered(&mut self, factor: f32) {
        self.zoom_about(factor, self.lane_width * 0.5);
    }

    /// Brings `time` into view, a page at a time, as a playing timeline does.
    pub fn follow(&mut self, time: Time) {
        let x = time.as_seconds() as f32 * self.zoom - self.scroll.x;
        if x > self.lane_width - 24.0 {
            self.scroll.x += self.lane_width * 0.85;
        } else if x < 0.0 {
            self.scroll.x = (time.as_seconds() as f32 * self.zoom - 24.0).max(0.0);
        }
    }
}

pub struct TimelineInput<'a> {
    pub p: &'a Palette,
    pub project: &'a Project,
    pub playhead: Time,
    pub snapping: bool,
    pub media: Option<&'a MediaService>,
    pub can_paste: bool,
    /// How long a frame frozen from here is held.
    pub freeze_hold: Time,
    /// Whether this session has a stem separator, for "Separate stems".
    pub separator: crate::stems::Tool,
}

struct Row<'a> {
    track: &'a Track,
    top: f32,
    height: f32,
    /// The height of the keyframe lane open under the track's clips.
    lane: f32,
}

/// Maps between time and lane coordinates.
#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub(super) lanes: Rect,
    pub(super) zoom: f32,
    pub(super) scroll: f32,
}

impl Geometry {
    pub(super) fn x(&self, time: Time) -> f32 {
        self.lanes.left() + (time.as_seconds() * f64::from(self.zoom)) as f32 - self.scroll
    }
    pub(super) fn time(&self, x: f32) -> Time {
        Time::from_seconds(f64::from(x - self.lanes.left() + self.scroll) / f64::from(self.zoom))
            .max(Time::ZERO)
    }
    pub(super) fn span(&self, points: f32) -> Time {
        Time::from_seconds(f64::from(points) / f64::from(self.zoom))
    }
}

fn lane_height(kind: TrackKind) -> f32 {
    match kind {
        TrackKind::Video => 58.0,
        TrackKind::Audio => 44.0,
        TrackKind::Text => 34.0,
    }
}

/// Tracks in the order they are shown: pictures top-down, the topmost layer
/// first, then sound.
pub fn display_order(project: &Project) -> Vec<&Track> {
    let visual = project
        .tracks
        .iter()
        .rev()
        .filter(|track| track.kind != TrackKind::Audio);
    let audio = project
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Audio);
    visual.chain(audio).map(AsRef::as_ref).collect()
}

/// Rows from `top` down, in the order given.
#[cfg(test)]
fn lay_out(order: Vec<&Track>, top: f32) -> Vec<Row<'_>> {
    lay_out_with(order, top, None)
}

/// Rows from `top` down, in the order given, with a keyframe lane of this
/// height under the clips of one track.
fn lay_out_with(order: Vec<&Track>, mut top: f32, lane: Option<(TrackId, f32)>) -> Vec<Row<'_>> {
    let mut rows = Vec::with_capacity(order.len());
    for track in order {
        let height = lane_height(track.kind);
        let lane = lane
            .filter(|(id, _)| *id == track.id)
            .map_or(0.0, |lane| lane.1);
        rows.push(Row {
            track,
            top,
            height,
            lane,
        });
        top += height + lane + LANE_GAP;
    }
    rows
}

fn carries_poster(height: f32) -> bool {
    height > POSTER_LANE
}

/// The clips of one track that are drawn for a view from `view_start` up to
/// `view_end`, in track order. A clip that only touches an end of the view
/// has no width in it and is not drawn.
fn clips_in_view(track: &Track, view_start: Time, view_end: Time) -> impl Iterator<Item = &Clip> {
    track.clips_overlapping(view_start, view_end)
}

/// The clips drawn in the lanes, row by row: those of the rows on screen that
/// are in the view the lanes give.
fn clips_drawn<'a>(
    rows: &'a [Row<'a>],
    geometry: &Geometry,
) -> impl Iterator<Item = (&'a Row<'a>, &'a Clip)> + use<'a> {
    let lanes = geometry.lanes;
    let view_start = geometry.time(lanes.left());
    let view_end = geometry.time(lanes.right());
    rows.iter()
        .filter(move |row| row.top < lanes.bottom() && row.top + row.height > lanes.top())
        .flat_map(move |row| {
            clips_in_view(row.track, view_start, view_end).map(move |clip| (row, clip))
        })
}

/// The media whose posters the lanes show: that of the clips drawn on
/// picture rows tall enough to carry one. Media comes up once for each of
/// its clips. A sound lane draws sound, of a video too: see [`wave_assets`].
fn poster_assets<'a>(
    project: &'a Project,
    rows: &'a [Row<'a>],
    geometry: &Geometry,
) -> impl Iterator<Item = &'a Asset> + use<'a> {
    clips_drawn(rows, geometry)
        .filter(|(row, _)| row.track.kind != TrackKind::Audio && carries_poster(row.height))
        .filter_map(move |(_, clip)| clip.asset_id().and_then(|id| project.assets.get(&id)))
}

/// The media whose sound the lanes draw: that of the clips drawn on sound
/// rows. Media comes up once for each of its clips.
fn wave_assets<'a>(
    project: &'a Project,
    rows: &'a [Row<'a>],
    geometry: &Geometry,
) -> impl Iterator<Item = &'a Asset> + use<'a> {
    clips_drawn(rows, geometry)
        .filter(|(row, _)| row.track.kind == TrackKind::Audio)
        .filter_map(move |(_, clip)| clip.asset_id().and_then(|id| project.assets.get(&id)))
}

/// The sound a tile on an audio lane draws, and where its columns fall.
struct Wave<'a> {
    peaks: &'a Peaks,
    geometry: &'a Geometry,
    /// How far along the timeline the tile is drawn from where its clip is:
    /// a clip being moved is drawn where it would land.
    shift: Time,
}

/// The held sound of a clip on an audio track, to draw on its tile.
fn wave_of<'a>(
    project: &Project,
    track: &Track,
    clip: &Clip,
    peaks: &'a HeldPeaks,
    geometry: &'a Geometry,
    shift: Time,
) -> Option<Wave<'a>> {
    if track.kind != TrackKind::Audio {
        return None;
    }
    let asset = project.assets.get(&clip.asset_id()?)?;
    Some(Wave {
        peaks: peaks.held(asset)?,
        geometry,
        shift,
    })
}

/// The columns of a waveform, one for each point of width from `left` to
/// `right`: where a column starts, and the source times its two sides show.
/// The times are the clip's own, through its rate and its time map, so a
/// stretch played backwards has its later time first and a held frame has
/// one time twice. `shift` is how far the tile is drawn from its clip.
fn wave_columns(
    clip: &Clip,
    geometry: &Geometry,
    shift: Time,
    left: f32,
    right: f32,
) -> impl Iterator<Item = (f32, Time, Time)> {
    let (origin, zoom) = (geometry.lanes.left() - geometry.scroll, geometry.zoom);
    let source = move |x: f32| {
        let at = Time::from_seconds(f64::from(x - origin) / f64::from(zoom));
        clip.source_time(at - shift)
    };
    let count = (right - left).ceil().max(0.0) as usize;
    (0..count).scan(source(left), move |from, index| {
        let x = left + index as f32;
        let to = source(x + 1.0);
        Some((x, std::mem::replace(from, to), to))
    })
}

/// Draws the sound of a clip over the part of `area` that is in view, as
/// one mesh: a column a point wide from the least to the greatest sample of
/// the source it shows. Silence is a line through the middle; a column that
/// reaches full scale is drawn in `peak`.
fn paint_wave(
    painter: &egui::Painter,
    area: Rect,
    clip: &Clip,
    wave: &Wave,
    (ink, peak): (Color32, Color32),
) {
    let visible = area.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let (middle, half) = (area.center().y, area.height() * 0.5);
    let columns = wave_columns(
        clip,
        wave.geometry,
        wave.shift,
        visible.left(),
        visible.right(),
    );
    let mut mesh = egui::Mesh::default();
    for (x, from, to) in columns {
        // Source the file has no sound for is left bare.
        let Some((min, max)) = wave.peaks.range(from, to) else {
            continue;
        };
        let mut top = middle - f32::from(max) / 128.0 * half;
        let mut bottom = middle - f32::from(min) / 128.0 * half;
        if bottom - top < 1.0 {
            let centre = (top + bottom) * 0.5;
            (top, bottom) = (centre - 0.5, centre + 0.5);
        }
        let column = Rect::from_min_max(pos2(x, top), pos2((x + 1.0).min(visible.right()), bottom));
        // A tenth of a decibel under full scale or more.
        let clipping = max == CLIPPING || min <= -CLIPPING;
        mesh.add_colored_rect(column, if clipping { peak } else { ink });
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// The sample, of 128, at which a column of a waveform reaches 0.1 dB under
/// full scale.
const CLIPPING: i8 = 127;

/// The beats of `grid` a tile of `clip` shows from `from` up to `to` at
/// `zoom` points a second: those [`peaks::beats_through`] puts on the
/// timeline, but any closer than [`BEAT_GAP`] points to another.
fn beat_ticks(clip: &Clip, grid: &BeatGrid, from: Time, to: Time, zoom: f32) -> Vec<Time> {
    let gap = Time::from_seconds(f64::from(BEAT_GAP / zoom));
    // A neighbour that is too near may be just outside the span.
    let near = peaks::beats_through(clip, grid, from - gap, to + gap);
    let apart = |index: usize| {
        let at = near[index];
        let before = index.checked_sub(1).map(|index| near[index]);
        let after = near.get(index + 1).copied();
        before.is_none_or(|before| at - before >= gap)
            && after.is_none_or(|after| after - at >= gap)
    };
    (0..near.len())
        .filter(|index| from <= near[*index] && near[*index] < to && apart(*index))
        .map(|index| near[index])
        .collect()
}

/// The beats the tile of `clip` on `track` shows from `from` up to `to`:
/// none unless the clip is on an audio lane, its media's beats are shown
/// and held, and it is heard (`sounds`, as [`sound::audible_at`] asks).
#[expect(
    clippy::too_many_arguments,
    reason = "one tile's beats need its clip, the beats held and the view"
)]
fn shown_ticks(
    project: &Project,
    sounding: &sound::Sounding,
    beats: &HeldBeats,
    track: &Track,
    clip: &Clip,
    from: Time,
    to: Time,
    zoom: f32,
) -> Vec<Time> {
    let grid = (track.kind == TrackKind::Audio)
        .then(|| clip.asset_id().and_then(|id| project.assets.get(&id)))
        .flatten()
        .filter(|asset| beats.shown(asset.id))
        .and_then(|asset| beats.grid(asset));
    match grid {
        Some(grid) if sounding.sounds(project, track, clip) => {
            beat_ticks(clip, grid, from, to, zoom)
        }
        _ => Vec::new(),
    }
}

/// The beats the tiles drawn show, clip by clip, at their times on the
/// timeline, from the view's start to its end. Has the beats wanted of the
/// media on the sound rows drawn whose beats are shown.
fn beats_drawn(
    project: &Project,
    rows: &[Row],
    geometry: &Geometry,
    beats: &mut HeldBeats,
) -> Vec<(ClipId, Time)> {
    if !beats.any_shown() {
        return Vec::new();
    }
    for asset in wave_assets(project, rows, geometry) {
        beats.want(asset);
    }
    let sounding = sound::Sounding::of(project);
    let (from, to) = (
        geometry.time(geometry.lanes.left()),
        geometry.time(geometry.lanes.right()),
    );
    let mut drawn = Vec::new();
    for (row, clip) in clips_drawn(rows, geometry) {
        let ticks = shown_ticks(
            project,
            &sounding,
            beats,
            row.track,
            clip,
            from,
            to,
            geometry.zoom,
        );
        drawn.extend(ticks.into_iter().map(|at| (clip.id, at)));
    }
    drawn
}

/// Draws a tick at each of `ticks`, points across, rising from the foot of
/// a tile's sound: one mesh.
fn paint_beats(painter: &egui::Painter, area: Rect, ticks: &[f32], ink: Color32) {
    let mut mesh = egui::Mesh::default();
    let top = area.bottom() - BEAT_TICK.min(area.height());
    for x in ticks {
        let tick = Rect::from_min_max(pos2(x - 0.5, top), pos2(x + 0.5, area.bottom()));
        mesh.add_colored_rect(tick, ink);
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

fn clip_tone(project: &Project, track: &Track, clip: &Clip) -> theme::Tone {
    match &clip.content {
        ClipContent::Media { asset_id } if track.kind == TrackKind::Video => project
            .assets
            .get(asset_id)
            .map_or(theme::Tone::Video, |asset| super::tone_of_media(asset.kind)),
        _ => tone_of_track(track.kind),
    }
}

/// Snaps a time to the nearest clip edge, marker, beat or the playhead
/// within reach. `beats` are the beats it may land on, those of the tiles
/// drawn less any of clips that are moving with it; `exclude` are the
/// clips whose edges it does not. Returns the time and whether it snapped.
fn snapped(
    input: &TimelineInput,
    geometry: &Geometry,
    beats: &[(ClipId, Time)],
    time: Time,
    exclude: &[ClipId],
) -> (Time, bool) {
    if !input.snapping {
        return (time, false);
    }
    let reach = geometry.span(SNAP_POINTS);
    let mut best = snap_time(input.project, time, reach, exclude);
    let beat = (beats.iter())
        .map(|(_, at)| *at)
        .min_by_key(|at| (at.0 - time.0).abs());
    if let Some(beat) = beat {
        let to_beat = (beat.0 - time.0).abs();
        if to_beat <= reach.0 && (best == time || to_beat < (best.0 - time.0).abs()) {
            best = beat;
        }
    }
    let to_playhead = (input.playhead.0 - time.0).abs();
    if to_playhead <= reach.0 && (best == time || to_playhead < (best.0 - time.0).abs()) {
        best = input.playhead;
    }
    (best, best != time)
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site; the timeline is drawn from all of it"
)]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &mut TimelineState,
    selection: &mut Selection,
    textures: &mut Textures,
    peaks: &mut HeldPeaks,
    input: &TimelineInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    // The timeline is a content pane, as the viewer is: the darkest ground,
    // with the tool strip on it and a hairline under the strip.
    kit::pane_well(ui.painter(), rect, p.bg, p);
    let strip = Rect::from_min_size(rect.min, vec2(rect.width(), STRIP));
    let body = Rect::from_min_max(pos2(rect.left(), strip.bottom()), rect.max);
    ui.painter()
        .hline(rect.x_range(), strip.bottom() - 0.5, p.hairline());

    let heads = Rect::from_min_max(
        pos2(body.left(), body.top() + RULER),
        pos2(body.left() + HEAD_WIDTH, body.bottom()),
    );
    let ruler = Rect::from_min_max(
        pos2(heads.right(), body.top()),
        pos2(body.right(), body.top() + RULER),
    );
    let lanes = Rect::from_min_max(pos2(heads.right(), ruler.bottom()), body.max);
    state.lane_width = lanes.width().max(1.0);

    let project = input.project;
    // A marking that went with its transition, or whose clip is no longer
    // the one selected, is not selected either.
    state.transition = state.marked(project, selection);
    let duration = project.duration();
    if state.fit {
        state.fit = false;
        let seconds = duration.as_seconds().max(10.0) as f32;
        state.zoom = (state.lane_width * 0.92 / seconds).clamp(MIN_ZOOM, MAX_ZOOM);
        state.scroll.x = 0.0;
    }

    tool_strip(ui, strip, state, selection, input, actions);

    // Rows, laid out once and shared by heads, lanes and hit tests.
    let order = display_order(project);
    let lane = (state.keys.lane_rows(project))
        .map(|(track, rows)| (track, KeyframesState::lane_height(rows)));
    let content_height: f32 = order
        .iter()
        .map(|track| lane_height(track.kind) + LANE_GAP)
        .sum::<f32>()
        + LANE_GAP
        + lane.map_or(0.0, |lane| lane.1);
    // The graph has the tracks' place: where they were scrolled to is kept
    // for when they come back.
    if !state.keys.graph {
        state.scroll.y = state
            .scroll
            .y
            .clamp(0.0, (content_height - lanes.height()).max(0.0));
    }
    let content_width = duration.as_seconds() as f32 * state.zoom + state.lane_width * 0.5;
    state.scroll.x = state
        .scroll
        .x
        .clamp(0.0, (content_width - state.lane_width).max(0.0));
    let mut rows = lay_out_with(order.clone(), lanes.top() + LANE_GAP - state.scroll.y, lane);
    // A lane that was just opened is scrolled into view: as much of it as
    // the lanes have room for, from its first row.
    if state.keys.take_reveal()
        && let Some(row) = rows.iter().find(|row| row.lane > 0.0)
    {
        let top = row.top + row.height;
        let hidden = (top + row.lane - lanes.bottom()).min(top - lanes.top());
        if hidden > 0.0 {
            state.scroll.y += hidden;
            rows = lay_out_with(order, lanes.top() + LANE_GAP - state.scroll.y, lane);
        }
    }
    // The focus ring stays with a clip only while it is selected.
    state.keyboard = state.keyboard.filter(|id| selection.contains(*id));
    // A clip the keyboard selected is scrolled to: its row, and its start.
    if let Some(id) = state.reveal.take()
        && !state.keys.graph
        && let Some((row, clip)) = rows.iter().find_map(|row| {
            let clip = row.track.clips.iter().find(|clip| clip.id == id)?;
            Some((row, clip))
        })
    {
        let above = lanes.top() + LANE_GAP - row.top;
        let below = row.top + row.height + LANE_GAP - lanes.bottom();
        if above > 0.0 {
            state.scroll.y -= above;
        } else if below > 0.0 {
            state.scroll.y += below.min(-above);
        }
        let x = clip.start.as_seconds() as f32 * state.zoom;
        if x < state.scroll.x || x > state.scroll.x + state.lane_width - 48.0 {
            state.scroll.x = (x - 24.0).max(0.0);
        }
        state.scroll.y = state.scroll.y.max(0.0);
        let order = display_order(project);
        rows = lay_out_with(order, lanes.top() + LANE_GAP - state.scroll.y, lane);
    }
    let geometry = Geometry {
        lanes,
        zoom: state.zoom,
        scroll: state.scroll.x,
    };

    // The ruler's band runs over the corner above the heads too, which
    // stays empty: the transport says the playhead's time. A hairline
    // under the band, and one between the heads and the lanes.
    let band = Rect::from_min_max(body.min, pos2(body.right(), ruler.bottom()));
    ui.painter()
        .rect_filled(band, CornerRadius::ZERO, p.ruler());
    ui.painter()
        .hline(band.x_range(), band.bottom() - 0.5, p.hairline());
    ui.painter().vline(
        heads.right() - 0.5,
        band.top()..=body.bottom(),
        p.hairline(),
    );

    // The beats the tiles show, which the ruler and the lanes snap to.
    state.beats = if state.keys.graph {
        Vec::new()
    } else {
        beats_drawn(project, &rows, &geometry, &mut peaks.beats)
    };
    draw_ruler(ui, ruler, &geometry, state, input, actions);
    state.keys.begin(ui, &geometry, selection, input, actions);
    let mut over_tiles = None;
    if state.keys.graph {
        (state.keys).show_graph(ui, heads, lanes, &geometry, selection, input, actions);
    } else {
        let keys = &mut state.keys;
        let targets = targets(project, selection);
        track_heads(ui, heads, &rows, &geometry, keys, &targets, input, actions);
        over_tiles = Some(draw_lanes(
            ui, lanes, &rows, &geometry, state, selection, &targets, textures, peaks, input,
            actions,
        ));
    }

    // The playhead runs through the ruler and the lanes, in ink and not the
    // accent: it says where, not what is chosen. A guide a held drag rests
    // against runs the same way while the drag lasts.
    // The pin may stand over the corner at the first frame, so it is not
    // cut in half there. The guide and the playhead are drawn over the
    // tiles and under the clips a drag carries, in the place the lanes kept
    // for them.
    let painter = ui.painter_at(Rect::from_min_max(
        pos2(ruler.left() - PIN.x * 0.5 - 1.0, ruler.top()),
        lanes.max,
    ));
    let mut marks = Vec::new();
    // Each only where the lanes show its time: the painter reaches into the
    // corner for the pin alone.
    if let Some(x) = (state.snap_guide)
        .map(|guide| geometry.x(guide))
        .filter(|x| (lanes.left()..=lanes.right()).contains(x))
    {
        marks.extend(snap_guide_shapes(&painter, ruler, lanes, x, p));
    }
    let x = geometry.x(input.playhead);
    if x >= lanes.left() - 1.0 && x <= lanes.right() + 1.0 {
        marks.extend(playhead_shapes(&painter, ruler, lanes, x, p));
    }
    match over_tiles {
        Some(slot) => painter.set(slot, egui::Shape::Vec(marks)),
        None => painter.extend(marks),
    }

    // Wheel, trackpad and pinch over the timeline. A wheel turned with Cmd
    // or Ctrl held, or a pinch, zooms about the time under the pointer
    // anywhere over the panel, its tool strip included.
    let zoom = ui.input(|i| i.zoom_delta());
    if zoom != 1.0 && ui.rect_contains_pointer(rect) {
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let anchor = pointer.map_or(state.lane_width * 0.5, |pos| {
            (pos.x - lanes.left()).clamp(0.0, state.lane_width)
        });
        state.zoom_about(zoom, anchor);
    } else if ui.rect_contains_pointer(body) {
        let (scroll, pointer) = ui.input(|i| (i.smooth_scroll_delta(), i.pointer.hover_pos()));
        if scroll != Vec2::ZERO {
            state.scroll.x -= scroll.x;
            if state.keys.graph {
                // Over the graph the wheel moves the values it shows.
                if pointer.is_some_and(|pos| lanes.contains(pos)) {
                    state.keys.pan(scroll.y, lanes.height());
                }
            } else if content_height > lanes.height() {
                state.scroll.y -= scroll.y;
            } else {
                state.scroll.x -= scroll.y;
            }
            state.scroll.x = state.scroll.x.max(0.0);
        }
    }
}

/// The playhead's pin: a 12 by 6 head over a 5-point point, which is as
/// tall as the ruler's lower half leaves room for.
const PIN: Vec2 = vec2(12.0, 11.0);
const PIN_POINT: f32 = 5.0;

/// The x of a vertical line one point wide at `x`, set on a pixel's middle so
/// it is drawn sharp.
fn sharp(painter: &egui::Painter, x: f32) -> f32 {
    painter.round_to_pixel_center(x)
}

/// A line one point wide down `ys` at `x`, between two lines of `halo` that
/// keep it clear of what it crosses: two rectangles, no mesh.
fn haloed_line(x: f32, ys: egui::Rangef, ink: Color32, halo: Color32) -> [egui::Shape; 2] {
    let span = |half: f32| Rect::from_x_y_ranges(x - half..=x + half, ys);
    [
        egui::Shape::rect_filled(span(1.5), CornerRadius::ZERO, halo),
        egui::Shape::rect_filled(span(0.5), CornerRadius::ZERO, ink),
    ]
}

/// The playhead at `x`: a line from its pin's top down through the lanes,
/// and the pin wholly in the ruler's lower half with its point on the
/// ruler's foot. The labels end above the middle, so the two never share a
/// row of pixels.
fn playhead_shapes(
    painter: &egui::Painter,
    ruler: Rect,
    lanes: Rect,
    x: f32,
    p: &Palette,
) -> Vec<egui::Shape> {
    let (ink, halo) = (p.playhead(), p.playhead_halo());
    let x = sharp(painter, x);
    let (half, foot) = (PIN.x * 0.5, ruler.bottom());
    // Its top and shoulders on a pixel's middle, as its sides are, so the
    // halo round it is one sharp pixel and not two faint ones.
    let shoulder = painter.round_to_pixel_center(foot - PIN_POINT);
    let top = painter.round_to_pixel_center(foot - PIN.y);
    // The line starts inside the pin, behind it, so none of it, nor its
    // feathered edge, stands above the pin.
    let mut shapes = Vec::from(haloed_line(
        x,
        egui::Rangef::new(top + 1.5, lanes.bottom()),
        ink,
        halo,
    ));
    shapes.push(egui::Shape::convex_polygon(
        vec![
            pos2(x - half, top),
            pos2(x + half, top),
            pos2(x + half, shoulder),
            pos2(x, foot),
            pos2(x - half, shoulder),
        ],
        ink,
        Stroke::new(1.0, halo),
    ));
    shapes
}

/// The guide a held drag rests against at `x`: a line through the ruler and
/// the lanes, and a small diamond on the ruler's foot at the time.
fn snap_guide_shapes(
    painter: &egui::Painter,
    ruler: Rect,
    lanes: Rect,
    x: f32,
    p: &Palette,
) -> Vec<egui::Shape> {
    let ink = p.snap();
    let x = sharp(painter, x);
    let mut shapes = Vec::from(haloed_line(
        x,
        egui::Rangef::new(ruler.top(), lanes.bottom()),
        ink,
        p.playhead_halo(),
    ));
    let foot = ruler.bottom() - 0.5;
    shapes.push(egui::Shape::convex_polygon(
        vec![
            pos2(x, foot - 2.5),
            pos2(x + 2.5, foot),
            pos2(x, foot + 2.5),
            pos2(x - 2.5, foot),
        ],
        ink,
        Stroke::NONE,
    ));
    shapes
}

/// The side of a tool strip button, the room between two of one group and
/// the room between groups.
const TOOL: f32 = metrics::ICON_BUTTON;
const TOOL_GAP: f32 = 2.0;
const GROUP_GAP: f32 = 12.0;
/// How far the strip's first and last controls stand from the pane's edges.
const STRIP_INSET: f32 = 8.0;
/// The room between the zoom controls, and the widths the zoom slider takes.
const ZOOM_GAP: f32 = 4.0;
const ZOOM_SLIDER: f32 = 120.0;
const ZOOM_SLIDER_NARROW: f32 = 64.0;
/// The least room left between the strip's leading groups and its trailing
/// controls.
const STRIP_ROOM: f32 = 10.0;
/// The name of the snapping toggle, which says whether it is on as a check
/// box does rather than in its name.
const SNAPPING: &str = "Snapping (N)";

/// What of the tool strip folds away in a pane too narrow for all of it, in
/// the order it goes: the zoom slider narrows, then goes; then the Insert
/// group folds into a menu at the end of the leading groups, then the Edit
/// group joins it. The keyframe and graph toggles, snapping, zoom and Fit
/// never fold. At an interface scale of one every window has room for all of
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Fold {
    Nothing,
    NarrowSlider,
    NoSlider,
    Insert,
    Edit,
}

impl Fold {
    /// The least that folds in a strip `width` wide.
    fn of(width: f32) -> Self {
        [
            Self::Nothing,
            Self::NarrowSlider,
            Self::NoSlider,
            Self::Insert,
        ]
        .into_iter()
        .find(|fold| fold.leading() + fold.trailing() + STRIP_ROOM <= width)
        .unwrap_or(Self::Edit)
    }

    /// How wide the zoom slider is, where it is shown.
    fn slider(self) -> Option<f32> {
        match self {
            Self::Nothing => Some(ZOOM_SLIDER),
            Self::NarrowSlider => Some(ZOOM_SLIDER_NARROW),
            _ => None,
        }
    }

    /// How wide the leading groups are, from the pane's edge: Edit, Insert
    /// and Animate, or what of them is left beside the menu they fold into.
    fn leading(self) -> f32 {
        let group = |buttons: f32| buttons * TOOL + (buttons - 1.0) * TOOL_GAP;
        let groups: &[f32] = match self {
            Self::Insert => &[group(3.0), group(2.0), TOOL],
            Self::Edit => &[group(2.0), TOOL],
            _ => &[group(3.0), group(3.0), group(2.0)],
        };
        STRIP_INSET + groups.iter().sum::<f32>() + GROUP_GAP * (groups.len() - 1) as f32
    }

    /// How wide the trailing controls are, to the pane's edge: snapping,
    /// then zoom out, the slider and zoom in, then Fit.
    fn trailing(self) -> f32 {
        let slider = self.slider().map_or(0.0, |width| width + ZOOM_GAP);
        TOOL + GROUP_GAP + TOOL + ZOOM_GAP + slider + TOOL + ZOOM_GAP + TOOL + STRIP_INSET
    }
}

fn tool_strip(
    ui: &mut Ui,
    strip: Rect,
    state: &mut TimelineState,
    selection: &Selection,
    input: &TimelineInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let fold = Fold::of(strip.width());
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(strip.shrink2(vec2(STRIP_INSET, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut ui;
    ui.spacing_mut().item_spacing.x = 0.0;
    let has_selection = !selection.clips.is_empty();
    let split = format!("Split at playhead ({})", "S");
    let duplicate = format!("Duplicate ({})", shortcut("Cmd+D"));

    // Edit.
    if fold < Fold::Edit {
        if icon_button(ui, p, Icon::Scissors, &split).clicked() {
            actions.push(Action::SplitAtPlayhead);
        }
        ui.add_space(TOOL_GAP);
        ui.add_enabled_ui(has_selection, |ui| {
            if icon_button(ui, p, Icon::Trash, "Delete (Delete)").clicked() {
                actions.push(Action::Delete { ripple: false });
            }
            ui.add_space(TOOL_GAP);
            if icon_button(ui, p, Icon::Copy, &duplicate).clicked() {
                actions.push(Action::Duplicate);
            }
        });
        ui.add_space(GROUP_GAP);
    }
    // Insert.
    if fold < Fold::Insert {
        if icon_button(ui, p, Icon::Flag, "Add marker at playhead (M)").clicked() {
            actions.push(Action::AddMarker);
        }
        ui.add_space(TOOL_GAP);
        if icon_button(ui, p, Icon::Text, "Add text at playhead (T)").clicked() {
            actions.push(Action::AddText);
        }
        ui.add_space(TOOL_GAP);
        let surface = ui.painter().add(egui::Shape::Noop);
        let add = icon_button(ui, p, Icon::Layers, "Add track");
        open_surface(ui, p, surface, &add);
        egui::Popup::menu(&add).show(|ui| {
            menu_style(ui, 180.0);
            add_track_rows(ui, p, actions);
        });
        ui.add_space(GROUP_GAP);
    }
    // Animate: toggles that stay pressed while what they show is open.
    let keys = &mut state.keys;
    let lane = keys.can_toggle_lane(input.project, selection) && !keys.graph;
    ui.add_enabled_ui(lane, |ui| {
        let open = keys.lane_clip().is_some() && !keys.graph;
        if icon_toggle(
            ui,
            p,
            Icon::Diamond,
            open,
            "Keyframes of the selected clip (U)",
        )
        .clicked()
        {
            keys.toggle_lane(input.project, selection);
        }
    });
    ui.add_space(TOOL_GAP);
    if icon_toggle(ui, p, Icon::Curve, keys.graph, "Graph editor (G)").clicked() {
        keys.graph = !keys.graph;
    }
    // What folded, in a menu after them.
    if fold >= Fold::Insert {
        ui.add_space(GROUP_GAP);
        let surface = ui.painter().add(egui::Shape::Noop);
        let more = icon_button(ui, p, Icon::Dots, "More timeline actions");
        open_surface(ui, p, surface, &more);
        egui::Popup::menu(&more).show(|ui| {
            menu_style(ui, 240.0);
            let mut item = |ui: &mut Ui, icon: Icon, text: &str, keys: &str, action: Action| {
                // A row that removes is red, as the clip menu's is.
                let row = if matches!(action, Action::Delete { .. }) {
                    widgets::menu_item_destructive(ui, p, Some(icon), text, &shortcut(keys))
                } else {
                    menu_item(ui, p, Some(icon), text, &shortcut(keys))
                };
                if row.clicked() {
                    actions.push(action);
                    ui.close();
                }
            };
            if fold >= Fold::Edit {
                item(
                    ui,
                    Icon::Scissors,
                    "Split at playhead",
                    "S",
                    Action::SplitAtPlayhead,
                );
                ui.add_enabled_ui(has_selection, |ui| {
                    let delete = Action::Delete { ripple: false };
                    item(ui, Icon::Trash, "Delete", "Delete", delete);
                    item(ui, Icon::Copy, "Duplicate", "Cmd+D", Action::Duplicate);
                });
                widgets::menu_separator(ui, p);
            }
            item(
                ui,
                Icon::Flag,
                "Add marker at playhead",
                "M",
                Action::AddMarker,
            );
            item(ui, Icon::Text, "Add text at playhead", "T", Action::AddText);
            controls::menu_submenu(ui, *p, Icon::Layers, "Add track", |ui| {
                menu_style(ui, 180.0);
                add_track_rows(ui, p, actions);
            });
        });
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if icon_button(ui, p, Icon::Fit, "Fit timeline to window (Shift+Z)").clicked() {
            actions.push(Action::FitTimeline);
        }
        ui.add_space(ZOOM_GAP);
        if icon_button(
            ui,
            p,
            Icon::ZoomIn,
            &format!("Zoom in ({})", shortcut("Cmd+Scroll, =")),
        )
        .clicked()
        {
            actions.push(Action::ZoomTimeline(1.4));
        }
        ui.add_space(ZOOM_GAP);
        if let Some(width) = fold.slider() {
            // The slider works on a logarithmic scale, as zoom is perceived.
            let mut level = (state.zoom / MIN_ZOOM).ln() / (MAX_ZOOM / MIN_ZOOM).ln();
            if zoom_slider(ui, p, &mut level, width).changed() {
                let zoom = MIN_ZOOM * (MAX_ZOOM / MIN_ZOOM).powf(level);
                state.zoom_centered(zoom / state.zoom);
            }
            ui.add_space(ZOOM_GAP);
        }
        if icon_button(
            ui,
            p,
            Icon::ZoomOut,
            &format!("Zoom out ({})", shortcut("Cmd+Scroll, -")),
        )
        .clicked()
        {
            actions.push(Action::ZoomTimeline(1.0 / 1.4));
        }
        ui.add_space(GROUP_GAP);
        if snapping_toggle(ui, p, input.snapping).clicked() {
            actions.push(Action::ToggleSnapping);
        }
    });
}

/// Fills the place kept under a strip button whose menu is open, as a
/// pressed button is filled, so the menu says where it came from.
fn open_surface(ui: &Ui, p: &Palette, surface: egui::layers::ShapeIdx, button: &Response) {
    if egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(button)) {
        ui.painter().set(
            surface,
            egui::Shape::rect_filled(button.rect.shrink(1.0), 7, p.pressed),
        );
    }
}

/// The snapping toggle: pressed while snapping is on, named the same either
/// way, with its state said by its role and its tooltip.
fn snapping_toggle(ui: &mut Ui, p: &Palette, on: bool) -> Response {
    let (_, response) = ui.allocate_exact_size(Vec2::splat(TOOL), Sense::click());
    kit::paint_icon_button(ui, p, &response, Icon::Magnet, on, None);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), on, SNAPPING)
    });
    let said = if on {
        "Snapping is on"
    } else {
        "Snapping is off"
    };
    icons::hint(
        response.on_hover_cursor(CursorIcon::PointingHand),
        said,
        "N",
    )
}

/// The zoom slider: Terminal's slider without the fill, since a zoom is not
/// an amount; `level` runs from nought, the widest view, to one.
fn zoom_slider(ui: &mut Ui, p: &Palette, level: &mut f32, width: f32) -> Response {
    const KNOB: f32 = 8.0;
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click_and_drag());
    let left = rect.left() + KNOB;
    let travel = (rect.width() - KNOB * 2.0).max(1.0);
    let mut next = *level;
    if let Some(pos) = response.interact_pointer_pos() {
        next = ((pos.x - left) / travel).clamp(0.0, 1.0);
    }
    // The left and right arrows are the slider's while it has the focus,
    // and do not take the focus on to its neighbours. A screen reader steps
    // it, or sets it, as it would egui's own.
    let focused = response.has_focus();
    if focused {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            );
        });
    }
    let nudge = ui.input(|input| {
        use egui::accesskit::Action;
        let keys = if focused {
            input.num_presses(egui::Key::ArrowRight) as f32
                - input.num_presses(egui::Key::ArrowLeft) as f32
        } else {
            0.0
        };
        keys + input.num_accesskit_action_requests(response.id, Action::Increment) as f32
            - input.num_accesskit_action_requests(response.id, Action::Decrement) as f32
    });
    next = (next + nudge * 0.05).clamp(0.0, 1.0);
    ui.input(|input| {
        use egui::accesskit::{Action, ActionData};
        for request in input.accesskit_action_requests(response.id, Action::SetValue) {
            if let Some(ActionData::NumericValue(value)) = request.data {
                next = (value as f32).clamp(0.0, 1.0);
            }
        }
    });
    if next != *level {
        *level = next;
        response.mark_changed();
    }
    let current = *level;
    response.widget_info(|| {
        egui::WidgetInfo::slider(ui.is_enabled(), f64::from(current), "Timeline zoom")
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        use egui::accesskit::Action;
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(1.0);
        node.set_numeric_value_step(0.05);
        node.add_action(Action::SetValue);
        if current < 1.0 {
            node.add_action(Action::Increment);
        }
        if current > 0.0 {
            node.add_action(Action::Decrement);
        }
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let knob = pos2(left + travel * current.clamp(0.0, 1.0), rect.center().y);
        let track = Rect::from_min_max(
            pos2(rect.left(), rect.center().y - 2.0),
            pos2(rect.right(), rect.center().y + 2.0),
        );
        painter.rect_filled(track, 2, p.pressed);
        if focused {
            painter.circle_stroke(
                knob,
                KNOB + 2.0,
                Stroke::new(2.0, theme::tint(p.accent, 0.55)),
            );
        }
        painter.circle_filled(knob + vec2(0.0, 1.0), KNOB, Color32::from_black_alpha(50));
        painter.circle_filled(knob, KNOB, Color32::WHITE);
        // On the light strip a white knob needs an edge of its own.
        if !p.dark {
            painter.circle_stroke(knob, KNOB - 0.5, Stroke::new(1.0, p.border));
        }
    }
    icons::hint(
        response.on_hover_cursor(CursorIcon::PointingHand),
        "Timeline zoom",
        "",
    )
}

/// The rows that add a track of each kind, each with the glyph its heads
/// show, quieter than the row's words.
fn add_track_rows(ui: &mut Ui, p: &Palette, actions: &mut Vec<Action>) {
    for (kind, label) in [
        (TrackKind::Video, "Video track"),
        (TrackKind::Audio, "Audio track"),
        (TrackKind::Text, "Text track"),
    ] {
        let row = kit::menu_row(ui, p, None, label, "", false, None);
        let ink = if row.hovered() || row.has_focus() {
            p.on_accent
        } else {
            p.secondary
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(row.rect.left() + 15.0, row.rect.center().y),
                Vec2::splat(14.0),
            ),
            icon_of_track(kind),
            ink,
        );
        if row.clicked() {
            actions.push(Action::AddTrack(kind));
            ui.close();
        }
    }
}

/// The part of a row a track's head is drawn in.
fn head_rect(heads: Rect, row: &Row) -> Rect {
    Rect::from_min_size(
        pos2(heads.left() + 6.0, row.top),
        vec2(heads.width() - 10.0, row.height),
    )
}

/// How many toggles the head of a track of this kind has: lock, then mute
/// and solo where the track carries sound and hide where it carries picture.
fn head_toggles(kind: TrackKind) -> usize {
    match kind {
        TrackKind::Video => 4,
        TrackKind::Audio => 3,
        TrackKind::Text => 2,
    }
}

/// How many rows the toggles of a head stand in: one, or two where there are
/// more than a row holds and the head is tall enough for both.
fn head_lines(head: Rect, toggles: usize) -> usize {
    if toggles > HEAD_ROW && head.height() >= HEAD_CELL_STEP + HEAD_CELL {
        2
    } else {
        1
    }
}

/// Where toggle `index` of `toggles` stands in a head. They fill columns from
/// the trailing edge, each column top down.
fn head_cell(head: Rect, toggles: usize, index: usize) -> Rect {
    let lines = head_lines(head, toggles);
    let (column, line) = (index / lines, index % lines);
    let tall = (lines - 1) as f32 * HEAD_CELL_STEP + HEAD_CELL;
    Rect::from_min_size(
        pos2(
            head.right() - 4.0 - HEAD_CELL - column as f32 * HEAD_CELL_STEP,
            head.center().y - tall * 0.5 + line as f32 * HEAD_CELL_STEP,
        ),
        Vec2::splat(HEAD_CELL),
    )
}

/// The room a head has for its track's name and volume, between the icon of
/// its kind and its toggles.
fn head_name_room(head: Rect, toggles: usize) -> egui::Rangef {
    let columns = toggles.div_ceil(head_lines(head, toggles));
    let right = head.right() - 4.0 - columns as f32 * HEAD_CELL_STEP;
    egui::Rangef::new(
        head.left() + HEAD_NAME,
        (right - 2.0).max(head.left() + HEAD_NAME),
    )
}

/// A 22-point toggle inside a track head: bare while off, pressed while on.
/// `ink` is its glyph's colour where that says more than on or off.
#[expect(clippy::too_many_arguments, reason = "a toggle is all of these")]
fn head_toggle(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    id: Id,
    icon: Icon,
    on: bool,
    label: &str,
    ink: Option<Color32>,
) -> bool {
    let response = ui.interact(rect, id, Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, label));
    let hover = kit::hover_amount(ui, &response);
    let painter = ui.painter();
    if on || response.is_pointer_button_down_on() {
        painter.rect_filled(rect, CornerRadius::same(6), p.pressed);
    } else if hover > 0.0 {
        painter.rect_filled(rect, CornerRadius::same(6), p.hover.gamma_multiply(hover));
    }
    if response.has_focus() {
        controls::focus_ring(painter, rect, 6, *p);
    }
    let color = if on {
        ink.unwrap_or(p.fg)
    } else if hover > 0.0 {
        theme::mix(p.muted, p.fg, hover)
    } else {
        ink.unwrap_or(p.muted)
    };
    icons::paint(
        painter,
        Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
        icon,
        color,
    );
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(label)
        .clicked()
}

/// What the mute toggle of a head is named. A track that another's solo
/// silences says so: muting it would change nothing heard.
fn mute_label(track: &Track, silenced: bool) -> &'static str {
    match (track.muted, silenced) {
        (true, _) => "Unmute track",
        (false, true) => "Mute track (silenced by a solo)",
        (false, false) => "Mute track",
    }
}

/// The rows of a head's menu that set its track's volume: a field in
/// percent, which is one edit when what is typed is entered or a drag is let
/// go, and a way back to the volume a track starts with.
fn volume_menu(ui: &mut Ui, p: &Palette, track: &Track, actions: &mut Vec<Action>) {
    // What a drag holds until it is let go: the project is not edited under
    // it.
    let held = Id::new(("track-volume", track.id));
    let read = tracks::percent(track);
    let dragging = ui.data(|data| data.get_temp::<f32>(held));
    let mut shown = dragging.unwrap_or(read);
    // A row as the others are: its word where theirs are, the field at the
    // trailing edge where their shortcuts are.
    let row = ui.allocate_ui_with_layout(
        vec2(ui.available_width(), metrics::COMPACT_HEIGHT),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.set_min_size(vec2(ui.available_width(), metrics::COMPACT_HEIGHT));
            ui.add_space(10.0);
            let field = volume_field(ui, p, &mut shown);
            // The word is a label, as it is read out.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add_space(32.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("Volume")
                            .font(theme::regular(13.0))
                            .color(p.fg),
                    )
                    .selectable(false),
                );
            });
            field
        },
    );
    let field = row.inner;
    // Escape gives a drag up, as it gives up what is typed: egui ends the
    // drag on that frame with the button still down, which is no release.
    let given_up = ui.input(|input| input.key_pressed(egui::Key::Escape));
    if field.dragged() && !given_up {
        ui.data_mut(|data| data.insert_temp(held, shown));
    } else {
        ui.data_mut(|data| data.remove_temp::<f32>(held));
        // A drag is an edit only where it was let go; the field takes what
        // is typed when it is entered. The field holds the volume the track
        // has, so one left as it was is no edit, and one given the number it
        // reads is: a track whose finer volume reads "0" can be silenced.
        let entered = match dragging {
            Some(_) => field.drag_stopped() && !given_up,
            None => !field.dragged(),
        };
        if entered
            && shown != read
            && let Some(command) = tracks::volume(track, shown)
            && !matches!(&command, Command::SetTrackState { gain: Some(gain), .. } if *gain == track.gain)
        {
            actions.push(Action::Edit(command));
            // A typed volume is entered and done; one dragged or stepped
            // with the arrows stays to be changed again.
            if field.lost_focus() {
                ui.close();
            }
        }
    }
    ui.add_enabled_ui(track.gain != 1.0, |ui| {
        if menu_item(ui, p, None, "Reset volume", "").clicked() {
            actions.extend(tracks::volume(track, 100.0).map(Action::Edit));
            ui.close();
        }
    });
}

/// The track volume's field: a number in percent, dragged or typed, on the
/// surface of the editor's other fields and with their quieter unit. What is
/// typed is the field's until it is entered. It is `kit::number_field`
/// but for that: the kit's field updates as it is typed in and rounds to
/// fixed decimals.
fn volume_field(ui: &mut Ui, p: &Palette, value: &mut f32) -> Response {
    const FIELD: Vec2 = vec2(72.0, metrics::INSPECTOR_CONTROL);
    const UNIT: &str = " %";
    let surface = ui.painter().add(egui::Shape::Noop);
    let unit_font = theme::regular(12.5);
    let unit = ui
        .painter()
        .layout_no_wrap(UNIT.to_owned(), unit_font.clone(), p.muted);
    let field = ui
        .scope(|ui| {
            // The drag value draws no surface of its own: this one is drawn
            // beneath it.
            let style = ui.style_mut();
            style
                .text_styles
                .insert(style.drag_value_text_style.clone(), theme::regular(12.5));
            // The value ends 8 points in from the unit.
            style.spacing.interact_size = FIELD;
            style.spacing.button_padding = vec2(8.0, 0.0);
            let visuals = &mut style.visuals;
            visuals.text_edit_bg_color = Some(Color32::TRANSPARENT);
            visuals.selection.stroke = Stroke::NONE;
            for widget in [
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
                &mut visuals.widgets.open,
            ] {
                widget.weak_bg_fill = Color32::TRANSPARENT;
                widget.bg_fill = Color32::TRANSPARENT;
                widget.bg_stroke = Stroke::NONE;
                widget.corner_radius = CornerRadius::same(7);
            }
            // The unit is laid out unseen after the value, which keeps the
            // value clear of it and its text in the value read out; it is
            // painted over that place below.
            let drag = egui::DragValue::new(value)
                .range(0.0..=tracks::MOST)
                .speed(1.0)
                .suffix(
                    egui::RichText::new(UNIT)
                        .font(unit_font.clone())
                        .color(Color32::TRANSPARENT),
                )
                .max_decimals(2)
                .update_while_editing(false);
            let layout = egui::Layout::centered_and_justified(egui::Direction::LeftToRight)
                .with_main_align(egui::Align::Max);
            ui.allocate_ui_with_layout(FIELD, layout, |ui| ui.add(drag))
                .inner
        })
        .inner;
    field.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Track volume")
    });
    let rect = Rect::from_center_size(field.rect.center(), FIELD);
    let mut shapes = vec![egui::Shape::rect_filled(rect, 7, p.control)];
    let editing = field.has_focus();
    if field.hovered() && !editing {
        shapes.push(egui::Shape::rect_filled(rect, 7, p.hover));
    }
    if editing {
        // A field being typed in: an accent outline with a soft halo.
        shapes.push(egui::Shape::rect_stroke(
            rect.expand(2.5),
            9,
            Stroke::new(3.0, theme::tint(p.accent, 0.28)),
            StrokeKind::Inside,
        ));
        shapes.push(egui::Shape::rect_stroke(
            rect,
            7,
            Stroke::new(1.0, p.accent),
            StrokeKind::Inside,
        ));
    }
    ui.painter().set(surface, egui::Shape::Vec(shapes));
    // While it is typed in, the field holds only the number.
    if !editing && ui.is_rect_visible(rect) {
        let at = pos2(
            rect.right() - 8.0 - unit.size().x,
            rect.center().y - unit.size().y * 0.5,
        );
        ui.painter().galley(at, unit, p.muted);
    }
    field
}

/// A context menu on `response`, opened where the secondary click was and
/// kept 8 points inside the window: below and after the pointer where it
/// fits, above it where there is more room above, and before it where the
/// window's trailing edge is too near.
fn context_menu(response: &Response) -> egui::Popup<'static> {
    let ctx = response.ctx.clone();
    let menu = egui::Popup::context_menu(response);
    let origin = menu.get_id().with("origin");
    if response.secondary_clicked()
        && let Some(pos) = ctx.pointer_interact_pos()
    {
        ctx.data_mut(|data| data.insert_temp(origin, pos));
    }
    let Some(at) = ctx.data(|data| data.get_temp::<Pos2>(origin)) else {
        return menu;
    };
    let size = menu.get_expected_size().unwrap_or(Vec2::ZERO);
    let room = ctx.content_rect().shrink(8.0);
    let left = if at.x + size.x <= room.right() {
        at.x
    } else {
        at.x - size.x
    };
    let (below, above) = (room.bottom() - at.y, at.y - room.top());
    let top = if size.y <= below || below >= above {
        at.y
    } else {
        at.y - size.y
    };
    let corner = pos2(
        left.clamp(room.left(), (room.right() - size.x).max(room.left())),
        top.clamp(room.top(), (room.bottom() - size.y).max(room.top())),
    );
    menu.at_position(corner)
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site; the heads need the rows and where edits go"
)]
fn track_heads(
    ui: &mut Ui,
    heads: Rect,
    rows: &[Row],
    geometry: &Geometry,
    keys: &mut KeyframesState,
    targets: &[TrackId],
    input: &TimelineInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(heads));
    ui.set_clip_rect(heads);
    let ui = &mut ui;
    for row in rows {
        let rect = head_rect(heads, row);
        // The card runs on down beside the keyframe lane open under the
        // track's clips, which names its rows there.
        let card = rect.with_max_y(rect.bottom() + row.lane);
        if !ui.is_rect_visible(card) {
            continue;
        }
        let track = row.track;
        let id = Id::new(("track-head", track.id));
        let response = ui.is_rect_visible(rect).then(|| {
            let response = ui.interact(rect, id, Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("Track {}", track.name),
                )
            });
            response
        });
        // A track that holds a selected clip is the one edits go to: its
        // card is pressed, as its row is lit.
        let target = targets.contains(&track.id);
        let radius = CornerRadius::same(8);
        ui.painter()
            .rect_filled(card, radius, if target { p.pressed } else { p.control });
        if let Some(response) = &response {
            // Lit while the pointer is anywhere on it, its toggles included.
            let hover = controls::animate(
                ui.ctx(),
                id.with("hover"),
                response.contains_pointer(),
                theme::motion::HOVER,
            );
            if hover > 0.0 {
                ui.painter()
                    .rect_filled(card, radius, p.hover.gamma_multiply(hover));
            }
            if response.has_focus() {
                controls::focus_ring(ui.painter(), card, 8, *p);
            }
        }
        if row.lane > 0.0 {
            // What each row of the keyframe lane holds, under the head.
            let below = Rect::from_min_size(rect.left_bottom(), vec2(rect.width(), row.lane));
            keys.lane_heads(ui, below, geometry, input, actions);
        }
        let Some(response) = response else {
            continue;
        };
        // The track's tone in a stripe at the leading edge, and the glyph of
        // its kind, quieter than its name.
        let stripe = Rect::from_min_size(
            pos2(
                rect.left() + 4.0,
                rect.center().y - (rect.height() - 12.0) * 0.5,
            ),
            vec2(3.0, rect.height() - 12.0),
        );
        ui.painter().rect_filled(
            stripe,
            CornerRadius::same(2),
            p.tone(tone_of_track(track.kind)),
        );
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(14.0)),
            icon_of_track(track.kind),
            if track.hidden && track.kind != TrackKind::Audio {
                p.muted
            } else {
                p.secondary
            },
        );

        // Toggles from the trailing edge: lock, then what the track carries.
        // A head with four has them in two columns, hide under lock and
        // solo under mute.
        let toggles = head_toggles(track.kind);
        let mut placed = 0;
        let mut toggle =
            |ui: &mut Ui, key: &str, icon: Icon, on: bool, label: &str, ink: Option<Color32>| {
                let cell = head_cell(rect, toggles, placed);
                placed += 1;
                head_toggle(ui, p, cell, id.with(key), icon, on, label, ink)
            };
        let state = |locked, hidden, muted| Command::SetTrackState {
            track_id: track.id,
            locked,
            hidden,
            muted,
            name: None,
            gain: None,
            solo: None,
        };
        if toggle(
            ui,
            "lock",
            if track.locked {
                Icon::Lock
            } else {
                Icon::Unlock
            },
            track.locked,
            if track.locked {
                "Unlock track"
            } else {
                "Lock track"
            },
            None,
        ) {
            actions.push(Action::Edit(state(Some(!track.locked), None, None)));
        }
        if track.kind != TrackKind::Audio
            && toggle(
                ui,
                "hide",
                if track.hidden {
                    Icon::EyeOff
                } else {
                    Icon::Eye
                },
                track.hidden,
                if track.hidden {
                    "Show track"
                } else {
                    "Hide track"
                },
                None,
            )
        {
            actions.push(Action::Edit(state(None, Some(!track.hidden), None)));
        }
        if tracks::carries_sound(track) {
            // Silenced by another track's solo, the cell is dimmer than one
            // that is only off.
            let silenced = tracks::silenced_by_solo(input.project, track);
            if toggle(
                ui,
                "mute",
                if track.muted {
                    Icon::VolumeOff
                } else {
                    Icon::Volume
                },
                track.muted,
                mute_label(track, silenced),
                silenced.then(|| p.muted.gamma_multiply(0.5)),
            ) {
                actions.push(Action::Edit(state(None, None, Some(!track.muted))));
            }
            // A soloed track's headphones take the accent, on a pressed
            // cell as the others that are on.
            if toggle(
                ui,
                "solo",
                Icon::Headphones,
                track.solo,
                if track.solo {
                    "Unsolo track"
                } else {
                    "Solo track"
                },
                track.solo.then_some(p.accent),
            ) {
                actions.push(Action::Edit(tracks::solo(track, !track.solo)));
            }
        }

        // The name, and under it the volume of a track that is not at the
        // one it starts with, where the head is tall enough for both: one
        // block, at the head's middle.
        let room = head_name_room(rect, toggles);
        let figure = tracks::figure(track)
            .filter(|_| rect.height() >= HEAD_FIGURE)
            .map(|figure| {
                widgets::truncated(ui, &figure, theme::regular(11.0), p.secondary, room.span())
            });
        let name = widgets::truncated(
            ui,
            &track.name,
            theme::medium(12.0),
            if track.locked { p.muted } else { p.fg },
            room.span(),
        );
        // Lines of text are set two points closer than their rows, as the
        // leading of the two sizes leaves them apart.
        let tall = name.size().y + figure.as_ref().map_or(0.0, |figure| figure.size().y - 2.0);
        let top = rect.center().y - tall * 0.5;
        let under = top + name.size().y - 2.0;
        ui.painter().galley(pos2(room.min, top), name, p.fg);
        if let Some(figure) = figure {
            ui.painter()
                .galley(pos2(room.min, under), figure, p.secondary);
        }

        // A click in the menu is the volume field's to take: each row closes
        // the menu itself, so only a click outside it does.
        let menu =
            context_menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        menu.show(|ui| {
            menu_style(ui, 240.0);
            if tracks::carries_sound(track) {
                volume_menu(ui, p, track, actions);
                widgets::menu_separator(ui, p);
            }
            if menu_item(ui, p, Some(Icon::Layers), "Add track of this kind", "").clicked() {
                actions.push(Action::AddTrack(track.kind));
                ui.close();
            }
            widgets::menu_separator(ui, p);
            ui.add_enabled_ui(!track.locked, |ui| {
                let remove =
                    widgets::menu_item_destructive(ui, p, Some(Icon::Trash), "Remove track", "");
                if remove.clicked() {
                    actions.push(Action::Edit(Command::RemoveTrack { track_id: track.id }));
                    ui.close();
                }
            });
        });
    }
}

/// The tracks edits go to: those that hold a selected clip. One pass over
/// the clips however many are selected, and none with nothing selected.
fn targets(project: &Project, selection: &Selection) -> Vec<TrackId> {
    if selection.clips.is_empty() {
        return Vec::new();
    }
    let chosen: std::collections::HashSet<ClipId> = selection.clips.iter().copied().collect();
    (project.tracks.iter())
        .filter(|track| track.clips.iter().any(|clip| chosen.contains(&clip.id)))
        .map(|track| track.id)
        .collect()
}

fn draw_ruler(
    ui: &mut Ui,
    ruler: Rect,
    geometry: &Geometry,
    state: &mut TimelineState,
    input: &TimelineInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let rate = input.project.settings.frame_rate;
    let response = ui.interact(ruler, Id::new("timeline-ruler"), Sense::click_and_drag());
    let painter = ui.painter_at(ruler);

    // Ticks rise from the ruler's foot; the times stand in its upper half,
    // their figures ending above its middle, clear of the playhead's pin
    // and the markers' flags in the lower.
    let (step, parts) = rules::ruler_step(geometry.zoom, rate.fps(), 92.0);
    let first = (f64::from(geometry.scroll) / f64::from(geometry.zoom) / step).floor() as i64;
    let last = (f64::from(geometry.scroll + ruler.width()) / f64::from(geometry.zoom) / step).ceil()
        as i64;
    let foot = ruler.bottom() - 1.0;
    for index in first..=last {
        let seconds = index as f64 * step;
        let x = sharp(&painter, geometry.x(Time::from_seconds(seconds)));
        painter.vline(x, foot - 6.0..=foot, Stroke::new(1.0, p.border));
        painter.text(
            pos2(x + 3.0, ruler.top() + 1.0),
            Align2::LEFT_TOP,
            timefmt::ruler(seconds, step, rate),
            theme::regular(11.0),
            p.secondary,
        );
        for part in 1..parts {
            let x = geometry.x(Time::from_seconds(
                seconds + step * f64::from(part) / f64::from(parts),
            ));
            painter.vline(
                sharp(&painter, x),
                foot - 3.0..=foot,
                Stroke::new(1.0, p.separator),
            );
        }
    }

    // Pressing or dragging the ruler moves the playhead.
    if response.drag_started() || response.clicked() {
        state.drag = Some(Drag::Scrub);
    }
    if matches!(state.drag, Some(Drag::Scrub))
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (time, hit) = snapped(input, geometry, &state.beats, geometry.time(pos.x), &[]);
        // The playhead never snaps to itself.
        let time = if hit && time != input.playhead {
            time
        } else {
            rules::quantize(geometry.time(pos.x), rate)
        };
        actions.push(Action::Seek(time));
    }
    if matches!(state.drag, Some(Drag::Scrub)) && !response.dragged() {
        state.drag = None;
    }

    for marker in input.project.markers.iter() {
        draw_marker(ui, ruler, geometry, marker, input, actions);
    }
}

fn draw_marker(
    ui: &mut Ui,
    ruler: Rect,
    geometry: &Geometry,
    marker: &Marker,
    input: &TimelineInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let x = geometry.x(marker.time);
    if x < ruler.left() - 8.0 || x > ruler.right() + 8.0 {
        return;
    }
    let foot = ruler.bottom() - 1.0;
    // A target wider than the flag, from its top to the ruler's foot.
    let rect = Rect::from_min_max(
        pos2(x - MARK_HANDLE * 0.5 - 1.0, foot - MARKER.y - 2.0),
        pos2(x + MARK_HANDLE * 0.5 + 1.0, ruler.bottom()),
    );
    let response = ui.interact(rect, Id::new(("marker", marker.id)), Sense::click());
    let [r, g, b, _] = marker.color;
    let color = Color32::from_rgb(r, g, b);
    let painter = ui.painter_at(ruler);
    // A small flag in the marker's own colour, its point on the ruler's foot
    // at the time it marks; outlined in the ground so it stands clear of the
    // ticks, and in ink while the pointer is on it.
    let x = sharp(&painter, x);
    let half = MARKER.x * 0.5;
    let shoulder = painter.round_to_pixel_center(foot - MARKER_POINT);
    let top = painter.round_to_pixel_center(foot - MARKER.y);
    painter.add(egui::Shape::convex_polygon(
        vec![
            pos2(x - half, top),
            pos2(x + half, top),
            pos2(x + half, shoulder),
            pos2(x, foot),
            pos2(x - half, shoulder),
        ],
        color,
        Stroke::new(1.0, if response.hovered() { p.fg } else { p.bg }),
    ));
    // The keyboard's ring round the whole target, inside the ruler.
    if response.has_focus() {
        controls::focus_ring(&painter, rect.shrink(2.0), 2, *p);
    }
    if response.clicked() {
        actions.push(Action::Seek(marker.time));
    }
    let time = timefmt::timecode(marker.time, input.project.settings.frame_rate);
    let response = if marker.label.is_empty() {
        response.on_hover_text(format!("Marker at {time}"))
    } else {
        kit::rich_tooltip(response, p, &marker.label, &[&time], None)
    };
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    context_menu(&response).show(|ui| {
        menu_style(ui, 180.0);
        let remove = widgets::menu_item_destructive(ui, p, Some(Icon::Trash), "Remove marker", "");
        if remove.clicked() {
            actions.push(Action::Edit(Command::RemoveMarker {
                marker_id: marker.id,
            }));
            ui.close();
        }
    });
}

/// A marker's flag: an 8 by 7 head over a 3-point point.
const MARKER: Vec2 = vec2(8.0, 10.0);
const MARKER_POINT: f32 = 3.0;

/// The narrowest tile that carries the chevron of a keyframe lane; a
/// narrower one leaves it to the tool strip and its shortcut.
const LANE_TOGGLE_TILE: f32 = 44.0;
/// The room the chevron takes at the head of a clip's name.
const LANE_TOGGLE: f32 = 16.0;

/// Where the chevron of a keyframe lane sits on a clip's tile: at the head
/// of its name, clear of the grip that trims it.
fn lane_toggle(tile: Rect) -> Rect {
    let y = if carries_poster(tile.height()) {
        tile.top() + 10.0
    } else {
        tile.center().y
    };
    Rect::from_center_size(pos2(tile.left() + 15.0, y), Vec2::splat(15.0))
}

/// What is under a point in the lanes.
enum Hit<'a> {
    Clip {
        track: &'a Track,
        clip: &'a Clip,
        edge: Option<Edge>,
        /// The transition whose marking the point is on.
        marking: Option<TransitionEdge>,
    },
    Lane,
    Nothing,
}

/// Where a transition's marking is drawn on its clip's tile: from the edge
/// it belongs to, as wide as it is long.
///
/// On a tile with room for both it is under the name, so the name stays
/// readable and a press on it is a press on the clip.
fn marking(tile: Rect, zoom: f32, edge: TransitionEdge, duration: Time) -> Rect {
    let least = MARK_MIN.min(tile.width());
    let width = (duration.as_seconds() as f32 * zoom).clamp(least, tile.width());
    let top = if carries_poster(tile.height()) {
        tile.top() + 1.0 + NAME_ROW
    } else {
        tile.top()
    };
    match edge {
        TransitionEdge::In => Rect::from_min_max(
            pos2(tile.left(), top),
            pos2(tile.left() + width, tile.bottom()),
        ),
        TransitionEdge::Out => Rect::from_min_max(pos2(tile.right() - width, top), tile.max),
    }
}

/// The handle at a marking's inner end, kept clear of the grips that trim
/// the clip at either edge however short or long the transition is, and so
/// of the clip beside it.
fn mark_handle(tile: Rect, mark: Rect, edge: TransitionEdge) -> Rect {
    let clear = EDGE_GRIP.min(tile.width() / 3.0) + MARK_HANDLE * 0.5;
    let (low, high) = (tile.left() + clear, tile.right() - clear);
    let inner = match edge {
        TransitionEdge::In => mark.right(),
        TransitionEdge::Out => mark.left(),
    };
    // A tile too narrow for both keeps the handle at its middle.
    let x = if low <= high {
        inner.clamp(low, high)
    } else {
        tile.center().x
    };
    Rect::from_center_size(pos2(x, mark.center().y), vec2(MARK_HANDLE, mark.height()))
}

/// The part of a tile on a sound lane its sound is drawn in: under the name,
/// on a tile tall enough to give the name a row.
fn sound_area(tile: Rect) -> Rect {
    let inner = tile.shrink(1.0);
    let top = if carries_poster(tile.height()) {
        inner.top() + NAME_ROW
    } else {
        inner.top()
    };
    Rect::from_min_max(pos2(inner.left(), top), inner.max)
}

/// Where a fade of this length ends on its clip's tile, from the edge it is
/// held to and no further than the tile.
fn fade_end(tile: Rect, zoom: f32, edge: FadeEdge, length: Time) -> f32 {
    let width = (length.as_seconds() as f32 * zoom).clamp(0.0, tile.width());
    match edge {
        FadeEdge::In => tile.left() + width,
        FadeEdge::Out => tile.right() - width,
    }
}

/// The time of the project a fade of this length ends at on its clip.
fn fade_end_time(clip: &Clip, edge: FadeEdge, length: Time) -> Time {
    match edge {
        FadeEdge::In => clip.start + length,
        FadeEdge::Out => clip.end() - length,
    }
}

/// The handles of the two fades of a clip's sound, the fade in first: at the
/// top corners of the tile's sound, each where its fade ends, clear of the
/// grips that trim the clip as the handle of a transition is and of one
/// another. A tile too narrow for them has none.
fn fade_handles(tile: Rect, zoom: f32, fade_in: Time, fade_out: Time) -> Option<[Rect; 2]> {
    if tile.width() < FADE_TILE {
        return None;
    }
    let area = sound_area(tile);
    let band = area.with_max_y(area.top() + FADE_HANDLE.min(area.height()));
    let mut start = mark_handle(
        tile,
        band.with_max_x(fade_end(tile, zoom, FadeEdge::In, fade_in)),
        TransitionEdge::In,
    );
    let mut end = mark_handle(
        tile,
        band.with_min_x(fade_end(tile, zoom, FadeEdge::Out, fade_out)),
        TransitionEdge::Out,
    );
    // Fades that meet, or nearly, keep a handle each: side by side about
    // where they meet, and still clear of the grips.
    if end.left() < start.right() {
        let clear = EDGE_GRIP + MARK_HANDLE;
        let middle = ((start.center().x + end.center().x) * 0.5)
            .clamp(tile.left() + clear, tile.right() - clear);
        start = start.translate(vec2(middle - start.right(), 0.0));
        end = end.translate(vec2(middle - end.left(), 0.0));
    }
    Some([start, end])
}

/// The curve of one fade over the part of `area` from `left` to `right`: a
/// point every [`FADE_STEP`] of width and one at each end, each as high in
/// the area as [`sound::fade_at`] says the sound is let through there. The
/// fade is measured from the edge of `tile` it is held to. Empty when none
/// of the fade is in that part.
fn fade_curve(
    clip: &Clip,
    tile: Rect,
    area: Rect,
    zoom: f32,
    edge: FadeEdge,
    (left, right): (f32, f32),
) -> Vec<Pos2> {
    let length = fades::of(clip, edge);
    let end = fade_end(tile, zoom, edge, length);
    let (from, to) = match edge {
        FadeEdge::In => (tile.left().max(left), end.min(right)),
        FadeEdge::Out => (end.max(left), tile.right().min(right)),
    };
    if length.is_zero() || to <= from {
        return Vec::new();
    }
    let point = |x: f32| {
        let local = match edge {
            FadeEdge::In => Time::from_seconds(f64::from(x - tile.left()) / f64::from(zoom)),
            FadeEdge::Out => {
                clip.duration - Time::from_seconds(f64::from(tile.right() - x) / f64::from(zoom))
            }
        };
        pos2(
            x,
            area.bottom() - sound::fade_at(clip, local) * area.height(),
        )
    };
    let steps = ((to - from) / FADE_STEP).ceil() as usize;
    (0..steps)
        .map(|step| from + step as f32 * FADE_STEP)
        .chain([to])
        .map(point)
        .collect()
}

/// Draws the fades of a clip over its sound: what each takes away is
/// darkened above the curve it follows, one mesh a fade, and the curve is a
/// line along its edge.
fn paint_fades(
    painter: &egui::Painter,
    tile: Rect,
    area: Rect,
    clip: &Clip,
    zoom: f32,
    p: &Palette,
    opacity: f32,
) {
    let visible = area.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let shade = p.fade_shade().gamma_multiply(opacity);
    for edge in fades::EDGES {
        let within = (visible.left(), visible.right());
        let curve = fade_curve(clip, tile, area, zoom, edge, within);
        if curve.len() < 2 {
            continue;
        }
        let mut mesh = egui::Mesh::default();
        for (index, point) in curve.iter().enumerate() {
            mesh.colored_vertex(pos2(point.x, area.top()), shade);
            mesh.colored_vertex(*point, shade);
            if index > 0 {
                let at = 2 * index as u32;
                mesh.add_triangle(at - 2, at - 1, at);
                mesh.add_triangle(at - 1, at, at + 1);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
        painter.add(egui::Shape::line(
            curve,
            Stroke::new(1.0, p.fade_curve().gamma_multiply(opacity)),
        ));
    }
}

/// How high in a tile's sound the line of a volume stands, from 0 at the
/// bottom to 1 at the top: 100% at two thirds, 200% and more at the top.
fn volume_level(gain: f32) -> f32 {
    if gain <= 1.0 {
        gain.max(0.0) * VOLUME_WHOLE
    } else {
        let over = (gain.min(VOLUME_TOP) - 1.0) / (VOLUME_TOP - 1.0);
        VOLUME_WHOLE + over * (1.0 - VOLUME_WHOLE)
    }
}

/// The volume whose line stands this high: [`volume_level`] the other way.
fn volume_at_level(level: f32) -> f32 {
    let level = level.clamp(0.0, 1.0);
    if level <= VOLUME_WHOLE {
        level / VOLUME_WHOLE
    } else {
        1.0 + (level - VOLUME_WHOLE) / (1.0 - VOLUME_WHOLE) * (VOLUME_TOP - 1.0)
    }
}

/// The least and the greatest volume a clip is given.
fn volume_bounds() -> (f32, f32) {
    (look::clip_spec().iter())
        .find(|spec| spec.key == ParamKey::Gain)
        .map_or((0.0, 8.0), |spec| (spec.min, spec.max))
}

/// The volume a line taken hold of at `level` has once the pointer has gone
/// `rise` points up in a sound `height` points high: a whole percent, within
/// what a clip is given. A line that stands where the `stored` volume is
/// drawn keeps that volume, which may be finer or louder than a line shows.
fn volume_dragged(stored: f32, level: f32, rise: f32, height: f32) -> f32 {
    let (least, most) = volume_bounds();
    let whole = |gain: f32| ((gain * 100.0).round() / 100.0).clamp(least, most);
    let wanted = whole(volume_at_level(level + rise / height.max(1.0)));
    if wanted == whole(stored.min(VOLUME_TOP)) {
        stored
    } else {
        wanted
    }
}

/// The time of its clip a point of a tile shows, from the clip's start.
fn tile_time(tile: Rect, zoom: f32, x: f32) -> Time {
    Time::from_seconds(f64::from(x - tile.left()) / f64::from(zoom))
}

/// Where the volume line of `clip` is at `x` of its tile.
fn volume_y(clip: &Clip, tile: Rect, area: Rect, zoom: f32, x: f32) -> f32 {
    let gain = gain_at(clip, clip.start + tile_time(tile, zoom, x));
    area.bottom() - volume_level(gain) * area.height()
}

/// The volume line of a clip over the part of `area` from `left` to `right`:
/// level at the clip's volume, or, where keyframes drive it, the track's own
/// evaluation a point every [`keyframes::SAMPLE`] of width and one at each
/// key, as the graph draws a curve. Empty when none of the tile is in that
/// part.
fn volume_line(
    clip: &Clip,
    tile: Rect,
    area: Rect,
    zoom: f32,
    (left, right): (f32, f32),
) -> Vec<Pos2> {
    let (from, to) = (tile.left().max(left), tile.right().min(right));
    if to <= from {
        return Vec::new();
    }
    let point = |x: f32, local: Time| {
        let gain = gain_at(clip, clip.start + local);
        pos2(x, area.bottom() - volume_level(gain) * area.height())
    };
    let local = |x: f32| tile_time(tile, zoom, x);
    let Some(track) =
        (clip.animation.iter()).find(|track| track.target.is_none() && track.key == ParamKey::Gain)
    else {
        return vec![point(from, local(from)), point(to, local(to))];
    };
    // From one key to the next, so a corner and a held step are drawn where
    // they are and not between two samples.
    let mut stops = vec![(from, local(from))];
    stops.extend(
        (track.keys.iter())
            .map(|key| (tile.left() + key.at.as_seconds() as f32 * zoom, key.at))
            .filter(|(x, _)| *x > from && *x < to),
    );
    stops.push((to, local(to)));
    let step = keyframes::SAMPLE;
    let mut points = Vec::with_capacity(((to - from) / step) as usize + stops.len() * 2);
    for pair in stops.windows(2) {
        let ((start, first), (end, last)) = (pair[0], pair[1]);
        points.push(point(start, first));
        let mut x = start + step;
        while x < end {
            points.push(point(x, local(x)));
            x += step;
        }
        points.push(point(end, Time((last.0 - 1).max(first.0))));
    }
    points.push(point(to, local(to)));
    points
}

/// The percent a held volume line says.
fn volume_percent(clip: &Clip) -> String {
    format!("{}%", (clip.gain * 100.0).round())
}

/// Where the percent of a held volume line is drawn on its tile, `size`
/// large: beside the point at `x` it was taken hold of, after it or, where
/// the room after it is too narrow, before it; above the line at `y`, or
/// under it where the name's row leaves no room above. It is kept inside the
/// tile's sound and clear of the grips, of both fades' curves and of their
/// handles, so a point taken hold of over a fade has it said just past that
/// fade. `None` when the room between the fades is too narrow for it.
fn volume_readout(
    tile: Rect,
    area: Rect,
    clip: &Clip,
    zoom: f32,
    (x, y): (f32, f32),
    size: Vec2,
) -> Option<Rect> {
    let mut low = area.left() + EDGE_GRIP + READOUT_GAP;
    let mut high = area.right() - EDGE_GRIP - READOUT_GAP;
    if !clip.fade_in.is_zero() {
        low = low.max(fade_end(tile, zoom, FadeEdge::In, clip.fade_in) + READOUT_GAP);
    }
    if !clip.fade_out.is_zero() {
        high = high.min(fade_end(tile, zoom, FadeEdge::Out, clip.fade_out) - READOUT_GAP);
    }
    if let Some([fade_in, fade_out]) = fade_handles(tile, zoom, clip.fade_in, clip.fade_out) {
        low = low.max(fade_in.right() + READOUT_GAP);
        high = high.min(fade_out.left() - READOUT_GAP);
    }
    if high - low < size.x || area.height() < size.y {
        return None;
    }
    let after = x + READOUT_GAP;
    let left = if after + size.x <= high {
        after
    } else {
        x - READOUT_GAP - size.x
    };
    let left = left.clamp(low, high - size.x);
    let top = if y - area.top() >= size.y + 1.0 {
        y - 1.0 - size.y
    } else {
        y + 2.0
    };
    let top = top.clamp(area.top(), area.bottom() - size.y);
    Some(Rect::from_min_size(pos2(left, top), size))
}

/// Draws the volume line of a clip over its sound, a diamond at each volume
/// keyframe in view, and beside a held line the volume it is held at, as
/// [`volume_readout`] places it. Held, the line takes the accent and `held`
/// is where across it was taken hold of. Whether the held percent was said:
/// when the fades leave it no room, the name's row says it.
fn paint_volume(
    painter: &egui::Painter,
    (tile, area): (Rect, Rect),
    clip: &Clip,
    zoom: f32,
    p: &Palette,
    held: Option<f32>,
    opacity: f32,
) -> bool {
    let visible = area.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return false;
    }
    let within = (visible.left(), visible.right());
    let line = volume_line(clip, tile, area, zoom, within);
    if line.is_empty() {
        return false;
    }
    let ink = match held {
        Some(_) => p.accent,
        None => p.volume_line().gamma_multiply(opacity),
    };
    painter.add(egui::Shape::line(line, Stroke::new(1.5, ink)));
    let keys = (clip.animation.iter())
        .find(|track| track.target.is_none() && track.key == ParamKey::Gain)
        .map_or(&[][..], |track| &track.keys);
    let key = match held {
        Some(_) => p.accent,
        None => p.key().gamma_multiply(opacity),
    };
    for at in keys {
        let x = tile.left() + at.at.as_seconds() as f32 * zoom;
        if x >= within.0 - 4.0 && x <= within.1 + 4.0 {
            let gain = gain_at(clip, clip.start + at.at);
            let y = area.bottom() - volume_level(gain) * area.height();
            painter.add(diamond(pos2(x, y), VOLUME_KEY, key));
        }
    }
    let Some(x) = held else {
        return false;
    };
    let galley = painter.layout_no_wrap(volume_percent(clip), theme::mono(11.0), Color32::WHITE);
    let y = volume_y(clip, tile, area, zoom, x);
    match volume_readout(tile, area, clip, zoom, (x, y), galley.size()) {
        Some(readout) => {
            readout_pill(painter, readout);
            painter.galley(readout.min, galley, Color32::WHITE);
            true
        }
        None => false,
    }
}

/// How wide a diamond at a key of the volume line is, corner to corner.
const VOLUME_KEY: f32 = 7.0;

/// A diamond `size` wide, corner to corner, about `centre`.
fn diamond(centre: Pos2, size: f32, fill: Color32) -> egui::Shape {
    let half = size * 0.5;
    egui::Shape::convex_polygon(
        vec![
            centre - vec2(0.0, half),
            centre + vec2(half, 0.0),
            centre + vec2(0.0, half),
            centre - vec2(half, 0.0),
        ],
        fill,
        Stroke::NONE,
    )
}

/// The dark pill a readout over a tile sits on: four points either side of
/// its words and one above and under them.
fn readout_pill(painter: &egui::Painter, words: Rect) {
    let pill = words.expand2(vec2(4.0, 1.0));
    let radius = (pill.height() * 0.5).round() as u8;
    painter.rect_filled(pill, radius, Color32::from_black_alpha(140));
}

/// The volume line of a clip's tile where `pos` is on it, as how high it
/// stands there and how high the tile's sound is: on a sound lane that takes
/// an edit, of a clip whose media has sound, on a tile wide enough to have a
/// body between the grips that trim it.
fn volume_grip(
    project: &Project,
    rows: &[Row],
    geometry: &Geometry,
    track: &Track,
    clip: &Clip,
    pos: Pos2,
) -> Option<(f32, f32)> {
    if track.kind != TrackKind::Audio || track.locked || !fades::has_sound(project, clip) {
        return None;
    }
    let row = rows.iter().find(|row| row.track.id == track.id)?;
    let tile = clip_rect(geometry, row, clip.start, clip.end());
    let area = sound_area(tile);
    if tile.width() < FADE_TILE || area.height() < 1.0 {
        return None;
    }
    let y = volume_y(clip, tile, area, geometry.zoom, pos.x);
    ((pos.y - y).abs() <= VOLUME_REACH)
        .then(|| ((area.bottom() - y) / area.height(), area.height()))
}

/// What the handle of a fade is called.
fn fade_name(clip: &Clip, edge: FadeEdge) -> String {
    format!("{} of {}", fades::name(edge), clip.name)
}

/// An edge under the pointer that could be given a transition, and the
/// badge that offers it.
struct AddSpot {
    clip: ClipId,
    edge: TransitionEdge,
    badge: Rect,
}

/// Whether `pos` is on the disc drawn in a badge's box.
fn on_badge(badge: Rect, pos: Pos2) -> bool {
    pos.distance(badge.center()) <= ADD_BADGE * 0.5
}

/// The cut or the free edge within reach of `pos` that has no transition
/// yet. A cut is its incoming clip's start.
fn add_spot(rows: &[Row], geometry: &Geometry, pos: Pos2) -> Option<AddSpot> {
    let row = rows
        .iter()
        .find(|row| pos.y >= row.top && pos.y <= row.top + row.height)?;
    let track = row.track;
    if track.locked || track.kind == TrackKind::Audio {
        return None;
    }
    let at = geometry.time(pos.x);
    let spot = |clip: &Clip, edge, x: f32| AddSpot {
        clip: clip.id,
        edge,
        badge: Rect::from_center_size(
            pos2(x, row.top + row.height - ADD_BADGE * 0.5 - 3.0),
            Vec2::splat(ADD_BADGE),
        ),
    };
    // A badge set in from a free edge reaches further than the edge does:
    // the pointer is still at that edge while it is anywhere on the badge.
    let inset = ADD_BADGE * 0.5 + 2.0;
    let abuts = |clip: &Clip, start: bool| {
        (track.clips.iter()).any(|other| {
            other.id != clip.id
                && if start {
                    other.end() == clip.start
                } else {
                    other.start == clip.end()
                }
        })
    };
    let reach_of = |clip: &Clip, start: bool| {
        if abuts(clip, start) {
            ADD_REACH
        } else {
            ADD_REACH.max(inset + ADD_BADGE * 0.5)
        }
    };
    let reach = geometry.span(ADD_REACH.max(inset + ADD_BADGE * 0.5));
    for clip in clips_in_view(track, at - reach, at + reach + Time(1)) {
        let tile = clip_rect(geometry, row, clip.start, clip.end());
        if tile.width() < ADD_TILE {
            continue;
        }
        if (pos.x - tile.left()).abs() <= reach_of(clip, true) {
            if clip.transition_in.is_some() {
                continue;
            }
            let cut =
                (track.clips.iter()).any(|other| other.id != clip.id && other.end() == clip.start);
            let x = tile.left() + if cut { 0.0 } else { inset };
            return Some(spot(clip, TransitionEdge::In, x));
        }
        if (pos.x - tile.right()).abs() <= reach_of(clip, false) {
            let next =
                (track.clips.iter()).find(|other| other.id != clip.id && other.start == clip.end());
            return match next {
                Some(next) if next.transition_in.is_none() => {
                    Some(spot(next, TransitionEdge::In, tile.right()))
                }
                Some(_) => None,
                None if clip.transition_out.is_none() => {
                    Some(spot(clip, TransitionEdge::Out, tile.right() - inset))
                }
                None => None,
            };
        }
    }
    None
}

/// What a transition's marking and its handle are called.
fn marking_name(clip: &Clip, edge: TransitionEdge, transition: &Transition) -> String {
    format!(
        "{} at the {} of {}",
        transitions::name(transition.kind),
        transitions::edge_name(edge),
        clip.name
    )
}

fn clip_rect(geometry: &Geometry, row: &Row, start: Time, end: Time) -> Rect {
    Rect::from_min_max(
        pos2(geometry.x(start), row.top),
        pos2(
            geometry.x(end).max(geometry.x(start) + 2.0),
            row.top + row.height,
        ),
    )
}

fn hit<'a>(rows: &[Row<'a>], geometry: &Geometry, pos: Pos2) -> Hit<'a> {
    let Some(row) = rows
        .iter()
        .find(|row| pos.y >= row.top && pos.y <= row.top + row.height)
    else {
        return Hit::Nothing;
    };
    // Later clips are drawn over earlier ones, so they are hit first.
    for clip in row.track.clips.iter().rev() {
        let rect = clip_rect(geometry, row, clip.start, clip.end());
        if pos.x >= rect.left() && pos.x <= rect.right() {
            // Short clips keep a body to hold: grips never take more than a
            // third of the clip each.
            let grip = EDGE_GRIP.min(rect.width() / 3.0);
            let edge = if row.track.locked {
                None
            } else if pos.x - rect.left() <= grip {
                Some(Edge::Start)
            } else if rect.right() - pos.x <= grip {
                Some(Edge::End)
            } else {
                None
            };
            let marking = transitions::EDGES.into_iter().find(|side| {
                transitions::of(clip, *side).is_some_and(|transition| {
                    let mark = marking(rect, geometry.zoom, *side, transition.duration);
                    pos.x >= mark.left() && pos.x <= mark.right() && pos.y >= mark.top()
                })
            });
            return Hit::Clip {
                track: row.track,
                clip,
                edge,
                marking,
            };
        }
    }
    Hit::Lane
}

/// How far apart the lines that hatch a locked track are.
const HATCH: f32 = 6.0;

/// Hatches `rect` with lines at 45 degrees, `HATCH` apart, cut to it: a line
/// segment each, no mesh.
fn paint_hatch(painter: &egui::Painter, rect: Rect, ink: Color32) {
    let visible = rect.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let stroke = Stroke::new(1.0, ink);
    // Each line rises to the right, so x + y is the same along it; the
    // lines are those where it is a whole number of steps.
    let (top, bottom) = (rect.top(), rect.bottom());
    let mut along = ((visible.left() + top) / HATCH).floor() * HATCH;
    while along <= visible.right() + bottom {
        // From where the line enters the rect to where it leaves.
        let mut from = pos2(along - bottom, bottom);
        let mut to = pos2(along - top, top);
        if from.x < rect.left() {
            from = pos2(rect.left(), along - rect.left());
        }
        if to.x > rect.right() {
            to = pos2(rect.right(), along - rect.right());
        }
        if from.x <= to.x {
            painter.line_segment([from, to], stroke);
        }
        along += HATCH;
    }
}

/// The dashed outline of a rounded rectangle, as line segments.
fn dashed_outline(rect: Rect, radius: f32, stroke: Stroke) -> Vec<egui::Shape> {
    let radius = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let inner = rect.shrink(radius);
    // Each corner's centre, and the angle its quarter starts at, clockwise
    // from the top left.
    let corners = [
        (inner.left_top(), std::f32::consts::PI),
        (inner.right_top(), std::f32::consts::PI * 1.5),
        (inner.right_bottom(), 0.0),
        (inner.left_bottom(), std::f32::consts::FRAC_PI_2),
    ];
    let mut path = Vec::with_capacity(4 * 7 + 1);
    for (centre, from) in corners {
        for step in 0..=6 {
            let angle = from + std::f32::consts::FRAC_PI_2 * step as f32 / 6.0;
            path.push(centre + radius * vec2(angle.cos(), angle.sin()));
        }
    }
    path.push(path[0]);
    egui::Shape::dashed_line(&path, stroke, 4.0, 3.0)
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site; the lanes need all of it"
)]
fn draw_lanes(
    ui: &mut Ui,
    lanes: Rect,
    rows: &[Row],
    geometry: &Geometry,
    state: &mut TimelineState,
    selection: &mut Selection,
    // The tracks edits go to, which are lit.
    targets: &[TrackId],
    textures: &mut Textures,
    peaks: &mut HeldPeaks,
    input: &TimelineInput,
    actions: &mut Vec<Action>,
) -> egui::layers::ShapeIdx {
    let p = input.p;
    let project = input.project;
    let rate = project.settings.frame_rate;
    let response = ui.interact(lanes, Id::new("timeline-lanes"), Sense::click_and_drag());
    // The tracks are one stop for the keyboard; the clip it selected is
    // what they are named for.
    let pointed = (state.keyboard)
        .and_then(|id| project.clip(id))
        .map(|(track, clip)| format!("Tracks: {} on {} selected", clip.name, track.name));
    response.widget_info(|| {
        let label = pointed.as_deref().unwrap_or("Tracks");
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, label)
    });
    let painter = ui.painter_at(lanes);
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let over = pointer.filter(|pos| lanes.contains(*pos) && response.contains_pointer());
    let modifiers = ui.input(|i| i.modifiers);
    let extend = modifiers.command || modifiers.shift;

    // Lanes: a band for each track, the ground showing between them and
    // under the last. The tracks edits go to are lit, and a locked one is
    // hatched under its tiles.
    for row in rows {
        let lane = Rect::from_min_max(
            pos2(lanes.left(), row.top),
            pos2(lanes.right(), row.top + row.height),
        );
        if !lane.intersects(lanes) {
            continue;
        }
        let fill = if targets.contains(&row.track.id) {
            p.lane_target()
        } else {
            p.lane()
        };
        painter.rect_filled(lane, CornerRadius::ZERO, fill);
        if row.track.locked {
            paint_hatch(&painter, lane, p.lane_hatch());
        }
    }
    // The row a drag would drop on is lit when the drag knows it, under the
    // tiles.
    let drop_row = painter.add(egui::Shape::Noop);
    let lit = |row: &Row| {
        egui::Shape::rect_filled(
            Rect::from_min_max(
                pos2(lanes.left(), row.top),
                pos2(lanes.right(), row.top + row.height),
            ),
            CornerRadius::ZERO,
            p.lane_target(),
        )
    };
    // An empty timeline shows where media goes.
    if project.tracks.iter().all(|track| track.clips.is_empty())
        && let Some(row) = (rows.iter())
            .find(|row| row.track.kind == TrackKind::Video)
            .or(rows.first())
    {
        let ghost = Rect::from_min_max(
            pos2(lanes.left(), row.top),
            pos2(lanes.right(), row.top + row.height),
        )
        .shrink2(vec2(8.0, 4.0));
        painter.extend(dashed_outline(
            ghost,
            f32::from(metrics::CLIP_RADIUS),
            Stroke::new(1.0, p.border),
        ));
        painter.text(
            ghost.center(),
            Align2::CENTER_CENTER,
            "Drag media here",
            theme::regular(12.0),
            p.muted,
        );
    }

    // The tiles that carry the chevron of a keyframe lane: a selected clip
    // with animation, and the clip whose lane is open, which for a clip
    // with sound may be the empty row of its volume.
    let open = (state.keys.lane_rows(project)).and_then(|_| state.keys.lane_clip());
    let mut toggles: Vec<(ClipId, Rect)> = Vec::new();
    for (row, clip) in clips_drawn(rows, geometry) {
        let animated = !clip.animation.is_empty() && selection.contains(clip.id);
        if !(animated || open == Some(clip.id)) {
            continue;
        }
        let tile = clip_rect(geometry, row, clip.start, clip.end());
        if tile.width() >= LANE_TOGGLE_TILE {
            toggles.push((clip.id, lane_toggle(tile)));
        }
    }
    let band = rows.iter().find(|row| row.lane > 0.0).map(|row| {
        let top = row.top + row.height;
        Rect::from_min_max(pos2(lanes.left(), top), pos2(lanes.right(), top + row.lane))
    });
    // The handles of the transitions in view, on tracks that take an edit.
    // One being dragged is where it is held.
    let held_mark = match &state.drag {
        Some(Drag::Transition {
            clip,
            edge,
            duration,
        }) => Some((*clip, *edge, *duration)),
        _ => None,
    };
    let mut handles: Vec<(ClipId, TransitionEdge, Rect)> = Vec::new();
    for (row, clip) in clips_drawn(rows, geometry) {
        if row.track.locked || (clip.transition_in.is_none() && clip.transition_out.is_none()) {
            continue;
        }
        let tile = clip_rect(geometry, row, clip.start, clip.end());
        for edge in transitions::EDGES {
            let Some(transition) = transitions::of(clip, edge) else {
                continue;
            };
            let duration = held_mark
                .filter(|held| (held.0, held.1) == (clip.id, edge))
                .map_or(transition.duration, |held| held.2);
            let mark = marking(tile, geometry.zoom, edge, duration);
            handles.push((clip.id, edge, mark_handle(tile, mark, edge)));
        }
    }
    // The edge under the pointer that could be given a transition. Asked of
    // the layer and not of the lanes' response, which the badge covers.
    let carrying = egui::DragAndDrop::has_any_payload(ui.ctx());
    let free = pointer
        .filter(|pos| lanes.contains(*pos) && ui.rect_contains_pointer(lanes))
        .filter(|_| state.drag.is_none() && !carrying);
    let spot = free
        .and_then(|pos| add_spot(rows, geometry, pos))
        .filter(|spot| !handles.iter().any(|(.., rect)| rect.intersects(spot.badge)));
    // The handles of the fades of the sound tiles in view, on tracks that
    // take an edit: of the selected clips, of the clip under the pointer
    // and of the one whose fade is held, which is where it is held. Asked
    // of the layer too: a handle covers the lanes where it stands.
    let held_fade = match &state.drag {
        Some(Drag::Fade {
            clip,
            edge,
            duration,
            ..
        }) => Some((*clip, *edge, *duration)),
        _ => None,
    };
    let under = free.and_then(|pos| match hit(rows, geometry, pos) {
        Hit::Clip { clip, .. } => Some(clip.id),
        _ => None,
    });
    // A tile being carried or trimmed is drawn where it is held, and its
    // handles would stand where it was: none is shown until it is let go.
    let reshaping = match &state.drag {
        Some(Drag::Move { moved, .. }) => *moved,
        Some(Drag::Trim { delta, .. }) => *delta != Time::ZERO,
        _ => false,
    };
    let mut faders: Vec<(ClipId, FadeEdge, Rect)> = Vec::new();
    for (row, clip) in clips_drawn(rows, geometry) {
        if reshaping || row.track.kind != TrackKind::Audio || row.track.locked {
            continue;
        }
        let held = held_fade.filter(|held| held.0 == clip.id);
        if !(selection.contains(clip.id) || under == Some(clip.id) || held.is_some()) {
            continue;
        }
        let length = |edge| {
            held.filter(|held| held.1 == edge)
                .map_or(fades::of(clip, edge), |held| held.2)
        };
        let tile = clip_rect(geometry, row, clip.start, clip.end());
        let (fade_in, fade_out) = (length(FadeEdge::In), length(FadeEdge::Out));
        if let Some(rects) = fade_handles(tile, geometry.zoom, fade_in, fade_out) {
            faders.extend(
                fades::EDGES
                    .into_iter()
                    .zip(rects)
                    .map(|(edge, rect)| (clip.id, edge, rect)),
            );
        }
    }
    // What a press on the lane, a chevron or a handle does is theirs, not
    // the clips'. The badge takes no press from the edge it sits on: see
    // `offer` below.
    let claimed = |pos: Pos2| {
        band.is_some_and(|band| band.contains(pos))
            || toggles.iter().any(|(_, rect)| rect.contains(pos))
            || handles.iter().any(|(.., rect)| rect.contains(pos))
            || faders.iter().any(|(.., rect)| rect.contains(pos))
    };

    // ---- Pointer: press, drag, release ------------------------------------
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    if pressed
        && let Some(pos) = over
        && !claimed(pos)
    {
        state.keys.let_go();
        state.snap_guide = None;
        state.keyboard = None;
        match hit(rows, geometry, pos) {
            // Alt on a volume line keys the volume at the frame under the
            // pointer, at the volume it has there, and opens the lane the
            // key is then in.
            Hit::Clip {
                track,
                clip,
                edge: None,
                ..
            } if modifiers.alt
                && volume_grip(project, rows, geometry, track, clip, pos).is_some() =>
            {
                selection.only(clip.id);
                selection.asset = None;
                state.transition = None;
                state.drag = None;
                let at = rules::quantize(geometry.time(pos.x), rate).clamp(clip.start, clip.end());
                let keyed = keyframes::key_volume(clip, at - clip.start);
                actions.extend(keyed.map(Action::Edit));
                state.keys.set_lane(Some(clip.id));
            }
            Hit::Clip {
                track,
                clip,
                edge,
                marking,
            } => {
                // The line of a clip that was selected before this press,
                // and whose volume no keyframe drives, is taken hold of:
                // anywhere else, and on any other clip, the press is the
                // clip's.
                let line = (edge.is_none() && !extend && selection.contains(clip.id))
                    .then(|| volume_grip(project, rows, geometry, track, clip, pos))
                    .flatten()
                    .filter(|_| !sound::volume_keyed(clip));
                if extend {
                    selection.toggle(clip.id);
                } else if !selection.contains(clip.id) {
                    selection.only(clip.id);
                }
                selection.asset = None;
                // A press on a marking selects its transition with the
                // clip; one anywhere else on the clip lets it go.
                state.transition = marking
                    .filter(|_| edge.is_none() && !extend)
                    .map(|side| (clip.id, side));
                if state.transition.is_some() {
                    selection.only(clip.id);
                }
                state.drag = if track.locked {
                    None
                } else if let Some(edge) = edge {
                    Some(Drag::Trim {
                        clip: clip.id,
                        edge,
                        delta: Time::ZERO,
                    })
                } else if let Some((level, height)) = line {
                    Some(Drag::Volume {
                        clip: clip.id,
                        gain: clip.gain,
                        stored: clip.gain,
                        level,
                        height,
                        from: pos.y,
                        at: geometry.time(pos.x),
                        moved: false,
                    })
                } else if selection.contains(clip.id) {
                    Some(Drag::Move {
                        grabbed: clip.id,
                        origin: geometry.time(pos.x),
                        delta: Time::ZERO,
                        target: None,
                        moved: false,
                    })
                } else {
                    None
                };
            }
            Hit::Lane | Hit::Nothing => {
                state.transition = None;
                state.drag = Some(Drag::Marquee {
                    from: pos,
                    base: if extend {
                        selection.clips.clone()
                    } else {
                        Vec::new()
                    },
                });
            }
        }
        // The badge sits on the edge it offers a transition for, where a
        // press also trims or moves the clip. The press took hold of the
        // clip as anywhere else on that edge; the badge has it only for as
        // long as the pointer stays where it went down.
        // The badge is the disc that was drawn, in the frame before this
        // press: not the box around it, and not one the press brought.
        state.offer = (spot.as_ref())
            .filter(|spot| state.badge == Some((spot.clip, spot.edge, spot.badge)))
            .filter(|spot| on_badge(spot.badge, pos))
            .map(|spot| (spot.clip, spot.edge, pos));
    }

    let released = ui.input(|i| i.pointer.primary_released());
    let dragging = response.dragged() || (response.is_pointer_button_down_on() && !pressed);
    // A press on the badge that has not travelled moves nothing yet.
    let offered = match (state.offer, pointer) {
        (Some((.., from)), Some(pos)) if pos.distance(from) < OFFER_TRAVEL => true,
        _ => {
            state.offer = None;
            false
        }
    };
    if released && let Some((clip, edge, _)) = state.offer.take() {
        // Let go where it was pressed: a click on the badge, and nothing
        // the press held is trimmed or moved.
        state.drag = None;
        state.snap_guide = None;
        actions.push(Action::AddTransitions {
            places: vec![(clip, edge)],
            kind: None,
        });
    }
    let mut marquee_rect = None;
    match &mut state.drag {
        _ if offered => {}
        Some(Drag::Move {
            grabbed,
            origin,
            delta,
            target,
            moved,
        }) if dragging || released => {
            if let (Some(pos), Some((track, clip))) = (pointer, project.clip(*grabbed)) {
                if response.dragged() {
                    *moved = true;
                }
                let moving = rules::related(project, &selection.clips, true);
                let wanted = geometry.time(pos.x.max(lanes.left() - 2000.0)) - *origin;
                // The beats of the clips held move with them.
                let beats: Vec<(ClipId, Time)> = (state.beats.iter())
                    .filter(|(id, _)| !moving.contains(id))
                    .copied()
                    .collect();
                // Whichever edge of the held clip is nearer a guide wins.
                let (start, start_hit) =
                    snapped(input, geometry, &beats, clip.start + wanted, &moving);
                let (end, end_hit) = snapped(input, geometry, &beats, clip.end() + wanted, &moving);
                let mut guide = None;
                let mut next = rules::quantize(clip.start + wanted, rate) - clip.start;
                let start_reach = (start.0 - (clip.start + wanted).0).abs();
                let end_reach = (end.0 - (clip.end() + wanted).0).abs();
                if start_hit && (!end_hit || start_reach <= end_reach) {
                    next = start - clip.start;
                    guide = Some(start);
                } else if end_hit {
                    next = end - clip.end();
                    guide = Some(end);
                }
                // Another lane of the same kind takes the held clips when
                // they all come from one track.
                let same_track = selection
                    .clips
                    .iter()
                    .all(|id| project.clip(*id).is_some_and(|(t, _)| t.id == track.id));
                *target = rows
                    .iter()
                    .find(|row| pos.y >= row.top && pos.y <= row.top + row.height + LANE_GAP)
                    .map(|row| row.track)
                    .filter(|other| {
                        same_track
                            && other.id != track.id
                            && other.kind == track.kind
                            && !other.locked
                    })
                    .map(|other| other.id);
                match rules::resolve_move(project, &selection.clips, next, *target) {
                    Some(resolved) => {
                        if resolved != next {
                            guide = None;
                        }
                        *delta = resolved;
                    }
                    // No room on that lane: stay on the clip's own.
                    None => {
                        *target = None;
                        *delta = rules::resolve_move(project, &selection.clips, next, None)
                            .unwrap_or(Time::ZERO);
                        guide = None;
                    }
                }
                state.snap_guide = guide;
            }
        }
        Some(Drag::Trim { clip, edge, delta }) if dragging || released => {
            // A press that has not travelled is a click on the clip: the
            // edge stays where it is however far inside the grip it landed.
            let travelled = response.dragged() || *delta != Time::ZERO;
            if let (Some(pos), Some((track, clip))) = (pointer, project.clip(*clip))
                && travelled
            {
                let edge_time = match edge {
                    Edge::Start => clip.start,
                    Edge::End => clip.end(),
                };
                let raw = geometry.time(pos.x);
                let (time, hit) = snapped(input, geometry, &state.beats, raw, &[clip.id]);
                let time = if hit {
                    time
                } else {
                    rules::quantize(raw, rate)
                };
                let wanted = time - edge_time;
                *delta = rules::trim_delta(project, track, clip, *edge, wanted);
                state.snap_guide = (hit && *delta == wanted).then_some(time);
            }
        }
        Some(Drag::Transition {
            clip,
            edge,
            duration,
        }) => {
            if let (Some(pos), Some((_, clip))) = (pointer, project.clip(*clip))
                && let Some((shortest, longest)) = transitions::lengths(project, clip)
            {
                let raw = geometry.time(pos.x);
                let (time, hit) = snapped(input, geometry, &state.beats, raw, &[clip.id]);
                let time = if hit {
                    time
                } else {
                    rules::quantize(raw, rate)
                };
                let wanted = match edge {
                    TransitionEdge::In => time - clip.start,
                    TransitionEdge::Out => clip.end() - time,
                };
                *duration = wanted.clamp(shortest, longest);
                state.snap_guide = (hit && *duration == wanted).then_some(time);
            }
        }
        Some(Drag::Fade {
            clip,
            edge,
            duration,
            grab,
        }) => {
            if let (Some(pos), Some((_, clip))) = (pointer, project.clip(*clip)) {
                let stored = fades::of(clip, *edge);
                let raw = geometry.time(pos.x) - *grab;
                if raw == fade_end_time(clip, *edge, stored) {
                    // Where it was taken hold of the fade is the one it
                    // has, on a frame or not.
                    *duration = stored;
                    state.snap_guide = None;
                } else {
                    let (time, hit) = snapped(input, geometry, &state.beats, raw, &[clip.id]);
                    let time = if hit {
                        time
                    } else {
                        rules::quantize(raw, rate)
                    };
                    let wanted = match edge {
                        FadeEdge::In => time - clip.start,
                        FadeEdge::Out => clip.end() - time,
                    };
                    // No further than the clip's other fade, nor than the
                    // clip.
                    *duration = wanted.clamp(Time::ZERO, fades::room(clip, *edge));
                    state.snap_guide = (hit && *duration == wanted).then_some(time);
                }
            }
        }
        Some(Drag::Volume {
            gain,
            stored,
            level,
            height,
            from,
            moved,
            ..
        }) => {
            if let Some(pos) = pointer {
                *moved |= (pos.y - *from).abs() >= VOLUME_TRAVEL;
                if *moved {
                    *gain = volume_dragged(*stored, *level, *from - pos.y, *height);
                }
            }
        }
        Some(Drag::Marquee { from, base }) if dragging || released => {
            if let Some(pos) = pointer
                && response.dragged()
            {
                let rect = Rect::from_two_pos(*from, pos);
                marquee_rect = Some(rect);
                let mut picked = base.clone();
                for row in rows {
                    if rect.bottom() < row.top || rect.top() > row.top + row.height {
                        continue;
                    }
                    for clip in &row.track.clips {
                        let body = clip_rect(geometry, row, clip.start, clip.end());
                        if body.intersects(rect) && !picked.contains(&clip.id) {
                            picked.push(clip.id);
                        }
                    }
                }
                selection.clips = picked;
                selection.asset = None;
            }
        }
        _ => {}
    }

    if released && let Some(drag) = state.drag.take() {
        state.snap_guide = None;
        match drag {
            Drag::Move {
                grabbed,
                delta,
                target,
                moved,
                ..
            } => {
                if moved && (delta != Time::ZERO || target.is_some()) {
                    actions.push(Action::Edit(Command::MoveClips {
                        clip_ids: selection.clips.clone(),
                        delta,
                        target_track: target,
                    }));
                } else if !moved && !extend {
                    // A plain click on one of several selected clips keeps
                    // only that one.
                    selection.only(grabbed);
                }
            }
            Drag::Trim { clip, edge, delta } => {
                if delta != Time::ZERO
                    && let Some((_, clip)) = project.clip(clip)
                {
                    actions.push(Action::Edit(trim_command(clip, edge, delta)));
                }
            }
            Drag::Transition {
                clip,
                edge,
                duration,
            } => {
                // The clip and the edge taken hold of, whatever is selected
                // by now.
                if let Some((_, held)) = project.clip(clip)
                    && let Some(stored) = transitions::of(held, edge)
                    && stored.duration != duration
                {
                    actions.push(Action::Edit(transitions::set_command(
                        clip,
                        edge,
                        Some(Transition {
                            duration,
                            ..stored.clone()
                        }),
                    )));
                }
            }
            Drag::Fade {
                clip,
                edge,
                duration,
                ..
            } => {
                // The clip and the edge taken hold of, whatever is selected
                // by now.
                if let Some((_, held)) = project.clip(clip)
                    && fades::of(held, edge) != duration
                {
                    actions.push(Action::Edit(fades::set_command(held, edge, duration)));
                }
            }
            Drag::Volume {
                clip, gain, moved, ..
            } => {
                // The clip taken hold of, as it is now: keyframes written
                // since drive its volume, and the line is no longer one.
                if moved {
                    if let Some((_, held)) = project.clip(clip)
                        && !sound::volume_keyed(held)
                        && held.gain != gain
                    {
                        actions.push(Action::Edit(Command::SetGain {
                            clip_id: clip,
                            gain,
                        }));
                    }
                } else if !extend {
                    // A plain click on one of several selected clips keeps
                    // only that one.
                    selection.only(clip);
                }
            }
            Drag::Marquee { from, .. } => {
                // A press that did not travel is a click on empty space: it
                // clears the selection and moves the playhead there.
                if !response.dragged()
                    && pointer.is_some_and(|pos| pos.distance(from) < 3.0)
                    && !extend
                {
                    selection.clear();
                    actions.push(Action::Seek(rules::quantize(geometry.time(from.x), rate)));
                }
            }
            Drag::Scrub => state.drag = Some(Drag::Scrub),
        }
    }

    // ---- Clips ------------------------------------------------------------
    // The clips about to be drawn carry their media's poster, whether or not
    // the library has that media in view.
    if let Some(media) = input.media {
        for asset in poster_assets(project, rows, geometry) {
            textures.want_poster(media, asset);
        }
    }
    let textures = &*textures;
    // And those on sound lanes draw their media's sound: asked for here and
    // nowhere else, so a tile that is not drawn asks for nothing.
    for asset in wave_assets(project, rows, geometry) {
        peaks.want(asset);
    }
    let peaks = &*peaks;
    let moving: Vec<ClipId> = match &state.drag {
        Some(Drag::Move { moved: true, .. }) => rules::related(project, &selection.clips, true),
        _ => Vec::new(),
    };
    let held_volume = match &state.drag {
        Some(Drag::Volume {
            clip,
            gain,
            at,
            moved: true,
            ..
        }) => Some((*clip, *gain, geometry.x(*at))),
        _ => None,
    };
    let held_keys = state.keys.overriding(project);
    // Whether a sound is heard is asked of the tiles of sound lanes alone. A
    // clip there is never silenced by a link, so no link is read for it: the
    // answer costs a look at the tracks and nothing is gathered.
    let heard = |track: &Track, clip: &Clip| {
        track.kind == TrackKind::Audio && sound::sounds(project, track, clip)
    };
    // The tile under the pointer while nothing is held, and the grip of it
    // the pointer is on.
    let pointed = over
        .filter(|pos| state.drag.is_none() && !claimed(*pos))
        .and_then(|pos| match hit(rows, geometry, pos) {
            Hit::Clip { clip, edge, .. } => Some((clip.id, edge)),
            _ => None,
        });
    // The tile being trimmed, where it is held, and by how much.
    let mut trimmed_tile = None;
    // The tile the keyboard is on.
    let mut focused = None;
    for (row, clip) in clips_drawn(rows, geometry) {
        let mut start = clip.start;
        let mut end = clip.end();
        let mut grip = (pointed.filter(|(id, _)| *id == clip.id))
            .and_then(|(_, edge)| edge)
            .map(|edge| (edge, false));
        if let Some(Drag::Trim {
            clip: trimmed,
            edge,
            delta,
        }) = &state.drag
            && *trimmed == clip.id
        {
            match edge {
                Edge::Start => start = start + *delta,
                Edge::End => end = end + *delta,
            }
            grip = Some((*edge, true));
        }
        // Where it came from stays faintly visible under the ghost.
        let dimmed = moving.contains(&clip.id);
        let rect = clip_rect(geometry, row, start, end);
        if let Some(Drag::Trim {
            clip: trimmed,
            edge,
            delta,
        }) = &state.drag
            && *trimmed == clip.id
            && *delta != Time::ZERO
        {
            trimmed_tile = Some((rect, *edge, *delta));
        }
        let chevron = toggles.iter().any(|(id, _)| *id == clip.id);
        // A fade that is held is drawn as long as it is held, a volume line
        // where it is held, and one bent by keys that are held as those
        // keys would bend it.
        let mut shown: Option<Clip> = None;
        if let Some((_, edge, duration)) = held_fade.filter(|held| held.0 == clip.id) {
            let shown = shown.get_or_insert_with(|| clip.clone());
            match edge {
                FadeEdge::In => shown.fade_in = duration,
                FadeEdge::Out => shown.fade_out = duration,
            }
        }
        let lifted = held_volume.filter(|held| held.0 == clip.id);
        if let Some((_, gain, _)) = lifted {
            shown.get_or_insert_with(|| clip.clone()).gain = gain;
        }
        if row.track.kind == TrackKind::Audio
            && let Some(animation) = (held_keys.as_ref())
                .filter(|held| held.clip == clip.id)
                .and_then(|held| held.animation.as_ref())
        {
            (shown.get_or_insert_with(|| clip.clone()).animation).clone_from(animation);
        }
        let clip = shown.as_ref().unwrap_or(clip);
        let marks = Marks {
            volume: lifted.map(|(.., x)| x),
            zoom: geometry.zoom,
            selected: (state.transition)
                .filter(|(id, _)| *id == clip.id)
                .map(|(_, edge)| edge),
            held: held_mark
                .filter(|held| held.0 == clip.id)
                .map(|held| (held.1, held.2)),
            beats: (state.beats.iter())
                .filter(|(id, _)| *id == clip.id)
                .map(|(_, at)| geometry.x(*at))
                .collect(),
            hovered: pointed.is_some_and(|(id, _)| id == clip.id),
            grip,
            heard: heard(row.track, clip),
        };
        paint_clip(
            ui,
            &painter,
            rect,
            project,
            row.track,
            clip,
            selection.contains(clip.id),
            if dimmed { 0.38 } else { 1.0 },
            if chevron { LANE_TOGGLE } else { 0.0 },
            &marks,
            textures,
            wave_of(project, row.track, clip, peaks, geometry, Time::ZERO),
            input,
        );
        if state.keyboard == Some(clip.id) {
            focused = Some(rect);
        }
    }
    // The keyboard's ring goes round its tile after every tile is drawn, so
    // no neighbour covers it: round the body the selection's ring is drawn
    // in, so the two nest, and on a painter wide enough that a tile at the
    // lanes' left edge keeps the ring's left side.
    if let Some(rect) = focused {
        let body = rect.shrink2(vec2(0.5, 0.0));
        let round = metrics::CLIP_RADIUS.min((rect.width() * 0.5) as u8);
        let ringed = ui.painter_at(lanes.expand2(vec2(3.0, 0.0)));
        controls::focus_ring(&ringed, body, round, *p);
    }
    if response.has_focus() {
        controls::focus_ring(&painter, lanes.shrink(3.0), 4, *p);
    }

    // ---- Keyframes: the chevrons and the open lane -------------------------
    {
        let mut over_lanes = ui.new_child(egui::UiBuilder::new().max_rect(lanes));
        over_lanes.set_clip_rect(lanes);
        for (id, rect) in &toggles {
            let is_open = open == Some(*id);
            if let Some((_, clip)) = project.clip(*id)
                && keyframes::disclosure(&mut over_lanes, p, *rect, clip, is_open)
            {
                state.keys.set_lane((!is_open).then_some(*id));
            }
        }
        if let Some(band) = band {
            (state.keys).show_lane(&mut over_lanes, band, geometry, selection, input, actions);
        }

        // ---- Transitions: the handles and the offer to add one ------------
        for (id, edge, rect) in &handles {
            let Some((_, clip)) = project.clip(*id) else {
                continue;
            };
            let Some(transition) = transitions::of(clip, *edge) else {
                continue;
            };
            let handle = over_lanes.interact(
                *rect,
                Id::new(("transition-handle", *id, *edge)),
                Sense::click_and_drag(),
            );
            handle.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    marking_name(clip, *edge, transition),
                )
            });
            // Under the pointer, the part of the marking that takes hold of
            // it is lit.
            if handle.hovered() && !handle.dragged() {
                (over_lanes.painter()).rect_filled(rect.shrink2(vec2(2.0, 3.0)), 3, p.hover);
            }
            if handle.has_focus() {
                controls::focus_ring(over_lanes.painter(), rect.shrink2(vec2(2.0, 3.0)), 3, *p);
            }
            if handle.clicked() || handle.drag_started() {
                state.mark(selection, *id, *edge);
            }
            if handle.drag_started() {
                state.snap_guide = None;
                state.drag = Some(Drag::Transition {
                    clip: *id,
                    edge: *edge,
                    duration: transition.duration,
                });
            }
            handle
                .on_hover_cursor(CursorIcon::ResizeHorizontal)
                .on_hover_text("Drag to change how long the transition lasts");
        }
        // ---- Fades: the handles ------------------------------------------
        for (id, edge, rect) in &faders {
            let Some((_, clip)) = project.clip(*id) else {
                continue;
            };
            let handle = over_lanes.interact(
                *rect,
                Id::new(("fade-handle", *id, *edge)),
                Sense::click_and_drag(),
            );
            handle.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, fade_name(clip, *edge))
            });
            let held = held_fade.filter(|held| (held.0, held.1) == (*id, *edge));
            let painter = over_lanes.painter();
            // Its keyline inside it, so the handle keeps its footprint and
            // the trim bar beside it stays whole. Under the pointer it stands
            // a point taller each way; held, it takes the accent.
            let shape = if handle.hovered() && held.is_none() {
                rect.expand2(vec2(0.0, 1.0))
            } else {
                *rect
            };
            let fill = if held.is_some() { p.accent } else { p.fg };
            painter.rect_filled(shape, 3, fill);
            painter.rect_stroke(shape, 3, Stroke::new(1.0, p.bg), StrokeKind::Inside);
            if handle.has_focus() {
                controls::focus_ring(painter, *rect, 3, *p);
            }
            // How long the held fade would be, beside its handle and inside
            // the tile.
            if let Some((.., duration)) = held {
                let x = match edge {
                    FadeEdge::In => rect.right() + 30.0,
                    FadeEdge::Out => rect.left() - 30.0,
                };
                let text = format!("{:.2} s", duration.as_seconds());
                time_capsule(painter, p, x, rect.bottom() + 12.0, &text);
            }
            if handle.clicked() || handle.drag_started() {
                // Taking hold of a clip's fade selects the clip.
                if !selection.contains(*id) {
                    selection.only(*id);
                }
                state.transition = None;
                state.keys.let_go();
            }
            if handle.drag_started() {
                let stored = fades::of(clip, *edge);
                // From where the press landed, not from where the pointer
                // has got to since.
                let taken = ui
                    .input(|i| i.pointer.press_origin())
                    .map_or(rect.center().x, |pos| pos.x);
                state.snap_guide = None;
                state.drag = Some(Drag::Fade {
                    clip: *id,
                    edge: *edge,
                    duration: stored,
                    grab: geometry.time(taken) - fade_end_time(clip, *edge, stored),
                });
            }
            handle
                .on_hover_cursor(CursorIcon::ResizeHorizontal)
                .on_hover_text(match edge {
                    FadeEdge::In => "Drag to set how long the sound fades in",
                    FadeEdge::Out => "Drag to set how long the sound fades out",
                });
        }
        state.badge = None;
        if let Some(spot) = &spot
            && let Some((_, clip)) = project.clip(spot.clip)
        {
            state.badge = Some((spot.clip, spot.edge, spot.badge));
            // Drawn and named, and no more: its press is the lanes', so a
            // drag from the edge under it still trims or moves the clip.
            let badge = over_lanes.interact(
                spot.badge,
                Id::new(("add-transition", spot.clip, spot.edge)),
                Sense::hover(),
            );
            let label = format!(
                "Add a transition at the {} of {}",
                transitions::edge_name(spot.edge),
                clip.name
            );
            badge.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            let hot = pointer.is_some_and(|pos| on_badge(spot.badge, pos));
            let painter = over_lanes.painter();
            let centre = spot.badge.center();
            let radius = ADD_BADGE * 0.5;
            painter.circle_filled(centre, radius, if hot { p.accent } else { p.elevated });
            painter.circle_stroke(centre, radius, Stroke::new(1.0, p.border));
            icons::paint(
                painter,
                Rect::from_center_size(centre, Vec2::splat(10.0)),
                Icon::Plus,
                if hot { p.on_accent } else { p.fg },
            );
            if hot {
                over_lanes.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            badge.on_hover_text(format!("{label} ({})", shortcut("Cmd+T")));
        }
    }

    // The playhead and the snap guide are drawn here, over the tiles and
    // under what is being dragged.
    let over_tiles = painter.add(egui::Shape::Noop);

    // The clips being moved, where they would land, with the beats they
    // show there, lifted off the lane they would land on.
    if let Some(Drag::Move {
        grabbed,
        delta,
        target,
        moved: true,
        ..
    }) = &state.drag
    {
        if let Some((track, _)) = project.clip(*grabbed)
            && let Some(row) = (rows.iter()).find(|row| row.track.id == target.unwrap_or(track.id))
        {
            painter.set(drop_row, lit(row));
        }
        let sounding = peaks
            .beats
            .any_shown()
            .then(|| sound::Sounding::of(project));
        let (from, to) = (geometry.time(lanes.left()), geometry.time(lanes.right()));
        for id in &moving {
            let Some((track, clip)) = project.clip(*id) else {
                continue;
            };
            let destination = match target {
                Some(target) if selection.contains(*id) => *target,
                _ => track.id,
            };
            let Some(row) = rows.iter().find(|row| row.track.id == destination) else {
                continue;
            };
            let rect = clip_rect(geometry, row, clip.start + *delta, clip.end() + *delta);
            let beats = sounding.as_ref().map_or_else(Vec::new, |sounding| {
                let (from, to) = (from - *delta, to - *delta);
                let zoom = geometry.zoom;
                shown_ticks(project, sounding, &peaks.beats, track, clip, from, to, zoom)
            });
            let marks = Marks {
                volume: None,
                zoom: geometry.zoom,
                selected: None,
                held: None,
                beats: (beats.into_iter())
                    .map(|at| geometry.x(at + *delta))
                    .collect(),
                hovered: false,
                grip: None,
                heard: heard(track, clip),
            };
            let wave = wave_of(project, track, clip, peaks, geometry, *delta);
            painter.add(p.popup_shadow().as_shape(rect, metrics::CLIP_RADIUS));
            paint_clip(
                ui, &painter, rect, project, track, clip, true, 0.92, 0.0, &marks, textures, wave,
                input,
            );
        }
    }

    // How far the edge being trimmed has gone, over the tile.
    if let Some((tile, edge, delta)) = trimmed_tile {
        let sign = if delta < Time::ZERO { "-" } else { "+" };
        let text = format!("{sign}{}", timefmt::timecode(Time(delta.0.abs()), rate));
        let x = match edge {
            Edge::Start => tile.left(),
            Edge::End => tile.right(),
        };
        time_capsule(&painter, p, x, tile.top() - 4.0, &text);
    }

    if let Some(rect) = marquee_rect {
        painter.rect_filled(rect, CornerRadius::same(2), theme::tint(p.accent, 0.10));
        painter.rect_stroke(
            rect,
            CornerRadius::same(2),
            Stroke::new(1.0, theme::tint(p.accent, 0.6)),
            StrokeKind::Inside,
        );
    }

    // ---- A library item held over the lanes -------------------------------
    if let Some(payload) = egui::DragAndDrop::payload::<AssetDrag>(ui.ctx())
        && let Some(pos) = pointer.filter(|pos| lanes.contains(*pos))
        && let Some(asset) = project.assets.get(&payload.0)
        && rules::placeable(asset)
    {
        let under = rows
            .iter()
            .find(|row| pos.y >= row.top && pos.y <= row.top + row.height + LANE_GAP);
        let row = under
            .filter(|row| rules::track_accepts(row.track.kind, asset) && !row.track.locked)
            .or_else(|| {
                rows.iter()
                    .find(|row| row.track.kind == rules::home_track(asset) && !row.track.locked)
            });
        let raw = geometry.time(pos.x);
        let (time, hit) = snapped(input, geometry, &state.beats, raw, &[]);
        let time = if hit {
            time
        } else {
            rules::quantize(raw, rate)
        };
        if let Some(row) = row {
            let start = rules::free_start(row.track, time, asset.duration, &[]);
            let rect = clip_rect(geometry, row, start, start + asset.duration);
            let tone = p.tone(super::tone_of_media(asset.kind));
            painter.set(drop_row, lit(row));
            painter.rect_filled(
                rect,
                CornerRadius::same(metrics::CLIP_RADIUS),
                theme::tint(tone, 0.35),
            );
            painter.rect_stroke(
                rect,
                CornerRadius::same(metrics::CLIP_RADIUS),
                Stroke::new(1.5, tone),
                StrokeKind::Inside,
            );
            drop_caret(
                &painter,
                p,
                row,
                rect.left(),
                &timefmt::timecode(start, rate),
            );
            if released {
                actions.push(Action::InsertAsset {
                    asset: asset.id,
                    place: Some((row.track.id, start)),
                });
            }
        } else if released {
            // No lane of the right kind yet: the application makes one.
            actions.push(Action::InsertAsset {
                asset: asset.id,
                place: None,
            });
        }
        ui.ctx().set_cursor_icon(CursorIcon::Copy);
    }

    // ---- A tile of the library held over a clip ---------------------------
    if let Some(payload) = egui::DragAndDrop::payload::<EntryDrag>(ui.ctx())
        && let Some(pos) = pointer.filter(|pos| lanes.contains(*pos))
    {
        let entry = payload.0;
        let under = match hit(rows, geometry, pos) {
            Hit::Clip { track, clip, .. } => (rows.iter())
                .find(|row| row.track.id == track.id)
                .map(|row| (clip, clip_rect(geometry, row, clip.start, clip.end()))),
            _ => None,
        };
        if let Some((clip, tile)) = under {
            // A transition goes to the edge it is let go nearer.
            let edge = matches!(entry, Entry::Transition(_)).then(|| {
                if pos.x - tile.left() <= tile.right() - pos.x {
                    TransitionEdge::In
                } else {
                    TransitionEdge::Out
                }
            });
            let target = match edge {
                Some(TransitionEdge::In) => tile.with_max_x(tile.left() + tile.width() * 0.5),
                Some(TransitionEdge::Out) => tile.with_min_x(tile.right() - tile.width() * 0.5),
                None => tile,
            };
            let radius = CornerRadius::same(metrics::CLIP_RADIUS);
            painter.rect_filled(target, radius, theme::tint(p.accent, 0.28));
            painter.rect_stroke(tile, radius, Stroke::new(1.5, p.accent), StrokeKind::Inside);
            if released {
                // The clip under the pointer as it is let go, and the
                // playhead of this frame.
                let clips = [clip.id];
                actions.push(browser::apply(project, entry, &clips, input.playhead, edge));
            }
            ui.ctx().set_cursor_icon(CursorIcon::Copy);
        } else {
            ui.ctx().set_cursor_icon(CursorIcon::NoDrop);
        }
    }

    // ---- A look of the Text tab held over the lanes -------------------------
    if let Some(payload) = egui::DragAndDrop::payload::<LookDrag>(ui.ctx())
        && let Some(pos) = pointer.filter(|pos| lanes.contains(*pos))
    {
        let look = &payload.0;
        let radius = CornerRadius::same(metrics::CLIP_RADIUS);
        // A title's look let go on a title gives it that look.
        let restyled = match hit(rows, geometry, pos) {
            Hit::Clip { track, clip, .. }
                if look.is_title()
                    && !track.locked
                    && matches!(clip.content, ClipContent::Text { .. }) =>
            {
                Some(text_tab::restyled_with(project, clip.id))
            }
            _ => None,
        };
        if let Some(clips) = restyled {
            for row in rows {
                for clip in row
                    .track
                    .clips
                    .iter()
                    .filter(|clip| clips.contains(&clip.id))
                {
                    let tile = clip_rect(geometry, row, clip.start, clip.end());
                    painter.rect_filled(tile, radius, theme::tint(p.accent, 0.28));
                    painter.rect_stroke(
                        tile,
                        radius,
                        Stroke::new(1.5, p.accent),
                        StrokeKind::Inside,
                    );
                }
            }
            if released {
                actions.push(text_tab::restyle(project, look, &clips));
            }
        } else {
            let raw = geometry.time(pos.x);
            let (time, hit) = snapped(input, geometry, &state.beats, raw, &[]);
            let time = if hit {
                time
            } else {
                rules::quantize(raw, rate)
            };
            let row = (rows.iter())
                .find(|row| pos.y >= row.top && pos.y <= row.top + row.height + LANE_GAP)
                .filter(|row| row.track.kind == TrackKind::Text && !row.track.locked);
            if let Some(row) = row {
                let duration = look.duration(rate);
                let start = rules::free_start(row.track, time, duration, &[]);
                let rect = clip_rect(geometry, row, start, start + duration);
                let tone = p.tone(tone_of_track(TrackKind::Text));
                painter.set(drop_row, lit(row));
                painter.rect_filled(rect, radius, theme::tint(tone, 0.35));
                painter.rect_stroke(rect, radius, Stroke::new(1.5, tone), StrokeKind::Inside);
                drop_caret(
                    &painter,
                    p,
                    row,
                    rect.left(),
                    &timefmt::timecode(start, rate),
                );
            }
            if released {
                actions.push(Action::AddLook {
                    look: std::sync::Arc::clone(look),
                    track: row.map(|row| row.track.id),
                    at: Some(time),
                });
            }
        }
        ui.ctx().set_cursor_icon(CursorIcon::Copy);
    }

    // ---- Cursor and context menu ------------------------------------------
    if state.drag.is_none()
        && let Some(pos) = over
        && !claimed(pos)
    {
        match hit(rows, geometry, pos) {
            Hit::Clip { edge: Some(_), .. } => {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            // A volume line a press would take hold of, or key with Alt.
            Hit::Clip { track, clip, .. }
                if (modifiers.alt
                    || (selection.contains(clip.id) && !sound::volume_keyed(clip)))
                    && volume_grip(project, rows, geometry, track, clip, pos).is_some() =>
            {
                ui.ctx().set_cursor_icon(if modifiers.alt {
                    CursorIcon::Cell
                } else {
                    CursorIcon::ResizeVertical
                });
            }
            Hit::Clip { track, .. } if !track.locked => {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            }
            _ => {}
        }
    }
    match &state.drag {
        Some(Drag::Trim { .. } | Drag::Fade { .. }) => {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        Some(Drag::Volume { .. }) => ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical),
        Some(Drag::Move { moved: true, .. }) => ui.ctx().set_cursor_icon(CursorIcon::Grabbing),
        _ => {}
    }

    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        state.menu_time = rules::quantize(geometry.time(pos.x), rate);
        state.menu_clip = match hit(rows, geometry, pos) {
            Hit::Clip { clip, .. } => {
                if !selection.contains(clip.id) {
                    selection.only(clip.id);
                }
                Some(clip.id)
            }
            _ => None,
        };
    }
    let menu_clip = state.menu_clip;
    let menu_time = state.menu_time;
    let mut marked = None;
    let mut keyed = None;
    let mut separated = None;
    context_menu(&response).show(|ui| {
        menu_style(ui, CLIP_MENU);
        // The edge of the clip nearer where the menu was opened, and the
        // transition it has: a cut is its incoming clip's start.
        let edge = menu_clip
            .and_then(|id| project.clip(id))
            .map(|(_, clip)| (clip.id, transitions::nearest_edge(clip, menu_time)));
        let place = edge.and_then(|(id, edge)| transitions::place(project, id, edge).ok());
        let has = place.is_some_and(|(id, edge)| {
            (project.clip(id)).is_some_and(|(_, clip)| transitions::of(clip, edge).is_some())
        });
        if let Some((id, edge)) = place.filter(|_| has)
            && menu_item(ui, p, Some(Icon::Pencil), "Edit transition", "").clicked()
        {
            marked = Some((id, edge));
            ui.close();
        }
        let item = |ui: &mut Ui,
                    actions: &mut Vec<Action>,
                    icon: Option<Icon>,
                    text: &str,
                    keys: &str,
                    action: Action| {
            if menu_item(ui, p, icon, text, &shortcut(keys)).clicked() {
                actions.push(action);
                ui.close();
            }
        };
        let Some(clip) = menu_clip else {
            ui.add_enabled_ui(input.can_paste, |ui| {
                let paste = Action::Paste;
                item(ui, actions, Some(Icon::Clipboard), "Paste", "Cmd+V", paste);
            });
            let marker = Action::Edit(marker_at(menu_time));
            item(ui, actions, Some(Icon::Flag), "Add marker here", "", marker);
            let seek = Action::Seek(menu_time);
            item(
                ui,
                actions,
                Some(Icon::Pointer),
                "Move playhead here",
                "",
                seek,
            );
            return;
        };
        let split = Action::SplitAtPlayhead;
        item(
            ui,
            actions,
            Some(Icon::Scissors),
            "Split at playhead",
            "S",
            split,
        );
        let duplicate = Action::Duplicate;
        item(
            ui,
            actions,
            Some(Icon::Copy),
            "Duplicate",
            "Cmd+D",
            duplicate,
        );
        // The clip and the playhead of this frame, whatever is selected by
        // the time the edit is made.
        let freeze = speed::freeze_command(project, clip, input.playhead, input.freeze_hold);
        ui.add_enabled_ui(freeze.is_ok(), |ui| {
            let action = match freeze {
                Ok(command) => Action::Edit(command),
                Err(reason) => Action::Notify(reason),
            };
            item(
                ui,
                actions,
                Some(Icon::Snowflake),
                "Freeze frame",
                "F",
                action,
            );
        });
        match (edge, place) {
            (_, Some((id, side))) if has => item(
                ui,
                actions,
                Some(Icon::Close),
                "Remove transition",
                "",
                Action::Edit(transitions::set_command(id, side, None)),
            ),
            (Some((id, side)), Some(_)) => item(
                ui,
                actions,
                Some(Icon::Transition),
                match side {
                    TransitionEdge::In => "Add transition at start",
                    TransitionEdge::Out => "Add transition at end",
                },
                "Cmd+T",
                Action::AddTransitions {
                    places: vec![(id, side)],
                    kind: None,
                },
            ),
            _ => {}
        }
        widgets::menu_separator(ui, p);
        item(ui, actions, None, "Cut", "Cmd+X", Action::Cut);
        item(ui, actions, None, "Copy", "Cmd+C", Action::Copy);
        widgets::menu_separator(ui, p);
        let ids = selection.clips.clone();
        let several = ids.len() > 1;
        let linked =
            (ids.iter()).any(|id| (project.clip(*id)).is_some_and(|(_, c)| c.linked_id.is_some()));
        let grouped =
            (ids.iter()).any(|id| (project.clip(*id)).is_some_and(|(_, c)| c.group_id.is_some()));
        if several {
            let link = Action::Edit(Command::LinkClips {
                clip_ids: ids.clone(),
            });
            item(ui, actions, Some(Icon::Link), "Link clips", "", link);
            let group = Action::Edit(Command::GroupClips {
                clip_ids: ids.clone(),
            });
            item(
                ui,
                actions,
                Some(Icon::Layers),
                "Group clips",
                "Cmd+G",
                group,
            );
        }
        if linked {
            let unlink = Action::Edit(Command::UnlinkClips {
                clip_ids: ids.clone(),
            });
            item(ui, actions, None, "Unlink", "", unlink);
        }
        if grouped {
            let ungroup = Action::Edit(Command::UngroupClips { clip_ids: ids });
            item(ui, actions, None, "Ungroup", "Shift+Cmd+G", ungroup);
        }
        if several || linked || grouped {
            widgets::menu_separator(ui, p);
        }
        // What the clip's sound can be asked for, in a menu of its own:
        // offered of a picture clip with sound, or of a clip whose media has
        // sound.
        let sounds = (project.clip(clip))
            .is_some_and(|(track, held)| rules::carries_sound(project, track, held));
        let fades_sound =
            (project.clip(clip)).is_some_and(|(_, held)| fades::has_sound(project, held));
        if sounds || fades_sound {
            controls::menu_submenu(ui, *p, Icon::Volume, "Audio", |ui| {
                menu_style(ui, CLIP_MENU);
                let asked = (&mut keyed, &mut separated);
                let offered = (sounds, fades_sound);
                audio_rows(
                    ui,
                    input,
                    selection,
                    &peaks.beats,
                    clip,
                    offered,
                    actions,
                    asked,
                );
            });
            widgets::menu_separator(ui, p);
        }
        // Each row its own glyph or none: the bin is the plain delete's.
        let delete =
            |ui: &mut Ui, actions: &mut Vec<Action>, text: &str, keys: &str, ripple: bool| {
                let icon = (!ripple).then_some(Icon::Trash);
                let row = widgets::menu_item_destructive(ui, p, icon, text, &shortcut(keys));
                if row.clicked() {
                    actions.push(Action::Delete { ripple });
                    ui.close();
                }
            };
        delete(ui, actions, "Delete", "Delete", false);
        delete(ui, actions, "Ripple delete", "Shift+Delete", true);
    });
    if let Some((clip, edge)) = marked {
        state.mark(selection, clip, edge);
    }
    if let Some(clip) = separated {
        actions.push(Action::SeparateStems(clip));
    }
    if let Some((clip, command)) = keyed {
        actions.push(Action::Edit(command));
        state.keys.set_lane(Some(clip));
    }
    over_tiles
}

/// How wide the menu of a clip and its Audio menu are.
const CLIP_MENU: f32 = 260.0;

/// The rows of a clip's Audio menu, in groups. `offered` is whether the clip
/// carries sound a detach would take, and whether its media has sound at
/// all. A key of the volume asked for, and a separation, are said through
/// `asked`: each opens or starts more than an edit.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site; the rows need the clip, the selection and what they ask"
)]
fn audio_rows(
    ui: &mut Ui,
    input: &TimelineInput,
    selection: &Selection,
    peaks: &HeldBeats,
    clip: ClipId,
    (sounds, fades_sound): (bool, bool),
    actions: &mut Vec<Action>,
    asked: (&mut Option<(ClipId, Command)>, &mut Option<ClipId>),
) {
    let p = input.p;
    let project = input.project;
    let (keyed, separated) = asked;
    let item =
        |ui: &mut Ui, actions: &mut Vec<Action>, icon: Option<Icon>, text: &str, action: Action| {
            if menu_item(ui, p, icon, text, "").clicked() {
                actions.push(action);
                ui.close();
            }
        };
    let mut groups = 0;
    let mut group = |ui: &mut Ui| {
        if groups > 0 {
            widgets::menu_separator(ui, p);
        }
        groups += 1;
    };
    // Offered of a picture clip with sound; one whose sound is on an audio
    // clip already, or whose track is locked, shows it without being able
    // to choose it.
    if sounds {
        group(ui);
        let free = rules::detach_refusal(project, clip).is_none();
        ui.add_enabled_ui(free, |ui| {
            let detach = Action::DetachAudio(clip);
            item(ui, actions, Some(Icon::ArrowDown), "Detach audio", detach);
        });
    }
    if !fades_sound {
        return;
    }
    // The fades of the selected clips' sound, by rows the keyboard reaches:
    // shown without being able to be chosen where no selected clip would be
    // changed.
    group(ui);
    for (label, asked) in fades::MENU {
        let made = fades::command(project, &selection.clips, asked);
        ui.add_enabled_ui(made.is_ok(), |ui| {
            let action = match made {
                Ok(command) => Action::Edit(command),
                Err(reason) => Action::Notify(reason),
            };
            item(ui, actions, None, label, action);
        });
    }
    // A key of the clip's volume at the playhead, at the volume it has
    // there: the lane it is in then opens. Shown without being able to be
    // chosen where none would be written.
    group(ui);
    let made = keyframes::volume_key(project, clip, input.playhead);
    ui.add_enabled_ui(made.is_ok(), |ui| {
        if menu_item(ui, p, Some(Icon::Diamond), "Key volume", "").clicked() {
            *keyed = made.ok().map(|command| (clip, command));
            ui.close();
        }
    });
    // The selected clips' loudness is measured and each one's volume set as
    // its figures come. Shown without being able to be chosen where none of
    // them would be measured.
    let taken = normalize::takers(project, &selection.clips);
    ui.add_enabled_ui(taken.is_ok(), |ui| {
        let action = match taken {
            Ok(clips) => Action::Normalize(clips),
            Err(reason) => Action::Notify(reason),
        };
        item(
            ui,
            actions,
            Some(Icon::Adjustments),
            "Normalize loudness",
            action,
        );
    });
    // On a clip of an audio lane: the selected clips are lowered wherever
    // the others are heard, at the default depth, once their sound is read.
    // Shown without being able to be chosen where none of them would be
    // ducked.
    let on_audio = (project.clip(clip)).is_some_and(|(track, _)| track.kind == TrackKind::Audio);
    if on_audio {
        // The whole selection goes: a clip passed over is still no voice.
        let taken = duck::takers(project, &selection.clips);
        ui.add_enabled_ui(taken.is_ok(), |ui| {
            let action = match taken {
                Ok(_) => Action::Duck {
                    clips: selection.clips.clone(),
                    depth: duck::DEPTH,
                },
                Err(reason) => Action::Notify(reason),
            };
            item(ui, actions, None, duck::LABEL, action);
        });
    }
    // The beats of the clip's media, on each of its tiles, for the session;
    // and while they are shown, a marker on each beat of this clip, shown
    // without being able to be chosen until they are found.
    let asset = (project.clip(clip))
        .and_then(|(_, held)| held.asset_id())
        .filter(|_| on_audio);
    if let Some(asset) = asset {
        group(ui);
        let shown = peaks.shown(asset);
        let label = if shown { "Hide beats" } else { "Show beats" };
        let action = Action::ShowBeats {
            assets: vec![asset],
            shown: !shown,
        };
        item(ui, actions, Some(Icon::Music), label, action);
        if shown {
            let made = peaks.markers(project, clip);
            ui.add_enabled_ui(made.is_ok(), |ui| {
                let action = match made {
                    Ok(command) => Action::Edit(command),
                    Err(reason) => Action::Notify(reason),
                };
                item(ui, actions, Some(Icon::Flag), peaks::BEAT_MARKERS, action);
            });
        }
    }
    // The span of its media's sound the clip plays, to a file: shown without
    // being able to be chosen when that span has none.
    group(ui);
    let source = crate::extract::Source::Clip(clip);
    let refused = crate::extract::refusal(project, source);
    ui.add_enabled_ui(refused.is_none(), |ui| {
        let action = match refused {
            None => Action::SaveAudio(source),
            Some(reason) => Action::Notify(reason),
        };
        item(ui, actions, Some(Icon::Save), crate::extract::LABEL, action);
    });
    // The whole of its media's sound, split by the stem separator and placed
    // under the clip when it comes. Shown without being able to be chosen,
    // and saying why, without a separator or for a clip that is not heard.
    let refused = (input.separator.refusal().map(str::to_owned))
        .or_else(|| crate::stems::refusal(project, clip));
    ui.add_enabled_ui(refused.is_none(), |ui| {
        let row = menu_item(ui, p, None, crate::stems::LABEL, "");
        if row.clicked() {
            *separated = Some(clip);
            ui.close();
        }
        if let Some(reason) = refused {
            row.on_disabled_hover_text(reason);
        }
    });
}

/// A marker with the default colour at `time`.
pub fn marker_at(time: Time) -> Command {
    Command::AddMarker {
        marker: Marker {
            id: uuid::Uuid::new_v4(),
            time,
            label: String::new(),
            color: [255, 159, 10, 255],
        },
    }
}

/// The command that moves one edge of a clip by `delta`.
pub fn trim_command(clip: &Clip, edge: Edge, delta: Time) -> Command {
    match edge {
        Edge::End => Command::TrimClip {
            clip_id: clip.id,
            source_in: clip.source_in,
            duration: clip.duration + delta,
            ripple: false,
        },
        // The change of `source_in` is how the editor learns that the start
        // edge moved, for a still or a title too, so keyframes stay where
        // they were on the timeline. With a time map the rate is 1 and the
        // change is timeline time.
        Edge::Start => Command::TrimClip {
            clip_id: clip.id,
            source_in: clip.source_in + delta.scaled(clip.playback_rate),
            duration: clip.duration - delta,
            ripple: false,
        },
    }
}

/// How a clip's transitions are marked on its tile, whether its volume line
/// is held, and the state the pointer and the mixer leave it in.
struct Marks {
    /// The volume line is being dragged, and where across it was taken hold
    /// of: the clip drawn has the volume it is held at.
    volume: Option<f32>,
    /// Points per second.
    zoom: f32,
    /// The edge whose transition is selected.
    selected: Option<TransitionEdge>,
    /// The edge whose transition is being dragged, and how long it would be.
    held: Option<(TransitionEdge, Time)>,
    /// Where the beats the tile shows are, points across.
    beats: Vec<f32>,
    /// The pointer is over the tile and nothing is held.
    hovered: bool,
    /// The edge whose grip the pointer is on, and whether it is being
    /// pulled.
    grip: Option<(Edge, bool)>,
    /// The clip's sound is heard: no mute and no solo elsewhere silences it.
    heard: bool,
}

#[expect(
    clippy::too_many_arguments,
    reason = "painting one clip needs its whole context"
)]
fn paint_clip(
    ui: &Ui,
    painter: &egui::Painter,
    rect: Rect,
    project: &Project,
    track: &Track,
    clip: &Clip,
    selected: bool,
    opacity: f32,
    // Room at the head of the name for the chevron of a keyframe lane.
    indent: f32,
    marks: &Marks,
    textures: &Textures,
    // The sound of a clip on an audio lane, once its peaks are held.
    wave: Option<Wave>,
    input: &TimelineInput,
) {
    let p = input.p;
    let tone = clip_tone(project, track, clip);
    let round = metrics::CLIP_RADIUS.min((rect.width() * 0.5) as u8);
    let radius = CornerRadius::same(round);
    // Half a point in from either end: tiles that abut keep a point of the
    // lane between them.
    let body = rect.shrink2(vec2(0.5, 0.0));
    let fill = if selected {
        p.tile_lift(tone)
    } else {
        p.tile_body(tone)
    };
    painter.rect_filled(body, radius, fill.gamma_multiply(opacity));

    let inner = rect.shrink(1.0);
    let clipped = painter.with_clip_rect(inner.intersect(painter.clip_rect()));
    let tall = carries_poster(rect.height());
    // A picture's name sits on a band over it; a sound's and a title's on
    // the tile.
    let banded = tall && track.kind != TrackKind::Audio;

    // A held volume line whose fades leave its sound no room for its percent
    // says it in the name's row.
    let mut unsaid = false;
    // A sound lane draws sound, of a video too; a picture lane the poster
    // frame at the head of the clip.
    if track.kind == TrackKind::Audio {
        let area = sound_area(rect);
        // A sound no one hears is drawn faint.
        let heard = if marks.heard { 1.0 } else { 0.4 } * opacity;
        match &wave {
            // The sound of the clip, under its name. It is the source's:
            // the clip's volume does not show in it.
            Some(wave) => {
                let inks = (
                    p.wave().gamma_multiply(heard),
                    p.clip_peak().gamma_multiply(heard),
                );
                paint_wave(&clipped, area, clip, wave, inks);
            }
            // Until its peaks are held, a level line says this is sound,
            // where the silence of a waveform is drawn.
            None => {
                clipped.hline(
                    inner.x_range(),
                    area.center().y,
                    Stroke::new(1.0, p.wave().gamma_multiply(0.5 * heard)),
                );
            }
        }
        // The beats of its music, at the foot of the sound.
        let ticks = p.fade_curve().gamma_multiply(opacity);
        paint_beats(&clipped, area, &marks.beats, ticks);
        // The fades of the sound, over it: how much of it is let through
        // from each edge.
        paint_fades(&clipped, rect, area, clip, marks.zoom, p, opacity);
        // The clip's volume, over both: a line where it stands, bent by its
        // keyframes.
        if fades::has_sound(project, clip) {
            let held = marks.volume;
            let said = paint_volume(&clipped, (rect, area), clip, marks.zoom, p, held, opacity);
            unsaid = held.is_some() && !said;
        }
    } else if tall
        && let ClipContent::Media { asset_id } = &clip.content
        && let Some(texture) = textures.poster(*asset_id)
    {
        // The poster frame at the head of the tile, its whole height, and no
        // further than the tile.
        let size = texture.size_vec2();
        let width = body.height() * size.x / size.y.max(1.0);
        let shown = width.min(body.width());
        let image = Rect::from_min_size(body.min, vec2(shown, body.height()));
        let across = if width > 0.0 { shown / width } else { 1.0 };
        let whole = shown >= body.width() - 0.5;
        let corners = CornerRadius {
            nw: round,
            sw: round,
            ne: if whole { round } else { 0 },
            se: if whole { round } else { 0 },
        };
        let poster =
            egui::epaint::RectShape::filled(image, corners, Color32::WHITE.gamma_multiply(opacity))
                .with_texture(
                    texture.id(),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(across, 1.0)),
                );
        painter
            .with_clip_rect(body.intersect(painter.clip_rect()))
            .add(poster);
    }

    // A transition: a shade over the part of the clip it lasts, with a line
    // rising from the cut it is at and an edge where it ends. It lies over a
    // picture as often as over the body, so it is drawn in the stage's
    // inks, light on a dark shade, in either appearance. A chosen one is
    // washed with the accent, and its ring waits for the clip's own.
    let mut chosen = None;
    for edge in transitions::EDGES {
        let Some(transition) = transitions::of(clip, edge) else {
            continue;
        };
        let duration = (marks.held)
            .filter(|(held, _)| *held == edge)
            .map_or(transition.duration, |(_, duration)| duration);
        let mark = marking(rect, marks.zoom, edge, duration).intersect(inner);
        let shade = Color32::from_black_alpha(90).gamma_multiply(opacity);
        let (ink, halo) = (
            Color32::from_white_alpha(170).gamma_multiply(opacity),
            Color32::from_black_alpha(80).gamma_multiply(opacity),
        );
        clipped.rect_filled(mark, 4, shade);
        if marks.selected == Some(edge) {
            clipped.rect_filled(mark, 4, p.accent.gamma_multiply(0.45 * opacity));
            chosen = Some(mark);
        }
        let (rising, end) = match edge {
            TransitionEdge::In => ([mark.left_bottom(), mark.right_top()], mark.right() - 0.5),
            TransitionEdge::Out => ([mark.right_bottom(), mark.left_top()], mark.left() + 0.5),
        };
        clipped.line_segment(rising, Stroke::new(3.0, halo));
        clipped.line_segment(rising, Stroke::new(1.0, ink));
        if mark.width() >= 6.0 {
            clipped.vline(
                clipped.round_to_pixel_center(end),
                (mark.top() + 2.0)..=(mark.bottom() - 2.0),
                Stroke::new(1.0, ink),
            );
        }
        if mark.width() >= 24.0 && mark.height() >= 14.0 {
            let at = Rect::from_center_size(mark.center(), Vec2::splat(12.0));
            icons::paint(
                &clipped,
                at.translate(vec2(0.0, 1.0)),
                Icon::Transition,
                halo,
            );
            icons::paint(
                &clipped,
                at,
                Icon::Transition,
                Color32::WHITE.gamma_multiply(opacity),
            );
        }
    }

    // The name's row: a band over a picture, filled with the accent while
    // the clip is selected. In the dark it is a shade under white ink; in
    // the light a frost under dark ink, which reads over a picture as well
    // and does not lay a dark slab along every light tile.
    let row = if tall {
        Rect::from_min_size(inner.min, vec2(inner.width(), NAME_ROW))
    } else {
        inner
    };
    if banded {
        let band = Rect::from_min_max(body.min, pos2(body.right(), inner.top() + NAME_ROW));
        let top = CornerRadius {
            nw: round,
            ne: round,
            sw: 0,
            se: 0,
        };
        let fill = if selected {
            p.accent
        } else if p.dark {
            p.tile_band()
        } else {
            Color32::from_white_alpha(190)
        };
        painter.rect_filled(band, top, fill.gamma_multiply(opacity));
    }
    let ink = if selected {
        p.on_accent
    } else if banded && p.dark {
        Color32::WHITE
    } else {
        p.fg
    };
    let ink = ink.gamma_multiply(opacity);

    // At the trailing end of the row: what keeps the track from an edit,
    // the link, how the clip is retimed and a held volume's percent. The
    // name gives way to them.
    let mut right = row.right() - 4.0;
    let middle = row.center().y;
    for (shown, glyph) in [(track.locked, Icon::Lock), (track.hidden, Icon::EyeOff)] {
        if shown && row.width() > 60.0 {
            let at = Rect::from_center_size(pos2(right - 6.0, middle), Vec2::splat(12.0));
            icons::paint(&clipped, at, glyph, ink);
            right -= 16.0;
        }
    }
    if clip.linked_id.is_some() && row.width() > 60.0 {
        let at = Rect::from_center_size(pos2(right - 6.0, middle), Vec2::splat(11.0));
        icons::paint(&clipped, at, Icon::Link, ink.gamma_multiply(0.8));
        right -= 16.0;
    }
    // The name starts clear of the bar at the tile's start, selected or
    // not, so choosing a tile never moves its name and a selected name's
    // capsule stands clear of the bar too. A chevron before it is wider.
    let start = rect.left() + 8.0 + indent.max(NAME_STEP);
    if row.width() > 96.0
        && let Some(badge) = speed::badge(clip)
    {
        let galley = widgets::truncated(
            ui,
            &badge,
            theme::medium(11.0),
            Color32::WHITE,
            (inner.width() * 0.5).clamp(48.0, 72.0),
        );
        let pill = row_badge(&clipped, galley, right, middle, opacity);
        right = pill.left() - 4.0;
    }
    // The percent of a held volume line its fades leave no room for, before
    // how the clip is retimed. The name gives way to it too, wholly where
    // not even its ellipsis would fit.
    let mut said_in_row = false;
    if unsaid && tall {
        let galley =
            clipped.layout_no_wrap(volume_percent(clip), theme::mono(11.0), Color32::WHITE);
        if right - galley.size().x - 8.0 >= inner.left() + 2.0 {
            let pill = row_badge(&clipped, galley, right, middle, opacity);
            right = pill.left() - 4.0;
            said_in_row = true;
        }
    }
    let named = right - 4.0 - start;
    if inner.width() > 22.0 {
        let galley = widgets::truncated(ui, &clip.name, theme::medium(11.5), ink, named);
        let at = pos2(start, middle - galley.size().y * 0.5);
        if !(said_in_row && galley.size().x > named) {
            // Without a band, a selected tile's name sits on a capsule of
            // the accent: a point short of its line at the top and the
            // foot, so a point of the body shows between it and the
            // selection's keyline.
            if selected && !banded {
                let capsule = Rect::from_min_size(at, galley.size()).expand2(vec2(4.0, -1.0));
                let round = (capsule.height() * 0.5).round() as u8;
                clipped.rect_filled(capsule, round, p.accent.gamma_multiply(opacity));
            }
            clipped.galley(at, galley, ink);
        }
    }

    // The tile's edge: a light line along its top in the dark, a line round
    // it in the light.
    if p.dark {
        let r = f32::from(round);
        painter.hline(
            (body.left() + r)..=(body.right() - r),
            body.top() + 0.5,
            Stroke::new(1.0, p.tile_edge().gamma_multiply(opacity)),
        );
    } else {
        painter.rect_stroke(
            body,
            radius,
            Stroke::new(1.0, p.tile_edge().gamma_multiply(opacity)),
            StrokeKind::Inside,
        );
    }
    // Over all of that: a hidden track's tiles are veiled, its name and
    // badges with them, and the tile under the pointer is lit.
    if track.hidden {
        painter.rect_filled(body, radius, theme::tint(p.bg, 0.45));
    }
    if marks.hovered {
        painter.rect_filled(body, radius, p.hover);
    }
    if selected {
        painter.rect_stroke(body, radius, Stroke::new(2.0, p.accent), StrokeKind::Inside);
        painter.rect_stroke(
            body.shrink(2.0),
            CornerRadius::same(round.saturating_sub(2)),
            Stroke::new(1.0, p.keyline()),
            StrokeKind::Inside,
        );
    }
    // Bars at the edges that can be pulled: lit while the pointer is on
    // their grip, the accent while one is pulled, both on a selected tile.
    if rect.width() > 26.0 && !track.locked {
        for edge in [Edge::Start, Edge::End] {
            let shade = match marks.grip {
                Some((held, true)) if held == edge => Some(p.accent),
                Some((near, false)) if near == edge => Some(theme::tint(p.fg, 0.9)),
                _ if selected => Some(theme::tint(p.fg, 0.8)),
                _ => None,
            };
            let Some(shade) = shade else {
                continue;
            };
            // Three points in from the body's edge: inside the selection's
            // ring and the keyline along it.
            let x = match edge {
                Edge::Start => rect.left() + 5.0,
                Edge::End => rect.right() - 5.0,
            };
            let bar = Rect::from_center_size(
                pos2(x, rect.center().y),
                vec2(3.0, (rect.height() - 10.0).max(4.0)),
            );
            painter.rect_filled(bar, 2, shade.gamma_multiply(opacity.max(0.6)));
        }
    }
    // The chosen transition's ring, over the clip's: a selected clip is
    // ringed and barred at its edges too, so this one stands clear of them,
    // inside the bars and the keyline and a little under the name's band.
    if let Some(mark) = chosen {
        let ring = Rect::from_x_y_ranges(
            mark.left().max(rect.left() + 7.5)..=mark.right().min(rect.right() - 7.5),
            (mark.top() + 2.0)..=(mark.bottom() - 3.0),
        );
        if ring.width() >= 6.0 && ring.height() >= 6.0 {
            clipped.rect_stroke(ring, 3, Stroke::new(2.0, p.accent), StrokeKind::Inside);
        }
    }
}

/// A pill at the trailing end of a tile's name row, its right at `right`
/// and centred on `middle`: white words on a dark pill, sixteen points
/// tall. Where it was drawn.
fn row_badge(
    painter: &egui::Painter,
    galley: std::sync::Arc<egui::Galley>,
    right: f32,
    middle: f32,
    opacity: f32,
) -> Rect {
    let size = vec2(galley.size().x + 10.0, 16.0);
    let pill = Rect::from_min_size(pos2(right - size.x, middle - size.y * 0.5), size);
    let black = Color32::from_black_alpha((140.0 * opacity) as u8);
    painter.rect_filled(pill, 8, black);
    let at = pos2(pill.left() + 5.0, pill.center().y - galley.size().y * 0.5);
    painter.galley(at, galley, Color32::WHITE.gamma_multiply(opacity));
    pill
}

/// A capsule saying `text` in the mono face above `bottom`, centred on `x`
/// and kept inside what `painter` draws in.
fn time_capsule(painter: &egui::Painter, p: &Palette, x: f32, bottom: f32, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), theme::mono(11.0), p.fg);
    let size = galley.size() + vec2(14.0, 6.0);
    let room = painter.clip_rect().shrink(2.0);
    let left = (x - size.x * 0.5).clamp(room.left(), (room.right() - size.x).max(room.left()));
    let top = (bottom - size.y).clamp(room.top(), (room.bottom() - size.y).max(room.top()));
    let capsule = Rect::from_min_size(pos2(left, top), size);
    controls::capsule(painter, capsule, *p);
    let at = capsule.center() - galley.size() * 0.5;
    painter.galley(at, galley, p.fg);
}

/// Where a drop would land on `row`: a caret of the accent at `x`, and the
/// time it would land at in a capsule above it.
fn drop_caret(painter: &egui::Painter, p: &Palette, row: &Row, x: f32, time: &str) {
    let x = painter.round_to_pixel_center(x);
    painter.vline(
        x,
        row.top..=(row.top + row.height),
        Stroke::new(2.0, p.accent),
    );
    time_capsule(painter, p, x, row.top - 3.0, time);
}

#[cfg(test)]
#[path = "beats_tests.rs"]
mod beating;

#[cfg(test)]
#[path = "fades_tests.rs"]
mod fading;

#[cfg(test)]
#[path = "heads_tests.rs"]
mod heads;

#[cfg(test)]
#[path = "volume_tests.rs"]
mod volume;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peaks::{Answer, Taken};
    use editor_core::MediaKind;
    use editor_media::{KeptPeaks, SourceIdentity};
    use std::{collections::BTreeSet, path::PathBuf, sync::Arc};
    use uuid::Uuid;

    fn seconds(seconds: i64) -> Time {
        Time(seconds * 1_000_000)
    }

    fn clip(start: Time, end: Time) -> Clip {
        Clip::text("Title", start, end - start)
    }

    fn track_of(clips: Vec<Clip>) -> Track {
        let mut track = Track::new("Text 1", TrackKind::Text);
        track.clips = clips;
        track
    }

    /// A track of clips over these whole seconds, in the order given.
    fn track(spans: &[(i64, i64)]) -> Track {
        track_of(
            spans
                .iter()
                .map(|&(start, end)| clip(seconds(start), seconds(end)))
                .collect(),
        )
    }

    fn drawn(track: &Track, view_start: Time, view_end: Time) -> Vec<ClipId> {
        clips_in_view(track, view_start, view_end)
            .map(|clip| clip.id)
            .collect()
    }

    /// Two picture tracks and one of sound, as they are shown: `over`
    /// above `under`, the sound last.
    ///
    /// ```text
    /// over    . . [b 2-4] . [c 6-8]
    /// under   [a 0-3] [d 3-6] . . .
    /// sound   . [e 1-9]
    /// ```
    fn stacked() -> (Project, [ClipId; 5]) {
        let mut project = Project::new("Reach");
        let [a, d] = [(0, 3), (3, 6)].map(|(start, end)| clip(seconds(start), seconds(end)));
        let [b, c] = [(2, 4), (6, 8)].map(|(start, end)| clip(seconds(start), seconds(end)));
        let e = clip(seconds(1), seconds(9));
        let ids = [a.id, b.id, c.id, d.id, e.id];
        let mut under = Track::new("Under", TrackKind::Text);
        under.clips = vec![a, d];
        let mut over = Track::new("Over", TrackKind::Text);
        over.clips = vec![b, c];
        let mut sound = Track::new("Sound", TrackKind::Audio);
        sound.clips = vec![e];
        project.tracks = vec![Arc::new(under), Arc::new(sound), Arc::new(over)];
        (project, ids)
    }

    #[test]
    fn the_keyboard_reaches_every_clip_from_the_playhead_and_from_its_neighbours() {
        let (project, [a, b, c, d, e]) = stacked();
        let at = |seconds_in: i64, from, way| reach(&project, from, seconds(seconds_in), way);
        // Under the playhead: the topmost first, then each one under it,
        // and round again.
        assert_eq!(at(2, None, Reach::AtPlayhead), Some(b));
        assert_eq!(at(2, Some(b), Reach::AtPlayhead), Some(a));
        assert_eq!(at(2, Some(a), Reach::AtPlayhead), Some(e));
        assert_eq!(at(2, Some(e), Reach::AtPlayhead), Some(b));
        // A clip selected elsewhere starts the round at the top.
        assert_eq!(at(2, Some(c), Reach::AtPlayhead), Some(b));
        assert_eq!(at(20, None, Reach::AtPlayhead), None);

        // Along a track, and no further than its ends.
        assert_eq!(at(0, Some(a), Reach::Next), Some(d));
        assert_eq!(at(0, Some(d), Reach::Next), None);
        assert_eq!(at(0, Some(d), Reach::Previous), Some(a));
        assert_eq!(at(0, Some(a), Reach::Previous), None);
        assert_eq!(at(0, Some(b), Reach::Next), Some(c));

        // Across tracks: the clip under the playhead while it is on the
        // clip left, and otherwise the one nearest that clip's start.
        assert_eq!(at(2, Some(b), Reach::Below), Some(a));
        assert_eq!(at(3, Some(b), Reach::Below), Some(d));
        assert_eq!(at(0, Some(c), Reach::Below), Some(d), "nearest six seconds");
        assert_eq!(at(2, Some(a), Reach::Above), Some(b));
        assert_eq!(at(2, Some(b), Reach::Above), None);
        assert_eq!(at(2, Some(a), Reach::Below), Some(e));
        assert_eq!(at(2, Some(e), Reach::Below), None);
        assert_eq!(at(0, Some(e), Reach::Above), Some(a));

        // With nothing selected a step is taken from the playhead.
        assert_eq!(at(20, None, Reach::Previous), Some(c));
        assert_eq!(at(20, None, Reach::Next), None);
        assert_eq!(at(20, None, Reach::Above), None);
        // A selected clip that has left the project is no start.
        assert_eq!(at(2, Some(uuid::Uuid::new_v4()), Reach::Next), Some(b));
    }

    #[test]
    fn a_start_trim_of_a_title_keeps_its_keyframes_on_the_timeline() {
        let mut title = clip(seconds(2), seconds(6));
        title.animation.push(editor_core::AnimTrack {
            target: None,
            key: editor_core::ParamKey::Opacity,
            keys: [(3, 1.0), (4, 0.0)]
                .map(|(at, value)| editor_core::Keyframe {
                    at: seconds(at),
                    value,
                    ease: editor_core::Ease::Linear,
                })
                .to_vec(),
        });
        let id = title.id;
        let mut project = Project::new("Trim");
        Arc::make_mut(&mut project.tracks[2]).clips.push(title);
        for delta in [1, -1] {
            let mut editor = editor_core::Editor::new(project.clone()).unwrap();
            let before = editor.project().clip(id).unwrap().1.clone();
            let command = trim_command(&before, Edge::Start, seconds(delta));
            assert!(matches!(command, Command::TrimClip { .. }));
            editor.dispatch(command).unwrap();
            let after = editor.project().clip(id).unwrap().1;
            assert_eq!(after.start, seconds(2 + delta));
            assert_eq!(after.end(), seconds(6));
            // The fade out still runs over the clip's last second.
            let fade: Vec<_> = after.animation[0]
                .keys
                .iter()
                .map(|key| after.start + key.at)
                .collect();
            assert_eq!(fade, [seconds(5), seconds(6)]);
        }
    }

    /// A four second clip of an eight second video at two seconds, with this
    /// time map, alone on the first track.
    fn mapped(keys: &[(f64, f64, editor_core::Ease)]) -> (Project, ClipId) {
        let mut asset = editor_core::Asset::new(
            PathBuf::from("/media/shot.mp4"),
            MediaKind::Video,
            seconds(8),
        );
        asset.has_audio = false;
        let mut clip = Clip::from_asset(&asset, seconds(2));
        clip.duration = seconds(4);
        clip.retime = Some(editor_core::Retime {
            keys: keys
                .iter()
                .map(|&(at, source, ease)| editor_core::TimeKey {
                    at: Time::from_seconds(at),
                    source: Time::from_seconds(source),
                    ease,
                })
                .collect(),
        });
        let id = clip.id;
        let mut project = Project::new("Trim");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        Arc::make_mut(&mut project.tracks[0]).clips.push(clip);
        project.validate().unwrap();
        (project, id)
    }

    #[test]
    fn a_trim_of_a_time_map_keeps_the_source_shown_at_each_timeline_time() {
        use editor_core::Ease::{Bezier, Linear};
        let ramp = Bezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        let maps: [&[(f64, f64, editor_core::Ease)]; 4] = [
            &[(0.0, 0.0, Linear), (4.0, 8.0, Linear)],
            &[(0.0, 1.0, Linear), (2.0, 3.0, Linear), (4.0, 3.0, Linear)],
            &[(0.0, 6.0, Linear), (4.0, 2.0, Linear)],
            &[(0.0, 0.0, ramp), (4.0, 6.0, Linear)],
        ];
        for keys in maps {
            let (project, id) = mapped(keys);
            let track = &project.tracks[0];
            let before = project.clip(id).unwrap().1.clone();
            for edge in [Edge::Start, Edge::End] {
                // A map is cropped, never let out, though the track has room.
                let (least, most) = rules::trim_range(&project, track, &before, edge);
                let inwards = match edge {
                    Edge::Start => most,
                    Edge::End => least,
                };
                assert_eq!(least.max(most), inwards.max(Time::ZERO));
                assert_eq!(least.min(most), inwards.min(Time::ZERO));
                for delta in [inwards, Time(inwards.0 / 3)] {
                    let mut editor = editor_core::Editor::new(project.clone()).unwrap();
                    if let Err(error) = editor.dispatch(trim_command(&before, edge, delta)) {
                        // What is left of a ramp can be slower than a map may
                        // be, where it eases to a stop: the editor refuses
                        // that trim and changes nothing.
                        let eased = keys.iter().any(|key| key.2 != Linear);
                        assert!(eased, "{keys:?} {edge:?} {delta:?}: {error}");
                        assert_eq!(editor.project().clip(id).unwrap().1, &before);
                        continue;
                    }
                    let after = editor.project().clip(id).unwrap().1;
                    match edge {
                        Edge::Start => assert_eq!(after.start, before.start + delta),
                        Edge::End => assert_eq!(after.start, before.start),
                    }
                    assert_eq!(after.duration, before.duration - Time(delta.0.abs()));
                    // The part that is left plays what it played before.
                    for step in 0..=16 {
                        let at = after.start + Time(after.duration.0 * step / 16);
                        let moved = (after.source_time(at).0 - before.source_time(at).0).abs();
                        assert!(moved <= 2, "{keys:?} {edge:?} {delta:?} at {at:?}: {moved}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_trim_stops_short_of_what_the_project_refuses_of_an_eased_map() {
        use editor_core::Ease::{Bezier, Linear};
        // Eased from a stop to a stop: what is left of either end alone is
        // slower than a map may be.
        let ramp = Bezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        let maps: [&[(f64, f64, editor_core::Ease)]; 3] = [
            &[(0.0, 0.0, ramp), (4.0, 6.0, Linear)],
            &[(0.0, 0.0, Linear), (1.0, 1.0, ramp), (4.0, 6.0, Linear)],
            &[(0.0, 6.0, Linear), (4.0, 2.0, Linear)],
        ];
        let mut stopped = 0;
        for keys in maps {
            let (project, id) = mapped(keys);
            let track = &project.tracks[0];
            let before = project.clip(id).unwrap().1.clone();
            let unit = rules::frame(project.settings.frame_rate);
            for edge in [Edge::Start, Edge::End] {
                let (least, most) = rules::trim_range(&project, track, &before, edge);
                let inwards = match edge {
                    Edge::Start => most,
                    Edge::End => least,
                };
                for wanted in [inwards, Time(inwards.0 / 2), Time(inwards.0 / 40)] {
                    let delta = rules::trim_delta(&project, track, &before, edge, wanted);
                    assert!(delta.0.abs() <= wanted.0.abs() && delta.0 * wanted.0 >= 0);
                    let trimmed = |delta: Time| {
                        let mut editor = editor_core::Editor::new(project.clone()).unwrap();
                        editor.dispatch(trim_command(&before, edge, delta)).is_ok()
                    };
                    // Where the gesture stops, the project takes the trim.
                    assert!(
                        delta == Time::ZERO || trimmed(delta),
                        "{keys:?} {edge:?} {wanted:?} stopped at {delta:?}"
                    );
                    if delta != wanted {
                        stopped += 1;
                        // And it stops no sooner than it has to.
                        let further = Time(delta.0 + wanted.0.signum() * unit.0);
                        assert!(!trimmed(further), "{keys:?} {edge:?} {wanted:?}");
                    } else {
                        assert!(trimmed(wanted));
                    }
                }
            }
        }
        assert!(stopped >= 2, "an eased map stops a trim somewhere");
        // A straight map is cropped as far as it is asked to be.
        let (project, id) = mapped(maps[2]);
        let clip = project.clip(id).unwrap().1;
        let track = &project.tracks[0];
        assert_eq!(
            rules::trim_delta(&project, track, clip, Edge::End, seconds(-3)),
            seconds(-3)
        );
        // And nothing is let out of a map, as before.
        assert_eq!(
            rules::trim_delta(&project, track, clip, Edge::End, seconds(1)),
            Time::ZERO
        );
    }

    #[test]
    fn a_long_clip_stays_in_view_past_shorter_clips_that_ended() {
        let track = track(&[(0, 100), (1, 2), (3, 4)]);
        assert_eq!(
            drawn(&track, seconds(50), seconds(60)),
            vec![track.clips[0].id],
            "the clip under the view was not drawn"
        );
    }

    #[test]
    fn a_long_clip_among_shorter_ones_stays_in_view() {
        let track = track(&[(0, 1), (2, 100), (3, 4), (5, 6), (7, 8)]);
        assert_eq!(
            drawn(&track, seconds(50), seconds(55)),
            vec![track.clips[1].id],
            "the clip under the view was not drawn"
        );
    }

    #[test]
    fn overlapping_clips_in_view_keep_their_track_order() {
        let track = track(&[(0, 100), (1, 2), (10, 90), (11, 12), (52, 53), (70, 80)]);
        let clips = &track.clips;
        let in_view = drawn(&track, seconds(50), seconds(60));
        assert_eq!(in_view, vec![clips[0].id, clips[2].id, clips[4].id]);
    }

    #[test]
    fn clips_end_to_end_are_drawn_only_where_the_view_is() {
        let track = track(&[(0, 2), (2, 4), (4, 6), (6, 8), (8, 10)]);
        let clips = &track.clips;
        let across = drawn(&track, Time(4_500_000), Time(6_500_000));
        assert_eq!(across, vec![clips[2].id, clips[3].id]);
        // A view inside one clip draws that clip.
        let inside = drawn(&track, Time(4_500_000), Time(5_500_000));
        assert_eq!(inside, vec![clips[2].id]);
    }

    #[test]
    fn clips_overlapping_in_step_are_drawn_where_the_view_is() {
        let track = track(&[(0, 10), (5, 20), (15, 30)]);
        let clips = &track.clips;
        let in_view = drawn(&track, seconds(12), seconds(18));
        assert_eq!(in_view, vec![clips[1].id, clips[2].id]);
    }

    #[test]
    fn nothing_is_drawn_for_a_view_clear_of_the_clips() {
        let track = track(&[(10, 12), (14, 16)]);
        let tick = Time(1);
        assert!(drawn(&track, seconds(0), seconds(5)).is_empty());
        assert!(drawn(&track, seconds(20), seconds(30)).is_empty());
        // The gap between the two.
        let (gap_start, gap_end) = (seconds(12) + tick, seconds(14) - tick);
        assert!(drawn(&track, gap_start, gap_end).is_empty());
        let bare = track_of(Vec::new());
        assert!(drawn(&bare, seconds(0), seconds(5)).is_empty());
    }

    #[test]
    fn a_clip_is_drawn_from_the_first_tick_it_enters_the_view() {
        let (view_start, view_end) = (seconds(10), seconds(20));
        let tick = Time(1);
        let track = track_of(vec![
            clip(Time::ZERO, view_start - tick),
            clip(seconds(1), view_start + tick),
            clip(view_end - tick, seconds(25)),
            clip(view_end + tick, seconds(26)),
        ]);
        let in_view = drawn(&track, view_start, view_end);
        assert_eq!(in_view, vec![track.clips[1].id, track.clips[2].id]);
    }

    #[test]
    fn a_clip_that_only_touches_the_view_is_not_drawn() {
        let track = track(&[(0, 10), (12, 15), (20, 30)]);
        let clips = &track.clips;
        // One ends where the view starts, another starts where it ends.
        assert_eq!(drawn(&track, seconds(10), seconds(20)), vec![clips[1].id]);
        // A view of no width, or one turned around, draws nothing.
        assert!(drawn(&track, seconds(13), seconds(13)).is_empty());
        assert!(drawn(&track, seconds(14), seconds(13)).is_empty());
    }

    /// A project of `count` one-second shots end to end on one track, each
    /// of its own media.
    fn shots(count: i64) -> Project {
        let mut project = Project::new("Shots");
        let mut track = Track::new("Video 1", TrackKind::Video);
        for index in 0..count {
            let path = PathBuf::from(format!("/media/shot-{index}.mp4"));
            let asset = Asset::new(path, MediaKind::Video, seconds(1));
            track.clips.push(Clip::from_asset(&asset, seconds(index)));
            Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        }
        project.tracks = vec![Arc::new(track)];
        project
    }

    /// A project of `count` video tracks, each holding one second of its own
    /// media at the start.
    fn layers(count: usize) -> Project {
        let mut project = Project::new("Layers");
        project.tracks.clear();
        for index in 0..count {
            let path = PathBuf::from(format!("/media/layer-{index}.mp4"));
            let asset = Asset::new(path, MediaKind::Video, seconds(1));
            let mut track = Track::new(format!("Video {index}"), TrackKind::Video);
            track.clips.push(Clip::from_asset(&asset, Time::ZERO));
            Arc::make_mut(&mut project.assets).insert(asset.id, asset);
            project.tracks.push(Arc::new(track));
        }
        project
    }

    /// Lanes of this size, a hundred points to the second, scrolled so that
    /// they start at `view_start`.
    fn lanes(view_start: Time, width: f32, height: f32) -> Geometry {
        Geometry {
            lanes: Rect::from_min_size(pos2(0.0, 0.0), vec2(width, height)),
            zoom: 100.0,
            scroll: view_start.as_seconds() as f32 * 100.0,
        }
    }

    /// The rows of a project as `show` lays them out, scrolled down by
    /// `scroll` points.
    fn rows<'a>(project: &'a Project, geometry: &Geometry, scroll: f32) -> Vec<Row<'a>> {
        lay_out(
            display_order(project),
            geometry.lanes.top() + LANE_GAP - scroll,
        )
    }

    fn media_in_view(project: &Project, view_start: Time, view_end: Time) -> BTreeSet<AssetId> {
        project
            .tracks
            .iter()
            .flat_map(|track| clips_in_view(track, view_start, view_end))
            .filter_map(Clip::asset_id)
            .collect()
    }

    fn media_asked_for(project: &Project, rows: &[Row], geometry: &Geometry) -> BTreeSet<AssetId> {
        poster_assets(project, rows, geometry)
            .map(|asset| asset.id)
            .collect()
    }

    /// The media of a row's clips.
    fn media_of(rows: &[Row]) -> BTreeSet<AssetId> {
        rows.iter()
            .flat_map(|row| &row.track.clips)
            .filter_map(Clip::asset_id)
            .collect()
    }

    #[test]
    fn clips_in_view_have_their_posters_asked_for() {
        // Nothing here depends on what the library has scrolled into view.
        let project = shots(40);
        let geometry = lanes(Time(10_500_000), 300.0, 200.0);
        let in_view = media_in_view(&project, Time(10_500_000), Time(13_500_000));
        assert_eq!(in_view.len(), 4);
        let rows = rows(&project, &geometry, 0.0);
        let asked = media_asked_for(&project, &rows, &geometry);
        assert!(in_view.is_subset(&asked));
    }

    #[test]
    fn posters_are_asked_for_only_for_clips_in_view() {
        let project = shots(40);
        let geometry = lanes(Time(10_500_000), 300.0, 200.0);
        let in_view = media_in_view(&project, Time(10_500_000), Time(13_500_000));
        let rows = rows(&project, &geometry, 0.0);
        let asked = media_asked_for(&project, &rows, &geometry);
        assert!(
            asked.is_subset(&in_view),
            "posters of {} assets are asked for where {} are in view",
            asked.len(),
            in_view.len()
        );
    }

    #[test]
    fn a_clip_without_media_in_the_library_asks_for_no_poster() {
        let mut project = shots(1);
        let path = PathBuf::from("/media/gone.mp4");
        let gone = Asset::new(path, MediaKind::Video, seconds(1));
        let track = Arc::make_mut(&mut project.tracks[0]);
        track.clips.push(Clip::from_asset(&gone, seconds(1)));
        track.clips.push(clip(seconds(2), seconds(3)));
        let geometry = lanes(Time::ZERO, 400.0, 200.0);
        let rows = rows(&project, &geometry, 0.0);
        assert_eq!(clips_drawn(&rows, &geometry).count(), 3);
        assert_eq!(media_asked_for(&project, &rows, &geometry).len(), 1);
    }

    #[test]
    fn posters_are_asked_for_only_on_rows_in_the_lanes() {
        let project = layers(6);
        // Lanes a hundred points high over rows of 58 with 4 between.
        let geometry = lanes(Time::ZERO, 300.0, 100.0);
        let top = rows(&project, &geometry, 0.0);
        let asked = media_asked_for(&project, &top, &geometry);
        assert_eq!(asked, media_of(&top[..2]));
        assert_eq!(asked.len(), 2);

        // Scrolled a hundred points down, the second to the fourth show.
        let scrolled = rows(&project, &geometry, 100.0);
        let asked = media_asked_for(&project, &scrolled, &geometry);
        assert_eq!(asked, media_of(&scrolled[1..4]));
        assert_eq!(asked.len(), 3);
        // What is asked for is what is drawn.
        let drawn: Vec<ClipId> = clips_drawn(&scrolled, &geometry)
            .map(|(_, clip)| clip.id)
            .collect();
        let shown: Vec<ClipId> = scrolled[1..4]
            .iter()
            .map(|row| row.track.clips[0].id)
            .collect();
        assert_eq!(drawn, shown);

        // A row that only touches an edge of the lanes shows nothing.
        let touching = rows(&project, &geometry, 62.0);
        assert_eq!(touching[0].top + touching[0].height, 0.0);
        let asked = media_asked_for(&project, &touching, &geometry);
        assert_eq!(asked, media_of(&touching[1..3]));
    }

    #[test]
    fn a_lane_too_low_for_a_poster_asks_for_none() {
        // The same kind of media on a picture lane, a sound lane and a lane
        // as low as a text lane.
        let mut project = Project::new("Lanes");
        project.tracks.clear();
        let mut media = Vec::new();
        for kind in [TrackKind::Video, TrackKind::Audio, TrackKind::Text] {
            let path = PathBuf::from(format!("/media/lane-{}.mp4", media.len()));
            let asset = Asset::new(path, MediaKind::Video, seconds(1));
            let mut track = Track::new("Lane", kind);
            track.clips.push(Clip::from_asset(&asset, Time::ZERO));
            media.push(asset.id);
            Arc::make_mut(&mut project.assets).insert(asset.id, asset);
            project.tracks.push(Arc::new(track));
        }
        let geometry = lanes(Time::ZERO, 300.0, 400.0);
        let rows = rows(&project, &geometry, 0.0);
        assert_eq!(clips_drawn(&rows, &geometry).count(), 3);
        // A sound lane is tall enough and draws the sound of its video, not
        // the poster; the low lane draws none. Neither asks for one.
        let asked = media_asked_for(&project, &rows, &geometry);
        assert_eq!(asked, BTreeSet::from([media[0]]));
        assert!(carries_poster(lane_height(TrackKind::Video)));
        assert!(carries_poster(lane_height(TrackKind::Audio)));
        assert!(!carries_poster(lane_height(TrackKind::Text)));
        assert!(!carries_poster(POSTER_LANE));
    }

    #[test]
    fn media_on_many_clips_in_view_is_one_poster() {
        let mut project = shots(1);
        let asset = project.assets.values().next().expect("one").clone();
        let track = Arc::make_mut(&mut project.tracks[0]);
        for index in 1..200 {
            track.clips.push(Clip::from_asset(&asset, seconds(index)));
        }
        let geometry = lanes(Time::ZERO, 3000.0, 200.0);
        let rows = rows(&project, &geometry, 0.0);
        assert_eq!(clips_drawn(&rows, &geometry).count(), 30);
        assert_eq!(media_asked_for(&project, &rows, &geometry).len(), 1);
    }

    /// A project of `count` one-second sounds end to end on each of two
    /// sound tracks under a picture track of one video with sound, every
    /// clip of its own media.
    fn sounds(count: i64) -> Project {
        let mut project = Project::new("Sounds");
        project.tracks.clear();
        let mut picture = Track::new("Video 1", TrackKind::Video);
        let mut video = Asset::new("/media/talk.mp4".into(), MediaKind::Video, seconds(count));
        video.has_audio = true;
        picture.clips.push(Clip::from_asset(&video, Time::ZERO));
        Arc::make_mut(&mut project.assets).insert(video.id, video);
        project.tracks.push(Arc::new(picture));
        for lane in 0..2 {
            let mut track = Track::new(format!("Audio {lane}"), TrackKind::Audio);
            for index in 0..count {
                let path = PathBuf::from(format!("/media/sound-{lane}-{index}.wav"));
                let asset = Asset::new(path, MediaKind::Audio, seconds(1));
                track.clips.push(Clip::from_asset(&asset, seconds(index)));
                Arc::make_mut(&mut project.assets).insert(asset.id, asset);
            }
            project.tracks.push(Arc::new(track));
        }
        project
    }

    fn sound_asked_for(project: &Project, rows: &[Row], geometry: &Geometry) -> BTreeSet<AssetId> {
        wave_assets(project, rows, geometry)
            .map(|asset| asset.id)
            .collect()
    }

    #[test]
    fn sound_is_asked_for_only_by_the_sound_tiles_drawn() {
        let project = sounds(40);
        // Lanes that show the picture row and the first sound row: 58 and
        // 44 points with 4 between and above.
        let geometry = lanes(Time(10_500_000), 300.0, 80.0);
        let laid = rows(&project, &geometry, 0.0);
        let asked = sound_asked_for(&project, &laid, &geometry);
        let first: BTreeSet<AssetId> =
            clips_in_view(laid[1].track, Time(10_500_000), Time(13_500_000))
                .filter_map(Clip::asset_id)
                .collect();
        assert_eq!(first.len(), 4);
        // Not the picture clip's sound, nor the row below the lanes, nor a
        // clip of this row that is out of view.
        assert_eq!(asked, first);
        // Sound is not a poster, and a picture is not sound.
        let posters = media_asked_for(&project, &laid, &geometry);
        assert_eq!(posters, media_of(&laid[..1]));
        assert!(posters.is_disjoint(&asked));

        // Scrolled down to the second sound row, the first asks for nothing.
        let scrolled = rows(&project, &geometry, 110.0);
        assert!(scrolled[1].top + scrolled[1].height <= geometry.lanes.top());
        let asked = sound_asked_for(&project, &scrolled, &geometry);
        assert_eq!(asked.len(), 4);
        assert!(asked.is_subset(&media_of(&scrolled[2..])));
        assert!(asked.is_disjoint(&first));

        // Drawn frame after frame, the tiles in view ask once and no other
        // tile asks at all.
        let mut held = HeldPeaks::default();
        let mut asked = BTreeSet::new();
        for frame in 0..3 {
            held.begin_frame();
            for asset in wave_assets(&project, &laid, &geometry) {
                held.want(asset);
            }
            let wants = held.take_wants();
            assert_eq!(wants.len(), if frame == 0 { 4 } else { 0 });
            for (request, (id, _)) in wants.into_iter().enumerate() {
                held.asked(id, request as u64);
                asked.insert(id);
            }
        }
        assert_eq!(asked, first);
    }

    /// A clip of a ten second sound that starts at two seconds of the
    /// timeline.
    fn voice() -> Clip {
        let asset = Asset::new("/media/voice.wav".into(), MediaKind::Audio, seconds(10));
        Clip::from_asset(&asset, seconds(2))
    }

    /// The columns of a clip's waveform across its whole tile, in lanes a
    /// hundred points to the second scrolled to `view_start`.
    fn columns(clip: &Clip, view_start: Time, shift: Time) -> Vec<(f32, Time, Time)> {
        let geometry = lanes(view_start, 2000.0, 200.0);
        let (left, right) = (
            geometry.x(clip.start + shift),
            geometry.x(clip.end() + shift),
        );
        wave_columns(clip, &geometry, shift, left, right).collect()
    }

    #[test]
    fn a_waveform_column_shows_the_source_its_point_of_the_clip_plays() {
        // At its own rate a point of width is a hundredth of a second of the
        // source, and the tile starts where the source does.
        let mut plain = voice();
        plain.duration = seconds(4);
        let drawn = columns(&plain, Time::ZERO, Time::ZERO);
        assert_eq!(drawn.len(), 400);
        assert_eq!(drawn[0], (200.0, Time::ZERO, Time(10_000)));
        assert_eq!(drawn[399], (599.0, Time(3_990_000), seconds(4)));
        // Each column starts where the one before it ended.
        assert!(drawn.windows(2).all(|pair| pair[0].2 == pair[1].1));

        // At twice the rate each point covers twice the source.
        let mut fast = plain.clone();
        fast.playback_rate = 2.0;
        let quick = columns(&fast, Time::ZERO, Time::ZERO);
        assert_eq!(quick.len(), 400);
        for (one, two) in drawn.iter().zip(&quick) {
            assert_eq!(one.0, two.0);
            assert_eq!((two.2 - two.1).0, 2 * (one.2 - one.1).0);
        }
        assert_eq!(quick[399].2, seconds(8));

        // A trimmed clip starts at its `source_in`, wherever it is scrolled.
        let mut trimmed = plain.clone();
        trimmed.source_in = Time(1_500_000);
        for view_start in [Time::ZERO, seconds(1)] {
            let cut = columns(&trimmed, view_start, Time::ZERO);
            assert_eq!(cut[0].1, Time(1_500_000));
            assert_eq!(cut[399].2, Time(5_500_000));
        }
        // Only the part in view is worked out: a tile whose head is
        // scrolled away starts at the source its first point in view plays.
        let geometry = lanes(seconds(3), 150.0, 200.0);
        let part: Vec<_> = wave_columns(
            &trimmed,
            &geometry,
            Time::ZERO,
            geometry.lanes.left(),
            geometry.lanes.right(),
        )
        .collect();
        assert_eq!(part.len(), 150);
        assert_eq!(part[0], (0.0, Time(2_500_000), Time(2_510_000)));
        assert_eq!(part[149].2, seconds(4));

        // A clip being moved is drawn where it would land, with its own
        // sound.
        let moved = columns(&plain, Time::ZERO, seconds(3));
        assert_eq!(moved[0], (500.0, Time::ZERO, Time(10_000)));
        assert_eq!(moved.len(), 400);
    }

    #[test]
    fn at_the_widest_zoom_a_bucket_of_peaks_is_drawn_by_six_or_seven_columns() {
        // Two seconds of the clip fill lanes 1280 points wide, scrolled to
        // where it starts.
        let clip = voice();
        let geometry = Geometry {
            lanes: Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 200.0)),
            zoom: MAX_ZOOM,
            scroll: 2.0 * MAX_ZOOM,
        };
        let (left, right) = (geometry.lanes.left(), geometry.lanes.right());
        let drawn: Vec<_> = wave_columns(&clip, &geometry, Time::ZERO, left, right).collect();
        assert_eq!(drawn.len(), 1280);
        assert_eq!(drawn[0].1, Time::ZERO);
        assert_eq!(drawn[1279].2, seconds(2));
        assert!(drawn.windows(2).all(|pair| pair[0].2 == pair[1].1));
        // A point is a 640th of a second of the source, to the microsecond.
        assert!(
            drawn
                .iter()
                .all(|column| (1562..=1563).contains(&(column.2 - column.1).0))
        );
        // A bucket is a hundredth of a second: 6.4 points. Six or seven
        // columns start in each, and the sound is no finer than that.
        let mut starts = [0usize; 200];
        for column in &drawn {
            starts[(column.1.0 / 10_000) as usize] += 1;
        }
        assert!(starts.iter().all(|count| (6..=7).contains(count)));
        assert_eq!(starts.iter().sum::<usize>(), 1280);
        // Buckets by turns quiet and loud: every column inside one bucket
        // reads that bucket alone, and one across an edge reads both.
        let peaks = Peaks {
            buckets: (0..1000i32)
                .map(|index| {
                    let level = 20 + 60 * (index % 2);
                    [(-level) as i8 as u8, level as u8, 0, 0]
                })
                .collect(),
        };
        let heights: Vec<i8> = drawn
            .iter()
            .map(|column| peaks.range(column.1, column.2).expect("sound").1)
            .collect();
        let runs: Vec<usize> = heights
            .chunk_by(|one, other| one == other)
            .map(<[i8]>::len)
            .collect();
        assert_eq!(runs.len(), 200, "a run of columns for each bucket");
        // The loud take the columns across their edges.
        assert!(runs.iter().all(|run| (5..=8).contains(run)), "{runs:?}");
    }

    #[test]
    fn a_waveform_follows_the_time_map_backwards_and_through_a_held_frame() {
        use editor_core::{Ease, Retime, TimeKey};
        let key = |at: i64, source: i64| TimeKey {
            at: seconds(at),
            source: seconds(source),
            ease: Ease::Linear,
        };
        // Two seconds forwards, one held, then the same two backwards.
        let mut clip = voice();
        clip.duration = seconds(5);
        clip.source_in = seconds(1);
        clip.retime = Some(Retime {
            keys: vec![key(0, 0), key(2, 2), key(3, 2), key(5, 0)],
        });
        let drawn = columns(&clip, Time::ZERO, Time::ZERO);
        assert_eq!(drawn.len(), 500);
        assert_eq!(drawn[0], (200.0, seconds(1), Time(1_010_000)));
        // The held second is one source time, from side to side.
        assert!(
            drawn[200..300]
                .iter()
                .all(|column| (column.1, column.2) == (seconds(3), seconds(3)))
        );
        // Backwards, a column has its later time first, and the stretch is
        // the forward one mirrored.
        assert_eq!(drawn[300], (500.0, seconds(3), Time(2_990_000)));
        for (index, column) in drawn[300..].iter().enumerate() {
            let mirror = drawn[199 - index];
            assert_eq!((column.1, column.2), (mirror.2, mirror.1));
        }
        assert_eq!(drawn[499].2, seconds(1));

        // The peaks read either end first: what is drawn is the same.
        let peaks = Peaks {
            buckets: (0..1000)
                .map(|index| [(-(index % 100)) as i8 as u8, (index % 100) as u8, 0, 0])
                .collect(),
        };
        for (index, column) in drawn[300..].iter().enumerate() {
            let mirror = drawn[199 - index];
            assert_eq!(
                peaks.range(column.1, column.2),
                peaks.range(mirror.1, mirror.2)
            );
            assert!(peaks.range(column.1, column.2).is_some());
        }
        // A held frame shows the one bucket it is in, flat.
        let flat: BTreeSet<_> = drawn[200..300]
            .iter()
            .map(|column| peaks.range(column.1, column.2))
            .collect();
        assert_eq!(flat.len(), 1);
    }
    /// The waveforms one pass of the whole timeline draws over `project` in
    /// a window of this size: the columns of each mesh of bare colour.
    fn waves_drawn(
        project: &Project,
        held: &mut HeldPeaks,
        state: &mut TimelineState,
        size: Vec2,
    ) -> Vec<Vec<Rect>> {
        fn meshes(shape: &egui::Shape, found: &mut Vec<Vec<Rect>>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| meshes(shape, found)),
                egui::Shape::Mesh(mesh) if mesh.texture_id == egui::TextureId::default() => {
                    let columns = mesh.vertices.chunks_exact(4);
                    found.push(
                        columns
                            .map(|corners| Rect::from_min_max(corners[0].pos, corners[3].pos))
                            .collect(),
                    );
                }
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let palette = Palette::new(theme::Appearance::Dark, theme::Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        held.begin_frame();
        let mut output = ctx.run_ui(input, |ui| {
            show(
                ui,
                ui.max_rect(),
                state,
                &mut Selection::default(),
                &mut Textures::default(),
                held,
                &TimelineInput {
                    p: &palette,
                    project,
                    playhead: Time::ZERO,
                    snapping: false,
                    media: None,
                    can_paste: false,
                    freeze_hold: seconds(2),
                    separator: crate::stems::Tool::Missing,
                },
                &mut Vec::new(),
            );
        });
        // No renderer takes the font texture: epaint is told it is meant.
        output.textures_delta.clear();
        let mut found = Vec::new();
        for clipped in &output.shapes {
            meshes(&clipped.shape, &mut found);
        }
        found
    }

    #[test]
    fn a_sound_tile_draws_one_mesh_over_what_is_in_view_once_its_peaks_are_held() {
        // A thirty second sound and the sound of a thirty second video, each
        // on a sound track of its own.
        let mut project = Project::new("Waves");
        project.tracks.clear();
        let mut media = Vec::new();
        for (kind, name) in [
            (MediaKind::Audio, "voice.wav"),
            (MediaKind::Video, "talk.mp4"),
        ] {
            let mut asset = Asset::new(format!("/media/{name}").into(), kind, seconds(30));
            asset.has_audio = true;
            let mut track = Track::new(name, TrackKind::Audio);
            track.clips.push(Clip::from_asset(&asset, seconds(1)));
            media.push(asset.clone());
            Arc::make_mut(&mut project.assets).insert(asset.id, asset);
            project.tracks.push(Arc::new(track));
        }
        let size = vec2(900.0, 300.0);
        let mut state = TimelineState::default();
        let mut held = HeldPeaks::default();

        // Before the peaks arrive nothing but the level line is drawn, and
        // both tiles have asked.
        assert!(waves_drawn(&project, &mut held, &mut state, size).is_empty());
        let wants = held.take_wants();
        assert_eq!(wants.len(), 2);
        // The voice's are held: its tile draws them and the other does not.
        let instance = Uuid::from_u128(7);
        let peaks = Arc::new(Peaks {
            buckets: (0..3000u32)
                .map(|index| {
                    // Silence for two seconds, then sound.
                    let level = if index < 200 { 0 } else { 100 };
                    [(-level) as i8 as u8, level as u8, 0, 0]
                })
                .collect(),
        });
        for (request, (id, _)) in wants.into_iter().enumerate() {
            held.asked(id, request as u64);
        }
        let voice = &media[0];
        let taken = held.take(
            instance,
            Some(voice),
            0,
            instance,
            voice.id,
            &SourceIdentity::of(voice),
            Answer::Kept(KeptPeaks::Held(peaks.clone())),
        );
        assert_eq!(taken, Taken::Kept);
        let drawn = waves_drawn(&project, &mut held, &mut state, size);
        assert_eq!(
            drawn.len(),
            1,
            "one mesh for the one tile whose peaks are held"
        );
        let columns = &drawn[0];
        // The lanes begin after the heads; the tile begins a second in, at
        // eighty points to the second, and runs out of the window. A column
        // for each point of it that is in view, and none for the rest.
        let lanes = Rect::from_min_max(pos2(HEAD_WIDTH, STRIP + RULER), size.to_pos2());
        let left = lanes.left() + 80.0 + 1.0;
        assert_eq!(columns[0].left(), left);
        assert_eq!(columns.len(), (lanes.right() - left).ceil() as usize);
        assert!(columns.len() < 30 * 80);
        assert!(columns.iter().all(|column| column.width() <= 1.0));
        assert!(columns.last().unwrap().right() <= lanes.right());
        // Under the name, inside the tile.
        let top = lanes.top() + LANE_GAP + 1.0 + NAME_ROW;
        let bottom = lanes.top() + LANE_GAP + lane_height(TrackKind::Audio) - 1.0;
        assert!(
            columns
                .iter()
                .all(|column| column.top() >= top && column.bottom() <= bottom)
        );
        // Silence is a line through the middle; sound stands about it.
        let middle = (top + bottom) * 0.5;
        let quiet = columns[100];
        assert_eq!((quiet.top(), quiet.bottom()), (middle - 0.5, middle + 0.5));
        let loud = columns[columns.len() - 1];
        let reach = 100.0 / 128.0 * (bottom - top) * 0.5;
        assert!((loud.top() - (middle - reach)).abs() < 1e-3, "{loud:?}");
        assert!((loud.bottom() - (middle + reach)).abs() < 1e-3, "{loud:?}");

        // Scrolled so that the tile's head is out of view, the mesh still
        // starts at the lanes and is no longer than they are wide.
        state.scroll.x = 800.0;
        let drawn = waves_drawn(&project, &mut held, &mut state, size);
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0][0].left(), lanes.left());
        assert_eq!(drawn[0].len(), lanes.width().ceil() as usize);

        // With the other's held too, the sound of a video is drawn as sound.
        let talk = &media[1];
        let taken = held.take(
            instance,
            Some(talk),
            1,
            instance,
            talk.id,
            &SourceIdentity::of(talk),
            Answer::Kept(KeptPeaks::Held(peaks)),
        );
        assert_eq!(taken, Taken::Kept);
        assert_eq!(waves_drawn(&project, &mut held, &mut state, size).len(), 2);

        // At the widest zoom, a second into the tiles: still a column a
        // point and no more of them than the lanes are wide, though the
        // tiles are 19,200 points long.
        state.zoom = MAX_ZOOM;
        state.scroll.x = 2.0 * MAX_ZOOM;
        let drawn = waves_drawn(&project, &mut held, &mut state, size);
        assert_eq!(drawn.len(), 2);
        for columns in &drawn {
            assert_eq!(columns[0].left(), lanes.left());
            assert_eq!(columns.len(), lanes.width().ceil() as usize);
            assert!(columns.iter().all(|column| column.width() <= 1.0));
        }
        // The silence ends two seconds into the sound, a second into view.
        let edge = MAX_ZOOM as usize;
        let tall = |column: &Rect| column.height() > 1.0;
        assert!(!drawn[0][..edge - 1].iter().any(tall));
        assert!(drawn[0][edge..].iter().all(tall));
    }
}
