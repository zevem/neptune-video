use editor_core::{
    Asset, BlendMode, Clip, ClipId, Command, Ease, EditError, Editor, Effect, EffectKind, Follow,
    Keyframe, Mask, MaskShape, MediaKind, ParamKey, Project, Retime, Time, TimeKey, TrackKey,
    Tracker, Transition, TransitionEdge, TransitionKind,
};
use editor_store::{
    SCHEMA_VERSION, StoreError, checkpoint, list_checkpoints, load_project,
    load_project_with_session, load_recovery, load_recovery_with_document, recovery_path,
    save_project, save_project_with_session, save_recovery,
};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc};
use tempfile::TempDir;
use uuid::Uuid;

const EASE: Ease = Ease::Bezier {
    x1: 0.42,
    y1: 0.0,
    x2: 0.58,
    y2: 1.0,
};

/// A project exactly as a version 1 build serialized it.
fn v1_project(name: &str) -> Value {
    json!({
        "id": "00000000-0000-4000-8000-000000000001",
        "name": name,
        "settings": {
            "width": 1920,
            "height": 1080,
            "frame_rate": { "numerator": 30, "denominator": 1 },
            "background": [14, 15, 18, 255],
            "audio_sample_rate": 48000
        },
        "assets": { "00000000-0000-4000-8000-000000000002": v1_asset() },
        "folders": {},
        "tracks": [v1_track()],
        "markers": []
    })
}

fn v1_asset() -> Value {
    json!({
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
    })
}

fn v1_track() -> Value {
    json!({
        "id": "00000000-0000-4000-8000-000000000003",
        "name": "Video 1",
        "kind": "video",
        "locked": false,
        "hidden": false,
        "muted": false,
        "clips": [{
            "id": "00000000-0000-4000-8000-000000000004",
            "name": "a.mp4",
            "content": {
                "type": "media",
                "asset_id": "00000000-0000-4000-8000-000000000002"
            },
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
        }]
    })
}

/// A version 1 recovery archive: one rename in history, saved before it.
fn v1_archive() -> Value {
    let record = |name: &str| {
        json!({
            "id": "00000000-0000-4000-8000-000000000001",
            "name": name,
            "settings": v1_project(name)["settings"],
            "assets": 0,
            "folders": 0,
            "tracks": [0],
            "markers": 0
        })
    };
    json!({
        "schema_version": 1,
        "revision": 1,
        "current": 1,
        "saved": 0,
        "projects": [record("Version one"), record("Renamed")],
        "tracks": [v1_track()],
        "assets": [{ "00000000-0000-4000-8000-000000000002": v1_asset() }],
        "folders": [{}],
        "markers": [[]],
        "past": [{ "label": "Rename project", "before": 0, "after": 1 }],
        "future": []
    })
}

