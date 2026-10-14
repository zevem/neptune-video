//! Readback of what the mixer returns: fades, track volume, solo, admission
//! and the master limiter, over sound whose every sample the test wrote.
use super::*;
use editor_core::{AnimTrack, Asset, Clip, Ease, Keyframe, MediaKind, ParamKey, Track, TrackKind};
use std::{
    f64::consts::{FRAC_PI_2, TAU},
    path::Path,
    process::Stdio,
};

const RATE: u32 = 48_000;
/// Off the frames a fade is read at, so the tone is not at a zero crossing there.
const TONE_HZ: f64 = 997.0;

pub(super) fn decoder_available(what: &str) -> bool {
    let available = tool_command("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !available {
        eprintln!("FFmpeg is unavailable; {what} fixture skipped");
    }
    available
}

/// A stereo 32-bit float WAV holding `mono` on both channels, so a decoder
/// that changes neither rate nor layout hands back the samples written here.
fn wave(path: &Path, mono: &[f32]) -> Asset {
    let data_bytes = mono.len() as u32 * 8;
    let mut wave = Vec::with_capacity(44 + data_bytes as usize);
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16_u32.to_le_bytes());
    wave.extend_from_slice(&3_u16.to_le_bytes()); // IEEE float
    wave.extend_from_slice(&2_u16.to_le_bytes());
    wave.extend_from_slice(&RATE.to_le_bytes());
    wave.extend_from_slice(&(RATE * 8).to_le_bytes());
    wave.extend_from_slice(&8_u16.to_le_bytes());
    wave.extend_from_slice(&32_u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&data_bytes.to_le_bytes());
    for sample in mono {
        wave.extend_from_slice(&sample.to_le_bytes());
        wave.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, wave).unwrap();
    Asset::new(
        path.to_path_buf(),
        MediaKind::Audio,
        Time::from_seconds(mono.len() as f64 / f64::from(RATE)),
    )
}

/// A full-scale tone: its peaks reach 1.0 and no sample is past it.
fn tone(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|frame| (TAU * TONE_HZ * frame as f64 / f64::from(RATE)).sin() as f32)
        .collect()
}

fn frames(seconds: f64) -> usize {
    (seconds * f64::from(RATE)).round() as usize
}

/// A project of one audio track for each clip, in order.
pub(super) fn project_of(assets: &[&Asset], clips: Vec<Clip>) -> Project {
    let mut project = Project::new("Mixer level fixture");
    for asset in assets {
        Arc::make_mut(&mut project.assets).insert(asset.id, (*asset).clone());
    }
    project.tracks = clips
        .into_iter()
        .enumerate()
        .map(|(index, clip)| {
            let mut track = Track::new(format!("Audio {index}"), TrackKind::Audio);
            track.clips.push(clip);
            Arc::new(track)
        })
        .collect();
    project.validate().unwrap();
    project
}

fn mixer(project: &Project) -> Mixer {
    project.validate().unwrap();
    Mixer::new(
        Arc::new(project.clone()),
        1.0,
        RATE,
        2,
        Processes::default(),
    )
}

/// The left channel of `chunks` whole chunks from the project's start, each
/// asked for as playback asks: at the time its first frame falls on.
fn played(mixer: &mut Mixer, chunks: usize) -> Vec<f32> {
    let mut left = Vec::with_capacity(chunks * CHUNK_FRAMES);
    for chunk in 0..chunks {
        let start = Time::from_seconds((chunk * CHUNK_FRAMES) as f64 / f64::from(RATE));
        let samples = mixer.chunk(start, CHUNK_FRAMES).unwrap();
        for pair in samples.chunks_exact(2) {
            assert_eq!(pair[0].to_bits(), pair[1].to_bits());
            left.push(pair[0]);
        }
    }
    left
}

fn started(mixer: &Mixer) -> Vec<ClipId> {
    let started: Vec<_> = mixer.voices.keys().copied().collect();
    assert_eq!(
        mixer.decoder_audit.opened,
        started.len(),
        "every decoder started is still held, so the held ones are all that started"
    );
    started
}

