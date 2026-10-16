use editor_core::{
    AnimTrack, Asset, Clip, ClipId, Command, Ease, EditError, Editor, Effect, EffectKind, Keyframe,
    MediaKind, ParamKey, ParamValue, Project, Retime, Time, TimeKey, Track, TrackKind, look,
    sound::{self, Sounding},
};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

const VIDEO: usize = 0;
const AUDIO: usize = 1;
const TEXT: usize = 2;

const EASE: Ease = Ease::Bezier {
    x1: 0.42,
    y1: 0.0,
    x2: 0.58,
    y2: 1.0,
};

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

/// The media a project can hold a clip of: a video with sound, a video
/// without, a still and a song, 60 s each.
struct Media {
    shot: Asset,
    silent: Asset,
    still: Asset,
    song: Asset,
}

fn library() -> (Project, Media) {
    let asset = |path: &str, kind| Asset::new(PathBuf::from(path), kind, seconds(60.0));
    let mut shot = asset("shot.mp4", MediaKind::Video);
    shot.has_audio = true;
    let media = Media {
        shot,
        silent: asset("silent.mp4", MediaKind::Video),
        still: asset("still.png", MediaKind::Image),
        song: asset("song.wav", MediaKind::Audio),
    };
    let mut project = Project::new("Sound");
    let assets = Arc::make_mut(&mut project.assets);
    for asset in [&media.shot, &media.silent, &media.still, &media.song] {
        assets.insert(asset.id, asset.clone());
    }
    (project, media)
}

/// A 10 s clip of `asset` starting at 2 s from source 5 s.
fn clip_of(asset: &Asset) -> Clip {
    let mut clip = Clip::from_asset(asset, seconds(2.0));
    clip.source_in = seconds(5.0);
    clip.duration = seconds(10.0);
    clip
}

fn place(project: &mut Project, track: usize, clip: Clip) -> ClipId {
    let id = clip.id;
    Arc::make_mut(&mut project.tracks[track]).clips.push(clip);
    id
}

fn track_mut(project: &mut Project, track: usize) -> &mut Track {
    Arc::make_mut(&mut project.tracks[track])
}

struct Fixture {
    editor: Editor,
    /// A clip of the video with sound, on the video track.
    picture: ClipId,
    /// A clip of the song, on the audio track.
    song: ClipId,
    silent: ClipId,
    still: ClipId,
    title: ClipId,
}

/// One clip of each kind, 10 s long, the pictures one after another.
fn fixture() -> Fixture {
    let (mut project, media) = library();
    let picture = place(&mut project, VIDEO, clip_of(&media.shot));
    let mut silent = clip_of(&media.silent);
    silent.start = seconds(20.0);
    let silent = place(&mut project, VIDEO, silent);
    let mut still = clip_of(&media.still);
    still.start = seconds(40.0);
    let still = place(&mut project, VIDEO, still);
    let song = place(&mut project, AUDIO, clip_of(&media.song));
    let title = place(
        &mut project,
        TEXT,
        Clip::text("Title", seconds(2.0), seconds(10.0)),
    );
    Fixture {
        editor: Editor::new(project).unwrap(),
        picture,
        song,
        silent,
        still,
        title,
    }
}

fn clip(editor: &Editor, id: ClipId) -> &Clip {
    editor.project().clip(id).unwrap().1
}

fn fades(editor: &Editor, id: ClipId) -> (Time, Time) {
    let clip = clip(editor, id);
    (clip.fade_in, clip.fade_out)
}

fn set_fades(clip_id: ClipId, fade_in: f64, fade_out: f64) -> Command {
    Command::SetFades {
        clip_id,
        fade_in: seconds(fade_in),
        fade_out: seconds(fade_out),
    }
}

fn track_state(editor: &Editor, track: usize) -> Command {
    Command::SetTrackState {
        track_id: editor.project().tracks[track].id,
        locked: None,
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    }
}

fn track_gain(editor: &Editor, track: usize, value: f32) -> Command {
    let Command::SetTrackState {
        track_id,
        locked,
        hidden,
        muted,
        name,
        solo,
        ..
    } = track_state(editor, track)
    else {
        unreachable!()
    };
    Command::SetTrackState {
        track_id,
        locked,
        hidden,
        muted,
        name,
        gain: Some(value),
        solo,
    }
}

fn trim(clip_id: ClipId, source_in: f64, duration: f64) -> Command {
    Command::TrimClip {
        clip_id,
        source_in: seconds(source_in),
        duration: seconds(duration),
        ripple: false,
    }
}

fn volume_keys(values: &[(f64, f32)]) -> AnimTrack {
    AnimTrack {
        target: None,
        key: ParamKey::Gain,
        keys: values
            .iter()
            .map(|(at, value)| Keyframe {
                at: seconds(*at),
                value: *value,
                ease: Ease::Linear,
            })
            .collect(),
    }
}

/// Refused with `what`, and the project, its revision and its history are
/// the ones from before.
#[track_caller]
fn refused(editor: &mut Editor, command: Command, what: &str) {
    let (before, revision) = (editor.snapshot(), editor.revision());
    match editor.dispatch(command) {
        Err(EditError::Invalid(reason)) => {
            assert_eq!(reason.split(" (clip ").next().unwrap(), what)
        }
        other => panic!("expected Invalid({what}), got {other:?}"),
    }
    assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    assert_eq!(editor.revision(), revision);
    assert!(!editor.can_undo());
}

#[track_caller]
fn invalid(project: &Project, what: &str) {
    match project.validate() {
        Err(EditError::Invalid(reason)) => {
            assert_eq!(reason.split(" (clip ").next().unwrap(), what)
        }
        other => panic!("expected Invalid({what}), got {other:?}"),
    }
}

#[test]
fn track_volume_is_finite_and_within_the_clip_volume_bound() {
    let (project, _) = library();
    for gain in [0.0, 1.0, 8.0] {
        let mut project = project.clone();
        track_mut(&mut project, AUDIO).gain = gain;
        project.validate().unwrap();
    }
    for gain in [
        -0.001,
        8.001,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MAX,
    ] {
        let mut project = project.clone();
        track_mut(&mut project, AUDIO).gain = gain;
        invalid(&project, "track volume");
    }
    // A text track's volume is bound like any other, though nothing reads it.
    let mut project = project.clone();
    track_mut(&mut project, TEXT).gain = 9.0;
    invalid(&project, "track volume");
}

#[test]
fn fades_are_non_negative_and_together_no_longer_than_the_clip() {
    let (mut project, media) = library();
    place(&mut project, AUDIO, clip_of(&media.song));
    let with = |fade_in: Time, fade_out: Time| {
        let mut project = project.clone();
        let clip = &mut track_mut(&mut project, AUDIO).clips[0];
        (clip.fade_in, clip.fade_out) = (fade_in, fade_out);
        project
    };
    for (fade_in, fade_out) in [(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (4.0, 6.0)] {
        with(seconds(fade_in), seconds(fade_out))
            .validate()
            .unwrap();
    }
    for (fade_in, fade_out) in [
        (Time(-1), Time::ZERO),
        (Time::ZERO, Time(-1)),
        (Time(-1), Time(1)),
        (seconds(10.0) + Time(1), Time::ZERO),
        (Time::ZERO, seconds(10.0) + Time(1)),
        (seconds(4.0), seconds(6.0) + Time(1)),
        (Time(i64::MAX), Time(i64::MAX)),
        (Time(i64::MIN), Time(i64::MAX)),
    ] {
        invalid(&with(fade_in, fade_out), "fades");
    }
}

#[test]
fn set_track_state_refuses_a_volume_out_of_bounds_and_changes_nothing() {
    let mut f = fixture();
    for gain in [-0.5, 8.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let command = track_gain(&f.editor, AUDIO, gain);
        refused(&mut f.editor, command, "track volume");
    }
}

#[test]
fn set_fades_is_refused_where_nothing_sounds_and_changes_nothing() {
    let mut f = fixture();
    for id in [f.title, f.still, f.silent] {
        refused(&mut f.editor, set_fades(id, 1.0, 0.0), "fades");
        refused(&mut f.editor, set_fades(id, 0.0, 1.0), "fades");
    }
    // Out of bounds on a clip that does sound.
    refused(&mut f.editor, set_fades(f.song, -1.0, 0.0), "fades");
    refused(&mut f.editor, set_fades(f.song, 0.0, -1.0), "fades");
    refused(&mut f.editor, set_fades(f.song, 6.0, 4.5), "fades");
    assert!(matches!(
        f.editor.dispatch(set_fades(Uuid::new_v4(), 1.0, 1.0)),
        Err(EditError::Missing(_))
    ));
    // A picture with sound fades, and so does a song.
    f.editor.dispatch(set_fades(f.picture, 1.0, 2.0)).unwrap();
    f.editor.dispatch(set_fades(f.song, 4.0, 6.0)).unwrap();
    assert_eq!(fades(&f.editor, f.picture), (seconds(1.0), seconds(2.0)));
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(6.0)));
}

