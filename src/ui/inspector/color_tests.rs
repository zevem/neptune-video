//! The Color section, driven as the rest of the inspector is in the tests
//! above: each interaction for the command it makes, its undo and its cancel.
use super::super::{
    color::{background_command, place},
    curve, key_colour_command,
};
use super::*;
use crate::{compose::pass, ui::ColourPick};
use editor_core::ParamValue;

/// A bench on the Color section of its clip.
fn colour() -> (Bench, ClipId) {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Color;
    (bench, clip)
}

fn kinds(bench: &Bench, clip: ClipId) -> Vec<EffectKind> {
    (bench.clip(clip).effects.iter())
        .map(|effect| effect.kind)
        .collect()
}

fn stack(bench: &mut Bench, clip: ClipId, effects: Vec<Effect>) {
    bench.edit(Command::SetEffects {
        clip_id: clip,
        effects,
    });
}

fn with_params(kind: EffectKind, params: &[(ParamKey, f32)]) -> Effect {
    let mut effect = Effect::new(kind);
    effect.params = (params.iter())
        .map(|&(key, value)| ParamValue { key, value })
        .collect();
    effect
}

fn curved(kind: EffectKind, points: &[[f32; 2]]) -> Effect {
    let mut effect = Effect::new(kind);
    effect.points = points.to_vec();
    effect
}

/// A press and a release at one place, with nothing between them.
fn tap(bench: &mut Bench, pos: Pos2) {
    bench.frame(vec![Event::PointerMoved(pos)]);
    bench.frame(vec![Bench::button(pos, PointerButton::Primary, true)]);
    bench.frame(vec![Bench::button(pos, PointerButton::Primary, false)]);
}

// ---- Grade ----------------------------------------------------------------

#[test]
fn a_grade_is_made_by_the_first_touch_as_one_step_and_undoes() {
    let (mut bench, clip) = colour();
    let before = bench.steps();
    let stored = bench.editor.snapshot();
    let end = bench.drag("Exposure", vec2(60.0, 0.0));

    // Held: the project is as it was, and the canvas shows the grade.
    assert_eq!(bench.steps(), before);
    assert_eq!(*bench.editor.project(), *stored);
    let held = bench.layer(clip).effects;
    assert_eq!(held.len(), 1, "one grade for the whole drag");
    assert_eq!(held[0].kind, EffectKind::Grade);
    let exposure = held[0].param(ParamKey::Exposure);
    assert!(exposure.is_some_and(|ev| ev > 0.0), "{exposure:?}");
    let id = held[0].id;

    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "the whole drag is one step");
    assert_eq!(bench.last(), "Change effects");
    let grade = &bench.clip(clip).effects[0];
    assert_eq!((grade.id, grade.param(ParamKey::Exposure)), (id, exposure));
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);

    assert!(bench.editor.undo());
    assert!(bench.clip(clip).effects.is_empty());
}

#[test]
fn an_escape_gives_up_a_grade_being_made() {
    let (mut bench, clip) = colour();
    let before = bench.steps();
    let end = bench.drag("Contrast", vec2(50.0, 0.0));
    assert!(bench.draft.is_some());
    bench.key(Key::Escape, Modifiers::NONE);
    assert!(bench.draft.is_none());
    bench.move_to(end + vec2(20.0, 0.0));
    assert!(bench.layer(clip).effects.is_empty(), "nothing is previewed");
    bench.release(end + vec2(20.0, 0.0));
    assert_eq!(bench.steps(), before);
    assert!(bench.clip(clip).effects.is_empty());
}

#[test]
fn a_grade_back_where_it_changes_nothing_is_taken_out() {
    let (mut bench, clip) = colour();
    let grade = with_params(EffectKind::Grade, &[(ParamKey::Exposure, 0.5)]);
    stack(
        &mut bench,
        clip,
        vec![grade.clone(), Effect::new(EffectKind::Blur)],
    );
    let before = bench.steps();
    bench.click("Reset exposure");
    assert_eq!(kinds(&bench, clip), [EffectKind::Blur]);
    assert_eq!(bench.steps(), before + 1);
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).effects[0], grade);

    // The whole of it at once, whatever it holds.
    let mut grade = with_params(
        EffectKind::Grade,
        &[(ParamKey::Tint, 0.2), (ParamKey::Gamma, 1.4)],
    );
    grade.mix = 0.5;
    stack(&mut bench, clip, vec![grade]);
    bench.click("Reset grade");
    assert!(bench.clip(clip).effects.is_empty());

    // One that is hidden is shown again from here, and is not taken out
    // for being hidden.
    let mut hidden = with_params(EffectKind::Grade, &[(ParamKey::Tint, 0.2)]);
    hidden.enabled = false;
    stack(&mut bench, clip, vec![hidden]);
    bench.click("Show grade");
    assert!(bench.clip(clip).effects[0].enabled);
}

