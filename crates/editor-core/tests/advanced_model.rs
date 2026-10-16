use editor_core::{
    AnimTarget, AnimTrack, Asset, BlendMode, Clip, Ease, EditError, Effect, EffectKind, Follow,
    Keyframe, Mask, MaskShape, MediaKind, ParamKey, ParamValue, Project, Retime, Time, TimeKey,
    TrackKey, Tracker, Transition, TransitionKind, look,
};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

const V1_CLIP: &str = r#"{
    "id": "00000000-0000-4000-8000-000000000004",
    "name": "a.mp4",
    "content": { "type": "media", "asset_id": "00000000-0000-4000-8000-000000000002" },
    "start": 0,
    "source_in": 1000000,
    "duration": 10000000,
    "playback_rate": 2.0,
    "transform": {
        "position": [0.0, 0.0],
        "scale": [1.0, 1.0],
        "rotation": 0.0,
        "opacity": 1.0,
        "flip_x": false,
        "flip_y": false,
        "crop": [0.0, 0.0, 0.0, 0.0]
    },
    "linked_id": null,
    "group_id": null,
    "gain": 1.0
}"#;

fn v1_project() -> String {
    format!(
        r#"{{
    "id": "00000000-0000-4000-8000-000000000001",
    "name": "Version one",
    "settings": {{
        "width": 1920,
        "height": 1080,
        "frame_rate": {{ "numerator": 30, "denominator": 1 }},
        "background": [14, 15, 18, 255],
        "audio_sample_rate": 48000
    }},
    "assets": {{
        "00000000-0000-4000-8000-000000000002": {{
            "id": "00000000-0000-4000-8000-000000000002",
            "name": "a.mp4",
            "path": "a.mp4",
            "kind": "video",
            "duration": 60000000,
            "width": 1920,
            "height": 1080,
            "frame_rate": null,
            "has_audio": true,
            "folder_id": null,
            "file_size": 0
        }}
    }},
    "folders": {{}},
    "tracks": [
        {{
            "id": "00000000-0000-4000-8000-000000000003",
            "name": "Video 1",
            "kind": "video",
            "locked": false,
            "hidden": false,
            "muted": false,
            "clips": [{V1_CLIP}]
        }}
    ],
    "markers": []
}}"#
    )
}

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

/// A 60 s video asset, a 10 s clip of it starting at 2 s from source 5 s, and
/// a 10 s title.
fn fixture() -> (Project, Uuid) {
    let mut project = Project::new("Advanced");
    let asset = Asset::new(PathBuf::from("a.mp4"), MediaKind::Video, seconds(60.0));
    let mut clip = Clip::from_asset(&asset, seconds(2.0));
    clip.source_in = seconds(5.0);
    clip.duration = seconds(10.0);
    let asset_id = asset.id;
    Arc::make_mut(&mut project.assets).insert(asset_id, asset);
    Arc::make_mut(&mut project.tracks[0]).clips.push(clip);
    Arc::make_mut(&mut project.tracks[2]).clips.push(Clip::text(
        "Title",
        Time::ZERO,
        seconds(10.0),
    ));
    project.validate().unwrap();
    (project, asset_id)
}

/// Adds an asset of `kind` as an import would describe it: a lookup table
/// has one microsecond and the points along an edge for a width.
fn add_asset(project: &mut Project, path: &str, kind: MediaKind) -> Uuid {
    let mut asset = Asset::new(PathBuf::from(path), kind, seconds(5.0));
    if kind == MediaKind::Lut {
        asset.duration = Time(1);
        asset.width = 33;
    }
    let id = asset.id;
    Arc::make_mut(&mut project.assets).insert(id, asset);
    id
}

fn media(edit: impl FnOnce(&mut Clip)) -> Result<(), EditError> {
    let (mut project, _) = fixture();
    edit(&mut Arc::make_mut(&mut project.tracks[0]).clips[0]);
    project.validate()
}

fn text(edit: impl FnOnce(&mut Clip)) -> Result<(), EditError> {
    let (mut project, _) = fixture();
    edit(&mut Arc::make_mut(&mut project.tracks[2]).clips[0]);
    project.validate()
}

#[track_caller]
fn invalid(result: Result<(), EditError>, what: &str) {
    match result {
        Err(EditError::Invalid(reason)) => {
            assert_eq!(reason.split(" (clip ").next().unwrap(), what)
        }
        other => panic!("expected Invalid({what}), got {other:?}"),
    }
}

#[track_caller]
fn missing(result: Result<(), EditError>, id: Uuid) {
    match result {
        Err(EditError::Missing(found)) => assert_eq!(found, id),
        other => panic!("expected Missing({id}), got {other:?}"),
    }
}

fn key(at: Time, value: f32) -> Keyframe {
    Keyframe {
        at,
        value,
        ease: Ease::Linear,
    }
}

fn track(target: Option<Uuid>, param: ParamKey, keys: Vec<Keyframe>) -> AnimTrack {
    AnimTrack {
        target,
        key: param,
        keys,
    }
}

fn opacity(keys: Vec<Keyframe>) -> AnimTrack {
    track(None, ParamKey::Opacity, keys)
}

fn value(param: ParamKey, value: f32) -> ParamValue {
    ParamValue { key: param, value }
}

fn time_key(at: f64, source: f64) -> TimeKey {
    TimeKey {
        at: seconds(at),
        source: seconds(source),
        ease: Ease::Linear,
    }
}

fn retime(keys: &[(f64, f64)]) -> Retime {
    Retime {
        keys: keys
            .iter()
            .map(|&(at, source)| time_key(at, source))
            .collect(),
    }
}

fn transition(duration: Time) -> Transition {
    Transition {
        kind: TransitionKind::Dissolve,
        duration,
        ease: Ease::Linear,
        params: Vec::new(),
    }
}

