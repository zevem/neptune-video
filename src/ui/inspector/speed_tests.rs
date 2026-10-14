//! The Speed section, driven as the rest of the inspector is in the tests
//! above: each interaction for the command it makes, its undo and its cancel.
use super::*;
use crate::speed::{self, Curve, Easing};
use editor_core::{Retime, TimeKey};

const REFUSED: &str =
    "Not applied: part of the clip would play slower than 0.05× or faster than 16×.";
const SILENT: &str = "Sound is silent where the clip is held, reversed or ramped; elsewhere speed changes its pitch.";

/// A bench on the Speed section of its clip: four seconds from the start of
/// a ten second video, with the playhead a second in.
fn timed() -> (Bench, ClipId) {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Speed;
    (bench, clip)
}

/// Gives the clip a straight map through these places: seconds of the clip,
/// and of its source.
fn map(bench: &mut Bench, clip: ClipId, keys: &[(f64, f64)]) {
    let retime = Retime {
        keys: (keys.iter())
            .map(|&(at, source)| TimeKey {
                at: seconds(at),
                source: seconds(source),
                ease: Ease::Linear,
            })
            .collect(),
    };
    let duration = retime.keys[retime.keys.len() - 1].at;
    bench.edit(Command::SetRetime {
        clip_id: clip,
        retime: Some(retime),
        duration,
    });
}

fn curve(bench: &Bench, clip: ClipId) -> Curve {
    Curve::of(bench.clip(clip))
}

/// Where the control with this name is, the section's own tab aside.
fn place(bench: &mut Bench, name: &str) -> Pos2 {
    bench.frame(Vec::new());
    (bench.seen.iter())
        .find(|seen| seen.name == name && seen.role != Role::RadioButton && !Bench::is_text(seen))
        .unwrap_or_else(|| panic!("FIXTURE: no control is named \"{name}\""))
        .rect
        .center()
}

/// Takes hold at `from` and moves `by`, without letting go.
fn pull(bench: &mut Bench, from: Pos2, by: Vec2) -> Pos2 {
    bench.press(from);
    for step in 1..=4 {
        bench.move_to(from + by * (step as f32 / 4.0));
    }
    from + by
}

// ---- Constant speed -------------------------------------------------------

#[test]
fn a_named_speed_is_one_step_and_the_clip_lasts_as_long_as_it_says() {
    let (mut bench, clip) = timed();
    let before = bench.steps();
    bench.click("2×");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.clip(clip).playback_rate, 2.0);
    assert_eq!(bench.clip(clip).duration, seconds(2.0));
    assert!(bench.has("00:00:02:00"), "the new length is read out");
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).playback_rate, 1.0);
    assert_eq!(bench.clip(clip).duration, seconds(4.0));
}

#[test]
fn a_slower_clip_is_cut_to_the_room_before_the_next_one() {
    let (mut bench, clip) = timed();
    let next = {
        let mut next = bench.clip(clip).clone();
        next.id = uuid::Uuid::new_v4();
        next.start = seconds(6.0);
        next
    };
    bench.add(TrackKind::Video, next);
    let before = bench.steps();
    bench.click("0.5×");
    assert_eq!(
        bench.steps(),
        before + 1,
        "the speed and the cut are one step"
    );
    assert_eq!(bench.clip(clip).playback_rate, 0.5);
    assert_eq!(bench.clip(clip).duration, seconds(6.0));
    assert!(bench.refused.is_empty());
}

#[test]
fn a_dragged_speed_is_one_step_and_an_escape_gives_it_up() {
    let (mut bench, clip) = timed();
    let before = bench.steps();
    let from = place(&mut bench, "Speed");
    let end = pull(&mut bench, from, vec2(60.0, 0.0));
    assert_eq!(bench.clip(clip).playback_rate, 1.0, "held, not applied");
    let held = bench.draft.as_ref().expect("a draft").rate;
    assert!(held > 1.0);
    bench.release(end);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.clip(clip).playback_rate, held);
    assert!(bench.editor.undo());

    let end = pull(&mut bench, from, vec2(60.0, 0.0));
    bench.key(Key::Escape, Modifiers::NONE);
    bench.release(end);
    assert_eq!(bench.steps(), before);
    assert_eq!(bench.clip(clip).playback_rate, 1.0);
}

