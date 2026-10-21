//! The Audio section of a clip on an audio track: the Playback card, a card
//! for each audio effect with rows from the table and no diamond, the add
//! menu with its two presets, and kinds the session's FFmpeg does not run.
use super::*;
use crate::audio::{DETACH_FIRST, UNAVAILABLE};

/// The kinds of the selected sound's stack, in order.
fn kinds(bench: &Bench, clip: ClipId) -> Vec<EffectKind> {
    (bench.clip(clip).effects.iter())
        .map(|effect| effect.kind)
        .collect()
}

/// A sound whose stack is `stack`, as a serialized command writes it.
fn with_stack(stack: &[Effect]) -> (Bench, ClipId) {
    let (mut bench, voice) = with_voice();
    bench.edit(Command::SetEffects {
        clip_id: voice,
        effects: stack.to_vec(),
    });
    bench.frame(Vec::new());
    (bench, voice)
}

fn read(bench: &Bench, name: &str) -> Option<String> {
    bench.find(name).and_then(|seen| seen.value.clone())
}

/// How many lines say a kind does not run here.
fn unavailable_lines(bench: &Bench) -> usize {
    (bench.seen.iter())
        .filter(|seen| seen.name == UNAVAILABLE && seen.role == Role::Label)
        .count()
}

#[test]
fn an_audio_clip_has_the_audio_section_under_its_playback_and_a_video_clip_has_none() {
    let (mut bench, voice) = with_voice();
    bench.frame(Vec::new());
    // One section, so no control to choose it, and the Playback card first.
    assert!(
        !bench.has("Audio"),
        "one section is not offered as a choice"
    );
    assert!(bench.has("No audio effects on this clip."));
    let playback = bench.at("Volume").y;
    assert!(bench.at("Speed").y < playback);
    assert!(playback < bench.at("Add audio effect").y);
    assert!(bench.has("Audio effects"));

    // A picture, with sound or not, has no Audio section in any of its own.
    let (mut shot, clip) = Bench::new();
    let (track, stored) = shot.editor.project().clip(clip).unwrap();
    let offered = sections(track, stored);
    assert!(!offered.contains(&Section::Audio));
    for section in offered {
        shot.state.section = *section;
        shot.frame(Vec::new());
        shot.frame(Vec::new());
        assert!(!shot.has("Add audio effect"), "{section:?}");
        assert!(!shot.has("Audio effects"), "{section:?}");
    }

    // A locked track takes nothing here either.
    let track_id = bench.track(TrackKind::Audio);
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    });
    bench.frame(Vec::new());
    assert!(
        bench
            .find("Add audio effect")
            .is_some_and(|seen| seen.disabled)
    );
    assert_eq!(kinds(&bench, voice), []);
}

#[test]
fn each_row_reads_its_unit_and_range_from_the_table_and_has_no_diamond() {
    use EffectKind as K;
    // What each field reads at the table's default, by its row's name.
    let rows: [(EffectKind, &[(&str, &str)]); 7] = [
        (K::HighPass, &[("Frequency", "80 Hz")]),
        (
            K::EqBand,
            &[
                ("Frequency", "1000 Hz"),
                ("Gain", "0.0 dB"),
                ("Width", "1.00 oct"),
            ],
        ),
        (
            K::Compressor,
            &[
                ("Threshold", "-18.0 dB"),
                ("Ratio", "3.0:1"),
                ("Make-up", "0.0 dB"),
            ],
        ),
        (K::Limiter, &[("Ceiling", "-1.0 dB")]),
        (
            K::Denoise,
            &[("Amount", "40 %"), ("Noise floor", "-40.0 dB")],
        ),
        (K::Pitch, &[("Shift", "0.0 st")]),
        // Its card is "Reduce centre"; its one row is how much.
        (K::CenterCut, &[("Amount", "100 %")]),
    ];
    for (kind, named) in rows {
        let effect = Effect::new(kind);
        let (mut bench, voice) = with_stack(std::slice::from_ref(&effect));
        bench.state.effect = Some(effect.id);
        bench.frame(Vec::new());
        bench.frame(Vec::new());
        let target = AnimTarget::Effect(kind);
        let specs = look::spec(kind);
        assert_eq!(specs.len(), named.len(), "{kind:?}");
        for (spec, (name, value)) in specs.iter().zip(named) {
            assert_eq!(read(&bench, name).as_deref(), Some(*value), "{kind:?}");
            let (label, field) = presented(target, spec);
            assert_eq!(label, *name);
            assert_eq!((field.min, field.max), (spec.min, spec.max), "{name}");
            // Hertz are dragged by a part of themselves; nothing else is.
            assert_eq!(field.log, spec.key == ParamKey::Frequency, "{name}");
            assert!(!bench.has(&format!("Animate {}", name.to_lowercase())));
        }
        // No diamond, no step between keys, and no mix: an audio effect is
        // whole and takes no keyframes.
        let diamonds = (bench.seen.iter()).filter(|seen| {
            seen.name.starts_with("Animate ") && seen.name != "Animate volume"
                || seen.name.ends_with(" keyframe")
                || seen.name.ends_with(" mix")
        });
        assert_eq!(diamonds.count(), 0, "{kind:?}");
        assert!(bench.has(&format!("Hear {}", effect_name(kind))));
        assert_eq!(bench.clip(voice).effects, [effect]);
    }
}