fn tracker(asset_id: Uuid) -> Tracker {
    Tracker {
        id: Uuid::new_v4(),
        name: "Subject".into(),
        asset_id,
        keys: vec![
            TrackKey {
                source: Time::ZERO,
                center: [0.5, 0.5],
                scale: 1.0,
                rotation: 0.0,
            },
            TrackKey {
                source: seconds(1.0),
                center: [0.6, 0.5],
                scale: 1.1,
                rotation: 2.0,
            },
        ],
    }
}

fn with_tracker(edit: impl FnOnce(&mut Tracker)) -> Result<(), EditError> {
    let (mut project, asset_id) = fixture();
    let mut tracker = tracker(asset_id);
    edit(&mut tracker);
    Arc::make_mut(&mut project.trackers).insert(tracker.id, Arc::new(tracker));
    project.validate()
}

const EASE_IN: Ease = Ease::Bezier {
    x1: 0.42,
    y1: 0.0,
    x2: 1.0,
    y2: 1.0,
};
const EASE_IN_OUT: Ease = Ease::Bezier {
    x1: 0.42,
    y1: 0.0,
    x2: 0.58,
    y2: 1.0,
};
const NAN_EASE: Ease = Ease::Bezier {
    x1: f32::NAN,
    y1: 0.0,
    x2: 1.0,
    y2: 1.0,
};

#[test]
fn version_one_clip_decodes_with_defaults_and_reserializes_unchanged() {
    let clip: Clip = serde_json::from_str(V1_CLIP).unwrap();
    assert_eq!(clip.blend, BlendMode::Normal);
    assert!(clip.effects.is_empty() && clip.masks.is_empty() && clip.animation.is_empty());
    assert!(clip.retime.is_none() && clip.follow.is_none());
    assert!(clip.transition_in.is_none() && clip.transition_out.is_none());
    assert_eq!(clip.source_time(seconds(3.0)), seconds(7.0));
    let fixture: serde_json::Value = serde_json::from_str(V1_CLIP).unwrap();
    assert_eq!(serde_json::to_value(&clip).unwrap(), fixture);
}

#[test]
fn version_one_project_decodes_validates_and_reserializes_unchanged() {
    let json = v1_project();
    let project: Project = serde_json::from_str(&json).unwrap();
    assert!(project.trackers.is_empty());
    project.validate().unwrap();
    let fixture: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_value(&project).unwrap(), fixture);
}

#[test]
fn default_clip_and_project_serialize_no_new_keys() {
    let (project, _) = fixture();
    let encoded = serde_json::to_value(&project).unwrap();
    let root: Vec<_> = encoded.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        root,
        [
            "assets", "folders", "id", "markers", "name", "settings", "tracks"
        ]
    );
    for track in [0, 2] {
        let clip = &encoded["tracks"][track]["clips"][0];
        let keys: Vec<_> = clip.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "content",
                "duration",
                "gain",
                "group_id",
                "id",
                "linked_id",
                "name",
                "playback_rate",
                "source_in",
                "start",
                "transform"
            ]
        );
    }
}