#[test]
fn a_named_speed_lets_a_ramp_go_and_a_dragged_one_plays_it_faster() {
    let (mut bench, clip) = timed();
    bench.click("Hero");
    let ramped = bench.clip(clip).clone();
    assert!(bench.has("Average speed"));
    let before = bench.steps();

    // The whole map as much faster as the field is dragged.
    let from = place(&mut bench, "Speed");
    let end = pull(&mut bench, from, vec2(40.0, 0.0));
    bench.release(end);
    assert_eq!(bench.steps(), before + 1);
    let faster = bench.clip(clip);
    assert!(faster.retime.is_some() && faster.duration < ramped.duration);
    assert_eq!(faster.source_span(), ramped.source_span());
    assert!(bench.editor.undo());

    bench.click("1×");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Change speed");
    let plain = bench.clip(clip);
    assert_eq!((plain.retime.as_ref(), plain.playback_rate), (None, 1.0));
    assert_eq!(plain.duration, seconds(4.0));
    assert!(bench.editor.undo());
    assert_eq!(*bench.clip(clip), ramped);
}

// ---- Reverse and freeze ---------------------------------------------------

#[test]
fn reverse_plays_the_clips_source_backwards_and_says_it_is_silent() {
    let (mut bench, clip) = timed();
    let before = bench.steps();
    bench.frame(Vec::new());
    assert!(!bench.has(SILENT));
    bench.click("Reverse");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Reverse clip");
    let reversed = bench.clip(clip);
    assert_eq!(reversed.source_time(reversed.start), seconds(4.0));
    assert_eq!(reversed.source_time(reversed.end()), Time::ZERO);
    assert_eq!(reversed.duration, seconds(4.0));
    assert!(bench.has(SILENT));

    // Off again, it plays forwards.
    bench.click("Reverse");
    let forwards = bench.clip(clip);
    assert_eq!(forwards.source_time(forwards.start), Time::ZERO);
    assert_eq!(forwards.source_time(forwards.end()), seconds(4.0));
    assert!(!bench.has(SILENT));
    assert!(bench.editor.undo() && bench.editor.undo());
    assert_eq!(bench.clip(clip).retime, None);
}

#[test]
fn a_frame_is_frozen_at_the_playhead_for_the_length_set() {
    let (mut bench, clip) = timed();
    let before = bench.steps();
    bench.click("Freeze at playhead");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Freeze frame");
    let frozen = bench.clip(clip).clone();
    assert_eq!(frozen.duration, seconds(6.0));
    for (playhead, source) in [(0.5, 0.5), (1.0, 1.0), (2.9, 1.0), (4.0, 2.0)] {
        assert_eq!(frozen.source_time(seconds(playhead)), seconds(source));
    }
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).retime, None);

    // The length is the field's, and the clip is the one the button was for.
    bench.state.hold = 0.5;
    bench.click("Freeze at playhead");
    assert_eq!(bench.clip(clip).duration, seconds(4.5));
    assert!(bench.editor.undo());

    // Off the clip there is no frame of it to hold.
    bench.playhead = seconds(8.0);
    bench.frame(Vec::new());
    assert!(
        bench
            .find("Freeze at playhead")
            .is_some_and(|seen| seen.disabled)
    );
    bench.click("Freeze at playhead");
    assert_eq!(bench.steps(), before);
}

// ---- Ramps and the curve --------------------------------------------------

#[test]
fn every_ramp_has_a_button_that_is_one_step() {
    for ramp in speed::Ramp::ALL {
        let (mut bench, clip) = timed();
        let before = bench.steps();
        bench.click(ramp.label());
        assert_eq!(bench.steps(), before + 1, "{ramp:?}");
        assert_eq!(bench.last(), ramp.label());
        assert_eq!(speed::badge(bench.clip(clip)).as_deref(), Some("Ramp"));
        assert_eq!(bench.clip(clip).source_span(), (Time::ZERO, seconds(4.0)));
        assert!(bench.refused.is_empty());
        assert!(bench.editor.undo());
        assert_eq!(bench.clip(clip).retime, None);
    }
}

