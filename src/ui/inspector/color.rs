//! The Color section: the grade, the hue bands, the curves and the LUT of a
//! clip, and the two ways its background is removed.
//!
//! All of these are ordinary effects in the clip's stack, which the Look
//! section lists whole. What this section adds is a fixed place for each:
//! it edits the first effect of a kind the stack has, and makes one when a
//! value is first touched where the stack has none. A grade or a curve it
//! edited back to where it changes nothing is taken out again. An effect it
//! makes goes after the last key or colour effect that comes no later in the
//! order key, grade, hue bands, curves, LUT, and so ahead of whatever
//! stylizes, blurs or distorts the graded picture; before everything when
//! there is none.
//!
//! Every change is the whole new stack, held in the draft and committed as
//! one `SetEffects`, as in the Look section. The rows are made from
//! `look::spec`.
use super::{
    card,
    clip::Pane,
    curve,
    look::{KEY_COLOUR, is_default, set_param},
};
use crate::{
    icons::Icon,
    rules, theme,
    theme::Palette,
    ui::{
        Action, ClipDraft, ColourPick, icon_of_media,
        widgets::{self, Kind},
        with_key,
    },
};
use editor_core::{
    AnimTarget, Clip, ClipId, Command, Effect, EffectKind, MediaKind, ParamKey, Project, Time,
    Track, TrackKind, look,
};
use eframe::egui::{
    self, Color32, Response, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType, ecolor::Hsva,
};
use std::borrow::Cow;
use uuid::Uuid;

/// The channels a curve is drawn for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Channel {
    #[default]
    Master,
    Red,
    Green,
    Blue,
}

impl Channel {
    const ALL: [(Self, &'static str); 4] = [
        (Self::Master, "Master"),
        (Self::Red, "Red"),
        (Self::Green, "Green"),
        (Self::Blue, "Blue"),
    ];

    pub fn kind(self) -> EffectKind {
        match self {
            Self::Master => EffectKind::CurveMaster,
            Self::Red => EffectKind::CurveRed,
            Self::Green => EffectKind::CurveGreen,
            Self::Blue => EffectKind::CurveBlue,
        }
    }

    fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(channel, _)| *channel == self)
            .map_or("Master", |(_, name)| name)
    }

    fn ink(self, p: &Palette) -> Color32 {
        match self {
            Self::Master => p.fg,
            Self::Red => p.red,
            Self::Green => p.green,
            Self::Blue => Color32::from_rgb(86, 148, 255),
        }
    }
}

/// The hues a band is started on, by the name a band near one is given.
const HUES: [(&str, f32); 6] = [
    ("Reds", 0.0),
    ("Yellows", 60.0),
    ("Greens", 120.0),
    ("Cyans", 180.0),
    ("Blues", 240.0),
    ("Magentas", 300.0),
];

/// The name of the hues about `hue`, in degrees.
fn hue_name(hue: f32) -> &'static str {
    let away = |centre: f32| ((hue - centre + 180.0).rem_euclid(360.0) - 180.0).abs();
    (HUES.iter())
        .min_by(|a, b| away(a.1).total_cmp(&away(b.1)))
        .map_or("Reds", |(name, _)| name)
}

/// How a background is removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Removal {
    Off,
    Key,
    Plate,
}

/// Where in the order of a stack a kind this section makes belongs. Nothing
/// for a kind it does not make.
fn rank(kind: EffectKind) -> Option<u8> {
    match kind {
        EffectKind::ChromaKey | EffectKind::DifferenceMatte => Some(0),
        EffectKind::Grade => Some(1),
        EffectKind::HslBand => Some(2),
        kind if kind.is_curve() => Some(3),
        EffectKind::Lut => Some(4),
        _ => None,
    }
}

/// Whether this section is where a kind is kept in order.
pub(super) fn makes(kind: EffectKind) -> bool {
    rank(kind).is_some()
}

/// Where an effect of `kind` made here is put in `stack`: after the last key
/// or colour effect that comes no later in the section's order, and first
/// when there is none.
pub(super) fn place(stack: &[Effect], kind: EffectKind) -> usize {
    let mine = rank(kind);
    (stack.iter())
        .rposition(|effect| rank(effect.kind).is_some_and(|rank| Some(rank) <= mine))
        .map_or(0, |index| index + 1)
}

