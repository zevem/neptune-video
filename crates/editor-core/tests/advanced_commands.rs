use editor_core::{
    AnimTrack, Asset, BlendMode, Clip, ClipId, Command, Ease, EditError, Editor, Effect,
    EffectKind, Follow, Keyframe, LookPreset, Mask, MaskShape, MediaKind, ParamKey, ParamValue,
    Project, Retime, Time, TimeKey, TrackKey, Tracker, Transition, TransitionEdge, TransitionKind,
    look,
};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

const EASE: Ease = Ease::Bezier {
    x1: 0.42,
    y1: 0.0,
    x2: 0.58,
    y2: 1.0,
};
const EASE_IN: Ease = Ease::Bezier {
    x1: 0.42,
    y1: 0.0,
    x2: 1.0,
    y2: 1.0,
};
const OVERSHOOT: Ease = Ease::Bezier {
    x1: 0.3,
    y1: 1.4,
    x2: 0.7,
    y2: 1.0,
};

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

struct Fixture {
    editor: Editor,
    asset: Uuid,
    video: ClipId,
    audio: ClipId,
    text: ClipId,
}

/// A 60 s asset with sound; a 10 s video clip of it starting at 2 s from
/// source 5 s, the same span on the audio track, and a 10 s title.
fn fixture() -> Fixture {
    let mut project = Project::new("Advanced");
    let mut asset = Asset::new(PathBuf::from("a.mp4"), MediaKind::Video, seconds(60.0));
    asset.has_audio = true;
    let mut video = Clip::from_asset(&asset, seconds(2.0));
    video.source_in = seconds(5.0);
    video.duration = seconds(10.0);
    let mut audio = video.clone();
    audio.id = Uuid::new_v4();
    let text = Clip::text("Title", seconds(2.0), seconds(10.0));
    let ids = (asset.id, video.id, audio.id, text.id);
    Arc::make_mut(&mut project.assets).insert(asset.id, asset);
    Arc::make_mut(&mut project.tracks[0]).clips.push(video);
    Arc::make_mut(&mut project.tracks[1]).clips.push(audio);
    Arc::make_mut(&mut project.tracks[2]).clips.push(text);
    Fixture {
        editor: Editor::new(project).unwrap(),
        asset: ids.0,
        video: ids.1,
        audio: ids.2,
        text: ids.3,
    }
}

/// A second 10 s clip of the fixture's asset, later on the video track.
fn second_video(f: &mut Fixture) -> ClipId {
    let mut clip = clip(&f.editor, f.video).clone();
    clip.id = Uuid::new_v4();
    clip.start = seconds(20.0);
    let id = clip.id;
    let track_id = f.editor.project().tracks[0].id;
    f.editor
        .dispatch(Command::AddClip { track_id, clip })
        .unwrap();
    id
}

fn clip(editor: &Editor, id: ClipId) -> &Clip {
    editor.project().clip(id).unwrap().1
}

fn key(at: f64, value: f32, ease: Ease) -> Keyframe {
    Keyframe {
        at: seconds(at),
        value,
        ease,
    }
}

fn time_key(at: f64, source: f64, ease: Ease) -> TimeKey {
    TimeKey {
        at: seconds(at),
        source: seconds(source),
        ease,
    }
}

fn keyframes(clip_id: ClipId, target: Option<Uuid>, key: ParamKey, keys: Vec<Keyframe>) -> Command {
    Command::SetKeyframes {
        clip_id,
        target,
        key,
        keys,
    }
}

fn fade(duration: f64) -> Transition {
    Transition {
        kind: TransitionKind::Fade,
        duration: seconds(duration),
        ease: Ease::Linear,
        params: Vec::new(),
    }
}

fn tracker(asset_id: Uuid) -> Tracker {
    Tracker {
        id: Uuid::new_v4(),
        name: "Face".into(),
        asset_id,
        keys: vec![TrackKey {
            source: seconds(1.0),
            center: [0.5, 0.5],
            scale: 1.0,
            rotation: 0.0,
        }],
    }
}

fn follow(tracker_id: Uuid) -> Follow {
    Follow {
        tracker_id,
        position: true,
        scale: false,
        rotation: false,
        offset: [0.0, 0.0],
    }
}

/// The command applies, undo restores the exact prior root and redo the exact result.
fn accepted(editor: &mut Editor, command: Command) {
    let before = editor.snapshot();
    let outcome = editor.dispatch(command).unwrap();
    assert!(outcome.changed);
    let after = editor.snapshot();
    assert_ne!(*before, *after);
    assert!(editor.undo());
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    assert!(editor.redo());
    assert!(Arc::ptr_eq(&after, &editor.snapshot()));
}

/// The command fails and leaves the document, revision and history alone.
fn rejected(editor: &mut Editor, command: Command) -> EditError {
    let (before, revision) = (editor.snapshot(), editor.revision());
    let depth = editor.history().count();
    let error = editor.dispatch(command).unwrap_err();
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    assert_eq!(revision, editor.revision());
    assert_eq!(depth, editor.history().count());
    error
}

fn invalid(editor: &mut Editor, command: Command, what: &str) {
    match rejected(editor, command) {
        EditError::Invalid(found) => assert_eq!(found, what),
        other => panic!("expected invalid {what}, found {other:?}"),
    }
}

#[test]
fn set_effects_replaces_the_stack_and_rejects_bad_values() {
    let mut f = fixture();
    let glow = Effect::new(EffectKind::Glow);
    let blur = Effect::new(EffectKind::Blur);
    accepted(
        &mut f.editor,
        Command::SetEffects {
            clip_id: f.video,
            effects: vec![glow.clone(), blur.clone()],
        },
    );
    assert_eq!(clip(&f.editor, f.video).effects, [glow.clone(), blur]);
    assert_eq!(f.editor.undo_label(), Some("Change effects"));

    let mut loud = glow.clone();
    loud.mix = 1.5;
    let effects = |effects| Command::SetEffects {
        clip_id: f.video,
        effects,
    };
    invalid(&mut f.editor, effects(vec![loud]), "effect");
    invalid(&mut f.editor, effects(vec![glow.clone(), glow]), "effect");
    let mut foreign = Effect::new(EffectKind::Blur);
    foreign.params.push(ParamValue {
        key: ParamKey::Hue,
        value: 0.0,
    });
    invalid(&mut f.editor, effects(vec![foreign]), "effect");
    let many = (0..17).map(|_| Effect::new(EffectKind::Blur)).collect();
    invalid(&mut f.editor, effects(many), "effect or mask count");
    let missing = Uuid::new_v4();
    let mut lut = Effect::new(EffectKind::Lut);
    lut.asset_id = Some(missing);
    assert!(matches!(
        rejected(&mut f.editor, effects(vec![lut])),
        EditError::Missing(id) if id == missing
    ));
    invalid(
        &mut f.editor,
        Command::SetEffects {
            clip_id: f.text,
            effects: vec![Effect::new(EffectKind::Blur)],
        },
        "text clip look",
    );
    let absent = Uuid::new_v4();
    assert!(matches!(
        rejected(
            &mut f.editor,
            Command::SetEffects {
                clip_id: absent,
                effects: Vec::new()
            }
        ),
        EditError::Missing(id) if id == absent
    ));
}

#[test]
fn set_masks_and_blend_mode_validate() {
    let mut f = fixture();
    let mask = Mask::new(MaskShape::Ellipse);
    accepted(
        &mut f.editor,
        Command::SetMasks {
            clip_id: f.video,
            masks: vec![mask.clone()],
        },
    );
    assert_eq!(clip(&f.editor, f.video).masks, std::slice::from_ref(&mask));
    let mut feathered = mask.clone();
    feathered.feather = f32::NAN;
    invalid(
        &mut f.editor,
        Command::SetMasks {
            clip_id: f.video,
            masks: vec![feathered],
        },
        "mask",
    );
    invalid(
        &mut f.editor,
        Command::SetMasks {
            clip_id: f.text,
            masks: vec![mask],
        },
        "text clip look",
    );

    accepted(
        &mut f.editor,
        Command::SetBlendMode {
            clip_id: f.video,
            mode: BlendMode::Screen,
        },
    );
    assert_eq!(clip(&f.editor, f.video).blend, BlendMode::Screen);
    invalid(
        &mut f.editor,
        Command::SetBlendMode {
            clip_id: f.text,
            mode: BlendMode::Multiply,
        },
        "text clip look",
    );
    let same = f
        .editor
        .dispatch(Command::SetBlendMode {
            clip_id: f.video,
            mode: BlendMode::Screen,
        })
        .unwrap();
    assert!(!same.changed);
}