#[test]
fn a_batch_that_ends_in_a_refused_fade_leaves_the_project_unchanged() {
    let mut f = fixture();
    let command = Command::Batch {
        label: "Fade everything".into(),
        commands: vec![
            set_fades(f.song, 1.0, 1.0),
            track_gain(&f.editor, AUDIO, 0.5),
            set_fades(f.title, 1.0, 0.0),
        ],
    };
    refused(&mut f.editor, command, "fades");
    assert_eq!(fades(&f.editor, f.song), (Time::ZERO, Time::ZERO));
    assert_eq!(f.editor.project().tracks[AUDIO].gain, 1.0);
}

#[test]
fn set_fades_does_not_follow_links() {
    let mut f = fixture();
    f.editor
        .dispatch(Command::LinkClips {
            clip_ids: vec![f.picture, f.song],
        })
        .unwrap();
    f.editor.dispatch(set_fades(f.song, 1.0, 2.0)).unwrap();
    assert_eq!(fades(&f.editor, f.song), (seconds(1.0), seconds(2.0)));
    assert_eq!(fades(&f.editor, f.picture), (Time::ZERO, Time::ZERO));
}

#[test]
fn undo_and_redo_restore_fades_track_volume_and_solo() {
    let mut f = fixture();
    let before = f.editor.snapshot();
    let outcome = f.editor.dispatch(set_fades(f.song, 1.0, 2.0)).unwrap();
    assert_eq!(outcome.label, "Change fades");
    let Command::SetTrackState { track_id, .. } = track_state(&f.editor, AUDIO) else {
        unreachable!()
    };
    let outcome = f
        .editor
        .dispatch(Command::SetTrackState {
            track_id,
            locked: None,
            hidden: None,
            muted: None,
            name: None,
            gain: Some(0.25),
            solo: Some(true),
        })
        .unwrap();
    assert_eq!(outcome.label, "Change track");
    let track = &f.editor.project().tracks[AUDIO];
    assert_eq!((track.gain, track.solo), (0.25, true));
    // The fields a command leaves out are left as they were.
    assert!(!track.muted && !track.locked && !track.hidden);
    assert_eq!(track.name, "Audio 1");

    let after = f.editor.snapshot();
    assert!(f.editor.undo());
    let track = &f.editor.project().tracks[AUDIO];
    assert_eq!((track.gain, track.solo), (1.0, false));
    assert_eq!(fades(&f.editor, f.song), (seconds(1.0), seconds(2.0)));
    assert!(f.editor.undo());
    assert_eq!(fades(&f.editor, f.song), (Time::ZERO, Time::ZERO));
    assert!(Arc::ptr_eq(&before, &f.editor.snapshot()));
    assert!(f.editor.redo() && f.editor.redo());
    assert!(Arc::ptr_eq(&after, &f.editor.snapshot()));

    // Setting what is already there is no edit.
    let revision = f.editor.revision();
    let outcome = f.editor.dispatch(set_fades(f.song, 1.0, 2.0)).unwrap();
    assert!(!outcome.changed);
    assert_eq!(f.editor.revision(), revision);
}

#[test]
fn a_locked_track_refuses_fades_and_accepts_its_volume_and_solo() {
    let mut f = fixture();
    let Command::SetTrackState { track_id, .. } = track_state(&f.editor, AUDIO) else {
        unreachable!()
    };
    let state = |locked, gain, solo| Command::SetTrackState {
        track_id,
        locked,
        hidden: None,
        muted: None,
        name: None,
        gain,
        solo,
    };
    f.editor.dispatch(state(Some(true), None, None)).unwrap();
    let before = f.editor.snapshot();
    assert!(matches!(
        f.editor.dispatch(set_fades(f.song, 1.0, 1.0)),
        Err(EditError::Locked)
    ));
    assert!(Arc::ptr_eq(&before, &f.editor.snapshot()));
    f.editor
        .dispatch(state(None, Some(2.0), Some(true)))
        .unwrap();
    let track = &f.editor.project().tracks[AUDIO];
    assert!(track.locked && track.solo);
    assert_eq!(track.gain, 2.0);
}

#[test]
fn an_end_trim_cuts_the_fade_out_to_what_the_fade_in_leaves() {
    let mut f = fixture();
    f.editor.dispatch(set_fades(f.song, 4.0, 5.0)).unwrap();
    f.editor.dispatch(trim(f.song, 5.0, 6.0)).unwrap();
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(2.0)));
    f.editor.dispatch(trim(f.song, 5.0, 3.0)).unwrap();
    assert_eq!(fades(&f.editor, f.song), (seconds(3.0), Time::ZERO));
    // Lengthening the clip again brings nothing back.
    f.editor.dispatch(trim(f.song, 5.0, 10.0)).unwrap();
    assert_eq!(fades(&f.editor, f.song), (seconds(3.0), Time::ZERO));
    assert!(f.editor.undo() && f.editor.undo() && f.editor.undo());
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(5.0)));
}

#[test]
fn a_start_trim_keeps_the_fade_in_on_the_new_edge_and_clamps_both() {
    let mut f = fixture();
    f.editor.dispatch(set_fades(f.song, 2.0, 5.0)).unwrap();
    // Two seconds off the start: both fades still fit.
    f.editor.dispatch(trim(f.song, 7.0, 8.0)).unwrap();
    let clip = clip(&f.editor, f.song);
    assert_eq!(clip.start, seconds(4.0));
    assert_eq!((clip.fade_in, clip.fade_out), (seconds(2.0), seconds(5.0)));
    assert_eq!(sound::fade_at(clip, Time::ZERO), 0.0);
    // Four more: the fade in is kept whole and the fade out gives way.
    f.editor.dispatch(trim(f.song, 11.0, 4.0)).unwrap();
    assert_eq!(fades(&f.editor, f.song), (seconds(2.0), seconds(2.0)));
    f.editor.dispatch(trim(f.song, 14.0, 1.0)).unwrap();
    assert_eq!(fades(&f.editor, f.song), (seconds(1.0), Time::ZERO));
}

#[test]
fn a_trim_clamps_the_fades_of_each_linked_clip() {
    let mut f = fixture();
    f.editor
        .dispatch(Command::LinkClips {
            clip_ids: vec![f.picture, f.song],
        })
        .unwrap();
    f.editor.dispatch(set_fades(f.picture, 1.0, 1.0)).unwrap();
    f.editor.dispatch(set_fades(f.song, 4.0, 6.0)).unwrap();
    f.editor.dispatch(trim(f.picture, 5.0, 5.0)).unwrap();
    assert_eq!(clip(&f.editor, f.song).duration, seconds(5.0));
    assert_eq!(fades(&f.editor, f.picture), (seconds(1.0), seconds(1.0)));
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(1.0)));
}

#[test]
fn a_rate_change_keeps_the_fades_lengths_and_clamps_them() {
    let mut f = fixture();
    f.editor.dispatch(set_fades(f.song, 4.0, 5.0)).unwrap();
    let rate = |clip_id, rate| Command::SetPlaybackRate { clip_id, rate };
    // Half speed: the clip is 20 s and the fades are as long as they were.
    f.editor.dispatch(rate(f.song, 0.5)).unwrap();
    assert_eq!(clip(&f.editor, f.song).duration, seconds(20.0));
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(5.0)));
    // Double speed: 5 s, which the fade in takes most of.
    f.editor.dispatch(rate(f.song, 2.0)).unwrap();
    assert_eq!(clip(&f.editor, f.song).duration, seconds(5.0));
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(1.0)));
    f.editor.dispatch(rate(f.song, 4.0)).unwrap();
    assert_eq!(clip(&f.editor, f.song).duration, seconds(2.5));
    assert_eq!(fades(&f.editor, f.song), (seconds(2.5), Time::ZERO));
    assert!(f.editor.undo() && f.editor.undo() && f.editor.undo());
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(5.0)));
}