#[test]
fn a_point_dragged_up_is_held_and_is_one_step_when_let_go() {
    let (mut bench, clip) = timed();
    let before = bench.steps();
    let stored = bench.editor.snapshot();
    let from = place(&mut bench, "Speed point 2 of 2");
    let end = pull(&mut bench, from, vec2(0.0, -24.0));

    // Held: the project is as it was, and the section shows the draft.
    assert_eq!(bench.steps(), before);
    assert_eq!(*bench.editor.project(), *stored);
    let (held, length) = (bench.draft.as_ref())
        .and_then(|draft| draft.retime.clone())
        .expect("the point's map is held");
    assert!(length < seconds(4.0), "a faster end is a shorter clip");

    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "the whole drag is one step");
    assert!(bench.draft.is_none());
    let ramped = bench.clip(clip);
    assert_eq!(
        (ramped.retime.as_ref(), ramped.duration),
        (Some(&held), length)
    );
    let read = curve(&bench, clip);
    assert!((read.speed(0) - 1.0).abs() < 1e-6);
    assert!(read.speed(1) > 1.5, "the end is faster: {}", read.speed(1));
    assert_eq!(ramped.source_span(), (Time::ZERO, seconds(4.0)));

    assert!(bench.editor.undo());
    assert_eq!(*bench.editor.project(), *stored);
}

#[test]
fn a_points_speed_follows_the_pointer_in_proportion_on_a_plot_that_holds_still() {
    let (mut bench, clip) = timed();
    let from = place(&mut bench, "Speed point 2 of 2");
    let first = place(&mut bench, "Speed point 1 of 2");
    let plot = bench.find("Speed curve").expect("named").rect;
    // The clip plays at normal speed, half way up a plot that tops out at
    // twice that: a point of travel is the same change wherever it is.
    let floor = from.y + (from.y - plot.top() - 9.5);
    let a_point = 1.0 / f64::from(floor - from.y);
    let end = pull(&mut bench, from, vec2(0.0, -20.0));
    // Held, the plot is as it was: the other point has not moved under it.
    bench.frame(Vec::new());
    let held = (bench.seen.iter())
        .find(|seen| seen.name == "Speed point 1 of 2")
        .expect("named")
        .rect
        .center();
    assert_eq!(held, first);
    bench.release(end);
    let speed = curve(&bench, clip).speed(1);
    assert!(
        (speed - (1.0 + 20.0 * a_point)).abs() < 0.03,
        "{speed} after twenty points at {a_point} a point"
    );
    assert!(
        speed < 1.6,
        "twenty points are not a tenfold change: {speed}"
    );
    // The point is where the pointer left it.
    let now = place(&mut bench, "Speed point 2 of 2");
    assert!((now.y - end.y).abs() <= 1.5, "{now:?} for {end:?}");

    // Pulled far past the top, it stops at the top of the scale it was
    // taken hold of on; let go, the scale makes room above it.
    let end = pull(&mut bench, now, vec2(0.0, -400.0));
    bench.release(end);
    assert!((curve(&bench, clip).speed(1) - 2.0).abs() < 1e-6);
    let after = place(&mut bench, "Speed point 2 of 2");
    assert!(after.y > plot.top() + 30.0, "{after:?} in {plot:?}");
}

#[test]
fn an_escape_gives_up_a_held_point() {
    let (mut bench, clip) = timed();
    let before = bench.steps();
    let from = place(&mut bench, "Speed point 2 of 2");
    let end = pull(&mut bench, from, vec2(0.0, -24.0));
    assert!(bench.draft.is_some());
    bench.key(Key::Escape, Modifiers::NONE);
    assert!(bench.draft.is_none());
    bench.move_to(end + vec2(0.0, -10.0));
    assert!(bench.draft.is_none(), "the cancelled drag writes no draft");
    bench.release(end + vec2(0.0, -10.0));
    assert_eq!(bench.steps(), before);
    assert_eq!(bench.clip(clip).retime, None);
}