#[test]
fn set_keyframes_replaces_one_track_and_empty_removes_it() {
    let mut f = fixture();
    let first = vec![key(0.0, 0.0, EASE), key(4.0, 100.0, Ease::Linear)];
    accepted(
        &mut f.editor,
        keyframes(f.video, None, ParamKey::PositionX, first.clone()),
    );
    accepted(
        &mut f.editor,
        keyframes(
            f.text,
            None,
            ParamKey::Opacity,
            vec![key(0.0, 0.0, Ease::Linear), key(1.0, 1.0, Ease::Linear)],
        ),
    );
    accepted(
        &mut f.editor,
        keyframes(
            f.video,
            None,
            ParamKey::Opacity,
            vec![key(1.0, 0.5, Ease::Hold)],
        ),
    );
    let replaced = vec![key(2.0, 7.0, Ease::Hold)];
    accepted(
        &mut f.editor,
        keyframes(f.video, None, ParamKey::PositionX, replaced.clone()),
    );
    let animation = &clip(&f.editor, f.video).animation;
    assert_eq!(animation.len(), 2);
    assert_eq!(animation[0].key, ParamKey::PositionX);
    assert_eq!(animation[0].keys, replaced);

    accepted(
        &mut f.editor,
        keyframes(f.video, None, ParamKey::PositionX, Vec::new()),
    );
    let animation = &clip(&f.editor, f.video).animation;
    assert_eq!(animation.len(), 1);
    assert_eq!(animation[0].key, ParamKey::Opacity);
    let absent = f
        .editor
        .dispatch(keyframes(f.video, None, ParamKey::PositionX, Vec::new()))
        .unwrap();
    assert!(!absent.changed);

    let video = f.video;
    let position = |keys| keyframes(video, None, ParamKey::PositionX, keys);
    invalid(
        &mut f.editor,
        position(vec![key(3.0, 0.0, EASE), key(3.0, 1.0, EASE)]),
        "animation keys",
    );
    invalid(
        &mut f.editor,
        position(vec![key(10.5, 0.0, EASE)]),
        "animation keys",
    );
    invalid(
        &mut f.editor,
        position(vec![key(1.0, f32::NAN, EASE)]),
        "animation keys",
    );
    invalid(
        &mut f.editor,
        keyframes(
            video,
            None,
            ParamKey::Opacity,
            vec![key(1.0, 1.5, Ease::Linear)],
        ),
        "animation keys",
    );
    invalid(
        &mut f.editor,
        keyframes(video, None, ParamKey::Hue, first.clone()),
        "animation parameter",
    );
    let stranger = Uuid::new_v4();
    assert!(matches!(
        rejected(
            &mut f.editor,
            keyframes(video, Some(stranger), ParamKey::Radius, first)
        ),
        EditError::Missing(id) if id == stranger
    ));
}

#[test]
fn replacing_effects_or_masks_prunes_orphan_animation() {
    let mut f = fixture();
    let (glow, blur) = (Effect::new(EffectKind::Glow), Effect::new(EffectKind::Blur));
    let mask = Mask::new(MaskShape::Rectangle);
    let radius = vec![key(0.0, 0.0, EASE), key(2.0, 0.1, Ease::Linear)];
    for command in [
        Command::SetEffects {
            clip_id: f.video,
            effects: vec![glow.clone(), blur.clone()],
        },
        Command::SetMasks {
            clip_id: f.video,
            masks: vec![mask.clone()],
        },
        keyframes(f.video, Some(glow.id), ParamKey::Radius, radius.clone()),
        keyframes(f.video, Some(blur.id), ParamKey::Radius, radius.clone()),
        keyframes(f.video, Some(mask.id), ParamKey::Feather, radius.clone()),
        keyframes(f.video, None, ParamKey::Opacity, radius.clone()),
    ] {
        accepted(&mut f.editor, command);
    }
    let targets = |editor: &Editor| -> Vec<Option<Uuid>> {
        clip(editor, f.video)
            .animation
            .iter()
            .map(|t| t.target)
            .collect()
    };

    accepted(
        &mut f.editor,
        Command::SetEffects {
            clip_id: f.video,
            effects: vec![blur.clone()],
        },
    );
    assert_eq!(
        targets(&f.editor),
        [Some(blur.id), Some(mask.id), None],
        "only the removed effect's track goes"
    );
    accepted(
        &mut f.editor,
        Command::SetMasks {
            clip_id: f.video,
            masks: Vec::new(),
        },
    );
    assert_eq!(targets(&f.editor), [Some(blur.id), None]);
    assert!(f.editor.undo());
    assert_eq!(targets(&f.editor), [Some(blur.id), Some(mask.id), None]);
}

#[test]
fn set_retime_follows_links_and_bakes_the_rate() {
    let mut f = fixture();
    f.editor
        .dispatch(Command::LinkClips {
            clip_ids: vec![f.video, f.audio],
        })
        .unwrap();
    f.editor
        .dispatch(Command::SetPlaybackRate {
            clip_id: f.video,
            rate: 2.0,
        })
        .unwrap();
    assert_eq!(clip(&f.editor, f.audio).duration, seconds(5.0));

    // Half speed for two seconds of clip time, a freeze, then backwards.
    let retime = Retime {
        keys: vec![
            time_key(0.0, 0.0, Ease::Linear),
            time_key(2.0, 1.0, Ease::Linear),
            time_key(3.0, 1.0, EASE),
            time_key(6.0, 0.25, Ease::Linear),
        ],
    };
    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.video,
            retime: Some(retime),
            duration: seconds(6.0),
        },
    );
    assert_eq!(f.editor.undo_label(), Some("Retime clip"));
    for id in [f.video, f.audio] {
        let clip = clip(&f.editor, id);
        assert_eq!(clip.playback_rate, 1.0);
        assert_eq!(clip.duration, seconds(6.0));
        let sources: Vec<_> = clip
            .retime
            .as_ref()
            .unwrap()
            .keys
            .iter()
            .map(|k| k.source)
            .collect();
        let baked = [0.0, 2.0, 2.0, 0.5].map(seconds);
        assert_eq!(sources, baked, "rate 2 is baked into the sources");
        assert_eq!(clip.source_time(seconds(2.0 + 1.0)), seconds(5.0 + 1.0));
    }

    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.audio,
            retime: None,
            duration: seconds(4.0),
        },
    );
    for id in [f.video, f.audio] {
        let clip = clip(&f.editor, id);
        assert_eq!(clip.retime, None);
        assert_eq!((clip.playback_rate, clip.duration), (1.0, seconds(4.0)));
    }
}

#[test]
fn set_retime_rejects_maps_the_clip_cannot_play() {
    let mut f = fixture();
    let video = f.video;
    let retime = |keys: Vec<TimeKey>, duration: f64| Command::SetRetime {
        clip_id: video,
        retime: Some(Retime { keys }),
        duration: seconds(duration),
    };
    let line = |to: f64, source: f64| {
        vec![
            time_key(0.0, 0.0, Ease::Linear),
            time_key(to, source, Ease::Linear),
        ]
    };
    invalid(&mut f.editor, retime(line(4.0, 4.0), 5.0), "time map");
    invalid(&mut f.editor, retime(line(1.0, 17.0), 1.0), "time map");
    invalid(&mut f.editor, retime(line(10.0, 0.4), 10.0), "time map");
    invalid(
        &mut f.editor,
        retime(vec![time_key(0.0, 0.0, Ease::Linear)], 10.0),
        "time map",
    );
    invalid(
        &mut f.editor,
        retime(line(10.0, 56.0), 10.0),
        "clip extends beyond source",
    );
    invalid(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.text,
            retime: Some(Retime::constant(seconds(10.0), 1.0)),
            duration: seconds(10.0),
        },
        "text clip look",
    );
    accepted(&mut f.editor, retime(line(10.0, 55.0), 10.0));
}

#[test]
fn set_transition_addresses_one_edge() {
    let mut f = fixture();
    let edge = |clip_id, edge, transition| Command::SetTransition {
        clip_id,
        edge,
        transition,
    };
    accepted(
        &mut f.editor,
        edge(f.video, TransitionEdge::In, Some(fade(1.0))),
    );
    assert_eq!(f.editor.undo_label(), Some("Set transition"));
    accepted(
        &mut f.editor,
        edge(f.video, TransitionEdge::Out, Some(fade(2.0))),
    );
    accepted(
        &mut f.editor,
        edge(f.text, TransitionEdge::In, Some(fade(0.5))),
    );
    let video = clip(&f.editor, f.video);
    assert_eq!(video.transition_in, Some(fade(1.0)));
    assert_eq!(video.transition_out, Some(fade(2.0)));

    invalid(
        &mut f.editor,
        edge(f.video, TransitionEdge::In, Some(fade(10.5))),
        "transition",
    );
    invalid(
        &mut f.editor,
        edge(f.video, TransitionEdge::Out, Some(fade(0.001))),
        "transition",
    );
    let mut soft = fade(1.0);
    soft.params.push(ParamValue {
        key: ParamKey::Softness,
        value: 0.5,
    });
    invalid(
        &mut f.editor,
        edge(f.video, TransitionEdge::In, Some(soft)),
        "transition",
    );

    accepted(&mut f.editor, edge(f.video, TransitionEdge::In, None));
    assert_eq!(f.editor.undo_label(), Some("Remove transition"));
    let video = clip(&f.editor, f.video);
    assert_eq!(video.transition_in, None);
    assert_eq!(video.transition_out, Some(fade(2.0)));

    let json = serde_json::to_value(edge(f.video, TransitionEdge::Out, None)).unwrap();
    assert_eq!(json["type"], "set_transition");
    assert_eq!(json["edge"], "out");
}