#[test]
fn an_effect_made_here_goes_after_the_keys_and_ahead_of_the_look() {
    use EffectKind::{Blur, ChromaKey, CurveMaster, CurveRed, Glow, Grade, HslBand, Lut, Vignette};
    let of = |kinds: &[EffectKind]| -> Vec<Effect> {
        kinds.iter().map(|kind| Effect::new(*kind)).collect()
    };
    assert_eq!(place(&[], Grade), 0);
    assert_eq!(place(&of(&[Blur, Vignette]), Grade), 0);
    assert_eq!(place(&of(&[ChromaKey, Vignette]), Grade), 1);
    assert_eq!(place(&of(&[Grade, CurveMaster, Glow]), ChromaKey), 0);
    assert_eq!(place(&of(&[Grade, Lut, Glow]), HslBand), 1);
    assert_eq!(place(&of(&[Grade, CurveMaster, Glow]), CurveRed), 2);
    assert_eq!(place(&of(&[ChromaKey, Grade, Glow]), Lut), 2);
    assert_eq!(place(&of(&[Glow, Grade]), Lut), 2);

    let (mut bench, clip) = colour();
    stack(&mut bench, clip, of(&[ChromaKey, Vignette]));
    // The chroma key's rows are above; the grade's exposure is the first.
    let end = bench.drag("Exposure", vec2(40.0, 0.0));
    bench.release(end);
    assert_eq!(kinds(&bench, clip), [ChromaKey, Grade, Vignette]);
}

#[test]
fn a_keyframe_on_a_grade_that_is_not_there_makes_both_in_one_step() {
    let (mut bench, clip) = colour();
    let before = bench.steps();
    bench.click("Animate exposure");
    assert_eq!(bench.steps(), before + 1, "the effect and its keyframe");
    assert_eq!(bench.last(), "Add keyframe");
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
    let grade = bench.clip(clip).effects[0].clone();
    assert_eq!(grade.kind, EffectKind::Grade);
    let track = &bench.clip(clip).animation[0];
    assert_eq!(
        (track.target, track.key, track.keys.len()),
        (Some(grade.id), ParamKey::Exposure, 1)
    );
    assert_eq!(track.keys[0].at, seconds(1.0));

    // It changes nothing as it is set, and stays: keyframes drive it.
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let end = bench.drag("Contrast", vec2(30.0, 0.0));
    bench.release(end);
    bench.click("Reset contrast");
    assert_eq!(kinds(&bench, clip), [EffectKind::Grade]);

    while bench.steps() > before {
        assert!(bench.editor.undo());
    }
    assert!(bench.clip(clip).effects.is_empty());
    assert!(bench.clip(clip).animation.is_empty());
}

// ---- Hue bands --------------------------------------------------------------

#[test]
fn a_hue_band_is_added_chosen_edited_and_removed() {
    let (mut bench, clip) = colour();
    let before = bench.steps();
    bench.click("Add hue band");
    bench.click("Greens");
    assert_eq!(kinds(&bench, clip), [EffectKind::HslBand]);
    assert_eq!(bench.steps(), before + 1);
    let green = bench.clip(clip).effects[0].clone();
    assert_eq!(green.param(ParamKey::Hue), Some(120.0));
    assert_eq!(bench.state.band, Some(green.id), "the new band is open");

    let end = bench.drag("Hue shift", vec2(40.0, 0.0));
    let held = bench.layer(clip).effects[0].param(ParamKey::HueShift);
    assert!(held.is_some_and(|shift| shift > 0.0));
    assert_eq!(
        bench.clip(clip).effects[0].param(ParamKey::HueShift),
        Some(0.0)
    );
    bench.release(end);
    assert_eq!(bench.steps(), before + 2);
    assert_eq!(bench.clip(clip).effects[0].param(ParamKey::HueShift), held);

    bench.click("Add hue band");
    bench.click("Blues");
    let blue = bench.clip(clip).effects[1].clone();
    assert_eq!(blue.param(ParamKey::Hue), Some(240.0));
    assert_eq!(bench.state.band, Some(blue.id));
    bench.frame(Vec::new());
    assert!(bench.has("Hue band 1, greens") && bench.has("Hue band 2, blues"));

    // The band chosen is the one edited and the one removed.
    bench.click("Hue band 1, greens");
    assert_eq!(bench.state.band, Some(green.id));
    let steps = bench.steps();
    bench.click("Remove hue band");
    assert_eq!(bench.steps(), steps + 1);
    assert_eq!(bench.clip(clip).effects, vec![blue.clone()]);
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).effects.len(), 2);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