#[test]
fn a_point_dragged_sideways_moves_in_the_clip_and_keeps_its_speed() {
    let (mut bench, clip) = timed();
    map(&mut bench, clip, &[(0.0, 0.0), (2.0, 2.0), (4.0, 4.0)]);
    let before = bench.steps();
    let from = place(&mut bench, "Speed point 2 of 3");
    let end = pull(&mut bench, from, vec2(40.0, 2.0));
    bench.release(end);
    assert_eq!(bench.steps(), before + 1);
    let read = curve(&bench, clip);
    assert!(read.at(1) > seconds(2.2), "moved later: {:?}", read.at(1));
    assert_eq!(read.speed(1), 1.0);
    assert_eq!(read.duration(), seconds(4.0));
}

#[test]
fn a_ramp_the_project_refuses_keeps_the_last_one_it_takes_and_says_why() {
    let (mut bench, clip) = timed();
    // As slow as a clip may be: any slower at one end is refused.
    map(&mut bench, clip, &[(0.0, 0.0), (4.0, 0.24)]);
    let before = bench.steps();
    let from = place(&mut bench, "Speed point 1 of 2");
    bench.press(from);
    bench.move_to(from + vec2(0.0, -20.0));
    let held = (bench.draft.as_ref()).and_then(|draft| draft.retime.clone());
    assert!(held.is_some(), "faster at the start is a map");
    assert!(!bench.has(REFUSED));
    // Down to a stop, the stretch is slower on the whole than a map may be.
    bench.move_to(from + vec2(0.0, 40.0));
    bench.move_to(from + vec2(0.0, 60.0));
    assert!(bench.has(REFUSED), "the refusal is said");
    let kept = (bench.draft.as_ref()).and_then(|draft| draft.retime.clone());
    assert_eq!(kept, held, "the last map the project takes is still held");

    bench.release(from + vec2(0.0, 60.0));
    assert_eq!(bench.steps(), before + 1);
    assert!(
        bench.refused.is_empty(),
        "nothing refused reached the editor"
    );
    assert_eq!(bench.clip(clip).retime, held.map(|(retime, _)| retime));
    bench.editor.project().validate().expect("a valid project");
}

#[test]
fn a_focused_point_is_moved_with_the_arrows_and_removed_with_delete() {
    let (mut bench, clip) = timed();
    map(&mut bench, clip, &[(0.0, 0.0), (2.0, 2.0), (4.0, 4.0)]);
    let before = bench.steps();
    bench.focus("Speed point 2 of 3");
    assert!(
        bench.state.owns_keys(&bench.ctx),
        "the arrows are the point's"
    );
    bench.key(Key::ArrowUp, Modifiers::NONE);
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Change speed curve");
    assert!((curve(&bench, clip).speed(1) - 1.05).abs() < 0.005);
    bench.key(Key::ArrowUp, Modifiers::SHIFT);
    bench.frame(Vec::new());
    assert!((curve(&bench, clip).speed(1) - 1.55).abs() < 0.005);
    let at = curve(&bench, clip).at(1);
    bench.key(Key::ArrowRight, Modifiers::NONE);
    bench.frame(Vec::new());
    assert!(curve(&bench, clip).at(1) > at);

    bench.key(Key::Delete, Modifiers::NONE);
    bench.frame(Vec::new());
    assert_eq!(curve(&bench, clip).points(), 2);
    assert_eq!(bench.steps(), before + 4);
    for _ in 0..4 {
        assert!(bench.editor.undo());
    }
    assert_eq!(curve(&bench, clip).points(), 3);
}