/// Whether a grade or a curve changes nothing as it is set.
fn neutral(effect: &Effect) -> bool {
    let plain = effect.enabled && effect.mix == 1.0;
    plain
        && match effect.kind {
            EffectKind::Grade => look::spec(effect.kind)
                .iter()
                .all(|spec| is_default(effect.param(spec.key).unwrap_or(spec.default), spec)),
            kind if kind.is_curve() => effect.points == curve::IDENTITY,
            _ => false,
        }
}

/// The clip's effect stack as the groups of the section change it in one
/// frame: read where it is, and copied by the first that changes it.
struct Stack<'a> {
    list: &'a [Effect],
    edited: Option<Vec<Effect>>,
    /// The effects written in this frame, and those taken out.
    touched: Vec<Uuid>,
    gone: Vec<Uuid>,
}

impl Stack<'_> {
    fn now(&self) -> &[Effect] {
        self.edited.as_deref().unwrap_or(self.list)
    }

    fn first(&self, kind: EffectKind) -> Option<&Effect> {
        self.now().iter().find(|effect| effect.kind == kind)
    }

    /// Writes `effect` over the one of its ID, or puts it in its place.
    fn put(&mut self, effect: Effect) {
        let stack = self.edited.get_or_insert_with(|| self.list.to_vec());
        self.touched.push(effect.id);
        match stack.iter_mut().find(|held| held.id == effect.id) {
            Some(held) => *held = effect,
            None => stack.insert(place(stack, effect.kind), effect),
        }
    }

    fn remove(&mut self, id: Uuid) {
        let stack = self.edited.get_or_insert_with(|| self.list.to_vec());
        stack.retain(|effect| effect.id != id);
        self.gone.push(id);
    }
}

/// What the controls of a group's heading asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Head {
    Toggle,
    Remove,
}

/// A hue band's button: its hue, ringed while its rows are the ones shown.
fn band_chip(ui: &mut Ui, p: &Palette, hue: f32, on: bool, dim: bool, label: &str) -> Response {
    let side = widgets::SMALL_BUTTON + 2.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, on, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let colour = Color32::from(Hsva::new(hue.rem_euclid(360.0) / 360.0, 0.78, 0.92, 1.0));
        let colour = if dim || !enabled {
            colour.gamma_multiply(0.4)
        } else {
            colour
        };
        painter.circle_filled(rect.center(), 6.5, colour);
        if on {
            painter.circle_stroke(rect.center(), 9.5, Stroke::new(1.5, p.fg));
        } else if response.hovered() && enabled {
            painter.circle_stroke(rect.center(), 9.5, Stroke::new(1.0, p.muted));
        }
        if response.has_focus() {
            painter.circle_stroke(rect.center(), 11.5, Stroke::new(1.5, p.accent));
        }
    }
    response.on_hover_text(label)
}

fn quiet(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(theme::regular(11.5))
            .color(p.muted),
    );
}

/// A percentage beside the slider that sets it.
fn percent(ui: &mut Ui, p: &Palette, value: f32) {
    ui.add_sized(
        [38.0, 18.0],
        egui::Label::new(
            egui::RichText::new(format!("{:.0} %", value * 100.0))
                .font(theme::regular(12.0))
                .color(p.secondary),
        ),
    );
}

