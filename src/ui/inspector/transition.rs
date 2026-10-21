//! The selected transition: its kind, how long it lasts, its easing and the
//! values its kind has in `look::transition_spec`.
//!
//! Its rows write the clip's draft as the rest of the inspector does, so a
//! scrubbed length is one undo step and an Escape gives it up. Removing it
//! and keeping it as its kind's default are [`Action`]s.
use super::{
    FIELD, InspectorInput, InspectorState, Thumb, button, button_row, card, look::is_default, note,
    number, object_header, row, section, select_list, select_popup, tall_row,
};
use crate::{
    icons::Icon,
    rules, theme,
    ui::{Action, ClipDraft, EASINGS, kit, transitions, widgets::Kind},
};
use editor_core::{
    Clip, Ease, ParamKey, ParamSpec, ParamValue, Time, Track, Transition, TransitionEdge, look,
};
use eframe::egui::{Rect, Ui};

/// The width of the selects of a transition's card.
const SELECT: f32 = 132.0;

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
        let field = number(
            ui,
            p,
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
            && kit::small_icon_button(ui, p, Icon::Rotate, &reset).clicked()
        {
            transition.params.retain(|param| param.key != spec.key);
        }
    });
}

/// The header of the transition at `edge` of `clip` in `rect`: its kind,
/// and which end of which clip it is at.
pub(super) fn head(
    ui: &mut Ui,
    input: &InspectorInput,
    rect: Rect,
    clip: &Clip,
    edge: TransitionEdge,
    compact: bool,
) {
    let p = input.p;
    let Some(stored) = transitions::of(clip, edge) else {
        return;
    };
    let words = match edge {
        TransitionEdge::In => format!("Start of {}", clip.name),
        TransitionEdge::Out => format!("End of {}", clip.name),
    };
    object_header(
        ui,
        p,
        rect,
        Thumb::Glyph(Icon::Transition, p.secondary),
        transitions::name(stored.kind),
        (&words, ""),
        compact,
    );
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

    if track.locked {
        note(ui, p, "This clip's track is locked.");
    }
    ui.add_enabled_ui(!track.locked, |ui| {
        section(ui, p, "Transition");
        card(ui, p, |ui| {
            tall_row(ui, p, "Kind", |ui| {
                let width = ui.available_width().min(SELECT);
                // Every kind is listed: none is under a scroll.
                let (_, chosen) = select_popup(
                    ui,
                    p,
                    ("transition-kind", "Transition kind"),
                    transitions::name(shown.kind),
                    width,
                    false,
                    |ui| {
                        let mut chosen = None;
                        for listed in transitions::KINDS {
                            let on = listed == shown.kind;
                            let name = transitions::name(listed);
                            if kit::menu_row(ui, p, None, name, "", false, Some(on)).clicked() {
                                chosen = Some(listed);
                            }
                        }
                        chosen
                    },
                );
                if let Some(kind) = chosen.flatten()
                    && kind != shown.kind
                {
                    shown = transitions::with_kind(&shown, kind);
                }
            });
            if let Some((shortest, longest)) = transitions::lengths(project, clip) {
                row(ui, p, "Duration", |ui| {
                    let mut seconds = shown.duration.as_seconds() as f32;
                    let range = shortest.as_seconds() as f32..=longest.as_seconds() as f32;
                    let field = number(
                        ui,
                        p,
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
            tall_row(ui, p, "Easing", |ui| {
                let width = ui.available_width().min(SELECT);
                let mut ease = shown.ease;
                let offered: Vec<(Ease, &str)> = (EASINGS.iter())
                    .filter(|(name, _)| OFFERED.contains(name))
                    .map(|(name, listed)| (*listed, *name))
                    .collect();
                if offered.iter().any(|(listed, _)| *listed == ease) {
                    select_list(
                        ui,
                        p,
                        "transition-easing",
                        "Transition easing",
                        &mut ease,
                        &offered,
                        width,
                    );
                } else {
                    // An easing the list does not name is shown as custom.
                    let (_, chosen) = select_popup(
                        ui,
                        p,
                        ("transition-easing", "Transition easing"),
                        ease_name(ease),
                        width,
                        false,
                        |ui| {
                            let mut chosen = None;
                            for (listed, name) in &offered {
                                if kit::menu_row(ui, p, None, name, "", false, Some(false))
                                    .clicked()
                                {
                                    chosen = Some(*listed);
                                }
                            }
                            chosen
                        },
                    );
                    if let Some(listed) = chosen.flatten() {
                        ease = listed;
                    }
                }
                shown.ease = ease;
            });
            for spec in look::transition_spec(shown.kind) {
                param_row(ui, p, &mut shown, spec);
            }
        });
        button_row(ui, p, |ui| {
            if button(ui, p, Some(Icon::Trash), "Remove", Kind::Plain)
                .on_hover_text("Remove this transition (Delete)")
                .clicked()
            {
                actions.push(Action::Edit(transitions::set_command(clip.id, edge, None)));
            }
            let keep = format!(
                "New {} transitions start with this length, easing and values",
                transitions::name(shown.kind).to_lowercase()
            );
            if button(ui, p, None, "Save as default", Kind::Plain)
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