#[test]
fn populated_clip_and_trackers_round_trip() {
    let (mut project, asset_id) = fixture();
    let tracker = tracker(asset_id);
    let tracker_id = tracker.id;
    Arc::make_mut(&mut project.trackers).insert(tracker_id, Arc::new(tracker));
    let plate_id = add_asset(&mut project, "plate.png", MediaKind::Image);
    let clip = &mut Arc::make_mut(&mut project.tracks[0]).clips[0];
    let mut glow = Effect::new(EffectKind::Glow);
    glow.params.push(value(ParamKey::Amount, 1.5));
    let mut matte = Effect::new(EffectKind::DifferenceMatte);
    matte.asset_id = Some(plate_id);
    let mask = Mask::new(MaskShape::Ellipse);
    clip.animation = vec![
        opacity(vec![key(Time::ZERO, 0.0), key(seconds(10.0), 1.0)]),
        track(
            Some(glow.id),
            ParamKey::Radius,
            vec![Keyframe {
                at: seconds(1.0),
                value: 0.1,
                ease: EASE_IN,
            }],
        ),
        track(
            Some(mask.id),
            ParamKey::Feather,
            vec![key(seconds(2.0), 0.5)],
        ),
    ];
    clip.effects = vec![glow, Effect::new(EffectKind::CurveRed), matte];
    clip.masks = vec![mask];
    clip.blend = BlendMode::SoftLight;
    clip.retime = Some(retime(&[(0.0, 0.0), (4.0, 4.0), (6.0, 4.0), (10.0, 0.0)]));
    clip.transition_in = Some(transition(seconds(1.0)));
    clip.transition_out = Some(Transition {
        kind: TransitionKind::WipeLeft,
        duration: seconds(10.0),
        ease: EASE_IN_OUT,
        params: vec![value(ParamKey::Softness, 0.2)],
    });
    clip.follow = Some(Follow {
        tracker_id,
        position: true,
        scale: false,
        rotation: true,
        offset: [10.0, -4.0],
    });
    project.validate().unwrap();
    let encoded = serde_json::to_string(&project).unwrap();
    assert!(encoded.contains(r#""blend":"soft_light""#));
    assert!(encoded.contains(r#""kind":"difference_matte""#));
    assert!(encoded.contains(r#""type":"bezier""#));
    let decoded: Project = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, project);
}

#[test]
fn text_clips_accept_animation_transitions_and_follow_only() {
    let (mut project, asset_id) = fixture();
    let tracker = tracker(asset_id);
    let tracker_id = tracker.id;
    Arc::make_mut(&mut project.trackers).insert(tracker_id, Arc::new(tracker));
    let clip = &mut Arc::make_mut(&mut project.tracks[2]).clips[0];
    clip.animation = vec![
        opacity(vec![key(Time::ZERO, 0.0)]),
        track(None, ParamKey::Gain, vec![key(Time::ZERO, 2.0)]),
    ];
    clip.transition_in = Some(transition(seconds(1.0)));
    clip.follow = Some(Follow {
        tracker_id,
        position: true,
        scale: true,
        rotation: false,
        offset: [0.0, 0.0],
    });
    project.validate().unwrap();

    invalid(
        text(|clip| clip.effects.push(Effect::new(EffectKind::Blur))),
        "text clip look",
    );
    invalid(
        text(|clip| clip.masks.push(Mask::new(MaskShape::Rectangle))),
        "text clip look",
    );
    invalid(
        text(|clip| clip.blend = BlendMode::Screen),
        "text clip look",
    );
    invalid(
        text(|clip| clip.retime = Some(Retime::constant(clip.duration, 1.0))),
        "text clip look",
    );
}

#[test]
fn effect_rules_reject_out_of_contract_stacks() {
    media(|clip| {
        clip.effects = (0..16).map(|_| Effect::new(EffectKind::Blur)).collect();
    })
    .unwrap();
    invalid(
        media(|clip| {
            clip.effects = (0..17).map(|_| Effect::new(EffectKind::Blur)).collect();
        }),
        "effect or mask count",
    );
    invalid(
        media(|clip| {
            let effect = Effect::new(EffectKind::Blur);
            clip.effects = vec![effect.clone(), effect];
        }),
        "effect",
    );
    for mix in [-0.01, 1.01, f32::NAN, f32::INFINITY] {
        invalid(
            media(|clip| {
                let mut effect = Effect::new(EffectKind::Blur);
                effect.mix = mix;
                clip.effects.push(effect);
            }),
            "effect",
        );
    }
    let params = [
        vec![value(ParamKey::Exposure, 0.0)],
        vec![value(ParamKey::Radius, 0.26)],
        vec![value(ParamKey::Radius, -0.01)],
        vec![value(ParamKey::Radius, f32::NAN)],
        vec![value(ParamKey::Radius, f32::NEG_INFINITY)],
        vec![value(ParamKey::Radius, 0.1), value(ParamKey::Radius, 0.1)],
    ];
    for params in params {
        invalid(
            media(|clip| {
                let mut effect = Effect::new(EffectKind::Blur);
                effect.params = params;
                clip.effects.push(effect);
            }),
            "effect",
        );
    }
}

#[test]
fn curve_points_are_bounded_ordered_and_exclusive_to_curves() {
    let curve = |points: Vec<[f32; 2]>| {
        media(|clip| {
            let mut effect = Effect::new(EffectKind::CurveMaster);
            effect.points = points;
            clip.effects.push(effect);
        })
    };
    curve(vec![[0.0, 0.1], [0.5, 0.7], [1.0, 0.9]]).unwrap();
    curve((0..16).map(|i| [i as f32 / 15.0, 0.5]).collect()).unwrap();
    invalid(curve(Vec::new()), "effect");
    invalid(curve(vec![[0.5, 0.5]]), "effect");
    invalid(
        curve((0..17).map(|i| [i as f32 / 16.0, 0.5]).collect()),
        "effect",
    );
    invalid(curve(vec![[0.5, 0.0], [0.5, 1.0]]), "effect");
    invalid(curve(vec![[0.6, 0.0], [0.4, 1.0]]), "effect");
    invalid(curve(vec![[0.0, 0.0], [1.1, 1.0]]), "effect");
    invalid(curve(vec![[0.0, -0.1], [1.0, 1.0]]), "effect");
    invalid(curve(vec![[0.0, f32::NAN], [1.0, 1.0]]), "effect");
    invalid(curve(vec![[f32::NAN, 0.0], [1.0, 1.0]]), "effect");
    invalid(
        media(|clip| {
            let mut effect = Effect::new(EffectKind::Blur);
            effect.points = vec![[0.0, 0.0], [1.0, 1.0]];
            clip.effects.push(effect);
        }),
        "effect",
    );
}

#[test]
fn effect_assets_are_required_only_where_used_and_must_exist() {
    let (_, asset_id) = fixture();
    for kind in [EffectKind::Lut, EffectKind::DifferenceMatte] {
        invalid(media(|clip| clip.effects.push(Effect::new(kind))), "effect");
        let absent = Uuid::new_v4();
        missing(
            media(|clip| {
                let mut effect = Effect::new(kind);
                effect.asset_id = Some(absent);
                clip.effects.push(effect);
            }),
            absent,
        );
        let (mut project, _) = fixture();
        let asset_id = match kind {
            EffectKind::Lut => add_asset(&mut project, "look.cube", MediaKind::Lut),
            _ => add_asset(&mut project, "plate.png", MediaKind::Image),
        };
        let mut effect = Effect::new(kind);
        effect.asset_id = Some(asset_id);
        Arc::make_mut(&mut project.tracks[0]).clips[0]
            .effects
            .push(effect);
        project.validate().unwrap();
    }
    invalid(
        media(|clip| {
            let mut effect = Effect::new(EffectKind::Blur);
            effect.asset_id = Some(asset_id);
            clip.effects.push(effect);
        }),
        "effect",
    );
}

#[test]
fn an_effect_asset_is_of_the_kind_its_effect_reads() {
    // A lookup is done in a table and nothing else; a clean plate is a still.
    for (kind, wrong) in [
        (EffectKind::Lut, MediaKind::Image),
        (EffectKind::Lut, MediaKind::Video),
        (EffectKind::Lut, MediaKind::Gif),
        (EffectKind::Lut, MediaKind::Audio),
        (EffectKind::DifferenceMatte, MediaKind::Lut),
        (EffectKind::DifferenceMatte, MediaKind::Video),
        (EffectKind::DifferenceMatte, MediaKind::Gif),
        (EffectKind::DifferenceMatte, MediaKind::Audio),
    ] {
        let (mut project, _) = fixture();
        let asset_id = add_asset(&mut project, "wrong", wrong);
        project.validate().unwrap();
        let mut effect = Effect::new(kind);
        effect.asset_id = Some(asset_id);
        Arc::make_mut(&mut project.tracks[0]).clips[0]
            .effects
            .push(effect);
        invalid(project.validate(), "effect");
    }
}

#[test]
fn a_lookup_table_is_an_asset_and_never_a_clip() {
    let (mut project, _) = fixture();
    let lut = add_asset(&mut project, "look.cube", MediaKind::Lut);
    project.validate().unwrap();
    let table = project.assets[&lut].clone();
    assert!(!table.has_audio);

    // On no track, whatever it claims to carry.
    for track in 0..2 {
        let mut placed = project.clone();
        Arc::make_mut(&mut placed.tracks[track])
            .clips
            .push(Clip::from_asset(&table, seconds(100.0)));
        assert!(matches!(
            placed.validate(),
            Err(EditError::IncompatibleTrack)
        ));
    }
    // It has neither sound nor a frame grid.
    let mut loud = project.clone();
    Arc::make_mut(&mut loud.assets)
        .get_mut(&lut)
        .unwrap()
        .has_audio = true;
    invalid(loud.validate(), "asset metadata");
    let mut timed = project.clone();
    Arc::make_mut(&mut timed.assets)
        .get_mut(&lut)
        .unwrap()
        .frame_rate = Some(project.settings.frame_rate);
    invalid(timed.validate(), "asset metadata");
    // Nothing in it moves, so nothing is tracked in it.
    let mut tracked = project.clone();
    let path = tracker(lut);
    Arc::make_mut(&mut tracked.trackers).insert(path.id, Arc::new(path));
    invalid(tracked.validate(), "tracker");
    // And there is nothing to monitor.
    assert!(matches!(
        Project::asset_preview(&table),
        Err(EditError::IncompatibleTrack)
    ));

    let json = serde_json::to_string(&project).unwrap();
    assert!(json.contains("\"kind\":\"lut\""));
    assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), project);
}

#[test]
fn mask_rules_reject_out_of_contract_masks() {
    media(|clip| {
        clip.masks = (0..8).map(|_| Mask::new(MaskShape::Ellipse)).collect();
    })
    .unwrap();
    invalid(
        media(|clip| {
            clip.masks = (0..9).map(|_| Mask::new(MaskShape::Ellipse)).collect();
        }),
        "effect or mask count",
    );
    invalid(
        media(|clip| {
            let mask = Mask::new(MaskShape::Rectangle);
            clip.masks = vec![mask.clone(), mask];
        }),
        "mask",
    );
    invalid(
        media(|clip| {
            let effect = Effect::new(EffectKind::Blur);
            let mut mask = Mask::new(MaskShape::Rectangle);
            mask.id = effect.id;
            clip.effects.push(effect);
            clip.masks.push(mask);
        }),
        "mask",
    );
    let edits: [fn(&mut Mask); 9] = [
        |mask| mask.center[0] = f32::NAN,
        |mask| mask.center[1] = 2.5,
        |mask| mask.size[0] = -0.1,
        |mask| mask.size[1] = f32::INFINITY,
        |mask| mask.rotation = f32::NAN,
        |mask| mask.rotation = 36_001.0,
        |mask| mask.feather = 1.5,
        |mask| mask.roundness = -0.5,
        |mask| mask.roundness = f32::NAN,
    ];
    for edit in edits {
        invalid(
            media(|clip| {
                let mut mask = Mask::new(MaskShape::Rectangle);
                edit(&mut mask);
                clip.masks.push(mask);
            }),
            "mask",
        );
    }
}

#[test]
fn animation_rules_reject_out_of_contract_tracks() {
    let animate = |keys: Vec<Keyframe>| media(|clip| clip.animation.push(opacity(keys)));
    animate(vec![key(Time::ZERO, 0.0), key(seconds(10.0), 1.0)]).unwrap();
    animate((0..256).map(|i| key(Time(i), 0.5)).collect()).unwrap();
    invalid(animate(Vec::new()), "animation keys");
    invalid(
        animate((0..257).map(|i| key(Time(i), 0.5)).collect()),
        "animation keys",
    );
    invalid(
        animate(vec![key(seconds(1.0), 0.0), key(seconds(1.0), 1.0)]),
        "animation keys",
    );
    invalid(
        animate(vec![key(seconds(2.0), 0.0), key(seconds(1.0), 1.0)]),
        "animation keys",
    );
    invalid(animate(vec![key(Time(-1), 0.0)]), "animation keys");
    invalid(
        animate(vec![key(seconds(10.0) + Time(1), 0.0)]),
        "animation keys",
    );
    for value in [1.01, -0.01, f32::NAN, f32::INFINITY] {
        invalid(animate(vec![key(Time::ZERO, value)]), "animation keys");
    }
    let eases = [
        NAN_EASE,
        Ease::Bezier {
            x1: 1.1,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        },
        Ease::Bezier {
            x1: 0.0,
            y1: 0.0,
            x2: -0.1,
            y2: 1.0,
        },
        Ease::Bezier {
            x1: 0.0,
            y1: 4.1,
            x2: 1.0,
            y2: 1.0,
        },
        Ease::Bezier {
            x1: 0.0,
            y1: 0.0,
            x2: 1.0,
            y2: f32::NEG_INFINITY,
        },
    ];
    for ease in eases {
        invalid(
            animate(vec![Keyframe {
                at: Time::ZERO,
                value: 0.5,
                ease,
            }]),
            "animation keys",
        );
    }
    animate(vec![Keyframe {
        at: Time::ZERO,
        value: 0.5,
        ease: Ease::Bezier {
            x1: 0.3,
            y1: -4.0,
            x2: 0.7,
            y2: 4.0,
        },
    }])
    .unwrap();

    invalid(
        media(|clip| {
            clip.animation = vec![
                opacity(vec![key(Time::ZERO, 0.0)]),
                opacity(vec![key(Time::ZERO, 1.0)]),
            ];
        }),
        "animation keys",
    );
    invalid(
        media(|clip| {
            clip.animation = (0..65)
                .map(|_| opacity(vec![key(Time::ZERO, 0.0)]))
                .collect();
        }),
        "animation track count",
    );
}

#[test]
fn animation_targets_must_exist_and_accept_the_key() {
    invalid(
        media(|clip| {
            clip.animation
                .push(track(None, ParamKey::Radius, vec![key(Time::ZERO, 0.0)]));
        }),
        "animation parameter",
    );
    let absent = Uuid::new_v4();
    missing(
        media(|clip| {
            clip.animation.push(track(
                Some(absent),
                ParamKey::Radius,
                vec![key(Time::ZERO, 0.0)],
            ));
        }),
        absent,
    );
    let on_blur = |param: ParamKey, value: f32| {
        media(|clip| {
            let effect = Effect::new(EffectKind::Blur);
            clip.animation
                .push(track(Some(effect.id), param, vec![key(Time::ZERO, value)]));
            clip.effects.push(effect);
        })
    };
    on_blur(ParamKey::Radius, 0.2).unwrap();
    invalid(on_blur(ParamKey::Opacity, 0.2), "animation parameter");
    invalid(on_blur(ParamKey::Radius, 0.3), "animation keys");
    let on_mask = |param: ParamKey, value: f32| {
        media(|clip| {
            let mask = Mask::new(MaskShape::Ellipse);
            clip.animation
                .push(track(Some(mask.id), param, vec![key(Time::ZERO, value)]));
            clip.masks.push(mask);
        })
    };
    on_mask(ParamKey::CenterX, 0.2).unwrap();
    invalid(on_mask(ParamKey::Amount, 0.2), "animation parameter");
    invalid(on_mask(ParamKey::Feather, f32::NAN), "animation keys");
    // The same key may animate two different targets.
    media(|clip| {
        let (first, second) = (Effect::new(EffectKind::Blur), Effect::new(EffectKind::Glow));
        clip.animation = vec![
            track(Some(first.id), ParamKey::Radius, vec![key(Time::ZERO, 0.1)]),
            track(
                Some(second.id),
                ParamKey::Radius,
                vec![key(Time::ZERO, 0.1)],
            ),
        ];
        clip.effects = vec![first, second];
    })
    .unwrap();
}

#[test]
fn transition_duration_is_bounded_by_frame_clip_and_ten_seconds() {
    let frame = Time(33_333);
    let edge = |duration: Time| media(|clip| clip.transition_in = Some(transition(duration)));
    edge(frame).unwrap();
    edge(seconds(10.0)).unwrap();
    invalid(edge(frame - Time(1)), "transition");
    invalid(edge(Time::ZERO), "transition");
    invalid(edge(seconds(10.0) + Time(1)), "transition");
    invalid(
        media(|clip| {
            clip.duration = seconds(5.0);
            clip.transition_out = Some(transition(seconds(5.0) + Time(1)));
        }),
        "transition",
    );
    invalid(
        media(|clip| {
            clip.duration = seconds(20.0);
            clip.transition_out = Some(transition(seconds(10.0) + Time(1)));
        }),
        "transition",
    );
    media(|clip| {
        clip.duration = seconds(20.0);
        clip.transition_out = Some(transition(seconds(10.0)));
    })
    .unwrap();
    invalid(
        media(|clip| {
            clip.transition_in = Some(Transition {
                ease: NAN_EASE,
                ..transition(seconds(1.0))
            });
        }),
        "transition",
    );
    for params in [
        vec![value(ParamKey::Softness, 0.1)],
        vec![value(ParamKey::Amount, f32::NAN)],
        vec![value(ParamKey::Amount, 4.5)],
    ] {
        invalid(
            media(|clip| {
                clip.transition_in = Some(Transition {
                    kind: TransitionKind::ZoomIn,
                    params,
                    ..transition(seconds(1.0))
                });
            }),
            "transition",
        );
    }
}

#[test]
fn retime_rules_reject_out_of_contract_maps() {
    let map = |keys: &[(f64, f64)]| {
        let retime = retime(keys);
        media(|clip| clip.retime = Some(retime))
    };
    map(&[(0.0, 0.0), (10.0, 10.0)]).unwrap();
    map(&[(0.0, 10.0), (10.0, 0.0)]).unwrap();
    map(&[(0.0, 0.0), (4.0, 4.0), (6.0, 4.0), (10.0, 8.0)]).unwrap();
    map(&[(0.0, 0.0), (1.0, 16.0), (10.0, 16.45)]).unwrap();

    invalid(map(&[(0.0, 0.0)]), "time map");
    invalid(map(&[]), "time map");
    invalid(map(&[(1.0, 0.0), (10.0, 9.0)]), "time map");
    invalid(map(&[(0.0, 0.0), (9.0, 9.0)]), "time map");
    invalid(map(&[(0.0, 0.0), (11.0, 11.0)]), "time map");
    invalid(
        map(&[(0.0, 0.0), (5.0, 5.0), (5.0, 6.0), (10.0, 10.0)]),
        "time map",
    );
    invalid(
        map(&[(0.0, 0.0), (6.0, 6.0), (5.0, 5.0), (10.0, 10.0)]),
        "time map",
    );
    invalid(map(&[(0.0, 0.0), (1.0, 16.1), (10.0, 20.0)]), "time map");
    invalid(map(&[(0.0, 0.0), (10.0, 0.4)]), "time map");
    invalid(map(&[(0.0, -1.0), (10.0, 9.0)]), "time map");
    invalid(
        media(|clip| {
            clip.retime = Some(Retime {
                keys: (0..65)
                    .map(|i| TimeKey {
                        at: Time(i * 156_250),
                        source: Time(i * 156_250),
                        ease: Ease::Linear,
                    })
                    .collect(),
            });
        }),
        "time map",
    );
    invalid(
        media(|clip| {
            clip.playback_rate = 2.0;
            clip.duration = seconds(5.0);
            clip.retime = Some(Retime::constant(clip.duration, 1.0));
        }),
        "time map",
    );
    for ease in [
        NAN_EASE,
        Ease::Bezier {
            x1: 0.3,
            y1: 1.5,
            x2: 0.7,
            y2: 1.0,
        },
        Ease::Bezier {
            x1: 0.3,
            y1: 0.0,
            x2: 0.7,
            y2: -0.5,
        },
    ] {
        invalid(
            media(|clip| {
                let mut retime = Retime::constant(clip.duration, 1.0);
                retime.keys[0].ease = ease;
                clip.retime = Some(retime);
            }),
            "time map",
        );
    }
    // The asset is 60 s and the clip reads from 5 s: 55 s of source remain.
    map(&[(0.0, 0.0), (10.0, 55.0)]).unwrap();
    invalid(
        map(&[(0.0, 0.0), (10.0, 55.1)]),
        "clip extends beyond source",
    );
    invalid(
        map(&[(0.0, 55.1), (10.0, 0.0)]),
        "clip extends beyond source",
    );
}

#[test]
fn tracker_rules_reject_out_of_contract_paths() {
    with_tracker(|_| {}).unwrap();
    let absent = Uuid::new_v4();
    missing(with_tracker(|tracker| tracker.asset_id = absent), absent);
    let edits: [fn(&mut Tracker); 13] = [
        |tracker| tracker.name = "  ".into(),
        |tracker| tracker.keys.clear(),
        |tracker| tracker.keys[1].source = Time::ZERO,
        |tracker| tracker.keys.swap(0, 1),
        |tracker| tracker.keys[0].source = Time(-1),
        |tracker| tracker.keys[1].source = Time(60_000_003),
        |tracker| tracker.keys[0].center[0] = f32::NAN,
        |tracker| tracker.keys[0].center[1] = 4.5,
        |tracker| tracker.keys[0].scale = 0.0,
        |tracker| tracker.keys[0].scale = f32::NAN,
        |tracker| tracker.keys[0].rotation = f32::INFINITY,
        |tracker| tracker.keys[0].rotation = f32::NAN,
        |tracker| {
            tracker.keys = (0..2049)
                .map(|i| TrackKey {
                    source: Time(i),
                    center: [0.5, 0.5],
                    scale: 1.0,
                    rotation: 0.0,
                })
                .collect();
        },
    ];
    for edit in edits {
        invalid(with_tracker(edit), "tracker");
    }
    with_tracker(|tracker| {
        tracker.keys = (0..2048)
            .map(|i| TrackKey {
                source: Time(i),
                center: [0.5, 0.5],
                scale: 1.0,
                rotation: 0.0,
            })
            .collect();
    })
    .unwrap();

    let (mut project, asset_id) = fixture();
    Arc::make_mut(&mut project.trackers).insert(Uuid::new_v4(), Arc::new(tracker(asset_id)));
    invalid(project.validate(), "tracker");

    let (mut project, asset_id) = fixture();
    let mut colliding = tracker(asset_id);
    colliding.id = project.tracks[0].id;
    Arc::make_mut(&mut project.trackers).insert(colliding.id, Arc::new(colliding));
    invalid(project.validate(), "tracker");

    let (mut project, asset_id) = fixture();
    for _ in 0..64 {
        let tracker = tracker(asset_id);
        Arc::make_mut(&mut project.trackers).insert(tracker.id, Arc::new(tracker));
    }
    project.validate().unwrap();
    let extra = tracker(asset_id);
    Arc::make_mut(&mut project.trackers).insert(extra.id, Arc::new(extra));
    invalid(project.validate(), "tracker count");
}

#[test]
fn follow_requires_an_existing_tracker_and_finite_offset() {
    let follow = |tracker_id: Uuid, offset: [f32; 2]| Follow {
        tracker_id,
        position: true,
        scale: true,
        rotation: true,
        offset,
    };
    let absent = Uuid::new_v4();
    missing(
        media(|clip| clip.follow = Some(follow(absent, [0.0, 0.0]))),
        absent,
    );
    for offset in [[f32::NAN, 0.0], [0.0, f32::INFINITY], [0.0, 2_000_000.0]] {
        let (mut project, asset_id) = fixture();
        let tracker = tracker(asset_id);
        let tracker_id = tracker.id;
        Arc::make_mut(&mut project.trackers).insert(tracker_id, Arc::new(tracker));
        Arc::make_mut(&mut project.tracks[0]).clips[0].follow = Some(follow(tracker_id, offset));
        invalid(project.validate(), "follow offset");
    }
}

#[test]
fn spec_table_is_well_formed_for_every_kind() {
    let tables = EffectKind::ALL
        .iter()
        .map(|kind| look::spec(*kind))
        .chain([look::mask_spec(), look::clip_spec()]);
    for table in tables {
        for (index, spec) in table.iter().enumerate() {
            assert!(spec.min.is_finite() && spec.max.is_finite());
            assert!(spec.min < spec.max, "{spec:?}");
            assert!(spec.contains(spec.default), "{spec:?}");
            assert!(table[..index].iter().all(|earlier| earlier.key != spec.key));
        }
    }
    for kind in EffectKind::ALL {
        let mut effect = Effect::new(kind);
        effect.asset_id = kind.needs_asset().then(Uuid::new_v4);
        assert!(effect.validate(), "{kind:?}");
        for spec in look::spec(kind) {
            assert_eq!(effect.param(spec.key), Some(spec.default));
            assert!(look::animatable(AnimTarget::Effect(kind), spec.key));
        }
        assert_eq!(effect.param(ParamKey::PositionX), None);
    }
    let mut glow = Effect::new(EffectKind::Glow);
    glow.params.push(value(ParamKey::Amount, 2.0));
    assert_eq!(glow.param(ParamKey::Amount), Some(2.0));

    let mask_keys = [
        ParamKey::CenterX,
        ParamKey::CenterY,
        ParamKey::Width,
        ParamKey::Height,
        ParamKey::Rotation,
        ParamKey::Feather,
        ParamKey::Roundness,
    ];
    assert_eq!(look::mask_spec().len(), mask_keys.len());
    let mask = Mask::new(MaskShape::Rectangle);
    assert!(mask.validate());
    for key in mask_keys {
        assert!(look::animatable(AnimTarget::Mask, key));
        assert!(mask.param(key).is_some());
    }
    assert!(!look::animatable(AnimTarget::Mask, ParamKey::Opacity));
    assert!(look::animatable(AnimTarget::Clip, ParamKey::Gain));
    assert!(look::animatable(AnimTarget::Clip, ParamKey::CropBottom));
    assert!(!look::animatable(AnimTarget::Clip, ParamKey::Amount));
    assert!(!look::animatable(
        AnimTarget::Effect(EffectKind::Lut),
        ParamKey::Amount
    ));
    assert!(!look::animatable(
        AnimTarget::Effect(EffectKind::Grade),
        ParamKey::Radius
    ));
}

#[test]
fn ease_hits_its_endpoints_and_stays_monotone() {
    let eases = [
        Ease::Linear,
        EASE_IN,
        EASE_IN_OUT,
        Ease::Bezier {
            x1: 0.0,
            y1: 0.0,
            x2: 0.0,
            y2: 1.0,
        },
        Ease::Bezier {
            x1: 1.0,
            y1: 1.0,
            x2: 1.0,
            y2: 0.0,
        },
    ];
    for ease in eases {
        assert!(ease.is_monotone());
        assert_eq!(ease.eval(0.0), 0.0);
        assert_eq!(ease.eval(1.0), 1.0);
        assert_eq!(ease.eval(-1.0), 0.0);
        assert_eq!(ease.eval(2.0), 1.0);
        assert_eq!(ease.eval(f64::NAN), 0.0);
        let mut previous = 0.0;
        for step in 0..=1000 {
            let eased = ease.eval(f64::from(step) / 1000.0);
            assert!((0.0..=1.0).contains(&eased));
            assert!(eased >= previous, "{ease:?} at {step}");
            assert!(ease.slope(f64::from(step) / 1000.0).is_finite());
            previous = eased;
        }
    }
    assert_eq!(Ease::Linear.eval(0.25), 0.25);
    assert_eq!(Ease::Hold.eval(0.999), 0.0);
    assert_eq!(Ease::Hold.eval(1.0), 1.0);
    // A cubic with collinear control points is the identity.
    let identity = Ease::Bezier {
        x1: 0.25,
        y1: 0.25,
        x2: 0.75,
        y2: 0.75,
    };
    for step in 0..=100 {
        let t = f64::from(step) / 100.0;
        assert!((identity.eval(t) - t).abs() < 1e-6);
    }
    assert!((EASE_IN_OUT.eval(0.5) - 0.5).abs() < 1e-6);
    assert!(EASE_IN.eval(0.5) < 0.5);
    assert!(EASE_IN.slope(0.05) < 1.0 && EASE_IN.slope(0.95) > 1.0);
    // Overshoot is legal for animation but not for a time map.
    let back = Ease::Bezier {
        x1: 0.34,
        y1: 1.56,
        x2: 0.64,
        y2: 1.0,
    };
    assert!(back.validate() && !back.is_monotone());
    assert!(back.eval(0.6) > 1.0);
    assert!(!NAN_EASE.validate() && !NAN_EASE.is_monotone());
}

#[test]
fn sample_holds_outside_keys_and_eases_between_them() {
    let track = opacity(vec![
        key(seconds(1.0), 0.2),
        Keyframe {
            at: seconds(3.0),
            value: 0.6,
            ease: Ease::Hold,
        },
        Keyframe {
            at: seconds(5.0),
            value: 1.0,
            ease: EASE_IN,
        },
        key(seconds(9.0), 0.0),
    ]);
    assert_eq!(track.sample(Time::ZERO), 0.2);
    assert_eq!(track.sample(seconds(1.0)), 0.2);
    assert!((track.sample(seconds(2.0)) - 0.4).abs() < 1e-6);
    assert_eq!(track.sample(seconds(3.0)), 0.6);
    assert_eq!(track.sample(seconds(5.0) - Time(1)), 0.6);
    assert_eq!(track.sample(seconds(5.0)), 1.0);
    let eased = track.sample(seconds(7.0));
    assert!(eased > 0.5 && eased < 1.0);
    assert_eq!(track.sample(seconds(9.0)), 0.0);
    assert_eq!(track.sample(seconds(20.0)), 0.0);
    assert_eq!(
        opacity(vec![key(seconds(4.0), 0.7)]).sample(Time::ZERO),
        0.7
    );
    assert_eq!(opacity(Vec::new()).sample(Time::ZERO), 0.0);
}

fn media_clip() -> Clip {
    let (project, _) = fixture();
    project.tracks[0].clips[0].clone()
}

#[test]
fn unretimed_clip_keeps_the_linear_mapping() {
    let mut clip = media_clip();
    clip.playback_rate = 2.0;
    clip.duration = seconds(5.0);
    for playhead in [0.0, 2.0, 3.5, 7.0, 9.0] {
        let playhead = seconds(playhead);
        assert_eq!(
            clip.source_time(playhead),
            clip.source_in + (playhead - clip.start).scaled(2.0)
        );
        assert_eq!(clip.speed_at(playhead), 2.0);
    }
    assert_eq!(clip.source_span(), (seconds(5.0), seconds(15.0)));
}

#[test]
fn linear_retime_matches_the_constant_rate_it_replaces() {
    let mut linear = media_clip();
    linear.playback_rate = 2.0;
    let mut clip = media_clip();
    clip.retime = Some(Retime::constant(clip.duration, 2.0));
    for playhead in [0.0, 2.0, 3.3333, 7.0, 12.0, 15.0] {
        let playhead = seconds(playhead);
        assert_eq!(clip.source_time(playhead), linear.source_time(playhead));
        assert_eq!(clip.speed_at(playhead), 2.0);
    }
    assert_eq!(clip.source_time(seconds(5.0)), seconds(11.0));
    assert_eq!(clip.source_span(), linear.source_span());
    assert_eq!(clip.source_span(), (seconds(5.0), seconds(25.0)));
}

#[test]
fn freeze_holds_the_source_and_reports_zero_speed() {
    let mut clip = media_clip();
    clip.retime = Some(retime(&[(0.0, 0.0), (4.0, 4.0), (6.0, 4.0), (10.0, 8.0)]));
    assert_eq!(clip.source_time(seconds(4.0)), seconds(7.0));
    assert_eq!(clip.source_time(seconds(6.0)), seconds(9.0));
    assert_eq!(clip.source_time(seconds(7.0)), seconds(9.0));
    assert_eq!(clip.source_time(seconds(8.0)), seconds(9.0));
    assert_eq!(clip.source_time(seconds(10.0)), seconds(11.0));
    assert_eq!(clip.source_time(clip.end()), seconds(13.0));
    assert_eq!(clip.speed_at(seconds(3.0)), 1.0);
    assert_eq!(clip.speed_at(seconds(6.0)), 0.0);
    assert_eq!(clip.speed_at(seconds(7.0)), 0.0);
    assert_eq!(clip.speed_at(seconds(8.0)), 1.0);
    assert_eq!(clip.source_span(), (seconds(5.0), seconds(13.0)));

    let mut held = media_clip();
    let mut map = retime(&[(0.0, 2.0), (10.0, 6.0)]);
    map.keys[0].ease = Ease::Hold;
    held.retime = Some(map);
    assert_eq!(held.source_time(seconds(11.0)), seconds(7.0));
    assert_eq!(held.source_time(held.end()), seconds(11.0));
    assert_eq!(held.speed_at(seconds(11.0)), 0.0);
}

#[test]
fn reverse_plays_the_source_backwards() {
    let mut clip = media_clip();
    clip.retime = Some(retime(&[(0.0, 10.0), (10.0, 0.0)]));
    assert_eq!(clip.source_time(clip.start), seconds(15.0));
    assert_eq!(clip.source_time(seconds(5.0)), seconds(12.0));
    assert_eq!(clip.source_time(clip.end()), seconds(5.0));
    assert_eq!(clip.speed_at(seconds(5.0)), -1.0);
    assert_eq!(clip.source_span(), (seconds(5.0), seconds(15.0)));
    let mut previous = clip.source_time(clip.start);
    for step in 1..=100 {
        let source = clip.source_time(clip.start + Time(step * 100_000));
        assert!(source < previous);
        previous = source;
    }
}

#[test]
fn ramp_eases_between_its_keys_without_reversing() {
    let mut clip = media_clip();
    let mut map = retime(&[(0.0, 0.0), (10.0, 20.0)]);
    map.keys[0].ease = EASE_IN;
    clip.retime = Some(map);
    assert_eq!(clip.source_time(clip.start), seconds(5.0));
    assert_eq!(clip.source_time(clip.end()), seconds(25.0));
    assert!(clip.source_time(seconds(7.0)) < seconds(15.0));
    let mut previous = clip.source_time(clip.start);
    for step in 1..=1000 {
        let source = clip.source_time(clip.start + Time(step * 10_000));
        assert!(source >= previous, "step {step}");
        previous = source;
    }
    let (early, late) = (clip.speed_at(seconds(2.5)), clip.speed_at(seconds(11.5)));
    assert!(early > 0.0 && early < 2.0, "{early}");
    assert!(late > 2.0 && late.is_finite(), "{late}");
    assert_eq!(clip.source_span(), (seconds(5.0), seconds(25.0)));
}

#[test]
fn retime_continues_at_the_edge_speed_outside_the_clip() {
    let mut clip = media_clip();
    clip.retime = Some(retime(&[(0.0, 2.0), (5.0, 7.0), (10.0, 17.0)]));
    assert_eq!(clip.source_time(seconds(1.0)), seconds(6.0));
    assert_eq!(clip.source_time(seconds(13.0)), seconds(24.0));
    assert_eq!(clip.speed_at(seconds(1.0)), 1.0);
    assert_eq!(clip.speed_at(seconds(13.0)), 2.0);
    assert_eq!(clip.source_span(), (seconds(7.0), seconds(22.0)));
}
