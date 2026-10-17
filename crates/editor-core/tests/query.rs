use editor_core::{Clip, Command, Editor, Project, ProjectQuery, Time};
use std::sync::Arc;

fn compare(query: &ProjectQuery) {
    for time in [
        -1, 0, 1, 2, 3, 9, 10, 11, 20, 21, 50, 60, 70, 80, 99, 100, 120, 130,
    ] {
        let time = Time(time * 1_000_000);
        let expected: Vec<_> = query
            .project()
            .active_clips(time)
            .map(|(t, c)| (t.id, c.id))
            .collect();
        let actual: Vec<_> = query
            .active_clips(time)
            .map(|(t, c)| (t.id, c.id))
            .collect();
        assert_eq!(actual, expected, "time {}", time.0);
        assert_eq!(
            query.resolve(time, &[]),
            editor_core::resolve(query.project(), time, &[]),
            "render plan at {}",
            time.0
        );
    }
}

#[test]
fn indexed_queries_preserve_nested_same_start_and_half_open_order() {
    let mut project = Project::new("Intervals");
    let track = Arc::make_mut(&mut project.tracks[2]);
    for (start, duration) in [
        (0, 100),
        (1, 1),
        (3, 1),
        (10, 1),
        (10, 70),
        (20, 1),
        (60, 10),
        (120, 10),
    ] {
        track.clips.push(Clip::text(
            "Title",
            Time(start * 1_000_000),
            Time(duration * 1_000_000),
        ));
    }
    project.validate().unwrap();
    compare(&ProjectQuery::new(Arc::new(project)));
}

#[test]
fn query_updates_follow_move_undo_redo_and_hidden_tracks() {
    let mut project = Project::new("Updates");
    let track = Arc::make_mut(&mut project.tracks[2]);
    let id = track.id;
    let clip = Clip::text("Long", Time::ZERO, Time(100_000_000));
    let clip_id = clip.id;
    track.clips.push(clip);
    let mut editor = Editor::new(project).unwrap();
    let captured = ProjectQuery::new(editor.snapshot());
    let mut query = ProjectQuery::new(editor.snapshot());
    editor
        .dispatch(Command::MoveClips {
            clip_ids: vec![clip_id],
            delta: Time(110_000_000),
            target_track: None,
        })
        .unwrap();
    query.update(editor.snapshot());
    compare(&query);
    compare(&captured);
    assert_eq!(query.active_clips(Time(50_000_000)).count(), 0);
    assert_eq!(captured.active_clips(Time(50_000_000)).count(), 1);
    assert!(editor.undo());
    query.update(editor.snapshot());
    compare(&query);
    assert!(editor.redo());
    query.update(editor.snapshot());
    compare(&query);
    editor
        .dispatch(Command::SetTrackState {
            track_id: id,
            hidden: Some(true),
            locked: None,
            muted: None,
            name: None,
            gain: None,
            solo: None,
        })
        .unwrap();
    query.update(editor.snapshot());
    compare(&query);
    assert_eq!(query.active_clips(Time(120_000_000)).count(), 0);
}

#[test]
fn reordered_replaced_and_reloaded_roots_rebuild_correct_bounds() {
    let mut project = Project::new("Replacement");
    Arc::make_mut(&mut project.tracks[2]).clips.push(Clip::text(
        "Old",
        Time::ZERO,
        Time(10_000_000),
    ));
    let mut query = ProjectQuery::new(Arc::new(project.clone()));
    Arc::make_mut(&mut project.tracks[2]).clips[0].duration = Time(100_000_000);
    project.tracks.rotate_right(1);
    query.update(Arc::new(project));
    compare(&query);
    assert_eq!(query.active_clips(Time(50_000_000)).count(), 1);
    let json = serde_json::to_string(query.project()).unwrap();
    let reloaded: Project = serde_json::from_str(&json).unwrap();
    reloaded.validate().unwrap();
    query.update(Arc::new(reloaded));
    compare(&query);
}