/// A project, a checkpoint and a recovery archive of one session, as the last
/// build that wrote version 2 saved them: every advanced feature, two steps
/// undone in the archive, and media named beside the document.
const V2_PROJECT: &str = include_str!("fixtures/version-two.aster");
const V2_CHECKPOINT: &str = include_str!("fixtures/version-two-checkpoint.aster");
const V2_ARCHIVE: &str = include_str!("fixtures/version-two.aster.recovery");

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn write_json(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn write_text(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn apply(editor: &mut Editor, command: Command) {
    assert!(editor.dispatch(command).unwrap().changed);
}

fn keys(values: [(f64, f32); 2]) -> Vec<Keyframe> {
    values
        .into_iter()
        .map(|(at, value)| Keyframe {
            at: seconds(at),
            value,
            ease: EASE,
        })
        .collect()
}

fn transition(kind: TransitionKind) -> Option<Transition> {
    Some(Transition {
        kind,
        duration: seconds(1.0),
        ease: EASE,
        params: Vec::new(),
    })
}

/// Every advanced feature, built through commands so history holds the steps.
fn advanced(root: &Path) -> (Editor, ClipId) {
    let mut project = Project::new("Advanced");
    let asset = Asset::new(root.join("a.mp4"), MediaKind::Video, seconds(60.0));
    let mut clip = Clip::from_asset(&asset, seconds(2.0));
    clip.source_in = seconds(5.0);
    clip.duration = seconds(10.0);
    let title = Clip::text("Title", seconds(2.0), seconds(8.0));
    let (asset_id, clip_id, title_id) = (asset.id, clip.id, title.id);
    Arc::make_mut(&mut project.assets).insert(asset_id, asset);
    Arc::make_mut(&mut project.tracks[0]).clips.push(clip);
    Arc::make_mut(&mut project.tracks[2]).clips.push(title);
    let mut editor = Editor::new(project).unwrap();

    apply(
        &mut editor,
        Command::SetRetime {
            clip_id,
            retime: Some(Retime {
                keys: vec![
                    TimeKey {
                        at: Time::ZERO,
                        source: Time::ZERO,
                        ease: Ease::Linear,
                    },
                    TimeKey {
                        at: seconds(4.0),
                        source: seconds(6.0),
                        ease: EASE,
                    },
                    TimeKey {
                        at: seconds(8.0),
                        source: seconds(8.0),
                        ease: Ease::Linear,
                    },
                ],
            }),
            duration: seconds(8.0),
        },
    );
    let grade = Effect::new(EffectKind::Grade);
    let effects = vec![
        grade.clone(),
        Effect::new(EffectKind::CurveMaster),
        Effect::new(EffectKind::Sharpen),
    ];
    apply(&mut editor, Command::SetEffects { clip_id, effects });
    let mask = Mask::new(MaskShape::Ellipse);
    apply(
        &mut editor,
        Command::SetMasks {
            clip_id,
            masks: vec![mask.clone(), Mask::new(MaskShape::Rectangle)],
        },
    );
    apply(
        &mut editor,
        Command::SetBlendMode {
            clip_id,
            mode: BlendMode::Screen,
        },
    );
    for (target, key, values) in [
        (None, ParamKey::Opacity, [(0.0, 0.25), (8.0, 1.0)]),
        (
            Some(grade.id),
            ParamKey::Exposure,
            [(1.0, -0.5), (6.0, 0.75)],
        ),
        (Some(mask.id), ParamKey::Feather, [(0.5, 0.1), (7.5, 0.6)]),
    ] {
        apply(
            &mut editor,
            Command::SetKeyframes {
                clip_id,
                target,
                key,
                keys: keys(values),
            },
        );
    }
    for (edge, kind) in [
        (TransitionEdge::In, TransitionKind::Fade),
        (TransitionEdge::Out, TransitionKind::WipeLeft),
    ] {
        apply(
            &mut editor,
            Command::SetTransition {
                clip_id,
                edge,
                transition: transition(kind),
            },
        );
    }
    let tracker = Tracker {
        id: Uuid::new_v4(),
        name: "Face".into(),
        asset_id,
        keys: [1.0, 2.5, 4.0]
            .into_iter()
            .map(|at| TrackKey {
                source: seconds(at),
                center: [0.25 + at as f32 / 10.0, 0.4],
                scale: 1.0 + at as f32 / 8.0,
                rotation: at as f32 * 3.0,
            })
            .collect(),
    };
    let tracker_id = tracker.id;
    apply(&mut editor, Command::SetTracker { tracker });
    apply(
        &mut editor,
        Command::SetFollow {
            clip_id: title_id,
            follow: Some(Follow {
                tracker_id,
                position: true,
                scale: true,
                rotation: false,
                offset: [12.5, -40.0],
            }),
        },
    );
    (editor, clip_id)
}

#[test]
fn version_one_project_loads_and_the_next_save_writes_the_current_version() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    write_json(
        &path,
        &json!({"schema_version": 1, "project": v1_project("Version one")}),
    );

    let (project, document) = load_project_with_session(&path).unwrap();

    assert_eq!(project, load_project(&path).unwrap());
    project.validate().unwrap();
    assert_eq!(project.name, "Version one");
    assert!(project.trackers.is_empty());
    let clip = &project.tracks[0].clips[0];
    assert_eq!(clip.playback_rate, 2.0);
    assert!(clip.effects.is_empty() && clip.retime.is_none() && clip.follow.is_none());
    assert_eq!(
        project.assets.values().next().unwrap().path,
        directory.path().canonicalize().unwrap().join("a.mp4")
    );

    save_project_with_session(&document, &project).unwrap();

    assert_eq!(SCHEMA_VERSION, 4);
    assert_eq!(read_json(&path)["schema_version"], json!(SCHEMA_VERSION));
    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn version_one_project_can_be_replaced_by_a_one_shot_save() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    write_json(
        &path,
        &json!({"schema_version": 1, "project": v1_project("Version one")}),
    );
    let project = load_project(&path).unwrap();

    save_project(&path, &project).unwrap();

    assert_eq!(read_json(&path)["schema_version"], json!(SCHEMA_VERSION));
    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn version_one_checkpoint_is_listed_and_loads_beside_a_new_one() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let old = directory
        .path()
        .join("project.aster.checkpoints")
        .join("1-00000000-0000-4000-8000-000000000009.aster");
    write_json(
        &old,
        &json!({
            "schema_version": 1,
            "project": v1_project("Version one"),
            "checkpoint_label": "First assembly"
        }),
    );

    let project = load_project(&old).unwrap();
    project.validate().unwrap();
    let new = checkpoint(&path, &project, "Second assembly").unwrap();
    let checkpoints = list_checkpoints(&path).unwrap();

    assert_eq!(checkpoints.len(), 2);
    assert!(
        checkpoints
            .iter()
            .any(|entry| entry.path == old && entry.label == "First assembly")
    );
    assert_eq!(read_json(&old)["schema_version"], json!(1));
    assert_eq!(read_json(&new)["schema_version"], json!(SCHEMA_VERSION));
    assert_eq!(load_project(&new).unwrap(), project);
}

#[test]
fn version_one_recovery_restores_history_and_the_next_save_writes_the_current_version() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let archive_path = recovery_path(&path);
    write_json(&archive_path, &v1_archive());

    let mut restored = load_recovery(&path).unwrap();

    restored.project().validate().unwrap();
    assert_eq!(restored.project().name, "Renamed");
    assert!(restored.project().trackers.is_empty());
    assert!(restored.is_dirty());
    let state = restored.session_state();
    assert!(Arc::ptr_eq(
        &state.current.trackers,
        &state.past[0].before.trackers
    ));
    assert!(Arc::ptr_eq(
        &state.current.tracks[0],
        &state.past[0].before.tracks[0]
    ));

    save_recovery(&path, &restored).unwrap();

    let archive = read_json(&archive_path);
    assert_eq!(archive["schema_version"], json!(SCHEMA_VERSION));
    assert_eq!(archive["trackers"], json!([{}]));
    let mut reloaded = load_recovery(&path).unwrap();
    assert_eq!(reloaded.project(), restored.project());
    assert!(reloaded.undo() && restored.undo());
    assert_eq!(reloaded.project().name, "Version one");
    assert!(!reloaded.is_dirty());
}

/// The clip that carries the effects, masks, keys, retime and transitions.
fn v2_clip(project: &Project) -> &Clip {
    &project.tracks[0].clips[0]
}