#[test]
fn apply_preset_appends_ordinary_effects_with_fresh_ids() {
    let presets = [
        LookPreset::Cinematic,
        LookPreset::TealOrange,
        LookPreset::Vintage,
        LookPreset::Faded,
        LookPreset::Noir,
        LookPreset::Warm,
        LookPreset::Cool,
    ];
    for preset in presets {
        for strength in [0.0, 0.5, 1.0] {
            let mut f = fixture();
            let other = second_video(&mut f);
            let existing = Effect::new(EffectKind::Blur);
            f.editor
                .dispatch(Command::SetEffects {
                    clip_id: f.video,
                    effects: vec![existing.clone()],
                })
                .unwrap();
            accepted(
                &mut f.editor,
                Command::ApplyPreset {
                    clip_ids: vec![f.video, other],
                    preset,
                    strength,
                },
            );
            let (video, audio) = (clip(&f.editor, f.video), clip(&f.editor, other));
            assert_eq!(video.effects[0], existing);
            assert_eq!(video.effects.len(), audio.effects.len() + 1);
            assert_eq!(video.effects[1].kind, EffectKind::Grade, "{preset:?}");
            for (a, b) in video.effects[1..].iter().zip(&audio.effects) {
                assert_ne!(a.id, b.id);
                assert_eq!(
                    (a.kind, &a.params, &a.points),
                    (b.kind, &b.params, &b.points)
                );
            }
        }
    }

    let neutral = look::preset_effects(LookPreset::Vintage, 0.0);
    let full = look::preset_effects(LookPreset::Vintage, 1.0);
    let grade = &neutral[0];
    for spec in look::spec(EffectKind::Grade) {
        assert_eq!(grade.param(spec.key), Some(spec.default));
    }
    assert_ne!(full[0].params, grade.params);
    for effect in &neutral {
        assert!(effect.points.iter().all(|[x, y]| x == y));
        if matches!(effect.kind, EffectKind::Grain | EffectKind::Vignette) {
            assert_eq!(effect.param(ParamKey::Amount), Some(0.0));
        }
    }
    assert!(full.iter().any(|e| e.kind == EffectKind::Grain));
    assert!(full.iter().any(|e| e.kind == EffectKind::Vignette));
    assert!(full.iter().any(|e| e.kind == EffectKind::CurveBlue));
    let half = look::preset_effects(LookPreset::Warm, 0.5);
    assert_eq!(half[0].param(ParamKey::Temperature), Some(0.15));

    let mut f = fixture();
    let preset = |clip_ids, strength| Command::ApplyPreset {
        clip_ids,
        preset: LookPreset::Noir,
        strength,
    };
    invalid(&mut f.editor, preset(vec![f.video], 1.01), "look strength");
    invalid(&mut f.editor, preset(vec![f.video], -0.1), "look strength");
    invalid(
        &mut f.editor,
        preset(vec![f.video], f32::NAN),
        "look strength",
    );
    invalid(
        &mut f.editor,
        preset(vec![f.video, f.text], 1.0),
        "text clip look",
    );
    // A look takes the place of the one the clip has, so six are one.
    let twice = Command::Batch {
        label: "Looks".into(),
        commands: (0..6).map(|_| preset(vec![f.video], 1.0)).collect(),
    };
    accepted(&mut f.editor, twice);
    assert_eq!(clip(&f.editor, f.video).effects.len(), 3);
    assert!(f.editor.undo());
    f.editor
        .dispatch(Command::SetEffects {
            clip_id: f.video,
            effects: (0..14).map(|_| Effect::new(EffectKind::Blur)).collect(),
        })
        .unwrap();
    invalid(
        &mut f.editor,
        preset(vec![f.video], 1.0),
        "effect or mask count",
    );
}

#[test]
fn a_clip_holds_one_look_that_is_read_from_its_effects() {
    let mut f = fixture();
    let look = |preset, strength| Command::ApplyPreset {
        clip_ids: vec![f.video],
        preset,
        strength,
    };
    let glow = Effect::new(EffectKind::Glow);
    f.editor
        .dispatch(Command::SetEffects {
            clip_id: f.video,
            effects: vec![Effect::new(EffectKind::Blur)],
        })
        .unwrap();
    assert_eq!(look::applied_look(&clip(&f.editor, f.video).effects), None);
    for preset in LookPreset::ALL {
        for strength in [0.01, 0.37, 0.5, 1.0] {
            accepted(&mut f.editor, look(preset, strength));
            let effects = &clip(&f.editor, f.video).effects;
            let held = look::applied_look(effects).expect("a look");
            assert_eq!((held.preset, held.at), (preset, 1), "{preset:?} {strength}");
            assert_eq!(held.len, effects.len() - 1, "the last look is gone");
            assert!((held.strength - strength).abs() < 1e-3, "{held:?}");
            // It is the stored effects that say so: a round trip keeps it.
            let json = serde_json::to_string(effects).unwrap();
            let read: Vec<Effect> = serde_json::from_str(&json).unwrap();
            assert_eq!(look::applied_look(&read), Some(held));
        }
    }
    // The look made again keeps its effects, and so their keyframes; another
    // look takes its place in the stack among the clip's own effects.
    accepted(&mut f.editor, look(LookPreset::Noir, 1.0));
    let mut effects = clip(&f.editor, f.video).effects.clone();
    effects.push(glow.clone());
    f.editor
        .dispatch(Command::SetEffects {
            clip_id: f.video,
            effects,
        })
        .unwrap();
    let grade = clip(&f.editor, f.video).effects[1].id;
    f.editor
        .dispatch(keyframes(
            f.video,
            Some(grade),
            ParamKey::Contrast,
            vec![key(0.0, 0.0, Ease::Linear), key(2.0, 0.2, Ease::Linear)],
        ))
        .unwrap();
    accepted(&mut f.editor, look(LookPreset::Noir, 0.5));
    let toned = clip(&f.editor, f.video);
    assert_eq!(toned.effects[1].id, grade);
    assert_eq!(toned.animation.len(), 1);
    accepted(&mut f.editor, look(LookPreset::Warm, 0.5));
    let warm = clip(&f.editor, f.video);
    let kinds: Vec<_> = warm.effects.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [EffectKind::Blur, EffectKind::Grade, EffectKind::Glow]
    );
    assert!(
        warm.animation.is_empty(),
        "the keyframes went with the look"
    );

    // An effect of the look that was adjusted is the clip's own from then on.
    let mut effects = warm.effects.clone();
    effects[1].params[0].value += 0.1;
    f.editor
        .dispatch(Command::SetEffects {
            clip_id: f.video,
            effects,
        })
        .unwrap();
    assert_eq!(look::applied_look(&clip(&f.editor, f.video).effects), None);
    accepted(&mut f.editor, look(LookPreset::Cool, 1.0));
    assert_eq!(clip(&f.editor, f.video).effects.len(), 4);
    let taken = look::with_look(&clip(&f.editor, f.video).effects, None);
    assert_eq!(taken.len(), 3);
}

#[test]
fn trackers_upsert_and_removal_clears_every_follow() {
    let mut f = fixture();
    let mut face = tracker(f.asset);
    accepted(
        &mut f.editor,
        Command::SetTracker {
            tracker: face.clone(),
        },
    );
    assert_eq!(f.editor.undo_label(), Some("Set tracker"));
    face.name = "Hand".into();
    accepted(
        &mut f.editor,
        Command::SetTracker {
            tracker: face.clone(),
        },
    );
    assert_eq!(f.editor.project().trackers.len(), 1);
    assert_eq!(f.editor.project().trackers[&face.id].name, "Hand");

    let mut empty = tracker(f.asset);
    empty.keys.clear();
    invalid(
        &mut f.editor,
        Command::SetTracker { tracker: empty },
        "tracker",
    );
    let orphan = tracker(Uuid::new_v4());
    let asset = orphan.asset_id;
    assert!(matches!(
        rejected(&mut f.editor, Command::SetTracker { tracker: orphan }),
        EditError::Missing(id) if id == asset
    ));

    let stranger = Uuid::new_v4();
    assert!(matches!(
        rejected(
            &mut f.editor,
            Command::SetFollow {
                clip_id: f.video,
                follow: Some(follow(stranger))
            }
        ),
        EditError::Missing(id) if id == stranger
    ));
    let mut far = follow(face.id);
    far.offset = [f32::INFINITY, 0.0];
    invalid(
        &mut f.editor,
        Command::SetFollow {
            clip_id: f.video,
            follow: Some(far),
        },
        "follow offset",
    );
    let other = tracker(f.asset);
    f.editor
        .dispatch(Command::SetTracker {
            tracker: other.clone(),
        })
        .unwrap();
    let second = second_video(&mut f);
    for (clip_id, tracker_id) in [(f.video, face.id), (f.text, face.id), (second, other.id)] {
        accepted(
            &mut f.editor,
            Command::SetFollow {
                clip_id,
                follow: Some(follow(tracker_id)),
            },
        );
    }

    accepted(
        &mut f.editor,
        Command::RemoveTracker {
            tracker_id: face.id,
        },
    );
    assert!(!f.editor.project().trackers.contains_key(&face.id));
    assert_eq!(clip(&f.editor, f.video).follow, None);
    assert_eq!(clip(&f.editor, f.text).follow, None);
    assert_eq!(clip(&f.editor, second).follow, Some(follow(other.id)));
    assert!(f.editor.undo());
    assert_eq!(clip(&f.editor, f.video).follow, Some(follow(face.id)));
    assert!(f.editor.redo());
    assert!(matches!(
        rejected(
            &mut f.editor,
            Command::RemoveTracker {
                tracker_id: face.id
            }
        ),
        EditError::Missing(id) if id == face.id
    ));
    accepted(
        &mut f.editor,
        Command::SetFollow {
            clip_id: second,
            follow: None,
        },
    );
}

#[test]
fn failed_batch_of_new_commands_is_atomic() {
    let mut f = fixture();
    let face = tracker(f.asset);
    let batch = Command::Batch {
        label: "Look".into(),
        commands: vec![
            Command::SetTracker {
                tracker: face.clone(),
            },
            Command::SetFollow {
                clip_id: f.video,
                follow: Some(follow(face.id)),
            },
            Command::SetEffects {
                clip_id: f.video,
                effects: vec![Effect::new(EffectKind::Glow)],
            },
            Command::SetBlendMode {
                clip_id: f.video,
                mode: BlendMode::Add,
            },
            keyframes(
                f.video,
                None,
                ParamKey::Opacity,
                vec![key(0.0, 2.0, Ease::Linear)],
            ),
        ],
    };
    invalid(&mut f.editor, batch, "animation keys");
    assert!(f.editor.project().trackers.is_empty());
    let video = clip(&f.editor, f.video);
    assert!(video.effects.is_empty() && video.follow.is_none());
    assert!(!f.editor.can_undo());
}

