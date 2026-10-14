//! The selected transition: its kind, how long it lasts, its easing and the
//! values its kind has in `look::transition_spec`.
//!
//! Its rows write the clip's draft as the rest of the inspector does, so a
//! scrubbed length is one undo step and an Escape gives it up. Removing it
//! and keeping it as its kind's default are [`Action`]s.
use super::{InspectorInput, InspectorState, card, clip::FIELD, look::is_default, title};
use crate::{
    icons::Icon,
    rules, theme,
    ui::{
        Action, ClipDraft, EASINGS, transitions,
        widgets::{self, Kind, row, section},
    },
};
use editor_core::{
    Clip, Ease, ParamKey, ParamSpec, ParamValue, Time, Track, Transition, TransitionEdge, look,
};
use eframe::egui::{self, Ui, WidgetInfo, WidgetType};

/// The easings a transition is offered by name: the ones that take it from
/// one clip to the other without stopping short or running past.
const OFFERED: [&str; 4] = ["Linear", "Ease in", "Ease out", "Ease in and out"];

fn ease_name(ease: Ease) -> &'static str {
    (EASINGS.iter())
        .find(|(name, listed)| *listed == ease && OFFERED.contains(name))
        .map_or("Custom", |(name, _)| name)
}

/// What a value of a transition is called.
fn param_name(key: ParamKey) -> &'static str {
    match key {
        ParamKey::Softness => "Softness",
        ParamKey::Radius => "Blur",
        _ => "Amount",
    }
}

fn set_param(transition: &mut Transition, key: ParamKey, value: f32) {
    match transition.params.iter_mut().find(|param| param.key == key) {
        Some(param) => param.value = value,
        None => transition.params.push(ParamValue { key, value }),
    }
}

/// One value of the kind, in hundredths, with a reset while it is not the
/// kind's own.
fn param_row(ui: &mut Ui, p: &theme::Palette, transition: &mut Transition, spec: &ParamSpec) {
    let name = param_name(spec.key);
    let stored = transition.param(spec.key).unwrap_or(spec.default);
    row(ui, p, name, |ui| {
        let span = (spec.max - spec.min) * 100.0;
        let mut shown = stored * 100.0;
        let field = widgets::named_number(
            ui,
            &mut shown,
            spec.min * 100.0..=spec.max * 100.0,
            f64::from(span / 300.0).max(0.001),
            " %",
            usize::from(span < 100.0),
            FIELD,
            &format!("Transition {}", name.to_lowercase()),
        );
        if field.changed() {
            set_param(
                transition,
                spec.key,
                (shown / 100.0).clamp(spec.min, spec.max),
            );
        }
        let reset = format!("Reset {}", name.to_lowercase());
        if !is_default(stored, spec)
            && widgets::small_icon_button(ui, p, Icon::Rotate, &reset).clicked()
        {
            transition.params.retain(|param| param.key != spec.key);
        }
    });
}

#[expect(clippy::too_many_arguments, reason = "one call site in the frame")]
pub(super) fn pane(
    ui: &mut Ui,
    input: &InspectorInput,
    track: &Track,
    clip: &Clip,
    edge: TransitionEdge,
    state: &mut InspectorState,
    draft: &mut Option<ClipDraft>,
    actions: &mut Vec<Action>,
) {
    let (p, project) = (input.p, input.project);
    let Some(stored) = transitions::of(clip, edge) else {
        return;
    };
    let before = draft
        .clone()
        .filter(|draft| draft.clip == clip.id)
        .unwrap_or_else(|| ClipDraft::of(clip));
    let mut next = before.clone();
    let slot = transitions::slot(edge);
    let held = (next.transitions.as_ref())
        .and_then(|both| both[slot].clone())
        .unwrap_or_else(|| stored.clone());
    let mut shown = held.clone();

    title(
        ui,
        p,
        Icon::Transition,
        p.accent,
        transitions::name(shown.kind),
        &match edge {
            TransitionEdge::In => format!("Start of {}", clip.name),
            TransitionEdge::Out => format!("End of {}", clip.name),
        },
    );
    if track.locked {
        ui.label(
            egui::RichText::new("This clip's track is locked.")
                .font(theme::regular(12.0))
                .color(p.muted),
        );
    }
    ui.add_enabled_ui(!track.locked, |ui| {
        section(ui, p, "Transition");
        card(ui, p, |ui| {
            widgets::fitted_row(ui, p, "Kind", |ui| {
                let mut kind = shown.kind;
                // Tall enough for every kind: none is under a scroll.
                egui::ComboBox::from_id_salt("transition-kind")
                    .icon(widgets::combo_icon)
                    .selected_text(transitions::name(kind))
                    .width(118.0)
                    .height(540.0)
                    .show_ui(ui, |ui| {
                        for listed in transitions::KINDS {
                            ui.selectable_value(&mut kind, listed, transitions::name(listed));
                        }
                    })
                    .response
                    .widget_info(|| {
                        WidgetInfo::labeled(WidgetType::ComboBox, true, "Transition kind")
                    });
                if kind != shown.kind {
                    shown = transitions::with_kind(&shown, kind);
                }
            });
            if let Some((shortest, longest)) = transitions::lengths(project, clip) {
                row(ui, p, "Duration", |ui| {
                    let mut seconds = shown.duration.as_seconds() as f32;
                    let range = shortest.as_seconds() as f32..=longest.as_seconds() as f32;
                    let field = widgets::named_number(
                        ui,
                        &mut seconds,
                        range,
                        0.01,
                        " s",
                        2,
                        FIELD,
                        "Transition duration",
                    );
                    if field.changed() {
                        let wanted = Time::from_seconds(f64::from(seconds));
                        shown.duration = rules::quantize(wanted, project.settings.frame_rate)
                            .clamp(shortest, longest);
                    }
                });
            }
            widgets::fitted_row(ui, p, "Easing", |ui| {
                let mut ease = shown.ease;
                egui::ComboBox::from_id_salt("transition-easing")
                    .icon(widgets::combo_icon)
                    .selected_text(ease_name(ease))
                    .width(118.0)
                    .show_ui(ui, |ui| {
                        for (name, listed) in EASINGS {
                            if OFFERED.contains(&name) {
                                ui.selectable_value(&mut ease, listed, name);
                            }
                        }
                    })
                    .response
                    .widget_info(|| {
                        WidgetInfo::labeled(WidgetType::ComboBox, true, "Transition easing")
                    });
                shown.ease = ease;
            });
            for spec in look::transition_spec(shown.kind) {
                param_row(ui, p, &mut shown, spec);
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, p, Some(Icon::Trash), "Remove", Kind::Plain)
                .on_hover_text("Remove this transition (Delete)")
                .clicked()
            {
                actions.push(Action::Edit(transitions::set_command(clip.id, edge, None)));
            }
            let keep = format!(
                "New {} transitions start with this length, easing and values",
                transitions::name(shown.kind).to_lowercase()
            );
            if widgets::button(ui, p, None, "Save as default", Kind::Plain)
                .on_hover_text(keep)
                .clicked()
            {
                actions.push(Action::SaveTransitionDefault(shown.clone()));
            }
        });
    });

    if shown != held {
        let stored = [clip.transition_in.clone(), clip.transition_out.clone()];
        let mut both = next.transitions.take().unwrap_or_else(|| stored.clone());
        both[slot] = Some(shown);
        // Back at the stored ones is no draft.
        next.transitions = (both != stored).then_some(both);
    }
    // What an interaction an Escape gave up still writes is dropped.
    if next != before && !state.cancelled {
        *draft = Some(next);
    }
}