#[test]
fn a_retime_that_shortens_the_clip_clamps_its_fades() {
    let mut f = fixture();
    f.editor.dispatch(set_fades(f.song, 4.0, 5.0)).unwrap();
    let key = |at: f64, source: f64| TimeKey {
        at: seconds(at),
        source: seconds(source),
        ease: Ease::Linear,
    };
    f.editor
        .dispatch(Command::SetRetime {
            clip_id: f.song,
            retime: Some(Retime {
                keys: vec![key(0.0, 0.0), key(6.0, 10.0)],
            }),
            duration: seconds(6.0),
        })
        .unwrap();
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(2.0)));
    // Clearing the map at a shorter length clamps again.
    f.editor
        .dispatch(Command::SetRetime {
            clip_id: f.song,
            retime: None,
            duration: seconds(3.0),
        })
        .unwrap();
    assert_eq!(fades(&f.editor, f.song), (seconds(3.0), Time::ZERO));
}

#[test]
fn a_split_leaves_each_fade_on_its_outer_edge_and_none_on_the_cut() {
    let mut f = fixture();
    f.editor.dispatch(set_fades(f.song, 2.0, 3.0)).unwrap();
    let before = f.editor.snapshot();
    f.editor
        .dispatch(Command::SplitClip {
            clip_id: f.song,
            at: seconds(7.0),
        })
        .unwrap();
    let clips = &f.editor.project().tracks[AUDIO].clips;
    assert_eq!(clips.len(), 2);
    let (left, right) = (&clips[0], &clips[1]);
    assert_eq!(left.id, f.song);
    assert_eq!((left.fade_in, left.fade_out), (seconds(2.0), Time::ZERO));
    assert_eq!((right.fade_in, right.fade_out), (Time::ZERO, seconds(3.0)));
    // Nothing dips at the cut: the level is whole on both sides of it.
    assert_eq!(sound::fade_at(left, left.duration), 1.0);
    assert_eq!(sound::fade_at(right, Time::ZERO), 1.0);
    assert!(f.editor.undo());
    assert!(Arc::ptr_eq(&before, &f.editor.snapshot()));
}

#[test]
fn a_split_inside_a_fade_cuts_it_to_its_half() {
    let mut f = fixture();
    f.editor.dispatch(set_fades(f.song, 2.0, 3.0)).unwrap();
    // One second in, inside the fade in.
    f.editor
        .dispatch(Command::SplitClip {
            clip_id: f.song,
            at: seconds(3.0),
        })
        .unwrap();
    let clips = &f.editor.project().tracks[AUDIO].clips;
    let (left, right) = (&clips[0], &clips[1]);
    assert_eq!((left.fade_in, left.fade_out), (seconds(1.0), Time::ZERO));
    assert_eq!((right.fade_in, right.fade_out), (Time::ZERO, seconds(3.0)));
    // Two seconds from the end, inside the fade out of the right half.
    let right = right.id;
    f.editor
        .dispatch(Command::SplitClip {
            clip_id: right,
            at: seconds(10.0),
        })
        .unwrap();
    let clips = &f.editor.project().tracks[AUDIO].clips;
    let (middle, last) = (&clips[1], &clips[2]);
    assert_eq!((middle.fade_in, middle.fade_out), (Time::ZERO, Time::ZERO));
    assert_eq!((last.fade_in, last.fade_out), (Time::ZERO, seconds(2.0)));
}

/// The picture and the song in one link, the picture fading 1 s at each edge
/// and the song 4 s in and 6 s out.
fn linked_with_fades() -> Fixture {
    let mut f = fixture();
    f.editor
        .dispatch(Command::LinkClips {
            clip_ids: vec![f.picture, f.song],
        })
        .unwrap();
    f.editor.dispatch(set_fades(f.picture, 1.0, 1.0)).unwrap();
    f.editor.dispatch(set_fades(f.song, 4.0, 6.0)).unwrap();
    f
}

#[test]
fn a_split_keeps_the_outer_fades_of_each_linked_clip() {
    let mut f = linked_with_fades();
    // Named by the picture, 5 s into both clips.
    f.editor
        .dispatch(Command::SplitClip {
            clip_id: f.picture,
            at: seconds(7.0),
        })
        .unwrap();
    assert_eq!(fades(&f.editor, f.picture), (seconds(1.0), Time::ZERO));
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), Time::ZERO));
    let right = |track: usize| {
        let clips = &f.editor.project().tracks[track].clips;
        let mut halves = clips.iter().filter(|clip| clip.start == seconds(7.0));
        let half = halves.next().unwrap();
        assert!(halves.next().is_none());
        assert_eq!(half.duration, seconds(5.0));
        half
    };
    let (picture, song) = (right(VIDEO), right(AUDIO));
    assert_eq!(
        (picture.fade_in, picture.fade_out),
        (Time::ZERO, seconds(1.0))
    );
    // Six seconds of fade out do not fit the 5 s half.
    assert_eq!((song.fade_in, song.fade_out), (Time::ZERO, seconds(5.0)));
    assert!(song.linked_id.is_some() && song.linked_id == picture.linked_id);
    assert!(f.editor.undo());
    assert_eq!(fades(&f.editor, f.picture), (seconds(1.0), seconds(1.0)));
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(6.0)));
}

#[test]
fn a_rate_change_clamps_the_fades_of_each_linked_clip() {
    let mut f = linked_with_fades();
    f.editor
        .dispatch(Command::SetPlaybackRate {
            clip_id: f.picture,
            rate: 2.0,
        })
        .unwrap();
    assert_eq!(clip(&f.editor, f.picture).duration, seconds(5.0));
    assert_eq!(clip(&f.editor, f.song).duration, seconds(5.0));
    assert_eq!(fades(&f.editor, f.picture), (seconds(1.0), seconds(1.0)));
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(1.0)));
}

#[test]
fn a_retime_clamps_the_fades_of_each_linked_clip() {
    let mut f = linked_with_fades();
    let key = |at: f64, source: f64| TimeKey {
        at: seconds(at),
        source: seconds(source),
        ease: Ease::Linear,
    };
    f.editor
        .dispatch(Command::SetRetime {
            clip_id: f.picture,
            retime: Some(Retime {
                keys: vec![key(0.0, 0.0), key(6.0, 10.0)],
            }),
            duration: seconds(6.0),
        })
        .unwrap();
    assert_eq!(clip(&f.editor, f.picture).duration, seconds(6.0));
    assert_eq!(clip(&f.editor, f.song).duration, seconds(6.0));
    assert_eq!(fades(&f.editor, f.picture), (seconds(1.0), seconds(1.0)));
    assert_eq!(fades(&f.editor, f.song), (seconds(4.0), seconds(2.0)));
}

#[test]
fn a_pasted_clip_keeps_its_fades() {
    let mut f = fixture();
    f.editor.dispatch(set_fades(f.song, 1.0, 2.0)).unwrap();
    let clipboard = f.editor.copy(&[f.song]).unwrap();
    f.editor.paste(&clipboard, seconds(30.0), None).unwrap();
    let pasted = &f.editor.project().tracks[AUDIO].clips[1];
    assert_ne!(pasted.id, f.song);
    assert_eq!(
        (pasted.fade_in, pasted.fade_out),
        (seconds(1.0), seconds(2.0))
    );
}

#[test]
fn the_fade_curve_is_equal_power_from_each_edge() {
    let (_, media) = library();
    let mut clip = clip_of(&media.song);
    // Without fades the level is whole from the first sample to the last.
    for local in [-1.0, 0.0, 5.0, 10.0, 11.0] {
        assert_eq!(sound::fade_at(&clip, seconds(local)), 1.0);
    }
    assert!(!sound::level_varies(&clip));
    clip.fade_in = seconds(2.0);
    clip.fade_out = seconds(4.0);
    assert!(sound::level_varies(&clip));
    let half = std::f32::consts::FRAC_PI_4.sin();
    let near = |local: f64, expected: f32| {
        let level = sound::fade_at(&clip, seconds(local));
        assert!(
            (level - expected).abs() < 1e-6,
            "{level} at {local} s, expected {expected}"
        );
    };
    assert_eq!(sound::fade_at(&clip, Time::ZERO), 0.0);
    near(1.0, half);
    assert_eq!(sound::fade_at(&clip, seconds(2.0)), 1.0);
    assert_eq!(sound::fade_at(&clip, seconds(2.0) + Time(1)), 1.0);
    assert_eq!(sound::fade_at(&clip, seconds(6.0)), 1.0);
    near(8.0, half);
    assert_eq!(sound::fade_at(&clip, seconds(10.0)), 0.0);
    // Nothing sounds outside the clip's faded edges.
    assert_eq!(sound::fade_at(&clip, seconds(-1.0)), 0.0);
    assert_eq!(sound::fade_at(&clip, seconds(11.0)), 0.0);
    // A quarter of the way the two halves of a crossing pair keep their power.
    let (up, down) = (
        sound::fade_at(&clip, seconds(0.5)),
        sound::fade_at(&clip, seconds(7.0)),
    );
    near(0.5, (std::f32::consts::PI / 8.0).sin());
    assert!((up * up + down * down - 1.0).abs() < 1e-6);
    // Fades that meet leave one instant at most at full level.
    clip.fade_in = seconds(5.0);
    clip.fade_out = seconds(5.0);
    assert_eq!(sound::fade_at(&clip, seconds(5.0)), 1.0);
    assert!(sound::fade_at(&clip, seconds(4.9)) < 1.0);
    assert!(sound::fade_at(&clip, seconds(5.1)) < 1.0);
}

