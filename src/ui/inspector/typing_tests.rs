//! Values typed into the inspector's fields, one key a frame as a person
//! types them. A field keeps the keyboard from the first character to the
//! Return whatever the row gains or loses on the way (a reset, a relayout),
//! and what was typed is one step.
use super::*;

/// Gives the field with this name the keyboard: the field, not a tab or a
/// label of the same name.
fn focus_field(bench: &mut Bench, name: &str) {
    bench.frame(Vec::new());
    let target_node = (bench.seen.iter())
        .find(|seen| seen.name == name && seen.role != Role::RadioButton && !Bench::is_text(seen))
        .unwrap_or_else(|| panic!("FIXTURE: no field is named \"{name}\""))
        .node;
    bench.frame(vec![Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Focus,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node,
            data: None,
        },
    )]);
    bench.frame(Vec::new());
}

/// Types `text` into the field a character a frame, without leaving it.
fn strike(bench: &mut Bench, name: &str, text: &str) {
    focus_field(bench, name);
    for character in text.chars() {
        bench.frame(vec![Event::Text(character.to_string())]);
    }
}

/// Types `text` into the field and leaves it with Return.
fn typed(bench: &mut Bench, name: &str, text: &str) {
    strike(bench, name, text);
    bench.enter(Modifiers::NONE);
}

fn near(value: Option<f32>, wanted: f32) -> bool {
    value.is_some_and(|value| (value - wanted).abs() < 1e-4)
}

#[test]
fn an_effects_value_typed_past_its_default_keeps_every_digit() {
    let (mut bench, clip, _) = with_effect(EffectKind::Glow);
    let before = bench.steps();
    // The first digit takes the value off its default: the row gains a reset.
    typed(&mut bench, "Amount", "125");
    let amount = bench.clip(clip).effects[0].param(ParamKey::Amount);
    assert!(near(amount, 1.25), "typed 125 %, stored {amount:?}");
    assert_eq!(bench.steps(), before + 1, "one typed value is one step");
    assert!(bench.has("Reset amount"));

    // And back onto it: the row loses the reset part way.
    typed(&mut bench, "Amount", "50");
    let amount = bench.clip(clip).effects[0].param(ParamKey::Amount);
    assert!(near(amount, 0.5), "typed 50 %, stored {amount:?}");
    assert_eq!(bench.steps(), before + 2);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_grade_value_typed_from_its_default_keeps_every_digit() {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Color;
    let before = bench.steps();
    typed(&mut bench, "Saturation", "140");
    let grade = &bench.clip(clip).effects[0];
    assert_eq!(grade.kind, EffectKind::Grade);
    let saturation = grade.param(ParamKey::Saturation);
    assert!(near(saturation, 1.4), "typed 140 %, stored {saturation:?}");
    assert_eq!(
        bench.steps(),
        before + 1,
        "the grade and its value: one step"
    );

    typed(&mut bench, "Contrast", "20");
    let contrast = bench.clip(clip).effects[0].param(ParamKey::Contrast);
    assert!(near(contrast, 0.2), "typed 20 %, stored {contrast:?}");
    typed(&mut bench, "Tint", "-10");
    let tint = bench.clip(clip).effects[0].param(ParamKey::Tint);
    assert!(near(tint, -0.1), "typed -10 %, stored {tint:?}");
    typed(&mut bench, "Exposure", "0.5");
    let exposure = bench.clip(clip).effects[0].param(ParamKey::Exposure);
    assert!(near(exposure, 0.5), "typed 0.5 EV, stored {exposure:?}");
    assert_eq!(bench.steps(), before + 4);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_hue_bands_value_typed_from_its_default_keeps_every_digit() {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Color;
    bench.click("Add hue band");
    bench.click("Greens");
    let before = bench.steps();
    typed(&mut bench, "Hue shift", "45");
    let shift = bench.clip(clip).effects[0].param(ParamKey::HueShift);
    assert!(near(shift, 45.0), "typed 45°, stored {shift:?}");
    typed(&mut bench, "Saturation shift", "25");
    let shift = bench.clip(clip).effects[0].param(ParamKey::SatShift);
    assert!(near(shift, 0.25), "typed 25 %, stored {shift:?}");
    assert_eq!(bench.steps(), before + 2);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_masks_value_typed_from_its_default_keeps_every_digit() {
    let (mut bench, clip) = Bench::new();
    bench.click("Add mask");
    bench.click("Ellipse");
    let before = bench.steps();
    typed(&mut bench, "Feather", "20");
    let feather = bench.clip(clip).masks[0].feather;
    assert!(near(Some(feather), 0.2), "typed 20 %, stored {feather}");
    assert_eq!(bench.steps(), before + 1);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_speed_typed_with_its_point_keeps_every_character() {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Speed;
    let before = bench.steps();
    typed(&mut bench, "Speed", "2.5");
    assert_eq!(bench.clip(clip).playback_rate, 2.5);
    assert_eq!(bench.steps(), before + 1);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_transform_value_typed_keeps_every_digit() {
    let (mut bench, clip) = Bench::new();
    let before = bench.steps();
    typed(&mut bench, "Rotation", "145");
    assert_eq!(bench.clip(clip).transform.rotation, 145.0);
    typed(&mut bench, "Opacity", "35");
    let opacity = bench.clip(clip).transform.opacity;
    assert!(near(Some(opacity), 0.35), "typed 35 %, stored {opacity}");
    assert_eq!(bench.steps(), before + 2);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

// ---- A value being typed is a draft until the field is left ---------------

#[test]
fn a_value_typed_with_pauses_is_one_step_when_the_field_is_left() {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Color;
    let before = bench.steps();
    focus_field(&mut bench, "Contrast");
    for character in ["4", "7"] {
        bench.frame(vec![Event::Text(character.to_owned())]);
        // Longer than any rest: the field still has the keyboard.
        bench.rest(1.5);
        assert_eq!(bench.steps(), before, "a value being typed is no step");
        assert!(bench.draft.is_some(), "it is held");
    }
    bench.enter(Modifiers::NONE);
    assert_eq!(bench.steps(), before + 1, "the Return is the one step");
    let contrast = bench.clip(clip).effects[0].param(ParamKey::Contrast);
    assert!(near(contrast, 0.47), "typed 47 %, stored {contrast:?}");
    assert!(bench.editor.undo());
    assert!(
        bench.clip(clip).effects.is_empty(),
        "and one undo takes it back"
    );
}

#[test]
fn an_escape_after_a_pause_gives_up_all_that_was_typed() {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Color;
    let before = bench.steps();
    focus_field(&mut bench, "Contrast");
    bench.frame(vec![Event::Text("4".to_owned())]);
    bench.rest(1.5);
    bench.frame(vec![Event::Text("7".to_owned())]);
    bench.rest(1.5);
    bench.key(Key::Escape, Modifiers::NONE);
    bench.frame(Vec::new());
    bench.rest(1.5);
    assert_eq!(bench.steps(), before, "nothing typed was kept");
    assert!(bench.draft.is_none());
    assert!(bench.clip(clip).effects.is_empty());
}

#[test]
fn a_drag_made_while_another_field_has_the_keyboard_commits_when_let_go() {
    let (mut bench, clip) = Bench::new();
    let before = bench.steps();
    // The keyboard stays in the field a press elsewhere does not click.
    focus_field(&mut bench, "Rotation");
    let end = bench.drag("Opacity", vec2(-40.0, 0.0));
    assert_eq!(bench.steps(), before, "held under the pointer");
    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "the drag is its own step");
    assert!(bench.clip(clip).transform.opacity < 1.0);
}