#[test]
fn locked_track_refuses_every_clip_command() {
    let mut f = fixture();
    let face = tracker(f.asset);
    f.editor
        .dispatch(Command::Batch {
            label: "Setup".into(),
            commands: vec![
                Command::SetTracker {
                    tracker: face.clone(),
                },
                Command::SetFollow {
                    clip_id: f.video,
                    follow: Some(follow(face.id)),
                },
                Command::SetTrackState {
                    track_id: f.editor.project().tracks[0].id,
                    locked: Some(true),
                    hidden: None,
                    muted: None,
                    name: None,
                },
            ],
        })
        .unwrap();
    let clip_id = f.video;
    let commands = [
        Command::SetEffects {
            clip_id,
            effects: vec![Effect::new(EffectKind::Glow)],
        },
        Command::SetMasks {
            clip_id,
            masks: vec![Mask::new(MaskShape::Ellipse)],
        },
        Command::SetBlendMode {
            clip_id,
            mode: BlendMode::Add,
        },
        keyframes(
            clip_id,
            None,
            ParamKey::Opacity,
            vec![key(0.0, 0.5, Ease::Linear)],
        ),
        Command::SetRetime {
            clip_id,
            retime: Some(Retime::constant(seconds(10.0), 1.0)),
            duration: seconds(10.0),
        },
        Command::SetTransition {
            clip_id,
            edge: TransitionEdge::In,
            transition: Some(fade(1.0)),
        },
        Command::ApplyPreset {
            clip_ids: vec![clip_id],
            preset: LookPreset::Warm,
            strength: 1.0,
        },
        Command::SetFollow {
            clip_id,
            follow: None,
        },
        Command::RemoveTracker {
            tracker_id: face.id,
        },
    ];
    for command in commands {
        let label = command.label().to_owned();
        assert!(
            matches!(rejected(&mut f.editor, command), EditError::Locked),
            "{label}"
        );
    }
    // A linked partner on a locked track refuses the whole retime.
    let mut g = fixture();
    g.editor
        .dispatch(Command::Batch {
            label: "Setup".into(),
            commands: vec![
                Command::LinkClips {
                    clip_ids: vec![g.video, g.audio],
                },
                Command::SetTrackState {
                    track_id: g.editor.project().tracks[1].id,
                    locked: Some(true),
                    hidden: None,
                    muted: None,
                    name: None,
                },
            ],
        })
        .unwrap();
    let retime = Command::SetRetime {
        clip_id: g.video,
        retime: Some(Retime::constant(seconds(10.0), 1.0)),
        duration: seconds(10.0),
    };
    assert!(matches!(rejected(&mut g.editor, retime), EditError::Locked));
}

/// The clip of the fixture's video track showing timeline `at`; an edge
/// belongs to the clip that starts there.
fn showing(editor: &Editor, at: Time) -> &Clip {
    let clips = &editor.project().tracks[0].clips;
    clips
        .iter()
        .find(|c| c.start <= at && at < c.end())
        .unwrap_or_else(|| clips.last().unwrap())
}

fn animated() -> Vec<(ParamKey, Vec<Keyframe>)> {
    vec![
        (
            ParamKey::PositionX,
            vec![
                key(0.0, 0.0, EASE),
                key(4.0, 100.0, Ease::Hold),
                key(6.0, -50.0, Ease::Linear),
                key(8.0, 20.0, OVERSHOOT),
                key(10.0, 0.0, Ease::Linear),
            ],
        ),
        (
            ParamKey::Rotation,
            vec![key(3.0, -90.0, EASE_IN), key(7.0, 270.0, Ease::Linear)],
        ),
        (ParamKey::ScaleX, vec![key(5.0, 2.0, Ease::Hold)]),
    ]
}

#[test]
fn split_keeps_every_sampled_animation_value() {
    for cut in [0.5, 2.0, 3.999, 4.0, 5.0, 6.5, 8.25, 9.0, 9.999] {
        let mut f = fixture();
        let blur = Effect::new(EffectKind::Blur);
        f.editor
            .dispatch(Command::SetEffects {
                clip_id: f.video,
                effects: vec![blur.clone()],
            })
            .unwrap();
        for (param, keys) in animated() {
            f.editor
                .dispatch(keyframes(f.video, None, param, keys))
                .unwrap();
        }
        f.editor
            .dispatch(keyframes(
                f.video,
                Some(blur.id),
                ParamKey::Radius,
                vec![key(1.0, 0.0, EASE), key(9.5, 0.25, Ease::Linear)],
            ))
            .unwrap();
        let whole = clip(&f.editor, f.video).clone();
        accepted(
            &mut f.editor,
            Command::SplitClip {
                clip_id: f.video,
                at: whole.start + seconds(cut),
            },
        );
        let clips = &f.editor.project().tracks[0].clips;
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].duration, seconds(cut));
        for half in clips {
            assert_eq!(half.animation.len(), whole.animation.len());
        }
        for step in 0..=2000 {
            let at = whole.start + Time(step * whole.duration.0 / 2000);
            let half = showing(&f.editor, at);
            for (before, after) in whole.animation.iter().zip(&half.animation) {
                let (expected, found) = (
                    before.sample(at - whole.start),
                    after.sample(at - half.start),
                );
                let tolerance = 2e-4 * expected.abs().max(1.0);
                assert!(
                    (expected - found).abs() <= tolerance,
                    "{:?} cut {cut} at {at:?}: {expected} became {found}",
                    before.key
                );
            }
        }
        // The left half ends on the value the right half starts with.
        let (left, right) = (&clips[0], &clips[1]);
        for (a, b) in left.animation.iter().zip(&right.animation) {
            assert!((a.sample(left.duration) - b.sample(Time::ZERO)).abs() <= 2e-4 * 100.0);
        }
    }
}

#[test]
fn split_remaps_look_ids_and_divides_transitions() {
    let mut f = fixture();
    let (blur, mask) = (Effect::new(EffectKind::Blur), Mask::new(MaskShape::Ellipse));
    let ramp = vec![key(0.0, 0.0, EASE), key(10.0, 0.2, Ease::Linear)];
    for command in [
        Command::SetEffects {
            clip_id: f.video,
            effects: vec![blur.clone()],
        },
        Command::SetMasks {
            clip_id: f.video,
            masks: vec![mask.clone()],
        },
        keyframes(f.video, Some(blur.id), ParamKey::Radius, ramp.clone()),
        keyframes(f.video, Some(mask.id), ParamKey::Feather, ramp.clone()),
        Command::SetTransition {
            clip_id: f.video,
            edge: TransitionEdge::In,
            transition: Some(fade(3.0)),
        },
        Command::SetTransition {
            clip_id: f.video,
            edge: TransitionEdge::Out,
            transition: Some(fade(1.0)),
        },
    ] {
        f.editor.dispatch(command).unwrap();
    }
    accepted(
        &mut f.editor,
        Command::SplitClip {
            clip_id: f.video,
            at: seconds(4.0),
        },
    );
    let clips = &f.editor.project().tracks[0].clips;
    let (left, right) = (&clips[0], &clips[1]);
    assert_eq!(left.id, f.video);
    assert_eq!((left.effects[0].id, left.masks[0].id), (blur.id, mask.id));
    assert_ne!(right.effects[0].id, blur.id);
    assert_ne!(right.masks[0].id, mask.id);
    assert_eq!(right.animation[0].target, Some(right.effects[0].id));
    assert_eq!(right.animation[1].target, Some(right.masks[0].id));
    assert_eq!(left.animation[0].target, Some(blur.id));
    // A transition longer than its half is shortened to fit.
    assert_eq!(left.transition_in, Some(fade(2.0)));
    assert_eq!(left.transition_out, None);
    assert_eq!(right.transition_in, None);
    assert_eq!(right.transition_out, Some(fade(1.0)));
}

#[test]
fn split_clamps_an_overshooting_boundary_key_into_range() {
    let mut f = fixture();
    let back = Ease::Bezier {
        x1: 0.3,
        y1: 1.6,
        x2: 0.6,
        y2: 1.3,
    };
    let track = AnimTrack {
        target: None,
        key: ParamKey::Opacity,
        keys: vec![key(0.0, 0.2, back), key(10.0, 1.0, Ease::Linear)],
    };
    assert!(track.sample(seconds(6.0)) > 1.0);
    f.editor
        .dispatch(keyframes(f.video, None, track.key, track.keys))
        .unwrap();
    accepted(
        &mut f.editor,
        Command::SplitClip {
            clip_id: f.video,
            at: seconds(2.0 + 6.0),
        },
    );
    let clips = &f.editor.project().tracks[0].clips;
    assert_eq!(clips[0].animation[0].keys.last().unwrap().value, 1.0);
    assert_eq!(clips[1].animation[0].keys[0].value, 1.0);
}