#[test]
fn the_level_is_track_volume_times_clip_volume_times_the_fade() {
    let (_, media) = library();
    let mut track = Track::new("Music", TrackKind::Audio);
    let mut clip = clip_of(&media.song);
    let at = |local: f64| clip_of(&media.song).start + seconds(local);
    assert_eq!(sound::level_at(&track, &clip, at(3.0)), 1.0);

    track.gain = 0.5;
    clip.gain = 0.8;
    assert_eq!(sound::level_at(&track, &clip, at(3.0)), 0.5 * 0.8);
    clip.fade_in = seconds(2.0);
    let half = std::f32::consts::FRAC_PI_4.sin();
    assert!((sound::level_at(&track, &clip, at(1.0)) - 0.5 * 0.8 * half).abs() < 1e-6);
    assert_eq!(sound::level_at(&track, &clip, at(0.0)), 0.0);
    assert_eq!(sound::level_at(&track, &clip, at(3.0)), 0.5 * 0.8);

    // Keys take the place of the stored volume: 0 at the start, 2 from 4 s on.
    clip.animation = vec![volume_keys(&[(0.0, 0.0), (4.0, 2.0)])];
    assert!(sound::level_varies(&clip));
    assert!((sound::level_at(&track, &clip, at(1.0)) - 0.5 * 0.5 * half).abs() < 1e-6);
    assert!((sound::level_at(&track, &clip, at(2.0)) - 0.5 * 1.0).abs() < 1e-6);
    assert_eq!(sound::level_at(&track, &clip, at(6.0)), 0.5 * 2.0);
    clip.fade_in = Time::ZERO;
    assert!(sound::level_varies(&clip));
    clip.animation.clear();
    assert!(!sound::level_varies(&clip));
    track.gain = 0.0;
    assert_eq!(sound::level_at(&track, &clip, at(6.0)), 0.0);
}

const V1_TRACK: &str = r#"{
    "id": "00000000-0000-4000-8000-000000000003",
    "name": "Audio 1",
    "kind": "audio",
    "locked": false,
    "hidden": false,
    "muted": true,
    "clips": []
}"#;

const V1_CLIP: &str = r#"{
    "id": "00000000-0000-4000-8000-000000000004",
    "name": "song.wav",
    "content": { "type": "media", "asset_id": "00000000-0000-4000-8000-000000000002" },
    "start": 0,
    "source_in": 1000000,
    "duration": 10000000,
    "playback_rate": 1.0,
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
    "gain": 0.5
}"#;

/// The version 1 clip with what version 2 added to an audio clip: volume
/// keys and a time map.
fn v2_clip() -> serde_json::Value {
    let mut clip: serde_json::Value = serde_json::from_str(V1_CLIP).unwrap();
    clip["animation"] = serde_json::json!([{
        "target": null,
        "key": "gain",
        "keys": [
            { "at": 0, "value": 0.0, "ease": { "type": "linear" } },
            { "at": 10000000, "value": 1.0, "ease": { "type": "linear" } }
        ]
    }]);
    clip["retime"] = serde_json::json!({ "keys": [
        { "at": 0, "source": 0, "ease": { "type": "linear" } },
        { "at": 10000000, "source": 10000000, "ease": { "type": "linear" } }
    ]});
    clip
}

#[test]
fn older_tracks_and_clips_decode_with_defaults_and_reserialize_unchanged() {
    let track: Track = serde_json::from_str(V1_TRACK).unwrap();
    assert_eq!((track.gain, track.solo), (1.0, false));
    assert!(track.muted);
    let fixture: serde_json::Value = serde_json::from_str(V1_TRACK).unwrap();
    assert_eq!(serde_json::to_value(&track).unwrap(), fixture);

    let v1: serde_json::Value = serde_json::from_str(V1_CLIP).unwrap();
    for fixture in [v1, v2_clip()] {
        let clip: Clip = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!((clip.fade_in, clip.fade_out), (Time::ZERO, Time::ZERO));
        assert_eq!(clip.gain, 0.5);
        assert_eq!(serde_json::to_value(&clip).unwrap(), fixture);
    }
    let v2: Clip = serde_json::from_value(v2_clip()).unwrap();
    assert!(v2.retime.is_some() && sound::level_varies(&v2));
}

#[test]
fn a_default_track_and_clip_emit_no_new_key_and_set_ones_round_trip() {
    let f = fixture();
    let encoded = serde_json::to_value(f.editor.project()).unwrap();
    for track in [VIDEO, AUDIO, TEXT] {
        let track = &encoded["tracks"][track];
        let keys: Vec<_> = track.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            ["clips", "hidden", "id", "kind", "locked", "muted", "name"]
        );
        for clip in track["clips"].as_array().unwrap() {
            let clip = clip.as_object().unwrap();
            assert!(!clip.contains_key("fade_in") && !clip.contains_key("fade_out"));
            assert_eq!(clip.len(), 11);
        }
    }

    let mut project = f.editor.project().clone();
    let track = track_mut(&mut project, AUDIO);
    (track.gain, track.solo) = (0.0, true);
    (track.clips[0].fade_in, track.clips[0].fade_out) = (seconds(1.0), seconds(2.5));
    project.validate().unwrap();
    let encoded = serde_json::to_value(&project).unwrap();
    let track = &encoded["tracks"][AUDIO];
    assert_eq!(track["gain"], 0.0);
    assert_eq!(track["solo"], true);
    assert_eq!(track["clips"][0]["fade_in"], 1_000_000);
    assert_eq!(track["clips"][0]["fade_out"], 2_500_000);
    let back: Project = serde_json::from_value(encoded).unwrap();
    assert_eq!(back, project);
}

#[test]
fn serialized_commands_carry_fades_track_volume_and_solo() {
    let mut f = fixture();
    let track_id = f.editor.project().tracks[AUDIO].id;
    let fades: Command = serde_json::from_value(serde_json::json!({
        "type": "set_fades",
        "clip_id": f.song,
        "fade_in": 500_000,
        "fade_out": 1_500_000
    }))
    .unwrap();
    f.editor.dispatch(fades).unwrap();
    assert_eq!(fades_of(&f), (seconds(0.5), seconds(1.5)));
    // A command written before the fields existed still reads, and sets neither.
    let older: Command = serde_json::from_value(serde_json::json!({
        "type": "set_track_state",
        "track_id": track_id,
        "locked": null,
        "hidden": null,
        "muted": true,
        "name": null
    }))
    .unwrap();
    f.editor.dispatch(older).unwrap();
    let track = &f.editor.project().tracks[AUDIO];
    assert!(track.muted);
    assert_eq!((track.gain, track.solo), (1.0, false));
    let newer: Command = serde_json::from_value(serde_json::json!({
        "type": "set_track_state",
        "track_id": track_id,
        "locked": null,
        "hidden": null,
        "muted": null,
        "name": null,
        "gain": 0.5,
        "solo": true
    }))
    .unwrap();
    let text = serde_json::to_value(&newer).unwrap();
    assert_eq!(text["gain"], 0.5);
    assert_eq!(text["solo"], true);
    f.editor.dispatch(newer).unwrap();
    let track = &f.editor.project().tracks[AUDIO];
    assert!(track.muted && track.solo);
    assert_eq!(track.gain, 0.5);
}

fn fades_of(f: &Fixture) -> (Time, Time) {
    fades(&f.editor, f.song)
}

/// `sound::sounds` for the clip, which the many-clip route must agree with.
#[track_caller]
fn sounds(project: &Project, id: ClipId) -> bool {
    project.validate().unwrap();
    let (track, clip) = project.clip(id).unwrap();
    let answer = sound::sounds(project, track, clip);
    let sounding = Sounding::of(project);
    assert_eq!(sounding.sounds(project, track, clip), answer);
    assert_eq!(sounding.owned(track, clip), sound::owned(project, clip));
    answer
}