#[test]
fn version_two_project_loads_and_the_next_save_writes_the_current_version() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    write_text(&path, V2_PROJECT);
    let fixture = read_json(&path);
    assert_eq!(fixture["schema_version"], json!(2));

    let (project, document) = load_project_with_session(&path).unwrap();

    assert_eq!(project, load_project(&path).unwrap());
    project.validate().unwrap();
    assert_eq!(project.name, "Advanced");
    let clip = v2_clip(&project);
    assert_eq!(
        (clip.effects.len(), clip.masks.len(), clip.animation.len()),
        (3, 2, 3)
    );
    assert_eq!(clip.blend, BlendMode::Screen);
    assert!(clip.retime.is_some());
    assert!(clip.transition_in.is_some() && clip.transition_out.is_some());
    assert_eq!(project.trackers.len(), 1);
    assert!(project.tracks[2].clips[0].follow.is_some());
    // What versions 3 and 4 add reads as its default.
    for track in &project.tracks {
        assert_eq!((track.gain, track.solo), (1.0, false));
        for clip in &track.clips {
            assert_eq!((clip.fade_in, clip.fade_out), (Time::ZERO, Time::ZERO));
        }
    }
    let media = directory.path().canonicalize().unwrap().join("a.mp4");
    assert_eq!(project.assets.values().next().unwrap().path, media);

    save_project_with_session(&document, &project).unwrap();

    // Nothing is stripped and nothing is added: but for the version and the
    // resolved media path the file is the one version 2 wrote.
    let mut written = read_json(&path);
    assert_eq!(written["schema_version"], json!(SCHEMA_VERSION));
    for asset in written["project"]["assets"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        assert_eq!(asset["path"], json!(media.to_str().unwrap()));
        asset["path"] = json!("a.mp4");
    }
    assert_eq!(written["project"], fixture["project"]);
    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn version_two_project_can_be_replaced_by_a_one_shot_save() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    write_text(&path, V2_PROJECT);
    let project = load_project(&path).unwrap();

    save_project(&path, &project).unwrap();

    assert_eq!(read_json(&path)["schema_version"], json!(SCHEMA_VERSION));
    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn version_two_checkpoint_is_listed_and_loads_beside_a_new_one() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let old = directory
        .path()
        .join("project.aster.checkpoints")
        .join("1-00000000-0000-4000-8000-000000000009.aster");
    write_text(&old, V2_CHECKPOINT);

    let project = load_project(&old).unwrap();
    project.validate().unwrap();
    assert_eq!(v2_clip(&project).effects.len(), 3);
    let new = checkpoint(&path, &project, "Second assembly").unwrap();
    let checkpoints = list_checkpoints(&path).unwrap();

    assert_eq!(checkpoints.len(), 2);
    assert!(
        checkpoints
            .iter()
            .any(|entry| entry.path == old && entry.label == "First assembly")
    );
    assert_eq!(fs::read_to_string(&old).unwrap(), V2_CHECKPOINT);
    assert_eq!(read_json(&new)["schema_version"], json!(SCHEMA_VERSION));
    assert_eq!(load_project(&new).unwrap(), project);
}

#[test]
fn version_two_recovery_restores_history_and_the_next_save_writes_the_current_version() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let archive_path = recovery_path(&path);
    write_text(&path, V2_PROJECT);
    write_text(&archive_path, V2_ARCHIVE);
    assert_eq!(read_json(&archive_path)["schema_version"], json!(2));

    let mut restored = load_recovery(&path).unwrap();

    restored.project().validate().unwrap();
    assert!(restored.is_dirty());
    let state = restored.session_state();
    assert_eq!(state.future.len(), 2);
    assert!(restored.project().trackers.is_empty());
    assert_eq!(v2_clip(restored.project()).effects.len(), 3);

    save_recovery(&path, &restored).unwrap();

    assert_eq!(
        read_json(&archive_path)["schema_version"],
        json!(SCHEMA_VERSION)
    );
    let mut reloaded = load_recovery(&path).unwrap();
    let again = reloaded.session_state();
    assert_eq!(again.current, state.current);
    assert_eq!(
        (again.past.len(), again.future.len()),
        (state.past.len(), 2)
    );
    // The two steps undone before the crash lead to the document saved
    // beside the archive, in both builds' archives.
    let saved = load_project(&path).unwrap();
    assert!(restored.redo() && restored.redo());
    assert!(reloaded.redo() && reloaded.redo());
    assert_eq!(restored.project(), &saved);
    assert_eq!(reloaded.project(), &saved);
}

/// A version 5 document holding what this build cannot parse: the refusal has
/// to come from the version, before the body is read as a project.
fn v5_project() -> Value {
    let mut future: Value = serde_json::from_str(V2_PROJECT).unwrap();
    future["schema_version"] = json!(5);
    future["project"]["tracks"][0]["clips"][0]["effects"][0]["kind"] = json!("from_the_future");
    future["project"]["from_the_future"] = json!({"valuable": true});
    future
}

#[test]
fn version_five_is_refused_and_protected_everywhere() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let future = v5_project();
    let mut as_current = future.clone();
    as_current["schema_version"] = json!(SCHEMA_VERSION);
    write_json(&path, &as_current);
    assert!(matches!(load_project(&path), Err(StoreError::Json(_))));

    write_json(&path, &future);
    let original = fs::read(&path).unwrap();

    assert!(matches!(
        load_project(&path),
        Err(StoreError::FutureSchema(5))
    ));
    assert!(matches!(
        load_project_with_session(&path),
        Err(StoreError::FutureSchema(5))
    ));
    assert!(matches!(
        save_project(&path, &Project::default()),
        Err(StoreError::FutureSchema(5))
    ));
    assert_eq!(fs::read(&path).unwrap(), original);

    let archive = json!({"schema_version": 5, "from_the_future": {"valuable": true}});
    let archive_path = recovery_path(&path);
    write_json(&archive_path, &archive);
    let original = fs::read(&archive_path).unwrap();
    assert!(matches!(
        load_recovery(&path),
        Err(StoreError::FutureSchema(5))
    ));
    assert!(matches!(
        load_recovery_with_document(&path),
        Err(StoreError::FutureSchema(5))
    ));
    assert!(matches!(
        save_recovery(&path, &Editor::default()),
        Err(StoreError::FutureSchema(5))
    ));
    assert_eq!(fs::read(&archive_path).unwrap(), original);

    let checkpoint = directory
        .path()
        .join("project.aster.checkpoints")
        .join("1-00000000-0000-4000-8000-000000000009.aster");
    write_json(&checkpoint, &future);
    let original = fs::read(&checkpoint).unwrap();
    assert!(matches!(
        list_checkpoints(&path),
        Err(StoreError::FutureSchema(5))
    ));
    assert!(matches!(
        load_project(&checkpoint),
        Err(StoreError::FutureSchema(5))
    ));
    assert!(matches!(
        save_project(&checkpoint, &Project::default()),
        Err(StoreError::FutureSchema(5))
    ));
    assert_eq!(fs::read(&checkpoint).unwrap(), original);
}

