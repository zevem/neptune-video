use editor_core::*;
use std::time::{Duration, Instant};

#[test]
fn restoring_a_checkpoint_is_one_reversible_same_project_transaction() {
    let mut editor = Editor::new(Project::new("Before")).unwrap();
    let checkpoint = editor.project().clone();
    editor
        .dispatch(Command::RenameProject {
            name: "Later".into(),
        })
        .unwrap();
    let current = editor.snapshot();
    editor
        .dispatch(Command::RestoreProject {
            project: Box::new(checkpoint),
        })
        .unwrap();
    assert_eq!(editor.project().name, "Before");
    assert_eq!(editor.undo_label(), Some("Restore checkpoint"));
    assert!(editor.undo());
    assert_eq!(editor.project(), current.as_ref());
    assert!(editor.redo());
    assert_eq!(editor.project().name, "Before");
}
#[test]
fn a_checkpoint_cannot_change_project_identity() {
    let mut editor = Editor::default();
    let before = editor.snapshot();
    assert!(
        editor
            .dispatch(Command::RestoreProject {
                project: Box::new(Project::new("Different"))
            })
            .is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&before, &editor.snapshot()));
}
#[test]
fn playback_follows_device_time_and_falls_back_without_drift() {
    let mut playback = Playback::default();
    let now = Instant::now();
    playback.play();
    playback.tick(now, Time::from_seconds(10.0));
    playback.tick_with_audio(
        now + Duration::from_secs(2),
        Time::from_seconds(10.0),
        Some(Time::from_seconds(1.5)),
    );
    assert_eq!(playback.position, Time::from_seconds(1.5));
    playback.tick_with_audio(now + Duration::from_secs(3), Time::from_seconds(10.0), None);
    assert_eq!(playback.position, Time::from_seconds(2.5));
    playback.tick_with_audio(
        now + Duration::from_secs(4),
        Time::from_seconds(10.0),
        Some(Time::from_seconds(10.0)),
    );
    assert!(!playback.playing);
    assert_eq!(playback.position, Time::from_seconds(10.0));
}

#[test]
fn delayed_audio_handoff_keeps_the_playhead_monotonic_until_audio_catches_up() {
    let mut playback = Playback::default();
    let now = Instant::now();
    let duration = Time::from_seconds(10.0);
    playback.play();
    playback.tick_with_audio(now, duration, None);
    playback.tick_with_audio(now + Duration::from_millis(200), duration, None);
    assert_eq!(playback.position, Time::from_seconds(0.2));

    // The first decoded buffer is ready after the fallback clock advanced.
    assert!(!playback.tick_with_audio(
        now + Duration::from_millis(200),
        duration,
        Some(Time::ZERO),
    ));
    assert_eq!(playback.position, Time::from_seconds(0.2));
    assert!(!playback.tick_with_audio(
        now + Duration::from_millis(250),
        duration,
        Some(Time::from_seconds(0.05)),
    ));
    assert_eq!(playback.position, Time::from_seconds(0.2));

    assert!(playback.tick_with_audio(
        now + Duration::from_millis(500),
        duration,
        Some(Time::from_seconds(0.3)),
    ));
    assert_eq!(playback.position, Time::from_seconds(0.3));
    playback.tick_with_audio(now + Duration::from_millis(600), duration, None);
    assert_eq!(playback.position, Time::from_seconds(0.4));

    // A deliberate seek still starts a new interval at its chosen position.
    playback.seek(Time::from_seconds(0.1));
    playback.tick_with_audio(
        now + Duration::from_millis(700),
        duration,
        Some(Time::from_seconds(0.1)),
    );
    assert_eq!(playback.position, Time::from_seconds(0.1));
    assert!(playback.playing);
}

#[test]
fn source_preview_does_not_insert_into_the_editing_project() {
    let editor = Editor::default();
    let mut asset = Asset::new(
        std::path::PathBuf::from("/video.mov"),
        MediaKind::Video,
        Time::from_seconds(12.0),
    );
    asset.width = 1080;
    asset.height = 1920;
    asset.frame_rate = Some(FrameRate {
        numerator: 30000,
        denominator: 1001,
    });
    asset.folder_id = Some(uuid::Uuid::new_v4());
    let preview = Project::asset_preview(&asset).unwrap();
    assert_eq!(preview.tracks.len(), 1);
    assert_eq!(preview.duration(), asset.duration);
    assert_eq!(
        (preview.settings.width, preview.settings.height),
        (1080, 1920)
    );
    assert!(editor.project().assets.is_empty());
    assert!(editor.project().tracks.iter().all(|t| t.clips.is_empty()));
    assert_ne!(preview.id, editor.project().id);
    assert!(preview.assets[&asset.id].folder_id.is_none());
    assert!(asset.folder_id.is_some());
}
