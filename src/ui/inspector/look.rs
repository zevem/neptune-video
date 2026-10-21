//! The Look section: the looks a clip can be given and its effect stack.
//!
//! The rows of an effect are made from `look::spec`, one a parameter; what
//! this file adds to that table is how a parameter is named and shown. Every
//! change is the whole new stack, held in the draft and committed as one
//! `SetEffects` for the clip the draft was made for.
use super::{
    FIELD, button, button_row, card, card_head, clip::Field, clip::Pane, fitted_row, head_button,
    keyed_row, menu_heading, note, section, select_menu, tall_row, well,
};
use crate::{
    icons::Icon,
    theme,
    ui::{
        Action, ClipDraft, kit,
        widgets::{self, KeyState, Kind},
    },
};
use editor_core::{
    AnimTarget, Command, Effect, EffectKind, LookPreset, MediaKind, ParamKey, ParamSpec,
    ParamValue, look,
};
use eframe::egui::{self, Ui};
use uuid::Uuid;

pub(super) const PRESETS: [(LookPreset, &str); 7] = [
    (LookPreset::Cinematic, "Cinematic"),
    (LookPreset::TealOrange, "Teal & orange"),
    (LookPreset::Vintage, "Vintage"),
    (LookPreset::Faded, "Faded"),
    (LookPreset::Noir, "Noir"),
    (LookPreset::Warm, "Warm"),
    (LookPreset::Cool, "Cool"),
];

/// What an effect does to the picture: the heading it is added under, and
/// the section that adds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Family {
    Stylize,
    Blur,
    Distort,
    Key,
    Color,
    Motion,
    /// Heard, not drawn: the kinds a clip on an audio track takes.
    Audio,
}

impl Family {
    /// Every family, in the order the library lists them.
    pub(super) const ALL: [Self; 7] = [
        Self::Stylize,
        Self::Blur,
        Self::Distort,
        Self::Key,
        Self::Color,
        Self::Motion,
        Self::Audio,
    ];

    /// The families the Look section adds, in the order of its menu. Colour
    /// and motion effects are added by their own sections.
    const LOOK: [Self; 4] = [Self::Stylize, Self::Blur, Self::Distort, Self::Key];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Stylize => "Stylize",
            Self::Blur => "Blur and sharpen",
            Self::Distort => "Distort",
            Self::Key => "Key",
            Self::Color => "Colour",
            Self::Motion => "Motion",
            Self::Audio => "Audio",
        }
    }
}

pub(super) fn family(kind: EffectKind) -> Family {
    match kind {
        EffectKind::Glow
        | EffectKind::Grain
        | EffectKind::Vignette
        | EffectKind::Glitch
        | EffectKind::Shadow => Family::Stylize,
        EffectKind::Blur | EffectKind::MotionBlur | EffectKind::Sharpen => Family::Blur,
        EffectKind::Shake | EffectKind::Distort => Family::Distort,
        EffectKind::ChromaKey | EffectKind::DifferenceMatte => Family::Key,
        EffectKind::Grade
        | EffectKind::HslBand
        | EffectKind::CurveMaster
        | EffectKind::CurveRed
        | EffectKind::CurveGreen
        | EffectKind::CurveBlue
        | EffectKind::Lut => Family::Color,
        EffectKind::Stabilize => Family::Motion,
        EffectKind::HighPass
        | EffectKind::EqBand
        | EffectKind::Compressor
        | EffectKind::Limiter
        | EffectKind::Denoise
        | EffectKind::Pitch
        | EffectKind::CenterCut => Family::Audio,
    }
}

/// Whether the Look section's menu adds this kind. A difference matte is
/// not an effect until it has its clean plate: the Color section adds it,
/// with the plate chosen there.
pub(super) fn addable(kind: EffectKind) -> bool {
    Family::LOOK.contains(&family(kind)) && kind != EffectKind::DifferenceMatte
}

