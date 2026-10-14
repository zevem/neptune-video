//! The Speed section: how fast a clip plays, which way, where it holds a
//! frame, and the curve of its speed.
//!
//! Everything here is read from the clip's time map as a [`Curve`] and
//! written back as one. A point of the graph that is dragged, and a field
//! that is dragged or typed in, hold their map in the draft and are one undo
//! step when let go; a button, a menu and a key are a command at once. A map
//! the project would refuse is not held: the last one that it takes stays,
//! and a line says why.
use super::{
    card,
    clip::{FIELD, Pane},
    fact,
};
use crate::{
    icons::Icon,
    speed::{self, Curve, Easing, Ramp},
    theme::{self, Palette, metrics},
    timefmt,
    ui::{
        Action, shortcut,
        widgets::{self, Kind, row, section},
    },
};
use editor_core::{
    MediaKind, PreparedEdit, Project, Time, TrackKind, retime::MAX_TIME_KEYS, sound,
};
use eframe::egui::{
    self, CornerRadius, CursorIcon, Key, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use std::sync::Arc;
use uuid::Uuid;

/// What a picture clip with sound says under its speed when some of it is
/// silent: everywhere else its speed changes its pitch.
const SILENT: &str = "Sound is silent where the clip is held, reversed or ramped; elsewhere speed changes its pitch.";

/// The side of a point's target, and the radius it is drawn at.
const TARGET: f32 = 20.0;
const RADIUS: f32 = 4.5;
/// How far the pointer travels before a drag takes a direction.
const TRAVEL: f32 = 3.0;
/// The line of a stretch is drawn through this many places.
const STEPS: i64 = 10;

/// The way a held point is being moved: a point changes speed or place, not
/// both at once, so that neither changes by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Axis {
    Speed,
    Place,
}

/// What the graph keeps between frames.
#[derive(Debug, Default)]
pub(super) struct GraphState {
    /// The point last taken hold of.
    pub point: usize,
    axis: Option<Axis>,
    /// The speed at the top of the plot, kept as it was when a point was
    /// taken hold of: the speeds do not move under the pointer that holds a
    /// point. The clip always spans the plot, so its end stays at the edge.
    top: Option<f64>,
}

/// The speeds the top of the plot can stand for.
const TOPS: [f64; 4] = [2.0, 4.0, 8.0, speed::FASTEST];

/// The speed at the top of a plot of `curve`: the least of [`TOPS`] that
/// leaves its fastest point a quarter of headroom, so a point can always be
/// lifted into the next.
fn top_of(curve: &Curve) -> f64 {
    let fastest = (0..curve.points())
        .map(|point| curve.speed(point))
        .fold(0.0, f64::max);
    (TOPS.into_iter())
        .find(|top| *top >= fastest * 1.25)
        .unwrap_or(speed::FASTEST)
}

/// What a frame of the graph came to.
struct Graphed {
    /// The curve as changed, when it was.
    curve: Option<Curve>,
    /// The pointer holds what changed it: it is not yet an edit.
    held: bool,
    /// One of the points has the keyboard: the arrows and Delete are its.
    focused: bool,
}

/// Where a speed is drawn, from the bottom of the plot to its `top`: a
/// straight scale, so that the pointer's travel is the change in speed
/// wherever on the plot it is, and a stop is on the floor.
fn height(speed: f64, top: f64) -> f32 {
    (speed.clamp(0.0, top) / top) as f32
}

/// The speed drawn at `height` of a plot whose top is `top`, to the
/// hundredth; one near normal speed is normal speed.
fn speed_at(height: f32, top: f64) -> f64 {
    let speed = (f64::from(height.clamp(0.0, 1.0)) * top * 100.0).round() / 100.0;
    if (speed - 1.0).abs() < 0.04 {
        1.0
    } else {
        speed.clamp(0.0, top)
    }
}