#[test]
fn version_zero_stays_unsupported_and_preserved() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let zero = json!({"schema_version": 0, "project": v1_project("Before one")});
    write_json(&path, &zero);
    let original = fs::read(&path).unwrap();

    assert!(matches!(load_project(&path), Err(StoreError::Invalid(_))));
    assert!(save_project(&path, &Project::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);

    let mut archive = v1_archive();
    archive["schema_version"] = json!(0);
    write_json(&recovery_path(&path), &archive);
    assert!(matches!(load_recovery(&path), Err(StoreError::Invalid(_))));

    let checkpoint = directory
        .path()
        .join("project.aster.checkpoints")
        .join("1-00000000-0000-4000-8000-000000000009.aster");
    write_json(&checkpoint, &zero);
    let original = fs::read(&checkpoint).unwrap();
    assert!(matches!(
        list_checkpoints(&path),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(fs::read(&checkpoint).unwrap(), original);
}

#[test]
fn every_advanced_feature_round_trips_through_a_project_file() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let (editor, clip_id) = advanced(directory.path());
    let project = editor.project();
    let clip = project.clip(clip_id).unwrap().1;
    assert_eq!(clip.effects.len(), 3);
    assert_eq!(clip.masks.len(), 2);
    assert_eq!(clip.animation.len(), 3);
    assert_eq!(clip.blend, BlendMode::Screen);
    assert!(clip.retime.is_some());
    assert!(clip.transition_in.is_some() && clip.transition_out.is_some());
    assert_eq!(project.trackers.len(), 1);
    assert!(project.tracks[2].clips[0].follow.is_some());

    save_project(&path, project).unwrap();
    let restored = load_project(&path).unwrap();

    assert_eq!(read_json(&path)["schema_version"], json!(SCHEMA_VERSION));
    assert_eq!(&restored, project);
    save_project(&path, &restored).unwrap();
    assert_eq!(&load_project(&path).unwrap(), project);
}

#[test]
fn a_lut_asset_and_the_effects_that_read_assets_round_trip_through_a_project_file() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let (mut editor, clip_id) = advanced(directory.path());
    // A table as import describes it: no time, no picture, its points along
    // an edge for a width.
    let mut lut = Asset::new(directory.path().join("look.cube"), MediaKind::Lut, Time(1));
    lut.width = 33;
    lut.file_size = 1_234_567;
    let plate = Asset::new(
        directory.path().join("plate.png"),
        MediaKind::Image,
        seconds(5.0),
    );
    let (lut_id, plate_id) = (lut.id, plate.id);
    apply(
        &mut editor,
        Command::AddAssets {
            assets: vec![lut.clone(), plate],
        },
    );
    let mut effects = editor.project().clip(clip_id).unwrap().1.effects.clone();
    let mut lookup = Effect::new(EffectKind::Lut);
    lookup.asset_id = Some(lut_id);
    lookup.mix = 0.625;
    let mut matte = Effect::new(EffectKind::DifferenceMatte);
    matte.asset_id = Some(plate_id);
    effects.extend([lookup.clone(), matte]);
    apply(&mut editor, Command::SetEffects { clip_id, effects });
    let project = editor.project();

    save_project(&path, project).unwrap();
    let restored = load_project(&path).unwrap();

    let written = read_json(&path);
    assert_eq!(written["schema_version"], json!(SCHEMA_VERSION));
    let asset = &written["project"]["assets"][lut_id.to_string()];
    assert_eq!(asset["kind"], json!("lut"));
    assert_eq!(asset["duration"], json!(1));
    assert_eq!((&asset["width"], &asset["height"]), (&json!(33), &json!(0)));
    assert_eq!(&restored, project);
    assert_eq!(restored.assets[&lut_id], lut);
    let clip = restored.clip(clip_id).unwrap().1;
    assert_eq!(clip.effects.len(), 5);
    assert_eq!(clip.effects[3], lookup);
    // And through recovery, with the history that added them.
    let recovery = recovery_path(&path);
    save_recovery(&recovery, &editor).unwrap();
    let recovered = load_recovery(&recovery).unwrap();
    assert_eq!(recovered.project(), project);

    // A table is still never a clip, whatever a file says.
    let mut tampered = written.clone();
    let clips = &mut tampered["project"]["tracks"][0]["clips"];
    clips[0]["content"]["asset_id"] = json!(lut_id.to_string());
    write_json(&path, &tampered);
    let refused = load_project(&path).unwrap_err().to_string();
    assert!(
        refused.contains("incompatible with this track"),
        "{refused}"
    );
}