#[test]
fn a_committed_drag_is_one_entry_and_a_frequency_moves_by_octaves() {
    let mut low = Effect::new(EffectKind::EqBand);
    low.params = vec![editor_core::ParamValue {
        key: ParamKey::Frequency,
        value: 100.0,
    }];
    let high = {
        let mut high = low.clone();
        high.id = uuid::Uuid::new_v4();
        high.params[0].value = 1_000.0;
        high
    };
    let (mut bench, voice) = with_stack(&[low.clone(), high.clone()]);
    let mut ratios = Vec::new();
    for (index, effect) in [low, high].into_iter().enumerate() {
        bench.state.effect = Some(effect.id);
        let before = bench.steps();
        let stored = bench.editor.snapshot();
        let end = bench.drag("Frequency", vec2(30.0, 0.0));
        assert_eq!(bench.steps(), before, "a held drag is no step");
        assert_eq!(*bench.editor.project(), *stored);
        bench.release(end);
        assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
        assert_eq!(bench.last(), "Change effects");
        let moved = bench.clip(voice).effects[index].param(ParamKey::Frequency);
        let from = effect.param(ParamKey::Frequency).unwrap();
        ratios.push(moved.unwrap() / from);
        assert!(bench.editor.undo());
        assert_eq!(bench.clip(voice).effects[index], effect);
    }
    // The same drag is the same interval at 100 Hz and at 1 kHz.
    assert!(ratios[0] > 1.2, "{ratios:?}");
    assert!((ratios[0] / ratios[1] - 1.0).abs() < 0.05, "{ratios:?}");
    assert!(bench.refused.is_empty());
}

