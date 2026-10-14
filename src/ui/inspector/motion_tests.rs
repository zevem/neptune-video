//! The Motion section, driven as the rest of the inspector is in the tests
//! above: a region chosen and handed to the analysis worker, a clip made to
//! follow a tracker, and trackers renamed and removed.
use super::*;
use crate::ui::{TrackProgress, TrackRegion};
use editor_core::{Follow, TrackKey, Tracker};
use uuid::Uuid;

/// A bench on the Motion section of its clip: four seconds from the start of
/// a ten second video, with the playhead a second in.
fn moving() -> (Bench, ClipId) {
    let (mut bench, clip) = Bench::new();
    bench.state.section = Section::Motion;
    (bench, clip)
}

/// Gives the project a tracker of the clip's media that goes from the left
/// of the frame to the right over four seconds of source, a quarter down.
fn tracked(bench: &mut Bench, clip: ClipId, name: &str) -> Uuid {
    let asset_id = bench.clip(clip).asset_id().expect("a media clip");
    let id = Uuid::new_v4();
    let key = |at: f64, x: f32| TrackKey {
        source: seconds(at),
        center: [x, 0.25],
        scale: 1.0,
        rotation: 0.0,
    };
    bench.edit(Command::SetTracker {
        tracker: Tracker {
            id,
            name: name.to_owned(),
            asset_id,
            keys: vec![key(0.0, 0.1), key(4.0, 0.9)],
        },
    });
    id
}

/// A title over the bench's clip, selected, with its Motion section shown.
fn titled(bench: &mut Bench) -> ClipId {
    let title = bench.add(
        TrackKind::Text,
        Clip::text("Label", Time::ZERO, seconds(4.0)),
    );
    bench.selection.only(title);
    bench.state.section = Section::Motion;
    bench.frame(Vec::new());
    title
}

#[test]
fn a_region_is_chosen_and_handed_over_with_the_clip_and_frame_it_was_made_for() {
    let (mut bench, clip) = moving();
    let asset = bench.clip(clip).asset_id().unwrap();
    let before = bench.steps();
    bench.frame(Vec::new());
    assert!(!bench.has("Track"), "no region, nothing to track yet");
    bench.click("Track a region");
    let region = bench.state.region.expect("a region is being chosen");
    assert_eq!(region, TrackRegion::new(clip, asset));
    // The canvas is told of it while this section of this clip is shown.
    assert_eq!(bench.state.track_region(&bench.selection), Some(region));

    // Its numbers are fields: a drag of one changes the region alone.
    let end = bench.drag("Region width", vec2(40.0, 0.0));
    bench.release(end);
    let wider = bench.state.region.unwrap();
    assert!(wider.size[0] > region.size[0], "{wider:?}");
    assert_eq!(wider.size[1], region.size[1]);
    assert_eq!(bench.steps(), before, "a region is no part of the project");
    assert!(bench.draft.is_none());

    // What the canvas drags is taken, held inside the frame.
    bench.state.set_region(TrackRegion {
        center: [0.98, 0.5],
        size: [0.2, 0.3],
        ..wider
    });
    let held = bench.state.region.unwrap();
    assert!((held.center[0] - 0.9).abs() < 1e-6 && held.size == [0.2, 0.3]);

    bench.click("Track");
    assert_eq!(bench.tracks.len(), 1);
    let asked = &bench.tracks[0];
    assert_eq!(asked.region, held);
    assert_eq!((asked.region.clip, asked.region.asset), (clip, asset));
    // The frame under the playhead, to the end of what the clip shows.
    assert_eq!((asked.from, asked.until), (seconds(1.0), seconds(4.0)));
    assert_eq!(asked.name, "Tracker 1");
    assert!(bench.state.region.is_none(), "the region was handed over");
    assert_eq!(
        bench.steps(),
        before,
        "nothing is edited until a path comes"
    );
    assert!(bench.refused.is_empty());
}

