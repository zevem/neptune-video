//! The library's effects, transitions and looks: what a clip can be given,
//! as tiles.
//!
//! A tile is listed from the project's own tables, drawn with a few shapes
//! (nothing is decoded for it), and applied to the selected clips with a
//! double click or Enter, or to the clip it is dragged onto. Either way the
//! clips are captured when the tile is let go of, and what is made is a
//! command or an [`Action::AddTransitions`] like any other.
use super::{
    Action, Selection, inspector,
    library::Tab,
    transitions,
    widgets::{self},
};
use crate::theme::{self, Palette, metrics};
use editor_core::{
    ClipContent, ClipId, Command, Effect, EffectKind, LookPreset, Project, Time, TrackKind,
    TransitionEdge, TransitionKind,
};
use eframe::egui::{
    self, Color32, CornerRadius, Id, Key, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
    WidgetInfo, WidgetType, pos2, vec2,
};

const TILE_MIN: f32 = 84.0;
const TILE_GAP: f32 = 8.0;
const NAME_HEIGHT: f32 = 22.0;
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
            Self::Effect(_) => "an effect",
            Self::Transition(_) => "a transition",
            Self::Look(_) => "a look",
        }
    }
}

/// Whether the library offers an effect kind: one a clip takes as it is
/// made and the canvas draws. A table or a clean plate is chosen in the
/// Color section, and nothing draws a stabilization yet.
pub fn offered(kind: EffectKind) -> bool {
    !kind.needs_asset() && kind != EffectKind::Stabilize
}

