use editor_core::{
    Asset, Clip, Command, EditError, Editor, MediaKind, PreparedEdit, Project, Time, TrackKind,
};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

fn paired_fixture() -> (Editor, [Uuid; 2]) {
    let mut project = Project::new("Prepared edit fixture");
    let mut asset = Asset::new(
        PathBuf::from("prepared.mov"),
        MediaKind::Video,
        Time(8_000_000),
    );
    asset.has_audio = true;
    let picture = Clip::from_asset(&asset, Time::ZERO);
    let sound = Clip::from_asset(&asset, Time::ZERO);
    let ids = [picture.id, sound.id];
    Arc::make_mut(&mut project.assets).insert(asset.id, asset);
    for (kind, clip) in [(TrackKind::Video, picture), (TrackKind::Audio, sound)] {
        let track = project.tracks.iter_mut().find(|t| t.kind == kind).unwrap();
        Arc::make_mut(track).clips.push(clip);
    }
    (Editor::new(project).unwrap(), ids)
}

#[test]
fn prepared_batch_commits_exact_split_link_group_ids_as_one_history_entry() {
    let (mut editor, ids) = paired_fixture();
    let before = editor.snapshot();
    let prepared = editor
        .prepare(Command::Batch {
            label: "Reviewed linked split".into(),
            commands: vec![
                Command::LinkClips {
                    clip_ids: ids.to_vec(),
                },
                Command::GroupClips {
                    clip_ids: ids.to_vec(),
                },
                Command::SplitClip {
                    clip_id: ids[0],
                    at: Time(3_000_000),
                },
            ],
        })
        .unwrap();
    let reviewed = prepared.after();
    let left = reviewed.clip(ids[0]).unwrap().1;
    assert!(left.linked_id.is_some());
    assert!(left.group_id.is_some());
    let right: Vec<_> = reviewed
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .filter(|clip| !ids.contains(&clip.id))
        .map(|clip| (clip.id, clip.linked_id, clip.group_id))
        .collect();
    assert_eq!(right.len(), 2);
    assert_eq!(right[0].1, right[1].1);
    assert_ne!(right[0].1, left.linked_id);
    assert_eq!(right[0].2, left.group_id);
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    assert_eq!(editor.revision(), 0);
    assert_eq!(editor.history().count(), 0);

    let cloned = prepared.clone();
    let outcome = editor.commit_prepared(&cloned).unwrap();
    assert!(outcome.changed);
    assert_eq!(outcome.label, "Reviewed linked split");
    assert_eq!(outcome.revision, 1);
    assert_eq!(editor.history().count(), 1);
    assert!(Arc::ptr_eq(&reviewed, &editor.snapshot()));
    for (id, linked_id, group_id) in right {
        let committed = editor.project().clip(id).unwrap().1;
        assert_eq!(committed.linked_id, linked_id);
        assert_eq!(committed.group_id, group_id);
    }
    assert!(editor.undo());
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    assert!(!editor.can_undo());
    assert!(editor.redo());
    assert!(Arc::ptr_eq(&reviewed, &editor.snapshot()));
    assert!(!editor.can_redo());
    assert!(editor.commit_prepared(&prepared).is_err());
    assert_eq!(editor.history().count(), 1);
}

#[test]
fn invalid_preparation_and_prepared_noop_preserve_redo_and_saved_boundary() {
    let (mut editor, ids) = paired_fixture();
    let original = editor.snapshot();
    editor
        .dispatch(Command::RenameProject {
            name: "Undo this".into(),
        })
        .unwrap();
    assert!(editor.undo());
    let revision = editor.revision();
    assert!(!editor.is_dirty());
    assert!(
        editor
            .prepare(Command::Batch {
                label: "Invalid proposal".into(),
                commands: vec![
                    Command::RenameProject {
                        name: "Must not survive".into(),
                    },
                    Command::SplitClip {
                        clip_id: ids[0],
                        at: Time::ZERO,
                    },
                ],
            })
            .is_err()
    );
    let prepared = editor
        .prepare(Command::Batch {
            label: "Net no-op".into(),
            commands: vec![
                Command::RenameProject {
                    name: "Temporary".into(),
                },
                Command::RenameProject {
                    name: original.name.clone(),
                },
            ],
        })
        .unwrap();
    assert!(!prepared.changed());
    assert!(Arc::ptr_eq(&original, &prepared.before()));
    assert!(Arc::ptr_eq(&original, &prepared.after()));
    let outcome = editor.commit_prepared(&prepared).unwrap();
    assert!(!outcome.changed);
    assert_eq!(outcome.revision, revision);
    assert_eq!(editor.revision(), revision);
    assert!(Arc::ptr_eq(&original, &editor.snapshot()));
    assert!(editor.can_redo());
    assert_eq!(editor.redo_label(), Some("Rename project"));
    assert!(!editor.can_undo());
    assert!(!editor.is_dirty());
    assert!(editor.redo());
    assert_eq!(editor.project().name, "Undo this");
}