#[test]
fn a_region_is_given_up_by_cancel_by_another_clip_and_out_of_its_section() {
    let (mut bench, clip) = moving();
    bench.click("Track a region");
    assert!(bench.state.region.is_some());
    bench.click("Cancel");
    assert!(bench.state.region.is_none());
    assert!(bench.tracks.is_empty());

    // Out of its section the canvas shows no region; back in it, it does.
    bench.click("Track a region");
    bench.state.section = Section::Video;
    assert!(bench.state.track_region(&bench.selection).is_none());
    bench.state.section = Section::Motion;
    assert!(bench.state.track_region(&bench.selection).is_some());
    // What an Escape does in the application.
    assert!(bench.state.end_region());
    assert!(!bench.state.end_region());

    // Another clip's Motion section drops the region of the first.
    bench.click("Track a region");
    let other = bench.add(TrackKind::Video, {
        let asset = bench
            .editor
            .project()
            .assets
            .values()
            .next()
            .unwrap()
            .clone();
        let mut other = Clip::from_asset(&asset, seconds(5.0));
        other.duration = seconds(2.0);
        other
    });
    bench.selection.only(other);
    assert!(bench.state.track_region(&bench.selection).is_none());
    bench.frame(Vec::new());
    assert!(bench.state.region.is_none());
    bench.selection.only(clip);
    bench.frame(Vec::new());
    assert!(bench.find("Track a region").is_some());
}

#[test]
fn track_waits_for_the_playhead_to_be_on_the_clip() {
    let (mut bench, _) = moving();
    bench.click("Track a region");
    bench.playhead = seconds(6.0);
    bench.frame(Vec::new());
    assert!(bench.find("Track").expect("the button is there").disabled);
    bench.click("Track");
    assert!(bench.tracks.is_empty());
    bench.playhead = seconds(2.5);
    bench.click("Track");
    assert_eq!(bench.tracks[0].from, seconds(2.5));
}

#[test]
fn a_region_is_begun_and_tracked_from_the_keyboard() {
    let (mut bench, _) = moving();
    bench.focus("Track a region");
    bench.enter(Modifiers::NONE);
    assert!(bench.state.region.is_some());
    // A number of it is typed.
    bench.focus("Region centre X");
    bench.frame(vec![Event::Text("25".to_owned())]);
    bench.enter(Modifiers::NONE);
    let region = bench.state.region.unwrap();
    assert!((region.center[0] - 0.25).abs() < 1e-6, "{region:?}");
    bench.focus("Track");
    bench.enter(Modifiers::NONE);
    assert_eq!(bench.tracks.len(), 1);
    assert_eq!(bench.tracks[0].region.center[0], region.center[0]);
}

#[test]
fn tracking_under_way_shows_how_far_it_is_and_can_be_given_up() {
    let (mut bench, clip) = moving();
    let asset = bench.clip(clip).asset_id().unwrap();
    bench.state.set_tracking(Some(TrackProgress {
        clip,
        asset,
        name: "Tracker 1".to_owned(),
        done: 0.42,
        waits: None,
    }));
    bench.frame(Vec::new());
    assert!(bench.has("Tracking Tracker 1") && bench.has("42 %"));
    assert!(bench.find("Tracking").is_some(), "the bar is named");
    assert!(
        bench.find("Track a region").is_none(),
        "one region at a time"
    );
    bench.click("Cancel tracking");
    assert_eq!(bench.cancels, 1);
    bench.state.set_tracking(None);
    bench.frame(Vec::new());
    assert!(bench.find("Track a region").is_some());
}