fn retimes() -> Vec<(&'static str, Vec<TimeKey>)> {
    vec![
        (
            "linear",
            vec![
                time_key(0.0, 0.0, Ease::Linear),
                time_key(4.0, 12.0, Ease::Linear),
                time_key(10.0, 15.0, Ease::Linear),
            ],
        ),
        (
            "freeze",
            vec![
                time_key(0.0, 0.0, Ease::Linear),
                time_key(3.0, 3.0, Ease::Linear),
                time_key(7.0, 3.0, Ease::Linear),
                time_key(10.0, 9.0, Ease::Linear),
            ],
        ),
        (
            "reverse",
            vec![
                time_key(0.0, 8.0, Ease::Linear),
                time_key(6.0, 2.0, EASE),
                time_key(10.0, 0.0, Ease::Linear),
            ],
        ),
        (
            "eased",
            vec![
                time_key(0.0, 0.0, EASE),
                time_key(5.0, 10.0, EASE_IN),
                time_key(10.0, 30.0, Ease::Linear),
            ],
        ),
        (
            "hold",
            vec![
                time_key(0.0, 1.0, Ease::Hold),
                time_key(5.0, 6.0, Ease::Linear),
                time_key(10.0, 11.0, Ease::Linear),
            ],
        ),
    ]
}

#[test]
fn split_keeps_every_retimed_source_time() {
    for (name, keys) in retimes() {
        for cut in [1.0, 2.5, 3.0, 4.0, 5.0, 6.523456, 7.0, 8.75] {
            if name == "hold" && cut > 4.0 && cut < 5.0 {
                // The jump would have to happen inside under a second: too fast.
                continue;
            }
            let mut f = fixture();
            f.editor
                .dispatch(Command::SetRetime {
                    clip_id: f.video,
                    retime: Some(Retime { keys: keys.clone() }),
                    duration: seconds(10.0),
                })
                .unwrap();
            let whole = clip(&f.editor, f.video).clone();
            accepted(
                &mut f.editor,
                Command::SplitClip {
                    clip_id: f.video,
                    at: whole.start + seconds(cut),
                },
            );
            let clips = &f.editor.project().tracks[0].clips;
            assert_eq!(clips.len(), 2);
            for half in clips {
                assert_eq!(half.playback_rate, 1.0);
                let (low, _) = half.retime.as_ref().unwrap().span();
                assert_eq!(low, Time::ZERO, "{name}: sources measure from source_in");
            }
            for step in 0..=2000 {
                let at = whole.start + Time(step * whole.duration.0 / 2000);
                let half = showing(&f.editor, at);
                let (expected, found) = (whole.source_time(at), half.source_time(at));
                assert!(
                    (expected.0 - found.0).abs() <= 4,
                    "{name} cut {cut} at {at:?}: {expected:?} became {found:?}"
                );
            }
            let (left, right) = (&clips[0], &clips[1]);
            let seam = (left.source_time(left.end()).0 - right.source_time(right.start).0).abs();
            assert!(seam <= 4 || name == "hold", "{name} cut {cut}: seam {seam}");
        }
    }
}

#[test]
fn split_that_leaves_a_part_outside_the_speed_bounds_is_refused() {
    let mut f = fixture();
    f.editor
        .dispatch(Command::SetRetime {
            clip_id: f.video,
            retime: Some(Retime {
                keys: retimes().remove(2).1,
            }),
            duration: seconds(10.0),
        })
        .unwrap();
    // The first 3% of an eased half-speed segment averages under 0.05.
    invalid(
        &mut f.editor,
        Command::SplitClip {
            clip_id: f.video,
            at: seconds(2.0 + 6.12),
        },
        "time map",
    );
}

#[test]
fn split_bakes_linked_clips_together() {
    let mut f = fixture();
    f.editor
        .dispatch(Command::LinkClips {
            clip_ids: vec![f.video, f.audio],
        })
        .unwrap();
    f.editor
        .dispatch(Command::SetRetime {
            clip_id: f.video,
            retime: Some(Retime {
                keys: retimes().remove(3).1,
            }),
            duration: seconds(10.0),
        })
        .unwrap();
    accepted(
        &mut f.editor,
        Command::SplitClip {
            clip_id: f.audio,
            at: seconds(6.0),
        },
    );
    let (video, audio) = (
        &f.editor.project().tracks[0].clips,
        &f.editor.project().tracks[1].clips,
    );
    assert_eq!((video.len(), audio.len()), (2, 2));
    for (v, a) in video.iter().zip(audio) {
        assert_eq!(v.retime, a.retime);
        assert_eq!((v.source_in, v.duration), (a.source_in, a.duration));
    }
    assert_eq!(video[1].linked_id, audio[1].linked_id);
    assert_ne!(video[0].linked_id, video[1].linked_id);
}

#[test]
fn trim_rebases_animation_and_keeps_in_range_values() {
    let mut f = fixture();
    for (param, keys) in animated() {
        f.editor
            .dispatch(keyframes(f.video, None, param, keys))
            .unwrap();
    }
    let whole = clip(&f.editor, f.video).clone();
    let same_values = |editor: &Editor, from: f64, to: f64| {
        let trimmed = clip(editor, whole.id);
        for step in 0..=1000 {
            let at = whole.start + seconds(from) + Time(step * seconds(to - from).0 / 1000);
            for (before, after) in whole.animation.iter().zip(&trimmed.animation) {
                let (expected, found) = (
                    before.sample(at - whole.start),
                    after.sample(at - trimmed.start),
                );
                assert!(
                    (expected - found).abs() <= 2e-4 * expected.abs().max(1.0),
                    "{:?} at {at:?}: {expected} became {found}",
                    before.key
                );
            }
        }
    };

    // Start trim by 2.5 s of source: the clip starts later and keys move earlier.
    accepted(
        &mut f.editor,
        Command::TrimClip {
            clip_id: f.video,
            source_in: seconds(7.5),
            duration: seconds(7.5),
            ripple: false,
        },
    );
    let trimmed = clip(&f.editor, f.video);
    assert_eq!(trimmed.start, seconds(4.5));
    let position = &trimmed.animation[0].keys;
    assert_eq!(position[0].at, Time::ZERO, "a boundary key is synthesized");
    assert_eq!(position[1].at, seconds(1.5));
    assert_eq!(position.len(), 5);
    assert_eq!(trimmed.animation[2].keys, [key(2.5, 2.0, Ease::Hold)]);
    same_values(&f.editor, 2.5, 10.0);

    // End trim: keys past the new end are replaced by one on the boundary.
    accepted(
        &mut f.editor,
        Command::TrimClip {
            clip_id: f.video,
            source_in: seconds(7.5),
            duration: seconds(4.0),
            ripple: false,
        },
    );
    let trimmed = clip(&f.editor, f.video);
    let position = &trimmed.animation[0].keys;
    assert_eq!(position.last().unwrap().at, seconds(4.0));
    assert_eq!(position.len(), 4);
    same_values(&f.editor, 2.5, 6.5);

    // Extending both edges moves the keys without inventing any.
    accepted(
        &mut f.editor,
        Command::TrimClip {
            clip_id: f.video,
            source_in: seconds(6.5),
            duration: seconds(8.0),
            ripple: false,
        },
    );
    let trimmed = clip(&f.editor, f.video);
    assert_eq!(trimmed.start, seconds(3.5));
    assert_eq!(trimmed.animation[0].keys.len(), 4);
    assert_eq!(trimmed.animation[0].keys[0].at, seconds(1.0));
    same_values(&f.editor, 2.5, 6.5);

    // A ripple start trim keeps the start and still rebases.
    let mut g = fixture();
    g.editor
        .dispatch(keyframes(
            g.video,
            None,
            ParamKey::Opacity,
            vec![key(4.0, 0.0, Ease::Linear), key(6.0, 1.0, Ease::Linear)],
        ))
        .unwrap();
    accepted(
        &mut g.editor,
        Command::TrimClip {
            clip_id: g.video,
            source_in: seconds(10.0),
            duration: seconds(5.0),
            ripple: true,
        },
    );
    let trimmed = clip(&g.editor, g.video);
    assert_eq!(trimmed.start, seconds(2.0));
    assert_eq!(
        trimmed.animation[0].keys,
        [key(0.0, 0.5, Ease::Linear), key(1.0, 1.0, Ease::Linear)]
    );
}

#[test]
fn trim_at_a_scaled_rate_rebases_by_timeline_time() {
    let mut f = fixture();
    f.editor
        .dispatch(Command::SetPlaybackRate {
            clip_id: f.video,
            rate: 2.0,
        })
        .unwrap();
    f.editor
        .dispatch(keyframes(
            f.video,
            None,
            ParamKey::Rotation,
            vec![key(1.0, 0.0, Ease::Linear), key(3.0, 90.0, Ease::Linear)],
        ))
        .unwrap();
    // Four seconds of source is two seconds of timeline at rate 2.
    accepted(
        &mut f.editor,
        Command::TrimClip {
            clip_id: f.video,
            source_in: seconds(9.0),
            duration: seconds(3.0),
            ripple: false,
        },
    );
    let trimmed = clip(&f.editor, f.video);
    assert_eq!(trimmed.start, seconds(4.0));
    assert_eq!(
        trimmed.animation[0].keys,
        [key(0.0, 45.0, Ease::Linear), key(1.0, 90.0, Ease::Linear)]
    );
}

#[test]
fn trim_crops_a_time_map_and_refuses_to_extend_it() {
    for (name, keys) in retimes() {
        let mut f = fixture();
        f.editor
            .dispatch(Command::SetRetime {
                clip_id: f.video,
                retime: Some(Retime { keys }),
                duration: seconds(10.0),
            })
            .unwrap();
        let whole = clip(&f.editor, f.video).clone();
        // On a retimed clip the start edge moves by the `source_in` change.
        accepted(
            &mut f.editor,
            Command::TrimClip {
                clip_id: f.video,
                source_in: whole.source_in + seconds(1.5),
                duration: seconds(6.0),
                ripple: false,
            },
        );
        let trimmed = clip(&f.editor, f.video);
        assert_eq!(
            (trimmed.start, trimmed.duration),
            (seconds(3.5), seconds(6.0))
        );
        assert_eq!(trimmed.playback_rate, 1.0);
        for step in 0..=1000 {
            let at = trimmed.start + Time(step * trimmed.duration.0 / 1000);
            let (expected, found) = (whole.source_time(at), trimmed.source_time(at));
            assert!(
                (expected.0 - found.0).abs() <= 4,
                "{name} at {at:?}: {expected:?} became {found:?}"
            );
        }

        let source_in = trimmed.source_in;
        invalid(
            &mut f.editor,
            Command::TrimClip {
                clip_id: f.video,
                source_in,
                duration: seconds(6.5),
                ripple: false,
            },
            "time map",
        );
        invalid(
            &mut f.editor,
            Command::TrimClip {
                clip_id: f.video,
                source_in: source_in - seconds(0.5),
                duration: seconds(6.0),
                ripple: false,
            },
            "time map",
        );
    }
}

