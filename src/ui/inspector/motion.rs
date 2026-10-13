//! The Motion section: a video steadied, a region of a video followed
//! through its frames, and clips that go where a tracked region goes.
//!
//! Stabilizing is an effect of two numbers on the clip and a camera path
//! that is not the project's: the analysis worker measures it from the whole
//! video when [`Action::Stabilize`] asks, it is kept in a cache, and the
//! application says here what is known of it. What it does is move, turn and
//! size the whole frame against the camera's shake and crop in to hide the
//! borders: it corrects no rolling shutter and warps nothing. Until the path
//! is known the clip is shown as it is, and a line here says so.
//!
//! Tracking follows one box by matching its picture from frame to frame: it
//! finds where the box moved, not how it turned or grew, and it stops when
//! what the box showed is no longer found. The box is chosen here and on the
//! canvas and is no part of the project; Track hands it to the analysis
//! worker as an [`Action::Track`], and the path comes back as one tracker.
//! Following, renaming and removing are the project's own commands.
use super::{
    card,
    clip::{FIELD, Pane},
    look::set_param,
};
use crate::{
    icons::Icon,
    steady::Status,
    theme,
    ui::{
        Action, ClipDraft, TrackRegion, TrackRequest, viewer,
        widgets::{self, Kind, row, section},
    },
};
use editor_core::{
    AnimTarget, Clip, Command, Effect, EffectKind, Follow, MediaKind, ParamKey, Time, Tracker,
    look, render,
};
use eframe::egui::{self, CornerRadius, Sense, Ui, WidgetInfo, WidgetType, vec2};
use uuid::Uuid;

/// What a tracker is called until it is renamed: the first free "Tracker n".
pub(super) fn fresh_name<'a>(names: impl Iterator<Item = &'a str> + Clone) -> String {
    (1..)
        .map(|index| format!("Tracker {index}"))
        .find(|name| names.clone().all(|taken| taken != name))
        .unwrap_or_else(|| "Tracker".to_owned())
}

/// How long a tracker's path is and how many points it holds.
fn facts(tracker: &Tracker) -> String {
    let (Some(first), Some(last)) = (tracker.keys.first(), tracker.keys.last()) else {
        return String::new();
    };
    let points = tracker.keys.len();
    format!(
        "{:.2} s · {points} {}",
        (last.source - first.source).as_seconds(),
        if points == 1 { "point" } else { "points" }
    )
}

/// The furthest place in its source that `clip` shows from `playhead` to
/// its end: where a path that starts at the playhead and is read forwards
/// through the source has covered all the clip still shows of it.
fn shown_until(clip: &Clip, playhead: Time) -> Time {
    let turns = (clip.retime.iter())
        .flat_map(|retime| &retime.keys)
        .map(|key| clip.start + key.at)
        .filter(|at| *at > playhead);
    turns
        .chain([clip.end()])
        .map(|at| clip.source_time(at))
        .fold(clip.source_time(playhead), Time::max)
}

fn note(ui: &mut Ui, p: &theme::Palette, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::regular(11.5))
                .color(p.muted),
        )
        .wrap(),
    );
}

/// How far something the analysis worker does has got: what it is, a
/// figure, and a bar named `name` for those who do not see it.
fn bar(ui: &mut Ui, p: &theme::Palette, title: &str, name: &str, done: f32) {
    let done = done.clamp(0.0, 1.0);
    let percent = (done * 100.0).round() as i32;
    row(ui, p, title, |ui| {
        ui.label(
            egui::RichText::new(format!("{percent} %"))
                .font(theme::regular(12.5))
                .color(p.fg),
        );
    });
    let (bar, response) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::ProgressIndicator, true, name);
        info.value = Some(f64::from(done));
        info
    });
    let painter = ui.painter();
    painter.rect_filled(bar, CornerRadius::same(2), p.pressed);
    let mut filled = bar;
    filled.set_width(bar.width() * done);
    painter.rect_filled(filled, CornerRadius::same(2), p.accent);
}