#[test]
fn every_advanced_feature_round_trips_through_recovery_with_history() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let (mut editor, _) = advanced(directory.path());
    assert!(editor.undo());
    assert!(editor.undo());
    let before = editor.session_state();
    save_recovery(&path, &editor).unwrap();

    let mut restored = load_recovery(&path).unwrap();
    let after = restored.session_state();

    assert_eq!(after.revision, before.revision);
    assert_eq!(after.current, before.current);
    assert_eq!(after.past.len(), before.past.len());
    assert_eq!(after.future.len(), 2);
    for (restored, original) in after
        .past
        .iter()
        .zip(&before.past)
        .chain(after.future.iter().zip(&before.future))
    {
        assert_eq!(restored.label, original.label);
        assert_eq!(restored.before, original.before);
        assert_eq!(restored.after, original.after);
    }
    // One tracker map is stored per distinct version, not per history root.
    assert!(after.current.trackers.is_empty());
    let followed = &after
        .future
        .iter()
        .find(|h| h.label == "Change tracking follow")
        .unwrap();
    assert!(Arc::ptr_eq(
        &followed.before.trackers,
        &followed.after.trackers
    ));
    assert_eq!(
        read_json(&recovery_path(&path))["trackers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    assert!(restored.redo() && restored.redo());
    assert!(editor.redo() && editor.redo());
    assert_eq!(restored.project(), editor.project());
    assert_eq!(restored.project().trackers.len(), 1);
    restored.project().validate().unwrap();
}

#[test]
fn recovery_stores_each_tracker_once_however_many_maps_share_it() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let (mut editor, _) = advanced(directory.path());
    let asset_id = *editor.project().assets.keys().next().unwrap();
    let tracker = |name: &str| Tracker {
        id: Uuid::new_v4(),
        name: name.into(),
        asset_id,
        keys: vec![TrackKey {
            source: seconds(1.0),
            center: [0.5, 0.5],
            scale: 1.0,
            rotation: 0.0,
        }],
    };
    let mut hand = tracker("Hand");
    apply(
        &mut editor,
        Command::SetTracker {
            tracker: hand.clone(),
        },
    );
    apply(
        &mut editor,
        Command::SetTracker {
            tracker: tracker("Ball"),
        },
    );
    for step in 0..20 {
        hand.keys[0].center[0] = 0.1 + step as f32 / 100.0;
        apply(
            &mut editor,
            Command::SetTracker {
                tracker: hand.clone(),
            },
        );
    }
    save_recovery(&path, &editor).unwrap();

    // Face, Ball and 21 versions of Hand; 24 maps hold them (one is empty).
    let archive = read_json(&recovery_path(&path));
    assert_eq!(archive["tracker_values"].as_array().unwrap().len(), 23);
    assert_eq!(archive["trackers"].as_array().unwrap().len(), 24);

    let restored = load_recovery(&path).unwrap().session_state();
    let original = editor.session_state();
    assert_eq!(restored.current, original.current);
    let (first, last) = (
        &restored.past[restored.past.len() - 20].after.trackers,
        &restored.current.trackers,
    );
    assert_eq!(first.len(), 3);
    let shared = first
        .iter()
        .filter(|(id, tracker)| Arc::ptr_eq(tracker, &last[*id]))
        .count();
    assert_eq!(shared, 2, "untouched trackers stay one allocation");
}

/// Fades on a picture clip with sound and on a music clip, a quieter music
/// track and a soloed picture track, each its own step in history.
fn sound(root: &Path) -> Editor {
    let mut project = Project::new("Sound");
    let mut video = Asset::new(root.join("a.mp4"), MediaKind::Video, seconds(60.0));
    video.has_audio = true;
    let music = Asset::new(root.join("bed.wav"), MediaKind::Audio, seconds(90.0));
    let mut picture = Clip::from_asset(&video, seconds(1.0));
    picture.duration = seconds(10.0);
    let mut bed = Clip::from_asset(&music, Time::ZERO);
    bed.duration = seconds(20.0);
    let title = Clip::text("Title", seconds(2.0), seconds(8.0));
    let (picture_id, bed_id) = (picture.id, bed.id);
    let assets = Arc::make_mut(&mut project.assets);
    assets.insert(video.id, video);
    assets.insert(music.id, music);
    Arc::make_mut(&mut project.tracks[0]).clips.push(picture);
    Arc::make_mut(&mut project.tracks[1]).clips.push(bed);
    Arc::make_mut(&mut project.tracks[2]).clips.push(title);
    let (picture_track, music_track) = (project.tracks[0].id, project.tracks[1].id);
    let mut editor = Editor::new(project).unwrap();

    apply(
        &mut editor,
        Command::SetFades {
            clip_id: picture_id,
            fade_in: seconds(0.5),
            fade_out: Time::ZERO,
        },
    );
    apply(
        &mut editor,
        Command::SetFades {
            clip_id: bed_id,
            fade_in: seconds(1.25),
            fade_out: seconds(2.0),
        },
    );
    let state = |track_id, gain, solo| Command::SetTrackState {
        track_id,
        locked: None,
        hidden: None,
        muted: None,
        name: None,
        gain,
        solo,
    };
    apply(&mut editor, state(music_track, Some(0.35), None));
    apply(&mut editor, state(picture_track, None, Some(true)));
    editor
}

fn assert_sound(project: &Project) {
    let [picture, music, text] = &project.tracks[..] else {
        panic!("three tracks");
    };
    assert_eq!((picture.gain, picture.solo), (1.0, true));
    assert_eq!((music.gain, music.solo), (0.35, false));
    assert_eq!((text.gain, text.solo), (1.0, false));
    let fades = |clip: &Clip| (clip.fade_in, clip.fade_out);
    assert_eq!(fades(&picture.clips[0]), (seconds(0.5), Time::ZERO));
    assert_eq!(fades(&music.clips[0]), (seconds(1.25), seconds(2.0)));
    assert_eq!(fades(&text.clips[0]), (Time::ZERO, Time::ZERO));
}