#[test]
fn an_audio_effect_is_added_switched_reordered_reset_and_removed() {
    let (mut bench, voice) = with_voice();
    let before = bench.steps();
    bench.click("Add audio effect");
    bench.frame(Vec::new());
    // The presets first, then each kind but Pitch; nothing drawn.
    for listed in [
        "Voice",
        "3-band EQ",
        "High-pass",
        "EQ band",
        "Compressor",
        "Limiter",
        "Noise reduction",
        "Reduce centre",
    ] {
        assert!(bench.has(listed), "{listed}");
    }
    assert!(bench.at("Voice").y < bench.at("3-band EQ").y);
    assert!(bench.at("3-band EQ").y < bench.at("High-pass").y);
    for absent in ["Pitch", "Glow", "Blur"] {
        assert!(!bench.has(absent), "{absent}");
    }
    bench.click("Compressor");
    assert_eq!(kinds(&bench, voice), [EffectKind::Compressor]);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Change effects");
    let compressor = bench.clip(voice).effects[0].clone();
    assert_eq!(bench.state.effect, Some(compressor.id), "its rows are open");

    bench.click("Add audio effect");
    bench.click("Limiter");
    assert_eq!(
        kinds(&bench, voice),
        [EffectKind::Compressor, EffectKind::Limiter]
    );
    bench.click("Move Limiter up");
    assert_eq!(
        kinds(&bench, voice),
        [EffectKind::Limiter, EffectKind::Compressor]
    );
    bench.click("Hear Compressor");
    assert!(!bench.clip(voice).effects[1].enabled);
    assert_eq!(bench.find("Hear Compressor").unwrap().selected, Some(false));

    // A value off its default is given back by its reset, in one step.
    bench.state.effect = Some(compressor.id);
    bench.focus("Ratio");
    bench.frame(vec![Event::Text("8".to_owned())]);
    bench.enter(Modifiers::NONE);
    let ratio = |bench: &Bench| bench.clip(voice).effects[1].param(ParamKey::Amount);
    assert_eq!(ratio(&bench), Some(8.0));
    let steps = bench.steps();
    bench.click("Reset ratio");
    assert_eq!(bench.steps(), steps + 1);
    assert_eq!(ratio(&bench), Some(3.0));

    let steps = bench.steps();
    bench.click("Remove Limiter");
    assert_eq!(kinds(&bench, voice), [EffectKind::Compressor]);
    assert_eq!(bench.steps(), steps + 1);
    assert!(bench.editor.undo());
    assert_eq!(
        kinds(&bench, voice),
        [EffectKind::Limiter, EffectKind::Compressor]
    );
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn voice_and_three_band_eq_write_fresh_effects_in_one_entry_each() {
    use EffectKind as K;
    let (mut bench, voice) = with_voice();
    let before = bench.steps();
    bench.click("Add audio effect");
    bench.click("Voice");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.said);
    assert_eq!(bench.last(), "Add Voice");
    assert_eq!(
        kinds(&bench, voice),
        [K::HighPass, K::Denoise, K::EqBand, K::Compressor]
    );
    let stack = bench.clip(voice).effects.clone();
    let value = |effect: &Effect, key| effect.param(key);
    assert_eq!(value(&stack[0], ParamKey::Frequency), Some(80.0));
    assert_eq!(value(&stack[2], ParamKey::Frequency), Some(3_500.0));
    assert_eq!(value(&stack[2], ParamKey::Gain), Some(3.0));
    assert_eq!(value(&stack[2], ParamKey::Width), Some(1.0));
    // Noise reduction and the compressor are at their defaults.
    assert!(stack[1].params.is_empty() && stack[3].params.is_empty());
    // Its first effect is open: the high-pass.
    assert_eq!(bench.state.effect, Some(stack[0].id));
    assert_eq!(read(&bench, "Frequency").as_deref(), Some("80 Hz"));

    bench.click("Add audio effect");
    bench.click("3-band EQ");
    assert_eq!(bench.steps(), before + 2);
    assert_eq!(bench.last(), "Add 3-band EQ");
    let stack = bench.clip(voice).effects.clone();
    let bands: Vec<Option<f32>> = (stack[4..].iter())
        .map(|effect| effect.param(ParamKey::Frequency))
        .collect();
    assert_eq!(bands, [Some(120.0), Some(1_000.0), Some(8_000.0)]);

    // Twice is twice, and every id is its own.
    bench.click("Add audio effect");
    bench.click("Voice");
    let stack = bench.clip(voice).effects.clone();
    assert_eq!(stack.len(), 11);
    let mut ids: Vec<_> = stack.iter().map(|effect| effect.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 11);

    for left in [7, 4, 0] {
        assert!(bench.editor.undo());
        assert_eq!(bench.clip(voice).effects.len(), left);
    }
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}

#[test]
fn voice_past_sixteen_effects_is_refused_in_words_and_changes_nothing() {
    let thirteen: Vec<Effect> = (0..13).map(|_| Effect::new(EffectKind::Limiter)).collect();
    let (mut bench, voice) = with_stack(&thirteen);
    let before = bench.editor.project().clone();
    let steps = bench.steps();
    bench.click("Add audio effect");
    bench.click("Voice");
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
    assert_eq!(
        bench.said,
        ["Voice adds 4 effects and that clip has room for 3; a clip holds 16 at most."]
    );
    // Three still fit.
    bench.click("Add audio effect");
    bench.click("3-band EQ");
    assert_eq!(bench.steps(), steps + 1);
    assert_eq!(bench.clip(voice).effects.len(), 16);
    // A full stack offers nothing more.
    bench.frame(Vec::new());
    assert!(
        bench
            .find("Add audio effect")
            .is_some_and(|seen| seen.disabled)
    );
}

/// The row of the open add menu that adds `kind`: below the button, where
/// the cards of the stack are not.
fn menu_row(bench: &Bench, kind: EffectKind) -> &Seen {
    let below = bench.at("Add audio effect").y;
    (bench.seen.iter())
        .find(|seen| {
            seen.name == effect_name(kind) && seen.role == Role::Button && seen.rect.top() > below
        })
        .unwrap_or_else(|| panic!("{kind:?} is listed in the menu"))
}