impl Pane<'_> {
    pub(super) fn color(&mut self, ui: &mut Ui) {
        let clip = self.clip;
        // The stack is read where it is and copied only when it changes.
        let held = self.next.effects.take();
        let mut stack = Stack {
            list: held.as_deref().unwrap_or(&clip.effects),
            edited: None,
            touched: Vec::new(),
            gone: Vec::new(),
        };
        self.grade(ui, &mut stack);
        self.bands(ui, &mut stack);
        self.curves(ui, &mut stack);
        self.lut(ui, &mut stack);
        self.removal(ui, &mut stack);
        self.background(ui);

        let Stack {
            edited,
            touched,
            mut gone,
            ..
        } = stack;
        self.next.effects = match edited {
            None => held,
            Some(mut stack) => {
                // A grade or a curve written back to where it changes
                // nothing is not kept, unless keyframes drive it.
                stack.retain(|effect| {
                    let spent =
                        touched.contains(&effect.id) && neutral(effect) && !self.driven(effect.id);
                    if spent {
                        gone.push(effect.id);
                    }
                    !spent
                });
                // What was being keyframed on an effect goes with it.
                (self.next.keys)
                    .retain(|key| key.target.is_none_or(|target| !gone.contains(&target)));
                // A stack back at the stored one is no draft.
                (stack != clip.effects).then_some(stack)
            }
        };
    }

    /// Whether keyframes, stored or being written, drive a value of the
    /// effect or mask `id`.
    fn driven(&self, id: Uuid) -> bool {
        (self.clip.animation.iter()).any(|track| track.target == Some(id) && !track.keys.is_empty())
            || self.next.keys.iter().any(|key| key.target == Some(id))
    }

    /// The ID an effect of `kind` is made with when a value of it is first
    /// touched: the same for every frame of the drag that makes it, and the
    /// stored one's when a draft has only just taken that out.
    fn fresh(&mut self, kind: EffectKind) -> Uuid {
        let clip = self.clip;
        if let Some(stored) = clip.effects.iter().find(|effect| effect.kind == kind) {
            return stored.id;
        }
        let (of, ids) = (self.state.fresh).get_or_insert_with(|| (clip.id, Vec::new()));
        if *of != clip.id {
            *of = clip.id;
            ids.clear();
        }
        match ids.iter().find(|(made, _)| *made == kind) {
            Some((_, id)) => *id,
            None => {
                let id = Uuid::new_v4();
                ids.push((kind, id));
                id
            }
        }
    }

    /// The first effect of `kind` in the stack, or the one that would be
    /// made, which changes nothing yet.
    fn managed<'s>(&mut self, stack: &'s Stack<'_>, kind: EffectKind) -> Cow<'s, Effect> {
        match stack.first(kind) {
            Some(effect) => Cow::Borrowed(effect),
            None => {
                let mut effect = Effect::new(kind);
                effect.id = self.fresh(kind);
                Cow::Owned(effect)
            }
        }
    }

    /// A keyframe was asked for on `effect`, which the stack does not have
    /// yet: the actions made since `from` are made one, after the stack that
    /// has it, so that the effect and its keyframe are one undo step.
    fn creating(&mut self, from: usize, stack: &[Effect], effect: &Effect) {
        let mut commands = Vec::new();
        let mut others = Vec::new();
        for action in self.actions.drain(from..) {
            match action {
                Action::Edit(Command::Batch {
                    commands: several, ..
                }) => commands.extend(several),
                Action::Edit(command) => commands.push(command),
                other => others.push(other),
            }
        }
        if !commands.is_empty() {
            let mut effects = stack.to_vec();
            effects.insert(place(&effects, effect.kind), effect.clone());
            commands.insert(
                0,
                Command::SetEffects {
                    clip_id: self.clip.id,
                    effects,
                },
            );
            self.actions.push(Action::Edit(Command::Batch {
                label: "Add keyframe".into(),
                commands,
            }));
        }
        self.actions.extend(others);
    }

    /// A group's heading: its name and, for an effect the stack has, whether
    /// it is shown and the way to take it out, named `remove`.
    fn heading(
        &mut self,
        ui: &mut Ui,
        title: &str,
        name: &str,
        effect: Option<&Effect>,
        remove: Option<(Icon, &str)>,
    ) -> Option<Head> {
        let p = self.p;
        let mut head = None;
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.set_min_height(widgets::SMALL_BUTTON);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if let Some(effect) = effect {
                    if let Some((icon, label)) = remove
                        && widgets::small_icon_button(ui, p, icon, label).clicked()
                    {
                        head = Some(Head::Remove);
                    }
                    let shown = widgets::small_icon_toggle(
                        ui,
                        p,
                        Icon::Eye,
                        effect.enabled,
                        &format!("Show {name}"),
                    );
                    if shown.clicked() {
                        head = Some(Head::Toggle);
                    }
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(title)
                                .font(theme::medium(11.5))
                                .color(p.muted),
                        )
                        .truncate(),
                    );
                });
            });
        });
        head
    }

    /// Does what a heading asked of the effect `id`.
    fn headed(stack: &mut Stack<'_>, id: Uuid, head: Option<Head>) {
        match head {
            Some(Head::Remove) => stack.remove(id),
            Some(Head::Toggle) => {
                let found = stack.now().iter().find(|effect| effect.id == id).cloned();
                if let Some(mut effect) = found {
                    effect.enabled = !effect.enabled;
                    stack.put(effect);
                }
            }
            None => {}
        }
    }

    /// The rows of an effect's table, but for `skip`. Hands back the effect
    /// as edited, when it was.
    fn table_rows(&mut self, ui: &mut Ui, effect: &Effect, skip: &[ParamKey]) -> Option<Effect> {
        let target = AnimTarget::Effect(effect.kind);
        let stored_of = |_: &ClipDraft, key: ParamKey| effect.param(key).unwrap_or(0.0);
        let mut edited: Option<Effect> = None;
        for spec in look::spec(effect.kind) {
            if skip.contains(&spec.key) {
                continue;
            }
            let stored = effect.param(spec.key).unwrap_or(spec.default);
            if let Some(value) = self.param_row(ui, effect.id, target, spec, stored, &stored_of) {
                set_param(
                    edited.get_or_insert_with(|| effect.clone()),
                    spec.key,
                    value,
                );
            }
        }
        edited
    }

    // ---- Grade ------------------------------------------------------------

    fn grade(&mut self, ui: &mut Ui, stack: &mut Stack<'_>) {
        let p = self.p;
        let from = self.actions.len();
        let (id, head, edited) = {
            let effect = self.managed(stack, EffectKind::Grade);
            let made = matches!(effect, Cow::Borrowed(_));
            let head = self.heading(
                ui,
                "Grade",
                "grade",
                made.then_some(&*effect),
                Some((Icon::Rotate, "Reset grade")),
            );
            let edited = ui
                .push_id(effect.id, |ui| {
                    card(ui, p, |ui| self.table_rows(ui, &effect, &[]))
                })
                .inner;
            if !made {
                self.creating(from, stack.now(), &effect);
            }
            (effect.id, head, edited)
        };
        Self::headed(stack, id, head);
        if let Some(effect) = edited {
            stack.put(effect);
        }
    }

    // ---- Hue bands --------------------------------------------------------

    fn bands(&mut self, ui: &mut Ui, stack: &mut Stack<'_>) {
        let p = self.p;
        let bands = || (stack.now().iter()).filter(|effect| effect.kind == EffectKind::HslBand);
        // The band last opened stays open while the stack has it.
        let open = bands()
            .find(|band| Some(band.id) == self.state.band)
            .or_else(|| bands().next());
        let mut chosen = open.map(|band| band.id);
        let full = stack.now().len() >= look::MAX_EFFECTS;
        let mut add: Option<f32> = None;
        let mut edited: Option<Effect> = None;

        let head = self.heading(
            ui,
            "Hue bands",
            "hue band",
            open,
            Some((Icon::Trash, "Remove hue band")),
        );
        card(ui, p, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                for (index, band) in bands().enumerate() {
                    let hue = band.param(ParamKey::Hue).unwrap_or(0.0);
                    let label = format!("Hue band {}, {}", index + 1, hue_name(hue).to_lowercase());
                    let on = chosen == Some(band.id);
                    if band_chip(ui, p, hue, on, !band.enabled, &label).clicked() {
                        chosen = Some(band.id);
                    }
                }
                ui.add_enabled_ui(!full, |ui| {
                    let plus = widgets::small_icon_button(ui, p, Icon::Plus, "Add hue band")
                        .on_disabled_hover_text("A clip holds sixteen effects at most");
                    egui::Popup::menu(&plus).show(|ui| {
                        widgets::menu_style(ui, 150.0);
                        for (name, hue) in HUES {
                            if widgets::menu_item(ui, p, None, name, "").clicked() {
                                add = Some(hue);
                            }
                        }
                    });
                });
            });
            match open {
                Some(band) => {
                    widgets::hairline(ui, p);
                    edited = ui
                        .push_id(band.id, |ui| self.table_rows(ui, band, &[]))
                        .inner;
                }
                None => quiet(ui, p, "Add a band to shift one range of hues."),
            }
        });

        self.state.band = chosen;
        if let Some(id) = open.map(|band| band.id) {
            Self::headed(stack, id, head);
        }
        if let Some(effect) = edited {
            stack.put(effect);
        }
        if let Some(hue) = add {
            let mut band = Effect::new(EffectKind::HslBand);
            set_param(&mut band, ParamKey::Hue, hue);
            self.state.band = Some(band.id);
            stack.put(band);
        }
    }

    // ---- Curves -----------------------------------------------------------

    fn curves(&mut self, ui: &mut Ui, stack: &mut Stack<'_>) {
        let p = self.p;
        let channel = self.state.channel;
        let name = channel.name();
        let mut chosen = channel;
        let (id, head, points) = {
            let effect = self.managed(stack, channel.kind());
            let made = matches!(effect, Cow::Borrowed(_));
            let head = self.heading(
                ui,
                "Curves",
                &format!("{} curve", name.to_lowercase()),
                made.then_some(&*effect),
                Some((Icon::Rotate, "Reset curve")),
            );
            let mut points: Option<Vec<[f32; 2]>> = None;
            card(ui, p, |ui| {
                let width = ui.available_width();
                widgets::segmented(ui, p, &mut chosen, &Channel::ALL, width);
                ui.add_space(4.0);
                let edit = curve::editor(
                    ui,
                    p,
                    &effect.points,
                    channel.ink(p),
                    &mut self.state.point,
                    name,
                );
                if edit.focused {
                    self.state.keys = Some(ui.ctx().cumulative_frame_nr());
                }
                points = edit.points;
                let count = effect.points.len();
                let label = format!("{count} of {} points", look::MAX_CURVE_POINTS);
                widgets::fitted_row(ui, p, &label, |ui| {
                    ui.add_enabled_ui(count < look::MAX_CURVE_POINTS, |ui| {
                        let add =
                            widgets::button(ui, p, Some(Icon::Plus), "Add point", Kind::Plain)
                                .on_hover_text("Add a point in the curve's widest stretch")
                                .on_disabled_hover_text("A curve holds sixteen points at most");
                        if add.clicked()
                            && let Some((with, index)) = curve::added_midway(&effect.points)
                        {
                            self.state.point = index;
                            points = Some(with);
                        }
                    });
                });
            });
            let points = points.map(|points| Effect {
                points,
                ..effect.clone().into_owned()
            });
            (effect.id, head, points)
        };
        if chosen != channel {
            self.state.channel = chosen;
            self.state.point = 0;
        }
        Self::headed(stack, id, head);
        if let Some(effect) = points {
            stack.put(effect);
        }
    }

    // ---- LUT --------------------------------------------------------------

    fn lut(&mut self, ui: &mut Ui, stack: &mut Stack<'_>) {
        let p = self.p;
        let project = self.project;
        let effect = stack.first(EffectKind::Lut);
        let stored = effect.and_then(|effect| effect.asset_id);
        let mut table = stored;
        let mut mix = effect.map(|effect| effect.mix);
        let tables = || (project.assets.values()).filter(|asset| asset.kind == MediaKind::Lut);
        let head = self.heading(ui, "LUT", "LUT", effect, None);
        card(ui, p, |ui| {
            widgets::fitted_row(ui, p, "Table", |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let import = widgets::small_icon_button(ui, p, Icon::Import, "Import a LUT file");
                if import.clicked() {
                    self.actions.push(Action::ImportFiles);
                }
                let current = match table {
                    None => "None",
                    Some(id) => (project.assets.get(&id)).map_or("Missing", |a| a.name.as_str()),
                };
                egui::ComboBox::from_id_salt("color-lut")
                    .selected_text(current)
                    .width(118.0)
                    .truncate()
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut table, None, "None");
                        for asset in tables() {
                            ui.selectable_value(&mut table, Some(asset.id), &asset.name);
                        }
                    })
                    .response
                    .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "LUT table"));
            });
            if let Some(mix) = &mut mix {
                widgets::fitted_row(ui, p, "Intensity", |ui| {
                    percent(ui, p, *mix);
                    widgets::slider(ui, p, mix, 1.0, 112.0, "LUT intensity");
                });
            } else if tables().next().is_none() {
                quiet(ui, p, "Import a .cube file to grade with it.");
            }
        });

        let id = effect.map(|effect| effect.id);
        let changed = match (effect, table) {
            (Some(effect), Some(asset)) if stored != table || mix != Some(effect.mix) => {
                Some(Effect {
                    asset_id: Some(asset),
                    mix: mix.unwrap_or(effect.mix),
                    ..effect.clone()
                })
            }
            (None, Some(asset)) => Some(Effect {
                asset_id: Some(asset),
                ..Effect::new(EffectKind::Lut)
            }),
            _ => None,
        };
        if let Some(id) = id {
            Self::headed(stack, id, head);
            if table.is_none() {
                stack.remove(id);
            }
        }
        if let Some(effect) = changed {
            stack.put(effect);
        }
    }

    // ---- Background removal -----------------------------------------------

    fn removal(&mut self, ui: &mut Ui, stack: &mut Stack<'_>) {
        let p = self.p;
        let clip = self.clip;
        let project = self.project;
        // A pick is of an effect this clip still has.
        if (self.state.pick).is_some_and(|pick| {
            pick.clip != clip.id || !stack.now().iter().any(|effect| effect.id == pick.effect)
        }) {
            self.state.pick = None;
        }
        let keyed = (stack.now().iter()).find(|effect| {
            matches!(
                effect.kind,
                EffectKind::ChromaKey | EffectKind::DifferenceMatte
            )
        });
        let method = match keyed.map(|effect| effect.kind) {
            Some(EffectKind::ChromaKey) => Removal::Key,
            Some(_) => Removal::Plate,
            None if self.state.plate_wanted == Some(clip.id) => Removal::Plate,
            None => Removal::Off,
        };
        let mut chosen = method;
        let stored_plate = keyed.and_then(|effect| effect.asset_id);
        let mut plate = stored_plate;
        let mut edited: Option<Effect> = None;

        let head = self.heading(ui, "Remove background", "background removal", keyed, None);
        let width = ui.available_width();
        widgets::segmented(
            ui,
            p,
            &mut chosen,
            &[
                (Removal::Off, "Off"),
                (Removal::Key, "Chroma key"),
                (Removal::Plate, "Clean plate"),
            ],
            width,
        );
        match (method, keyed) {
            (Removal::Key, Some(effect)) => {
                ui.push_id(effect.id, |ui| {
                    card(ui, p, |ui| {
                        edited = self.key_colour_row(ui, effect);
                        let rows =
                            self.table_rows(ui, edited.as_ref().unwrap_or(effect), &KEY_COLOUR);
                        edited = rows.or(edited.take());
                    });
                });
            }
            (Removal::Plate, _) => {
                card(ui, p, |ui| {
                    widgets::fitted_row(ui, p, "Plate", |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let import =
                            widgets::small_icon_button(ui, p, Icon::Import, "Import a clean plate");
                        if import.clicked() {
                            self.actions.push(Action::ImportFiles);
                        }
                        let current = match plate {
                            None => "Choose",
                            Some(id) => {
                                (project.assets.get(&id)).map_or("Missing", |a| a.name.as_str())
                            }
                        };
                        egui::ComboBox::from_id_salt("color-plate")
                            .selected_text(current)
                            .width(118.0)
                            .truncate()
                            .show_ui(ui, |ui| {
                                let stills = (project.assets.values())
                                    .filter(|asset| asset.kind == MediaKind::Image);
                                for asset in stills {
                                    ui.selectable_value(&mut plate, Some(asset.id), &asset.name);
                                }
                            })
                            .response
                            .widget_info(|| {
                                WidgetInfo::labeled(WidgetType::ComboBox, true, "Clean plate")
                            });
                    });
                    match keyed {
                        Some(effect) => {
                            edited = ui
                                .push_id(effect.id, |ui| self.table_rows(ui, effect, &[]))
                                .inner;
                        }
                        None => quiet(ui, p, "Choose a still of the scene without its subject."),
                    }
                });
            }
            _ => {}
        }
        quiet(
            ui,
            p,
            "No automatic subject cut-out: use a key colour or a clean plate.",
        );

        // A matte is made, or given another plate, when one is chosen.
        let matte = (plate != stored_plate).then(|| Effect {
            asset_id: plate,
            ..keyed.map_or_else(|| Effect::new(EffectKind::DifferenceMatte), Effect::clone)
        });
        let keyed = keyed.map(|effect| effect.id);
        if let Some(id) = keyed {
            Self::headed(stack, id, head);
        }
        if chosen != method {
            // One way at a time: the other's effect is taken out.
            if let Some(id) = keyed {
                stack.remove(id);
            }
            self.state.pick = None;
            self.state.plate_wanted = (chosen == Removal::Plate).then_some(clip.id);
            if chosen == Removal::Key {
                stack.put(Effect::new(EffectKind::ChromaKey));
            }
            return;
        }
        if let Some(effect) = edited {
            stack.put(effect);
        }
        if let Some(matte) = matte {
            self.state.plate_wanted = None;
            stack.put(matte);
        }
    }

    /// The key colour of a chroma key: a swatch, and beside it the way to
    /// pick the colour off the picture in the viewer. Hands back the effect
    /// as edited, when it was.
    fn key_colour_row(&mut self, ui: &mut Ui, effect: &Effect) -> Option<Effect> {
        let p = self.p;
        let id = Some(effect.id);
        let stored_of = |_: &ClipDraft, key: ParamKey| effect.param(key).unwrap_or(0.0);
        let state = self.key_state(id, &KEY_COLOUR);
        let mut rgb = KEY_COLOUR.map(|key| self.shown(id, key, stored_of(self.next, key)));
        let pick = ColourPick {
            clip: self.clip.id,
            effect: effect.id,
        };
        let armed = self.state.pick == Some(pick);
        let (mut picked, mut toggled) = (false, false);
        let row = widgets::keyed_row_with(
            ui,
            p,
            "Key colour",
            "key colour",
            state,
            |ui| {
                let swatch = ui.color_edit_button_rgb(&mut rgb);
                swatch.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::ColorButton, true, "Key colour")
                });
                picked = swatch.changed();
            },
            |ui| {
                toggled = widgets::small_icon_toggle(
                    ui,
                    p,
                    Icon::Pipette,
                    armed,
                    "Pick the key colour from the picture",
                )
                .clicked();
            },
        );
        let mut edited: Option<Effect> = None;
        if picked {
            for (key, value) in KEY_COLOUR.into_iter().zip(rgb) {
                if let Some(value) = self.write(id, key, value.clamp(0.0, 1.0)) {
                    set_param(edited.get_or_insert_with(|| effect.clone()), key, value);
                }
            }
        }
        let target = AnimTarget::Effect(effect.kind);
        self.key_controls(ui, &row, id, target, &KEY_COLOUR, &stored_of);
        if toggled {
            self.state.pick = (!armed).then_some(pick);
        }
        edited
    }

    // ---- Background -------------------------------------------------------

    /// What is put behind the clip once its background is gone: a clip of a
    /// picture on a track under it, or the canvas's own colour.
    fn background(&mut self, ui: &mut Ui) {
        let p = self.p;
        let project = self.project;
        let clip = self.clip;
        widgets::section(ui, p, "Background");
        card(ui, p, |ui| {
            widgets::fitted_row(ui, p, "Behind this clip", |ui| {
                let add = widgets::button(ui, p, Some(Icon::Plus), "Add", Kind::Plain)
                    .on_hover_text("Put a picture or a video on a track under this clip");
                egui::Popup::menu(&add).show(|ui| {
                    widgets::menu_style(ui, 200.0);
                    let pictures = || {
                        (project.assets.values())
                            .filter(|asset| rules::track_accepts(TrackKind::Video, asset))
                    };
                    if pictures().next().is_none() {
                        ui.add_enabled_ui(false, |ui| {
                            widgets::menu_item(ui, p, None, "No pictures in the library", "");
                        });
                    }
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for asset in pictures() {
                                let icon = Some(icon_of_media(asset.kind));
                                if widgets::menu_item(ui, p, icon, &asset.name, "").clicked() {
                                    self.actions.push(
                                        match background_command(project, clip.id, asset.id) {
                                            Ok(command) => Action::Edit(command),
                                            Err(why) => Action::Notify(why),
                                        },
                                    );
                                }
                            }
                        });
                });
            });
            widgets::fitted_row(ui, p, "Canvas colour", |ui| {
                let settings = (self.canvas.canvas.as_ref()).unwrap_or(&project.settings);
                let [r, g, b, _] = settings.background;
                let mut rgb = [r, g, b];
                let swatch = ui.color_edit_button_srgb(&mut rgb);
                swatch.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::ColorButton, true, "Canvas colour")
                });
                if swatch.changed() {
                    let mut settings = settings.clone();
                    settings.background = [rgb[0], rgb[1], rgb[2], 255];
                    self.canvas.canvas = Some(settings);
                }
            });
        });
        quiet(
            ui,
            p,
            "The canvas colour shows where no track has a picture under the clip.",
        );
    }
}