#[test]
fn fades_track_volume_and_solo_round_trip_through_a_project_file() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let editor = sound(directory.path());
    let project = editor.project();
    assert_sound(project);

    save_project(&path, project).unwrap();
    let (restored, document) = load_project_with_session(&path).unwrap();

    // A field at its default writes no key; one that was set writes its value.
    let written = read_json(&path);
    assert_eq!(written["schema_version"], json!(SCHEMA_VERSION));
    let tracks = written["project"]["tracks"].as_array().unwrap();
    let keys = |value: &Value, names: [&str; 2]| names.map(|name| value.get(name).cloned());
    assert_eq!(
        keys(&tracks[0], ["gain", "solo"]),
        [None, Some(json!(true))]
    );
    assert_eq!(keys(&tracks[1], ["gain", "solo"])[1], None);
    assert_eq!(tracks[1]["gain"].as_f64().unwrap() as f32, 0.35);
    assert_eq!(keys(&tracks[2], ["gain", "solo"]), [None, None]);
    let fades = |track: usize| keys(&tracks[track]["clips"][0], ["fade_in", "fade_out"]);
    assert_eq!(fades(0), [Some(json!(500_000)), None]);
    assert_eq!(fades(1), [Some(json!(1_250_000)), Some(json!(2_000_000))]);
    assert_eq!(fades(2), [None, None]);

    assert_eq!(&restored, project);
    assert_sound(&restored);
    save_project_with_session(&document, &restored).unwrap();
    assert_eq!(read_json(&path), written);
    assert_eq!(&load_project(&path).unwrap(), project);
}

#[test]
fn fades_track_volume_and_solo_round_trip_through_a_checkpoint() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let editor = sound(directory.path());

    let kept = checkpoint(&path, editor.project(), "With sound").unwrap();

    let checkpoints = list_checkpoints(&path).unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(
        (&checkpoints[0].path, checkpoints[0].label.as_str()),
        (&kept, "With sound")
    );
    assert_eq!(read_json(&kept)["schema_version"], json!(SCHEMA_VERSION));
    let restored = load_project(&kept).unwrap();
    assert_eq!(&restored, editor.project());
    assert_sound(&restored);
}

#[test]
fn fades_track_volume_and_solo_round_trip_through_recovery_with_history() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut editor = sound(directory.path());
    // Solo and the music track's volume are undone when the archive is written.
    assert!(editor.undo() && editor.undo());
    let before = editor.session_state();
    save_recovery(&path, &editor).unwrap();

    let mut restored = load_recovery(&path).unwrap();
    let after = restored.session_state();

    assert_eq!(
        read_json(&recovery_path(&path))["schema_version"],
        json!(SCHEMA_VERSION)
    );
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.current, before.current);
    assert_eq!((after.past.len(), after.future.len()), (2, 2));
    for (restored, original) in after
        .past
        .iter()
        .zip(&before.past)
        .chain(after.future.iter().zip(&before.future))
    {
        assert_eq!(restored.label, original.label);
        assert_eq!(restored.before, original.before);
        assert_eq!(restored.after, original.after);
    }
    let current = &after.current;
    assert_eq!(
        (current.tracks[1].gain, current.tracks[0].solo),
        (1.0, false)
    );
    assert_eq!(current.tracks[1].clips[0].fade_out, seconds(2.0));
    // A track a step did not touch is still one allocation across it.
    let soloed = after
        .future
        .iter()
        .find(|h| h.after.tracks[0].solo)
        .unwrap();
    assert!(Arc::ptr_eq(
        &soloed.before.tracks[1],
        &soloed.after.tracks[1]
    ));

    assert!(restored.redo() && restored.redo());
    assert!(editor.redo() && editor.redo());
    assert_eq!(restored.project(), editor.project());
    assert_sound(restored.project());
    while restored.undo() {}
    for track in &restored.project().tracks {
        assert_eq!((track.gain, track.solo), (1.0, false));
        assert!(track.clips.iter().all(|clip| clip.fade_in.is_zero()));
    }
}

#[test]
fn a_file_whose_fades_or_track_volume_are_out_of_bounds_is_refused_and_preserved() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let editor = sound(directory.path());
    save_project(&path, editor.project()).unwrap();
    let written = read_json(&path);
    let refused = |edit: &dyn Fn(&mut Value), reason: &str| {
        let mut tampered = written.clone();
        edit(&mut tampered["project"]["tracks"]);
        write_json(&path, &tampered);
        let original = fs::read(&path).unwrap();
        for error in [
            load_project(&path).unwrap_err(),
            load_project_with_session(&path).map(|_| ()).unwrap_err(),
            save_project(&path, editor.project()).unwrap_err(),
        ] {
            assert!(matches!(error, StoreError::Edit(_)), "{error}");
            assert!(error.to_string().contains(reason), "{error}");
        }
        assert_eq!(fs::read(&path).unwrap(), original);
    };

    // Longer together than the clip, negative, and on a clip with no sound.
    refused(
        &|tracks| tracks[1]["clips"][0]["fade_in"] = json!(18_000_001),
        "fades",
    );
    refused(
        &|tracks| tracks[0]["clips"][0]["fade_out"] = json!(-1),
        "fades",
    );
    refused(
        &|tracks| tracks[2]["clips"][0]["fade_in"] = json!(1),
        "fades",
    );
    refused(&|tracks| tracks[1]["gain"] = json!(8.5), "track volume");
    refused(&|tracks| tracks[2]["gain"] = json!(-0.1), "track volume");

    // The same bound holds for every track state a recovery archive carries.
    assert!(editor.can_undo());
    save_recovery(&path, &editor).unwrap();
    let archive_path = recovery_path(&path);
    let mut archive = read_json(&archive_path);
    for track in archive["tracks"].as_array_mut().unwrap() {
        track["gain"] = json!(9.0);
    }
    write_json(&archive_path, &archive);
    let original = fs::read(&archive_path).unwrap();
    let error = load_recovery(&path).map(|_| ()).unwrap_err();
    assert!(error.to_string().contains("track volume"), "{error}");
    assert_eq!(fs::read(&archive_path).unwrap(), original);
}