pub(super) fn effect_name(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Glow => "Glow",
        EffectKind::Shake => "Shake",
        EffectKind::Distort => "Wave",
        EffectKind::MotionBlur => "Motion blur",
        EffectKind::Grain => "Grain",
        EffectKind::Sharpen => "Sharpen",
        EffectKind::Shadow => "Shadow",
        EffectKind::Blur => "Blur",
        EffectKind::Vignette => "Vignette",
        EffectKind::Glitch => "Glitch",
        EffectKind::Grade => "Grade",
        EffectKind::HslBand => "Hue band",
        EffectKind::CurveMaster => "Curve",
        EffectKind::CurveRed => "Red curve",
        EffectKind::CurveGreen => "Green curve",
        EffectKind::CurveBlue => "Blue curve",
        EffectKind::Lut => "LUT",
        EffectKind::ChromaKey => "Chroma key",
        EffectKind::DifferenceMatte => "Difference matte",
        EffectKind::Stabilize => "Stabilize",
        EffectKind::HighPass => "High-pass",
        EffectKind::EqBand => "EQ band",
        EffectKind::Compressor => "Compressor",
        EffectKind::Limiter => "Limiter",
        EffectKind::Denoise => "Noise reduction",
        EffectKind::Pitch => "Pitch",
        EffectKind::CenterCut => "Reduce centre",
    }
}

/// The name a parameter's row has.
pub(super) fn label(target: AnimTarget, key: ParamKey) -> &'static str {
    use EffectKind::{
        Compressor, Denoise, Distort, EqBand, Glitch, Grain, HighPass, Limiter, Pitch, Stabilize,
        Vignette,
    };
    match (target, key) {
        // A key of an audio kind means what the sound does with it.
        (AnimTarget::Effect(HighPass | EqBand), ParamKey::Frequency) => "Frequency",
        (AnimTarget::Effect(EqBand), ParamKey::Gain) => "Gain",
        (AnimTarget::Effect(Compressor), ParamKey::Amount) => "Ratio",
        (AnimTarget::Effect(Compressor), ParamKey::Gain) => "Make-up",
        (AnimTarget::Effect(Limiter), ParamKey::Threshold) => "Ceiling",
        (AnimTarget::Effect(Denoise), ParamKey::Threshold) => "Noise floor",
        (AnimTarget::Effect(Pitch), ParamKey::Amount) => "Shift",
        (AnimTarget::Effect(Stabilize), ParamKey::Smoothness) => "Strength",
        (AnimTarget::Effect(Stabilize), ParamKey::CropZoom) => "Crop zoom",
        (AnimTarget::Effect(Grain | Vignette), ParamKey::Radius) => "Size",
        (AnimTarget::Effect(Distort), ParamKey::Frequency) => "Waves",
        (AnimTarget::Effect(Distort), ParamKey::OffsetX) => "Phase",
        (AnimTarget::Effect(Glitch), ParamKey::OffsetX) => "Shift",
        (_, key) => match key {
            ParamKey::PositionX => "Position X",
            ParamKey::PositionY => "Position Y",
            ParamKey::ScaleX => "Scale X",
            ParamKey::ScaleY => "Scale Y",
            ParamKey::Rotation => "Rotation",
            ParamKey::Opacity => "Opacity",
            ParamKey::CropLeft => "Crop left",
            ParamKey::CropTop => "Crop top",
            ParamKey::CropRight => "Crop right",
            ParamKey::CropBottom => "Crop bottom",
            ParamKey::Gain => "Volume",
            ParamKey::Amount => "Amount",
            ParamKey::Radius => "Radius",
            ParamKey::Angle => "Angle",
            ParamKey::Threshold => "Threshold",
            ParamKey::Softness => "Softness",
            ParamKey::Frequency => "Rate",
            ParamKey::OffsetX => "Offset X",
            ParamKey::OffsetY => "Offset Y",
            ParamKey::Exposure => "Exposure",
            ParamKey::Contrast => "Contrast",
            ParamKey::Saturation => "Saturation",
            ParamKey::Temperature => "Temperature",
            ParamKey::Tint => "Tint",
            ParamKey::Lift => "Lift",
            ParamKey::Gamma => "Gamma",
            ParamKey::GainLuma => "Gain",
            ParamKey::Hue => "Hue",
            ParamKey::HueWidth => "Range",
            ParamKey::HueShift => "Hue shift",
            ParamKey::SatShift => "Saturation shift",
            ParamKey::LumShift => "Lightness shift",
            ParamKey::KeyR => "Key red",
            ParamKey::KeyG => "Key green",
            ParamKey::KeyB => "Key blue",
            ParamKey::Tolerance => "Tolerance",
            ParamKey::Spill => "Spill",
            ParamKey::Smoothness => "Smoothness",
            ParamKey::CropZoom => "Zoom",
            ParamKey::CenterX => "Centre X",
            ParamKey::CenterY => "Centre Y",
            ParamKey::Width => "Width",
            ParamKey::Height => "Height",
            ParamKey::Feather => "Feather",
            ParamKey::Roundness => "Roundness",
        },
    }
}