/// A picture with sound on the video track and a song on the audio track.
fn pair() -> (Project, Media, ClipId, ClipId) {
    let (mut project, media) = library();
    let picture = place(&mut project, VIDEO, clip_of(&media.shot));
    let song = place(&mut project, AUDIO, clip_of(&media.song));
    (project, media, picture, song)
}

#[test]
fn clips_with_sound_on_open_tracks_sound() {
    let (project, _, picture, song) = pair();
    assert!(sounds(&project, picture) && sounds(&project, song));
}

#[test]
fn a_muted_track_does_not_sound() {
    let (mut project, _, picture, song) = pair();
    track_mut(&mut project, AUDIO).muted = true;
    assert!(!sounds(&project, song));
    assert!(sounds(&project, picture));
}

#[test]
fn a_track_soloed_out_does_not_sound() {
    let (mut project, _, picture, song) = pair();
    track_mut(&mut project, AUDIO).solo = true;
    assert!(sounds(&project, song));
    assert!(!sounds(&project, picture));
    // Every soloed track sounds, of either kind.
    track_mut(&mut project, VIDEO).solo = true;
    assert!(sounds(&project, song) && sounds(&project, picture));
}

#[test]
fn a_soloed_text_track_silences_nothing() {
    let (mut project, _, picture, song) = pair();
    track_mut(&mut project, TEXT).solo = true;
    assert!(sounds(&project, song) && sounds(&project, picture));
}

#[test]
fn a_soloed_and_muted_track_does_not_sound_and_still_solos_the_rest_out() {
    let (mut project, _, picture, song) = pair();
    let track = track_mut(&mut project, AUDIO);
    (track.solo, track.muted) = (true, true);
    assert!(!sounds(&project, song));
    assert!(!sounds(&project, picture));
}

#[test]
fn a_track_is_soloed_out_exactly_where_only_solo_keeps_its_clips_from_sounding() {
    // Every setting of solo on the three tracks and of mute on the two that
    // carry sound.
    for bits in 0..32u8 {
        let on = |bit: u8| bits & (1 << bit) != 0;
        let (mut project, _, picture, song) = pair();
        track_mut(&mut project, VIDEO).solo = on(0);
        track_mut(&mut project, AUDIO).solo = on(1);
        track_mut(&mut project, TEXT).solo = on(2);
        track_mut(&mut project, VIDEO).muted = on(3);
        track_mut(&mut project, AUDIO).muted = on(4);
        for id in [picture, song] {
            let (track, _) = project.clip(id).unwrap();
            assert_eq!(
                sound::soloed_out(&project, track),
                !track.solo && (on(0) || on(1)),
                "{bits:05b}"
            );
            // Unmuted, solo is all that can keep these clips out.
            if !track.muted {
                assert_eq!(
                    sound::soloed_out(&project, track),
                    !sounds(&project, id),
                    "{bits:05b}"
                );
            }
        }
    }
}

#[test]
fn a_track_at_volume_zero_does_not_sound() {
    let (mut project, _, picture, song) = pair();
    track_mut(&mut project, AUDIO).gain = 0.0;
    assert!(!sounds(&project, song));
    assert!(sounds(&project, picture));
    track_mut(&mut project, AUDIO).gain = 0.001;
    assert!(sounds(&project, song));
}

#[test]
fn a_still_a_silent_video_and_a_title_do_not_sound() {
    let f = fixture();
    let project = f.editor.project();
    for id in [f.still, f.silent, f.title] {
        assert!(!sounds(project, id));
        let (track, clip) = project.clip(id).unwrap();
        assert!(!sound::owned(project, clip));
        assert!(!sound::audible_at(project, track, clip, clip.start));
    }
    assert!(sounds(project, f.picture) && sounds(project, f.song));
}

/// A picture and an audio-track clip of the same media in one link.
fn detached() -> (Project, ClipId, ClipId) {
    let (mut project, media) = library();
    let link = Some(Uuid::new_v4());
    let mut picture = clip_of(&media.shot);
    picture.linked_id = link;
    let mut owner = clip_of(&media.shot);
    owner.linked_id = link;
    let picture = place(&mut project, VIDEO, picture);
    let owner = place(&mut project, AUDIO, owner);
    (project, picture, owner)
}

#[test]
fn a_picture_whose_linked_audio_clip_holds_its_sound_is_owned() {
    let (project, picture, owner) = detached();
    assert!(!sounds(&project, picture));
    assert!(sounds(&project, owner));
    assert!(sound::owned(&project, project.clip(picture).unwrap().1));
    // The owner is on an audio track and is never owned itself.
    assert!(!sound::owned(&project, project.clip(owner).unwrap().1));
}

#[test]
fn an_owner_on_a_muted_or_soloed_out_track_still_owns_the_sound() {
    let (mut project, picture, owner) = detached();
    track_mut(&mut project, AUDIO).muted = true;
    assert!(!sounds(&project, picture) && !sounds(&project, owner));
    assert!(sound::owned(&project, project.clip(picture).unwrap().1));

    let (mut project, picture, owner) = detached();
    track_mut(&mut project, VIDEO).solo = true;
    assert!(!sounds(&project, picture) && !sounds(&project, owner));
    assert!(sound::owned(&project, project.clip(picture).unwrap().1));

    // An owner at volume 0 owns it too.
    let (mut project, picture, owner) = detached();
    track_mut(&mut project, AUDIO).clips[0].gain = 0.0;
    assert!(!sounds(&project, picture) && !sounds(&project, owner));
}

#[test]
fn a_linked_pair_of_different_media_owns_nothing() {
    let (mut project, picture, song) = {
        let (project, _, picture, song) = pair();
        (project, picture, song)
    };
    let link = Some(Uuid::new_v4());
    track_mut(&mut project, VIDEO).clips[0].linked_id = link;
    track_mut(&mut project, AUDIO).clips[0].linked_id = link;
    assert!(sounds(&project, picture) && sounds(&project, song));
    assert!(!sound::owned(&project, project.clip(picture).unwrap().1));
}

#[test]
fn an_audio_clip_of_the_same_media_in_another_link_owns_nothing() {
    let (mut project, picture, owner) = detached();
    track_mut(&mut project, AUDIO).clips[0].linked_id = Some(Uuid::new_v4());
    assert!(sounds(&project, picture) && sounds(&project, owner));
    track_mut(&mut project, AUDIO).clips[0].linked_id = None;
    assert!(sounds(&project, picture) && sounds(&project, owner));
}

#[test]
fn volume_zero_is_silent_without_keys_and_admitted_with_them() {
    let (mut project, _, picture, song) = pair();
    track_mut(&mut project, AUDIO).clips[0].gain = 0.0;
    track_mut(&mut project, VIDEO).clips[0].gain = 0.0;
    assert!(!sounds(&project, song) && !sounds(&project, picture));
    track_mut(&mut project, AUDIO).clips[0].animation =
        vec![volume_keys(&[(0.0, 0.0), (4.0, 1.0)])];
    track_mut(&mut project, VIDEO).clips[0].animation = vec![volume_keys(&[(1.0, 0.5)])];
    assert!(sounds(&project, song) && sounds(&project, picture));
}

/// A 6 s song clip at 2 s whose map plays 1 s forwards, holds 1 s, plays 1 s
/// forwards at double speed, 1 s backwards, ramps for 1 s and plays 1 s more.
fn retimed() -> (Project, ClipId) {
    let (mut project, media) = library();
    let key = |at: f64, source: f64, ease| TimeKey {
        at: seconds(at),
        source: seconds(source),
        ease,
    };
    let mut clip = clip_of(&media.song);
    clip.duration = seconds(6.0);
    clip.retime = Some(Retime {
        keys: vec![
            key(0.0, 0.0, Ease::Linear),
            key(1.0, 1.0, Ease::Linear),
            key(2.0, 1.0, Ease::Linear),
            key(3.0, 3.0, Ease::Linear),
            key(4.0, 2.0, EASE),
            key(5.0, 4.0, Ease::Linear),
            key(6.0, 5.0, Ease::Linear),
        ],
    });
    let id = place(&mut project, AUDIO, clip);
    project.validate().unwrap();
    (project, id)
}

#[track_caller]
fn audible(project: &Project, id: ClipId, local: f64) -> bool {
    let (track, clip) = project.clip(id).unwrap();
    let at = clip.start + seconds(local);
    let answer = sound::audible_at(project, track, clip, at);
    assert_eq!(
        Sounding::of(project).audible_at(project, track, clip, at),
        answer
    );
    answer
}