#[test]
fn changed_then_undone_and_reopened_editor_reject_prepared_edit() {
    let (mut editor, _) = paired_fixture();
    let before = editor.snapshot();
    let prepared = editor
        .prepare(Command::RenameProject {
            name: "Reviewed".into(),
        })
        .unwrap();
    let mut restored = Editor::from_session(editor.session_state()).unwrap();
    assert!(Arc::ptr_eq(&before, &restored.snapshot()));
    assert!(!prepared.is_current(&restored));
    assert!(restored.commit_prepared(&prepared).is_err());
    assert_eq!(restored.revision(), 0);
    assert!(!restored.can_undo());
    assert!(!restored.is_dirty());

    let mut reopened = Editor::new(editor.project().clone()).unwrap();
    assert!(reopened.commit_prepared(&prepared).is_err());
    assert!(!reopened.can_undo());
    editor
        .dispatch(Command::RenameProject {
            name: "Human edit".into(),
        })
        .unwrap();
    editor.undo();
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    let revision = editor.revision();
    assert!(matches!(
        editor.commit_prepared(&prepared),
        Err(EditError::StaleRevision { .. })
    ));
    assert_eq!(editor.revision(), revision);
    assert!(editor.can_redo());
    assert!(!editor.can_undo());
    assert!(!editor.is_dirty());
}

#[test]
fn equal_value_source_copy_and_other_project_cannot_confer_commit_authority() {
    let (mut editor, _) = paired_fixture();
    let before = editor.snapshot();
    for source in [
        Arc::new(editor.project().clone()),
        Arc::new(Project::new("Other document")),
    ] {
        let prepared = PreparedEdit::from_snapshot(
            source,
            editor.instance_id(),
            editor.revision(),
            Command::RenameProject {
                name: "Untrusted root".into(),
            },
        )
        .unwrap();
        assert!(!prepared.is_current(&editor));
        assert!(editor.commit_prepared(&prepared).is_err());
        assert!(Arc::ptr_eq(&before, &editor.snapshot()));
        assert_eq!(editor.revision(), 0);
        assert!(!editor.can_undo());
        assert!(!editor.is_dirty());
    }
}

#[test]
fn mutated_preview_copies_and_save_acknowledgement_leave_prepared_authority_intact() {
    let (mut editor, _) = paired_fixture();
    editor
        .dispatch(Command::RenameProject {
            name: "Pending save".into(),
        })
        .unwrap();
    let saved_revision = editor.register_save();
    let before = editor.snapshot();
    let prepared = editor
        .prepare(Command::RenameProject {
            name: "Reviewed".into(),
        })
        .unwrap();
    let reviewed = prepared.after();
    let mut modified_after = prepared.after();
    Arc::make_mut(&mut modified_after).name = "Changed preview copy".into();
    let mut modified_before = prepared.before();
    Arc::make_mut(&mut modified_before).id = Uuid::new_v4();
    editor.mark_saved(saved_revision);
    assert!(!editor.is_dirty());
    assert!(prepared.is_current(&editor));
    editor.commit_prepared(&prepared).unwrap();
    assert!(Arc::ptr_eq(&reviewed, &editor.snapshot()));
    assert_eq!(editor.project().name, "Reviewed");
    assert!(editor.is_dirty());
    editor.undo();
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    assert!(!editor.is_dirty());
}

#[test]
fn prepared_commands_use_the_same_sort_and_validation_as_dispatch() {
    let (mut editor, ids) = paired_fixture();
    let source_track = editor.project().clip(ids[0]).unwrap().0.id;
    let mut later = Clip::from_asset(
        editor.project().assets.values().next().unwrap(),
        Time(5_000_000),
    );
    // Keep the source range within the eight-second fixture.
    later.duration = Time(2_000_000);
    let later_id = later.id;
    let prepared = editor
        .prepare(Command::AddClip {
            track_id: source_track,
            clip: later,
        })
        .unwrap();
    editor.commit_prepared(&prepared).unwrap();
    let prepared = editor
        .prepare(Command::MoveClips {
            clip_ids: vec![ids[0]],
            delta: Time(6_000_000),
            target_track: None,
        })
        .unwrap();
    assert!(prepared.after().validate().is_ok());
    assert!(
        prepared
            .after()
            .track(source_track)
            .unwrap()
            .clips
            .windows(2)
            .all(|pair| pair[0].start <= pair[1].start)
    );
    editor.commit_prepared(&prepared).unwrap();
    assert_eq!(
        editor.project().track(source_track).unwrap().clips[0].id,
        later_id
    );
    assert_eq!(
        editor.project().clip(ids[0]).unwrap().1.start,
        Time(6_000_000)
    );
}
