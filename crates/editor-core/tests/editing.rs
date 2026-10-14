use editor_core::*;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

fn seconds(n: f64) -> Time {
    Time::from_seconds(n)
}
fn fixture() -> (Editor, AssetId, TrackId, ClipId) {
    let mut editor = Editor::default();
    let asset = Asset::new(
        PathBuf::from("/media/shot.mp4"),
        MediaKind::Video,
        seconds(60.0),
    );
    let id = asset.id;
    let track = editor.project().tracks[0].id;
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    clip.duration = seconds(10.0);
    let clip_id = clip.id;
    editor
        .dispatch(Command::Batch {
            label: "Insert footage".into(),
            commands: vec![
                Command::AddAssets {
                    assets: vec![asset],
                },
                Command::AddClip {
                    track_id: track,
                    clip,
                },
            ],
        })
        .unwrap();
    (editor, id, track, clip_id)
}
#[test]
fn split_preserves_source_and_total_duration() {
    let (mut e, _, track, id) = fixture();
    e.dispatch(Command::SplitClip {
        clip_id: id,
        at: seconds(4.0),
    })
    .unwrap();
    let clips = &e.project().track(track).unwrap().clips;
    assert_eq!(
        clips.iter().map(|c| c.duration.0).sum::<i64>(),
        seconds(10.0).0
    );
    assert_eq!(clips[1].source_in, seconds(4.0));
    assert_ne!(clips[0].id, clips[1].id);
}
#[test]
fn undo_redo_restores_exact_document_and_identity() {
    let (mut e, _, _, id) = fixture();
    let before = e.snapshot();
    e.dispatch(Command::SplitClip {
        clip_id: id,
        at: seconds(4.0),
    })
    .unwrap();
    let after = e.snapshot();
    assert!(e.undo());
    assert!(Arc::ptr_eq(&before, &e.snapshot()));
    assert!(e.redo());
    assert!(Arc::ptr_eq(&after, &e.snapshot()));
}
#[test]
fn failed_batch_is_atomic_and_does_not_clear_redo() {
    let (mut e, _, _, id) = fixture();
    e.dispatch(Command::SplitClip {
        clip_id: id,
        at: seconds(4.0),
    })
    .unwrap();
    e.undo();
    let before = e.snapshot();
    let revision = e.revision();
    let error = e.dispatch(Command::Batch {
        label: "Bad".into(),
        commands: vec![
            Command::RenameProject {
                name: "Changed".into(),
            },
            Command::SetTransform {
                clip_id: Uuid::new_v4(),
                transform: Transform::default(),
            },
        ],
    });
    assert!(error.is_err());
    assert!(Arc::ptr_eq(&before, &e.snapshot()));
    assert_eq!(revision, e.revision());
    assert!(e.can_redo());
}
#[test]
fn unchanged_tracks_share_storage_with_history() {
    let (mut e, _, _, id) = fixture();
    let before = e.snapshot();
    e.dispatch(Command::MoveClips {
        clip_ids: vec![id],
        delta: seconds(1.0),
        target_track: None,
    })
    .unwrap();
    assert!(Arc::ptr_eq(&before.tracks[1], &e.project().tracks[1]));
    assert!(Arc::ptr_eq(&before.assets, &e.project().assets));
}
#[test]
fn lock_blocks_mutations_but_allows_unlock() {
    let (mut e, _, track, id) = fixture();
    e.dispatch(Command::SetTrackState {
        track_id: track,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    })
    .unwrap();
    let before = e.snapshot();
    assert!(matches!(
        e.dispatch(Command::DeleteClips {
            clip_ids: vec![id],
            ripple: false
        }),
        Err(EditError::Locked)
    ));
    assert!(Arc::ptr_eq(&before, &e.snapshot()));
    e.dispatch(Command::SetTrackState {
        track_id: track,
        locked: Some(false),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    })
    .unwrap();
    assert!(
        e.dispatch(Command::DeleteClips {
            clip_ids: vec![id],
            ripple: false
        })
        .is_ok()
    );
}
#[test]
fn locked_target_track_rejects_move() {
    let (mut e, _, _, id) = fixture();
    let mut track = Track::new("Locked", TrackKind::Video);
    track.locked = true;
    let target = track.id;
    e.dispatch(Command::AddTrack { track }).unwrap();
    assert!(matches!(
        e.dispatch(Command::MoveClips {
            clip_ids: vec![id],
            delta: Time::ZERO,
            target_track: Some(target)
        }),
        Err(EditError::Locked)
    ));
}
#[test]
fn incompatible_track_rejects_move_atomically() {
    let (mut e, _, _, id) = fixture();
    let target = e.project().tracks[2].id;
    let before = e.snapshot();
    assert!(matches!(
        e.dispatch(Command::MoveClips {
            clip_ids: vec![id],
            delta: Time::ZERO,
            target_track: Some(target)
        }),
        Err(EditError::IncompatibleTrack)
    ));
    assert!(Arc::ptr_eq(&before, &e.snapshot()));
}
#[test]
fn cannot_trim_beyond_source_or_move_before_zero() {
    let (mut e, _, _, id) = fixture();
    assert!(
        e.dispatch(Command::TrimClip {
            clip_id: id,
            source_in: Time::ZERO,
            duration: seconds(61.0),
            ripple: false
        })
        .is_err()
    );
    assert!(
        e.dispatch(Command::MoveClips {
            clip_ids: vec![id],
            delta: seconds(-1.0),
            target_track: None
        })
        .is_err()
    );
}
#[test]
fn left_trim_advances_timeline_and_source() {
    let (mut e, _, _, id) = fixture();
    e.dispatch(Command::TrimClip {
        clip_id: id,
        source_in: seconds(2.0),
        duration: seconds(8.0),
        ripple: false,
    })
    .unwrap();
    let c = e.project().clip(id).unwrap().1;
    assert_eq!(
        (c.start, c.source_in, c.duration, c.end()),
        (seconds(2.0), seconds(2.0), seconds(8.0), seconds(10.0))
    );
}
#[test]
fn ripple_left_trim_keeps_start_and_shifts_following_clips() {
    let (mut e, asset, track, id) = fixture();
    let mut next = Clip::from_asset(&e.project().assets[&asset], seconds(10.0));
    next.duration = seconds(3.0);
    let next_id = next.id;
    e.dispatch(Command::AddClip {
        track_id: track,
        clip: next,
    })
    .unwrap();
    e.dispatch(Command::TrimClip {
        clip_id: id,
        source_in: seconds(2.0),
        duration: seconds(8.0),
        ripple: true,
    })
    .unwrap();
    assert_eq!(e.project().clip(id).unwrap().1.start, Time::ZERO);
    assert_eq!(e.project().clip(next_id).unwrap().1.start, seconds(8.0));
}
#[test]
fn ripple_delete_closes_gap_and_moves_markers() {
    let (mut e, asset, track, id) = fixture();
    let mut next = Clip::from_asset(&e.project().assets[&asset], seconds(12.0));
    next.duration = seconds(4.0);
    let next_id = next.id;
    e.dispatch(Command::AddClip {
        track_id: track,
        clip: next,
    })
    .unwrap();
    let marker = Marker {
        id: Uuid::new_v4(),
        time: seconds(13.0),
        label: "Beat".into(),
        color: [255; 4],
    };
    e.dispatch(Command::AddMarker { marker }).unwrap();
    e.dispatch(Command::DeleteClips {
        clip_ids: vec![id],
        ripple: true,
    })
    .unwrap();
    assert_eq!(e.project().clip(next_id).unwrap().1.start, seconds(2.0));
    assert_eq!(e.project().markers[0].time, seconds(3.0));
}
#[test]
fn ripple_delete_overlapping_intervals_only_removes_union() {
    let (mut e, asset, track, id) = fixture();
    let mut second = Clip::from_asset(&e.project().assets[&asset], seconds(5.0));
    second.duration = seconds(10.0);
    let second_id = second.id;
    let mut third = Clip::from_asset(&e.project().assets[&asset], seconds(20.0));
    third.duration = seconds(2.0);
    let third_id = third.id;
    e.dispatch(Command::Batch {
        label: "Insert".into(),
        commands: vec![
            Command::AddClip {
                track_id: track,
                clip: second,
            },
            Command::AddClip {
                track_id: track,
                clip: third,
            },
        ],
    })
    .unwrap();
    e.dispatch(Command::DeleteClips {
        clip_ids: vec![id, second_id],
        ripple: true,
    })
    .unwrap();
    assert_eq!(e.project().clip(third_id).unwrap().1.start, seconds(5.0));
}
#[test]
fn ripple_edits_reject_locked_sync_tracks_atomically() {
    let (mut e, _, _, id) = fixture();
    let track = e.project().tracks[2].id;
    e.dispatch(Command::AddClip {
        track_id: track,
        clip: Clip::text("Title", seconds(15.0), seconds(2.0)),
    })
    .unwrap();
    e.dispatch(Command::SetTrackState {
        track_id: track,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    })
    .unwrap();
    let before = e.snapshot();
    assert!(matches!(
        e.dispatch(Command::DeleteClips {
            clip_ids: vec![id],
            ripple: true
        }),
        Err(EditError::Locked)
    ));
    assert!(Arc::ptr_eq(&before, &e.snapshot()));
}
fn linked_fixture() -> (Editor, ClipId, ClipId) {
    let (mut e, asset_id, _, id) = fixture();
    let asset = e.project().assets[&asset_id].clone();
    let track = e.project().tracks[0].id;
    let mut second = Clip::from_asset(&asset, Time::ZERO);
    second.duration = seconds(10.0);
    let second_id = second.id;
    e.dispatch(Command::AddClip {
        track_id: track,
        clip: second,
    })
    .unwrap();
    e.dispatch(Command::LinkClips {
        clip_ids: vec![id, second_id],
    })
    .unwrap();
    (e, id, second_id)
}
#[test]
fn linked_move_and_delete_apply_to_entire_link() {
    let (mut e, id, partner) = linked_fixture();
    e.dispatch(Command::MoveClips {
        clip_ids: vec![id],
        delta: seconds(3.0),
        target_track: None,
    })
    .unwrap();
    assert_eq!(e.project().clip(partner).unwrap().1.start, seconds(3.0));
    e.dispatch(Command::DeleteClips {
        clip_ids: vec![id],
        ripple: false,
    })
    .unwrap();
    assert!(e.project().clip(partner).is_none());
}
#[test]
fn linked_split_creates_matching_new_relationships() {
    let (mut e, id, partner) = linked_fixture();
    let original = e.project().clip(id).unwrap().1.linked_id;
    e.dispatch(Command::SplitClip {
        clip_id: id,
        at: seconds(4.0),
    })
    .unwrap();
    let clips = &e.project().tracks[0].clips;
    let right: Vec<_> = clips.iter().filter(|c| c.start == seconds(4.0)).collect();
    assert_eq!(right.len(), 2);
    assert_eq!(right[0].linked_id, right[1].linked_id);
    assert_ne!(right[0].linked_id, original);
    assert_eq!(e.project().clip(partner).unwrap().1.duration, seconds(4.0));
}
#[test]
fn linked_trim_and_speed_preserve_sync() {
    let (mut e, id, partner) = linked_fixture();
    e.dispatch(Command::TrimClip {
        clip_id: id,
        source_in: seconds(2.0),
        duration: seconds(6.0),
        ripple: false,
    })
    .unwrap();
    assert_eq!(e.project().clip(partner).unwrap().1.source_in, seconds(2.0));
    e.dispatch(Command::SetPlaybackRate {
        clip_id: id,
        rate: 2.0,
    })
    .unwrap();
    assert_eq!(e.project().clip(partner).unwrap().1.duration, seconds(3.0));
}
#[test]
fn grouped_move_uses_transitive_relationship_closure() {
    let (mut e, id, partner) = linked_fixture();
    let track = e.project().tracks[2].id;
    let third = Clip::text("Third", Time::ZERO, seconds(2.0));
    let third_id = third.id;
    e.dispatch(Command::AddClip {
        track_id: track,
        clip: third,
    })
    .unwrap();
    e.dispatch(Command::GroupClips {
        clip_ids: vec![partner, third_id],
    })
    .unwrap();
    e.dispatch(Command::MoveClips {
        clip_ids: vec![id],
        delta: seconds(3.0),
        target_track: None,
    })
    .unwrap();
    assert_eq!(e.project().clip(third_id).unwrap().1.start, seconds(3.0));
}
#[test]
fn clipboard_remaps_ids_and_links() {
    let (mut e, id, _) = linked_fixture();
    let clipboard = e.copy(&[id]).unwrap();
    assert_eq!(clipboard.clips.len(), 2);
    e.paste(&clipboard, seconds(20.0), None).unwrap();
    let pasted: Vec<_> = e.project().tracks[0]
        .clips
        .iter()
        .filter(|c| c.start == seconds(20.0))
        .collect();
    assert_eq!(pasted.len(), 2);
    assert_eq!(pasted[0].linked_id, pasted[1].linked_id);
    assert_ne!(pasted[0].linked_id, clipboard.clips[0].clip.linked_id);
    assert_ne!(pasted[0].id, clipboard.clips[0].clip.id);
    assert_eq!(e.undo_label(), Some("Paste clips"));
}
#[test]
fn clip_sorting_is_restored_after_crossing_another_clip() {
    let (mut e, _, _, id) = fixture();
    e.dispatch(Command::SplitClip {
        clip_id: id,
        at: seconds(4.0),
    })
    .unwrap();
    e.dispatch(Command::MoveClips {
        clip_ids: vec![id],
        delta: seconds(10.0),
        target_track: None,
    })
    .unwrap();
    assert_eq!(e.project().tracks[0].clips[1].id, id);
    assert!(e.project().validate().is_ok());
}
#[test]
fn image_can_extend_beyond_default_still_duration() {
    let mut e = Editor::default();
    let asset = Asset::new(PathBuf::from("/photo.png"), MediaKind::Image, seconds(5.0));
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    clip.duration = seconds(90.0);
    e.dispatch(Command::Batch {
        label: "Still".into(),
        commands: vec![
            Command::AddAssets {
                assets: vec![asset],
            },
            Command::AddClip {
                track_id: e.project().tracks[0].id,
                clip,
            },
        ],
    })
    .unwrap();
    assert_eq!(e.project().duration(), seconds(90.0));
}
#[test]
fn project_validation_rejects_duplicate_ids_and_folder_cycles() {
    let mut p = Project::default();
    p.tracks.push(p.tracks[0].clone());
    assert!(p.validate().is_err());
    let mut p = Project::default();
    let id = Uuid::new_v4();
    Arc::make_mut(&mut p.folders).insert(
        id,
        MediaFolder {
            id,
            name: "Cycle".into(),
            parent_id: Some(id),
        },
    );
    assert!(p.validate().is_err());
}
#[test]
fn invalid_nan_transform_is_rejected_without_history() {
    let (mut e, _, _, id) = fixture();
    let revision = e.revision();
    let mut transform = Transform::default();
    transform.scale[0] = f32::NAN;
    assert!(
        e.dispatch(Command::SetTransform {
            clip_id: id,
            transform
        })
        .is_err()
    );
    assert_eq!(e.revision(), revision);
}
#[test]
fn referenced_media_cannot_be_removed() {
    let (mut e, asset, _, _) = fixture();
    assert!(
        e.dispatch(Command::RemoveAssets {
            asset_ids: vec![asset]
        })
        .is_err()
    );
}
#[test]
fn revision_guard_rejects_stale_agent_transaction() {
    let (mut e, _, _, _) = fixture();
    assert!(matches!(
        e.dispatch_at(
            0,
            Command::RenameProject {
                name: "Agent".into()
            }
        ),
        Err(EditError::StaleRevision { .. })
    ));
}
#[test]
fn no_op_does_not_increment_revision() {
    let (mut e, _, _, _) = fixture();
    let revision = e.revision();
    let outcome = e
        .dispatch(Command::RenameProject {
            name: e.project().name.clone(),
        })
        .unwrap();
    assert!(!outcome.changed);
    assert_eq!(revision, e.revision());
}
#[test]
fn save_ack_is_bound_to_the_submitted_snapshot() {
    let (mut e, _, _, id) = fixture();
    let saved_revision = e.register_save();
    let saved = e.snapshot();
    e.dispatch(Command::MoveClips {
        clip_ids: vec![id],
        delta: seconds(2.0),
        target_track: None,
    })
    .unwrap();
    e.mark_saved(saved_revision);
    assert!(e.is_dirty());
    e.undo();
    assert!(Arc::ptr_eq(&saved, &e.snapshot()));
    assert!(!e.is_dirty());
}
#[test]
fn no_op_edit_after_undo_keeps_redo() {
    let (mut e, _, _, id) = fixture();
    e.dispatch(Command::SplitClip {
        clip_id: id,
        at: seconds(4.0),
    })
    .unwrap();
    e.undo();
    e.dispatch(Command::RenameProject {
        name: e.project().name.clone(),
    })
    .unwrap();
    assert!(e.can_redo());
}
#[test]
fn active_layers_have_half_open_boundaries_and_ignore_hidden_tracks() {
    let (mut e, _, track, _) = fixture();
    assert_eq!(e.project().active_clips(Time::ZERO).count(), 1);
    assert_eq!(e.project().active_clips(seconds(10.0)).count(), 0);
    e.dispatch(Command::SetTrackState {
        track_id: track,
        locked: None,
        hidden: Some(true),
        muted: None,
        name: None,
        gain: None,
        solo: None,
    })
    .unwrap();
    assert_eq!(e.project().active_clips(seconds(5.0)).count(), 0);
}
#[test]
fn snapping_excludes_selected_clips_and_resolves_ties_earlier() {
    let (e, _, _, id) = fixture();
    assert_eq!(
        snap_time(e.project(), seconds(9.9), seconds(0.2), &[]),
        seconds(10.0)
    );
    assert_eq!(
        snap_time(e.project(), seconds(9.9), seconds(0.2), &[id]),
        seconds(9.9)
    );
}
#[test]
fn rational_frame_boundaries_round_trip_for_an_hour() {
    for rate in [
        FrameRate {
            numerator: 24000,
            denominator: 1001,
        },
        FrameRate {
            numerator: 30000,
            denominator: 1001,
        },
        FrameRate {
            numerator: 60000,
            denominator: 1001,
        },
        FrameRate::default(),
    ] {
        for frame in [0, 1, 2, 999, 10_000, 100_000, 215_784] {
            assert_eq!(rate.frame_at(rate.frame_time(frame)), frame);
        }
    }
}
#[test]
fn frame_step_stays_on_rational_grid() {
    let rate = FrameRate {
        numerator: 30000,
        denominator: 1001,
    };
    let mut p = Playback::default();
    for _ in 0..10000 {
        p.step(1, rate);
    }
    assert_eq!(p.position, rate.frame_time(10000));
    p.step(-10001, rate);
    assert_eq!(p.position, Time::ZERO);
}
#[test]
fn monotonic_playback_applies_speed_and_stops_at_end() {
    let mut p = Playback::default();
    let now = Instant::now();
    p.set_speed(2.0).unwrap();
    p.play();
    p.tick(now, seconds(5.0));
    p.tick(now + Duration::from_secs(1), seconds(5.0));
    assert_eq!(p.position, seconds(2.0));
    p.tick(now + Duration::from_secs(5), seconds(5.0));
    assert_eq!(p.position, seconds(5.0));
    assert!(!p.playing);
}
#[test]
fn seek_resets_clock_anchor() {
    let mut p = Playback::default();
    let now = Instant::now();
    p.play();
    p.tick(now, seconds(20.0));
    p.seek(seconds(10.0));
    p.tick(now + Duration::from_secs(1), seconds(20.0));
    p.tick(now + Duration::from_secs(2), seconds(20.0));
    assert_eq!(p.position, seconds(11.0));
}
#[test]
fn serialization_round_trips_project_and_command() {
    let (e, _, _, id) = fixture();
    let json = serde_json::to_string(e.project()).unwrap();
    assert_eq!(
        &serde_json::from_str::<Project>(&json).unwrap(),
        e.project()
    );
    let command = Command::SplitClip {
        clip_id: id,
        at: seconds(4.0),
    };
    assert!(matches!(
        serde_json::from_str::<Command>(&serde_json::to_string(&command).unwrap()).unwrap(),
        Command::SplitClip { .. }
    ));
}