#[test]
fn a_clip_is_heard_inside_itself_and_not_outside() {
    let (project, _, picture, song) = pair();
    for id in [picture, song] {
        assert!(!audible(&project, id, -0.000_001));
        assert!(audible(&project, id, 0.0));
        assert!(audible(&project, id, 5.0));
        assert!(audible(&project, id, 9.999_999));
        // The end is the first instant after the clip.
        assert!(!audible(&project, id, 10.0));
        assert!(!audible(&project, id, 30.0));
    }
}

#[test]
fn a_clip_that_does_not_sound_is_heard_nowhere() {
    let (mut project, _, picture, song) = pair();
    track_mut(&mut project, AUDIO).muted = true;
    assert!(!audible(&project, song, 5.0));
    assert!(audible(&project, picture, 5.0));
}

#[test]
fn a_time_map_is_heard_on_its_straight_forward_segments_only() {
    let (project, id) = retimed();
    assert!(!audible(&project, id, -0.5));
    assert!(audible(&project, id, 0.0));
    assert!(audible(&project, id, 0.5));
    // The freeze is silent from the sample it starts on.
    assert!(!audible(&project, id, 1.0));
    assert!(!audible(&project, id, 1.5));
    // A straight segment between the freeze and the reverse.
    assert!(audible(&project, id, 2.0));
    assert!(audible(&project, id, 2.5));
    assert!(!audible(&project, id, 3.0));
    assert!(!audible(&project, id, 3.5));
    // The ramp moves forwards and is silent all the same.
    assert!(!audible(&project, id, 4.0));
    assert!(!audible(&project, id, 4.5));
    assert!(audible(&project, id, 5.0));
    assert!(audible(&project, id, 5.999_999));
    assert!(!audible(&project, id, 6.0));

    let clip = project.clip(id).unwrap().1;
    let at = |local: f64| clip.start + seconds(local);
    assert_eq!(sound::steady_speed(clip, at(0.5)), Some(1.0));
    assert_eq!(sound::steady_speed(clip, at(2.5)), Some(2.0));
    assert_eq!(sound::steady_speed(clip, at(1.5)), None);
    // One run for each stretch a decoder can play through.
    let mut runs = Vec::new();
    let mut segment = 0;
    while let Some((from, to, after)) = sound::audible_run(clip, segment) {
        runs.push((from, to));
        segment = after;
    }
    assert_eq!(
        runs,
        [(at(0.0), at(1.0)), (at(2.0), at(3.0)), (at(5.0), at(6.0))]
    );
}

#[test]
fn a_clip_without_a_map_is_one_straight_run_at_its_rate() {
    let (mut project, _, _, song) = pair();
    track_mut(&mut project, AUDIO).clips[0].playback_rate = 2.0;
    track_mut(&mut project, AUDIO).clips[0].duration = seconds(5.0);
    let clip = project.clip(song).unwrap().1;
    assert_eq!(sound::steady_speed(clip, clip.start), Some(2.0));
    assert_eq!(
        sound::audible_run(clip, 0),
        Some((clip.start, clip.end(), 1))
    );
    assert_eq!(sound::audible_run(clip, 1), None);
    assert!(audible(&project, song, 4.0));
    assert!(!audible(&project, song, 5.0));
}

const AUDIO_KINDS: [EffectKind; 7] = [
    EffectKind::HighPass,
    EffectKind::EqBand,
    EffectKind::Compressor,
    EffectKind::Limiter,
    EffectKind::Denoise,
    EffectKind::Pitch,
    EffectKind::CenterCut,
];

fn set_effects(clip_id: ClipId, effects: Vec<Effect>) -> Command {
    Command::SetEffects { clip_id, effects }
}

/// An effect of `kind` with one stored value.
fn effect_with(kind: EffectKind, key: ParamKey, value: f32) -> Effect {
    let mut effect = Effect::new(kind);
    effect.params.push(ParamValue { key, value });
    effect
}

#[test]
fn the_audio_kinds_are_the_last_seven_and_use_the_keys_there_are() {
    assert_eq!(EffectKind::ALL.len(), 27);
    assert_eq!(EffectKind::ALL[20..], AUDIO_KINDS);
    for kind in EffectKind::ALL {
        assert_eq!(kind.is_audio(), AUDIO_KINDS.contains(&kind), "{kind:?}");
    }
    let keys = |kind| -> Vec<ParamKey> { look::spec(kind).iter().map(|spec| spec.key).collect() };
    use ParamKey::{Amount, Frequency, Gain, Threshold, Width};
    assert_eq!(keys(EffectKind::HighPass), [Frequency]);
    assert_eq!(keys(EffectKind::EqBand), [Frequency, Gain, Width]);
    assert_eq!(keys(EffectKind::Compressor), [Threshold, Amount, Gain]);
    assert_eq!(keys(EffectKind::Limiter), [Threshold]);
    assert_eq!(keys(EffectKind::Denoise), [Amount, Threshold]);
    assert_eq!(keys(EffectKind::Pitch), [Amount]);
    assert_eq!(keys(EffectKind::CenterCut), [Amount]);
    for kind in AUDIO_KINDS {
        assert!(!kind.needs_asset() && !kind.is_curve(), "{kind:?}");
        let effect = Effect::new(kind);
        assert!(effect.points.is_empty() && effect.mix == 1.0);
    }
}

#[test]
fn each_audio_kind_is_taken_on_an_audio_clip_within_its_ranges_and_refused_outside() {
    let mut f = fixture();
    for kind in AUDIO_KINDS {
        // At its defaults, stored or not.
        f.editor
            .dispatch(set_effects(f.song, vec![Effect::new(kind)]))
            .unwrap();
        for spec in look::spec(kind) {
            for value in [spec.default, spec.min, spec.max] {
                f.editor
                    .dispatch(set_effects(
                        f.song,
                        vec![effect_with(kind, spec.key, value)],
                    ))
                    .unwrap();
                assert_eq!(
                    clip(&f.editor, f.song).effects[0].param(spec.key),
                    Some(value)
                );
            }
        }
    }
    let mut f = fixture();
    for kind in AUDIO_KINDS {
        for spec in look::spec(kind) {
            let outside = [
                spec.min.next_down(),
                spec.max.next_up(),
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ];
            for value in outside {
                let effect = effect_with(kind, spec.key, value);
                refused(&mut f.editor, set_effects(f.song, vec![effect]), "effect");
            }
        }
        // A key of another table.
        let effect = effect_with(kind, ParamKey::Radius, 0.0);
        refused(&mut f.editor, set_effects(f.song, vec![effect]), "effect");
    }
}

#[test]
fn an_audio_kind_is_refused_off_an_audio_track_and_a_picture_kind_on_one() {
    let mut f = fixture();
    for kind in AUDIO_KINDS {
        // A picture with sound, a silent video and a still.
        for id in [f.picture, f.silent, f.still] {
            refused(
                &mut f.editor,
                set_effects(id, vec![Effect::new(kind)]),
                "audio effect on a picture clip",
            );
        }
        refused(
            &mut f.editor,
            set_effects(f.title, vec![Effect::new(kind)]),
            "text clip look",
        );
    }
    for kind in EffectKind::ALL {
        if kind.is_audio() || kind.needs_asset() {
            continue;
        }
        refused(
            &mut f.editor,
            set_effects(f.song, vec![Effect::new(kind)]),
            "audio clip look",
        );
    }
    // One picture kind among audio kinds refuses the stack.
    let stack = vec![
        Effect::new(EffectKind::HighPass),
        Effect::new(EffectKind::Blur),
    ];
    refused(&mut f.editor, set_effects(f.song, stack), "audio clip look");
    // A video's sound on an audio track takes them as a song does.
    let (mut project, media) = library();
    let detached = place(&mut project, AUDIO, clip_of(&media.shot));
    let mut editor = Editor::new(project).unwrap();
    editor
        .dispatch(set_effects(
            detached,
            vec![Effect::new(EffectKind::Denoise)],
        ))
        .unwrap();
}