#[test]
fn playback_rate_rescales_animation_with_the_duration() {
    let mut f = fixture();
    for (param, keys) in animated() {
        f.editor
            .dispatch(keyframes(f.video, None, param, keys))
            .unwrap();
    }
    f.editor
        .dispatch(Command::SetTransition {
            clip_id: f.video,
            edge: TransitionEdge::In,
            transition: Some(fade(4.0)),
        })
        .unwrap();
    let whole = clip(&f.editor, f.video).clone();
    accepted(
        &mut f.editor,
        Command::SetPlaybackRate {
            clip_id: f.video,
            rate: 4.0,
        },
    );
    let fast = clip(&f.editor, f.video);
    assert_eq!((fast.duration, fast.playback_rate), (seconds(2.5), 4.0));
    let times: Vec<_> = fast.animation[0].keys.iter().map(|k| k.at).collect();
    assert_eq!(times, [0.0, 1.0, 1.5, 2.0, 2.5].map(seconds));
    assert_eq!(fast.transition_in, Some(fade(2.5)));
    for step in 0..=1000 {
        let local = Time(step * whole.duration.0 / 1000);
        for (before, after) in whole.animation.iter().zip(&fast.animation) {
            let (expected, found) = (before.sample(local), after.sample(Time(local.0 / 4)));
            assert!((expected - found).abs() <= 2e-3 * expected.abs().max(1.0));
        }
    }

    accepted(
        &mut f.editor,
        Command::SetPlaybackRate {
            clip_id: f.video,
            rate: 0.5,
        },
    );
    let slow = clip(&f.editor, f.video);
    assert_eq!(slow.duration, seconds(20.0));
    let times: Vec<_> = slow.animation[0].keys.iter().map(|k| k.at).collect();
    assert_eq!(times, [0.0, 8.0, 12.0, 16.0, 20.0].map(seconds));
}

#[test]
fn playback_rate_scales_a_time_map_and_keeps_rate_one() {
    let mut f = fixture();
    f.editor
        .dispatch(Command::LinkClips {
            clip_ids: vec![f.video, f.audio],
        })
        .unwrap();
    f.editor
        .dispatch(Command::SetRetime {
            clip_id: f.video,
            retime: Some(Retime {
                keys: retimes().remove(1).1,
            }),
            duration: seconds(10.0),
        })
        .unwrap();
    let whole = clip(&f.editor, f.video).clone();
    accepted(
        &mut f.editor,
        Command::SetPlaybackRate {
            clip_id: f.video,
            rate: 2.0,
        },
    );
    for id in [f.video, f.audio] {
        let fast = clip(&f.editor, id);
        assert_eq!((fast.duration, fast.playback_rate), (seconds(5.0), 1.0));
        let map = fast.retime.as_ref().unwrap();
        let times: Vec<_> = map.keys.iter().map(|k| k.at).collect();
        assert_eq!(times, [0.0, 1.5, 3.5, 5.0].map(seconds));
        assert_eq!(fast.source_span(), whole.source_span());
        assert_eq!(fast.source_time(seconds(2.0 + 1.0)), seconds(5.0 + 2.0));
    }
    // The map's fastest segment would exceed sixteen times.
    invalid(
        &mut f.editor,
        Command::SetPlaybackRate {
            clip_id: f.video,
            rate: 16.0,
        },
        "time map",
    );
}

#[test]
fn remove_assets_refuses_media_used_by_an_effect_or_tracker() {
    let mut f = fixture();
    let plate = Asset::new(PathBuf::from("plate.png"), MediaKind::Image, seconds(5.0));
    let tracked = Asset::new(PathBuf::from("b.mp4"), MediaKind::Video, seconds(5.0));
    let spare = Asset::new(PathBuf::from("c.mp4"), MediaKind::Video, seconds(5.0));
    let (plate_id, tracked_id, spare_id) = (plate.id, tracked.id, spare.id);
    f.editor
        .dispatch(Command::AddAssets {
            assets: vec![plate, tracked, spare],
        })
        .unwrap();
    let mut matte = Effect::new(EffectKind::DifferenceMatte);
    matte.asset_id = Some(plate_id);
    accepted(
        &mut f.editor,
        Command::SetEffects {
            clip_id: f.video,
            effects: vec![matte],
        },
    );
    let path = tracker(tracked_id);
    f.editor
        .dispatch(Command::SetTracker {
            tracker: path.clone(),
        })
        .unwrap();

    let remove = |asset_ids| Command::RemoveAssets { asset_ids };
    invalid(
        &mut f.editor,
        remove(vec![spare_id, plate_id]),
        "media is still used on the timeline",
    );
    invalid(
        &mut f.editor,
        remove(vec![tracked_id]),
        "media is still used by a tracker",
    );
    accepted(&mut f.editor, remove(vec![spare_id]));

    f.editor
        .dispatch(Command::Batch {
            label: "Release".into(),
            commands: vec![
                Command::SetEffects {
                    clip_id: f.video,
                    effects: Vec::new(),
                },
                Command::RemoveTracker {
                    tracker_id: path.id,
                },
            ],
        })
        .unwrap();
    accepted(&mut f.editor, remove(vec![plate_id, tracked_id]));
}

#[test]
fn paste_remaps_effect_and_mask_ids_and_animation_targets() {
    let mut f = fixture();
    let (blur, mask) = (Effect::new(EffectKind::Blur), Mask::new(MaskShape::Ellipse));
    let face = tracker(f.asset);
    let ramp = vec![key(0.0, 0.0, EASE), key(10.0, 0.2, Ease::Linear)];
    for command in [
        Command::SetEffects {
            clip_id: f.video,
            effects: vec![blur.clone()],
        },
        Command::SetMasks {
            clip_id: f.video,
            masks: vec![mask.clone()],
        },
        keyframes(f.video, Some(blur.id), ParamKey::Radius, ramp.clone()),
        keyframes(f.video, Some(mask.id), ParamKey::Feather, ramp.clone()),
        keyframes(f.video, None, ParamKey::Opacity, ramp.clone()),
        Command::SetTracker {
            tracker: face.clone(),
        },
        Command::SetFollow {
            clip_id: f.video,
            follow: Some(follow(face.id)),
        },
    ] {
        f.editor.dispatch(command).unwrap();
    }
    let clipboard = f.editor.copy(&[f.video]).unwrap();
    let before = f.editor.snapshot();
    f.editor.paste(&clipboard, seconds(20.0), None).unwrap();
    f.editor.paste(&clipboard, seconds(40.0), None).unwrap();

    let clips = &f.editor.project().tracks[0].clips;
    assert_eq!(clips.len(), 3);
    assert_eq!(clips[0], *before.clip(f.video).unwrap().1);
    let mut seen = vec![blur.id, mask.id];
    for pasted in &clips[1..] {
        let (effect, shape) = (pasted.effects[0].id, pasted.masks[0].id);
        assert!(!seen.contains(&effect) && !seen.contains(&shape));
        seen.extend([effect, shape]);
        let targets: Vec<_> = pasted.animation.iter().map(|t| t.target).collect();
        assert_eq!(targets, [Some(effect), Some(shape), None]);
        assert_eq!(pasted.animation[0].keys, ramp);
        assert_eq!(pasted.follow, Some(follow(face.id)));
        assert_eq!(pasted.effects[0].kind, EffectKind::Blur);
    }
    assert!(f.editor.undo());
    assert!(f.editor.undo());
    assert!(Arc::ptr_eq(&before, &f.editor.snapshot()));

    // A follow whose tracker has gone is not pasted.
    f.editor
        .dispatch(Command::RemoveTracker {
            tracker_id: face.id,
        })
        .unwrap();
    f.editor.paste(&clipboard, seconds(20.0), None).unwrap();
    assert_eq!(f.editor.project().tracks[0].clips[1].follow, None);
}

