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
use super::{
    Action, Selection,
    browser::{self, Entry, EntryDrag},
    icon_of_track,
    keyframes::{self, KeyframesState},
    shortcut,
    text_tab::{self, LookDrag},
    tone_of_track, transitions,
    widgets::{self, icon_button, icon_button_tinted, icon_toggle, menu_item, menu_style},
};
use crate::{
    icons::{self, Icon},
    rules::{self, Edge},
    speed,
    textures::Textures,
    theme::{self, Palette, metrics},
    timefmt,
};
use editor_core::{
    Asset, AssetId, Clip, ClipContent, ClipId, Command, Marker, Project, Time, Track, TrackId,
    TrackKind, Transition, TransitionEdge, snap_time,
};
use editor_media::MediaService;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2, pos2, vec2,
};

const STRIP: f32 = metrics::PANEL_HEADER;
const RULER: f32 = 26.0;
const HEAD_WIDTH: f32 = 156.0;
const LANE_GAP: f32 = 4.0;
const EDGE_GRIP: f32 = 7.0;
const SNAP_POINTS: f32 = 8.0;
/// The least a transition's marking is drawn wide, however short it is at
/// the zoom in view.
const MARK_MIN: f32 = 6.0;
/// The width of the handle at a marking's inner end.
const MARK_HANDLE: f32 = 10.0;
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