// ---- Curves -----------------------------------------------------------------

fn plot(bench: &mut Bench, name: &str) -> Rect {
    bench.frame(Vec::new());
    bench
        .find(name)
        .unwrap_or_else(|| panic!("FIXTURE: no plot is named \"{name}\""))
        .rect
}

#[test]
fn a_press_on_the_plot_adds_a_point_which_is_dragged_and_removed() {
    let (mut bench, clip) = colour();
    let before = bench.steps();
    let area = plot(&mut bench, "Master curve");
    // Above the line, a third of the way along.
    let at = Pos2::new(
        area.left() + area.width() * 0.33,
        area.top() + area.height() * 0.4,
    );
    tap(&mut bench, at);
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(kinds(&bench, clip), [EffectKind::CurveMaster]);
    let points = bench.clip(clip).effects[0].points.clone();
    assert_eq!(points.len(), 3);
    assert!(
        (points[1][0] - 0.33).abs() < 0.05 && points[1][1] > 0.5,
        "{points:?}"
    );
    assert_eq!(bench.state.point, 1, "the new point is the one held");

    // Dragged: one step, previewed while it is held.
    let end = bench.drag("Master curve point 2 of 3", vec2(0.0, 30.0));
    let held = bench.layer(clip).effects[0].points[1];
    assert!(held[1] < points[1][1], "{held:?}");
    assert_eq!(bench.clip(clip).effects[0].points, points, "not yet stored");
    bench.release(end);
    assert_eq!(bench.steps(), before + 2);
    assert_eq!(bench.clip(clip).effects[0].points[1], held);

    // A double click takes it out; the line that is left changes nothing,
    // so the curve goes with it.
    let pos = bench.at("Master curve point 2 of 3");
    tap(&mut bench, pos);
    tap(&mut bench, pos);
    bench.frame(Vec::new());
    assert!(bench.clip(clip).effects.is_empty());
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).effects[0].points.len(), 3);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn an_escape_gives_up_a_curve_point_being_dragged() {
    let (mut bench, clip) = colour();
    let bent = curved(
        EffectKind::CurveMaster,
        &[[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]],
    );
    stack(&mut bench, clip, vec![bent.clone()]);
    let before = bench.steps();
    let end = bench.drag("Master curve point 2 of 3", vec2(20.0, 30.0));
    assert_ne!(bench.layer(clip).effects[0].points, bent.points);
    bench.key(Key::Escape, Modifiers::NONE);
    bench.move_to(end + vec2(5.0, 5.0));
    assert_eq!(bench.layer(clip).effects[0].points, bent.points);
    bench.release(end + vec2(5.0, 5.0));
    assert_eq!(bench.steps(), before);
    assert_eq!(bench.clip(clip).effects, vec![bent]);
}

