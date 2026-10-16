use editor_core::*;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn animated() -> (Editor, ClipId, AssetId) {
    let mut project = Project::new("Patch fixture");
    let asset = Asset::new(PathBuf::from("source.mp4"), MediaKind::Video, seconds(30.0));
    let mut clip = Clip::from_asset(&asset, seconds(2.0));
    clip.duration = seconds(10.0);
    clip.animation = vec![AnimTrack {
        target: None,
        key: ParamKey::PositionX,
        keys: vec![
            Keyframe {
                at: Time::ZERO,
                value: 0.0,
                ease: Ease::Bezier {
                    x1: 0.42,
                    y1: 0.0,
                    x2: 0.58,
                    y2: 1.0,
                },
            },
            Keyframe {
                at: seconds(10.0),
                value: 100.0,
                ease: Ease::Linear,
            },
        ],
    }];
    let ids = (clip.id, asset.id);
    Arc::make_mut(&mut project.assets).insert(asset.id, asset);
    Arc::make_mut(&mut project.tracks[0]).clips.push(clip);
    (Editor::new(project).unwrap(), ids.0, ids.1)
}

fn patch(clip_id: ClipId, fields: Value) -> Command {
    Command::PatchClip {
        clip_id,
        patch: fields.as_object().unwrap().clone(),
        trim_animation: AnimationTrim::Crop,
    }
}

fn rejected(editor: &mut Editor, command: Command) -> String {
    let before = editor.snapshot();
    let revision = editor.revision();
    let count = editor.history().count();
    let error = editor.dispatch(command).unwrap_err().to_string();
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    assert_eq!(editor.revision(), revision);
    assert_eq!(editor.history().count(), count);
    error
}

#[test]
fn patch_keeps_identity_and_unspecified_fields_and_is_one_reversible_step() {
    let (mut editor, id, asset) = animated();
    let mut replacement = editor.project().assets[&asset].clone();
    replacement.id = Uuid::new_v4();
    let replacement_id = replacement.id;
    editor
        .dispatch(Command::AddAssets {
            assets: vec![replacement],
        })
        .unwrap();
    let before = editor.snapshot();
    let command = patch(
        id,
        json!({
            "start": 7_000_000, "source_in": 16_667,
            "content": {"asset_id": replacement_id},
            "transform": {"opacity": 0.6}
        }),
    );
    let wire = serde_json::to_value(&command).unwrap();
    let command = serde_json::from_value(wire).unwrap();
    editor.dispatch(command).unwrap();
    let updated = editor.project().clip(id).unwrap().1;
    let mut expected = before.clip(id).unwrap().1.clone();
    expected.start = seconds(7.0);
    expected.source_in = Time(16_667);
    expected.content = ClipContent::Media {
        asset_id: replacement_id,
    };
    expected.transform.opacity = 0.6;
    assert_eq!(updated, &expected);
    assert_eq!(editor.history().count(), 2);
    let after = editor.snapshot();
    assert!(editor.undo());
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    assert!(editor.redo());
    assert!(Arc::ptr_eq(&after, &editor.snapshot()));
}

#[test]
fn shortening_crops_existing_curve_with_interpolated_end_and_unchanged_samples() {
    let (mut editor, id, _) = animated();
    let original = editor.project().clip(id).unwrap().1.animation[0].clone();
    editor
        .dispatch(patch(id, json!({"duration": 4_000_000})))
        .unwrap();
    let updated = editor.project().clip(id).unwrap().1;
    let cropped = &updated.animation[0];
    assert_eq!(updated.duration, seconds(4.0));
    assert_eq!(cropped.keys.last().unwrap().at, updated.duration);
    for tick in 0..=40 {
        let time = Time(tick * 100_000);
        assert!((cropped.sample(time) - original.sample(time)).abs() < 0.0001);
    }
}

#[test]
fn preserve_policy_and_explicit_out_of_bounds_keys_fail_atomically_with_context() {
    let (mut editor, id, _) = animated();
    let error = rejected(
        &mut editor,
        Command::PatchClip {
            clip_id: id,
            patch: json!({"duration": 4_000_000}).as_object().unwrap().clone(),
            trim_animation: AnimationTrim::Preserve,
        },
    );
    assert!(error.contains(&id.to_string()), "{error}");
    assert!(error.contains("animation[0]"), "{error}");
    assert!(error.contains("keys[1]"), "{error}");
    assert!(error.contains("10000000 us"), "{error}");
    assert!(error.contains("4000000 us"), "{error}");

    let animation = editor.project().clip(id).unwrap().1.animation.clone();
    let error = rejected(
        &mut editor,
        patch(
            id,
            json!({
                "duration": 4_000_000, "animation": animation
            }),
        ),
    );
    assert!(error.contains("keys[1]"), "{error}");
}