/// A point's speed and place, as they are read out.
fn told(curve: &Curve, point: usize) -> String {
    format!(
        "{} at {:.2} s",
        speed::times(curve.speed(point)),
        curve.at(point).as_seconds()
    )
}

/// Draws the speed of `curve` over the clip and takes what is done to its
/// points. `playhead` is where the playhead is in the clip, when the graph
/// shows the clip as it is stored.
fn graph(
    ui: &mut Ui,
    p: &Palette,
    curve: &Curve,
    state: &mut GraphState,
    playhead: Option<Time>,
) -> Graphed {
    let width = ui.available_width();
    let (rect, plot_response) = ui.allocate_exact_size(vec2(width, 112.0), Sense::click());
    let enabled = ui.is_enabled();
    plot_response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, enabled, "Speed curve"));
    // Room at the leading edge for the names of the speeds.
    let plot = Rect::from_min_max(
        rect.min + vec2(RADIUS + 22.0, RADIUS + 5.0),
        rect.max - vec2(RADIUS + 5.0, RADIUS + 5.0),
    );
    let count = curve.points();
    let point_id = |index: usize| ui.id().with(("speed-point", index));
    // While a point is held the plot keeps the speeds it was taken hold of
    // on.
    let holding = (0..count).any(|index| ui.ctx().is_being_dragged(point_id(index)));
    let duration = curve.duration().0.max(1) as f32;
    let top = (state.top.filter(|_| holding)).unwrap_or_else(|| top_of(curve));
    state.top = Some(top);
    let to_screen = |at: Time, speed: f64| {
        pos2(
            plot.left() + at.0 as f32 / duration * plot.width(),
            plot.bottom() - height(speed, top) * plot.height(),
        )
    };
    let time_at = |pos: Pos2| {
        let part = ((pos.x - plot.left()) / plot.width().max(1.0)).clamp(0.0, 1.0);
        Time((part * duration).round() as i64)
    };
    state.point = state.point.min(count - 1);
    let mut next: Option<Curve> = None;
    let (mut held, mut focused) = (false, false);

    // A double click on the plot puts a point on the curve there.
    if plot_response.double_clicked()
        && let Some(pos) = plot_response.interact_pointer_pos()
    {
        let mut with = curve.clone();
        if let Some(index) = with.insert(time_at(pos)) {
            state.point = index;
            ui.memory_mut(|memory| memory.request_focus(point_id(index)));
            next = Some(with);
        }
    }

    let mut responses = Vec::with_capacity(count);
    for index in 0..count {
        let centre = to_screen(curve.at(index), curve.speed(index));
        let target = Rect::from_center_size(centre, Vec2::splat(TARGET));
        let response = ui.interact(target, point_id(index), Sense::click_and_drag());
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(
                WidgetType::Other,
                enabled,
                format!("Speed point {} of {count}", index + 1),
            );
            info.current_text_value = Some(told(curve, index));
            info
        });
        if response.drag_started() || response.clicked() {
            state.point = index;
            state.axis = None;
            response.request_focus();
        }
        if response.double_clicked() {
            let mut without = curve.clone();
            if without.remove(index) {
                next = Some(without);
            }
        } else if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
        {
            // The ends of the clip stay where they are: they change speed.
            if index == 0 || index + 1 == count {
                state.axis = Some(Axis::Speed);
            }
            if state.axis.is_none()
                && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            {
                let travelled = pos - origin;
                if travelled.length() >= TRAVEL {
                    state.axis = Some(if travelled.y.abs() >= travelled.x.abs() {
                        Axis::Speed
                    } else {
                        Axis::Place
                    });
                }
            }
            let mut moved = curve.clone();
            match state.axis {
                Some(Axis::Speed) => moved.set_speed(
                    index,
                    speed_at((plot.bottom() - pos.y) / plot.height().max(1.0), top),
                ),
                Some(Axis::Place) => moved.slide(index, time_at(pos)),
                None => {}
            }
            if moved != *curve {
                next = Some(moved);
                held = true;
            }
        }
        if response.has_focus() {
            focused = true;
            state.point = index;
            // The arrows move the point and do not leave it.
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
            // A key is read with the modifiers of its own event: a whole
            // Shift+→ can arrive in one frame, and by its end Shift is up.
            let (faster, later, remove) = ui.input(|i| {
                let (mut faster, mut later, mut remove) = (0.0, 0, false);
                for event in &i.events {
                    let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                    else {
                        continue;
                    };
                    let (speed, time) = if modifiers.shift {
                        (0.5, 1_000_000)
                    } else {
                        (0.05, 100_000)
                    };
                    match key {
                        Key::ArrowUp => faster += speed,
                        Key::ArrowDown => faster -= speed,
                        Key::ArrowRight => later += time,
                        Key::ArrowLeft => later -= time,
                        Key::Delete | Key::Backspace => remove = true,
                        _ => {}
                    }
                }
                (faster, later, remove)
            });
            let mut keyed = curve.clone();
            if remove {
                keyed.remove(index);
            } else {
                if faster != 0.0 {
                    let speed = ((curve.speed(index) + faster) * 100.0).round() / 100.0;
                    keyed.set_speed(index, speed.max(0.0));
                }
                if later != 0 {
                    keyed.slide(index, Time(curve.at(index).0 + later));
                }
            }
            if keyed != *curve {
                next = Some(keyed);
            }
        }
        responses.push(response);
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
        painter.rect_filled(rect, radius, p.sunken);
        let faint = p.hairline();
        painter.rect_stroke(rect, radius, faint, StrokeKind::Inside);
        // The speeds the plot is read against: four even lines to its top,
        // each named, and normal speed, which is the stronger line.
        let level = |speed: f64| plot.bottom() - height(speed, top) * plot.height();
        let name = |speed: f64, ink| {
            painter.text(
                pos2(rect.left() + 4.0, level(speed)),
                egui::Align2::LEFT_CENTER,
                speed::times(speed),
                theme::regular(9.5),
                ink,
            );
        };
        let lines = plot.left() - RADIUS - 1.0..=rect.right();
        for step in 1..=4 {
            let speed = top * f64::from(step) / 4.0;
            painter.hline(
                lines.clone(),
                level(speed),
                Stroke::new(1.0, faint.color.gamma_multiply(0.5)),
            );
            if speed != 1.0 {
                name(speed, p.muted);
            }
        }
        painter.hline(lines, level(1.0), faint);
        name(1.0, p.secondary);
        if let Some(playhead) = playhead {
            let x = to_screen(playhead, 0.0).x;
            painter.vline(x, rect.y_range(), Stroke::new(1.0, theme::tint(p.fg, 0.28)));
        }

        let ink = if enabled { p.accent } else { p.muted };
        let shown = next.as_ref().unwrap_or(curve);
        let (map, _) = shown.retime();
        let place = |at: i64, speed: f64| {
            pos2(
                plot.left() + at as f32 / shown.duration().0.max(1) as f32 * plot.width(),
                plot.bottom() - height(speed, top) * plot.height(),
            )
        };
        let mut line: Vec<Pos2> = Vec::with_capacity(shown.pieces.len() * (STEPS as usize + 1));
        let mut begins = 0;
        for piece in &shown.pieces {
            let span = piece.span.0;
            line.push(place(begins, piece.from));
            if !piece.frozen() && !piece.flat() {
                for step in 1..STEPS {
                    let at = begins + span * step / STEPS;
                    line.push(place(at, map.speed_at(Time(at)).abs()));
                }
            }
            line.push(place(begins + span, piece.to));
            begins += span;
        }
        painter.add(Shape::line(line, Stroke::new(1.5, ink)));
        // Drawn where they are held now; the targets follow a frame later.
        for index in 0..shown.points() {
            let centre = place(shown.at(index).0, shown.speed(index));
            let response = responses.get(index).filter(|_| shown.points() == count);
            let grabbed = response.is_some_and(|r| r.dragged() || r.is_pointer_button_down_on());
            let size = if grabbed { RADIUS + 1.0 } else { RADIUS };
            let chosen = index == state.point;
            painter.circle_filled(centre, size, if chosen { ink } else { p.sunken });
            painter.circle_stroke(centre, size, Stroke::new(1.5, ink));
            if response.is_some_and(Response::has_focus) {
                painter.circle_stroke(centre, RADIUS + 3.5, Stroke::new(1.5, p.accent));
            }
        }
    }
    for (index, response) in responses.into_iter().enumerate() {
        let cursor = match (response.dragged(), state.axis) {
            (true, Some(Axis::Place)) => CursorIcon::ResizeHorizontal,
            (true, _) => CursorIcon::ResizeVertical,
            _ => CursorIcon::Grab,
        };
        response
            .on_hover_cursor(cursor)
            .on_hover_text(told(curve, index));
    }
    Graphed {
        curve: next,
        held,
        focused,
    }
}