/// The edit that gives the chroma key `pick` names the colour `rgb`, picked
/// at `playhead`: the stored colour, or the keyframe there of a channel that
/// keyframes drive. `stack` is the clip's effects as they are held in a
/// draft, when they are. One command, so one undo step; none when the clip
/// has no such effect any more.
pub fn key_colour_command(
    project: &Project,
    pick: ColourPick,
    stack: Option<&[Effect]>,
    rgb: [f32; 3],
    playhead: Time,
) -> Option<Command> {
    let (_, clip) = project.clip(pick.clip)?;
    let stack = stack.unwrap_or(&clip.effects);
    let effect = (stack.iter())
        .find(|effect| effect.id == pick.effect && effect.kind == EffectKind::ChromaKey)?;
    let local = (playhead - clip.start).clamp(Time::ZERO, clip.duration);
    let reach = project.settings.frame_rate.frame_time(1).0 / 2;
    let mut edited = effect.clone();
    let mut commands = Vec::new();
    for (key, value) in KEY_COLOUR.into_iter().zip(rgb) {
        let value = if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        };
        let track = (clip.animation.iter()).find(|track| {
            track.target == Some(effect.id) && track.key == key && !track.keys.is_empty()
        });
        match track {
            Some(track) => {
                let at = (track.keys.iter())
                    .find(|key| (key.at.0 - local.0).abs() <= reach)
                    .map_or(local, |key| key.at);
                commands.push(Command::SetKeyframes {
                    clip_id: clip.id,
                    target: Some(effect.id),
                    key,
                    keys: with_key(&track.keys, at, value),
                });
            }
            None => set_param(&mut edited, key, value),
        }
    }
    if edited != *effect || stack != clip.effects {
        let effects = (stack.iter())
            .map(|held| {
                if held.id == edited.id {
                    edited.clone()
                } else {
                    held.clone()
                }
            })
            .collect();
        commands.insert(
            0,
            Command::SetEffects {
                clip_id: clip.id,
                effects,
            },
        );
    }
    (!commands.is_empty()).then(|| Command::Batch {
        label: "Pick key colour".into(),
        commands,
    })
}