impl Pane<'_> {
    pub(super) fn motion(&mut self, ui: &mut Ui) {
        // A region is its clip's: another clip's is given up.
        if (self.state.region).is_some_and(|region| region.clip != self.clip.id) {
            self.state.region = None;
        }
        if self.asset.is_some() {
            self.stabilize(ui);
            self.track_region(ui);
        }
        self.follow(ui);
        self.trackers(ui);
    }

    // ---- Stabilize ---------------------------------------------------------

    fn stabilize(&mut self, ui: &mut Ui) {
        let p = self.p;
        let clip = self.clip;
        let Some(asset) = self.asset else { return };
        section(ui, p, "Stabilize");
        if asset.kind != MediaKind::Video {
            card(ui, p, |ui| {
                note(ui, p, "Only a video has a camera to steady.");
            });
            return;
        }
        // The stack is read where it is and copied only when it changes.
        let held = self.next.effects.take();
        let list: &[Effect] = held.as_deref().unwrap_or(&clip.effects);
        let found = (list.iter()).position(|effect| effect.kind == EffectKind::Stabilize);
        let shown = (self.state.steady.clone())
            .filter(|shown| shown.clip == clip.id && shown.asset == asset.id);
        let suggested = shown.as_ref().and_then(|shown| shown.suggested);
        let mut edited: Option<Vec<Effect>> = None;
        card(ui, p, |ui| {
            let on = found.is_some_and(|index| list[index].enabled);
            let full = found.is_none() && list.len() >= look::MAX_EFFECTS;
            let mut removed = false;
            widgets::fitted_row(ui, p, "Steady the camera", |ui| {
                if found.is_some() {
                    removed = widgets::small_icon_button(ui, p, Icon::Trash, "Remove Stabilize")
                        .clicked();
                }
                ui.add_enabled_ui(!full, |ui| {
                    let switch = widgets::switch(ui, p, on, "Stabilize")
                        .on_hover_text(if on {
                            "Show the clip as the camera took it"
                        } else {
                            "Smooth the camera's path and crop in to hide the borders"
                        })
                        .on_disabled_hover_text("A clip holds sixteen effects at most");
                    if switch.clicked() {
                        let mut stack = list.to_vec();
                        match found {
                            Some(index) => stack[index].enabled = !on,
                            None => {
                                // A clip whose path is known begins with the
                                // crop that hides what it would bring in.
                                let mut effect = Effect::new(EffectKind::Stabilize);
                                if let Some(zoom) = suggested {
                                    set_param(&mut effect, ParamKey::CropZoom, zoom);
                                }
                                stack.push(effect);
                            }
                        }
                        edited = Some(stack);
                    }
                });
            });
            if let (true, Some(index)) = (removed, found) {
                let mut stack = list.to_vec();
                // What was being keyframed on it goes with it.
                let gone = stack.remove(index).id;
                self.next.keys.retain(|key| key.target != Some(gone));
                edited = Some(stack);
            } else if let Some(index) = found {
                let effect = &list[index];
                let target = AnimTarget::Effect(EffectKind::Stabilize);
                let stored_of = |_: &ClipDraft, key: ParamKey| effect.param(key).unwrap_or(0.0);
                let mut change: Option<(ParamKey, f32)> = None;
                let mut crop = 1.0;
                for spec in look::spec(EffectKind::Stabilize) {
                    let stored = effect.param(spec.key).unwrap_or(spec.default);
                    if spec.key == ParamKey::CropZoom {
                        crop = stored;
                    }
                    if let Some(value) =
                        self.param_row(ui, effect.id, target, spec, stored, &stored_of)
                    {
                        change = Some((spec.key, value));
                    }
                }
                if let Some(zoom) = suggested {
                    let fits = (crop - zoom).abs() < 0.005;
                    widgets::fitted_row(ui, p, &format!("Borders hide at {zoom:.2} ×"), |ui| {
                        ui.add_enabled_ui(!fits, |ui| {
                            let fit = widgets::button(ui, p, None, "Fit crop", Kind::Plain)
                                .on_hover_text(
                                    "Set the crop to the least that hides the borders of this clip",
                                )
                                .on_disabled_hover_text(
                                    "The crop is the least that hides the borders",
                                );
                            if fit.clicked()
                                && let Some(value) =
                                    self.write(Some(effect.id), ParamKey::CropZoom, zoom)
                            {
                                change = Some((ParamKey::CropZoom, value));
                            }
                        });
                    });
                    if crop + 0.005 < zoom {
                        note(
                            ui,
                            p,
                            "With less crop some frames are steadied less, so that no border shows.",
                        );
                    }
                }
                if let Some((key, value)) = change {
                    let mut stack = list.to_vec();
                    set_param(&mut stack[index], key, value);
                    edited = Some(stack);
                }
            }
            widgets::hairline(ui, p);
            ui.add_space(2.0);
            self.steady_state(ui, asset.id, shown.as_ref().map(|shown| &shown.status), on);
        });
        self.next.effects = match edited {
            // A stack back at the stored one is no draft.
            Some(stack) => (stack != clip.effects).then_some(stack),
            None => held,
        };
    }

    /// What is known of the video's camera path, in a quiet line, and the
    /// way to measure it or to give the measuring up.
    fn steady_state(&mut self, ui: &mut Ui, asset: uuid::Uuid, status: Option<&Status>, on: bool) {
        let p = self.p;
        if let Some(Status::Analysing(done)) = status {
            bar(ui, p, "Analysing", "Analysing the video", *done);
            ui.add_space(4.0);
            if widgets::button(ui, p, None, "Cancel analysis", Kind::Plain).clicked() {
                self.actions.push(Action::CancelStabilize);
            }
            return;
        }
        let unsteadied = if on {
            " The clip is shown as it is."
        } else {
            ""
        };
        let (line, again) = match status {
            None | Some(Status::Reading) => ("Looking for an analysis…".to_owned(), false),
            Some(Status::NotAnalysed) => (format!("Not analysed.{unsteadied}"), false),
            Some(Status::Stale) => (
                format!("Stale: the media changed since it was analysed.{unsteadied}"),
                false,
            ),
            Some(Status::Failed(why)) => (format!("The analysis failed: {why}"), false),
            Some(Status::Ready { frames, unmeasured }) => {
                let frames_said = if *frames == 1 { "frame" } else { "frames" };
                let unmeasured = match unmeasured {
                    0 => String::new(),
                    count => format!(" · {count} not measured"),
                };
                (format!("Ready · {frames} {frames_said}{unmeasured}"), true)
            }
            Some(Status::Analysing(_)) => (String::new(), false),
        };
        let state = ui.add(
            egui::Label::new(
                egui::RichText::new(&line)
                    .font(theme::regular(11.5))
                    .color(p.muted),
            )
            .wrap(),
        );
        state.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Label, true, format!("Stabilization: {line}"))
        });
        ui.add_space(2.0);
        let (label, kind) = if again {
            ("Analyse again", Kind::Plain)
        } else if on {
            ("Analyse", Kind::Primary)
        } else {
            ("Analyse", Kind::Plain)
        };
        let analyse = widgets::button(ui, p, None, label, kind).on_hover_text(
            "Measure the camera's motion through the whole video, in the background",
        );
        if analyse.clicked() {
            self.actions.push(Action::Stabilize(asset));
        }
        note(
            ui,
            p,
            "Moves, turns and sizes the whole frame against the shake, and crops in. It does not correct a rolling shutter or warp the picture.",
        );
    }

    // ---- Track a region ---------------------------------------------------

    fn track_region(&mut self, ui: &mut Ui) {
        let p = self.p;
        let Some(asset) = self.asset else { return };
        section(ui, p, "Track a region");
        card(ui, p, |ui| {
            if asset.kind != MediaKind::Video {
                note(ui, p, "Only a video has motion to track.");
                return;
            }
            if let Some(tracking) = self.state.tracking.clone() {
                match &tracking.waits {
                    Some(why) => self.ready(ui, &tracking.name, why),
                    None => self.progress(ui, &tracking.name, tracking.done),
                }
                return;
            }
            let Some(region) = self.state.region else {
                let full = self.project.trackers.len() >= look::MAX_TRACKERS;
                ui.add_enabled_ui(!full, |ui| {
                    let begin =
                        widgets::button(ui, p, Some(Icon::Plus), "Track a region", Kind::Plain)
                            .on_hover_text("Choose a box on the picture for a tracker to follow")
                            .on_disabled_hover_text("A project holds 64 trackers at most");
                    if begin.clicked() {
                        self.state.region = Some(TrackRegion::new(self.clip.id, asset.id));
                    }
                });
                note(
                    ui,
                    p,
                    "Follows one box by its picture: where it moves, not how it turns or grows.",
                );
                return;
            };
            self.region_rows(ui, region);
        });
    }

    /// The tracking under way, and the way to give it up.
    fn progress(&mut self, ui: &mut Ui, name: &str, done: f32) {
        let p = self.p;
        bar(ui, p, &format!("Tracking {name}"), "Tracking", done);
        ui.add_space(4.0);
        if widgets::button(ui, p, None, "Cancel tracking", Kind::Plain).clicked() {
            self.actions.push(Action::CancelTracking);
        }
    }

    /// A path that has come and waits: adding it is an edit, and it came
    /// while an edit nobody pressed for would have cost something.
    fn ready(&mut self, ui: &mut Ui, name: &str, why: &str) {
        let p = self.p;
        note(ui, p, &format!("{name} is ready. {why}"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, p, None, "Add tracker", Kind::Primary).clicked() {
                self.actions.push(Action::AddTracked);
            }
            let discard = widgets::button(ui, p, None, "Discard", Kind::Plain)
                .on_hover_text("Give the tracked path up");
            if discard.clicked() {
                self.actions.push(Action::CancelTracking);
            }
        });
    }

    /// The box being chosen: its place and size as numbers, Track and Cancel.
    fn region_rows(&mut self, ui: &mut Ui, region: TrackRegion) {
        let p = self.p;
        let clip = self.clip;
        let mut next = region;
        let field = |ui: &mut Ui, value: &mut f32, least: f32, label: &str| {
            let mut shown = *value * 100.0;
            let response = widgets::named_number(
                ui,
                &mut shown,
                least * 100.0..=100.0,
                0.2,
                " %",
                1,
                FIELD,
                label,
            );
            if response.changed() {
                *value = shown / 100.0;
            }
        };
        row(ui, p, "Centre X", |ui| {
            field(ui, &mut next.center[0], 0.0, "Region centre X")
        });
        row(ui, p, "Centre Y", |ui| {
            field(ui, &mut next.center[1], 0.0, "Region centre Y")
        });
        row(ui, p, "Width", |ui| {
            field(ui, &mut next.size[0], TrackRegion::MIN_SIDE, "Region width")
        });
        row(ui, p, "Height", |ui| {
            field(
                ui,
                &mut next.size[1],
                TrackRegion::MIN_SIDE,
                "Region height",
            )
        });
        let next = next.held();
        if next != region {
            self.state.region = Some(next);
        }
        widgets::hairline(ui, p);
        ui.add_space(2.0);
        // The box is drawn on the frame the canvas shows at the playhead,
        // and the path starts on that very frame: the viewer's own reading
        // of which one that is. Frames are read forwards through the source
        // only, to the furthest the rest of the clip shows; where the clip
        // plays backwards that is what it has already shown, so nothing is
        // tracked there.
        let on_clip = clip.start <= self.playhead && self.playhead < clip.end();
        let backwards = viewer::backwards_until(clip, self.playhead).is_some();
        let from = self.asset.map_or(clip.source_time(self.playhead), |asset| {
            viewer::source_frame(self.project, asset, clip, self.playhead)
        });
        let until = shown_until(clip, self.playhead);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(on_clip && !backwards && from < until, |ui| {
                let track = widgets::button(ui, p, None, "Track", Kind::Primary)
                    .on_hover_text(
                        "Follow the box from the frame at the playhead through what the clip shows after it",
                    )
                    .on_disabled_hover_text(if !on_clip {
                        "The playhead is not on this clip"
                    } else if backwards {
                        "The clip plays backwards here: tracking only runs forwards"
                    } else {
                        "Nothing of the clip's source comes after the playhead"
                    });
                if track.clicked() {
                    let names = self.project.trackers.values().map(|t| t.name.as_str());
                    self.actions.push(Action::Track(TrackRequest {
                        region: next,
                        from,
                        until,
                        name: fresh_name(names),
                    }));
                    self.state.region = None;
                }
            });
            if widgets::button(ui, p, None, "Cancel", Kind::Plain).clicked() {
                self.state.region = None;
            }
        });
        note(
            ui,
            p,
            if on_clip && backwards {
                "The clip plays backwards here, and tracking only runs forwards through the source. Put the playhead where the clip plays forwards."
            } else {
                "Drag on the picture to draw the box around what to follow. Tracking starts at the playhead."
            },
        );
    }

    // ---- Follow -----------------------------------------------------------

    fn follow(&mut self, ui: &mut Ui) {
        let p = self.p;
        let project = self.project;
        section(ui, p, "Follow");
        card(ui, p, |ui| {
            if project.trackers.is_empty() {
                note(
                    ui,
                    p,
                    "No trackers yet. Track a region of a video clip, then a title, a picture or another clip can follow it.",
                );
                return;
            }
            let held = self.next.follow.clone();
            let chosen = held.as_ref().map(|follow| follow.tracker_id);
            let name = |id: Option<Uuid>| {
                id.and_then(|id| project.trackers.get(&id))
                    .map_or("None", |tracker| tracker.name.as_str())
            };
            let mut pick = chosen;
            widgets::fitted_row(ui, p, "Tracker", |ui| {
                egui::ComboBox::from_id_salt("follow-tracker")
                    .selected_text(name(chosen))
                    .width(118.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut pick, None, "None");
                        for tracker in project.trackers.values() {
                            ui.selectable_value(&mut pick, Some(tracker.id), &tracker.name);
                        }
                    })
                    .response
                    .widget_info(|| {
                        WidgetInfo::labeled(WidgetType::ComboBox, true, "Tracker to follow")
                    });
            });
            if pick != chosen {
                self.next.follow = pick.map(|id| self.follow_of(id));
            }
            let Some(follow) = self.next.follow.as_mut() else {
                return;
            };
            for (axis, label, name) in [
                (0, "Offset X", "Follow offset X"),
                (1, "Offset Y", "Follow offset Y"),
            ] {
                row(ui, p, label, |ui| {
                    widgets::named_number(
                        ui,
                        &mut follow.offset[axis],
                        -100_000.0..=100_000.0,
                        1.0,
                        " px",
                        0,
                        FIELD,
                        name,
                    );
                });
            }
            let tracker = follow.tracker_id;
            let seen = (project.trackers.get(&tracker))
                .and_then(|tracker| {
                    render::tracked_clip(project, self.playhead, self.clip.id, tracker)
                })
                .is_some();
            ui.horizontal(|ui| {
                ui.add_enabled_ui(seen, |ui| {
                    let centre = widgets::button(ui, p, None, "Centre on point", Kind::Plain)
                        .on_hover_text("Put the clip's centre on the tracked point")
                        .on_disabled_hover_text(
                            "The tracked clip is not on screen at the playhead",
                        );
                    if centre.clicked() {
                        let position = self.next.transform.position;
                        if let Some(follow) = self.next.follow.as_mut() {
                            follow.offset = [0.0 - position[0], 0.0 - position[1]];
                        }
                    }
                });
                if widgets::button(ui, p, None, "Stop following", Kind::Plain).clicked() {
                    self.next.follow = None;
                }
            });
            if !seen {
                note(
                    ui,
                    p,
                    "The tracked clip is not on screen at the playhead: the clip stays where it is put.",
                );
            }
        });
    }

    /// A clip beginning to follow `tracker`: it stays where it is at the
    /// playhead and moves with the point from there, so the offset is what
    /// takes the point's place back out. With the tracked clip off screen
    /// there is no point to take out, and the offset is nought.
    fn follow_of(&self, tracker: Uuid) -> Follow {
        let project = self.project;
        // Where the tracked clip draws the point, steadied as the canvas
        // last drew it.
        let offset = (project.trackers.get(&tracker))
            .and_then(|tracker| {
                let readings = &self.state.readings;
                render::tracked_place(project, self.playhead, self.clip.id, tracker, readings)
            })
            // Nought is written as nought, not as minus nought.
            .map_or([0.0; 2], |point| point.map(|value| -value.round() + 0.0));
        Follow {
            tracker_id: tracker,
            position: true,
            scale: false,
            rotation: false,
            offset,
        }
    }

    // ---- Trackers ---------------------------------------------------------

    fn trackers(&mut self, ui: &mut Ui) {
        let p = self.p;
        let project = self.project;
        if project.trackers.is_empty() {
            return;
        }
        section(ui, p, "Trackers");
        for tracker in project.trackers.values() {
            ui.push_id(tracker.id, |ui| {
                card(ui, p, |ui| self.tracker_rows(ui, tracker));
            });
        }
    }

    /// One tracker: its name as a field, what it is of, and Remove.
    fn tracker_rows(&mut self, ui: &mut Ui, tracker: &Tracker) {
        let p = self.p;
        let typed = (self.state.rename.as_ref())
            .filter(|(id, _)| *id == tracker.id)
            .map(|(_, name)| name.clone());
        let mut name = typed.clone().unwrap_or_else(|| tracker.name.clone());
        ui.horizontal(|ui| {
            ui.set_min_height(widgets::SMALL_BUTTON);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let remove = widgets::small_icon_button(
                    ui,
                    p,
                    Icon::Trash,
                    &format!("Remove {}", tracker.name),
                );
                if remove.clicked() {
                    self.actions.push(Action::Edit(Command::RemoveTracker {
                        tracker_id: tracker.id,
                    }));
                }
                let response = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(ui.available_width())
                        .char_limit(64)
                        .font(theme::regular(12.5)),
                );
                response.widget_info(|| {
                    WidgetInfo::labeled(
                        WidgetType::TextEdit,
                        true,
                        format!("Name of {}", tracker.name),
                    )
                });
                if response.changed() {
                    self.state.rename = Some((tracker.id, name.clone()));
                }
                // Leaving the field, by Return or by going elsewhere, is the
                // rename. An Escape gave up what was typed.
                if response.lost_focus() && typed.is_some() {
                    self.state.rename = None;
                    let name = name.trim();
                    if !self.state.cancelled && !name.is_empty() && name != tracker.name {
                        self.actions.push(Action::Edit(Command::SetTracker {
                            tracker: Tracker {
                                name: name.to_owned(),
                                ..tracker.clone()
                            },
                        }));
                    }
                }
            });
        });
        let of = (self.project.assets.get(&tracker.asset_id))
            .map_or("Missing media", |asset| asset.name.as_str());
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{of} · {}", facts(tracker)))
                    .font(theme::regular(11.5))
                    .color(p.muted),
            )
            .truncate(),
        );
    }
}
