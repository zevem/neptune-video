use editor_core::{Clip, ClipId, Command, Editor, Time, TrackId};

fn seconds(value: i64) -> Time {
    Time(value * 1_000_000)
}

fn overlapping_fixture() -> (Editor, TrackId, [ClipId; 8]) {
    let mut editor = Editor::default();
    let track_id = editor.project().tracks[2].id;
    let long = Clip::text("Long [0, 100)", seconds(0), seconds(100));
    let first_short = Clip::text("Short [1, 2)", seconds(1), seconds(1));
    let second_short = Clip::text("Short [3, 4)", seconds(3), seconds(1));
    let same_start_short = Clip::text("Short [10, 11)", seconds(10), seconds(1));
    let nested = Clip::text("Nested [10, 80)", seconds(10), seconds(70));
    let inner = Clip::text("Inner [20, 21)", seconds(20), seconds(1));
    let future = Clip::text("Future [60, 70)", seconds(60), seconds(10));
    let after = Clip::text("After [120, 130)", seconds(120), seconds(10));
    let ids = [
        long.id,
        first_short.id,
        second_short.id,
        same_start_short.id,
        nested.id,
        inner.id,
        future.id,
        after.id,
    ];
    // The reducer sorts starts, preserving insertion order for the same start.
    let clips = [
        future,
        same_start_short,
        after,
        second_short,
        nested,
        first_short,
        inner,
        long,
    ];
    editor
        .dispatch(Command::Batch {
            label: "Arrange overlapping clips".into(),
            commands: clips
                .into_iter()
                .map(|clip| Command::AddClip { track_id, clip })
                .collect(),
        })
        .unwrap();
    editor.project().validate().unwrap();
    (editor, track_id, ids)
}

#[test]
fn overlap_queries_preserve_long_nested_and_same_start_clips_at_half_open_boundaries() {
    let (editor, track_id, ids) = overlapping_fixture();
    let [
        long,
        first_short,
        second_short,
        same_start_short,
        nested,
        inner,
        future,
        after,
    ] = ids;
    let track = editor.project().track(track_id).unwrap();
    let cases = [
        (
            "long and nested after short clips",
            50,
            60,
            vec![long, nested],
        ),
        ("exclude clip starting at right edge", 0, 1, vec![long]),
        (
            "include coincident start and end",
            1,
            2,
            vec![long, first_short],
        ),
        ("exclude both touching short clips", 2, 3, vec![long]),
        ("second short clip", 3, 4, vec![long, second_short]),
        ("exclude same-start clips at right edge", 4, 10, vec![long]),
        (
            "same-start insertion order",
            10,
            11,
            vec![long, same_start_short, nested],
        ),
        (
            "exclude ended short and future inner",
            11,
            20,
            vec![long, nested],
        ),
        ("inner clip", 20, 21, vec![long, nested, inner]),
        (
            "exclude ended inner and future start",
            21,
            60,
            vec![long, nested],
        ),
        (
            "future clip at left edge",
            60,
            70,
            vec![long, nested, future],
        ),
        ("future clip touches left edge", 70, 80, vec![long, nested]),
        ("nested clip touches left edge", 80, 100, vec![long]),
        ("gap between touching clips", 100, 120, vec![]),
        ("last clip", 120, 130, vec![after]),
        ("view after every clip ends", 130, 140, vec![]),
        ("negative view touches zero", -10, 0, vec![]),
        ("negative view crossing zero", -10, 1, vec![long]),
        ("entirely negative view", -10, -1, vec![]),
        ("empty view within long clips", 50, 50, vec![]),
        ("reversed view within long clips", 60, 50, vec![]),
    ];
    for (name, start, end, expected) in cases {
        let actual: Vec<_> = track
            .clips_overlapping(seconds(start), seconds(end))
            .map(|clip| clip.id)
            .collect();
        assert_eq!(actual, expected, "{name}");
    }
    let borrowed = track
        .clips_overlapping(seconds(50), seconds(60))
        .next()
        .unwrap();
    assert!(std::ptr::eq(borrowed, &track.clips[0]));
    assert_eq!(
        track.clips.iter().map(|clip| clip.id).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        editor.project().tracks[0]
            .clips_overlapping(seconds(-10), seconds(140))
            .count(),
        0
    );
}

#[test]
fn overlap_queries_follow_move_undo_and_redo_without_changing_a_captured_track() {
    let (mut editor, track_id, ids) = overlapping_fixture();
    let long = ids[0];
    let nested = ids[4];
    let before = editor.snapshot();
    editor
        .dispatch(Command::MoveClips {
            clip_ids: vec![long],
            delta: seconds(110),
            target_track: None,
        })
        .unwrap();
    editor.project().validate().unwrap();
    assert_eq!(
        editor
            .project()
            .track(track_id)
            .unwrap()
            .clips_overlapping(seconds(50), seconds(60))
            .map(|clip| clip.id)
            .collect::<Vec<_>>(),
        vec![nested]
    );
    assert_eq!(
        before
            .track(track_id)
            .unwrap()
            .clips_overlapping(seconds(50), seconds(60))
            .map(|clip| clip.id)
            .collect::<Vec<_>>(),
        vec![long, nested]
    );
    assert_eq!(
        editor
            .project()
            .track(track_id)
            .unwrap()
            .clips_overlapping(seconds(120), seconds(130))
            .map(|clip| clip.id)
            .collect::<Vec<_>>(),
        vec![long, ids[7]]
    );
    assert!(editor.undo());
    assert_eq!(
        editor
            .project()
            .track(track_id)
            .unwrap()
            .clips_overlapping(seconds(50), seconds(60))
            .map(|clip| clip.id)
            .collect::<Vec<_>>(),
        vec![long, nested]
    );
    assert!(editor.redo());
    assert_eq!(
        editor
            .project()
            .track(track_id)
            .unwrap()
            .clips_overlapping(seconds(50), seconds(60))
            .map(|clip| clip.id)
            .collect::<Vec<_>>(),
        vec![nested]
    );
}