/// How a parameter's value is shown: its unit, and from the table's range
/// how fine its field is. A length or an amount is a part of something and
/// is shown in hundredths of it. An audio kind's levels are decibels, its
/// frequencies hertz dragged along a logarithmic scale, as they are heard.
pub(super) fn presented(target: AnimTarget, spec: &ParamSpec) -> (&'static str, Field) {
    use EffectKind::{Compressor, Distort, EqBand, Pitch};
    let audio = matches!(target, AnimTarget::Effect(kind) if kind.is_audio());
    let (scale, suffix) = match (target, spec.key) {
        (_, ParamKey::Frequency | ParamKey::Gain | ParamKey::Threshold) if audio => (
            1.0,
            if spec.key == ParamKey::Frequency {
                " Hz"
            } else {
                " dB"
            },
        ),
        (AnimTarget::Effect(Compressor), ParamKey::Amount) => (1.0, ":1"),
        (AnimTarget::Effect(EqBand), ParamKey::Width) => (1.0, " oct"),
        (AnimTarget::Effect(Pitch), ParamKey::Amount) => (1.0, " st"),
        (
            _,
            ParamKey::Angle
            | ParamKey::Rotation
            | ParamKey::Hue
            | ParamKey::HueWidth
            | ParamKey::HueShift,
        ) => (1.0, "°"),
        (AnimTarget::Effect(Distort), ParamKey::Frequency | ParamKey::OffsetX) => (1.0, ""),
        (_, ParamKey::Frequency) => (1.0, " Hz"),
        (_, ParamKey::Exposure) => (1.0, " EV"),
        (_, ParamKey::Gamma) => (1.0, ""),
        (_, ParamKey::CropZoom) => (1.0, " ×"),
        _ => (100.0, " %"),
    };
    let span = (spec.max - spec.min) * scale;
    let decimals = match span {
        span if span >= 100.0 => 0,
        span if span >= 10.0 => 1,
        _ => 2,
    };
    let field = Field {
        scale,
        min: spec.min,
        max: spec.max,
        prefix: None,
        suffix,
        decimals,
        // A field is dragged across its range in three hundred points, and
        // no faster than its unit is told apart: a range of many turns is
        // dragged half a degree at a time, as the clip's own rotation is.
        speed: f64::from(span / 300.0).clamp(0.001, if suffix == "°" { 0.5 } else { 1.0 }),
        width: FIELD,
        log: audio && spec.key == ParamKey::Frequency,
    };
    (label(target, spec.key), field)
}

/// Whether a value is the one its parameter has when none is stored.
pub(super) fn is_default(value: f32, spec: &ParamSpec) -> bool {
    (value - spec.default).abs() <= 1e-5 * spec.default.abs().max(1.0)
}

pub(super) fn set_param(effect: &mut Effect, key: ParamKey, value: f32) {
    match effect.params.iter_mut().find(|param| param.key == key) {
        Some(param) => param.value = value,
        None => effect.params.push(ParamValue { key, value }),
    }
}

/// A change to the stack that is not to one effect's own values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Reorder {
    Up(usize),
    Down(usize),
    Remove(usize),
    Add(EffectKind),
}