#[test]
fn a_point_is_added_given_a_speed_eased_and_removed_without_a_pointer_on_the_graph() {
    let (mut bench, clip) = timed();
    let before = bench.steps();
    // At the playhead, a second in.
    bench.click("Add point");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Add speed point");
    let read = curve(&bench, clip);
    assert_eq!((read.points(), read.at(1)), (3, seconds(1.0)));
    assert_eq!(read.duration(), seconds(4.0), "a new point changes nothing");
    assert!(bench.has("Point 2 of 3"), "the new point is the chosen one");

    // Its speed, typed or dragged in its field.
    let from = place(&mut bench, "Point speed");
    let end = pull(&mut bench, from, vec2(80.0, 0.0));
    assert_eq!(bench.steps(), before + 1, "held");
    bench.release(end);
    assert_eq!(bench.steps(), before + 2);
    let peak = curve(&bench, clip).speed(1);
    assert!(peak > 1.2, "{peak}");

    // The stretch that leaves it ramps evenly, or not at all.
    bench.click("Segment easing");
    bench.click("Linear");
    assert_eq!(bench.steps(), before + 3);
    assert_eq!(bench.last(), "Change speed easing");
    assert_eq!(curve(&bench, clip).pieces[1].easing(), Some(Easing::Linear));
    bench.click("Segment easing");
    bench.click("Constant");
    let flat = curve(&bench, clip);
    assert_eq!(flat.pieces[1].easing(), Some(Easing::Constant));
    assert!((flat.pieces[1].to - peak).abs() < 0.01);

    bench.click("Remove point");
    assert_eq!(bench.last(), "Remove speed point");
    assert_eq!(curve(&bench, clip).points(), 2);
    assert_eq!(bench.steps(), before + 5);
    assert!(bench.refused.is_empty());
}

#[test]
fn a_double_click_on_the_graph_adds_a_point_and_one_on_a_point_removes_it() {
    let (mut bench, clip) = timed();
    let before = bench.steps();
    bench.frame(Vec::new());
    let plot = bench.find("Speed curve").expect("the graph is named").rect;
    let spot = Pos2::new(plot.center().x, plot.top() + 12.0);
    for _ in 0..2 {
        bench.press(spot);
        bench.frame(vec![Bench::button(spot, PointerButton::Primary, false)]);
    }
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(curve(&bench, clip).points(), 3);

    // Long enough after the last click for these to be a pair of their own.
    for _ in 0..20 {
        bench.frame(Vec::new());
    }
    let point = place(&mut bench, "Speed point 2 of 3");
    for _ in 0..2 {
        bench.press(point);
        bench.frame(vec![Bench::button(point, PointerButton::Primary, false)]);
    }
    bench.frame(Vec::new());
    assert_eq!(curve(&bench, clip).points(), 2);
    assert_eq!(bench.steps(), before + 2);
}

// ---- Tracks, stills and rest ---------------------------------------------

#[test]
fn a_locked_track_takes_no_speed() {
    let (mut bench, clip) = timed();
    let track_id = bench.track(TrackKind::Video);
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    });
    let before = bench.steps();
    for name in ["2×", "Reverse", "Freeze at playhead", "Hero", "Add point"] {
        bench.click(name);
    }
    let from = place(&mut bench, "Speed point 2 of 2");
    let end = pull(&mut bench, from, vec2(0.0, -24.0));
    bench.release(end);
    assert_eq!(bench.steps(), before);
    assert!(bench.draft.is_none());
    // And the editor refuses what would be asked of it.
    let mut turned = curve(&bench, clip);
    turned.reverse();
    let command =
        speed::curve_command(bench.editor.project(), clip, &turned, "Reverse clip").expect("a map");
    assert!(bench.editor.dispatch(command).is_err());
    assert_eq!(bench.clip(clip).retime, None);
}

#[test]
fn a_still_has_no_speed_to_edit() {
    let mut bench = Bench::empty();
    let asset = media("still.png", MediaKind::Image);
    bench.edit(Command::AddAssets {
        assets: vec![asset.clone()],
    });
    let clip = bench.add(TrackKind::Video, Clip::from_asset(&asset, Time::ZERO));
    bench.selection.only(clip);
    bench.state.section = Section::Speed;
    bench.frame(Vec::new());
    assert!(bench.has("A still has no speed of its own."));
    assert!(bench.find("Speed curve").is_none());
}