#[test]
fn an_audio_effect_is_whole_takes_no_points_or_media_and_is_not_keyed() {
    let mut f = fixture();
    for kind in AUDIO_KINDS {
        for mix in [0.5, 0.0, 0.999, 1.001, f32::NAN] {
            let mut effect = Effect::new(kind);
            effect.mix = mix;
            refused(&mut f.editor, set_effects(f.song, vec![effect]), "effect");
        }
        let mut effect = Effect::new(kind);
        effect.points = vec![[0.0, 0.0], [1.0, 1.0]];
        refused(&mut f.editor, set_effects(f.song, vec![effect]), "effect");
        let mut effect = Effect::new(kind);
        effect.asset_id = Some(Uuid::new_v4());
        refused(&mut f.editor, set_effects(f.song, vec![effect]), "effect");
    }
    // A disabled one is kept, and is as bounded as an enabled one.
    let mut off = Effect::new(EffectKind::Limiter);
    off.enabled = false;
    f.editor
        .dispatch(set_effects(f.song, vec![off.clone()]))
        .unwrap();
    f.editor.undo();
    off.mix = 0.5;
    refused(&mut f.editor, set_effects(f.song, vec![off]), "effect");

    // No parameter of an audio effect takes keys; the clip's volume still does.
    for kind in AUDIO_KINDS {
        let effect = Effect::new(kind);
        let spec = look::spec(kind)[0];
        let keys = |value| Keyframe {
            at: Time::ZERO,
            value,
            ease: Ease::Linear,
        };
        f.editor
            .dispatch(set_effects(f.song, vec![effect.clone()]))
            .unwrap();
        let before = f.editor.snapshot();
        let keyed = f.editor.dispatch(Command::SetKeyframes {
            clip_id: f.song,
            target: Some(effect.id),
            key: spec.key,
            keys: vec![keys(spec.default)],
        });
        assert!(
            matches!(&keyed, Err(EditError::Invalid(reason)) if reason.split(" (clip ").next() == Some("audio clip look")),
            "{kind:?}: {keyed:?}"
        );
        assert!(Arc::ptr_eq(&before, &f.editor.snapshot()));
        f.editor
            .dispatch(Command::SetKeyframes {
                clip_id: f.song,
                target: None,
                key: ParamKey::Gain,
                keys: vec![keys(0.5)],
            })
            .unwrap();
        f.editor.undo();
        f.editor.undo();
    }
}

#[test]
fn an_audio_clip_holds_sixteen_audio_effects_and_not_seventeen() {
    let mut f = fixture();
    let bands = |count| {
        (0..count)
            .map(|_| Effect::new(EffectKind::EqBand))
            .collect()
    };
    refused(
        &mut f.editor,
        set_effects(f.song, bands(17)),
        "effect or mask count",
    );
    f.editor.dispatch(set_effects(f.song, bands(16))).unwrap();
    assert_eq!(clip(&f.editor, f.song).effects.len(), look::MAX_EFFECTS);
    // More than one pitch is legal.
    let pitches = (0..2).map(|_| Effect::new(EffectKind::Pitch)).collect();
    f.editor.dispatch(set_effects(f.song, pitches)).unwrap();
    // The same effect twice is not.
    let band = Effect::new(EffectKind::EqBand);
    let twice = vec![band.clone(), band];
    let (before, revision) = (f.editor.snapshot(), f.editor.revision());
    assert!(matches!(
        f.editor.dispatch(set_effects(f.song, twice)),
        Err(EditError::Invalid(reason)) if reason.split(" (clip ").next() == Some("effect")
    ));
    assert!(Arc::ptr_eq(&before, &f.editor.snapshot()));
    assert_eq!(f.editor.revision(), revision);
}

#[test]
fn a_refused_set_effects_leaves_the_stack_and_the_history_as_they_were() {
    let mut f = fixture();
    let voice = vec![
        effect_with(EffectKind::HighPass, ParamKey::Frequency, 120.0),
        Effect::new(EffectKind::Compressor),
    ];
    let before = f.editor.snapshot();
    let outcome = f
        .editor
        .dispatch(set_effects(f.song, voice.clone()))
        .unwrap();
    assert_eq!(outcome.label, "Change effects");
    let (after, revision) = (f.editor.snapshot(), f.editor.revision());

    let mut half = Effect::new(EffectKind::Denoise);
    half.mix = 0.5;
    let refusals = [
        (vec![half], "effect"),
        (vec![Effect::new(EffectKind::Glow)], "audio clip look"),
    ];
    for (stack, what) in refusals {
        match f.editor.dispatch(set_effects(f.song, stack)) {
            Err(EditError::Invalid(reason)) => {
                assert_eq!(reason.split(" (clip ").next().unwrap(), what)
            }
            other => panic!("expected Invalid({what}), got {other:?}"),
        }
        assert!(Arc::ptr_eq(&after, &f.editor.snapshot()));
        assert_eq!(f.editor.revision(), revision);
    }
    // In a batch the earlier commands are undone with it.
    let batch = Command::Batch {
        label: "Clean up".into(),
        commands: vec![
            set_fades(f.song, 1.0, 1.0),
            set_effects(f.song, vec![Effect::new(EffectKind::Pitch)]),
            set_effects(f.picture, vec![Effect::new(EffectKind::Pitch)]),
        ],
    };
    assert!(matches!(
        f.editor.dispatch(batch),
        Err(EditError::Invalid(reason)) if reason.split(" (clip ").next() == Some("audio effect on a picture clip")
    ));
    assert!(Arc::ptr_eq(&after, &f.editor.snapshot()));
    assert_eq!(clip(&f.editor, f.song).effects, voice);
    assert_eq!(fades(&f.editor, f.song), (Time::ZERO, Time::ZERO));

    assert!(f.editor.undo());
    assert!(Arc::ptr_eq(&before, &f.editor.snapshot()));
    assert!(clip(&f.editor, f.song).effects.is_empty());
    assert!(f.editor.redo());
    assert!(Arc::ptr_eq(&after, &f.editor.snapshot()));
}