#[test]
fn a_curve_point_is_moved_and_removed_from_the_keyboard() {
    let (mut bench, clip) = colour();
    let bent = curved(
        EffectKind::CurveMaster,
        &[[0.0, 0.1], [0.5, 0.7], [1.0, 1.0]],
    );
    stack(&mut bench, clip, vec![bent]);
    assert!(!bench.state.owns_keys(&bench.ctx));
    bench.focus("Master curve point 2 of 3");
    assert!(
        bench.state.owns_keys(&bench.ctx),
        "the arrows and Delete are the point's, not the timeline's"
    );
    let before = bench.steps();
    bench.key(Key::ArrowUp, Modifiers::NONE);
    bench.key(Key::ArrowRight, Modifiers::SHIFT);
    let moved = bench.clip(clip).effects[0].points[1];
    assert!(
        (moved[0] - 0.6).abs() < 1e-4 && (moved[1] - 0.71).abs() < 1e-4,
        "{moved:?}"
    );
    assert_eq!(bench.steps(), before + 2);

    bench.key(Key::Delete, Modifiers::NONE);
    assert_eq!(
        bench.clip(clip).effects[0].points,
        vec![[0.0, 0.1], [1.0, 1.0]]
    );
    // An end is moved up and down, and is not removed.
    bench.focus("Master curve point 1 of 2");
    bench.key(Key::ArrowRight, Modifiers::NONE);
    bench.key(Key::Delete, Modifiers::NONE);
    bench.key(Key::ArrowDown, Modifiers::SHIFT);
    assert!(
        bench.clip(clip).effects.is_empty(),
        "moved down to nought, which is the line that changes nothing"
    );

    // A point is added without a pointer, in the widest stretch.
    let (mut bench, clip) = colour();
    bench.click("Red");
    assert_eq!(bench.state.channel, super::super::Channel::Red);
    let before = bench.steps();
    bench.click("Add point");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(kinds(&bench, clip), [EffectKind::CurveRed]);
    assert_eq!(
        bench.clip(clip).effects[0].points,
        vec![[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]]
    );
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_curve_keeps_its_points_in_order_and_within_its_cap() {
    let points = [[0.0, 0.0], [0.4, 0.5], [0.6, 0.5], [1.0, 1.0]];
    // A point stops short of its neighbours, and an end keeps its input.
    let pushed = curve::moved(&points, 1, [0.9, 2.0]);
    assert!(pushed[1][0] < 0.6 && pushed[1][0] > 0.58, "{pushed:?}");
    assert_eq!(pushed[1][1], 1.0);
    assert_eq!(curve::moved(&points, 0, [0.3, 0.2])[0], [0.0, 0.2]);
    assert_eq!(curve::moved(&points, 3, [0.3, -1.0])[3], [1.0, 0.0]);
    assert_eq!(curve::moved(&points, 9, [0.3, 0.3]), points);
    // Whatever is done, the project accepts the curve.
    for moved in [pushed, curve::moved(&points, 2, [0.0, 0.0])] {
        assert!(
            curved(EffectKind::CurveMaster, &moved).validate(),
            "{moved:?}"
        );
    }
    assert!(curve::added(&points, [0.4, 0.9]).is_none(), "on a point");
    let (with, index) = curve::added(&points, [0.5, 0.2]).expect("there is room");
    assert_eq!((index, with[2]), (2, [0.5, 0.2]));
    assert!(curve::removed(&points, 0).is_none());
    assert!(curve::removed(&points, 3).is_none());
    assert_eq!(curve::removed(&points, 1).map(|left| left.len()), Some(3));
    assert!(curve::removed(&curve::IDENTITY, 0).is_none());

    // Sixteen points and no more, each added on the curve as it was.
    let mut full = curve::IDENTITY.to_vec();
    while let Some((with, index)) = curve::added_midway(&full) {
        let on = pass::curve_at(&full, &pass::slopes(&full), with[index][0]);
        assert!((with[index][1] - on).abs() < 1e-6);
        full = with;
    }
    assert_eq!(full.len(), look::MAX_CURVE_POINTS);
    assert!(curved(EffectKind::CurveMaster, &full).validate());
    assert!(curve::added(&full, [0.03, 0.5]).is_none());

    // The plot says so, and offers no more.
    let (mut bench, clip) = colour();
    stack(
        &mut bench,
        clip,
        vec![curved(EffectKind::CurveMaster, &full)],
    );
    bench.frame(Vec::new());
    assert!(bench.find("Add point").is_some_and(|seen| seen.disabled));
    let before = bench.steps();
    let area = plot(&mut bench, "Master curve");
    tap(
        &mut bench,
        Pos2::new(area.left() + area.width() * 0.03, area.center().y),
    );
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), before);
}

// ---- LUT --------------------------------------------------------------------

fn table(name: &str) -> Asset {
    let mut asset = media(name, MediaKind::Lut);
    asset.width = 17;
    asset.height = 0;
    asset
}