#[test]
fn an_unfinished_probe_shows_every_kind_and_a_finished_one_disables_the_missing() {
    let stack: Vec<Effect> = (EffectKind::ALL.into_iter())
        .filter(|kind| kind.is_audio())
        .map(Effect::new)
        .collect();
    let denoise = stack[4].clone();
    assert_eq!(denoise.kind, EffectKind::Denoise);
    let compressor = stack[2].clone();

    // Until the probe has answered, every kind runs; so it does once it
    // has answered that nothing is missing.
    for missing in [None, Some(Vec::new())] {
        let (mut bench, _) = with_stack(&stack);
        bench.missing = missing;
        bench.state.effect = Some(denoise.id);
        bench.frame(Vec::new());
        assert_eq!(unavailable_lines(&bench), 0);
        assert!(bench.find("Noise floor").is_some_and(|seen| !seen.disabled));
        bench.click("Add audio effect");
        for kind in crate::audio::added() {
            assert!(!menu_row(&bench, kind).disabled, "{kind:?}");
        }
    }

    // One that found two disables exactly those: their cards say so and
    // their rows take nothing.
    let (mut bench, voice) = with_stack(&stack);
    let missing = [EffectKind::Denoise, EffectKind::CenterCut];
    bench.missing = Some(missing.to_vec());
    bench.state.effect = Some(denoise.id);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert_eq!(unavailable_lines(&bench), 2);
    for name in ["Amount", "Noise floor"] {
        assert!(bench.find(name).is_some_and(|seen| seen.disabled), "{name}");
    }
    bench.state.effect = Some(compressor.id);
    bench.frame(Vec::new());
    for name in ["Threshold", "Ratio", "Make-up"] {
        assert!(
            bench.find(name).is_some_and(|seen| !seen.disabled),
            "{name}"
        );
    }
    // What the card of a missing kind still does is take it away.
    let steps = bench.steps();
    bench.click("Remove Noise reduction");
    assert_eq!(bench.steps(), steps + 1);
    assert!(!kinds(&bench, voice).contains(&EffectKind::Denoise));
    assert!(bench.refused.is_empty());
    assert_eq!(unavailable_lines(&bench), 1);
}

#[test]
fn a_missing_kind_is_added_as_every_route_adds_it_and_its_card_says_so() {
    // The menu refuses nothing the library, a phrase or an agent would
    // write: the missing kind's row and a preset holding it both add.
    let (mut bench, voice) = with_voice();
    bench.missing = Some(vec![EffectKind::Denoise]);
    bench.frame(Vec::new());
    bench.click("Add audio effect");
    for kind in crate::audio::added() {
        assert!(!menu_row(&bench, kind).disabled, "{kind:?}");
    }
    assert!(bench.find("Voice").is_some_and(|seen| !seen.disabled));
    bench.click("Voice");
    use EffectKind as K;
    assert_eq!(
        kinds(&bench, voice),
        [K::HighPass, K::Denoise, K::EqBand, K::Compressor]
    );
    assert_eq!(bench.last(), "Add Voice");
    bench.frame(Vec::new());
    assert_eq!(unavailable_lines(&bench), 1);
    bench.click("Add audio effect");
    // The menu's row, not the card of the same name above it.
    let row = menu_row(&bench, K::Denoise).rect.center();
    bench.press(row);
    bench.release(row);
    assert_eq!(kinds(&bench, voice).len(), 5);
    assert_eq!(unavailable_lines(&bench), 2);
    assert!(bench.refused.is_empty());

    // The library's tile adds it too, through the same rule.
    let project = bench.editor.project();
    let given =
        crate::audio::give_command(project, &[voice], crate::audio::Given::Kind(K::Denoise));
    assert!(given.is_ok());
}

#[test]
fn a_picture_clip_with_sound_is_told_to_detach_it_first() {
    // The words a library tile or a phrase gives a picture with sound.
    let (bench, clip) = Bench::new();
    let given = crate::audio::give_command(
        bench.editor.project(),
        &[clip],
        crate::audio::Given::Preset(crate::audio::Preset::Voice),
    );
    assert_eq!(given.unwrap_err(), DETACH_FIRST);
}