fn volume_keys(values: &[(f64, f32)]) -> AnimTrack {
    AnimTrack {
        target: None,
        key: ParamKey::Gain,
        keys: values
            .iter()
            .map(|&(at, value)| Keyframe {
                at: Time::from_seconds(at),
                value,
                ease: Ease::Linear,
            })
            .collect(),
    }
}

#[test]
fn a_plain_clip_and_the_source_monitor_leave_the_mixer_bit_for_bit() {
    let _heavy = crate::heavy_test();
    if !decoder_available("plain PCM") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let written = tone(frames(0.25));
    let asset = wave(&temp.path().join("tone.wav"), &written);
    assert_eq!(written.iter().fold(0_f32, |peak, s| peak.max(s.abs())), 1.0);
    let same = |heard: &[f32], what: &str| {
        for (frame, (heard, written)) in heard.iter().zip(&written).enumerate() {
            assert_eq!(heard.to_bits(), written.to_bits(), "{what}, frame {frame}");
        }
    };

    // No fades, unit track volume, no solo: a full-scale tone is what was written.
    let project = project_of(&[&asset], vec![Clip::from_asset(&asset, Time::ZERO)]);
    let mut timeline = mixer(&project);
    same(&played(&mut timeline, 8), "timeline");
    assert_eq!(timeline.limit, 1.0);

    // The source monitor plays its own one-asset document through the same mixer.
    let mut monitor = mixer(&Project::asset_preview(&asset).unwrap());
    same(&played(&mut monitor, 8), "source monitor");
    assert_eq!(monitor.limit, 1.0);
}

#[test]
fn a_fade_in_follows_the_equal_power_curve_across_chunks() {
    let _heavy = crate::heavy_test();
    if !decoder_available("fade in") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let written = tone(frames(0.5));
    let asset = wave(&temp.path().join("tone.wav"), &written);
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    clip.fade_in = Time::from_seconds(0.1);
    let project = project_of(&[&asset], vec![clip]);
    let heard = played(&mut mixer(&project), 8);
    let fade = frames(0.1);
    assert!(heard.len() > fade + CHUNK_FRAMES);

    assert_eq!(heard[0], 0.0);
    let middle = fade / 2;
    assert!(written[middle].abs() > 0.1, "the tone sounds at 50 ms");
    assert!((heard[middle] - written[middle] * std::f32::consts::FRAC_1_SQRT_2).abs() < 1.0e-4);
    // Frames 1023 and 1024, and three more chunk edges, are inside the fade:
    // every frame is on the one curve whatever chunk it came in.
    for frame in 0..fade {
        let level = (FRAC_PI_2 * frame as f64 / fade as f64).sin() as f32;
        assert!(
            (heard[frame] - written[frame] * level).abs() < 1.0e-4,
            "frame {frame}"
        );
    }
    for frame in fade..heard.len() {
        assert_eq!(
            heard[frame].to_bits(),
            written[frame].to_bits(),
            "frame {frame}"
        );
    }
}