#[test]
fn a_lut_is_chosen_among_the_tables_mixed_by_its_intensity_and_taken_off() {
    let (mut bench, clip) = colour();
    let (warm, cold) = (table("warm.cube"), table("cold.cube"));
    bench.edit(Command::AddAssets {
        assets: vec![
            warm.clone(),
            cold.clone(),
            media("still.png", MediaKind::Image),
        ],
    });
    stack(&mut bench, clip, vec![Effect::new(EffectKind::Vignette)]);
    let before = bench.steps();
    bench.frame(Vec::new());
    assert!(!bench.has("LUT intensity"), "nothing to mix yet");

    bench.click("LUT table");
    assert!(bench.has("warm.cube") && bench.has("cold.cube"));
    assert!(!bench.has("still.png"), "only tables are offered");
    bench.click("cold.cube");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(kinds(&bench, clip), [EffectKind::Lut, EffectKind::Vignette]);
    let lut = bench.clip(clip).effects[0].clone();
    assert_eq!((lut.asset_id, lut.mix), (Some(cold.id), 1.0));

    bench.click("LUT table");
    bench.click("warm.cube");
    assert_eq!(bench.clip(clip).effects[0].asset_id, Some(warm.id));
    assert_eq!(bench.clip(clip).effects[0].id, lut.id, "the same effect");

    // Its intensity is the effect's mix: a scrub of it is one step.
    let steps = bench.steps();
    bench.frame(Vec::new());
    let track = bench.find("LUT intensity").expect("named").rect;
    let from = Pos2::new(track.right() - 8.0, track.center().y);
    bench.press(from);
    for step in 1..=4 {
        bench.move_to(from - vec2(12.0 * step as f32, 0.0));
    }
    let held = bench.layer(clip).effects[0].mix;
    assert!(held < 0.9 && held > 0.0, "{held}");
    assert_eq!(bench.clip(clip).effects[0].mix, 1.0);
    bench.release(from - vec2(48.0, 0.0));
    assert_eq!(bench.steps(), steps + 1);
    assert_eq!(bench.clip(clip).effects[0].mix, held);

    bench.click("LUT table");
    bench.click("None");
    assert_eq!(kinds(&bench, clip), [EffectKind::Vignette]);
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).effects[0].mix, held);

    // A table that is not in the library is brought in from here.
    assert_eq!(bench.imports, 0);
    bench.click("Import a LUT file");
    assert_eq!(bench.imports, 1);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

// ---- Background removal -------------------------------------------------------

#[test]
fn a_chroma_key_is_put_first_in_the_stack_and_off_takes_it_out() {
    let (mut bench, clip) = colour();
    stack(&mut bench, clip, vec![Effect::new(EffectKind::Glow)]);
    let before = bench.steps();
    bench.frame(Vec::new());
    assert!(!bench.has("Tolerance"));

    bench.click("Chroma key");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(
        kinds(&bench, clip),
        [EffectKind::ChromaKey, EffectKind::Glow]
    );
    bench.frame(Vec::new());
    for hidden in ["Key red", "Key green", "Key blue"] {
        assert!(!bench.has(hidden), "{hidden} is part of the swatch");
    }
    assert!(bench.find("Key colour").is_some());

    let end = bench.drag("Tolerance", vec2(40.0, 0.0));
    let held = bench.layer(clip).effects[0].param(ParamKey::Tolerance);
    assert!(held.is_some_and(|tolerance| tolerance > 0.3));
    bench.key(Key::Escape, Modifiers::NONE);
    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "given up");
    let end = bench.drag("Spill", vec2(-30.0, 0.0));
    bench.release(end);
    assert_eq!(bench.steps(), before + 2);
    assert!(bench.clip(clip).effects[0].param(ParamKey::Spill) < Some(0.5));

    bench.click("Show background removal");
    assert!(!bench.clip(clip).effects[0].enabled);
    bench.click("Off");
    assert_eq!(kinds(&bench, clip), [EffectKind::Glow]);
    assert!(bench.editor.undo());
    assert_eq!(
        kinds(&bench, clip),
        [EffectKind::ChromaKey, EffectKind::Glow]
    );
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_clean_plate_is_no_effect_until_its_still_is_chosen() {
    let (mut bench, clip) = colour();
    let plate = media("empty room.png", MediaKind::Image);
    bench.edit(Command::AddAssets {
        assets: vec![plate.clone(), media("other.mp4", MediaKind::Video)],
    });
    bench.click("Chroma key");
    let before = bench.steps();

    // One way at a time: the key goes, and no matte is made without a plate.
    bench.click("Clean plate");
    assert_eq!(bench.steps(), before + 1);
    assert!(bench.clip(clip).effects.is_empty());
    bench.frame(Vec::new());
    assert!(bench.has("Choose a still of the scene without its subject."));

    let combo = (bench.seen.iter())
        .find(|seen| seen.name == "Clean plate" && seen.role == Role::ComboBox)
        .expect("the plate's list is named")
        .rect
        .center();
    bench.press(combo);
    bench.release(combo);
    assert!(bench.has("empty room.png"));
    assert!(!bench.has("other.mp4"), "only stills are offered");
    bench.click("empty room.png");
    assert_eq!(bench.steps(), before + 2);
    let matte = bench.clip(clip).effects[0].clone();
    assert_eq!(
        (matte.kind, matte.asset_id),
        (EffectKind::DifferenceMatte, Some(plate.id))
    );
    assert!(matte.validate());
    bench.frame(Vec::new());
    assert!(bench.find("Tolerance").is_some() && bench.find("Softness").is_some());

    assert!(bench.editor.undo());
    assert!(bench.clip(clip).effects.is_empty());
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
    // What is not built is said, and not offered.
    assert!(bench.has("No automatic subject cut-out: use a key colour or a clean plate."));
}