pub(super) const KEY_COLOUR: [ParamKey; 3] = [ParamKey::KeyR, ParamKey::KeyG, ParamKey::KeyB];

impl Pane<'_> {
    /// A row for one parameter of an effect or a mask, with its keyframe
    /// diamond, and a reset while the stored value is not the default. Hands
    /// back the new stored value when that is what changed.
    pub(super) fn param_row(
        &mut self,
        ui: &mut Ui,
        id: Uuid,
        target: AnimTarget,
        spec: &ParamSpec,
        stored: f32,
        stored_of: &dyn Fn(&ClipDraft, ParamKey) -> f32,
    ) -> Option<f32> {
        let p = self.p;
        let (label, field) = presented(target, spec);
        let what = label.to_lowercase();
        let keys = [spec.key];
        let state = self.key_state(Some(id), &keys);
        let mut changed = None;
        let mut reset = false;
        let row = keyed_row(
            ui,
            p,
            label,
            &what,
            state,
            |ui| changed = self.field(ui, Some(id), spec.key, stored, &field, label),
            |ui| {
                reset = state == KeyState::Off
                    && !is_default(stored, spec)
                    && kit::small_icon_button(ui, p, Icon::Rotate, &format!("Reset {what}"))
                        .clicked();
            },
        );
        self.key_controls(ui, &row, Some(id), target, &keys, stored_of);
        if reset { Some(spec.default) } else { changed }
    }

    /// As [`Self::param_row`], for a value that takes no keyframes: the
    /// field and its reset, and no diamond. An audio effect's values are
    /// read once when its sound starts, so the project keys none of them.
    pub(super) fn plain_param_row(
        &mut self,
        ui: &mut Ui,
        id: Uuid,
        target: AnimTarget,
        spec: &ParamSpec,
        stored: f32,
    ) -> Option<f32> {
        let p = self.p;
        let (label, field) = presented(target, spec);
        let what = label.to_lowercase();
        let mut changed = None;
        let mut reset = false;
        fitted_row(ui, p, label, |ui| {
            // The field keeps its place at the trailing edge whether or not
            // the reset is beside it.
            changed = self.field(ui, Some(id), spec.key, stored, &field, label);
            reset = !is_default(stored, spec)
                && kit::small_icon_button(ui, p, Icon::Rotate, &format!("Reset {what}")).clicked();
        });
        if reset { Some(spec.default) } else { changed }
    }

    pub(super) fn look(&mut self, ui: &mut Ui) {
        let p = self.p;
        let clip = self.clip;

        // The look the clip has is read from its effects, so it is the same
        // look after an undo, a save, or when the library or the agent gave
        // it: adjusted or taken apart in the stack below, it is a look no
        // longer.
        let held = self.next.effects.take();
        let stack: &[Effect] = held.as_deref().unwrap_or(&clip.effects);
        let mut remade: Option<Option<Vec<Effect>>> = None;
        let given = look::applied_look(stack);
        let look_command = |label: &str, effects: Vec<Effect>| {
            Action::Edit(Command::Batch {
                label: label.to_owned(),
                commands: vec![Command::SetEffects {
                    clip_id: clip.id,
                    effects,
                }],
            })
        };

        section(ui, p, "Looks");
        card(ui, p, |ui| {
            let given_preset = given.map(|given| given.preset);
            let picked = look_cards(ui, p, given_preset, |chosen| {
                if chosen {
                    "The look the clip has"
                } else if given.is_some() {
                    "Give the clip this look in place of the one it has"
                } else {
                    "Give the clip this look: its effects are added below"
                }
            });
            if let Some(preset) = picked.filter(|preset| Some(*preset) != given_preset) {
                // One look at a time: this one's effects take the place of
                // the last one's in the stack, as strong as that one was.
                let strength = given.map_or(self.state.strength, |given| given.strength);
                let effects = look::with_look(stack, Some((preset, strength)));
                self.actions.push(look_command("Apply look", effects));
            }
            fitted_row(ui, p, "Strength", |ui| {
                // Of the look the clip has, how much of it that is; with no
                // look, how much of one the next brings.
                let mut strength = given.map_or(self.state.strength, |given| given.strength);
                percent(ui, p, strength);
                let width = slider_width(ui, 112.0);
                let slider = widgets::slider(ui, p, &mut strength, 1.0, width, "Look strength")
                    .on_hover_text(if given.is_some() {
                        "How much of its look the clip has"
                    } else {
                        "How much of a look the next one brings"
                    });
                if slider.changed() {
                    // Weaker than this a look is no look that can be told.
                    let strength = strength.clamp(look::MIN_LOOK_STRENGTH, 1.0);
                    match given {
                        // The look the clip has is made again at this
                        // strength, as a draft: a drag of the slider is one
                        // step.
                        Some(given) => {
                            let effects = look::with_look(stack, Some((given.preset, strength)));
                            remade = Some((effects != clip.effects).then_some(effects));
                        }
                        None => self.state.strength = strength,
                    }
                }
            });
            if let Some(given) = given {
                let name = (PRESETS.iter())
                    .find(|(preset, _)| *preset == given.preset)
                    .map_or("look", |(_, name)| name);
                fitted_row(ui, p, name, |ui| {
                    if button(ui, p, None, "Remove look", Kind::Plain).clicked() {
                        let effects = look::with_look(stack, None);
                        self.actions.push(look_command("Remove look", effects));
                    }
                });
            }
        });
        self.next.effects = remade.unwrap_or(held);

        // The stack is read where it is and copied only when it changes.
        let held = self.next.effects.take();
        let list: &[Effect] = held.as_deref().unwrap_or(&clip.effects);
        let mut edited: Vec<(usize, Effect)> = Vec::new();
        let mut reorder: Option<Reorder> = None;

        section(ui, p, "Effects");
        for (index, effect) in list.iter().enumerate() {
            ui.push_id(effect.id, |ui| {
                super::keyed_card(ui, p, |ui| {
                    if let Some(effect) =
                        self.effect_rows(ui, effect, index, list.len(), &mut reorder)
                    {
                        edited.push((index, effect));
                    }
                });
            });
        }
        if list.is_empty() {
            card(ui, p, |ui| note(ui, p, "No effects on this clip."));
        }
        button_row(ui, p, |ui| {
            ui.add_enabled_ui(list.len() < look::MAX_EFFECTS, |ui| {
                let add = button(ui, p, Some(Icon::Plus), "Add effect", Kind::Plain)
                    .on_disabled_hover_text("A clip holds sixteen effects at most");
                egui::Popup::menu(&add).show(|ui| {
                    widgets::menu_style(ui, 196.0);
                    for (at, family) in Family::LOOK.into_iter().enumerate() {
                        if at > 0 {
                            widgets::menu_separator(ui, p);
                        }
                        menu_heading(ui, p, family.label());
                        for kind in EffectKind::ALL {
                            if self::family(kind) == family
                                && addable(kind)
                                && widgets::menu_item(ui, p, None, effect_name(kind), "").clicked()
                            {
                                reorder = Some(Reorder::Add(kind));
                            }
                        }
                    }
                });
            });
        });

        let restacked = self.restack(list, edited, reorder);
        self.next.effects = restacked.unwrap_or(held);
    }

    /// The stack `list` with its effects as `edited` and changed by
    /// `reorder`, to be held in the draft: `Some(None)` when that is the
    /// stored stack again, `None` when nothing was changed.
    pub(super) fn restack(
        &mut self,
        list: &[Effect],
        edited: Vec<(usize, Effect)>,
        reorder: Option<Reorder>,
    ) -> Option<Option<Vec<Effect>>> {
        if edited.is_empty() && reorder.is_none() {
            return None;
        }
        let mut stack = list.to_vec();
        for (index, effect) in edited {
            stack[index] = effect;
        }
        match reorder {
            Some(Reorder::Up(index)) if index > 0 => stack.swap(index, index - 1),
            Some(Reorder::Down(index)) if index + 1 < stack.len() => {
                stack.swap(index, index + 1);
            }
            Some(Reorder::Remove(index)) => {
                // What was being keyframed on it goes with it.
                let removed = stack.remove(index).id;
                self.next.keys.retain(|key| key.target != Some(removed));
            }
            Some(Reorder::Add(kind)) => {
                let effect = Effect::new(kind);
                self.state.effect = Some(effect.id);
                stack.push(effect);
            }
            _ => {}
        }
        // A stack back at the stored one is no draft.
        Some((stack != self.clip.effects).then_some(stack))
    }

    /// The heading of one effect of the stack: its name, which opens and
    /// closes its rows, and beside it the switch, the moves and the removal.
    /// `switch` is the switch's glyph and the verb that names it ("Show
    /// Glow"). Hands back whether the rows are open and whether the switch
    /// was pressed.
    pub(super) fn effect_head(
        &mut self,
        ui: &mut Ui,
        effect: &Effect,
        index: usize,
        count: usize,
        reorder: &mut Option<Reorder>,
        switch: (Icon, &str),
    ) -> (bool, bool) {
        let p = self.p;
        let name = effect_name(effect.kind);
        let open = self.state.effect == Some(effect.id);
        let mut switched = false;
        let header = card_head(
            ui,
            p,
            name,
            Some(open),
            !effect.enabled,
            (4, |ui: &mut Ui| {
                let (icon, verb) = switch;
                switched =
                    kit::small_icon_toggle(ui, p, icon, effect.enabled, &format!("{verb} {name}"))
                        .clicked();
                if head_button(ui, p, Icon::ArrowUp, index > 0, &format!("Move {name} up")) {
                    *reorder = Some(Reorder::Up(index));
                }
                if head_button(
                    ui,
                    p,
                    Icon::ArrowDown,
                    index + 1 < count,
                    &format!("Move {name} down"),
                ) {
                    *reorder = Some(Reorder::Down(index));
                }
                if head_button(ui, p, Icon::Trash, true, &format!("Remove {name}")) {
                    *reorder = Some(Reorder::Remove(index));
                }
            }),
        );
        if header.is_some_and(|header| header.clicked()) {
            self.state.effect = (!open).then_some(effect.id);
        }
        (open, switched)
    }

    /// One effect of the stack: its heading, and its rows while it is open.
    /// Hands back the effect as edited, when it was.
    fn effect_rows(
        &mut self,
        ui: &mut Ui,
        effect: &Effect,
        index: usize,
        count: usize,
        reorder: &mut Option<Reorder>,
    ) -> Option<Effect> {
        let p = self.p;
        let name = effect_name(effect.kind);
        let mut edited: Option<Effect> = None;
        let mut edit = |change: &dyn Fn(&mut Effect)| {
            change(edited.get_or_insert_with(|| effect.clone()));
        };

        let (open, switched) =
            self.effect_head(ui, effect, index, count, reorder, (Icon::Eye, "Show"));
        if switched {
            edit(&|effect| effect.enabled = !effect.enabled);
        }
        if !open {
            return edited;
        }

        fitted_row(ui, p, "Mix", |ui| {
            let mut mix = effect.mix;
            percent(ui, p, mix);
            let width = slider_width(ui, 112.0);
            if widgets::slider(ui, p, &mut mix, 1.0, width, &format!("{name} mix")).changed() {
                edit(&|effect| effect.mix = mix);
            }
        });

        let target = AnimTarget::Effect(effect.kind);
        let stored_of = |_: &ClipDraft, key: ParamKey| effect.param(key).unwrap_or(0.0);
        // A key colour is one swatch, not three numbers.
        if effect.kind == EffectKind::ChromaKey {
            let state = self.key_state(Some(effect.id), &KEY_COLOUR);
            let mut rgb =
                KEY_COLOUR.map(|key| self.shown(Some(effect.id), key, stored_of(self.next, key)));
            let mut picked = false;
            let row = keyed_row(
                ui,
                p,
                "Key colour",
                "key colour",
                state,
                |ui| picked = well::linear_well(ui, p, "Key colour", &mut rgb).changed(),
                |_| {},
            );
            if picked {
                for (key, value) in KEY_COLOUR.into_iter().zip(rgb) {
                    if let Some(value) = self.write(Some(effect.id), key, value.clamp(0.0, 1.0)) {
                        edit(&|effect| set_param(effect, key, value));
                    }
                }
            }
            self.key_controls(ui, &row, Some(effect.id), target, &KEY_COLOUR, &stored_of);
        }
        for spec in look::spec(effect.kind) {
            if KEY_COLOUR.contains(&spec.key) {
                continue;
            }
            let stored = effect.param(spec.key).unwrap_or(spec.default);
            if let Some(value) = self.param_row(ui, effect.id, target, spec, stored, &stored_of) {
                edit(&|effect| set_param(effect, spec.key, value));
            }
        }
        if effect.kind.is_curve() {
            super::fact(ui, p, "Points", effect.points.len().to_string());
        }
        if effect.kind.needs_asset() {
            // A lookup is done in a table, and a clean plate is a still.
            let wanted = if effect.kind == EffectKind::Lut {
                MediaKind::Lut
            } else {
                MediaKind::Image
            };
            let label = if effect.kind == EffectKind::Lut {
                "Table"
            } else {
                "Clean plate"
            };
            let project = self.project;
            let mut chosen = effect.asset_id;
            tall_row(ui, p, label, |ui| {
                let current = (chosen.and_then(|id| project.assets.get(&id)))
                    .map_or("Missing", |asset| asset.name.as_str());
                let width = ui.available_width().min(132.0);
                let (_, picked) =
                    select_menu(ui, p, ("effect-asset", label), current, width, |ui| {
                        let mut picked = None;
                        for asset in project.assets.values().filter(|a| a.kind == wanted) {
                            let on = chosen == Some(asset.id);
                            if kit::menu_row(ui, p, None, &asset.name, "", false, Some(on))
                                .clicked()
                            {
                                picked = Some(asset.id);
                            }
                        }
                        picked
                    });
                if let Some(id) = picked.flatten() {
                    chosen = Some(id);
                }
            });
            if chosen != effect.asset_id {
                edit(&|effect| effect.asset_id = chosen);
            }
        }
        edited
    }
}