#[test]
fn the_speed_section_asks_for_no_frame_when_nothing_is_done_in_it() {
    let (mut bench, clip) = timed();
    for ramped in [false, true] {
        if ramped {
            bench.click("Montage");
        }
        for _ in 0..30 {
            bench.frame(Vec::new());
        }
        assert!(
            bench.wait > Duration::from_secs(3600),
            "asked for a frame in {:?}",
            bench.wait
        );
        assert!(bench.draft.is_none() && bench.refused.is_empty());
    }
    assert!(bench.clip(clip).retime.is_some());
}

#[test]
fn a_button_pressed_while_a_typed_speed_is_held_edits_the_clip_that_speed_leaves() {
    let (mut bench, clip) = timed();
    let before = bench.steps();
    // The field, not the section's own tab.
    bench.frame(Vec::new());
    let target_node = (bench.seen.iter())
        .find(|seen| {
            seen.name == "Speed" && seen.role != Role::RadioButton && !Bench::is_text(seen)
        })
        .expect("FIXTURE: the speed field is named")
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
    bench.frame(vec![Event::Text("2".to_owned())]);
    assert_eq!(bench.draft.as_ref().map(|draft| draft.rate), Some(2.0));
    assert_eq!(bench.steps(), before, "typed and held, not yet a step");

    // Reverse, while what was typed is still held. The typed speed is
    // committed first and the clip it leaves is the one turned: two seconds
    // of the same four seconds of source, last frame first.
    let reverse = place(&mut bench, "Reverse");
    bench.press(reverse);
    assert_eq!(bench.steps(), before, "still held under the press");
    bench.release(reverse);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
    assert_eq!(bench.steps(), before + 2, "the speed, then the turn");
    let turned = bench.clip(clip);
    assert_eq!(turned.duration, seconds(2.0));
    assert_eq!(turned.source_span(), (Time::ZERO, seconds(4.0)));
    assert!(curve(&bench, clip).reversed());

    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).playback_rate, 2.0);
    assert_eq!(bench.clip(clip).duration, seconds(2.0));
    assert!(bench.clip(clip).retime.is_none());
}

// ---- Keep pitch -----------------------------------------------------------

const DETACH: &str = "Detach audio to keep pitch";

fn pitch_of(bench: &Bench, clip: ClipId) -> Option<f32> {
    editor_core::sound::pitch(bench.clip(clip))
}

/// Whether the "Keep pitch" switch reads on, and whether it can be turned.
fn switch(bench: &mut Bench) -> (Option<bool>, bool) {
    bench.frame(Vec::new());
    let seen = bench.find("Keep pitch").expect("a Keep pitch switch");
    (seen.selected, !seen.disabled)
}

#[test]
fn keep_pitch_on_a_sound_is_one_set_effects_each_way() {
    let (mut bench, voice) = with_voice();
    assert_eq!(switch(&mut bench), (Some(false), true));
    let before = bench.steps();

    bench.click("Keep pitch");
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Keep pitch");
    let stack = &bench.clip(voice).effects;
    assert_eq!(stack.len(), 1);
    assert_eq!(stack[0].kind, EffectKind::Pitch);
    assert_eq!(pitch_of(&bench, voice), Some(0.0));
    assert_eq!(switch(&mut bench), (Some(true), true));

    bench.click("Keep pitch");
    assert_eq!(bench.steps(), before + 2);
    assert_eq!(bench.last(), "Let speed change pitch");
    assert!(bench.clip(voice).effects.is_empty());
    assert_eq!(switch(&mut bench), (Some(false), true));

    assert!(bench.editor.undo());
    assert_eq!(pitch_of(&bench, voice), Some(0.0));
    assert!(bench.editor.undo());
    assert_eq!(pitch_of(&bench, voice), None);
    assert!(bench.said.is_empty(), "{:?}", bench.said);
}