#[test]
fn a_key_colour_is_picked_for_the_clip_and_effect_its_button_was_pressed_for() {
    let (mut bench, clip) = colour();
    let key = Effect::new(EffectKind::ChromaKey);
    stack(&mut bench, clip, vec![key.clone()]);
    let pick = ColourPick {
        clip,
        effect: key.id,
    };
    assert_eq!(bench.state.colour_pick(&bench.selection), None);
    bench.click("Pick the key colour from the picture");
    assert_eq!(bench.state.colour_pick(&bench.selection), Some(pick));
    // It is not picked into another clip, or from another section.
    let mut other = bench.selection.clone();
    other.clips.clear();
    assert_eq!(bench.state.colour_pick(&other), None);
    bench.state.section = Section::Look;
    assert_eq!(bench.state.colour_pick(&bench.selection), None);
    bench.state.section = Section::Color;

    // The colour the canvas hands back is one step, and undoes.
    let before = bench.steps();
    let rgb = [0.1, 0.8, 0.3];
    let command = key_colour_command(bench.editor.project(), pick, None, rgb, bench.playhead)
        .expect("the effect is there");
    assert_eq!(command.label(), "Pick key colour");
    bench.editor.dispatch(command).expect("accepted");
    assert!(bench.state.end_pick());
    assert!(!bench.state.end_pick());
    assert_eq!(bench.steps(), before + 1);
    let stored = &bench.clip(clip).effects[0];
    let colour = [ParamKey::KeyR, ParamKey::KeyG, ParamKey::KeyB].map(|key| stored.param(key));
    assert_eq!(colour, rgb.map(Some));
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).effects, vec![key.clone()]);

    // A channel keyframes drive takes the colour as a keyframe at the
    // playhead; the others as stored values: still one step.
    bench.edit(Command::SetKeyframes {
        clip_id: clip,
        target: Some(key.id),
        key: ParamKey::KeyG,
        keys: vec![super::key(0.0, 1.0)],
    });
    let before = bench.steps();
    let command = key_colour_command(bench.editor.project(), pick, None, rgb, seconds(2.0))
        .expect("the effect is there");
    bench.editor.dispatch(command).expect("accepted");
    assert_eq!(bench.steps(), before + 1);
    let stored = bench.clip(clip);
    assert_eq!(stored.effects[0].param(ParamKey::KeyR), Some(0.1));
    assert_eq!(stored.effects[0].param(ParamKey::KeyG), Some(1.0));
    assert_eq!(
        stored.animation[0].keys,
        vec![super::key(0.0, 1.0), super::key(2.0, 0.8)]
    );

    // A stack held in a draft is the one written, and an effect that has
    // gone is picked into by nothing.
    let held = vec![key.clone(), Effect::new(EffectKind::Blur)];
    let command = key_colour_command(bench.editor.project(), pick, Some(&held), rgb, Time::ZERO)
        .expect("the effect is held");
    bench.editor.dispatch(command).expect("accepted");
    assert_eq!(
        kinds(&bench, clip),
        [EffectKind::ChromaKey, EffectKind::Blur]
    );
    let gone = ColourPick {
        clip,
        effect: uuid::Uuid::new_v4(),
    };
    assert!(key_colour_command(bench.editor.project(), gone, None, rgb, Time::ZERO).is_none());

    // Taking the key out ends a pick of its colour.
    bench.click("Pick the key colour from the picture");
    assert!(bench.state.colour_pick(&bench.selection).is_some());
    bench.click("Off");
    bench.frame(Vec::new());
    assert_eq!(bench.state.colour_pick(&bench.selection), None);
}

// ---- Background ---------------------------------------------------------------