/// The two colours a look leaves in the shadows and in the lights, as the
/// library's look cards show them (`browser::look_colours`).
fn look_colours(preset: LookPreset) -> (egui::Color32, egui::Color32) {
    let rgb = egui::Color32::from_rgb;
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

/// The narrowest a look card is: room for its name under its swatch.
const LOOK_CARD: f32 = 76.0;
const LOOK_GAP: f32 = 6.0;

/// The looks as the library shows them, a swatch over a name, as many to a
/// row as fit, at most four. The one the clip has is pressed, with a check
/// on its swatch. Each is a button named by its look, said to be chosen or
/// not, and `hint` is its tooltip. Hands back the look clicked.
fn look_cards(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    given: Option<LookPreset>,
    hint: impl Fn(bool) -> &'static str,
) -> Option<LookPreset> {
    use egui::{Rect, Sense, Stroke, Vec2, WidgetInfo, WidgetType, pos2, vec2};
    super::rule(ui, p);
    let inner = ui.available_width() - 2.0 * super::LEAD;
    let columns = (((inner + LOOK_GAP) / (LOOK_CARD + LOOK_GAP)).floor() as usize).clamp(2, 4);
    let cell = (inner - LOOK_GAP * (columns - 1) as f32) / columns as f32;
    let swatch = vec2(cell - 8.0, ((cell - 8.0) * 32.0 / 56.0).round());
    let cell_height = 4.0 + swatch.y + 4.0 + 16.0 + 4.0;
    let rows = PRESETS.len().div_ceil(columns);
    let height = 8.0 + rows as f32 * cell_height + (rows - 1) as f32 * LOOK_GAP + 8.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let enabled = ui.is_enabled();
    let mut picked = None;
    for (index, (preset, name)) in PRESETS.into_iter().enumerate() {
        let (row, column) = (index / columns, index % columns);
        let min = pos2(
            rect.left() + super::LEAD + column as f32 * (cell + LOOK_GAP),
            rect.top() + 8.0 + row as f32 * (cell_height + LOOK_GAP),
        );
        let cell_rect = Rect::from_min_size(min, vec2(cell, cell_height));
        let chosen = given == Some(preset);
        let response = ui.interact(cell_rect, ui.id().with(("look-card", name)), Sense::click());
        response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, chosen, name));
        if ui.is_rect_visible(cell_rect) {
            let painter = ui.painter();
            let hover = kit::hover_amount(ui, &response);
            if chosen || (enabled && response.is_pointer_button_down_on()) {
                painter.rect_filled(cell_rect, 8, p.pressed);
            } else if hover > 0.0 {
                painter.rect_filled(cell_rect, 8, p.hover.gamma_multiply(hover));
            }
            if response.has_focus() {
                super::controls::focus_ring(painter, cell_rect, 8, *p);
            }
            let tile = Rect::from_min_size(cell_rect.min + vec2(4.0, 4.0), swatch);
            look_swatch(painter, tile, preset);
            if chosen {
                // Over a picture, as a badge is: white on a dark disc.
                let centre = pos2(tile.right() - 10.0, tile.top() + 10.0);
                painter.circle_filled(centre, 8.0, egui::Color32::from_black_alpha(150));
                crate::icons::paint(
                    painter,
                    Rect::from_center_size(centre, Vec2::splat(12.0)),
                    Icon::Check,
                    egui::Color32::WHITE,
                );
            }
            painter.rect_stroke(
                tile,
                6,
                Stroke::new(1.0, p.canvas_edge()),
                egui::StrokeKind::Inside,
            );
            let ink = if enabled { p.fg } else { p.muted };
            let galley = super::controls::elided(
                painter,
                name,
                theme::regular(11.5),
                ink,
                cell_rect.width() - 8.0,
            );
            super::controls::galley_at(
                painter,
                pos2(tile.left(), tile.bottom() + 4.0 + 8.0),
                galley,
            );
        }
        let response = response
            .on_hover_text(hint(chosen))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            picked = Some(preset);
        }
    }
    picked
}