/// A ramp's button: the shape of its speed.
fn ramp_button(ui: &mut Ui, p: &Palette, ramp: Ramp) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(34.0, 26.0), Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, ramp.label()));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = CornerRadius::same(6);
        if response.is_pointer_button_down_on() && enabled {
            painter.rect_filled(rect, radius, p.pressed);
        } else if response.hovered() && enabled {
            painter.rect_filled(rect, radius, p.hover);
        } else {
            painter.rect_filled(rect, radius, p.control);
        }
        if response.has_focus() {
            painter.rect_stroke(rect, radius, Stroke::new(1.5, p.accent), StrokeKind::Inside);
        }
        let ink = if enabled {
            p.secondary
        } else {
            p.muted.gamma_multiply(0.5)
        };
        let glyph = Rect::from_center_size(rect.center(), vec2(22.0, 13.0));
        // Among a ramp's own speeds, the slowest on the floor.
        let points = ramp.points();
        let (low, high) = points
            .iter()
            .fold((f32::MAX, f32::MIN), |(low, high), point| {
                (low.min(point.1 as f32), high.max(point.1 as f32))
            });
        let line: Vec<Pos2> = (points.iter())
            .map(|(along, speed)| {
                let up = (*speed as f32 - low) / (high - low).max(0.01);
                pos2(
                    glyph.left() + *along as f32 * glyph.width(),
                    glyph.bottom() - up * glyph.height(),
                )
            })
            .collect();
        painter.add(Shape::line(line, Stroke::new(1.5, ink)));
    }
    response.on_hover_text(ramp.label())
}

