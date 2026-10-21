//! The library's effects, transitions and looks: what a clip can be given,
//! as cards.
//!
//! A card is listed from the project's own tables, drawn with a few shapes
//! on a sample frame (nothing is decoded for it), and applied to the
//! selected clips with a double click or Enter, or to the clip it is dragged
//! onto. Either way the clips are captured when the card is let go of, and
//! what is made is a command or an [`Action::AddTransitions`] like any
//! other.
use super::{
    Action, Selection, inspector, kit,
    library::{self, Tab},
    transitions,
};
use crate::{
    icons::Icon,
    theme::{self, Palette, metrics},
};
use editor_core::{
    ClipContent, ClipId, Command, Effect, EffectKind, LookPreset, Project, Time, TrackKind,
    TransitionEdge, TransitionKind,
};
use eframe::egui::{
    self, Color32, CornerRadius, Id, Key, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
    Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};

/// What a card has under its well: its name, 6 below it.
const NAME_HEIGHT: f32 = 24.0;
/// Round the cards: room for their rings, which stand 6 out from them.
const MARGIN: egui::Margin = egui::Margin {
    left: library::LEADING as i8,
    right: library::TRAILING as i8,
    top: 6,
    bottom: 8,
};
/// How soon after a double click gave a tile another click is part of it.
const AGAIN: f64 = 0.35;

/// Something the library offers a clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    Effect(EffectKind),
    Transition(TransitionKind),
    Look(LookPreset),
}

/// The payload of a tile dragged toward the timeline.
#[derive(Clone, Copy, Debug)]
pub struct EntryDrag(pub Entry);

impl Entry {
    pub fn name(self) -> &'static str {
        match self {
            Self::Effect(kind) => inspector::effect_title(kind),
            Self::Transition(kind) => transitions::name(kind),
            Self::Look(preset) => (inspector::looks().iter())
                .find(|(listed, _)| *listed == preset)
                .map_or("Look", |(_, name)| name),
        }
    }

    /// What kind of thing it is, as a sentence names it.
    fn noun(self) -> &'static str {
        match self {
            Self::Effect(kind) if kind.is_audio() => "an audio effect",
            Self::Effect(_) => "an effect",
            Self::Transition(_) => "a transition",
            Self::Look(_) => "a look",
        }
    }
}

/// Whether the library offers an effect kind: one a clip takes as it is
/// made and the canvas draws or the sound goes through. A table or a clean
/// plate is chosen in the Color section, nothing draws a stabilization yet,
/// and a pitch is kept or shifted from the Playback card. The audio kinds
/// are listed under their own heading and go to clips on audio tracks only.
pub fn offered(kind: EffectKind) -> bool {
    !kind.needs_asset() && kind != EffectKind::Stabilize && kind != EffectKind::Pitch
}

/// What a tab lists, each under its heading: the effects by family, in the
/// families' order.
pub fn entries(tab: Tab) -> Vec<(&'static str, Entry)> {
    match tab {
        Tab::Media | Tab::Text | Tab::Captions => Vec::new(),
        Tab::Effects => {
            let mut kinds: Vec<(usize, &str, EffectKind)> = (EffectKind::ALL.into_iter())
                .filter(|kind| offered(*kind))
                .map(|kind| {
                    let (order, family) = inspector::effect_family(kind);
                    (order, family, kind)
                })
                .collect();
            kinds.sort_by_key(|(order, ..)| *order);
            (kinds.into_iter())
                .map(|(_, family, kind)| (family, Entry::Effect(kind)))
                .collect()
        }
        Tab::Transitions => (transitions::KINDS.into_iter())
            .map(|kind| ("", Entry::Transition(kind)))
            .collect(),
        Tab::Looks => (inspector::looks().iter())
            .map(|(preset, _)| ("", Entry::Look(*preset)))
            .collect(),
    }
}

/// Whether a clip takes picture effects and looks: a picture on a video
/// track. A sound on an audio track takes audio effects (`crate::audio`).
fn takes_effects(project: &Project, clip: ClipId) -> bool {
    project.clip(clip).is_some_and(|(track, clip)| {
        track.kind == TrackKind::Video && matches!(clip.content, ClipContent::Media { .. })
    })
}