#[test]
fn new_commands_round_trip_through_serde() {
    let f = fixture();
    let commands = vec![
        Command::SetEffects {
            clip_id: f.video,
            effects: look::preset_effects(LookPreset::Cinematic, 0.7),
        },
        Command::SetMasks {
            clip_id: f.video,
            masks: vec![Mask::new(MaskShape::Rectangle)],
        },
        Command::SetBlendMode {
            clip_id: f.video,
            mode: BlendMode::SoftLight,
        },
        keyframes(f.video, None, ParamKey::ScaleX, vec![key(1.0, 2.0, EASE)]),
        Command::SetRetime {
            clip_id: f.video,
            retime: Some(Retime::constant(seconds(5.0), 2.0)),
            duration: seconds(5.0),
        },
        Command::SetTransition {
            clip_id: f.video,
            edge: TransitionEdge::In,
            transition: Some(fade(1.0)),
        },
        Command::ApplyPreset {
            clip_ids: vec![f.video],
            preset: LookPreset::TealOrange,
            strength: 0.5,
        },
        Command::SetTracker {
            tracker: tracker(f.asset),
        },
        Command::RemoveTracker {
            tracker_id: Uuid::new_v4(),
        },
        Command::SetFollow {
            clip_id: f.video,
            follow: Some(follow(Uuid::new_v4())),
        },
    ];
    let tags: Vec<_> = commands
        .iter()
        .map(|command| {
            let json = serde_json::to_value(command).unwrap();
            let back: Command = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(&back).unwrap(), json);
            json["type"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        tags,
        [
            "set_effects",
            "set_masks",
            "set_blend_mode",
            "set_keyframes",
            "set_retime",
            "set_transition",
            "apply_preset",
            "set_tracker",
            "remove_tracker",
            "set_follow"
        ]
    );
}

fn bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> Ease {
    Ease::Bezier { x1, y1, x2, y2 }
}

#[test]
fn split_keeps_steep_named_easings_of_a_time_map() {
    let easings = [
        ("expo", bezier(0.87, 0.0, 0.13, 1.0)),
        ("circ", bezier(0.85, 0.0, 0.15, 1.0)),
        ("quint", bezier(0.83, 0.0, 0.17, 1.0)),
    ];
    for (name, ease) in easings {
        for reversed in [false, true] {
            let sources = if reversed { (10.0, 0.0) } else { (0.0, 10.0) };
            let mut kept = 0;
            for cut in 1..40 {
                let cut = f64::from(cut) * 0.25;
                let mut f = fixture();
                f.editor
                    .dispatch(Command::SetRetime {
                        clip_id: f.video,
                        retime: Some(Retime {
                            keys: vec![
                                time_key(0.0, sources.0, ease),
                                time_key(10.0, sources.1, Ease::Linear),
                            ],
                        }),
                        duration: seconds(10.0),
                    })
                    .unwrap();
                let whole = clip(&f.editor, f.video).clone();
                let split = Command::SplitClip {
                    clip_id: f.video,
                    at: whole.start + seconds(cut),
                };
                // A cut that leaves a part under the speed bound is refused whole.
                if f.editor.dispatch(split).is_err() {
                    assert_eq!(clip(&f.editor, f.video), &whole);
                    continue;
                }
                kept += 1;
                for step in 0..=2000 {
                    let at = whole.start + Time(step * whole.duration.0 / 2000);
                    let half = showing(&f.editor, at);
                    let (expected, found) = (whole.source_time(at), half.source_time(at));
                    assert!(
                        (expected.0 - found.0).abs() <= 8,
                        "{name} cut {cut} at {at:?}: {expected:?} became {found:?}"
                    );
                }
            }
            assert!(kept >= 20, "{name}: only {kept} cuts were accepted");
        }
    }
}

#[test]
fn split_keeps_back_and_steep_easings_of_animation() {
    let easings = [
        ("in back", bezier(0.36, 0.0, 0.66, -0.56)),
        ("out back", bezier(0.34, 1.56, 0.64, 1.0)),
        ("in out back", bezier(0.68, -0.6, 0.32, 1.6)),
        ("expo", bezier(0.87, 0.0, 0.13, 1.0)),
        ("circ", bezier(0.85, 0.0, 0.15, 1.0)),
    ];
    for (name, ease) in easings {
        for cut in 1..40 {
            let cut = f64::from(cut) * 0.25;
            let mut f = fixture();
            f.editor
                .dispatch(keyframes(
                    f.video,
                    None,
                    ParamKey::PositionX,
                    vec![key(0.0, 0.0, ease), key(10.0, 1.0, Ease::Linear)],
                ))
                .unwrap();
            let whole = clip(&f.editor, f.video).clone();
            accepted(
                &mut f.editor,
                Command::SplitClip {
                    clip_id: f.video,
                    at: whole.start + seconds(cut),
                },
            );
            for step in 0..=2000 {
                let at = whole.start + Time(step * whole.duration.0 / 2000);
                let half = showing(&f.editor, at);
                let (expected, found) = (
                    whole.animation[0].sample(at - whole.start),
                    half.animation[0].sample(at - half.start),
                );
                assert!(
                    (expected - found).abs() <= 2e-4,
                    "{name} cut {cut} at {at:?}: {expected} became {found}"
                );
            }
        }
    }
}

#[test]
fn a_lower_frame_rate_lengthens_transitions_under_one_frame() {
    let mut f = fixture();
    let mut settings = f.editor.project().settings.clone();
    settings.frame_rate = editor_core::FrameRate {
        numerator: 60,
        denominator: 1,
    };
    f.editor
        .dispatch(Command::SetSettings {
            settings: settings.clone(),
        })
        .unwrap();
    let mut short = fade(0.02);
    short.duration = Time(20_000);
    for edge in [TransitionEdge::In, TransitionEdge::Out] {
        f.editor
            .dispatch(Command::SetTransition {
                clip_id: f.video,
                edge,
                transition: Some(short.clone()),
            })
            .unwrap();
    }
    f.editor
        .dispatch(Command::SetTransition {
            clip_id: f.text,
            edge: TransitionEdge::In,
            transition: Some(fade(1.0)),
        })
        .unwrap();
    settings.frame_rate.numerator = 24;
    accepted(&mut f.editor, Command::SetSettings { settings });
    let video = clip(&f.editor, f.video);
    let frame = Time(41_666);
    assert_eq!(video.transition_in.as_ref().unwrap().duration, frame);
    assert_eq!(video.transition_out.as_ref().unwrap().duration, frame);
    let text = clip(&f.editor, f.text);
    assert_eq!(text.transition_in, Some(fade(1.0)));
}

#[test]
fn start_trim_of_a_title_rebases_its_animation() {
    for delta in [1.0, -1.0] {
        let mut f = fixture();
        let fades = vec![
            key(0.0, 0.0, Ease::Linear),
            key(1.0, 1.0, Ease::Linear),
            key(9.0, 1.0, Ease::Linear),
            key(10.0, 0.0, Ease::Linear),
        ];
        f.editor
            .dispatch(keyframes(f.text, None, ParamKey::Opacity, fades))
            .unwrap();
        let whole = clip(&f.editor, f.text).clone();
        accepted(
            &mut f.editor,
            Command::TrimClip {
                clip_id: f.text,
                source_in: whole.source_in + seconds(delta),
                duration: whole.duration - seconds(delta),
                ripple: false,
            },
        );
        let trimmed = clip(&f.editor, f.text);
        assert_eq!(trimmed.start, whole.start + seconds(delta));
        assert_eq!(trimmed.end(), whole.end());
        assert!(trimmed.source_in >= Time::ZERO);
        // Every moment both clips cover shows the value it showed before.
        for step in 0..=90 {
            let at = whole.start + seconds(1.0) + Time(step * 100_000);
            let (expected, found) = (
                whole.animation[0].sample(at - whole.start),
                trimmed.animation[0].sample(at - trimmed.start),
            );
            assert!(
                (expected - found).abs() <= 1e-5,
                "delta {delta} at {at:?}: {expected} became {found}"
            );
        }
    }
}

#[test]
fn set_retime_leaves_a_linked_title_without_a_map() {
    let mut f = fixture();
    f.editor
        .dispatch(Command::LinkClips {
            clip_ids: vec![f.video, f.text],
        })
        .unwrap();
    f.editor
        .dispatch(keyframes(
            f.text,
            None,
            ParamKey::Opacity,
            vec![key(0.0, 0.0, Ease::Linear), key(10.0, 1.0, Ease::Linear)],
        ))
        .unwrap();
    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.video,
            retime: Some(Retime::constant(seconds(5.0), 2.0)),
            duration: seconds(5.0),
        },
    );
    assert!(clip(&f.editor, f.video).retime.is_some());
    let text = clip(&f.editor, f.text);
    assert_eq!(text.retime, None);
    assert_eq!(text.duration, seconds(5.0));
    assert_eq!(text.animation[0].keys[1].at, seconds(5.0));
    // Named directly, a title still refuses a map.
    invalid(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.text,
            retime: Some(Retime::constant(seconds(5.0), 1.0)),
            duration: seconds(5.0),
        },
        "text clip look",
    );
}

#[test]
fn set_retime_keeps_the_proportion_of_a_linked_clip_of_another_length() {
    let mut f = fixture();
    f.editor
        .dispatch(Command::TrimClip {
            clip_id: f.audio,
            source_in: seconds(5.0),
            duration: seconds(30.0),
            ripple: false,
        })
        .unwrap();
    f.editor
        .dispatch(Command::LinkClips {
            clip_ids: vec![f.video, f.audio],
        })
        .unwrap();
    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.video,
            retime: Some(Retime {
                keys: vec![
                    time_key(0.0, 0.0, Ease::Linear),
                    time_key(2.0, 4.0, Ease::Linear),
                    time_key(5.0, 10.0, Ease::Linear),
                ],
            }),
            duration: seconds(5.0),
        },
    );
    let video = clip(&f.editor, f.video);
    assert_eq!(video.duration, seconds(5.0));
    assert_eq!(video.source_span(), (seconds(5.0), seconds(15.0)));
    // Three times as long before, three times as long after, at the same speed.
    let audio = clip(&f.editor, f.audio);
    assert_eq!(audio.duration, seconds(15.0));
    assert_eq!(audio.source_span(), (seconds(5.0), seconds(35.0)));
    let keys = &audio.retime.as_ref().unwrap().keys;
    assert_eq!(
        keys.iter().map(|k| (k.at, k.source)).collect::<Vec<_>>(),
        [(0.0, 0.0), (6.0, 12.0), (15.0, 30.0)].map(|(at, source)| (seconds(at), seconds(source)))
    );
}

