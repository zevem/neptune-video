use editor_core::{Asset, Clip, Command, EditError, Editor, Marker, MediaKind, Project, Time};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn clip_fixture() -> (Editor, Uuid, Uuid) {
    let mut project = Project::default();
    let asset = Asset::new(
        PathBuf::from("offline-video.mov"),
        MediaKind::Video,
        seconds(60.0),
    );
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    clip.duration = seconds(10.0);
    let clip_id = clip.id;
    let track_id = project.tracks[0].id;
    Arc::make_mut(&mut project.assets).insert(asset.id, asset);
    Arc::make_mut(&mut project.tracks[0]).clips.push(clip);
    (Editor::new(project).unwrap(), track_id, clip_id)
}

#[test]
fn ripple_trim_moves_downstream_markers_with_clips() {
    let (mut editor, track_id, clip_id) = clip_fixture();
    let mut following = editor.project().clip(clip_id).unwrap().1.clone();
    following.id = Uuid::new_v4();
    following.start = seconds(10.0);
    following.source_in = seconds(10.0);
    following.duration = seconds(5.0);
    let following_id = following.id;
    let earlier_marker = Uuid::new_v4();
    let later_marker = Uuid::new_v4();
    editor
        .dispatch(Command::Batch {
            label: "Arrange sequence".into(),
            commands: vec![
                Command::AddClip {
                    track_id,
                    clip: following,
                },
                Command::AddMarker {
                    marker: Marker {
                        id: earlier_marker,
                        time: seconds(5.0),
                        label: "Inside retained content".into(),
                        color: [255; 4],
                    },
                },
                Command::AddMarker {
                    marker: Marker {
                        id: later_marker,
                        time: seconds(12.0),
                        label: "Following clip".into(),
                        color: [255; 4],
                    },
                },
            ],
        })
        .unwrap();

    editor
        .dispatch(Command::TrimClip {
            clip_id,
            source_in: Time::ZERO,
            duration: seconds(8.0),
            ripple: true,
        })
        .unwrap();

    assert_eq!(
        editor.project().clip(following_id).unwrap().1.start,
        seconds(8.0)
    );
    assert_eq!(
        editor
            .project()
            .markers
            .iter()
            .find(|marker| marker.id == later_marker)
            .unwrap()
            .time,
        seconds(10.0)
    );
    assert_eq!(
        editor
            .project()
            .markers
            .iter()
            .find(|marker| marker.id == earlier_marker)
            .unwrap()
            .time,
        seconds(5.0)
    );
}

#[test]
fn duplicate_clip_ids_cannot_create_a_single_clip_link() {
    let (mut editor, _, clip_id) = clip_fixture();
    let before = editor.snapshot();

    let result = editor.dispatch(Command::LinkClips {
        clip_ids: vec![clip_id, clip_id],
    });

    assert!(matches!(result, Err(EditError::Invalid(_))));
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
}

#[test]
fn duplicate_clip_ids_cannot_create_a_single_clip_group() {
    let (mut editor, _, clip_id) = clip_fixture();
    let before = editor.snapshot();

    let result = editor.dispatch(Command::GroupClips {
        clip_ids: vec![clip_id, clip_id],
    });

    assert!(matches!(result, Err(EditError::Invalid(_))));
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
}

#[test]
fn changing_one_linked_rate_preserves_relative_rates_and_timeline_alignment() {
    let mut project = Project::default();
    let mut asset = Asset::new(
        PathBuf::from("offline-video-with-audio.mov"),
        MediaKind::Video,
        seconds(60.0),
    );
    asset.has_audio = true;
    let link_id = Uuid::new_v4();
    let mut video = Clip::from_asset(&asset, Time::ZERO);
    video.duration = seconds(10.0);
    video.playback_rate = 2.0;
    video.linked_id = Some(link_id);
    let video_id = video.id;
    let mut audio = Clip::from_asset(&asset, Time::ZERO);
    audio.duration = seconds(10.0);
    audio.linked_id = Some(link_id);
    let audio_id = audio.id;
    Arc::make_mut(&mut project.assets).insert(asset.id, asset);
    Arc::make_mut(&mut project.tracks[0]).clips.push(video);
    Arc::make_mut(&mut project.tracks[1]).clips.push(audio);
    let mut editor = Editor::new(project).unwrap();

    editor
        .dispatch(Command::SetPlaybackRate {
            clip_id: video_id,
            rate: 4.0,
        })
        .unwrap();

    let video = editor.project().clip(video_id).unwrap().1;
    let audio = editor.project().clip(audio_id).unwrap().1;
    assert_eq!(
        (video.duration, audio.duration),
        (seconds(5.0), seconds(5.0))
    );
    assert_eq!((video.playback_rate, audio.playback_rate), (4.0, 2.0));
    assert_eq!(video.duration.scaled(video.playback_rate), seconds(20.0));
    assert_eq!(audio.duration.scaled(audio.playback_rate), seconds(10.0));
}

#[test]
fn acknowledging_an_older_save_after_a_newer_save_does_not_revert_saved_state() {
    let (mut editor, _, _) = clip_fixture();
    let old_revision = editor.register_save();
    editor
        .dispatch(Command::RenameProject {
            name: "Newly saved".into(),
        })
        .unwrap();
    let new_revision = editor.register_save();
    editor.mark_saved(new_revision);

    editor.mark_saved(old_revision);

    assert!(!editor.is_dirty());
    assert!(editor.undo());
    assert!(editor.is_dirty());
}