#[test]
fn a_shifted_pitch_holds_the_switch_on_and_its_speeds_are_said() {
    let (mut bench, voice) = with_voice();
    bench.edit(Command::SetEffects {
        clip_id: voice,
        effects: vec![Effect {
            params: vec![editor_core::ParamValue {
                key: ParamKey::Amount,
                value: 12.0,
            }],
            ..Effect::new(EffectKind::Pitch)
        }],
    });
    let before = bench.steps();
    assert_eq!(switch(&mut bench), (Some(true), false));
    bench.click("Keep pitch");
    assert_eq!(bench.steps(), before, "a shift is not taken away here");
    assert_eq!(pitch_of(&bench, voice), Some(12.0));
    // An octave up is held from 1x to 4x: at 1x it is.
    let past = |bench: &Bench, from: &str, to: &str| {
        bench.has(&format!("Pitch is kept from {from} to {to}."))
    };
    assert!(!past(&bench, "1×", "4×"));
    bench.edit(Command::SetPlaybackRate {
        clip_id: voice,
        rate: 0.5,
    });
    bench.frame(Vec::new());
    assert!(past(&bench, "1×", "4×"));

    // At 0 the speeds are 0.5x to 2x, and 2.5x is past them.
    let (mut bench, voice) = with_voice();
    bench.click("Keep pitch");
    bench.edit(Command::SetPlaybackRate {
        clip_id: voice,
        rate: 2.0,
    });
    bench.frame(Vec::new());
    assert!(!past(&bench, "0.5×", "2×"));
    bench.edit(Command::SetPlaybackRate {
        clip_id: voice,
        rate: 2.5,
    });
    bench.frame(Vec::new());
    assert!(past(&bench, "0.5×", "2×"));
    // Without the switch nothing is said: speed changes the pitch.
    bench.click("Keep pitch");
    assert!(!past(&bench, "0.5×", "2×"));
}

#[test]
fn a_speed_being_typed_and_a_mapped_stretch_are_said_past_what_keeps_pitch() {
    const PAST: &str = "Pitch is kept from 0.5× to 2×.";
    let (mut bench, voice) = with_voice();
    bench.click("Keep pitch");
    let before = bench.steps();
    bench.frame(Vec::new());
    assert!(!bench.has(PAST));
    // Typed into the Playback card and held, not yet a step: it is said
    // already, and giving it up takes it back.
    bench.focus("Speed");
    bench.frame(vec![Event::Text("3".to_owned())]);
    assert_eq!(bench.draft.as_ref().map(|draft| draft.rate), Some(3.0));
    assert_eq!(bench.steps(), before, "typed and held, not yet a step");
    bench.frame(Vec::new());
    assert!(bench.has(PAST));
    bench.key(Key::Escape, Modifiers::NONE);
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), before);
    assert_eq!(bench.clip(voice).playback_rate, 1.0);
    assert!(!bench.has(PAST));

    // A map is heard at the speed of each straight stretch: 2× and 1× keep
    // pitch, a stretch at 2.5× does not.
    map(&mut bench, voice, &[(0.0, 0.0), (1.0, 2.0), (2.0, 3.0)]);
    bench.frame(Vec::new());
    assert!(!bench.has(PAST));
    map(&mut bench, voice, &[(0.0, 0.0), (1.0, 2.5), (2.0, 3.5)]);
    bench.frame(Vec::new());
    assert!(bench.has(PAST));
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_picture_clip_is_told_to_detach_its_audio_to_keep_pitch() {
    let (mut bench, clip) = timed();
    bench.frame(Vec::new());
    assert!(bench.has(DETACH));
    assert!(bench.find("Keep pitch").is_none());
    // A picture whose sound is on its linked audio clip says nothing of it.
    let detached = crate::rules::detach_audio(bench.editor.project(), clip).unwrap();
    bench.edit(detached);
    bench.frame(Vec::new());
    assert!(!bench.has(DETACH));
    // Nor does a still, which has no sound.
    let (mut bench, clip) = timed();
    let id = bench.clip(clip).asset_id().unwrap();
    let mut silent = bench.editor.project().assets[&id].clone();
    silent.has_audio = false;
    silent.id = editor_core::AssetId::new_v4();
    bench.edit(Command::AddAssets {
        assets: vec![silent.clone()],
    });
    let quiet = bench.add(TrackKind::Video, {
        let mut quiet = Clip::from_asset(&silent, seconds(5.0));
        quiet.duration = seconds(2.0);
        quiet
    });
    bench.selection.only(quiet);
    bench.frame(Vec::new());
    assert!(!bench.has(DETACH));
}