/// What giving `entry` to `clips` asks of the application: one edit, or a
/// line that says what to select when there is nothing it can be given to.
/// A transition goes on `edge` of each clip, or on the edge nearest
/// `playhead` when none was pointed at.
pub fn apply(
    project: &Project,
    entry: Entry,
    clips: &[ClipId],
    playhead: Time,
    edge: Option<TransitionEdge>,
) -> Action {
    if clips.is_empty() {
        return Action::Notify(format!(
            "Select a clip on the timeline to give {} to, or drag it onto one.",
            entry.noun()
        ));
    }
    let pictures = || -> Vec<ClipId> {
        (clips.iter().copied())
            .filter(|clip| takes_effects(project, *clip))
            .collect()
    };
    let no_picture = || {
        Action::Notify(format!(
            "Only a picture clip takes {}: select a video or an image.",
            entry.noun()
        ))
    };
    match entry {
        // Heard, not drawn: it goes to the sounds of the clips, and a
        // picture is told why it takes none.
        Entry::Effect(kind) if kind.is_audio() => {
            match crate::audio::give_command(project, clips, crate::audio::Given::Kind(kind)) {
                Ok(command) => Action::Edit(command),
                Err(reason) => Action::Notify(reason),
            }
        }
        Entry::Effect(kind) => {
            let mut commands: Vec<Command> = (pictures().into_iter())
                .filter_map(|id| project.clip(id))
                .map(|(_, clip)| {
                    let mut effects = clip.effects.clone();
                    effects.insert(inspector::place_effect(&effects, kind), Effect::new(kind));
                    Command::SetEffects {
                        clip_id: clip.id,
                        effects,
                    }
                })
                .collect();
            match commands.len() {
                0 => no_picture(),
                1 => Action::Edit(commands.remove(0)),
                _ => Action::Edit(Command::Batch {
                    label: "Add effect".to_owned(),
                    commands,
                }),
            }
        }
        Entry::Look(preset) => match pictures() {
            clip_ids if clip_ids.is_empty() => no_picture(),
            clip_ids => Action::Edit(Command::ApplyPreset {
                clip_ids,
                preset,
                strength: 1.0,
            }),
        },
        Entry::Transition(kind) => Action::AddTransitions {
            places: match edge {
                Some(edge) => clips.iter().map(|clip| (*clip, edge)).collect(),
                None => transitions::at_playhead(project, clips, playhead),
            },
            kind: Some(kind),
        },
    }
}

pub struct BrowserState {
    pub search: String,
    /// The tile last clicked: the one Enter gives to the selected clips.
    pub picked: Option<Entry>,
    /// When a double click last gave a tile, in the frame's time.
    given_at: f64,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self {
            search: String::new(),
            picked: None,
            given_at: f64::NEG_INFINITY,
        }
    }
}

pub struct BrowserInput<'a> {
    pub p: &'a Palette,
    pub project: &'a Project,
    pub playhead: Time,
    pub tab: Tab,
}

/// The cards of one tab in `rect`, filtered by what is searched for.
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &mut BrowserState,
    selection: &mut Selection,
    input: &BrowserInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let query = state.search.trim().to_lowercase();
    let listed = entries(input.tab);
    let shown = |(family, entry): &(&str, Entry)| {
        query.is_empty()
            || entry.name().to_lowercase().contains(&query)
            || family.to_lowercase().contains(&query)
    };
    if !listed.iter().any(shown) {
        kit::empty_state(
            ui,
            p,
            rect,
            Icon::Search,
            "No matches",
            "Nothing here has that name.",
            None,
        );
        return;
    }

    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    body.set_clip_rect(rect.intersect(ui.clip_rect()));
    body.spacing_mut().item_spacing = Vec2::ZERO;
    let width = rect.width() - library::LEADING - library::TRAILING;
    let (columns, tile_width) = library::card_columns(width);
    let well = tile_width * 9.0 / 16.0;
    let tile_height = well + NAME_HEIGHT;
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let output = egui::ScrollArea::vertical()
        .id_salt(("browser", input.tab))
        .auto_shrink([false, false])
        .show(&mut body, |ui| {
            egui::Frame::NONE.inner_margin(MARGIN).show(ui, |ui| {
                ui.set_width(width);
                let mut heading = "";
                let mut column = 0;
                let mut row = Rect::NOTHING;
                for (family, entry) in listed.iter().filter(|listed| shown(listed)) {
                    if *family != heading {
                        heading = family;
                        column = 0;
                        library::heading(ui, p, heading);
                        ui.add_space(4.0);
                    }
                    if column == 0 {
                        (row, _) = ui.allocate_exact_size(
                            vec2(width, tile_height + library::CARD_ROW_GAP),
                            Sense::hover(),
                        );
                    }
                    let cell = Rect::from_min_size(
                        pos2(
                            row.left() + column as f32 * (tile_width + library::CARD_GAP),
                            row.top(),
                        ),
                        vec2(tile_width, tile_height),
                    );
                    column = (column + 1) % columns;
                    // Every card is named, so that each can be found; one
                    // reached from the keyboard comes into view.
                    tile(ui, cell, *entry, state, selection, input, enter, actions);
                }
            });
        });
    kit::scroll_fades(ui, &output, p.chrome);
}

