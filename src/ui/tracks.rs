//! Track volume and solo as the interface handles them: which tracks have
//! them, what a head shows of them, and the edit a head's cell, its menu or
//! a line of the command panel asks for.
//!
//! Nothing here draws. A track's volume is read and typed in percent and
//! kept as the factor the project holds.
use editor_core::{Command, Project, Track, TrackKind, sound};

/// The most a track's volume is, in percent: the project's bound of 8.
pub const MOST: f32 = 800.0;

/// Whether the track carries sound, so that its head has mute, solo and a
/// volume. A text track's solo is ignored by the mix and is not offered.
pub fn carries_sound(track: &Track) -> bool {
    track.kind != TrackKind::Text
}

/// The track's volume in percent, as the field of its menu holds it: what
/// the project keeps, not a rounding of it.
pub fn percent(track: &Track) -> f32 {
    track.gain * 100.0
}

/// How a volume reads in percent: to a hundredth, as the field writes it
/// and [`volume`] keeps it, without the zeros that say nothing. A volume a
/// file or an agent holds finer than that, and that would read as silence or
/// as the volume a track starts with while being neither, reads finer.
pub fn reading(gain: f32) -> String {
    let percent = f64::from(gain) * 100.0;
    let to = |decimals: usize| {
        let text = format!("{percent:.decimals$}");
        let text = text.trim_end_matches('0').trim_end_matches('.');
        if text.is_empty() { "0" } else { text }.to_owned()
    };
    let text = to(2);
    if (text == "100" && gain != 1.0) || (text == "0" && gain != 0.0) {
        return to(6);
    }
    text
}

/// What a head says of a volume that is not the one a track starts with.
pub fn figure(track: &Track) -> Option<String> {
    (carries_sound(track) && track.gain != 1.0).then(|| format!("{}%", reading(track.gain)))
}

/// Whether another track's solo, and nothing of its own, silences this one:
/// the solo rule of [`sound::sounds`]. A muted track is silent by its mute,
/// which its head already shows.
pub fn silenced_by_solo(project: &Project, track: &Track) -> bool {
    carries_sound(track) && !track.muted && sound::soloed_out(project, track)
}

fn state(track: &Track, gain: Option<f32>, solo: Option<bool>) -> Command {
    Command::SetTrackState {
        track_id: track.id,
        locked: None,
        hidden: None,
        muted: None,
        name: None,
        gain,
        solo,
    }
}

/// The command that solos the track, or ends its solo.
pub fn solo(track: &Track, on: bool) -> Command {
    state(track, None, Some(on))
}