#[test]
fn a_background_goes_on_a_track_under_the_clip_as_one_step() {
    let (mut bench, clip) = colour();
    let sky = media("sky.png", MediaKind::Image);
    let waves = media("waves.mp4", MediaKind::Video);
    bench.edit(Command::AddAssets {
        assets: vec![
            sky.clone(),
            waves.clone(),
            media("hum.wav", MediaKind::Audio),
            table("warm.cube"),
        ],
    });
    let before = bench.steps();
    let tracks = bench.editor.project().tracks.len();
    let own = bench.track(TrackKind::Video);
    let index_of = |bench: &Bench, id: TrackId| {
        (bench.editor.project().tracks.iter())
            .position(|track| track.id == id)
            .expect("the track exists")
    };
    let was = index_of(&bench, own);

    // No video track is under the clip's: one is made directly under it.
    bench.click("Add");
    assert!(bench.has("sky.png") && bench.has("waves.mp4"));
    assert!(!bench.has("hum.wav") && !bench.has("warm.cube"));
    bench.click("sky.png");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Add background");
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
    let project = bench.editor.snapshot();
    assert_eq!(project.tracks.len(), tracks + 1);
    assert_eq!(index_of(&bench, own), was + 1, "the clip's track moved up");
    let under = &project.tracks[was];
    assert_eq!(under.kind, TrackKind::Video);
    let behind = &under.clips[0];
    assert_eq!(behind.asset_id(), Some(sky.id));
    assert_eq!(
        (behind.start, behind.duration),
        (bench.clip(clip).start, bench.clip(clip).duration),
        "a still lasts as long as the clip"
    );
    // The plan draws it under the clip.
    let plan = resolve(&project, bench.playhead, &[]);
    let order: Vec<ClipId> = plan.layers.iter().map(|layer| layer.clip).collect();
    assert_eq!(order, [behind.id, clip]);

    assert!(bench.editor.undo());
    assert_eq!(bench.editor.project().tracks.len(), tracks);
    assert_eq!(index_of(&bench, own), was);
    assert!(bench.editor.redo());

    // That track is taken over the clip's stretch now: another is made,
    // nearer the clip. A longer clip takes of a video only what it has.
    let long = {
        let mut long = Clip::from_asset(&media("shot.mp4", MediaKind::Video), seconds(4.0));
        long.content = bench.clip(clip).content.clone();
        long.duration = seconds(6.0);
        let id = long.id;
        bench.edit(Command::AddClip {
            track_id: own,
            clip: long,
        });
        id
    };
    // Something else is on the track that was made, over the clip's stretch.
    let command = background_command(bench.editor.project(), clip, sky.id).expect("made");
    assert!(matches!(&command, Command::Batch { commands, .. } if commands.len() == 3));
    let command = background_command(bench.editor.project(), clip, waves.id).expect("made");
    bench.editor.dispatch(command).expect("accepted");
    let project = bench.editor.snapshot();
    assert_eq!(project.tracks.len(), tracks + 2);
    assert_eq!(project.tracks[was + 1].clips[0].asset_id(), Some(waves.id));
    // Under the later clip the first track made is free, and is used.
    let command = background_command(bench.editor.project(), long, waves.id).expect("made");
    bench.editor.dispatch(command).expect("accepted");
    let project = bench.editor.snapshot();
    assert_eq!(project.tracks.len(), tracks + 2, "no track was made");
    let reused = &project.tracks[was + 1].clips;
    assert_eq!(reused.len(), 2);
    assert!(
        (reused.iter())
            .any(|behind| (behind.start, behind.duration) == (seconds(4.0), seconds(6.0))),
        "{reused:?}"
    );

    // What cannot be a background is refused in words.
    let hum = (project.assets.values())
        .find(|asset| asset.kind == MediaKind::Audio)
        .expect("the sound");
    assert!(background_command(&project, clip, hum.id).is_err());
    assert!(background_command(&project, ClipId::new_v4(), sky.id).is_err());
}

// ---- Locks, names and room ------------------------------------------------------

#[test]
fn a_locked_track_takes_no_colour_edit() {
    let (mut bench, clip) = colour();
    let track = bench.track(TrackKind::Video);
    stack(&mut bench, clip, vec![Effect::new(EffectKind::ChromaKey)]);
    bench.edit(Command::SetTrackState {
        track_id: track,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
    });
    let before = bench.steps();
    bench.frame(Vec::new());
    for name in ["Exposure", "Add hue band", "Add point", "Off", "Tolerance"] {
        assert!(
            bench.find(name).is_some_and(|seen| seen.disabled),
            "{name} is not offered"
        );
    }
    bench.click("Off");
    let area = plot(&mut bench, "Master curve");
    tap(&mut bench, area.center() - vec2(20.0, 30.0));
    let end = bench.drag("Exposure", vec2(40.0, 0.0));
    bench.release(end);
    assert_eq!(bench.steps(), before);
    assert!(bench.draft.is_none());
    assert_eq!(kinds(&bench, clip), [EffectKind::ChromaKey]);
}