/// The media whose posters the lanes show: that of the clips drawn on rows
/// tall enough to carry one. Media comes up once for each of its clips.
fn poster_assets<'a>(
    project: &'a Project,
    rows: &'a [Row<'a>],
    geometry: &Geometry,
) -> impl Iterator<Item = &'a Asset> + use<'a> {
    clips_drawn(rows, geometry)
        .filter(|(row, _)| carries_poster(row.height))
        .filter_map(move |(_, clip)| clip.asset_id().and_then(|id| project.assets.get(&id)))
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

/// Snaps a time to the nearest clip edge, marker or the playhead within
/// reach. Returns the time and whether it snapped.
fn snapped(
    input: &TimelineInput,
    geometry: &Geometry,
    time: Time,
    exclude: &[ClipId],
) -> (Time, bool) {
    if !input.snapping {
        return (time, false);
    }
    let reach = geometry.span(SNAP_POINTS);
    let mut best = snap_time(input.project, time, reach, exclude);
    let to_playhead = (input.playhead.0 - time.0).abs();
    if to_playhead <= reach.0 && (best == time || to_playhead < (best.0 - time.0).abs()) {
        best = input.playhead;
    }
    (best, best != time)
}

pub fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &mut TimelineState,
    selection: &mut Selection,
    textures: &mut Textures,
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

    draw_ruler(ui, ruler, &geometry, state, input, actions);
    state.keys.begin(ui, &geometry, selection, input, actions);
    if state.keys.graph {
        (state.keys).show_graph(ui, heads, lanes, &geometry, selection, input, actions);
    } else {
        track_heads(ui, heads, &rows, &geometry, &mut state.keys, input, actions);
        draw_lanes(
            ui, lanes, &rows, &geometry, state, selection, textures, input, actions,
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
            &format!("Zoom in ({})", shortcut("Cmd+Scroll, =")),
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
            &format!("Zoom out ({})", shortcut("Cmd+Scroll, -")),
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

/// A 22-point toggle inside a track head.
fn head_toggle(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    id: Id,
    icon: Icon,
    on: bool,
    label: &str,
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
    let color = if on {
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
        let rect = Rect::from_min_size(
            pos2(heads.left() + 6.0, row.top),
            vec2(heads.width() - 10.0, row.height),
        );
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
        let mut right = rect.right() - 4.0;
        let mut toggle = |ui: &mut Ui, key: &str, icon: Icon, on: bool, label: &str| {
            let cell = Rect::from_min_size(
                pos2(right - 22.0, rect.center().y - 11.0),
                Vec2::splat(22.0),
            );
            right -= 23.0;
            head_toggle(ui, p, cell, id.with(key), icon, on, label)
        };
        let state = |locked, hidden, muted| Command::SetTrackState {
            track_id: track.id,
            locked,
            hidden,
            muted,
            name: None,
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
        ) {
            actions.push(Action::Edit(state(Some(!track.locked), None, None)));
        }
        if track.kind != TrackKind::Text
            && toggle(
                ui,
                "mute",
                if track.muted {
                    Icon::VolumeOff
                } else {
                    Icon::Volume
                },
                track.muted,
                if track.muted {
                    "Unmute track"
                } else {
                    "Mute track"
                },
            )
        {
            actions.push(Action::Edit(state(None, None, Some(!track.muted))));
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
            )
        {
            actions.push(Action::Edit(state(None, Some(!track.hidden), None)));
        }

        let name_left = rect.left() + 28.0;
        let galley = widgets::truncated(
            ui,
            &track.name,
            theme::medium(12.0),
            if track.locked { p.muted } else { p.fg },
            (right - name_left - 2.0).max(0.0),
        );
        ui.painter().galley(
            pos2(name_left, rect.center().y - galley.size().y * 0.5),
            galley,
            p.fg,
        );

        response.context_menu(|ui| {
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
        let (time, hit) = snapped(input, geometry, geometry.time(pos.x), &[]);
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
    // with animation, and the clip whose lane is open.
    let open = state.keys.lane_clip();
    let mut toggles: Vec<(ClipId, Rect)> = Vec::new();
    for (row, clip) in clips_drawn(rows, geometry) {
        if clip.animation.is_empty() || !(selection.contains(clip.id) || open == Some(clip.id)) {
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
    let spot = pointer
        .filter(|pos| lanes.contains(*pos) && ui.rect_contains_pointer(lanes))
        .filter(|_| state.drag.is_none() && !carrying)
        .and_then(|pos| add_spot(rows, geometry, pos))
        .filter(|spot| !handles.iter().any(|(.., rect)| rect.intersects(spot.badge)));
    // What a press on the lane, a chevron or a handle does is theirs, not
    // the clips'. The badge takes no press from the edge it sits on: see
    // `offer` below.
    let claimed = |pos: Pos2| {
        band.is_some_and(|band| band.contains(pos))
            || toggles.iter().any(|(_, rect)| rect.contains(pos))
            || handles.iter().any(|(.., rect)| rect.contains(pos))
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
            Hit::Clip {
                track,
                clip,
                edge,
                marking,
            } => {
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
                // Whichever edge of the held clip is nearer a guide wins.
                let (start, start_hit) = snapped(input, geometry, clip.start + wanted, &moving);
                let (end, end_hit) = snapped(input, geometry, clip.end() + wanted, &moving);
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
                let (time, hit) = snapped(input, geometry, raw, &[clip.id]);
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
                let (time, hit) = snapped(input, geometry, raw, &[clip.id]);
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
    let moving: Vec<ClipId> = match &state.drag {
        Some(Drag::Move { moved: true, .. }) => rules::related(project, &selection.clips, true),
        _ => Vec::new(),
    };
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
        let marks = Marks {
            zoom: geometry.zoom,
            selected: (state.transition)
                .filter(|(id, _)| *id == clip.id)
                .map(|(_, edge)| edge),
            held: held_mark
                .filter(|held| held.0 == clip.id)
                .map(|held| (held.1, held.2)),
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

    // The clips being moved, where they would land.
    if let Some(Drag::Move {
        delta,
        target,
        moved: true,
        ..
    }) = &state.drag
    {
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
            let marks = Marks {
                zoom: geometry.zoom,
                selected: None,
                held: None,
            };
            paint_clip(
                ui, &painter, rect, project, track, clip, true, 0.92, 0.0, &marks, textures, input,
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
        let (time, hit) = snapped(input, geometry, raw, &[]);
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
            let (time, hit) = snapped(input, geometry, raw, &[]);
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
                painter.rect_filled(rect, radius, theme::tint(tone, 0.35));
                painter.rect_stroke(rect, radius, Stroke::new(1.5, tone), StrokeKind::Inside);
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
            Hit::Clip { track, .. } if !track.locked => {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            }
            _ => {}
        }
    }
    match &state.drag {
        Some(Drag::Trim { .. }) => ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal),
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

/// How a clip's transitions are marked on its tile.
struct Marks {
    /// Points per second.
    zoom: f32,
    /// The edge whose transition is selected.
    selected: Option<TransitionEdge>,
    /// The edge whose transition is being dragged, and how long it would be.
    held: Option<(TransitionEdge, Time)>,
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

    // The poster frame at the head of a picture clip.
    if tall
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
    } else if track.kind == TrackKind::Audio {
        // No waveform data yet: a level line says this is sound.
        clipped.hline(
            inner.x_range(),
            inner.center().y + 6.0,
            Stroke::new(1.0, tone.gamma_multiply(0.7 * opacity)),
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
    if inner.width() > 96.0
        && let Some(badge) = speed::badge(clip)
    {
        let right = inner.right() - if clip.linked_id.is_some() { 20.0 } else { 5.0 };
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
        clipped.galley(pos2(inner.left() + 6.0 + indent, y), galley, ink);
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
mod tests {
    use super::*;
    use editor_core::MediaKind;
    use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

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
        // A sound lane is tall enough: the sound of a video draws its
        // poster. The low lane draws none, so none is asked for.
        let asked = media_asked_for(&project, &rows, &geometry);
        assert_eq!(asked, BTreeSet::from([media[0], media[1]]));
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
}