#[expect(clippy::too_many_arguments, reason = "one call site per tile")]
fn tile(
    ui: &mut Ui,
    rect: Rect,
    entry: Entry,
    state: &mut BrowserState,
    selection: &mut Selection,
    input: &BrowserInput,
    enter: bool,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let name = entry.name();
    let response = ui.interact(
        rect,
        Id::new(("browser-tile", entry)),
        Sense::click_and_drag(),
    );
    let enabled = ui.is_enabled();
    let picked = state.picked == Some(entry);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, picked, name));

    let picture = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - NAME_HEIGHT));
    // Tab walks through the cards out of view too: the one it reaches is
    // scrolled to with room for its ring, and drawn from the next frame.
    if response.gained_focus() {
        ui.scroll_to_rect(rect.expand(8.0), None);
    }
    if ui.is_rect_visible(rect.expand(6.0)) {
        library::card_states(ui, p, rect, picture, &response, picked);
        let painter = ui.painter();
        painter.rect_filled(picture, metrics::CONTROL_RADIUS, p.stage());
        swatch(
            &within(painter, picture),
            picture,
            metrics::CONTROL_RADIUS,
            entry,
        );
        library::card_name(ui, p, picture, name, None);
    }

    if response.clicked() || response.drag_started() {
        state.picked = Some(entry);
        // Return is this tile's from here on, not a library item's.
        selection.asset = None;
        response.request_focus();
    }
    // The click that picked a tile may be the first of three: the third is
    // then the second of a double click. One that follows a click that gave
    // the tile already is not a second wish.
    let now = ui.input(|i| i.time);
    let again = response.double_clicked() || response.triple_clicked();
    let twice = again && now - state.given_at > AGAIN;
    if twice {
        state.given_at = now;
    }
    if twice || (response.has_focus() && enter) {
        actions.push(apply(
            input.project,
            entry,
            &selection.clips,
            input.playhead,
            None,
        ));
    }
    if response.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), EntryDrag(entry));
    }
    // What is being carried follows the pointer: its card's picture,
    // lifted.
    if response.dragged()
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        let layer = egui::LayerId::new(egui::Order::Tooltip, Id::new("browser-carry"));
        let mut carry = ui.ctx().layer_painter(layer);
        let lifted = Rect::from_min_size(pos + vec2(12.0, 10.0), vec2(64.0, 36.0));
        carry.add(p.popup_shadow().as_shape(lifted, 6));
        carry.set_opacity(0.85);
        carry.rect_filled(lifted, 6, p.stage());
        swatch(&within(&carry, lifted), lifted, 6, entry);
    }
    response.on_hover_text(match entry {
        Entry::Effect(kind) if kind.is_audio() => {
            "Double-click to add to the selected clips on audio tracks, or drag onto one"
        }
        Entry::Effect(_) => "Double-click to add to the selected clips, or drag onto a clip",
        Entry::Look(_) => "Double-click to give the selected clips this look, or drag onto a clip",
        Entry::Transition(_) => {
            "Double-click to put on the edge nearest the playhead, or drag onto a clip's edge"
        }
    });
}

// ---- Swatches -------------------------------------------------------------

/// A point of `rect` from parts of its width and height.
fn at(rect: Rect, x: f32, y: f32) -> Pos2 {
    pos2(
        rect.left() + rect.width() * x,
        rect.top() + rect.height() * y,
    )
}

/// A part of `rect`: left, top, right and bottom as parts of it.
fn part(rect: Rect, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect::from_min_max(at(rect, left, top), at(rect, right, bottom))
}

/// The outgoing picture of a transition: the sample frame in greys.
fn grey(level: f32) -> Color32 {
    Color32::from_gray((14.0 + level.clamp(0.0, 1.0) * 206.0).round() as u8)
}