#[test]
fn a_fade_out_mirrors_the_fade_in_and_ends_in_silence() {
    let _heavy = crate::heavy_test();
    if !decoder_available("fade out") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let written = tone(frames(0.5));
    let asset = wave(&temp.path().join("tone.wav"), &written);
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    clip.duration = Time::from_seconds(0.2);
    clip.fade_out = Time::from_seconds(0.1);
    let project = project_of(&[&asset], vec![clip]);
    let heard = played(&mut mixer(&project), 10);
    let (end, fade) = (frames(0.2), frames(0.1));

    for frame in 0..end - fade {
        assert_eq!(
            heard[frame].to_bits(),
            written[frame].to_bits(),
            "frame {frame}"
        );
    }
    let middle = end - fade / 2;
    assert!(
        written[middle].abs() > 0.1,
        "the tone sounds 50 ms from the end"
    );
    assert!((heard[middle] - written[middle] * std::f32::consts::FRAC_1_SQRT_2).abs() < 1.0e-4);
    for frame in end - fade..end {
        let level = (FRAC_PI_2 * (end - frame) as f64 / fade as f64).sin() as f32;
        assert!(
            (heard[frame] - written[frame] * level).abs() < 1.0e-4,
            "frame {frame}"
        );
    }
    assert!(
        heard[end - 1].abs() < 1.0e-3,
        "the last frame is all but silent"
    );
    assert!(heard[end..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn track_volume_scales_a_clip_and_multiplies_its_volume_keys() {
    let _heavy = crate::heavy_test();
    if !decoder_available("track volume") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let written = tone(frames(0.5));
    let asset = wave(&temp.path().join("tone.wav"), &written);
    let mut project = project_of(&[&asset], vec![Clip::from_asset(&asset, Time::ZERO)]);
    Arc::make_mut(&mut project.tracks[0]).gain = 0.5;
    for (frame, heard) in played(&mut mixer(&project), 4).iter().enumerate() {
        assert_eq!(*heard, written[frame] * 0.5, "frame {frame}");
    }

    // Keys from 0 to 2 over 200 ms under a track at one half: the product.
    let clip = &mut Arc::make_mut(&mut project.tracks[0]).clips[0];
    clip.gain = 0.0;
    clip.animation.push(volume_keys(&[(0.0, 0.0), (0.2, 2.0)]));
    let heard = played(&mut mixer(&project), 8);
    let ramp = frames(0.2);
    for frame in 0..ramp.min(heard.len()) {
        let level = 0.5 * 2.0 * frame as f32 / ramp as f32;
        assert!(
            (heard[frame] - written[frame] * level).abs() < 1.0e-4,
            "frame {frame}"
        );
    }
    assert!(heard[ramp / 2].abs() > 0.05 || written[ramp / 2].abs() < 0.1);

    // And with a fade in over the same stretch, all three.
    Arc::make_mut(&mut project.tracks[0]).clips[0].fade_in = Time::from_seconds(0.1);
    let heard = played(&mut mixer(&project), 4);
    let fade = frames(0.1);
    for frame in 0..heard.len() {
        let level = 0.5
            * (2.0 * frame as f32 / ramp as f32)
            * (FRAC_PI_2 * (frame as f64 / fade as f64).min(1.0)).sin() as f32;
        assert!(
            (heard[frame] - written[frame] * level).abs() < 1.0e-4,
            "frame {frame}"
        );
    }
}

#[test]
fn solo_admits_only_soloed_tracks_and_starts_only_their_decoders() {
    let _heavy = crate::heavy_test();
    if !decoder_available("solo") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let quiet = wave(&temp.path().join("quiet.wav"), &vec![0.25; frames(0.25)]);
    let loud = wave(&temp.path().join("loud.wav"), &vec![0.5; frames(0.25)]);
    let mut project = project_of(
        &[&quiet, &loud],
        vec![
            Clip::from_asset(&quiet, Time::ZERO),
            Clip::from_asset(&loud, Time::ZERO),
        ],
    );
    let ids: Vec<_> = project
        .tracks
        .iter()
        .map(|track| track.clips[0].id)
        .collect();
    let heard = |project: &Project| {
        let mut mixer = mixer(project);
        let samples = mixer.chunk(Time::ZERO, CHUNK_FRAMES).unwrap();
        assert!(samples.iter().all(|sample| *sample == samples[0]));
        (samples[0], started(&mixer))
    };
    assert_eq!(
        heard(&project),
        (0.75, {
            let mut both = ids.clone();
            both.sort_unstable();
            both
        })
    );

    Arc::make_mut(&mut project.tracks[0]).solo = true;
    assert_eq!(heard(&project), (0.25, vec![ids[0]]));
    Arc::make_mut(&mut project.tracks[1]).solo = true;
    Arc::make_mut(&mut project.tracks[0]).solo = false;
    assert_eq!(heard(&project), (0.5, vec![ids[1]]));

    // Mute wins over solo, and the muted solo still solos the other out.
    Arc::make_mut(&mut project.tracks[1]).muted = true;
    assert_eq!(heard(&project), (0.0, vec![]));
}

#[test]
fn a_soloed_picture_whose_linked_audio_clip_is_soloed_out_stays_silent() {
    let _heavy = crate::heavy_test();
    if !decoder_available("soloed picture") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    // Only the sound of a video is read here, so a sound file stands for it.
    let mut shot = wave(&temp.path().join("shot.wav"), &vec![0.25; frames(0.25)]);
    shot.kind = MediaKind::Video;
    shot.width = 16;
    shot.height = 16;
    shot.has_audio = true;
    let mut project = Project::new("Soloed picture fixture");
    Arc::make_mut(&mut project.assets).insert(shot.id, shot.clone());
    let picture = Clip::from_asset(&shot, Time::ZERO);
    let mut sound = picture.clone();
    sound.id = ClipId::new_v4();
    let (picture_id, sound_id) = (picture.id, sound.id);
    Arc::make_mut(&mut project.tracks[0]).clips.push(picture);
    Arc::make_mut(&mut project.tracks[1]).clips.push(sound);
    let heard = |project: &Project| {
        let mut mixer = mixer(project);
        let samples = mixer.chunk(Time::ZERO, CHUNK_FRAMES).unwrap();
        assert!(samples.iter().all(|sample| *sample == samples[0]));
        (samples[0], started(&mixer))
    };
    // Soloed and not linked, the picture plays its own sound.
    Arc::make_mut(&mut project.tracks[0]).solo = true;
    assert_eq!(heard(&project), (0.25, vec![picture_id]));

    // Linked, the audio clip owns the sound and its track is soloed out.
    let link = Some(ClipId::new_v4());
    Arc::make_mut(&mut project.tracks[0]).clips[0].linked_id = link;
    Arc::make_mut(&mut project.tracks[1]).clips[0].linked_id = link;
    assert_eq!(heard(&project), (0.0, vec![]));

    // Soloing the owner's track instead is what brings the sound back.
    Arc::make_mut(&mut project.tracks[0]).solo = false;
    Arc::make_mut(&mut project.tracks[1]).solo = true;
    assert_eq!(heard(&project), (0.25, vec![sound_id]));
}

#[test]
fn the_decoders_that_start_are_exactly_the_clips_the_core_says_sound() {
    let _heavy = crate::heavy_test();
    if !decoder_available("admission") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let sound = wave(&temp.path().join("sound.wav"), &vec![0.03; frames(0.25)]);
    let mut shot = wave(&temp.path().join("shot.wav"), &vec![0.03; frames(0.25)]);
    shot.kind = MediaKind::Video;
    (shot.width, shot.height, shot.has_audio) = (16, 16, true);
    // A video without sound: nothing of it is ever opened.
    let mut silent = Asset::new(
        temp.path().join("never-created.mp4"),
        MediaKind::Video,
        Time::from_seconds(0.25),
    );
    (silent.width, silent.height) = (16, 16);

    let mut project = Project::new("Admission fixture");
    for asset in [&sound, &shot, &silent] {
        Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    }
    project.tracks.clear();
    let mut add = |name: &str, kind, clip: Clip, change: fn(&mut Track)| {
        let mut track = Track::new(name, kind);
        let id = clip.id;
        track.clips.push(clip);
        change(&mut track);
        project.tracks.push(Arc::new(track));
        id
    };
    let link = Some(ClipId::new_v4());
    let linked = |asset: &Asset| {
        let mut clip = Clip::from_asset(asset, Time::ZERO);
        clip.linked_id = link;
        clip
    };
    let mut silenced = Clip::from_asset(&sound, Time::ZERO);
    silenced.gain = 0.0;
    let mut keyed = silenced.clone();
    keyed.id = ClipId::new_v4();
    keyed.animation.push(volume_keys(&[(0.0, 1.0)]));

    let open = add(
        "Open",
        TrackKind::Audio,
        Clip::from_asset(&sound, Time::ZERO),
        |_| {},
    );
    let muted = add(
        "Muted",
        TrackKind::Audio,
        Clip::from_asset(&sound, Time::ZERO),
        |track| {
            track.muted = true;
        },
    );
    let soloed = add(
        "Soloed",
        TrackKind::Audio,
        Clip::from_asset(&sound, Time::ZERO),
        |track| {
            track.solo = true;
        },
    );
    let at_zero = add(
        "At volume 0",
        TrackKind::Audio,
        Clip::from_asset(&sound, Time::ZERO),
        |track| {
            track.gain = 0.0;
        },
    );
    let picture = add("Detached picture", TrackKind::Video, linked(&shot), |_| {});
    let owner = add("Detached sound", TrackKind::Audio, linked(&shot), |_| {});
    let mute_video = add(
        "Silent video",
        TrackKind::Video,
        Clip::from_asset(&silent, Time::ZERO),
        |_| {},
    );
    let no_volume = add("Volume 0", TrackKind::Audio, silenced, |_| {});
    let keyed = add("Volume 0 with keys", TrackKind::Audio, keyed, |_| {});

    // Each choice of soloed tracks, the one above among them, and none.
    let solos: [&[usize]; 5] = [&[2], &[], &[0, 4, 7, 8], &[1, 2, 3, 6], &[4, 5, 8]];
    let expected: [Vec<ClipId>; 5] = [
        vec![soloed],
        vec![open, soloed, owner, keyed],
        vec![open, keyed],
        vec![soloed],
        vec![owner, keyed],
    ];
    for (solos, mut expected) in solos.into_iter().zip(expected) {
        for (index, track) in project.tracks.iter_mut().enumerate() {
            Arc::make_mut(track).solo = solos.contains(&index);
        }
        project.validate().unwrap();
        let mut sounding: Vec<_> = project
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter().map(move |clip| (track, clip)))
            .filter(|(track, clip)| sound::sounds(&project, track, clip))
            .map(|(_, clip)| clip.id)
            .collect();
        sounding.sort_unstable();
        expected.sort_unstable();
        assert_eq!(sounding, expected, "the core, soloing {solos:?}");

        let mut mixer = mixer(&project);
        mixer.chunk(Time::ZERO, CHUNK_FRAMES).unwrap();
        assert_eq!(started(&mixer), sounding, "the mixer, soloing {solos:?}");
    }
    for id in [muted, at_zero, picture, mute_video, no_volume] {
        assert!(!expected_ever(&project, id), "{id} never sounds here");
    }
}

/// Whether `id` sounds with no track soloed, which is the most a clip can.
fn expected_ever(project: &Project, id: ClipId) -> bool {
    let mut project = project.clone();
    for track in &mut project.tracks {
        Arc::make_mut(track).solo = false;
    }
    project.tracks.iter().any(|track| {
        track
            .clips
            .iter()
            .any(|clip| clip.id == id && sound::sounds(&project, track, clip))
    })
}

#[test]
fn two_full_scale_tones_in_phase_are_turned_down_and_keep_their_shape() {
    let _heavy = crate::heavy_test();
    if !decoder_available("limiter") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let written = tone(frames(0.25));
    let asset = wave(&temp.path().join("tone.wav"), &written);
    let project = project_of(
        &[&asset],
        vec![
            Clip::from_asset(&asset, Time::ZERO),
            Clip::from_asset(&asset, Time::ZERO),
        ],
    );
    let mut mixer = mixer(&project);
    let heard = played(&mut mixer, 8);
    assert!(heard.iter().all(|sample| sample.abs() <= 1.0));
    assert!(heard.iter().any(|sample| sample.abs() > 0.99));

    // The sum is the tone at twice full scale. The clamp cut it flat at 1
    // for two thirds of every cycle; held near one half it is the tone again.
    let first_peak = written.iter().position(|sample| *sample > 0.999).unwrap();
    for frame in first_peak..heard.len() {
        assert!(
            (heard[frame] - written[frame]).abs() < 0.01,
            "frame {frame}"
        );
    }
    let flat = heard.iter().filter(|sample| sample.abs() == 1.0).count();
    assert!(
        flat * 10 < heard.len(),
        "{flat} of {} samples sit on the ceiling",
        heard.len()
    );
    assert!(mixer.limit < 0.51);

    // A seek or an underrun starts the mix again, and the limiter with it.
    mixer.clear();
    assert_eq!(mixer.limit, 1.0);
}

fn silent_mixer(channels: usize) -> Mixer {
    Mixer::new(
        Arc::new(Project::new("Master fixture")),
        1.0,
        RATE,
        channels,
        Processes::default(),
    )
}

#[test]
fn the_master_leaves_a_mix_within_the_ceiling_bit_for_bit() {
    let mut mixer = silent_mixer(2);
    let mix = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.999_999_9,
        -0.25,
        f32::MIN_POSITIVE,
        0.5,
    ];
    let mut output = mix;
    mixer.master(&mut output);
    for (after, before) in output.iter().zip(&mix) {
        assert_eq!(after.to_bits(), before.to_bits());
    }
    assert_eq!(mixer.limit, 1.0);
}

#[test]
fn the_master_drops_at_once_holds_every_channel_and_lets_go_over_200_ms() {
    let mut mixer = silent_mixer(2);
    // The loudest channel sets the gain of the whole frame.
    let mut output = [2.0, -0.5];
    mixer.master(&mut output);
    assert_eq!(output, [1.0, -0.25]);
    assert_eq!(mixer.limit, 0.5);

    // A louder frame while it is held drops it further, at once.
    let mut output = [0.5, -4.0];
    mixer.master(&mut output);
    assert_eq!(output, [0.125, -1.0]);
    assert_eq!(mixer.limit, 0.25);

    // One time constant later the gain has covered 1 - 1/e of the way back.
    mixer.limit = 0.5;
    let mut quiet = vec![0.5_f32; frames(0.2) * 2];
    mixer.master(&mut quiet);
    let after = 1.0 - 0.5 * (-1.0_f32).exp();
    assert!((mixer.limit - after).abs() < 1.0e-3, "{}", mixer.limit);
    assert!(
        quiet.windows(4).all(|pair| pair[0] <= pair[2]),
        "it only rises"
    );
    assert!(quiet[0] > 0.25 && *quiet.last().unwrap() < 0.5);

    // And then it is exactly 1 again, and the mix is untouched from there on.
    let mut quiet = vec![0.5_f32; frames(3.0) * 2];
    mixer.master(&mut quiet);
    assert_eq!(mixer.limit, 1.0);
    assert_eq!(quiet.last().unwrap().to_bits(), 0.5_f32.to_bits());
    let mut output = [0.3, -0.7];
    mixer.master(&mut output);
    assert_eq!(output, [0.3, -0.7]);
}

#[test]
fn the_master_never_passes_the_ceiling_and_a_reset_opens_it() {
    let mut mixer = silent_mixer(1);
    // A rising gain meets a frame it would lift past the ceiling.
    let mut output: Vec<f32> = (0..frames(1.0))
        .map(|frame| if frame % 100 == 0 { 3.0 } else { 1.5 })
        .collect();
    mixer.master(&mut output);
    assert!(output.iter().all(|sample| sample.abs() <= 1.0));
    assert!(mixer.limit < 0.51);
    mixer.clear();
    assert_eq!(mixer.limit, 1.0);

    // A sample that is not finite is left to the clamp and moves no gain.
    let mut output = [f32::INFINITY, 0.5, f32::NEG_INFINITY];
    mixer.master(&mut output);
    assert_eq!(output, [1.0, 0.5, -1.0]);
    assert_eq!(mixer.limit, 1.0);
}

#[test]
fn the_master_turns_down_what_follows_a_peak_until_the_gain_is_back() {
    // A frame twice the ceiling, then a mix well within it: every frame is
    // turned down until the gain is exactly 1, well past the time constant.
    let mut mixer = silent_mixer(1);
    mixer.master(&mut [2.0]);
    assert_eq!(mixer.limit, 0.5);
    let mut held = 0_usize;
    let mut straight = 0_usize;
    while mixer.limit < 1.0 {
        // The floor on the step takes over for the last stretch.
        if (1.0 - mixer.limit) * mixer.release < f32::EPSILON {
            straight += 1;
        }
        let mut output = [0.25];
        mixer.master(&mut output);
        assert!(
            output[0] < 0.25 || mixer.limit == 1.0,
            "frame {held} left untouched at gain {}",
            mixer.limit
        );
        held += 1;
        assert!(held <= frames(2.0), "the gain never came back");
    }
    assert!(
        (frames(1.3)..=frames(1.5)).contains(&held),
        "{held} frames turned down"
    );
    assert!(
        straight.abs_diff(frames(LIMITER_RELEASE_SECONDS)) <= 2,
        "{straight} frames on the straight line"
    );
    let mut output = [0.25];
    mixer.master(&mut output);
    assert_eq!(output[0].to_bits(), 0.25_f32.to_bits());
}