#[test]
fn only_a_video_is_offered_a_region_and_a_title_only_follows() {
    let mut bench = Bench::empty();
    let still = media("card.png", MediaKind::Image);
    bench.edit(Command::AddAssets {
        assets: vec![still.clone()],
    });
    let picture = bench.add(TrackKind::Video, {
        let mut clip = Clip::from_asset(&still, Time::ZERO);
        clip.duration = seconds(4.0);
        clip
    });
    bench.selection.only(picture);
    bench.state.section = Section::Motion;
    bench.frame(Vec::new());
    assert!(bench.has("Only a video has motion to track."));
    assert!(bench.find("Track a region").is_none());
    assert!(bench.has("Follow"));

    let (mut bench, _) = moving();
    titled(&mut bench);
    assert!(!bench.has("Track a region") && !bench.has("Only a video has motion to track."));
    assert!(bench.has("Follow"));
    // With no tracker there is nothing to follow, and nothing to list.
    assert!(!bench.has("Tracker to follow") && !bench.has("Trackers"));
}

#[test]
fn a_title_follows_a_tracker_from_where_it_is_and_stops() {
    let (mut bench, clip) = moving();
    let tracker = tracked(&mut bench, clip, "Ball");
    let title = titled(&mut bench);
    let before = bench.steps();
    let placed = bench.layer(title).transform.position;

    bench.click("Tracker to follow");
    bench.click("Ball");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Change tracking follow");
    // A second in, the point is at 0.3 across and a quarter down a canvas
    // of 1920 by 1080: the offset takes that back out.
    let follow = bench.clip(title).follow.clone().expect("it follows");
    assert_eq!(
        follow,
        Follow {
            tracker_id: tracker,
            position: true,
            scale: false,
            rotation: false,
            offset: [384.0, 270.0],
        }
    );
    let now = bench.layer(title).transform.position;
    assert!((now[0] - placed[0]).abs() < 0.5 && (now[1] - placed[1]).abs() < 0.5);
    // And it goes with the point from there.
    bench.playhead = seconds(2.0);
    let later = bench.layer(title).transform.position;
    assert!((later[0] - placed[0] - 384.0).abs() < 0.5, "{later:?}");
    bench.playhead = seconds(1.0);

    // The offset is dragged as one step, drawn while it is held.
    let end = bench.drag("Follow offset X", vec2(30.0, 0.0));
    let held = bench.layer(title).transform.position;
    assert!(held[0] > now[0] + 1.0, "the layer moves with the drag");
    assert_eq!(bench.steps(), before + 1, "a held offset is no step yet");
    bench.release(end);
    assert_eq!(bench.steps(), before + 2);
    let dragged = bench.clip(title).follow.clone().unwrap().offset;
    assert!(dragged[0] > 384.0 && dragged[1] == 270.0);
    let settled = bench.layer(title).transform.position;
    assert!(
        (settled[0] - held[0]).abs() < 0.5,
        "it stays where it was drawn"
    );

    // An Escape gives a held offset up.
    bench.drag("Follow offset Y", vec2(30.0, 0.0));
    bench.key(Key::Escape, Modifiers::NONE);
    let pointer = bench.at("Follow offset Y") + vec2(30.0, 0.0);
    bench.release(pointer);
    assert_eq!(bench.steps(), before + 2);
    assert_eq!(bench.clip(title).follow.clone().unwrap().offset, dragged);

    // Centred on the point, the layer is where the point is.
    bench.click("Centre on point");
    assert_eq!(bench.steps(), before + 3);
    let centred = bench.layer(title).transform.position;
    assert!(
        (centred[0] - (0.3 - 0.5) * 1920.0).abs() < 0.5,
        "{centred:?}"
    );
    assert!(
        (centred[1] - (0.25 - 0.5) * 1080.0).abs() < 0.5,
        "{centred:?}"
    );

    bench.click("Stop following");
    assert_eq!(bench.steps(), before + 4);
    assert!(bench.clip(title).follow.is_none());
    assert!(!bench.has("Follow offset X"));
    assert!(bench.editor.undo());
    assert!(bench.clip(title).follow.is_some());
    assert!(bench.refused.is_empty());
}

