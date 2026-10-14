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
    fades::{self, FadeEdge},
    icon_of_track,
    keyframes::{self, KeyframesState},
    shortcut, tone_of_track, tracks, transitions,
    widgets::{self, icon_button, icon_button_tinted, icon_toggle, menu_item, menu_style},
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
    self, Align2, Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2, pos2, vec2,
};

const STRIP: f32 = metrics::PANEL_HEADER;
const RULER: f32 = 26.0;
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
/// the source it shows. Silence is a line through the middle.
fn paint_wave(painter: &egui::Painter, area: Rect, clip: &Clip, wave: &Wave, ink: Color32) {
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
        mesh.add_colored_rect(column, ink);
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

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
    widgets::panel(ui, rect, p);
    let strip = Rect::from_min_size(rect.min, vec2(rect.width(), STRIP));
    let body = Rect::from_min_max(pos2(rect.left(), strip.bottom()), rect.max);
    ui.painter()
        .hline(rect.x_range(), strip.bottom(), p.hairline());

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

    // The corner over the track heads.
    let corner = Rect::from_min_max(body.min, pos2(heads.right(), ruler.bottom()));
    ui.painter().text(
        pos2(corner.left() + 14.0, corner.center().y),
        Align2::LEFT_CENTER,
        timefmt::timecode(input.playhead, project.settings.frame_rate),
        theme::mono(11.5),
        p.secondary,
    );

    // The beats the tiles show, which the ruler and the lanes snap to.
    state.beats = if state.keys.graph {
        Vec::new()
    } else {
        beats_drawn(project, &rows, &geometry, &mut peaks.beats)
    };
    draw_ruler(ui, ruler, &geometry, state, input, actions);
    state.keys.begin(ui, &geometry, selection, input, actions);
    if state.keys.graph {
        (state.keys).show_graph(ui, heads, lanes, &geometry, selection, input, actions);
    } else {
        track_heads(ui, heads, &rows, &geometry, &mut state.keys, input, actions);
        draw_lanes(
            ui, lanes, &rows, &geometry, state, selection, textures, peaks, input, actions,
        );
    }

    // The playhead runs through the ruler and the lanes.
    let x = geometry.x(input.playhead);
    if x >= lanes.left() - 1.0 && x <= lanes.right() {
        let painter = ui.painter_at(Rect::from_min_max(ruler.min, lanes.max));
        painter.vline(
            x,
            ruler.center().y..=lanes.bottom(),
            Stroke::new(1.5, p.accent),
        );
        let head = Rect::from_center_size(pos2(x, ruler.top() + 10.0), vec2(11.0, 14.0));
        painter.rect_filled(head, CornerRadius::same(3), p.accent);
    }
    if let Some(guide) = state.snap_guide {
        let x = geometry.x(guide);
        ui.painter_at(lanes).vline(
            x,
            lanes.y_range(),
            Stroke::new(1.0, p.yellow.gamma_multiply(0.9)),
        );
    }

    // Wheel, trackpad and pinch over the timeline.
    if ui.rect_contains_pointer(body) {
        let (scroll, zoom, pointer) = ui.input(|i| {
            (
                i.smooth_scroll_delta(),
                i.zoom_delta(),
                i.pointer.hover_pos(),
            )
        });
        if zoom != 1.0 {
            let anchor = pointer.map_or(state.lane_width * 0.5, |pos| {
                (pos.x - lanes.left()).clamp(0.0, state.lane_width)
            });
            state.zoom_about(zoom, anchor);
        } else if scroll != Vec2::ZERO {
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

fn tool_strip(
    ui: &mut Ui,
    strip: Rect,
    state: &mut TimelineState,
    selection: &Selection,
    input: &TimelineInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(strip.shrink2(vec2(10.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut ui;
    ui.spacing_mut().item_spacing.x = 2.0;
    let has_selection = !selection.clips.is_empty();

    if icon_button(
        ui,
        p,
        Icon::Scissors,
        &format!("Split at playhead ({})", "S"),
    )
    .clicked()
    {
        actions.push(Action::SplitAtPlayhead);
    }
    ui.add_enabled_ui(has_selection, |ui| {
        if icon_button(ui, p, Icon::Trash, "Delete (Delete)").clicked() {
            actions.push(Action::Delete { ripple: false });
        }
        if icon_button(
            ui,
            p,
            Icon::Copy,
            &format!("Duplicate ({})", shortcut("Cmd+D")),
        )
        .clicked()
        {
            actions.push(Action::Duplicate);
        }
    });
    ui.add_space(8.0);
    if icon_button(ui, p, Icon::Flag, "Add marker at playhead (M)").clicked() {
        actions.push(Action::AddMarker);
    }
    if icon_button(ui, p, Icon::Text, "Add text at playhead (T)").clicked() {
        actions.push(Action::AddText);
    }
    let add = icon_button(ui, p, Icon::Layers, "Add track");
    egui::Popup::menu(&add).show(|ui| {
        menu_style(ui, 180.0);
        for (kind, icon, label) in [
            (TrackKind::Video, Icon::Film, "Video track"),
            (TrackKind::Audio, Icon::Music, "Audio track"),
            (TrackKind::Text, Icon::Text, "Text track"),
        ] {
            if menu_item(ui, p, Some(icon), label, "").clicked() {
                actions.push(Action::AddTrack(kind));
                ui.close();
            }
        }
    });
    ui.add_space(8.0);
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
    if icon_toggle(ui, p, Icon::Curve, keys.graph, "Graph editor (G)").clicked() {
        keys.graph = !keys.graph;
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if icon_button(ui, p, Icon::Fit, "Fit timeline to window (Shift+Z)").clicked() {
            actions.push(Action::FitTimeline);
        }
        if icon_button(
            ui,
            p,
            Icon::ZoomIn,
            &format!("Zoom in ({})", shortcut("Cmd+=")),
        )
        .clicked()
        {
            actions.push(Action::ZoomTimeline(1.4));
        }
        // The slider works on a logarithmic scale, as zoom is perceived.
        let mut level = (state.zoom / MIN_ZOOM).ln() / (MAX_ZOOM / MIN_ZOOM).ln();
        let slider = ui.add_sized(
            [96.0, 18.0],
            egui::Slider::new(&mut level, 0.0..=1.0).show_value(false),
        );
        if slider.changed() {
            let zoom = MIN_ZOOM * (MAX_ZOOM / MIN_ZOOM).powf(level);
            state.zoom_centered(zoom / state.zoom);
        }
        slider.on_hover_text("Timeline zoom");
        if icon_button(
            ui,
            p,
            Icon::ZoomOut,
            &format!("Zoom out ({})", shortcut("Cmd+-")),
        )
        .clicked()
        {
            actions.push(Action::ZoomTimeline(1.0 / 1.4));
        }
        ui.add_space(8.0);
        let snap = icon_button_tinted(
            ui,
            p,
            Icon::Magnet,
            if input.snapping {
                "Snapping is on (N)"
            } else {
                "Snapping is off (N)"
            },
            input.snapping.then_some(p.accent),
        );
        if snap.clicked() {
            actions.push(Action::ToggleSnapping);
        }
    });
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

/// A 22-point toggle inside a track head. `ink` is its glyph's colour where
/// that says more than on or off.
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
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), p.hover);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.5, p.accent),
            StrokeKind::Inside,
        );
    }
    let color = if let Some(ink) = ink {
        ink
    } else if on {
        p.fg
    } else if response.hovered() {
        p.secondary
    } else {
        p.muted
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
        icon,
        color,
    );
    response.on_hover_text(label).clicked()
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
    let field = ui
        .horizontal(|ui| {
            ui.add_space(30.0);
            ui.label(
                egui::RichText::new("Volume")
                    .font(theme::regular(13.0))
                    .color(p.fg),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                let field = ui.add_sized(
                    [72.0, 22.0],
                    egui::DragValue::new(&mut shown)
                        .range(0.0..=tracks::MOST)
                        .speed(1.0)
                        .suffix(" %")
                        .max_decimals(2)
                        .update_while_editing(false),
                );
                field.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Track volume")
                });
                field
            })
            .inner
        })
        .inner;
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