/// A clip with something in every group of the section, and its widest rows.
fn graded(width: f32) -> Bench {
    let (mut bench, clip) = colour();
    let plate = media("plate.png", MediaKind::Image);
    let lut_asset = table("a table with a long name.cube");
    bench.edit(Command::AddAssets {
        assets: vec![plate, lut_asset.clone()],
    });
    let grade = with_params(EffectKind::Grade, &[(ParamKey::Exposure, 0.5)]);
    let band = Effect::new(EffectKind::HslBand);
    let mut lut = Effect::new(EffectKind::Lut);
    lut.asset_id = Some(lut_asset.id);
    let key = Effect::new(EffectKind::ChromaKey);
    stack(
        &mut bench,
        clip,
        vec![
            key.clone(),
            grade.clone(),
            band.clone(),
            curved(
                EffectKind::CurveMaster,
                &[[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]],
            ),
            lut,
        ],
    );
    for (target, key, value) in [
        (grade.id, ParamKey::Exposure, 0.5),
        (band.id, ParamKey::SatShift, 0.0),
        (key.id, ParamKey::Tolerance, 0.3),
        (key.id, ParamKey::KeyR, 0.0),
    ] {
        bench.edit(Command::SetKeyframes {
            clip_id: clip,
            target: Some(target),
            key,
            keys: vec![super::key(0.0, value), super::key(2.0, value)],
        });
    }
    bench.size = vec2(width, 3200.0);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    bench
}

#[test]
fn every_colour_control_is_named_and_fits_a_narrow_panel() {
    for width in [300.0, 240.0] {
        let bench = graded(width);
        let panel = Rect::from_min_size(Pos2::ZERO, bench.size);
        let mut controls = 0;
        for seen in bench.seen.iter().filter(|seen| !Bench::is_text(seen)) {
            // The window and the panel's own containers are not controls.
            if seen.rect.width() >= width - 1.0 {
                continue;
            }
            controls += 1;
            assert!(
                seen.rect.left() >= panel.left() + 11.0
                    && seen.rect.right() <= panel.right() - 11.0,
                "at {width}: \"{}\" is at {:?}",
                seen.name,
                seen.rect
            );
            assert!(!seen.name.trim().is_empty());
        }
        assert!(controls > 30, "{controls} controls at {width}");
        for name in [
            "Exposure",
            "Gain",
            "Hue band 1, reds",
            "Master curve",
            "Master curve point 2 of 3",
            "LUT table",
            "LUT intensity",
            "Key colour",
            "Pick the key colour from the picture",
            "Add",
            "Canvas colour",
        ] {
            assert!(bench.find(name).is_some(), "\"{name}\" at {width}");
        }
    }
}

#[test]
fn every_row_of_the_grade_and_the_bands_is_a_row_of_the_tables() {
    let bench = graded(300.0);
    for kind in [EffectKind::Grade, EffectKind::HslBand] {
        for spec in look::spec(kind) {
            let (label, _) = presented(AnimTarget::Effect(kind), spec);
            assert!(bench.find(label).is_some(), "{kind:?} has no \"{label}\"");
        }
    }
}

#[test]
fn a_colour_section_nothing_is_done_in_asks_for_no_frame() {
    let mut bench = graded(300.0);
    for _ in 0..30 {
        bench.frame(Vec::new());
    }
    assert!(
        bench.wait > Duration::from_secs(3600),
        "asked for a frame in {:?}",
        bench.wait
    );
    assert!(bench.draft.is_none() && bench.refused.is_empty());
    assert!(!bench.state.owns_keys(&bench.ctx));
}

#[test]
fn the_canvas_colour_is_the_projects_and_is_committed_with_the_draft() {
    let (mut bench, _) = colour();
    let before = bench.steps();
    let mut settings = bench.editor.project().settings.clone();
    settings.background = [10, 200, 30, 255];
    // What the swatch writes, as the frame's end commits it.
    bench.project_draft.canvas = Some(settings.clone());
    bench.frame(Vec::new());
    assert_eq!(bench.editor.project().settings, settings);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Project settings");
    assert!(bench.editor.undo());
    assert_ne!(bench.editor.project().settings, settings);
}