#[test]
fn following_is_chosen_and_ended_from_the_keyboard() {
    let (mut bench, clip) = moving();
    tracked(&mut bench, clip, "Ball");
    let title = titled(&mut bench);
    bench.focus("Tracker to follow");
    bench.enter(Modifiers::NONE);
    bench.focus("Ball");
    bench.enter(Modifiers::NONE);
    assert!(bench.clip(title).follow.is_some(), "{:?}", bench.refused);
    bench.focus("Follow offset X");
    bench.frame(vec![Event::Text("12".to_owned())]);
    bench.enter(Modifiers::NONE);
    assert_eq!(bench.clip(title).follow.clone().unwrap().offset[0], 12.0);
    bench.focus("Stop following");
    bench.enter(Modifiers::NONE);
    assert!(bench.clip(title).follow.is_none());
}

#[test]
fn a_follower_of_a_clip_that_is_off_screen_is_told_so() {
    let (mut bench, clip) = moving();
    tracked(&mut bench, clip, "Ball");
    // A title after the tracked clip has ended.
    let title = bench.add(
        TrackKind::Text,
        Clip::text("Late", seconds(5.0), seconds(2.0)),
    );
    bench.selection.only(title);
    bench.playhead = seconds(5.5);
    bench.frame(Vec::new());
    bench.click("Tracker to follow");
    bench.click("Ball");
    // With no point to read, it follows from no offset.
    assert_eq!(bench.clip(title).follow.clone().unwrap().offset, [0.0; 2]);
    bench.frame(Vec::new());
    assert!(bench.find("Centre on point").unwrap().disabled);
    assert!(
        bench
            .seen
            .iter()
            .any(|seen| seen.name.starts_with("The tracked clip is not on screen"))
    );
}

#[test]
fn a_tracker_is_renamed_by_leaving_its_field_and_not_by_an_escape() {
    let (mut bench, clip) = moving();
    let tracker = tracked(&mut bench, clip, "Tracker 1");
    let keys = bench.editor.project().trackers[&tracker].keys.clone();
    let before = bench.steps();
    bench.frame(Vec::new());
    assert!(bench.has("shot.mp4 · 4.00 s · 2 points"));

    let all = Modifiers::COMMAND;
    bench.focus("Name of Tracker 1");
    bench.key(Key::A, all);
    bench.frame(vec![Event::Text("Ball".to_owned())]);
    assert_eq!(bench.steps(), before, "a name being typed is no step");
    bench.enter(Modifiers::NONE);
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Set tracker");
    let renamed = &bench.editor.project().trackers[&tracker];
    assert_eq!(renamed.name, "Ball");
    assert_eq!(renamed.keys, keys, "the path is the one it had");

    // An Escape gives up what was typed.
    bench.focus("Name of Ball");
    bench.key(Key::A, all);
    bench.frame(vec![Event::Text("Other".to_owned())]);
    bench.key(Key::Escape, Modifiers::NONE);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.editor.project().trackers[&tracker].name, "Ball");
    // A name of nothing is not a name.
    bench.focus("Name of Ball");
    bench.key(Key::A, all);
    bench.key(Key::Backspace, Modifiers::NONE);
    bench.enter(Modifiers::NONE);
    assert_eq!(bench.steps(), before + 1);
    assert!(bench.refused.is_empty());

    // The next region made is named past the ones there are.
    bench.edit(Command::SetTracker {
        tracker: Tracker {
            id: Uuid::new_v4(),
            name: "Tracker 1".to_owned(),
            ..bench.editor.project().trackers[&tracker].as_ref().clone()
        },
    });
    bench.click("Track a region");
    bench.click("Track");
    assert_eq!(bench.tracks[0].name, "Tracker 2");
}