#[test]
fn the_audio_section_fits_a_narrow_panel() {
    for width in [300.0, 240.0] {
        let (mut bench, voice) = with_voice();
        bench.click("Add audio effect");
        bench.click("Voice");
        let stack = bench.clip(voice).effects.clone();
        // The widest rows: the presence band's three, its reset among them.
        bench.state.effect = Some(stack[2].id);
        bench.missing = Some(vec![EffectKind::Denoise]);
        bench.size = vec2(width, 2400.0);
        bench.frame(Vec::new());
        bench.frame(Vec::new());
        assert!(bench.has("Reset gain"));
        let panel = Rect::from_min_size(Pos2::ZERO, bench.size);
        let mut controls = 0;
        for seen in bench.seen.iter().filter(|seen| !Bench::is_text(seen)) {
            if seen.rect.width() >= width - 1.0 {
                continue;
            }
            controls += 1;
            // Cards span the panel and their key column ends 8 in (spec
            // §5.4, §4.3.3).
            assert!(
                fits_cards(seen, panel),
                "at {width}: \"{}\" is at {:?}",
                seen.name,
                seen.rect
            );
            assert!(!seen.name.trim().is_empty());
        }
        assert!(controls > 10, "at {width}: {controls}");
    }
}

#[test]
fn the_audio_section_asks_for_no_frame_while_nothing_is_done() {
    let (mut bench, voice) = with_voice();
    bench.click("Add audio effect");
    bench.click("Voice");
    bench.missing = Some(vec![EffectKind::Denoise]);
    // What a first frame eases in has settled within a second.
    for _ in 0..30 {
        bench.frame(Vec::new());
    }
    assert!(
        bench.wait > Duration::from_secs(3600),
        "asked for a frame in {:?}",
        bench.wait
    );
    assert!(bench.draft.is_none() && bench.refused.is_empty());
    assert_eq!(bench.clip(voice).effects.len(), 4);
}

#[test]
fn ducking_is_asked_from_the_audio_section_at_the_depth_set_there() {
    use crate::{duck::LABEL, ui::DuckProgress};
    let (mut bench, voice) = with_voice();
    // Under the Playback card and above the audio effects.
    assert!(bench.at("Fade out").y < bench.at(LABEL).y);
    assert!(bench.at(LABEL).y < bench.at("Add audio effect").y);
    assert!(bench.at("Depth").y < bench.at(LABEL).y);
    // Asked for by the clip's id at the depth shown, -12 dB, and nothing
    // is edited until the others' sound is read.
    let before = bench.steps();
    bench.click(LABEL);
    assert_eq!(bench.ducked.len(), 1);
    assert_eq!(bench.ducked[0].0, [voice]);
    assert!((bench.ducked[0].1 - crate::duck::DEPTH).abs() < 1e-4);
    assert_eq!(bench.steps(), before);
    // Another depth set in the field is the one asked for.
    bench.state.duck_db = -18.0;
    bench.click(LABEL);
    let deeper = bench.ducked[1].1;
    assert!((deeper - 0.125_89).abs() < 1e-4, "{deeper}");

    // While a duck of the clip reads the others' sound: how far it is, and
    // the way to stop it, in place of the button.
    bench.state.set_ducking(Some(DuckProgress {
        clips: vec![voice],
        reading: Some("speech.wav".to_owned()),
        taken: 1,
        total: 3,
    }));
    bench.frame(Vec::new());
    assert!(bench.has("Reading speech.wav, 2 of 3"));
    assert!(bench.find(LABEL).is_none());
    bench.click("Stop ducking");
    assert_eq!(bench.cancels, 1);
    bench.state.set_ducking(None);
    bench.frame(Vec::new());
    assert!(bench.find(LABEL).is_some());

    // On a muted track it is shown and not to be chosen.
    bench.edit(Command::SetTrackState {
        track_id: bench.track(TrackKind::Audio),
        locked: None,
        hidden: None,
        muted: Some(true),
        name: None,
        gain: None,
        solo: None,
    });
    bench.frame(Vec::new());
    assert!(bench.find(LABEL).is_some_and(|seen| seen.disabled));
    bench.click(LABEL);
    assert_eq!(bench.ducked.len(), 2, "a disabled button asks nothing");

    // A picture has no Audio section, and so no ducking of its own.
    let (mut shot, _) = Bench::new();
    shot.frame(Vec::new());
    assert!(shot.find(LABEL).is_none());
}