/// The incoming picture, and the picture effects are shown on: the sample
/// frame in the steel of video's tone.
fn steel(level: f32) -> Color32 {
    theme::mix(
        Color32::from_rgb(14, 16, 24),
        Color32::from_rgb(198, 210, 240),
        level,
    )
}

fn white(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn black(alpha: f32) -> Color32 {
    Color32::from_black_alpha((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The sample frame every card of the tabs shows what it does to: a sky
/// lightening toward the horizon, a sun, two ridges and the ground. Each is
/// a level of `ink`, from dark (0) to light (1), so that a look can grade
/// it and a transition can show two of it. Rounded at `radius`, as the
/// well it fills is. Drawn with a handful of shapes, the same every frame.
fn sample(painter: &Painter, rect: Rect, radius: u8, ink: &dyn Fn(f32) -> Color32) {
    sample_in(painter, rect, rect, radius, ink);
}

/// The sample frame laid out over `frame` and seen through the well
/// `bounds`, rounded at `radius` where the well is. A picture pushed half
/// out of the well keeps the well's corners, and shows none of its own in
/// the middle of it. The painter is to clip to `bounds`.
fn sample_in(
    painter: &Painter,
    frame: Rect,
    bounds: Rect,
    radius: u8,
    ink: &dyn Fn(f32) -> Color32,
) {
    // Each band runs half a point under the next, so no seam shows, and is
    // rounded at the corners of the well it reaches. In a pushed picture it
    // runs a radius under, so that a sliver of it at the top of the well
    // still has room for the corner; such a picture is never seen through.
    let under = if frame == bounds {
        0.5
    } else {
        f32::from(radius).max(0.5)
    };
    let band = |top: f32, bottom: f32, level: f32| {
        let mut band = part(frame, 0.0, top, 1.0, bottom);
        if bottom < 1.0 {
            band.max.y += under;
        }
        let band = band.intersect(bounds);
        if !band.is_positive() {
            return;
        }
        let reaches = |a: f32, b: f32| (a - b).abs() < 0.5;
        let left = reaches(band.min.x, bounds.min.x);
        let right = reaches(band.max.x, bounds.max.x);
        let top = reaches(band.min.y, bounds.min.y);
        let bottom = reaches(band.max.y, bounds.max.y);
        let round = |at: bool| if at { radius } else { 0 };
        let corners = CornerRadius {
            nw: round(top && left),
            ne: round(top && right),
            sw: round(bottom && left),
            se: round(bottom && right),
        };
        painter.rect_filled(band, corners, ink(level));
    };
    band(0.0, 0.36, 0.3);
    band(0.36, 0.56, 0.38);
    band(0.56, 0.74, 0.47);
    painter.circle_filled(at(frame, 0.72, 0.34), frame.height() * 0.12, ink(0.95));
    let ridge = |points: [(f32, f32); 3], level: f32| {
        painter.add(Shape::convex_polygon(
            points.iter().map(|(x, y)| at(frame, *x, *y)).collect(),
            ink(level),
            Stroke::NONE,
        ));
    };
    ridge([(-0.06, 0.76), (0.32, 0.44), (0.74, 0.76)], 0.24);
    ridge([(0.34, 0.76), (0.7, 0.52), (1.08, 0.76)], 0.17);
    band(0.74, 1.0, 0.1);
}

/// A painter of `painter`'s that draws only inside `area` as well.
fn within(painter: &Painter, area: Rect) -> Painter {
    painter.with_clip_rect(area.intersect(painter.clip_rect()))
}

/// What an entry does, on the sample frame in `rect`, rounded at `radius`.
/// Its inks are neutral and the tones' own, never the accent: a card at
/// rest does not take it.
fn swatch(painter: &Painter, rect: Rect, radius: u8, entry: Entry) {
    match entry {
        Entry::Effect(kind) => effect_swatch(painter, rect, radius, kind),
        Entry::Transition(kind) => transition_swatch(painter, rect, radius, kind),
        Entry::Look(preset) => look_sample(painter, rect, radius, preset),
    }
}

/// A look on the sample frame in `rect`, rounded at `radius`: its shadows
/// and its lights toned as the look leaves them. The library's look cards
/// and the inspector's are drawn by it alike.
pub(crate) fn look_sample(painter: &Painter, rect: Rect, radius: u8, preset: LookPreset) {
    let (dark, light) = look_colours(preset);
    sample(painter, rect, radius, &|level| {
        theme::mix(dark, light, level)
    });
}

/// The two colours a look leaves in the shadows and in the lights.
fn look_colours(preset: LookPreset) -> (Color32, Color32) {
    let rgb = Color32::from_rgb;
    match preset {
        LookPreset::Cinematic => (rgb(18, 32, 40), rgb(226, 196, 150)),
        LookPreset::TealOrange => (rgb(12, 74, 84), rgb(244, 150, 72)),
        LookPreset::Vintage => (rgb(72, 50, 34), rgb(232, 212, 160)),
        LookPreset::Faded => (rgb(70, 74, 84), rgb(214, 212, 204)),
        LookPreset::Noir => (rgb(10, 10, 10), rgb(236, 236, 236)),
        LookPreset::Warm => (rgb(92, 42, 20), rgb(255, 204, 138)),
        LookPreset::Cool => (rgb(20, 40, 86), rgb(170, 214, 255)),
    }
}

/// A transition between two pictures of the sample frame: the outgoing in
/// greys, the incoming in steel.
fn transition_swatch(painter: &Painter, rect: Rect, radius: u8, kind: TransitionKind) {
    use TransitionKind as K;
    let from = |painter: &Painter, rect: Rect| sample(painter, rect, radius, &grey);
    let to = |painter: &Painter, rect: Rect| sample(painter, rect, radius, &steel);
    // An arrowhead at `tip`, pointing along `towards`.
    let arrow = |tip: Pos2, towards: Vec2| {
        let back = tip - towards * 5.0;
        let side = towards.rot90() * 3.5;
        painter.add(Shape::line(
            vec![back + side, tip, back - side],
            Stroke::new(1.5, white(0.9)),
        ));
    };
    match kind {
        K::Dissolve => {
            from(&within(painter, part(rect, 0.0, 0.0, 0.62, 1.0)), rect);
            to(&within(painter, part(rect, 0.62, 0.0, 1.0, 1.0)), rect);
            // Where the two are seen through each other.
            sample(
                &within(painter, part(rect, 0.38, 0.0, 0.62, 1.0)),
                rect,
                radius,
                &|level| steel(level).gamma_multiply(0.5),
            );
        }
        K::Fade => {
            from(&within(painter, part(rect, 0.0, 0.0, 0.42, 1.0)), rect);
            to(&within(painter, part(rect, 0.58, 0.0, 1.0, 1.0)), rect);
            painter.rect_filled(part(rect, 0.42, 0.0, 0.58, 1.0), 0, Color32::from_gray(6));
        }
        K::SlideLeft | K::SlideRight | K::SlideUp | K::SlideDown => {
            let towards = match kind {
                K::SlideLeft => vec2(-1.0, 0.0),
                K::SlideRight => vec2(1.0, 0.0),
                K::SlideUp => vec2(0.0, -1.0),
                _ => vec2(0.0, 1.0),
            };
            // The incoming picture has pushed the outgoing one half way.
            let size = vec2(rect.width(), rect.height());
            let half = size * towards * 0.5;
            sample_in(painter, rect.translate(half), rect, radius, &grey);
            let incoming = rect.translate(half - size * towards);
            sample_in(painter, incoming, rect, radius, &steel);
            // On a disc of its own, as the play offer is, so that it reads
            // over whatever of the frame it lands on.
            let middle = incoming.intersect(rect).center();
            painter.circle_filled(middle, 9.0, Color32::from_black_alpha(140));
            arrow(middle + towards * 2.5, towards);
        }
        K::WipeLeft | K::WipeRight | K::WipeUp | K::WipeDown => {
            from(painter, rect);
            let (shown, edge) = match kind {
                K::WipeLeft => (part(rect, 0.5, 0.0, 1.0, 1.0), [0.5, 0.0, 0.5, 1.0]),
                K::WipeRight => (part(rect, 0.0, 0.0, 0.5, 1.0), [0.5, 0.0, 0.5, 1.0]),
                K::WipeUp => (part(rect, 0.0, 0.5, 1.0, 1.0), [0.0, 0.5, 1.0, 0.5]),
                _ => (part(rect, 0.0, 0.0, 1.0, 0.5), [0.0, 0.5, 1.0, 0.5]),
            };
            to(&within(painter, shown), rect);
            painter.line_segment(
                [at(rect, edge[0], edge[1]), at(rect, edge[2], edge[3])],
                Stroke::new(1.5, white(0.9)),
            );
        }
        K::ZoomIn | K::ZoomOut => {
            let inner = part(rect, 0.24, 0.22, 0.76, 0.78);
            if kind == K::ZoomIn {
                from(painter, rect);
                sample(painter, inner, 4, &steel);
            } else {
                to(painter, rect);
                sample(painter, inner, 4, &grey);
            }
            painter.rect_stroke(inner, 4, Stroke::new(1.0, white(0.5)), StrokeKind::Outside);
        }
        K::Blur => {
            from(&within(painter, part(rect, 0.0, 0.0, 0.5, 1.0)), rect);
            to(&within(painter, part(rect, 0.5, 0.0, 1.0, 1.0)), rect);
            // The seam, soft where the two run into each other.
            let haze = theme::mix(grey(0.4), steel(0.4), 0.5);
            for (reach, alpha) in [(0.2, 0.2), (0.14, 0.3), (0.09, 0.45), (0.04, 0.7)] {
                painter.rect_filled(
                    part(rect, 0.5 - reach, 0.0, 0.5 + reach, 1.0),
                    0,
                    haze.gamma_multiply(alpha),
                );
            }
        }
        K::Glitch => {
            from(painter, rect);
            for (top, bottom, left) in [
                (0.0, 0.22, 0.55),
                (0.3, 0.44, 0.2),
                (0.52, 0.7, 0.7),
                (0.8, 1.0, 0.35),
            ] {
                let band = part(rect, left, top, (left + 0.5_f32).min(1.0), bottom);
                to(&within(painter, band), rect);
            }
        }
    }
}

/// An effect, shown on the sample frame in steel; one of sound, on a plot.
fn effect_swatch(painter: &Painter, rect: Rect, radius: u8, kind: EffectKind) {
    use EffectKind as K;
    let unit = rect.height() * 0.5;
    let sun = at(rect, 0.72, 0.34);
    let frame = || sample(painter, rect, radius, &steel);
    match kind {
        K::Glow => {
            frame();
            // Many faint discs, one over another: a halo that falls away
            // smoothly rather than in rings.
            for step in 0..8 {
                let reach = 0.6 - step as f32 * 0.045;
                painter.circle_filled(sun, unit * reach, white(0.055));
            }
        }
        K::Grain => {
            frame();
            // The same specks in every frame: a fixed sequence, not noise.
            let mut seed: u32 = 0x9E37_79B9;
            for index in 0..64 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let x = (seed >> 8 & 0xFFFF) as f32 / 65_535.0;
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let y = (seed >> 8 & 0xFFFF) as f32 / 65_535.0;
                let alpha = 0.25 + 0.4 * ((seed >> 26) as f32 / 63.0);
                let speck = if index % 2 == 0 {
                    white(alpha)
                } else {
                    black(alpha)
                };
                painter.circle_filled(at(rect, 0.04 + x * 0.92, 0.06 + y * 0.88), 0.9, speck);
            }
        }
        K::Vignette => {
            frame();
            // A falloff, not bands: darkest at the edge and clear three
            // tenths of the way in. Light enough at the edge that the card
            // keeps its outline where its neighbours have theirs.
            // Round the well's own outline, each point drawn in towards
            // the middle for the clear edge.
            let mut outline = Vec::new();
            egui::epaint::tessellator::path::rounded_rectangle(
                &mut outline,
                rect,
                CornerRadius::same(radius).into(),
            );
            let middle = rect.center();
            let mut mesh = egui::Mesh::default();
            for point in &outline {
                mesh.colored_vertex(*point, black(0.5));
                mesh.colored_vertex(middle + (*point - middle) * 0.4, Color32::TRANSPARENT);
            }
            let count = outline.len() as u32;
            for index in 0..count {
                let (o, i) = (index * 2, index * 2 + 1);
                let next = (index + 1) % count;
                let (next_o, next_i) = (next * 2, next * 2 + 1);
                mesh.add_triangle(o, next_o, next_i);
                mesh.add_triangle(o, next_i, i);
            }
            painter.add(Shape::mesh(mesh));
        }
        K::Glitch => {
            frame();
            for (top, bottom, shift) in [(0.18, 0.3, -0.1), (0.46, 0.56, 0.12), (0.7, 0.8, -0.06)] {
                sample(
                    &within(painter, part(rect, 0.0, top, 1.0, bottom)),
                    rect.translate(vec2(rect.width() * shift, 0.0)),
                    radius,
                    &steel,
                );
            }
            painter.rect_filled(part(rect, 0.0, 0.56, 1.0, 0.6), 0, white(0.35));
        }
        K::Shadow => {
            frame();
            let subject = Rect::from_center_size(at(rect, 0.44, 0.5), vec2(unit * 0.9, unit * 0.9));
            let offset = vec2(unit * 0.16, unit * 0.16);
            painter.rect_filled(subject.translate(offset), 4, black(0.55));
            painter.rect_filled(subject, 4, white(0.9));
        }
        K::Blur => {
            frame();
            painter.rect_filled(rect, radius, steel(0.38).gamma_multiply(0.6));
            for (reach, alpha) in [(0.4, 0.1), (0.3, 0.16), (0.2, 0.26)] {
                painter.circle_filled(sun, unit * reach, white(alpha));
            }
        }
        K::MotionBlur => {
            frame();
            for (shift, alpha) in [(-0.21, 0.14), (-0.14, 0.24), (-0.07, 0.4)] {
                painter.circle_filled(
                    sun + vec2(rect.width() * shift, 0.0),
                    rect.height() * 0.12,
                    steel(0.95).gamma_multiply(alpha),
                );
            }
        }
        K::Sharpen => {
            frame();
            let edge = Stroke::new(1.2, white(0.85));
            for points in [
                [(-0.06, 0.76), (0.32, 0.44), (0.74, 0.76)],
                [(0.34, 0.76), (0.7, 0.52), (1.08, 0.76)],
            ] {
                painter.add(Shape::line(
                    points.iter().map(|(x, y)| at(rect, *x, *y)).collect(),
                    edge,
                ));
            }
            painter.circle_stroke(sun, rect.height() * 0.12, edge);
        }
        K::Shake => {
            frame();
            let subject = rect.shrink2(vec2(rect.width() * 0.18, rect.height() * 0.2));
            for (shift, alpha) in [(vec2(-0.1, 0.07), 0.35), (vec2(0.1, -0.06), 0.55)] {
                painter.rect_stroke(
                    subject.translate(shift * unit),
                    4,
                    Stroke::new(1.5, white(alpha)),
                    StrokeKind::Inside,
                );
            }
            painter.rect_stroke(subject, 4, Stroke::new(1.5, white(0.9)), StrokeKind::Inside);
        }
        K::Distort => {
            frame();
            for row in [0.3, 0.5, 0.7] {
                let wave: Vec<Pos2> = (0..=24)
                    .map(|step| {
                        let x = step as f32 / 24.0;
                        let lift = (x * std::f32::consts::TAU * 1.5).sin() * 0.06;
                        at(rect, 0.08 + x * 0.84, row + lift)
                    })
                    .collect();
                painter.add(Shape::line(wave, Stroke::new(1.5, white(0.75))));
            }
        }
        K::ChromaKey | K::DifferenceMatte => {
            let green = Color32::from_rgb(52, 168, 83);
            painter.rect_filled(rect, radius, green);
            // A figure cut out of the screen behind it.
            painter.circle_filled(at(rect, 0.5, 0.36), unit * 0.2, white(0.92));
            painter.rect_filled(
                part(rect, 0.34, 0.56, 0.66, 1.0),
                CornerRadius {
                    nw: 8,
                    ne: 8,
                    sw: 0,
                    se: 0,
                },
                white(0.92),
            );
        }
        K::Grade | K::Lut => {
            // Before and after: the frame as it is, and graded.
            sample(
                &within(painter, part(rect, 0.0, 0.0, 0.5, 1.0)),
                rect,
                radius,
                &steel,
            );
            let (shadows, lights) = (
                Color32::from_rgb(10, 44, 52),
                Color32::from_rgb(250, 196, 132),
            );
            sample(
                &within(painter, part(rect, 0.5, 0.0, 1.0, 1.0)),
                rect,
                radius,
                &|level| theme::mix(shadows, lights, level),
            );
            painter.line_segment(
                [at(rect, 0.5, 0.0), at(rect, 0.5, 1.0)],
                Stroke::new(1.5, white(0.9)),
            );
        }
        K::HslBand => {
            frame();
            let hues = [
                Color32::from_rgb(232, 80, 80),
                Color32::from_rgb(236, 190, 70),
                Color32::from_rgb(92, 190, 110),
                Color32::from_rgb(80, 190, 220),
                Color32::from_rgb(96, 120, 236),
                Color32::from_rgb(206, 96, 220),
            ];
            for (index, color) in hues.into_iter().enumerate() {
                let left = 0.14 + index as f32 * 0.12;
                painter.rect_filled(part(rect, left, 0.8, left + 0.12, 0.9), 0, color);
            }
            // One band of them is the one that is changed.
            painter.rect_stroke(
                part(rect, 0.38, 0.76, 0.5, 0.94),
                2,
                Stroke::new(1.5, white(0.95)),
                StrokeKind::Outside,
            );
        }
        K::CurveMaster | K::CurveRed | K::CurveGreen | K::CurveBlue => {
            frame();
            painter.rect_filled(rect, radius, black(0.45));
            let color = match kind {
                K::CurveRed => Color32::from_rgb(236, 100, 90),
                K::CurveGreen => Color32::from_rgb(96, 196, 120),
                K::CurveBlue => Color32::from_rgb(96, 148, 244),
                _ => white(0.95),
            };
            let graph = part(rect, 0.22, 0.16, 0.78, 0.84);
            painter.line_segment(
                [graph.left_bottom(), graph.right_top()],
                Stroke::new(1.0, white(0.3)),
            );
            let curve: Vec<Pos2> = (0..=24)
                .map(|step| {
                    let x = step as f32 / 24.0;
                    // An S: darker shadows, brighter lights.
                    let y = x * x * (3.0 - 2.0 * x);
                    at(graph, x, 1.0 - (x + (y - x) * 1.6).clamp(0.0, 1.0))
                })
                .collect();
            painter.add(Shape::line(curve, Stroke::new(1.5, color)));
        }
        K::Stabilize => {
            frame();
            let held = rect.shrink2(vec2(rect.width() * 0.2, rect.height() * 0.2));
            painter.rect_stroke(held, 3, Stroke::new(1.5, white(0.9)), StrokeKind::Inside);
        }
        K::HighPass | K::EqBand | K::Compressor | K::Limiter | K::Denoise | K::Pitch => {
            audio_swatch(painter, rect, kind);
        }
        K::CenterCut => {
            let stage = Palette::stage_palette(Default::default());
            // Left, centre and right: the centre is what is taken down.
            for (x, alpha) in [(0.26, 1.0), (0.5, 0.25), (0.74, 1.0)] {
                painter.circle_filled(
                    at(rect, x, 0.5),
                    unit * 0.18,
                    stage.wave().gamma_multiply(alpha),
                );
            }
            painter.circle_stroke(
                at(rect, 0.5, 0.5),
                unit * 0.18,
                Stroke::new(1.0, stage.secondary),
            );
        }
    }
}

/// What an audio kind does to a sound, as a line on a plot: what passes at
/// each frequency for a filter, what comes out for what goes in for a level,
/// a quieter noise under a waveform for noise reduction. In the stage's
/// inks, on the well.
fn audio_swatch(painter: &Painter, rect: Rect, kind: EffectKind) {
    use EffectKind as K;
    let stage = Palette::stage_palette(Default::default());
    let plot = part(rect, 0.16, 0.2, 0.84, 0.8);
    let axis = Stroke::new(1.0, stage.separator);
    let line = Stroke::new(1.5, stage.wave());
    painter.line_segment([plot.left_bottom(), plot.right_bottom()], axis);
    // A curve of 24 steps over the plot, from `y(x)`, both in `0..=1`.
    let curve = |y: &dyn Fn(f32) -> f32| {
        let points: Vec<Pos2> = (0..=24)
            .map(|step| {
                let x = step as f32 / 24.0;
                at(plot, x, 1.0 - y(x).clamp(0.0, 1.0))
            })
            .collect();
        painter.add(Shape::line(points, line));
    };
    match kind {
        K::HighPass => curve(&|x| 0.7 * (x / 0.35).min(1.0).powf(0.6)),
        K::EqBand => curve(&|x| 0.45 + 0.3 * (-((x - 0.55) / 0.12).powi(2)).exp()),
        K::Compressor => curve(&|x| if x < 0.5 { x } else { 0.5 + (x - 0.5) * 0.3 }),
        K::Limiter => curve(&|x| x.min(0.7)),
        K::Denoise => {
            curve(&|x| 0.55 + 0.3 * (x * std::f32::consts::TAU * 2.0).sin());
            // What is left of the noise along the floor.
            for step in 0..9 {
                let x = 0.06 + step as f32 * 0.11;
                painter.circle_filled(at(plot, x, 0.92), 0.9, stage.secondary.gamma_multiply(0.5));
            }
        }
        _ => {}
    }
}
