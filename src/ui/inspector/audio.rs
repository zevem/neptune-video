//! The Audio section of a clip on an audio track: how it plays, and its
//! audio effects.
//!
//! The Playback card is the one a sound had before it had a section. Under
//! it is the Ducking card, then a card for each audio effect in the order the
//! sound goes through them, with rows made from `look::spec` as the Look
//! section's are, and no diamond: the project keys no audio effect's values.
//! A change to the stack is held in the draft and committed as one
//! `SetEffects`; a preset is one step of its own, made in `crate::audio` as
//! the library and the command panel make it.
use super::{
    FIELD, button, button_row, card,
    clip::Pane,
    keyed_card,
    look::{Reorder, effect_name, set_param},
    motion::bar,
    note, number, row, section,
};
use crate::{
    audio::{self, Given, Preset},
    duck,
    icons::Icon,
    ui::{
        Action, fades,
        widgets::{self, Kind},
    },
};
use editor_core::{AnimTarget, Effect, look};
use eframe::egui::{self, Ui};

impl Pane<'_> {
    pub(super) fn audio(&mut self, ui: &mut Ui) {
        self.sound(ui);
        if fades::has_sound(self.project, self.clip) {
            self.ducking(ui);
        }
        // A sound takes audio effects; a stack written on a clip without one
        // is still shown, to be taken away.
        if fades::has_sound(self.project, self.clip) || !self.clip.effects.is_empty() {
            self.audio_effects(ui);
        }
    }

    /// Ducks the clip under the other clips by the depth above the button,
    /// as the clip menu does at its default. Shown and not to be chosen,
    /// with the reason on hover, for a clip that would not be ducked. While
    /// a duck of the clip reads the others' sound, how far it has got and
    /// the way to stop it are in their place.
    fn ducking(&mut self, ui: &mut Ui) {
        let p = self.p;
        section(ui, p, "Ducking");
        card(ui, p, |ui| {
            let under_way = (self.state.ducking.as_ref())
                .filter(|run| run.clips.contains(&self.clip.id))
                .cloned();
            if let Some(run) = under_way {
                let title = match &run.reading {
                    Some(name) => format!("Reading {name}, {} of {}", run.taken + 1, run.total),
                    None => "Writing the volume keys".to_owned(),
                };
                let done = run.taken as f32 / run.total.max(1) as f32;
                bar(ui, p, &title, "Ducking", done);
                button_row(ui, p, |ui| {
                    if button(ui, p, None, "Stop ducking", Kind::Plain).clicked() {
                        self.actions.push(Action::CancelDuck);
                    }
                });
                return;
            }
            row(ui, p, "Depth", |ui| {
                let range = -duck::DEEPEST_DB..=-duck::SHALLOWEST_DB;
                let depth = &mut self.state.duck_db;
                number(ui, p, depth, range, 0.5, " dB", 0, FIELD, "Depth")
                    .on_hover_text("How far the clip is lowered where other clips are heard");
            });
            let refused = duck::refusal(self.project, self.clip.id);
            button_row(ui, p, |ui| {
                ui.add_enabled_ui(refused.is_none(), |ui| {
                    let button = button(ui, p, Some(Icon::Volume), duck::LABEL, Kind::Plain)
                        .on_hover_text(
                            "Lower the clip wherever other clips are heard, as volume keyframes",
                        )
                        .on_disabled_hover_text(refused.as_deref().unwrap_or_default());
                    if button.clicked() {
                        let depth = duck::depth_of_db(self.state.duck_db).unwrap_or(duck::DEPTH);
                        self.actions.push(Action::Duck {
                            clips: vec![self.clip.id],
                            depth,
                        });
                    }
                });
            });
            note(
                ui,
                p,
                "Any sound on the other clips counts, not only speech. Written once: run it \
                 again when they move.",
            );
        });
    }

    fn audio_effects(&mut self, ui: &mut Ui) {
        let p = self.p;
        let clip = self.clip;
        // The stack is read where it is and copied only when it changes.
        let held = self.next.effects.take();
        let list: &[Effect] = held.as_deref().unwrap_or(&clip.effects);
        let mut edited: Vec<(usize, Effect)> = Vec::new();
        let mut reorder: Option<Reorder> = None;
        let mut preset: Option<Preset> = None;

        section(ui, p, "Audio effects");
        for (index, effect) in list.iter().enumerate() {
            ui.push_id(effect.id, |ui| {
                // Its rows keep the key column clear, with no diamond in it:
                // the fields of every card end at one place.
                keyed_card(ui, p, |ui| {
                    if let Some(effect) =
                        self.audio_effect_rows(ui, effect, index, list.len(), &mut reorder)
                    {
                        edited.push((index, effect));
                    }
                });
            });
        }
        if list.is_empty() {
            card(ui, p, |ui| note(ui, p, "No audio effects on this clip."));
        }
        button_row(ui, p, |ui| {
            ui.add_enabled_ui(list.len() < look::MAX_EFFECTS, |ui| {
                let add = button(ui, p, Some(Icon::Plus), "Add audio effect", Kind::Plain)
                    .on_disabled_hover_text("A clip holds sixteen effects at most");
                egui::Popup::menu(&add).show(|ui| {
                    widgets::menu_style(ui, 196.0);
                    // What a voice or a mix most often wants first, then
                    // each kind by itself. Pitch is the Playback card's.
                    for listed in Preset::ALL {
                        if widgets::menu_item(ui, p, None, listed.name(), "").clicked() {
                            preset = Some(listed);
                        }
                    }
                    widgets::menu_separator(ui, p);
                    // A kind this FFmpeg does not run is offered as every
                    // route offers it; its card says so once it is added.
                    for kind in audio::added() {
                        if widgets::menu_item(ui, p, None, effect_name(kind), "").clicked() {
                            reorder = Some(Reorder::Add(kind));
                        }
                    }
                });
            });
        });

        // A preset is a step of its own, made of the stack as it is shown.
        if let Some(preset) = preset {
            match audio::with_given(list, Given::Preset(preset)) {
                Ok(effects) => {
                    // Its first effect is the one whose rows are open.
                    self.state.effect = effects.get(list.len()).map(|effect| effect.id);
                    (self.actions).push(Action::Edit(audio::preset_command(
                        clip.id, preset, effects,
                    )));
                }
                Err(reason) => self.actions.push(Action::Notify(reason)),
            }
        }
        let restacked = self.restack(list, edited, reorder);
        self.next.effects = restacked.unwrap_or(held);
    }

    /// One audio effect of the stack: its heading, a line when the session's
    /// FFmpeg does not run it, and its rows while it is open, disabled then.
    /// Hands back the effect as edited, when it was.
    fn audio_effect_rows(
        &mut self,
        ui: &mut Ui,
        effect: &Effect,
        index: usize,
        count: usize,
        reorder: &mut Option<Reorder>,
    ) -> Option<Effect> {
        let mut edited: Option<Effect> = None;
        let (open, switched) =
            self.effect_head(ui, effect, index, count, reorder, (Icon::Volume, "Hear"));
        if switched {
            let mut effect = effect.clone();
            effect.enabled = !effect.enabled;
            edited = Some(effect);
        }
        let runs = !audio::unavailable(effect.kind, self.missing);
        if !runs {
            note(ui, self.p, audio::UNAVAILABLE);
        }
        if !open {
            return edited;
        }
        let target = AnimTarget::Effect(effect.kind);
        ui.add_enabled_ui(runs, |ui| {
            for spec in look::spec(effect.kind) {
                let stored = effect.param(spec.key).unwrap_or(spec.default);
                if let Some(value) = self.plain_param_row(ui, effect.id, target, spec, stored) {
                    let effect = edited.get_or_insert_with(|| effect.clone());
                    set_param(effect, spec.key, value);
                }
            }
        });
        edited
    }
}