#[test]
fn a_project_with_fades_out_of_bounds_is_not_written_anywhere() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let editor = sound(directory.path());
    save_project(&path, editor.project()).unwrap();
    let original = fs::read(&path).unwrap();
    let (_, document) = load_project_with_session(&path).unwrap();
    let mut broken = editor.project().clone();
    Arc::make_mut(&mut broken.tracks[1]).clips[0].fade_in = seconds(19.0);

    for error in [
        save_project(&path, &broken).unwrap_err(),
        save_project_with_session(&document, &broken).unwrap_err(),
        checkpoint(&path, &broken, "Broken")
            .map(|_| ())
            .unwrap_err(),
    ] {
        assert!(matches!(error, StoreError::Edit(_)), "{error}");
    }

    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(list_checkpoints(&path).unwrap().is_empty());
}

/// One of each audio kind on the music clip of [`sound`], the last switched
/// off, as one more step in history.
fn audio_effects(root: &Path) -> (Editor, Vec<Effect>) {
    let mut editor = sound(root);
    let mut stack: Vec<Effect> = (EffectKind::ALL.into_iter())
        .filter(|kind| kind.is_audio())
        .map(Effect::new)
        .collect();
    assert_eq!(stack.len(), 7);
    stack[1].params = vec![
        editor_core::ParamValue {
            key: ParamKey::Frequency,
            value: 3_500.0,
        },
        editor_core::ParamValue {
            key: ParamKey::Gain,
            value: -6.0,
        },
    ];
    stack[6].enabled = false;
    let clip_id = editor.project().tracks[1].clips[0].id;
    apply(
        &mut editor,
        Command::SetEffects {
            clip_id,
            effects: stack.clone(),
        },
    );
    (editor, stack)
}

#[test]
fn one_of_each_audio_kind_round_trips_through_a_project_file_and_a_checkpoint() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let (editor, stack) = audio_effects(directory.path());
    let project = editor.project();

    save_project(&path, project).unwrap();
    let (restored, document) = load_project_with_session(&path).unwrap();

    let written = read_json(&path);
    assert_eq!(written["schema_version"], json!(SCHEMA_VERSION));
    let kinds: Vec<&str> = written["project"]["tracks"][1]["clips"][0]["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|effect| effect["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "high_pass",
            "eq_band",
            "compressor",
            "limiter",
            "denoise",
            "pitch",
            "center_cut"
        ]
    );
    assert_eq!(&restored, project);
    assert_eq!(restored.tracks[1].clips[0].effects, stack);
    assert_sound(&restored);
    save_project_with_session(&document, &restored).unwrap();
    assert_eq!(read_json(&path), written);

    let kept = checkpoint(&path, project, "With audio effects").unwrap();
    assert_eq!(read_json(&kept)["schema_version"], json!(SCHEMA_VERSION));
    let restored = load_project(&kept).unwrap();
    assert_eq!(&restored, project);
    assert_eq!(restored.tracks[1].clips[0].effects, stack);
}

#[test]
fn audio_effects_round_trip_through_recovery_with_history() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let (mut editor, stack) = audio_effects(directory.path());
    // The effects are undone when the archive is written.
    assert!(editor.undo());
    let before = editor.session_state();
    save_recovery(&path, &editor).unwrap();

    let mut restored = load_recovery(&path).unwrap();
    let after = restored.session_state();

    assert_eq!(
        read_json(&recovery_path(&path))["schema_version"],
        json!(SCHEMA_VERSION)
    );
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.current, before.current);
    assert_eq!((after.past.len(), after.future.len()), (4, 1));
    assert_eq!(after.future[0].label, "Change effects");
    assert_eq!(after.future[0].after, before.future[0].after);
    assert!(after.current.tracks[1].clips[0].effects.is_empty());

    assert!(restored.redo() && editor.redo());
    assert_eq!(restored.project(), editor.project());
    assert_eq!(restored.project().tracks[1].clips[0].effects, stack);
    assert!(restored.undo());
    assert!(restored.project().tracks[1].clips[0].effects.is_empty());
}

#[test]
fn a_file_whose_audio_effects_break_a_rule_is_refused_and_preserved() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let (editor, _) = audio_effects(directory.path());
    save_project(&path, editor.project()).unwrap();
    let written = read_json(&path);
    let effects = written["project"]["tracks"][1]["clips"][0]["effects"].clone();
    let refused = |edit: &dyn Fn(&mut Value), reason: &str| {
        let mut tampered = written.clone();
        edit(&mut tampered["project"]["tracks"]);
        write_json(&path, &tampered);
        let original = fs::read(&path).unwrap();
        for error in [
            load_project(&path).unwrap_err(),
            load_project_with_session(&path).map(|_| ()).unwrap_err(),
            save_project(&path, editor.project()).unwrap_err(),
        ] {
            // Compare the reason without its clip context: "effect" is also
            // inside two other refusals.
            match error {
                StoreError::Edit(EditError::Invalid(found)) => {
                    assert_eq!(found.split(" (clip ").next().unwrap(), reason)
                }
                other => panic!("expected Invalid({reason}), got {other}"),
            }
        }
        assert_eq!(fs::read(&path).unwrap(), original);
    };

    // On a picture clip, part mixed, out of range, and a picture kind on a sound.
    refused(
        &|tracks| tracks[0]["clips"][0]["effects"] = json!([effects[0]]),
        "audio effect on a picture clip",
    );
    refused(
        &|tracks| tracks[1]["clips"][0]["effects"][2]["mix"] = json!(0.5),
        "effect",
    );
    refused(
        &|tracks| {
            tracks[1]["clips"][0]["effects"][0]["params"] =
                json!([{ "key": "frequency", "value": 19.0 }]);
        },
        "effect",
    );
    refused(
        &|tracks| tracks[1]["clips"][0]["effects"][3]["kind"] = json!("glow"),
        "audio clip look",
    );

    // A project that breaks the rule is written nowhere.
    write_json(&path, &written);
    let original = fs::read(&path).unwrap();
    let mut broken = editor.project().clone();
    Arc::make_mut(&mut broken.tracks[1]).clips[0].effects[0].mix = 0.5;
    for error in [
        save_project(&path, &broken).unwrap_err(),
        checkpoint(&path, &broken, "Broken")
            .map(|_| ())
            .unwrap_err(),
    ] {
        assert!(matches!(error, StoreError::Edit(_)), "{error}");
    }
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(list_checkpoints(&path).unwrap().is_empty());
}