#[test]
fn audio_effects_are_written_by_name_and_read_back() {
    let names = [
        "high_pass",
        "eq_band",
        "compressor",
        "limiter",
        "denoise",
        "pitch",
        "center_cut",
    ];
    for (kind, name) in AUDIO_KINDS.into_iter().zip(names) {
        assert_eq!(serde_json::to_value(kind).unwrap(), name);
    }
    let mut f = fixture();
    let stack: Vec<Effect> = AUDIO_KINDS.into_iter().map(Effect::new).collect();
    let command = set_effects(f.song, stack.clone());
    let text = serde_json::to_string(&command).unwrap();
    assert!(text.contains(r#""type":"set_effects""#), "{text}");
    f.editor
        .dispatch(serde_json::from_str(&text).unwrap())
        .unwrap();
    assert_eq!(clip(&f.editor, f.song).effects, stack);
    let text = serde_json::to_string(f.editor.project()).unwrap();
    let read: Project = serde_json::from_str(&text).unwrap();
    read.validate().unwrap();
    assert_eq!(read.clip(f.song).unwrap().1.effects, stack);
}

#[test]
fn the_first_enabled_pitch_is_read_and_its_ratio_holds_pitch_within_one_tempo_stage() {
    let asset = Asset::new(
        PathBuf::from("/media/voice.wav"),
        MediaKind::Audio,
        seconds(10.0),
    );
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    let pitch = |semitones: f32| {
        let mut effect = Effect::new(EffectKind::Pitch);
        effect.params = vec![ParamValue {
            key: ParamKey::Amount,
            value: semitones,
        }];
        effect
    };
    // No `Pitch`: speed changes pitch.
    assert_eq!(sound::pitch(&clip), None);
    // A `Pitch` with no value stored is at its default, 0: keep pitch.
    clip.effects = vec![
        Effect::new(EffectKind::Compressor),
        Effect::new(EffectKind::Pitch),
    ];
    assert_eq!(sound::pitch(&clip), Some(0.0));
    // A disabled one is passed over and only the first enabled one is read.
    let mut disabled = pitch(5.0);
    disabled.enabled = false;
    clip.effects = vec![disabled.clone(), pitch(-3.0), pitch(7.0)];
    assert_eq!(sound::pitch(&clip), Some(-3.0));
    clip.effects = vec![disabled];
    assert_eq!(sound::pitch(&clip), None);

    assert_eq!(sound::pitch_ratio(0.0), 1.0);
    assert_eq!(sound::pitch_ratio(12.0), 2.0);
    assert_eq!(sound::pitch_ratio(-12.0), 0.5);
    assert!((sound::pitch_ratio(7.0) - 1.498_307).abs() < 1e-6);

    // Kept while speed over ratio is within 0.5 to 2, ends included.
    for (speed, semitones, kept) in [
        (1.0, 0.0, true),
        (0.5, 0.0, true),
        (2.0, 0.0, true),
        (0.49, 0.0, false),
        (2.01, 0.0, false),
        (4.0, 12.0, true),
        (4.0, 0.0, false),
        (0.25, -12.0, true),
        (0.05, 0.0, false),
        (16.0, 12.0, false),
    ] {
        assert_eq!(
            sound::pitch_kept(speed, semitones),
            kept,
            "{speed}x at {semitones} semitones"
        );
    }
    assert_eq!(sound::PITCH_KEPT, 0.5..=2.0);
}

// ---- Ducking ----------------------------------------------------------------

const ATTACK: Time = Time(150_000);
const RELEASE: Time = Time(400_000);

fn keys_of(keys: &[Keyframe]) -> Vec<(f64, f32)> {
    (keys.iter())
        .map(|key| (key.at.as_seconds(), key.value))
        .collect()
}

fn span(from: f64, to: f64) -> (Time, Time) {
    (seconds(from), seconds(to))
}

/// The track a duck's keys make on a clip of `duration`, held to the bounds
/// a clip's volume has.
fn validates(keys: &[Keyframe], duration: Time) -> bool {
    let spec = (look::clip_spec().iter())
        .find(|spec| spec.key == ParamKey::Gain)
        .expect("a clip's volume has bounds");
    AnimTrack {
        target: None,
        key: ParamKey::Gain,
        keys: keys.to_vec(),
    }
    .validate(duration, spec)
}

#[test]
fn one_span_dips_the_clip_with_four_keys() {
    let keys = sound::duck_keys(&[span(2.0, 3.0)], seconds(10.0), 0.5, 0.25, ATTACK, RELEASE);
    assert_eq!(
        keys_of(&keys),
        [(1.85, 0.5), (2.0, 0.125), (3.0, 0.125), (3.4, 0.5)]
    );
    assert!(keys.iter().all(|key| key.ease == Ease::Linear));
    assert!(validates(&keys, seconds(10.0)));
    // No span, or one that does not reach the clip, gives no key.
    assert!(sound::duck_keys(&[], seconds(10.0), 1.0, 0.25, ATTACK, RELEASE).is_empty());
    let away = [span(-3.0, -1.0), span(10.2, 12.0)];
    assert!(sound::duck_keys(&away, seconds(10.0), 1.0, 0.25, ATTACK, RELEASE).is_empty());
}

#[test]
fn a_span_at_the_clip_edges_is_held_to_them_and_the_keys_still_rise() {
    let duration = seconds(10.0);
    // From before the start into the clip: the key at 0 carries the dip.
    let keys = sound::duck_keys(&[span(-1.0, 1.0)], duration, 1.0, 0.25, ATTACK, RELEASE);
    assert_eq!(keys_of(&keys), [(0.0, 0.25), (1.0, 0.25), (1.4, 1.0)]);
    // Starting inside the attack: the key at 0 is part way down it.
    let keys = sound::duck_keys(&[span(0.075, 1.0)], duration, 1.0, 0.25, ATTACK, RELEASE);
    assert_eq!(keys.first().map(|key| key.at), Some(Time::ZERO));
    assert!((keys[0].value - 0.625).abs() < 1e-6, "{:?}", keys_of(&keys));
    // Past the end: the key at the end is part way up the release.
    let keys = sound::duck_keys(&[span(8.0, 9.8)], duration, 1.0, 0.25, ATTACK, RELEASE);
    let last = keys.last().unwrap();
    assert_eq!(last.at, duration);
    assert!((last.value - 0.625).abs() < 1e-6, "{:?}", keys_of(&keys));
    // Over the whole clip: low at both ends.
    let keys = sound::duck_keys(&[span(-1.0, 11.0)], duration, 1.0, 0.25, ATTACK, RELEASE);
    assert_eq!(keys_of(&keys), [(0.0, 0.25), (10.0, 0.25)]);
    for spans in [
        vec![span(-1.0, 1.0)],
        vec![span(0.0, 1.0)],
        vec![span(9.0, 10.0)],
        vec![span(0.075, 9.9)],
    ] {
        let keys = sound::duck_keys(&spans, duration, 1.0, 0.25, ATTACK, RELEASE);
        assert!(keys.windows(2).all(|pair| pair[0].at < pair[1].at));
        assert!(
            validates(&keys, duration),
            "{spans:?}: {:?}",
            keys_of(&keys)
        );
    }
}

#[test]
fn spans_whose_ramps_cross_are_one_dip() {
    // 0.3 s apart: the release of the first would cross the attack of the
    // second, so the level stays down between them.
    let ducked = sound::duck(
        &[span(4.0, 5.0), span(2.0, 3.0), span(3.3, 3.6)],
        seconds(10.0),
        1.0,
        0.5,
        ATTACK,
        RELEASE,
    );
    assert_eq!(
        keys_of(&ducked.keys),
        [(1.85, 1.0), (2.0, 0.5), (5.0, 0.5), (5.4, 1.0)]
    );
    assert_eq!((ducked.dips, ducked.merged), (1, 0));
    // Ramps that only meet share their key.
    let ducked = sound::duck(
        &[span(2.0, 3.0), span(3.55, 4.0)],
        seconds(10.0),
        1.0,
        0.5,
        ATTACK,
        RELEASE,
    );
    assert_eq!(ducked.dips, 2);
    assert_eq!(
        keys_of(&ducked.keys),
        [
            (1.85, 1.0),
            (2.0, 0.5),
            (3.0, 0.5),
            (3.4, 1.0),
            (3.55, 0.5),
            (4.0, 0.5),
            (4.4, 1.0)
        ]
    );
}

#[test]
fn two_hundred_spans_fit_the_key_bound_by_merging_the_closest() {
    // A dip a second, each 0.3 s long; every tenth gap a little shorter.
    let spans: Vec<(Time, Time)> = (0..200)
        .map(|index| {
            let from = index as f64 + if index % 10 == 1 { -0.05 } else { 0.0 };
            span(from, from + 0.3)
        })
        .collect();
    let duration = seconds(201.0);
    let ducked = sound::duck(&spans, duration, 1.0, 0.25, ATTACK, RELEASE);
    assert!(ducked.keys.len() <= editor_core::animate::MAX_KEYFRAMES);
    assert!(validates(&ducked.keys, duration));
    assert_eq!(ducked.dips + ducked.merged, 200);
    assert_eq!(ducked.dips, 64);
    // The shortest gaps were the first closed.
    let low: Vec<f64> = (ducked.keys.iter())
        .filter(|key| key.value == 0.25)
        .map(|key| key.at.as_seconds())
        .collect();
    assert!(!low.contains(&0.3) && !low.contains(&0.95), "{low:?}");
    assert_eq!(
        sound::duck_keys(&spans, duration, 1.0, 0.25, ATTACK, RELEASE),
        ducked.keys
    );
}

#[test]
fn the_closest_spans_are_merged_first() {
    let mut spans = vec![
        span(0.0, 1.0),
        span(1.5, 2.0),
        span(4.0, 5.0),
        span(5.1, 6.0),
    ];
    assert_eq!(sound::merge_closest(&mut spans, 4), 0);
    assert_eq!(sound::merge_closest(&mut spans, 2), 2);
    assert_eq!(spans, [span(0.0, 2.0), span(4.0, 6.0)]);
    assert_eq!(sound::merge_closest(&mut spans, 0), 1);
    assert_eq!(spans, [span(0.0, 6.0)]);
}

#[test]
fn ducked_keys_are_written_as_one_step_and_undone() {
    let (mut project, media) = library();
    let clip = place(&mut project, AUDIO, clip_of(&media.song));
    let mut editor = Editor::new(project).unwrap();
    let keys = sound::duck_keys(&[span(2.0, 3.0)], seconds(10.0), 1.0, 0.25, ATTACK, RELEASE);
    editor
        .dispatch(Command::Batch {
            label: "Duck under speech".to_owned(),
            commands: vec![Command::SetKeyframes {
                clip_id: clip,
                target: None,
                key: ParamKey::Gain,
                keys: keys.clone(),
            }],
        })
        .unwrap();
    assert_eq!(editor.history().count(), 1);
    let stored = editor.project().clip(clip).unwrap().1;
    assert_eq!(stored.animation[0].keys, keys);
    // Halfway down the attack, and at the bottom.
    let track = editor.project().clip(clip).unwrap().0;
    let start = stored.start;
    let at = |local: f64| sound::level_at(track, stored, start + seconds(local));
    assert!((at(1.925) - 0.625).abs() < 1e-4);
    assert_eq!(at(2.5), 0.25);
    assert!(editor.undo());
    assert!(editor.project().clip(clip).unwrap().1.animation.is_empty());
}