fn track_heads(
    ui: &mut Ui,
    heads: Rect,
    rows: &[Row],
    geometry: &Geometry,
    keys: &mut KeyframesState,
    input: &TimelineInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(heads));
    ui.set_clip_rect(heads);
    let ui = &mut ui;
    for row in rows {
        let rect = head_rect(heads, row);
        if row.lane > 0.0 {
            // What each row of the keyframe lane holds, under the head.
            let below = Rect::from_min_size(rect.left_bottom(), vec2(rect.width(), row.lane));
            keys.lane_heads(ui, below, geometry, input, actions);
        }
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let track = row.track;
        let id = Id::new(("track-head", track.id));
        let response = ui.interact(rect, id, Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("Track {}", track.name),
            )
        });
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(metrics::CLIP_RADIUS + 2),
            p.control,
        );
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(metrics::CLIP_RADIUS + 2),
                Stroke::new(1.5, p.accent),
                StrokeKind::Inside,
            );
        }
        let tone = p.tone(tone_of_track(track.kind));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), Vec2::splat(14.0)),
            icon_of_track(track.kind),
            if track.hidden && track.kind != TrackKind::Audio {
                p.muted
            } else {
                tone
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
                silenced.then(|| p.muted.gamma_multiply(0.45)),
            ) {
                actions.push(Action::Edit(state(None, None, Some(!track.muted))));
            }
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
        // one it starts with, where the head is tall enough for both.
        let room = head_name_room(rect, toggles);
        let figure = tracks::figure(track).filter(|_| rect.height() >= HEAD_FIGURE);
        let galley = widgets::truncated(
            ui,
            &track.name,
            theme::medium(12.0),
            if track.locked { p.muted } else { p.fg },
            room.span(),
        );
        let lift = if figure.is_some() { 6.0 } else { 0.0 };
        ui.painter().galley(
            pos2(room.min, rect.center().y - lift - galley.size().y * 0.5),
            galley,
            p.fg,
        );
        if let Some(figure) = figure {
            let galley =
                widgets::truncated(ui, &figure, theme::regular(10.5), p.muted, room.span());
            ui.painter().galley(
                pos2(room.min, rect.center().y + 9.0 - galley.size().y * 0.5),
                galley,
                p.muted,
            );
        }

        // A click in the menu is the volume field's to take: each row closes
        // the menu itself, so only a click outside it does.
        let menu = egui::Popup::context_menu(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        menu.show(|ui| {
            menu_style(ui, 200.0);
            ui.add_enabled_ui(!track.locked, |ui| {
                if menu_item(ui, p, Some(Icon::Trash), "Remove track", "").clicked() {
                    actions.push(Action::Edit(Command::RemoveTrack { track_id: track.id }));
                    ui.close();
                }
            });
            if menu_item(ui, p, Some(Icon::Layers), "Add track of this kind", "").clicked() {
                actions.push(Action::AddTrack(track.kind));
                ui.close();
            }
            if tracks::carries_sound(track) {
                ui.separator();
                volume_menu(ui, p, track, actions);
            }
        });
    }
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
    painter.hline(ruler.x_range(), ruler.bottom() - 0.5, p.hairline());

    let (step, parts) = rules::ruler_step(geometry.zoom, rate.fps(), 92.0);
    let first = (f64::from(geometry.scroll) / f64::from(geometry.zoom) / step).floor() as i64;
    let last = (f64::from(geometry.scroll + ruler.width()) / f64::from(geometry.zoom) / step).ceil()
        as i64;
    for index in first..=last {
        let seconds = index as f64 * step;
        let x = geometry.x(Time::from_seconds(seconds));
        painter.vline(
            x,
            ruler.bottom() - 8.0..=ruler.bottom(),
            Stroke::new(1.0, p.border),
        );
        painter.text(
            pos2(x + 5.0, ruler.top() + 9.0),
            Align2::LEFT_CENTER,
            timefmt::ruler(seconds, step, rate),
            theme::regular(10.5),
            p.muted,
        );
        for part in 1..parts {
            let x = geometry.x(Time::from_seconds(
                seconds + step * f64::from(part) / f64::from(parts),
            ));
            painter.vline(
                x,
                ruler.bottom() - 4.0..=ruler.bottom(),
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
    let rect = Rect::from_center_size(pos2(x, ruler.bottom() - 8.0), vec2(12.0, 14.0));
    let response = ui.interact(rect, Id::new(("marker", marker.id)), Sense::click());
    let [r, g, b, _] = marker.color;
    let color = Color32::from_rgb(r, g, b);
    let painter = ui.painter_at(ruler);
    // A small pennant whose point marks the time.
    painter.add(egui::Shape::convex_polygon(
        vec![
            pos2(x - 4.5, rect.top()),
            pos2(x + 4.5, rect.top()),
            pos2(x + 4.5, rect.bottom() - 5.0),
            pos2(x, rect.bottom()),
            pos2(x - 4.5, rect.bottom() - 5.0),
        ],
        color,
        Stroke::new(1.0, if response.hovered() { p.fg } else { color }),
    ));
    if response.clicked() {
        actions.push(Action::Seek(marker.time));
    }
    let label = if marker.label.is_empty() {
        format!(
            "Marker at {}",
            timefmt::timecode(marker.time, input.project.settings.frame_rate)
        )
    } else {
        marker.label.clone()
    };
    let response = response.on_hover_text(label);
    response.context_menu(|ui| {
        menu_style(ui, 180.0);
        if menu_item(ui, p, Some(Icon::Trash), "Remove marker", "").clicked() {
            actions.push(Action::Edit(Command::RemoveMarker {
                marker_id: marker.id,
            }));
            ui.close();
        }
    });
}

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
/// darkened above the curve it follows.
fn paint_fades(
    painter: &egui::Painter,
    tile: Rect,
    area: Rect,
    clip: &Clip,
    zoom: f32,
    ink: Color32,
    opacity: f32,
) {
    let visible = area.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let shade = Color32::from_black_alpha((96.0 * opacity) as u8);
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
            Stroke::new(1.5, ink.gamma_multiply(0.85 * opacity)),
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

/// Draws the volume line of a clip over its sound, a dot at each volume
/// keyframe in view, and beside a held line the volume it is held at, as
/// [`volume_readout`] places it. Held, `held` is the accent and where
/// across the line was taken hold of. Whether the held percent was said:
/// when the fades leave it no room, the name's row says it.
fn paint_volume(
    painter: &egui::Painter,
    tile: Rect,
    area: Rect,
    clip: &Clip,
    zoom: f32,
    (ink, held): (Color32, Option<(Color32, f32)>),
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
    let stroke = match held {
        Some((accent, _)) => Stroke::new(1.5, accent),
        None => Stroke::new(1.0, ink.gamma_multiply(0.8 * opacity)),
    };
    painter.add(egui::Shape::line(line, stroke));
    let keys = (clip.animation.iter())
        .find(|track| track.target.is_none() && track.key == ParamKey::Gain)
        .map_or(&[][..], |track| &track.keys);
    for key in keys {
        let x = tile.left() + key.at.as_seconds() as f32 * zoom;
        if x >= within.0 - 2.0 && x <= within.1 + 2.0 {
            let gain = gain_at(clip, clip.start + key.at);
            let y = area.bottom() - volume_level(gain) * area.height();
            painter.circle_filled(pos2(x, y), 2.0, stroke.color);
        }
    }
    let Some((_, x)) = held else {
        return false;
    };
    let galley = painter.layout_no_wrap(volume_percent(clip), theme::mono(10.0), ink);
    let y = volume_y(clip, tile, area, zoom, x);
    match volume_readout(tile, area, clip, zoom, (x, y), galley.size()) {
        Some(readout) => {
            painter.galley(readout.min, galley, ink);
            true
        }
        None => false,
    }
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
    textures: &mut Textures,
    peaks: &mut HeldPeaks,
    input: &TimelineInput,
    actions: &mut Vec<Action>,
) {
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

    // Lanes.
    for row in rows {
        let lane = Rect::from_min_max(
            pos2(lanes.left(), row.top),
            pos2(lanes.right(), row.top + row.height),
        );
        if !lane.intersects(lanes) {
            continue;
        }
        painter.rect_filled(
            lane,
            CornerRadius::ZERO,
            if row.track.locked {
                p.sunken.gamma_multiply(0.55)
            } else {
                p.sunken.gamma_multiply(0.8)
            },
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
    for (row, clip) in clips_drawn(rows, geometry) {
        let mut start = clip.start;
        let mut end = clip.end();
        let mut dimmed = row.track.locked || row.track.hidden;
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
        }
        if moving.contains(&clip.id) {
            // Where it came from stays faintly visible under the ghost.
            dimmed = true;
        }
        let rect = clip_rect(geometry, row, start, end);
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
            painter.rect_stroke(
                rect.expand(2.0),
                CornerRadius::same(metrics::CLIP_RADIUS + 2),
                Stroke::new(2.0, p.accent),
                StrokeKind::Outside,
            );
        }
    }
    if response.has_focus() {
        painter.rect_stroke(
            lanes,
            CornerRadius::ZERO,
            Stroke::new(1.5, p.accent),
            StrokeKind::Inside,
        );
    }

    // The clips being moved, where they would land, with the beats they
    // show there.
    if let Some(Drag::Move {
        delta,
        target,
        moved: true,
        ..
    }) = &state.drag
    {
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
            };
            let wave = wave_of(project, track, clip, peaks, geometry, *delta);
            paint_clip(
                ui, &painter, rect, project, track, clip, true, 0.92, 0.0, &marks, textures, wave,
                input,
            );
        }
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
            if handle.has_focus() {
                over_lanes.painter().rect_stroke(
                    rect.shrink2(vec2(2.0, 3.0)),
                    CornerRadius::same(3),
                    Stroke::new(1.5, p.accent),
                    StrokeKind::Inside,
                );
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
            let hot = held.is_some() || handle.hovered() || handle.has_focus();
            let centre = rect.center();
            let painter = over_lanes.painter();
            painter.circle_filled(centre, 4.0, if hot { p.accent } else { p.elevated });
            painter.circle_stroke(centre, 4.0, Stroke::new(1.0, p.fg));
            if handle.has_focus() {
                painter.rect_stroke(
                    rect.expand(1.0),
                    CornerRadius::same(3),
                    Stroke::new(1.5, p.accent),
                    StrokeKind::Inside,
                );
            }
            // How long the held fade would be, beside its handle and inside
            // the tile.
            if let Some((.., duration)) = held {
                let (align, x) = match edge {
                    FadeEdge::In => (Align2::LEFT_CENTER, rect.right() + 3.0),
                    FadeEdge::Out => (Align2::RIGHT_CENTER, rect.left() - 3.0),
                };
                painter.text(
                    pos2(x, centre.y),
                    align,
                    format!("{:.2} s", duration.as_seconds()),
                    theme::mono(10.0),
                    p.fg,
                );
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
                Rect::from_center_size(centre, Vec2::splat(9.0)),
                Icon::Plus,
                if hot { p.on_accent } else { p.fg },
            );
            if hot {
                over_lanes.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            badge.on_hover_text(format!("{label} ({})", shortcut("Cmd+T")));
        }
    }

    if let Some(rect) = marquee_rect {
        painter.rect_filled(rect, CornerRadius::same(3), theme::tint(p.accent, 0.14));
        painter.rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, p.accent),
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
    response.context_menu(|ui| {
        menu_style(ui, 220.0);
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
            && menu_item(ui, p, Some(Icon::Transition), "Edit transition", "").clicked()
        {
            marked = Some((id, edge));
            ui.close();
        }
        let mut item = |ui: &mut Ui, icon: Icon, text: &str, keys: &str, action: Action| {
            if menu_item(ui, p, Some(icon), text, &shortcut(keys)).clicked() {
                actions.push(action);
                ui.close();
            }
        };
        if menu_clip.is_some() {
            item(
                ui,
                Icon::Scissors,
                "Split at playhead",
                "S",
                Action::SplitAtPlayhead,
            );
            item(ui, Icon::Copy, "Duplicate", "Cmd+D", Action::Duplicate);
            // The clip and the playhead of this frame, whatever is selected
            // by the time the edit is made.
            if let Some(clip) = menu_clip {
                let freeze =
                    speed::freeze_command(project, clip, input.playhead, input.freeze_hold);
                ui.add_enabled_ui(freeze.is_ok(), |ui| {
                    let action = match freeze {
                        Ok(command) => Action::Edit(command),
                        Err(reason) => Action::Notify(reason),
                    };
                    item(ui, Icon::Pause, "Freeze frame", "F", action);
                });
                // Offered of a picture clip with sound; one whose sound is
                // on an audio clip already, or whose track is locked, shows
                // it without being able to choose it.
                let sounds = (project.clip(clip))
                    .is_some_and(|(track, held)| rules::carries_sound(project, track, held));
                if sounds {
                    let free = rules::detach_refusal(project, clip).is_none();
                    ui.add_enabled_ui(free, |ui| {
                        item(
                            ui,
                            Icon::Volume,
                            "Detach audio",
                            "",
                            Action::DetachAudio(clip),
                        );
                    });
                }
            }
            // The fades of the selected clips' sound, by rows the keyboard
            // reaches: offered of a clip whose media has sound, and shown
            // without being able to be chosen where no selected clip would
            // be changed.
            let fades_sound = menu_clip
                .and_then(|id| project.clip(id))
                .is_some_and(|(_, held)| fades::has_sound(project, held));
            if fades_sound {
                for (label, asked) in fades::MENU {
                    let made = fades::command(project, &selection.clips, asked);
                    ui.add_enabled_ui(made.is_ok(), |ui| {
                        let icon = match asked {
                            fades::Asked::Set(..) => Icon::Volume,
                            fades::Asked::Remove => Icon::Trash,
                        };
                        let action = match made {
                            Ok(command) => Action::Edit(command),
                            Err(reason) => Action::Notify(reason),
                        };
                        item(ui, icon, label, "", action);
                    });
                }
            }
            // A key of the clip's volume at the playhead, at the volume it
            // has there: the lane it is in then opens. Shown without being
            // able to be chosen where none would be written.
            if let Some(clip) = menu_clip.filter(|_| fades_sound) {
                let made = keyframes::volume_key(project, clip, input.playhead);
                ui.add_enabled_ui(made.is_ok(), |ui| {
                    if menu_item(ui, p, Some(Icon::Diamond), "Key volume", "").clicked() {
                        keyed = made.ok().map(|command| (clip, command));
                        ui.close();
                    }
                });
                // The selected clips' loudness is measured and each one's
                // volume set as its figures come. Shown without being able
                // to be chosen where none of them would be measured.
                let taken = normalize::takers(project, &selection.clips);
                ui.add_enabled_ui(taken.is_ok(), |ui| {
                    let action = match taken {
                        Ok(clips) => Action::Normalize(clips),
                        Err(reason) => Action::Notify(reason),
                    };
                    item(ui, Icon::Volume, "Normalize loudness", "", action);
                });
                // On a clip of an audio lane: the selected clips are lowered
                // wherever the others are heard, at the default depth, once
                // their sound is read. Shown without being able to be chosen
                // where none of them would be ducked.
                let on_audio =
                    (project.clip(clip)).is_some_and(|(track, _)| track.kind == TrackKind::Audio);
                if on_audio {
                    // The whole selection goes: a clip passed over is
                    // still no voice.
                    let taken = duck::takers(project, &selection.clips);
                    ui.add_enabled_ui(taken.is_ok(), |ui| {
                        let action = match taken {
                            Ok(_) => Action::Duck {
                                clips: selection.clips.clone(),
                                depth: duck::DEPTH,
                            },
                            Err(reason) => Action::Notify(reason),
                        };
                        item(ui, Icon::Volume, duck::LABEL, "", action);
                    });
                }
                // The beats of the clip's media, on each of its tiles, for
                // the session; and while they are shown, a marker on each
                // beat of this clip, shown without being able to be chosen
                // until they are found.
                let asset = (project.clip(clip))
                    .and_then(|(_, held)| held.asset_id())
                    .filter(|_| on_audio);
                if let Some(asset) = asset {
                    let shown = peaks.beats.shown(asset);
                    let label = if shown { "Hide beats" } else { "Show beats" };
                    let action = Action::ShowBeats {
                        assets: vec![asset],
                        shown: !shown,
                    };
                    item(ui, Icon::Music, label, "", action);
                    if shown {
                        let made = peaks.beats.markers(project, clip);
                        ui.add_enabled_ui(made.is_ok(), |ui| {
                            let action = match made {
                                Ok(command) => Action::Edit(command),
                                Err(reason) => Action::Notify(reason),
                            };
                            item(ui, Icon::Flag, peaks::BEAT_MARKERS, "", action);
                        });
                    }
                }
            }
            // The span of its media's sound the clip plays, to a file:
            // offered of a clip whose media has sound, and shown without
            // being able to be chosen when that span has none.
            if let Some(clip) = menu_clip.filter(|_| fades_sound) {
                let source = crate::extract::Source::Clip(clip);
                let refused = crate::extract::refusal(project, source);
                ui.add_enabled_ui(refused.is_none(), |ui| {
                    let action = match refused {
                        None => Action::SaveAudio(source),
                        Some(reason) => Action::Notify(reason),
                    };
                    item(ui, Icon::Save, crate::extract::LABEL, "", action);
                });
                // The whole of its media's sound, split by the stem
                // separator and placed under the clip when it comes. Shown
                // without being able to be chosen, and saying why, without a
                // separator or for a clip that is not heard.
                let refused = (input.separator.refusal().map(str::to_owned))
                    .or_else(|| crate::stems::refusal(project, clip));
                ui.add_enabled_ui(refused.is_none(), |ui| {
                    let row = menu_item(ui, p, Some(Icon::Music), crate::stems::LABEL, "");
                    if row.clicked() {
                        separated = Some(clip);
                        ui.close();
                    }
                    if let Some(reason) = refused {
                        row.on_disabled_hover_text(reason);
                    }
                });
            }
            match (edge, place) {
                (_, Some((id, side))) if has => item(
                    ui,
                    Icon::Trash,
                    "Remove transition",
                    "",
                    Action::Edit(transitions::set_command(id, side, None)),
                ),
                (Some((id, side)), Some(_)) => item(
                    ui,
                    Icon::Transition,
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
            ui.separator();
            item(ui, Icon::Scissors, "Cut", "Cmd+X", Action::Cut);
            item(ui, Icon::Copy, "Copy", "Cmd+C", Action::Copy);
            ui.separator();
            let ids = selection.clips.clone();
            let several = ids.len() > 1;
            let linked = ids.iter().any(|id| {
                project
                    .clip(*id)
                    .is_some_and(|(_, c)| c.linked_id.is_some())
            });
            let grouped = ids
                .iter()
                .any(|id| project.clip(*id).is_some_and(|(_, c)| c.group_id.is_some()));
            if several {
                item(
                    ui,
                    Icon::Link,
                    "Link clips",
                    "",
                    Action::Edit(Command::LinkClips {
                        clip_ids: ids.clone(),
                    }),
                );
                item(
                    ui,
                    Icon::Layers,
                    "Group clips",
                    "Cmd+G",
                    Action::Edit(Command::GroupClips {
                        clip_ids: ids.clone(),
                    }),
                );
            }
            if linked {
                item(
                    ui,
                    Icon::Link,
                    "Unlink",
                    "",
                    Action::Edit(Command::UnlinkClips {
                        clip_ids: ids.clone(),
                    }),
                );
            }
            if grouped {
                item(
                    ui,
                    Icon::Layers,
                    "Ungroup",
                    "Shift+Cmd+G",
                    Action::Edit(Command::UngroupClips { clip_ids: ids }),
                );
            }
            if several || linked || grouped {
                ui.separator();
            }
            item(
                ui,
                Icon::Trash,
                "Delete",
                "Delete",
                Action::Delete { ripple: false },
            );
            item(
                ui,
                Icon::Trash,
                "Ripple delete",
                "Shift+Delete",
                Action::Delete { ripple: true },
            );
        } else {
            ui.add_enabled_ui(input.can_paste, |ui| {
                item(ui, Icon::Clipboard, "Paste", "Cmd+V", Action::Paste);
            });
            item(
                ui,
                Icon::Flag,
                "Add marker here",
                "",
                Action::Edit(marker_at(menu_time)),
            );
            item(
                ui,
                Icon::Pointer,
                "Move playhead here",
                "",
                Action::Seek(menu_time),
            );
        }
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

/// How a clip's transitions are marked on its tile, and whether its volume
/// line is held.
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
    let tone = p.tone(clip_tone(project, track, clip));
    let radius = CornerRadius::same(metrics::CLIP_RADIUS.min((rect.width() * 0.5) as u8));
    let body = theme::mix(p.sunken, tone, if selected { 0.52 } else { 0.36 });
    painter.rect_filled(rect, radius, body.gamma_multiply(opacity));

    let inner = rect.shrink(1.0);
    let clipped = painter.with_clip_rect(inner.intersect(painter.clip_rect()));
    let tall = carries_poster(rect.height());
    let label_height = if tall { NAME_ROW } else { rect.height() };

    // A held volume line whose fades leave its sound no room for its percent
    // says it in the name's row.
    let mut unsaid = false;
    // A sound lane draws sound, of a video too; a picture lane the poster
    // frame at the head of the clip.
    if track.kind == TrackKind::Audio {
        let area = sound_area(rect);
        match &wave {
            // The sound of the clip, under its name. It is the source's:
            // the clip's volume does not show in it.
            Some(wave) => {
                let ink = tone.gamma_multiply(0.9 * opacity);
                paint_wave(&clipped, area, clip, wave, ink);
            }
            // Until its peaks are held, a level line says this is sound,
            // where the silence of a waveform is drawn.
            None => {
                clipped.hline(
                    inner.x_range(),
                    area.center().y,
                    Stroke::new(1.0, tone.gamma_multiply(0.7 * opacity)),
                );
            }
        }
        let ink = if p.dark {
            Color32::WHITE
        } else {
            theme::mix(tone, Color32::BLACK, 0.72)
        };
        // The beats of its music, at the foot of the sound.
        paint_beats(
            &clipped,
            area,
            &marks.beats,
            ink.gamma_multiply(0.75 * opacity),
        );
        // The fades of the sound, over it: how much of it is let through
        // from each edge.
        paint_fades(&clipped, rect, area, clip, marks.zoom, ink, opacity);
        // The clip's volume, over both: a line where it stands, bent by its
        // keyframes.
        if fades::has_sound(project, clip) {
            let inks = (ink, marks.volume.map(|x| (p.accent, x)));
            let said = paint_volume(&clipped, rect, area, clip, marks.zoom, inks, opacity);
            unsaid = marks.volume.is_some() && !said;
        }
    } else if tall
        && let ClipContent::Media { asset_id } = &clip.content
        && let Some(texture) = textures.poster(*asset_id)
    {
        let size = texture.size_vec2();
        let height = inner.height() - label_height;
        let width = height * size.x / size.y.max(1.0);
        let image = Rect::from_min_size(
            pos2(inner.left(), inner.top() + label_height),
            vec2(width, height),
        );
        clipped.image(
            texture.id(),
            image,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE.gamma_multiply(opacity),
        );
    }

    // A transition: a ramp over the part of the clip it lasts, rising from
    // the edge it brings the clip in at and falling to the one it takes it
    // out at, with a bar at its inner end.
    for edge in transitions::EDGES {
        let Some(transition) = transitions::of(clip, edge) else {
            continue;
        };
        let duration = (marks.held)
            .filter(|(held, _)| *held == edge)
            .map_or(transition.duration, |(_, duration)| duration);
        let mark = marking(rect, marks.zoom, edge, duration).intersect(inner);
        let selected = marks.selected == Some(edge);
        let ink = if p.dark {
            Color32::WHITE
        } else {
            theme::mix(tone, Color32::BLACK, 0.72)
        };
        let line = if selected { p.accent } else { ink };
        clipped.rect_filled(
            mark,
            CornerRadius::ZERO,
            ink.gamma_multiply(if selected { 0.3 } else { 0.18 } * opacity),
        );
        let (ramp, end) = match edge {
            TransitionEdge::In => ([mark.left_bottom(), mark.right_top()], mark.right()),
            TransitionEdge::Out => ([mark.left_top(), mark.right_bottom()], mark.left()),
        };
        clipped.line_segment(ramp, Stroke::new(1.5, line.gamma_multiply(0.85 * opacity)));
        clipped.vline(
            end,
            mark.y_range(),
            Stroke::new(
                if selected { 2.0 } else { 1.0 },
                line.gamma_multiply(opacity),
            ),
        );
    }

    // How the clip is retimed, at the trailing end of its name's row. The
    // name gives way to it.
    let mut named = inner.width() - 12.0 - indent;
    let mut right = inner.right() - if clip.linked_id.is_some() { 20.0 } else { 5.0 };
    if inner.width() > 96.0
        && let Some(badge) = speed::badge(clip)
    {
        let galley = widgets::truncated(
            ui,
            &badge,
            theme::medium(10.0),
            Color32::WHITE.gamma_multiply(0.86 * opacity),
            (inner.width() * 0.5).clamp(48.0, 72.0),
        );
        let pill = Rect::from_min_size(
            pos2(right - galley.size().x - 8.0, inner.top() + 2.5),
            vec2(galley.size().x + 8.0, 13.0),
        );
        named = pill.left() - inner.left() - 10.0 - indent;
        clipped.rect_filled(
            pill,
            CornerRadius::same(4),
            Color32::from_black_alpha((96.0 * opacity) as u8),
        );
        clipped.galley(
            pos2(pill.left() + 4.0, pill.center().y - galley.size().y * 0.5),
            galley,
            Color32::WHITE,
        );
        right = pill.left() - 4.0;
    }
    // The percent of a held volume line its fades leave no room for, at the
    // trailing end of the name's row, before how the clip is retimed. The
    // name gives way to it too, wholly where not even its ellipsis would fit.
    let mut said_in_row = false;
    if unsaid && tall {
        let white = Color32::WHITE;
        let galley = clipped.layout_no_wrap(volume_percent(clip), theme::mono(10.0), white);
        let pill = Rect::from_min_size(
            pos2(right - galley.size().x - 8.0, inner.top() + 2.5),
            vec2(galley.size().x + 8.0, 13.0),
        );
        if pill.left() >= inner.left() + 2.0 {
            named = pill.left() - inner.left() - 10.0 - indent;
            clipped.rect_filled(
                pill,
                CornerRadius::same(4),
                Color32::from_black_alpha((96.0 * opacity) as u8),
            );
            clipped.galley(
                pos2(pill.left() + 4.0, pill.center().y - galley.size().y * 0.5),
                galley,
                white,
            );
            said_in_row = true;
        }
    }

    if inner.width() > 22.0 {
        let ink = if p.dark {
            Color32::WHITE
        } else {
            theme::mix(tone, Color32::BLACK, 0.72)
        };
        let galley = widgets::truncated(
            ui,
            &clip.name,
            theme::medium(11.0),
            ink.gamma_multiply(opacity),
            named,
        );
        let y = if tall {
            inner.top() + 2.0
        } else {
            inner.center().y - galley.size().y * 0.5
        };
        if !(said_in_row && galley.size().x > named) {
            clipped.galley(pos2(inner.left() + 6.0 + indent, y), galley, ink);
        }
    }
    if clip.linked_id.is_some() && inner.width() > 60.0 {
        icons::paint(
            &clipped,
            Rect::from_center_size(
                pos2(inner.right() - 10.0, inner.top() + 9.0),
                Vec2::splat(11.0),
            ),
            Icon::Link,
            Color32::WHITE.gamma_multiply(0.7 * opacity),
        );
    }

    if selected {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.5, p.fg.gamma_multiply(opacity.max(0.6))),
            StrokeKind::Inside,
        );
        // Grips say the edges can be pulled.
        if rect.width() > 26.0 {
            for x in [rect.left() + 3.5, rect.right() - 3.5] {
                painter.line_segment(
                    [
                        pos2(x, rect.center().y - 6.0),
                        pos2(x, rect.center().y + 6.0),
                    ],
                    Stroke::new(1.5, p.fg.gamma_multiply(0.8 * opacity)),
                );
            }
        }
    } else {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, Color32::from_black_alpha(60)),
            StrokeKind::Inside,
        );
    }
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