/// The edit that puts `asset` behind the clip `clip_id`, as one undo step: a
/// clip of it over the same stretch of the timeline, on the nearest video
/// track under the clip's that is unlocked and free there, or on a new one
/// put directly under it. A still lasts as long as the clip; anything else as
/// long as it has.
pub(super) fn background_command(
    project: &Project,
    clip_id: ClipId,
    asset: editor_core::AssetId,
) -> Result<Command, String> {
    let (track, clip) = project
        .clip(clip_id)
        .ok_or("That clip is no longer on the timeline.")?;
    let asset = project
        .assets
        .get(&asset)
        .ok_or("That media is no longer in the library.")?;
    if !rules::track_accepts(TrackKind::Video, asset) {
        return Err("Only a picture or a video can be a background.".to_owned());
    }
    let index = (project.tracks.iter())
        .position(|other| other.id == track.id)
        .ok_or("That clip is no longer on the timeline.")?;
    let mut behind = Clip::from_asset(asset, clip.start);
    behind.duration = if asset.kind == MediaKind::Image {
        clip.duration
    } else {
        asset.duration.min(clip.duration)
    };
    let free = |track: &Track| {
        track.kind == TrackKind::Video
            && !track.locked
            && rules::free_start(track, behind.start, behind.duration, &[]) == behind.start
    };
    let under = (project.tracks[..index].iter().rev()).find(|track| free(track));
    let commands = match under {
        Some(under) => vec![Command::AddClip {
            track_id: under.id,
            clip: behind,
        }],
        None => {
            let count = (project.tracks.iter())
                .filter(|track| track.kind == TrackKind::Video)
                .count();
            let under = Track::new(format!("Video {}", count + 1), TrackKind::Video);
            let track_id = under.id;
            vec![
                Command::AddTrack { track: under },
                // A new track is the topmost: it is brought down to where
                // the clip's track is, which moves up over it.
                Command::ReorderTrack { track_id, index },
                Command::AddClip {
                    track_id,
                    clip: behind,
                },
            ]
        }
    };
    Ok(Command::Batch {
        label: "Add background".into(),
        commands,
    })
}