/// A look's swatch: its shadows' colour running into its lights'.
fn look_swatch(painter: &egui::Painter, rect: egui::Rect, preset: LookPreset) {
    const STEPS: usize = 8;
    let (dark, light) = look_colours(preset);
    for step in 0..STEPS {
        let from = rect.left() + rect.width() * step as f32 / STEPS as f32;
        let to = rect.left() + rect.width() * (step + 1) as f32 / STEPS as f32;
        let radius = egui::CornerRadius {
            nw: if step == 0 { 6 } else { 0 },
            sw: if step == 0 { 6 } else { 0 },
            ne: if step + 1 == STEPS { 6 } else { 0 },
            se: if step + 1 == STEPS { 6 } else { 0 },
        };
        // Half a point over the next, so no seam shows between two.
        let stripe = egui::Rect::from_min_max(
            egui::pos2(from, rect.top()),
            egui::pos2((to + 0.5).min(rect.right()), rect.bottom()),
        );
        let amount = step as f32 / (STEPS - 1) as f32;
        painter.rect_filled(stripe, radius, theme::mix(dark, light, amount));
    }
}

/// A part of a whole in hundredths, read beside the slider that sets it.
pub(super) fn percent(ui: &mut Ui, p: &theme::Palette, value: f32) {
    ui.add_sized(
        [40.0, 18.0],
        egui::Label::new(
            egui::RichText::new(format!("{:.0} %", value * 100.0))
                .font(theme::regular(12.5))
                .color(p.secondary),
        ),
    );
}

/// The width of a slider of a row whose figure is laid out, at most
/// `widest`: what the row leaves beside a short label.
pub(super) fn slider_width(ui: &Ui, widest: f32) -> f32 {
    (ui.available_width() - 72.0).clamp(48.0, widest)
}