#[test]
fn audio_clips_and_audio_assets_refuse_what_cannot_be_drawn() {
    let mut f = fixture();
    let audio = f.audio;
    let look = [
        Command::SetEffects {
            clip_id: audio,
            effects: vec![Effect::new(EffectKind::Blur)],
        },
        Command::SetMasks {
            clip_id: audio,
            masks: vec![Mask::new(MaskShape::Ellipse)],
        },
        Command::SetBlendMode {
            clip_id: audio,
            mode: BlendMode::Screen,
        },
        Command::SetTransition {
            clip_id: audio,
            edge: TransitionEdge::In,
            transition: Some(fade(1.0)),
        },
        keyframes(
            audio,
            None,
            ParamKey::Opacity,
            vec![key(0.0, 0.0, Ease::Linear)],
        ),
        Command::ApplyPreset {
            clip_ids: vec![audio],
            preset: LookPreset::Noir,
            strength: 1.0,
        },
    ];
    for command in look {
        invalid(&mut f.editor, command, "audio clip look");
    }
    accepted(
        &mut f.editor,
        keyframes(
            audio,
            None,
            ParamKey::Gain,
            vec![key(0.0, 0.0, Ease::Linear), key(1.0, 1.0, Ease::Linear)],
        ),
    );

    let sound = Asset::new(PathBuf::from("s.wav"), MediaKind::Audio, seconds(60.0));
    let sound_id = sound.id;
    f.editor
        .dispatch(Command::AddAssets {
            assets: vec![sound],
        })
        .unwrap();
    let path = tracker(sound_id);
    invalid(
        &mut f.editor,
        Command::SetTracker { tracker: path },
        "tracker",
    );
    let seen = tracker(f.asset);
    f.editor
        .dispatch(Command::SetTracker {
            tracker: seen.clone(),
        })
        .unwrap();
    invalid(
        &mut f.editor,
        Command::SetFollow {
            clip_id: audio,
            follow: Some(follow(seen.id)),
        },
        "audio clip look",
    );
    let mut matte = Effect::new(EffectKind::DifferenceMatte);
    matte.asset_id = Some(sound_id);
    invalid(
        &mut f.editor,
        Command::SetEffects {
            clip_id: f.video,
            effects: vec![matte],
        },
        "effect",
    );
}

/// Key times of the video clip's first animation track, in seconds.
fn key_times(editor: &Editor, id: ClipId) -> Vec<f64> {
    clip(editor, id).animation[0]
        .keys
        .iter()
        .map(|k| k.at.as_seconds())
        .collect()
}

fn opacity_keys(f: &mut Fixture, times: &[f64]) {
    let keys = times
        .iter()
        .enumerate()
        .map(|(index, at)| key(*at, index as f32 / 10.0, Ease::Linear))
        .collect();
    f.editor
        .dispatch(keyframes(f.video, None, ParamKey::Opacity, keys))
        .unwrap();
}

/// Every key still sits on the source moment it had in `before`.
fn assert_on_source(before: &Clip, after: &Clip) {
    for (old, new) in before.animation.iter().zip(&after.animation) {
        for was in &old.keys {
            let source = before.source_time(before.start + was.at);
            assert!(
                new.keys.iter().any(|k| {
                    k.value == was.value
                        && (after.source_time(after.start + k.at).0 - source.0).abs() <= 40
                }),
                "the key at {:?} left its source moment",
                was.at
            );
        }
    }
}

#[test]
fn retime_at_a_constant_rate_keeps_keys_on_their_source() {
    let mut f = fixture();
    opacity_keys(&mut f, &[0.0, 2.0, 4.0, 10.0]);
    let before = clip(&f.editor, f.video).clone();
    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.video,
            retime: Some(Retime::constant(seconds(5.0), 2.0)),
            duration: seconds(5.0),
        },
    );
    assert_eq!(key_times(&f.editor, f.video), [0.0, 1.0, 2.0, 5.0]);
    assert_on_source(&before, clip(&f.editor, f.video));
}

#[test]
fn a_freeze_moves_later_keys_by_its_length_and_leaves_earlier_ones() {
    let mut f = fixture();
    opacity_keys(&mut f, &[1.0, 3.0, 4.0, 6.0]);
    f.editor
        .dispatch(Command::SetTransition {
            clip_id: f.video,
            edge: TransitionEdge::In,
            transition: Some(fade(1.0)),
        })
        .unwrap();
    let before = clip(&f.editor, f.video).clone();
    // Two seconds held on the frame three seconds in.
    let freeze = Retime {
        keys: vec![
            time_key(0.0, 0.0, Ease::Linear),
            time_key(3.0, 3.0, Ease::Linear),
            time_key(5.0, 3.0, Ease::Linear),
            time_key(12.0, 10.0, Ease::Linear),
        ],
    };
    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.video,
            retime: Some(freeze.clone()),
            duration: seconds(12.0),
        },
    );
    // The key on the held frame stays where the hold starts.
    assert_eq!(key_times(&f.editor, f.video), [1.0, 3.0, 6.0, 8.0]);
    assert_on_source(&before, clip(&f.editor, f.video));
    assert_eq!(clip(&f.editor, f.video).transition_in, Some(fade(1.0)));

    // A key set inside the hold keeps its place in it while the map around
    // it changes, and the keys after it follow their pictures.
    opacity_keys(&mut f, &[1.0, 3.5, 4.5, 6.0, 8.0]);
    let longer = Retime {
        keys: vec![
            time_key(0.0, 0.0, Ease::Linear),
            time_key(1.5, 3.0, Ease::Linear),
            time_key(3.5, 3.0, Ease::Linear),
            time_key(10.5, 10.0, Ease::Linear),
        ],
    };
    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.video,
            retime: Some(longer),
            duration: seconds(10.5),
        },
    );
    assert_eq!(key_times(&f.editor, f.video), [0.5, 2.0, 3.0, 4.5, 6.5]);

    // Without the hold its keys meet on one frame and the last one stays.
    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.video,
            retime: None,
            duration: seconds(10.0),
        },
    );
    let track = &clip(&f.editor, f.video).animation[0];
    assert_eq!(key_times(&f.editor, f.video), [1.0, 3.0, 4.0, 6.0]);
    assert_eq!(track.keys[1].value, 0.2, "the later key of the hold");
}

#[test]
fn a_reverse_mirrors_keys_and_their_easing() {
    let mut f = fixture();
    f.editor
        .dispatch(keyframes(
            f.video,
            None,
            ParamKey::PositionX,
            vec![
                key(1.0, 0.0, EASE_IN),
                key(4.0, 100.0, Ease::Linear),
                key(10.0, 50.0, Ease::Linear),
            ],
        ))
        .unwrap();
    let before = clip(&f.editor, f.video).clone();
    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.video,
            retime: Some(Retime {
                keys: vec![
                    time_key(0.0, 10.0, Ease::Linear),
                    time_key(10.0, 0.0, Ease::Linear),
                ],
            }),
            duration: seconds(10.0),
        },
    );
    let after = clip(&f.editor, f.video);
    assert_eq!(key_times(&f.editor, f.video), [0.0, 6.0, 9.0]);
    assert_on_source(&before, after);
    // Every picture keeps the value it had: the eased stretch plays backwards.
    for step in 0..=100 {
        let local = Time(seconds(1.0).0 + step * seconds(9.0).0 / 100);
        let mirrored = Time(seconds(10.0).0 - local.0);
        let (was, now) = (
            before.animation[0].sample(local),
            after.animation[0].sample(mirrored),
        );
        assert!((was - now).abs() < 0.05, "{was} became {now} at {local:?}");
    }
}

#[test]
fn a_ramp_keeps_keys_on_their_source_and_drops_none() {
    let mut f = fixture();
    opacity_keys(&mut f, &[0.0, 2.5, 5.0, 7.5, 10.0]);
    let before = clip(&f.editor, f.video).clone();
    for (name, keys) in [
        (
            "eased",
            vec![
                time_key(0.0, 0.0, EASE),
                time_key(4.0, 5.0, EASE_IN),
                time_key(8.0, 10.0, Ease::Linear),
            ],
        ),
        (
            "there and back",
            vec![
                time_key(0.0, 0.0, Ease::Linear),
                time_key(5.0, 10.0, Ease::Linear),
                time_key(10.0, 0.0, Ease::Linear),
            ],
        ),
    ] {
        let duration = keys.last().unwrap().at;
        accepted(
            &mut f.editor,
            Command::SetRetime {
                clip_id: f.video,
                retime: Some(Retime { keys }),
                duration,
            },
        );
        let after = clip(&f.editor, f.video);
        assert_eq!(after.animation[0].keys.len(), 5, "{name}");
        assert_on_source(&before, after);
        assert!(f.editor.undo());
    }
    // Shown twice, a moment keeps the showing nearest the key's old time.
    assert!(f.editor.redo());
    assert_eq!(
        key_times(&f.editor, f.video),
        [0.0, 1.25, 2.5, 5.0, 6.25],
        "the key at 7.5 s is nearer the way back, past the last key's turn"
    );
    assert!(f.editor.undo());

    // A map that shows less of the source gathers what it left out on its end.
    accepted(
        &mut f.editor,
        Command::SetRetime {
            clip_id: f.video,
            retime: Some(Retime::constant(seconds(6.0), 1.0)),
            duration: seconds(6.0),
        },
    );
    let track = &clip(&f.editor, f.video).animation[0];
    assert_eq!(key_times(&f.editor, f.video), [0.0, 2.5, 5.0, 6.0]);
    assert_eq!(track.keys[3].value, 0.3, "the moment nearest the end");
}