/// What a tab lists, each under its heading: the effects by family, in the
/// families' order.
pub fn entries(tab: Tab) -> Vec<(&'static str, Entry)> {
    match tab {
        Tab::Media => Vec::new(),
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

/// Whether a clip takes effects and looks: a picture on a video track.
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

/// The tiles of one tab in `rect`, filtered by what is searched for.
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
        widgets::empty_state(
            ui,
            p,
            rect,
            crate::icons::Icon::Search,
            "No matches",
            "Nothing here has that name.",
        );
        return;
    }

    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 0.0))));
    body.set_clip_rect(rect);
    body.spacing_mut().item_spacing = Vec2::ZERO;
    // The scroll bar has the last points of the panel to itself.
    let width = body.available_width() - 6.0;
    let columns = ((width + TILE_GAP) / (TILE_MIN + TILE_GAP))
        .floor()
        .max(1.0) as usize;
    let tile_width = (width - TILE_GAP * (columns as f32 - 1.0)) / columns as f32;
    let tile_height = (tile_width * 0.58).round() + NAME_HEIGHT;
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    egui::ScrollArea::vertical()
        .id_salt(("browser", input.tab))
        .auto_shrink([false, false])
        .show(&mut body, |ui| {
            let mut heading = "";
            let mut column = 0;
            let mut row = Rect::NOTHING;
            for (family, entry) in listed.iter().filter(|listed| shown(listed)) {
                if *family != heading {
                    heading = family;
                    column = 0;
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(heading)
                            .font(theme::medium(11.5))
                            .color(p.muted),
                    );
                    ui.add_space(4.0);
                }
                if column == 0 {
                    (row, _) =
                        ui.allocate_exact_size(vec2(width, tile_height + TILE_GAP), Sense::hover());
                }
                let cell = Rect::from_min_size(
                    pos2(
                        row.left() + column as f32 * (tile_width + TILE_GAP),
                        row.top(),
                    ),
                    vec2(tile_width, tile_height),
                );
                column = (column + 1) % columns;
                if !ui.is_rect_visible(cell) {
                    continue;
                }
                tile(ui, cell, *entry, state, selection, input, enter, actions);
            }
        });
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
    let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
    let painter = ui.painter();
    painter.rect_filled(picture, radius, p.sunken);
    swatch(
        &painter.with_clip_rect(picture.intersect(painter.clip_rect())),
        picture.shrink(1.0),
        entry,
        p,
    );
    let outline = if response.has_focus() || picked {
        Stroke::new(1.5, p.accent)
    } else if response.hovered() {
        Stroke::new(1.0, p.secondary)
    } else if p.dark {
        Stroke::new(1.0, Color32::from_white_alpha(26))
    } else {
        Stroke::new(1.0, Color32::from_black_alpha(26))
    };
    painter.rect_stroke(picture, radius, outline, StrokeKind::Inside);
    let label = widgets::truncated(ui, name, theme::medium(11.5), p.fg, rect.width());
    ui.painter()
        .galley(pos2(rect.left() + 1.0, picture.bottom() + 4.0), label, p.fg);

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
    // What is being carried follows the pointer.
    if response.dragged()
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        let layer = egui::LayerId::new(egui::Order::Tooltip, Id::new("browser-carry"));
        let carry = ui.ctx().layer_painter(layer);
        let label = carry.layout_no_wrap(name.to_owned(), theme::medium(12.0), p.fg);
        let chip = Rect::from_min_size(pos + vec2(12.0, 10.0), label.size() + vec2(20.0, 10.0));
        carry.rect_filled(chip, CornerRadius::same(8), p.elevated);
        carry.rect_stroke(
            chip,
            CornerRadius::same(8),
            Stroke::new(1.0, p.border),
            StrokeKind::Inside,
        );
        carry.galley(chip.min + vec2(10.0, 5.0), label, p.fg);
    }
    response.on_hover_text(match entry {
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

/// A small picture of what an entry does. It is drawn with a handful of
/// shapes from the palette, the same in every frame.
fn swatch(painter: &egui::Painter, rect: Rect, entry: Entry, p: &Palette) {
    match entry {
        Entry::Effect(kind) => effect_swatch(painter, rect, kind, p),
        Entry::Transition(kind) => transition_swatch(painter, rect, kind, p),
        Entry::Look(preset) => look_swatch(painter, rect, preset),
    }
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

fn look_swatch(painter: &egui::Painter, rect: Rect, preset: LookPreset) {
    const STEPS: usize = 8;
    let (dark, light) = look_colours(preset);
    let outer = metrics::CONTROL_RADIUS - 1;
    for step in 0..STEPS {
        let from = step as f32 / STEPS as f32;
        let to = (step + 1) as f32 / STEPS as f32;
        let radius = CornerRadius {
            nw: if step == 0 { outer } else { 0 },
            sw: if step == 0 { outer } else { 0 },
            ne: if step + 1 == STEPS { outer } else { 0 },
            se: if step + 1 == STEPS { outer } else { 0 },
        };
        // Half a point over the next, so no seam shows between two.
        let mut stripe = part(rect, from, 0.0, to, 1.0);
        stripe.max.x += 0.5;
        let amount = step as f32 / (STEPS - 1) as f32;
        painter.rect_filled(
            stripe.intersect(rect),
            radius,
            theme::mix(dark, light, amount),
        );
    }
}

fn transition_swatch(painter: &egui::Painter, rect: Rect, kind: TransitionKind, p: &Palette) {
    use TransitionKind as K;
    let from = theme::mix(p.sunken, p.secondary, 0.45);
    let to = p.accent;
    let frame = rect.shrink2(vec2(rect.width() * 0.14, rect.height() * 0.18));
    let radius = CornerRadius::same(3);
    let fill = |rect: Rect, color: Color32| painter.rect_filled(rect, radius, color);
    // An arrowhead at `tip`, pointing along `towards`.
    let arrow = |tip: Pos2, towards: Vec2| {
        let back = tip - towards * 5.0;
        let side = towards.rot90() * 3.5;
        painter.add(Shape::line(
            vec![back + side, tip, back - side],
            Stroke::new(1.5, p.on_accent),
        ));
    };
    match kind {
        K::Dissolve => {
            fill(part(frame, 0.0, 0.0, 0.64, 1.0), from);
            fill(part(frame, 0.36, 0.0, 1.0, 1.0), to.gamma_multiply(0.72));
        }
        K::Fade => {
            fill(part(frame, 0.0, 0.0, 0.36, 1.0), from);
            fill(part(frame, 0.4, 0.0, 0.6, 1.0), from.gamma_multiply(0.3));
            fill(part(frame, 0.64, 0.0, 1.0, 1.0), to);
        }
        K::SlideLeft | K::SlideRight | K::SlideUp | K::SlideDown => {
            let towards = match kind {
                K::SlideLeft => vec2(-1.0, 0.0),
                K::SlideRight => vec2(1.0, 0.0),
                K::SlideUp => vec2(0.0, -1.0),
                _ => vec2(0.0, 1.0),
            };
            // The incoming picture has pushed the outgoing one half way.
            let half = vec2(frame.width(), frame.height()) * towards * 0.5;
            let clipped = painter.with_clip_rect(frame.intersect(painter.clip_rect()));
            clipped.rect_filled(frame.translate(half), CornerRadius::ZERO, from);
            let incoming = frame.translate(half - vec2(frame.width(), frame.height()) * towards);
            clipped.rect_filled(incoming, CornerRadius::ZERO, to);
            arrow(incoming.intersect(frame).center() + towards * 2.5, towards);
        }
        K::WipeLeft | K::WipeRight | K::WipeUp | K::WipeDown => {
            fill(frame, from);
            let (shown, edge) = match kind {
                K::WipeLeft => (part(frame, 0.5, 0.0, 1.0, 1.0), [0.5, 0.0, 0.5, 1.0]),
                K::WipeRight => (part(frame, 0.0, 0.0, 0.5, 1.0), [0.5, 0.0, 0.5, 1.0]),
                K::WipeUp => (part(frame, 0.0, 0.5, 1.0, 1.0), [0.0, 0.5, 1.0, 0.5]),
                _ => (part(frame, 0.0, 0.0, 1.0, 0.5), [0.0, 0.5, 1.0, 0.5]),
            };
            fill(shown, to);
            painter.line_segment(
                [at(frame, edge[0], edge[1]), at(frame, edge[2], edge[3])],
                Stroke::new(1.5, p.fg),
            );
        }
        K::ZoomIn | K::ZoomOut => {
            let (outer, inner) = if kind == K::ZoomIn {
                (from, to)
            } else {
                (to, from)
            };
            fill(frame, outer);
            fill(part(frame, 0.26, 0.24, 0.74, 0.76), inner);
            painter.rect_stroke(
                part(frame, 0.13, 0.12, 0.87, 0.88),
                radius,
                Stroke::new(1.0, inner.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
        }
        K::Blur => {
            fill(frame, from);
            for (inset, alpha) in [(0.0, 0.2), (0.1, 0.3), (0.2, 0.45), (0.3, 0.7)] {
                let soft = part(
                    frame,
                    0.3 + inset * 0.5,
                    inset,
                    1.0 - inset * 0.3,
                    1.0 - inset,
                );
                painter.rect_filled(soft, CornerRadius::same(6), to.gamma_multiply(alpha));
            }
        }
        K::Glitch => {
            fill(frame, from);
            let bands = [
                (0.0, 0.22, 0.55),
                (0.3, 0.44, 0.2),
                (0.52, 0.7, 0.7),
                (0.8, 1.0, 0.35),
            ];
            for (top, bottom, left) in bands {
                painter.rect_filled(
                    part(frame, left, top, (left + 0.5_f32).min(1.08), bottom),
                    CornerRadius::ZERO,
                    to,
                );
            }
        }
    }
}

fn effect_swatch(painter: &egui::Painter, rect: Rect, kind: EffectKind, p: &Palette) {
    use EffectKind as K;
    let ink = p.secondary;
    let soft = theme::mix(p.sunken, p.secondary, 0.4);
    let centre = rect.center();
    let unit = rect.height() * 0.5;
    let thin = Stroke::new(1.5, ink);
    let subject = Rect::from_center_size(centre, vec2(unit * 0.9, unit * 0.9));
    match kind {
        K::Glow => {
            for (reach, alpha) in [(0.78, 0.1), (0.6, 0.18), (0.44, 0.3)] {
                painter.circle_filled(centre, unit * reach, p.accent.gamma_multiply(alpha));
            }
            painter.circle_filled(centre, unit * 0.26, p.fg);
        }
        K::Grain => {
            // The same specks in every frame: a fixed sequence, not noise.
            let mut seed: u32 = 0x9E37_79B9;
            for _ in 0..42 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let x = (seed >> 8 & 0xFFFF) as f32 / 65_535.0;
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let y = (seed >> 8 & 0xFFFF) as f32 / 65_535.0;
                let alpha = 0.35 + 0.6 * ((seed >> 26) as f32 / 63.0);
                painter.circle_filled(
                    at(rect, 0.08 + x * 0.84, 0.1 + y * 0.8),
                    0.9,
                    ink.gamma_multiply(alpha),
                );
            }
        }
        K::Vignette => {
            painter.rect_filled(rect.shrink(3.0), CornerRadius::same(5), soft);
            for (inset, alpha) in [(0.0, 0.55), (0.12, 0.35), (0.24, 0.2)] {
                painter.rect_stroke(
                    rect.shrink(3.0 + unit * inset),
                    CornerRadius::same(8),
                    Stroke::new(unit * 0.16, Color32::BLACK.gamma_multiply(alpha)),
                    StrokeKind::Inside,
                );
            }
        }
        K::Glitch => {
            for (top, bottom, shift, color) in [
                (0.2, 0.36, -0.08, ink),
                (0.36, 0.5, 0.1, p.accent),
                (0.5, 0.66, -0.03, ink),
                (0.66, 0.8, 0.07, ink),
            ] {
                painter.rect_filled(
                    part(rect, 0.26 + shift, top, 0.74 + shift, bottom),
                    CornerRadius::ZERO,
                    color,
                );
            }
        }
        K::Shadow => {
            let offset = vec2(unit * 0.18, unit * 0.18);
            painter.rect_filled(
                subject.translate(offset),
                CornerRadius::same(4),
                Color32::BLACK.gamma_multiply(0.55),
            );
            painter.rect_filled(subject, CornerRadius::same(4), ink);
        }
        K::Blur => {
            for (reach, alpha) in [(0.7, 0.16), (0.55, 0.26), (0.4, 0.42), (0.24, 0.7)] {
                painter.circle_filled(centre, unit * reach, ink.gamma_multiply(alpha));
            }
        }
        K::MotionBlur => {
            for (shift, alpha) in [(-0.9, 0.14), (-0.6, 0.24), (-0.3, 0.4)] {
                painter.rect_filled(
                    subject.translate(vec2(unit * shift, 0.0)),
                    CornerRadius::same(4),
                    ink.gamma_multiply(alpha),
                );
            }
            painter.rect_filled(
                subject.translate(vec2(unit * 0.2, 0.0)),
                CornerRadius::same(4),
                ink,
            );
        }
        K::Sharpen => {
            painter.add(Shape::convex_polygon(
                vec![
                    centre + vec2(0.0, -unit * 0.58),
                    centre + vec2(unit * 0.62, unit * 0.5),
                    centre + vec2(-unit * 0.62, unit * 0.5),
                ],
                soft,
                Stroke::new(1.5, p.fg),
            ));
        }
        K::Shake => {
            for (shift, alpha) in [(vec2(-0.2, 0.12), 0.35), (vec2(0.2, -0.1), 0.6)] {
                painter.rect_stroke(
                    subject.translate(shift * unit),
                    CornerRadius::same(4),
                    Stroke::new(1.5, ink.gamma_multiply(alpha)),
                    StrokeKind::Inside,
                );
            }
            painter.rect_stroke(subject, CornerRadius::same(4), thin, StrokeKind::Inside);
        }
        K::Distort => {
            for row in [0.34, 0.5, 0.66] {
                let wave: Vec<Pos2> = (0..=24)
                    .map(|step| {
                        let x = step as f32 / 24.0;
                        let lift = (x * std::f32::consts::TAU * 1.5).sin() * 0.07;
                        at(rect, 0.14 + x * 0.72, row + lift)
                    })
                    .collect();
                painter.add(Shape::line(wave, thin));
            }
        }
        K::ChromaKey | K::DifferenceMatte => {
            let green = Color32::from_rgb(52, 168, 83);
            painter.rect_filled(rect.shrink(3.0), CornerRadius::same(5), green);
            // A figure cut out of the screen behind it.
            painter.circle_filled(at(rect, 0.5, 0.36), unit * 0.2, p.fg);
            painter.rect_filled(
                part(rect, 0.34, 0.56, 0.66, 1.0).intersect(rect.shrink(3.0)),
                CornerRadius {
                    nw: 8,
                    ne: 8,
                    sw: 0,
                    se: 0,
                },
                p.fg,
            );
        }
        K::Grade | K::Lut => {
            let wheels = [
                (0.3, Color32::from_rgb(232, 96, 84)),
                (0.5, Color32::from_rgb(92, 190, 120)),
                (0.7, Color32::from_rgb(88, 142, 240)),
            ];
            for (x, color) in wheels {
                painter.circle_filled(at(rect, x, 0.5), unit * 0.3, color.gamma_multiply(0.85));
            }
        }
        K::HslBand => {
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
                painter.rect_filled(
                    part(rect, left, 0.36, left + 0.12, 0.64),
                    CornerRadius::ZERO,
                    color,
                );
            }
            // One band of them is the one that is changed.
            painter.rect_stroke(
                part(rect, 0.36, 0.28, 0.52, 0.72),
                CornerRadius::same(3),
                Stroke::new(1.5, p.fg),
                StrokeKind::Outside,
            );
        }
        K::CurveMaster | K::CurveRed | K::CurveGreen | K::CurveBlue => {
            let color = match kind {
                K::CurveRed => Color32::from_rgb(236, 100, 90),
                K::CurveGreen => Color32::from_rgb(96, 196, 120),
                K::CurveBlue => Color32::from_rgb(96, 148, 244),
                _ => p.fg,
            };
            let graph = part(rect, 0.22, 0.16, 0.78, 0.84);
            painter.line_segment(
                [graph.left_bottom(), graph.right_top()],
                Stroke::new(1.0, soft),
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
            painter.rect_stroke(subject, CornerRadius::same(4), thin, StrokeKind::Inside);
        }
    }
}