impl Pane<'_> {
    /// Takes a curve a control made. One the pointer or a field holds goes
    /// into the draft; any other is an edit at once. A curve the project
    /// would refuse is neither, and is said to be refused.
    fn take_curve(&mut self, curve: &Curve, held: bool, label: &str) {
        let Some((retime, duration)) = curve.valid() else {
            self.state.speed_refused = Some(self.clip.id);
            return;
        };
        self.state.speed_refused = None;
        if held {
            self.next.retime = Some((retime, duration));
            return;
        }
        let settled = self.settled();
        let project = settled.as_deref().unwrap_or(self.project);
        if let Some(command) = speed::curve_command(project, self.clip.id, curve, label) {
            self.actions.push(Action::Edit(command));
        }
    }

    /// The project as it is once what the draft holds is committed, when the
    /// draft holds a change. An edit a button makes is dispatched after the
    /// draft is, so it is made of the clip the draft leaves and not of the
    /// stored one: a speed typed a moment ago is not played over twice.
    fn settled(&self) -> Option<Arc<Project>> {
        let command = self.next.command(self.project)?;
        let before = Arc::new(self.project.clone());
        PreparedEdit::from_snapshot(before, Uuid::nil(), 0, command)
            .ok()
            .map(|edit| edit.after())
    }

    /// An edit a button makes of the clip's curve, at once: `change` is
    /// handed the curve the clip has when the edit is dispatched.
    fn change_curve(&mut self, label: &str, change: impl FnOnce(&mut Curve) -> bool) {
        let settled = self.settled();
        let project = settled.as_deref().unwrap_or(self.project);
        let Some((_, clip)) = project.clip(self.clip.id) else {
            return;
        };
        let mut curve = Curve::of(clip);
        if !change(&mut curve) {
            return;
        }
        if curve.valid().is_none() {
            self.state.speed_refused = Some(clip.id);
            return;
        }
        self.state.speed_refused = None;
        if let Some(command) = speed::curve_command(project, clip.id, &curve, label) {
            self.actions.push(Action::Edit(command));
        }
    }

    pub(super) fn speed(&mut self, ui: &mut Ui) {
        let p = self.p;
        if !(self.asset).is_some_and(|asset| asset.kind != MediaKind::Image) {
            self.placeholder(ui, "A still has no speed of its own.");
            return;
        }
        let clip = self.clip;
        let rate = self.project.settings.frame_rate;
        // The curve a held point or field has made, or the stored one.
        let curve = match &self.next.retime {
            Some((retime, _)) => Curve::of_map(retime),
            None => Curve::of(clip),
        };
        let mapped = clip.retime.is_some() || self.next.retime.is_some();

        section(ui, p, "Speed");
        card(ui, p, |ui| {
            self.speed_row(ui);
            // The speeds offered by name; none is chosen at any other.
            let constant = if mapped {
                curve.steady().map(|speed| speed * self.next.rate)
            } else {
                Some(self.next.rate)
            };
            const NONE: usize = usize::MAX;
            let mut chosen = constant
                .and_then(|speed| {
                    speed::RATES
                        .iter()
                        .position(|rate| (rate - speed).abs() < 0.004)
                })
                .unwrap_or(NONE);
            let width = ui.available_width();
            let options = [(0, "0.25×"), (1, "0.5×"), (2, "1×"), (3, "2×"), (4, "4×")];
            if widgets::segmented(ui, p, &mut chosen, &options, width)
                && let Some(named) = speed::RATES.get(chosen)
            {
                if mapped {
                    self.next.retime = None;
                    let settled = self.settled();
                    let project = settled.as_deref().unwrap_or(self.project);
                    if let Some(command) = speed::constant_command(project, clip.id, *named) {
                        self.actions.push(Action::Edit(command));
                    }
                } else {
                    self.next.rate = *named;
                }
            }
            ui.add_space(2.0);
            let duration = match &self.next.retime {
                Some((_, duration)) => *duration,
                None => clip.duration.scaled(clip.playback_rate / self.next.rate),
            };
            fact(ui, p, "Duration", timefmt::timecode(duration, rate));
            row(ui, p, "Reverse", |ui| {
                let on = curve.reversed();
                if widgets::switch(ui, p, on, "Reverse").clicked() {
                    self.change_curve("Reverse clip", |curve| {
                        curve.reverse();
                        true
                    });
                }
            });
            // A picture clip's sound is not where its pitch is kept.
            if self.asset.is_some_and(|asset| asset.has_audio) && !sound::owned(self.project, clip)
            {
                self.pitch_row(ui);
            }
        });

        section(ui, p, "Freeze frame");
        card(ui, p, |ui| {
            row(ui, p, "Hold for", |ui| {
                widgets::named_number(
                    ui,
                    &mut self.state.hold,
                    0.1..=60.0,
                    0.02,
                    " s",
                    2,
                    FIELD,
                    "Freeze length",
                );
            });
            let on_clip = clip.start <= self.playhead && self.playhead <= clip.end();
            ui.add_enabled_ui(on_clip, |ui| {
                let hint = format!("Hold the frame at the playhead ({})", shortcut("F"));
                let freeze = widgets::button(ui, p, None, "Freeze at playhead", Kind::Plain)
                    .on_hover_text(hint)
                    .on_disabled_hover_text("The playhead is not on this clip");
                if freeze.clicked() {
                    let hold = Time::from_seconds(f64::from(self.state.hold));
                    let settled = self.settled();
                    let project = settled.as_deref().unwrap_or(self.project);
                    match speed::freeze_command(project, clip.id, self.playhead, hold) {
                        Ok(command) => self.actions.push(Action::Edit(command)),
                        Err(_) => self.state.speed_refused = Some(clip.id),
                    }
                }
            });
        });

        section(ui, p, "Ramp");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for ramp in Ramp::ALL {
                if ramp_button(ui, p, ramp).clicked() {
                    self.change_curve(ramp.label(), |curve| {
                        *curve = ramp.over(curve);
                        true
                    });
                }
            }
        });
        let on_clip = (clip.start <= self.playhead && self.playhead <= clip.end())
            .then_some(self.local)
            .filter(|_| self.next.retime.is_none() && self.next.rate == clip.playback_rate);
        let edit = graph(ui, p, &curve, &mut self.state.graph, on_clip);
        if edit.focused {
            self.state.keys = Some(ui.ctx().cumulative_frame_nr());
        }
        if let Some(changed) = &edit.curve {
            self.take_curve(changed, edit.held, "Change speed curve");
        }

        let point = self.state.graph.point.min(curve.points() - 1);
        let count = curve.points();
        card(ui, p, |ui| {
            let label = format!("Point {} of {count}", point + 1);
            row(ui, p, &label, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_enabled_ui(count < MAX_TIME_KEYS, |ui| {
                    let add = widgets::small_icon_button(ui, p, Icon::Plus, "Add point")
                        .on_hover_text("Add a point at the playhead, or in the longest stretch");
                    if add.clicked() {
                        let local = self.local;
                        let mut added = None;
                        self.change_curve("Add speed point", |with| {
                            added = (with.insert(local)).or_else(|| {
                                let widest = (0..with.pieces.len())
                                    .filter(|index| !with.pieces[*index].frozen())
                                    .max_by_key(|index| with.pieces[*index].span)?;
                                let middle =
                                    Time(with.at(widest).0 + with.pieces[widest].span.0 / 2);
                                with.insert(middle)
                            });
                            added.is_some()
                        });
                        if let Some(index) = added {
                            self.state.graph.point = index;
                        }
                    }
                });
                ui.add_enabled_ui(point > 0 && point + 1 < count, |ui| {
                    let remove = widgets::small_icon_button(ui, p, Icon::Trash, "Remove point")
                        .on_disabled_hover_text("The ends of the clip stay");
                    if remove.clicked() {
                        self.change_curve("Remove speed point", |curve| curve.remove(point));
                    }
                });
            });
            row(ui, p, "Speed here", |ui| {
                let mut shown = curve.speed(point) as f32;
                let field = widgets::named_number(
                    ui,
                    &mut shown,
                    0.0..=speed::FASTEST as f32,
                    0.01,
                    " ×",
                    2,
                    FIELD,
                    "Point speed",
                );
                if field.changed() {
                    let mut with = curve.clone();
                    with.set_speed(point, f64::from(shown));
                    self.take_curve(&with, true, "Change speed curve");
                }
            });
            // The easing of the stretch that leaves the point.
            let piece = curve.piece_of(point);
            let easing = piece.and_then(|index| curve.pieces[index].easing());
            let held = piece.is_some_and(|index| curve.pieces[index].frozen());
            let ramps = piece.is_some_and(|index| !held && !curve.pieces[index].flat());
            widgets::fitted_row(ui, p, "Easing", |ui| {
                let name = match easing {
                    Some(easing) => easing.label(),
                    None if held => "Held frame",
                    None => "Custom",
                };
                let mut picked = None;
                ui.add_enabled_ui(piece.is_some() && !held, |ui| {
                    egui::ComboBox::from_id_salt("speed-easing")
                        .icon(widgets::combo_icon)
                        .selected_text(name)
                        .width(104.0)
                        .show_ui(ui, |ui| {
                            for option in Easing::ALL {
                                // A stretch at one speed has nothing to ease.
                                let offered = option == Easing::Constant || ramps;
                                let chosen = easing == Some(option);
                                let item = ui.add_enabled(
                                    offered,
                                    egui::Button::selectable(chosen, option.label()),
                                );
                                if item.clicked() {
                                    picked = Some(option);
                                }
                            }
                        })
                        .response
                        .widget_info(|| {
                            WidgetInfo::labeled(WidgetType::ComboBox, true, "Segment easing")
                        });
                });
                if let (Some(option), Some(index)) = (picked, piece)
                    && easing != Some(option)
                {
                    self.change_curve("Change speed easing", |curve| {
                        // The stretch is one of the curve the rows showed.
                        if index >= curve.pieces.len() {
                            return false;
                        }
                        curve.set_easing(index, option);
                        true
                    });
                }
            });
        });

        if self.state.speed_refused == Some(clip.id) {
            self.note(
                ui,
                "Not applied: part of the clip would play slower than 0.05× or faster than 16×.",
            );
        }
        if self.asset.is_some_and(|asset| asset.has_audio) && curve.silent() {
            self.note(ui, SILENT);
        }
    }

    /// The "Keep pitch" switch of a clip on an audio track, under its speed:
    /// on is a `Pitch` in its stack, one undo step each way. A `Pitch` that
    /// shifts the pitch holds the switch on. A line says when a speed the
    /// clip plays at is past what keeps pitch. A picture clip has a line in
    /// its place: its sound has to be on an audio track first.
    pub(super) fn pitch_row(&mut self, ui: &mut Ui) {
        let p = self.p;
        let clip = self.clip;
        let on_audio =
            (self.project.clip(clip.id)).is_some_and(|(track, _)| track.kind == TrackKind::Audio);
        if !on_audio {
            ui.label(
                egui::RichText::new(speed::DETACH_TO_KEEP_PITCH)
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
            return;
        }
        let semitones = sound::pitch(clip);
        let shifted = semitones.filter(|semitones| *semitones != 0.0);
        row(ui, p, "Keep pitch", |ui| {
            ui.add_enabled_ui(shifted.is_none(), |ui| {
                let switch = widgets::switch(ui, p, semitones.is_some(), "Keep pitch")
                    .on_hover_text("Speed changes how fast the sound plays, not its pitch")
                    .on_disabled_hover_text(format!(
                        "Pitch is shifted {:+} semitones; speed leaves it there",
                        shifted.unwrap_or_default()
                    ));
                if switch.clicked() {
                    let settled = self.settled();
                    let project = settled.as_deref().unwrap_or(self.project);
                    self.actions.push(
                        match speed::keep_pitch_command(project, clip.id, semitones.is_none()) {
                            Ok(command) => Action::Edit(command),
                            Err(reason) => Action::Notify(reason),
                        },
                    );
                }
            });
        });
        let Some(semitones) = semitones else {
            return;
        };
        // The speeds the clip is heard at, the one being typed or dragged
        // included.
        let rate = self.next.rate;
        let speeds: Vec<f64> = match (&self.next.retime, &clip.retime) {
            (Some((map, _)), _) | (None, Some(map)) => (Curve::of_map(map).heard_speeds())
                .map(|speed| speed * rate)
                .collect(),
            (None, None) => vec![rate],
        };
        if speeds
            .iter()
            .any(|speed| !sound::pitch_kept(*speed, semitones))
        {
            let (least, most) = speed::pitch_kept_speeds(semitones);
            self.note(
                ui,
                &format!(
                    "Pitch is kept from {} to {}.",
                    speed::times(least),
                    speed::times(most)
                ),
            );
        }
    }

    /// One quiet line under a section's rows.
    fn note(&mut self, ui: &mut Ui, text: &str) {
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .font(theme::regular(11.5))
                    .color(self.p.muted),
            )
            .wrap(),
        );
    }
}