#[test]
fn recursive_content_patch_preserves_style_and_arrays_replace() {
    let mut project = Project::new("Text fixture");
    let mut title = Clip::text("Hello world", Time::ZERO, seconds(4.0));
    if let ClipContent::Text { style, words, .. } = &mut title.content {
        style.font.weight = 800;
        style.outline = Some(TextOutline {
            color: [0, 0, 0, 255],
            width: 0.1,
        });
        *words = vec![
            WordTime {
                at: Time::ZERO,
                end: seconds(1.0),
            },
            WordTime {
                at: seconds(1.0),
                end: seconds(4.0),
            },
        ];
    }
    let id = title.id;
    Arc::make_mut(&mut project.tracks[2]).clips.push(title);
    let mut editor = Editor::new(project).unwrap();
    editor
        .dispatch(patch(
            id,
            json!({"content": {
                "style": {"font": {"italic": true}, "outline": null}, "words": []
            }}),
        ))
        .unwrap();
    let ClipContent::Text {
        style, words, text, ..
    } = &editor.project().clip(id).unwrap().1.content
    else {
        panic!()
    };
    assert_eq!(text, "Hello world");
    assert_eq!(style.font.weight, 800);
    assert!(style.font.italic);
    assert!(style.outline.is_none());
    assert!(words.is_empty());
}

#[test]
fn patch_rejects_unknown_and_protected_fields_including_null_without_mutation() {
    let (mut editor, id, _) = animated();
    for fields in [
        json!({"duraton": 1}),
        json!({"transform": {"opactiy": 0.5}}),
        json!({"transform": {"opactiy": null}}),
        json!({"content": {"asset": Uuid::new_v4()}}),
        json!({"id": id}),
        json!({"linked_id": null}),
        json!({"group_id": Uuid::new_v4()}),
    ] {
        let error = rejected(&mut editor, patch(id, fields));
        assert!(error.contains(&id.to_string()), "{error}");
        assert!(
            error.contains("unknown field") || error.contains("stable"),
            "{error}"
        );
    }
}

#[test]
fn nested_unknown_fields_on_default_optional_content_are_rejected() {
    let mut project = Project::new("Text fixture");
    let title = Clip::text("Hello", Time::ZERO, seconds(4.0));
    let id = title.id;
    Arc::make_mut(&mut project.tracks[2]).clips.push(title);
    let mut editor = Editor::new(project).unwrap();
    for fields in [
        json!({"content": {"style": {"font": {"famly": "Wrong"}}}}),
        json!({"content": {"style": {"shadow": {"blur": 0.1, "color": [0,0,0,255], "offset": [0.0,0.0], "wrong": null}}}}),
        json!({"content": {"words": [{"at": 0, "end": 1, "word": "wrong"}]}}),
    ] {
        let error = rejected(&mut editor, patch(id, fields));
        assert!(error.contains("unknown field"), "{error}");
    }
}

#[test]
fn patches_cannot_bypass_locks_source_bounds_or_batch_atomicity() {
    let (mut editor, id, _) = animated();
    let error = rejected(&mut editor, patch(id, json!({"source_in": 29_000_000})));
    assert!(error.contains(&id.to_string()), "{error}");
    for duration in [0, -1, i64::MAX] {
        rejected(&mut editor, patch(id, json!({"duration": duration})));
    }
    rejected(
        &mut editor,
        Command::Batch {
            label: "Atomic patch".into(),
            commands: vec![
                patch(id, json!({"name": "Changed"})),
                patch(id, json!({"duration": 0})),
            ],
        },
    );
    let track_id = editor.project().tracks[0].id;
    editor
        .dispatch(Command::SetTrackState {
            track_id,
            locked: Some(true),
            hidden: None,
            muted: None,
            name: None,
            gain: None,
            solo: None,
        })
        .unwrap();
    assert!(rejected(&mut editor, patch(id, json!({"name": "Forbidden"}))).contains("locked"));
}

#[test]
fn trim_already_crops_curves_without_delete_and_add() {
    let (mut editor, id, _) = animated();
    let original = editor.project().clip(id).unwrap().1.animation[0].clone();
    editor
        .dispatch(Command::TrimClip {
            clip_id: id,
            source_in: seconds(1.0),
            duration: seconds(4.0),
            ripple: false,
        })
        .unwrap();
    let updated = editor.project().clip(id).unwrap().1;
    assert_eq!(updated.start, seconds(3.0));
    assert_eq!(updated.source_in, seconds(1.0));
    for tick in 0..=40 {
        let time = Time(tick * 100_000);
        assert!(
            (updated.animation[0].sample(time) - original.sample(time + seconds(1.0))).abs()
                < 0.0001
        );
    }
}

#[test]
fn invalid_words_and_text_animation_identify_clip_and_field() {
    let mut project = Project::new("Text validation");
    let title = Clip::text("Hello world", Time::ZERO, seconds(4.0));
    let id = title.id;
    Arc::make_mut(&mut project.tracks[2]).clips.push(title);
    let mut editor = Editor::new(project).unwrap();
    let error = rejected(
        &mut editor,
        patch(
            id,
            json!({"content": {"words": [
                {"at": 0, "end": 1_000_000}, {"at": 1_000_000, "end": 5_000_000}
            ]}}),
        ),
    );
    assert!(
        error.contains(&id.to_string()) && error.contains("content.words[1]"),
        "{error}"
    );
    let animation = TextAnimation {
        kind: TextAnimationKind::Fade,
        unit: TextUnit::Whole,
        duration: seconds(5.0),
        delay: Time::ZERO,
    };
    let error = rejected(
        &mut editor,
        patch(id, json!({"content": {"enter": animation}})),
    );
    assert!(
        error.contains(&id.to_string()) && error.contains("content.enter"),
        "{error}"
    );
}