#[test]
fn version_two_title_loads_plain_and_the_next_save_writes_the_current_version() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut project = v1_project("Version two");
    let mut text = v1_track();
    text["id"] = json!("00000000-0000-4000-8000-000000000005");
    text["name"] = json!("Text 1");
    text["kind"] = json!("text");
    text["clips"][0]["id"] = json!("00000000-0000-4000-8000-000000000006");
    text["clips"][0]["name"] = json!("Hello");
    text["clips"][0]["content"] = json!({
        "type": "text",
        "text": "Hello",
        "font_size": 64.0,
        "color": [255, 255, 255, 255]
    });
    text["clips"][0]["source_in"] = json!(0);
    text["clips"][0]["playback_rate"] = json!(1.0);
    project["tracks"].as_array_mut().unwrap().push(text);
    write_json(&path, &json!({"schema_version": 2, "project": project}));

    let project = load_project(&path).unwrap();
    project.validate().unwrap();
    let track = &project.tracks[1];
    assert!(track.language.is_none());
    let editor_core::ClipContent::Text {
        style,
        enter,
        exit,
        words,
        ..
    } = &track.clips[0].content
    else {
        panic!("not a title");
    };
    assert!(style.is_plain() && enter.is_none() && exit.is_none() && words.is_empty());

    save_project(&path, &project).unwrap();
    let written = read_json(&path);
    assert_eq!(written["schema_version"], json!(SCHEMA_VERSION));
    // A plain title is written as version 2 wrote it.
    assert_eq!(
        written["project"]["tracks"][1]["clips"][0]["content"],
        json!({"type": "text", "text": "Hello", "font_size": 64.0, "color": [255, 255, 255, 255]})
    );
    assert!(written["project"]["tracks"][1].get("language").is_none());
    assert_eq!(load_project(&path).unwrap(), project);
}

#[test]
fn titles_graphics_captions_and_fonts_round_trip_through_a_project_file() {
    use editor_core::{
        Element, ElementKind, Highlight, HighlightMode, TextAnimation, TextAnimationKind,
        TextOutline, TextStyle, TextUnit, TrackKind, WordTime,
    };
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("project.aster");
    let mut editor = Editor::default();
    let track = editor
        .project()
        .tracks
        .iter()
        .find(|track| track.kind == TrackKind::Text)
        .unwrap()
        .id;
    let font = Asset::new(directory.path().join("Brand.ttf"), MediaKind::Font, Time(1));
    let cue = Clip::text("hello there", Time::ZERO, seconds(2.0));
    let cue_id = cue.id;
    let graphic = Clip::graphic(
        "Lower third",
        [800.0, 200.0],
        vec![
            Element::new(
                "Bar",
                ElementKind::Rect {
                    color: [230, 60, 40, 255],
                    radius: 6.0,
                    stroke: None,
                },
                [0.0, 0.0, 800.0, 200.0],
            ),
            Element::new(
                "Name",
                ElementKind::Text {
                    text: "Ada Lovelace".into(),
                    font_size: 72.0,
                    color: [255; 4],
                    style: TextStyle::default(),
                },
                [24.0, 24.0, 752.0, 152.0],
            ),
        ],
        seconds(3.0),
        seconds(4.0),
    );
    let style = TextStyle {
        outline: Some(TextOutline {
            color: [0, 0, 0, 255],
            width: 0.1,
        }),
        highlight: Some(Highlight {
            mode: HighlightMode::Fill,
            color: [255, 214, 0, 255],
            scale: 1.15,
            upcoming: 0.6,
        }),
        ..TextStyle::default()
    };
    apply(
        &mut editor,
        Command::Batch {
            label: "Captions".into(),
            commands: vec![
                Command::AddAssets { assets: vec![font] },
                Command::AddClip {
                    track_id: track,
                    clip: cue,
                },
                Command::AddClip {
                    track_id: track,
                    clip: graphic,
                },
                Command::SetTrackLanguage {
                    track_id: track,
                    language: Some("en-GB".into()),
                },
                Command::SetTextStyle {
                    clip_id: cue_id,
                    style,
                },
                Command::SetTextAnimation {
                    clip_id: cue_id,
                    edge: TransitionEdge::In,
                    animation: Some(TextAnimation {
                        kind: TextAnimationKind::Typewriter,
                        unit: TextUnit::Letter,
                        duration: seconds(0.5),
                        delay: seconds(0.1),
                    }),
                },
                Command::SetWords {
                    clip_id: cue_id,
                    words: vec![
                        WordTime {
                            at: seconds(0.1),
                            end: seconds(0.6),
                        },
                        WordTime {
                            at: seconds(0.7),
                            end: seconds(1.4),
                        },
                    ],
                },
            ],
        },
    );
    let project = editor.project();
    save_project(&path, project).unwrap();
    assert_eq!(read_json(&path)["schema_version"], json!(SCHEMA_VERSION));
    assert_eq!(&load_project(&path).unwrap(), project);
}