#[test]
fn removing_a_tracker_frees_what_followed_it_in_one_step() {
    let (mut bench, clip) = moving();
    let tracker = tracked(&mut bench, clip, "Ball");
    let title = titled(&mut bench);
    bench.click("Tracker to follow");
    bench.click("Ball");
    assert!(bench.clip(title).follow.is_some());
    let before = bench.steps();
    bench.click("Remove Ball");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Remove tracker");
    assert!(bench.editor.project().trackers.is_empty());
    assert!(bench.clip(title).follow.is_none());
    assert!(bench.draft.is_none());
    assert!(bench.editor.undo());
    assert!(bench.editor.project().trackers.contains_key(&tracker));
    assert!(bench.clip(title).follow.is_some());
}

#[test]
fn a_locked_track_takes_no_follow() {
    let (mut bench, clip) = moving();
    tracked(&mut bench, clip, "Ball");
    let title = titled(&mut bench);
    let track_id = bench.track(TrackKind::Text);
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    });
    bench.frame(Vec::new());
    assert!(bench.find("Tracker to follow").unwrap().disabled);
    // And what would ask for it anyway is refused by the project.
    let refused = bench.editor.dispatch(Command::SetFollow {
        clip_id: title,
        follow: Some(Follow {
            tracker_id: *bench.editor.project().trackers.keys().next().unwrap(),
            position: true,
            scale: false,
            rotation: false,
            offset: [0.0; 2],
        }),
    });
    assert!(refused.is_err());
    assert!(bench.clip(title).follow.is_none());
}

/// A second video clip on the bench, selected, on its Motion section.
fn another(bench: &mut Bench, make: impl FnOnce(&mut Clip)) -> ClipId {
    let asset = (bench.editor.project().assets.values().next())
        .expect("the bench has media")
        .clone();
    let mut clip = Clip::from_asset(&asset, seconds(5.0));
    clip.duration = seconds(2.0);
    make(&mut clip);
    let id = bench.add(TrackKind::Video, clip);
    bench.selection.only(id);
    bench.state.section = Section::Motion;
    bench.frame(Vec::new());
    id
}

#[test]
fn tracking_starts_on_the_frame_the_canvas_shows() {
    // Two frames trimmed off the head, and the playhead on the clip's second
    // frame: the source time comes out a microsecond short of the fourth
    // frame of the source, which is the one the canvas shows and the box is
    // drawn on.
    let (mut bench, _) = moving();
    let clip = another(&mut bench, |clip| clip.source_in = Time(66_666));
    bench.playhead = seconds(5.0) + Time(33_333);
    assert_eq!(bench.clip(clip).source_time(bench.playhead), Time(99_999));
    bench.click("Track a region");
    bench.click("Track");
    let asked = &bench.tracks[0];
    let rate = FrameRate::default();
    assert_eq!(asked.from, rate.frame_time(3));
    assert_eq!(rate.frame_at(asked.from), 3, "the worker reads that frame");
    assert_eq!(asked.until, Time(66_666) + seconds(2.0));
}

#[test]
fn a_clip_played_backwards_is_not_tracked_from_there() {
    use editor_core::{Ease, Retime, TimeKey};
    let (mut bench, _) = moving();
    let map = |keys: &[(f64, f64)]| Retime {
        keys: (keys.iter())
            .map(|(at, source)| TimeKey {
                at: seconds(*at),
                source: seconds(*source),
                ease: Ease::Linear,
            })
            .collect(),
    };
    // Reversed: what comes after the playhead is earlier in the source,
    // and frames are only read forwards.
    another(&mut bench, |clip| {
        clip.retime = Some(map(&[(0.0, 2.0), (2.0, 0.0)]));
    });
    bench.playhead = seconds(5.5);
    bench.click("Track a region");
    assert!(bench.find("Track").expect("the button is there").disabled);
    assert!(bench.has(
        "The clip plays backwards here, and tracking only runs forwards through the source. Put the playhead where the clip plays forwards."
    ));
    bench.click("Track");
    assert!(bench.tracks.is_empty());
    bench.click("Cancel");

    // Forwards and then back: tracked from the playhead to the furthest
    // the rest of the clip shows, which is where it turns.
    another(&mut bench, |clip| {
        clip.start = seconds(8.0);
        clip.retime = Some(map(&[(0.0, 0.0), (1.0, 3.0), (2.0, 1.0)]));
    });
    bench.playhead = seconds(8.5);
    bench.click("Track a region");
    bench.click("Track");
    let asked = &bench.tracks[0];
    assert_eq!((asked.from, asked.until), (seconds(1.5), seconds(3.0)));
    // On its way back it is not.
    bench.playhead = seconds(9.5);
    bench.click("Track a region");
    assert!(bench.find("Track").expect("the button is there").disabled);
}