/// The command that gives the track this volume in percent, held to what
/// the project takes and kept to the hundredth of a percent a head and its
/// field read, so that what is entered is what is then shown. `None` for a
/// number that is none.
pub fn volume(track: &Track, percent: f32) -> Option<Command> {
    (percent.is_finite()).then(|| {
        let hundredths = (percent.clamp(0.0, MOST) * 100.0).round();
        state(track, Some(hundredths / 10_000.0), None)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Asset, Clip, Editor, MediaKind, Time};
    use std::sync::Arc;

    /// A video, an audio and a text track, the first two with a clip of
    /// media that has sound.
    fn project() -> Project {
        let mut project = Project::new("Tracks");
        project.tracks.clear();
        for (name, kind) in [
            ("Video 1", TrackKind::Video),
            ("Audio 1", TrackKind::Audio),
            ("Text 1", TrackKind::Text),
        ] {
            let mut track = Track::new(name, kind);
            if kind != TrackKind::Text {
                let path = format!("/media/{name}.mp4");
                let mut asset = Asset::new(path.into(), MediaKind::Video, Time(4_000_000));
                asset.has_audio = true;
                track.clips.push(Clip::from_asset(&asset, Time::ZERO));
                Arc::make_mut(&mut project.assets).insert(asset.id, asset);
            }
            project.tracks.push(Arc::new(track));
        }
        project
    }

    fn gain_of(command: Option<Command>) -> Option<f32> {
        match command? {
            Command::SetTrackState {
                gain,
                locked: None,
                hidden: None,
                muted: None,
                name: None,
                solo: None,
                ..
            } => gain,
            other => panic!("not a volume alone: {other:?}"),
        }
    }

    #[test]
    fn a_volume_is_held_to_what_the_project_takes_and_is_one_step_that_undoes() {
        let mut editor = Editor::new(project()).unwrap();
        let track = editor.project().tracks[1].clone();
        assert_eq!(gain_of(volume(&track, 50.0)), Some(0.5));
        assert_eq!(gain_of(volume(&track, 999.0)), Some(8.0));
        assert_eq!(gain_of(volume(&track, -5.0)), Some(0.0));
        assert_eq!(gain_of(volume(&track, f32::INFINITY)), None);
        assert_eq!(gain_of(volume(&track, f32::NAN)), None);
        // The most the field gives is the most the project takes.
        let steps = editor.history().count();
        editor.dispatch(volume(&track, 999.0).unwrap()).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        let track = editor.project().tracks[1].clone();
        assert_eq!(track.gain, 8.0);
        assert_eq!(figure(&track).as_deref(), Some("800%"));
        assert!(editor.undo());
        assert_eq!(editor.project().tracks[1].gain, 1.0);
        assert_eq!(figure(&editor.project().tracks[1]), None);
    }

    #[test]
    fn a_head_says_a_volume_only_where_it_is_not_the_one_a_track_starts_with() {
        let mut project = project();
        assert!(project.tracks.iter().all(|track| figure(track).is_none()));
        for track in &mut project.tracks {
            Arc::make_mut(track).gain = 0.5;
        }
        assert_eq!(figure(&project.tracks[0]).as_deref(), Some("50%"));
        assert_eq!(figure(&project.tracks[1]).as_deref(), Some("50%"));
        // A text track has no sound to say the volume of.
        assert_eq!(figure(&project.tracks[2]), None);
        Arc::make_mut(&mut project.tracks[1]).gain = 0.0;
        assert_eq!(figure(&project.tracks[1]).as_deref(), Some("0%"));
    }

    #[test]
    fn a_head_never_says_silence_or_the_starting_volume_of_a_volume_that_is_neither() {
        let mut project = project();
        let mut said = |gain: f32| {
            Arc::make_mut(&mut project.tracks[1]).gain = gain;
            figure(&project.tracks[1])
        };
        assert_eq!(said(1.0), None);
        assert_eq!(said(0.0).as_deref(), Some("0%"));
        // A part of a percent is said, not rounded away.
        assert_eq!(said(0.004).as_deref(), Some("0.4%"));
        assert_eq!(said(0.124).as_deref(), Some("12.4%"));
        assert_eq!(said(1.004).as_deref(), Some("100.4%"));
        assert_eq!(said(0.9996).as_deref(), Some("99.96%"));
        assert_eq!(said(8.0).as_deref(), Some("800%"));
        // Finer than the field writes, from a file or an agent: still
        // neither "0%" nor "100%".
        for gain in [0.000_04_f32, 1.000_04, 0.999_96] {
            let said = said(gain).unwrap();
            assert!(said != "0%" && said != "100%", "{gain}: {said}");
        }
    }

    #[test]
    fn a_volume_is_kept_to_the_hundredth_of_a_percent_it_reads() {
        let track = project().tracks[1].clone();
        for (percent, gain) in [
            (12.4, 0.124),
            (0.4, 0.004),
            (100.4, 1.004),
            (33.333, 0.3333),
        ] {
            let kept = gain_of(volume(&track, percent)).unwrap();
            assert_eq!(kept, gain, "{percent}");
            // What it reads, entered again, is the same volume.
            let read: f32 = reading(kept).parse().unwrap();
            assert_eq!(gain_of(volume(&track, read)), Some(kept), "{percent}");
        }
    }

    #[test]
    fn a_solo_is_one_step_that_undoes() {
        let mut editor = Editor::new(project()).unwrap();
        let track = editor.project().tracks[1].clone();
        let steps = editor.history().count();
        editor.dispatch(solo(&track, true)).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        assert!(editor.project().tracks[1].solo);
        assert!(silenced_by_solo(
            editor.project(),
            &editor.project().tracks[0]
        ));
        assert!(editor.undo());
        assert!(!editor.project().tracks[1].solo);
        assert!(!silenced_by_solo(
            editor.project(),
            &editor.project().tracks[0]
        ));
    }

    #[test]
    fn a_head_is_dimmed_exactly_where_solo_alone_keeps_its_clips_from_sounding() {
        // Every setting of solo on the three tracks and of mute on the two
        // that carry sound.
        for bits in 0..32u8 {
            let on = |bit: u8| bits & (1 << bit) != 0;
            let mut project = project();
            for (index, track) in project.tracks.iter_mut().enumerate() {
                let track = Arc::make_mut(track);
                track.solo = on(index as u8);
                track.muted = index < 2 && on(index as u8 + 3);
            }
            for track in &project.tracks[..2] {
                let clip = &track.clips[0];
                let sounds = sound::sounds(&project, track, clip);
                let dimmed = silenced_by_solo(&project, track);
                // An unmuted track is heard or dimmed, never both or
                // neither; a muted one is not heard and not dimmed.
                assert_eq!(dimmed, !track.muted && !sounds, "{bits:05b}");
            }
            // A text track has no mute cell to dim.
            assert!(!silenced_by_solo(&project, &project.tracks[2]));
        }
    }
}