#[test]
fn a_path_that_waits_is_added_or_given_up_by_the_person() {
    let (mut bench, clip) = moving();
    let asset = bench.clip(clip).asset_id().unwrap();
    let before = bench.steps();
    bench.state.set_tracking(Some(TrackProgress {
        clip,
        asset,
        name: "Tracker 1".to_owned(),
        done: 1.0,
        waits: Some("Adding it gives up what was undone. Redo first to keep both.".to_owned()),
    }));
    bench.frame(Vec::new());
    assert!(
        bench.has(
            "Tracker 1 is ready. Adding it gives up what was undone. Redo first to keep both."
        )
    );
    assert!(bench.find("Cancel tracking").is_none());
    assert!(bench.find("Track a region").is_none());
    bench.click("Add tracker");
    assert_eq!(bench.said, ["add tracked"]);
    bench.focus("Discard");
    bench.enter(Modifiers::NONE);
    assert_eq!(bench.cancels, 1);
    assert_eq!(bench.steps(), before, "the inspector edits nothing itself");
}

#[test]
fn a_follower_keeps_its_place_over_a_clip_that_does_not_fill_the_canvas() {
    // The tracked clip is a small picture in a corner, and steadied.
    let (mut bench, clip) = moving();
    let tracker = tracked(&mut bench, clip, "Ball");
    bench.edit(Command::SetTransform {
        clip_id: clip,
        transform: editor_core::Transform {
            position: [480.0, -270.0],
            scale: [0.5, 0.5],
            ..Default::default()
        },
    });
    let title = titled(&mut bench);
    let placed = bench.layer(title).transform.position;
    bench.click("Tracker to follow");
    bench.click("Ball");
    // A second in the point is 0.3 across and a quarter down a picture of
    // 960 by 540 centred on (480, -270).
    let follow = bench.clip(title).follow.clone().expect("it follows");
    assert_eq!(follow.tracker_id, tracker);
    assert_eq!(
        follow.offset,
        [-(480.0 - 0.2 * 960.0), 270.0 + 0.25 * 540.0]
    );
    let now = bench.layer(title).transform.position;
    assert!((now[0] - placed[0]).abs() < 0.5 && (now[1] - placed[1]).abs() < 0.5);
    bench.click("Centre on point");
    let centred = bench.layer(title).transform.position;
    assert!(
        (centred[0] - (480.0 - 0.2 * 960.0)).abs() < 0.5,
        "{centred:?}"
    );
    assert!(
        (centred[1] - (-270.0 - 0.25 * 540.0)).abs() < 0.5,
        "{centred:?}"
    );
    bench.click("Stop following");

    // Read enlarged 1.25 times, as the canvas draws it steadied, the point
    // is further from the picture's centre, and the offset takes that out.
    bench.state.set_readings(&[editor_core::ClipOverride {
        clip,
        reading: Some(editor_core::Reading {
            matrix: [0.8, 0.0, 0.0, 0.8],
            offset: [0.0, 0.0],
        }),
        ..Default::default()
    }]);
    bench.click("Tracker to follow");
    bench.click("Ball");
    let follow = bench.clip(title).follow.clone().expect("it follows");
    // Offsets are whole canvas pixels.
    assert_eq!(
        follow.offset,
        [
            -(480.0 - 0.25 * 960.0),
            (270.0_f32 + 0.3125 * 540.0).round()
        ]
    );
    assert!(bench.refused.is_empty());
}
