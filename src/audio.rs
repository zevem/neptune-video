//! The audio effects a person gives a clip on an audio track: one kind, the
//! Voice and 3-band EQ presets, and an amount of noise reduction.
//!
//! The inspector, the library and the command panel make them here, each as
//! one step of the history, and refuse in the same words. A preset is not a
//! type: it writes ordinary effects through `SetEffects`, and nothing
//! remembers that it made them. Pitch is not given here; its home is the
//! Playback card's "Keep pitch" and the pitch phrases (`crate::speed`).
use crate::ui::fades;
use editor_core::{
    Clip, ClipId, Command, Effect, EffectKind, ParamKey, ParamValue, Project, TrackKind, look,
    sound,
};

/// What a picture clip with sound is told: its sound takes audio effects
/// once it is on an audio track.
pub const DETACH_FIRST: &str = "Detach audio first: audio effects go on clips on audio tracks.";
/// What a picture clip is told once its sound is detached: the linked clip
/// on an audio track takes them, in the words a fade asked of it is given
/// (`fades::OWNED`).
pub const ON_LINKED: &str = "Sound is on the linked audio clip: give it audio effects there.";
/// What a kind the session's FFmpeg does not run says, in its card.
///
/// It is never a refusal: the menu, a preset, a tile, a phrase and an
/// agent's serialized command all add it alike, since the project is not
/// held to one machine's FFmpeg and `editor-agent` cannot see the probe.
/// Added, its card says this and disables its rows, and the mixer leaves it
/// out of the sound.
pub const UNAVAILABLE: &str = "Not available in this FFmpeg build";

/// The audio kinds a person adds one at a time, in the order of the table.
pub fn added() -> impl Iterator<Item = EffectKind> {
    (EffectKind::ALL.into_iter()).filter(|kind| kind.is_audio() && *kind != EffectKind::Pitch)
}

/// Whether `kind` is one the session's FFmpeg does not run. `missing` is the
/// probe's answer, `None` while it has not finished: then every kind is
/// taken to run.
pub fn unavailable(kind: EffectKind, missing: Option<&[EffectKind]>) -> bool {
    missing.is_some_and(|missing| missing.contains(&kind))
}

/// Ordinary effects written together by one name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    /// A high-pass at 80 Hz, noise reduction at its defaults, a presence
    /// band of +3 dB at 3.5 kHz one octave wide and a compressor at its
    /// defaults.
    Voice,
    /// Three flat bands at 120 Hz, 1 kHz and 8 kHz, to be set by hand.
    ThreeBandEq,
}

impl Preset {
    /// In the order the add menu lists them.
    pub const ALL: [Self; 2] = [Self::Voice, Self::ThreeBandEq];

    pub fn name(self) -> &'static str {
        match self {
            Self::Voice => "Voice",
            Self::ThreeBandEq => "3-band EQ",
        }
    }

    /// Its effects, each made now with an id of its own.
    pub fn effects(self) -> Vec<Effect> {
        let band = |hz: f32, gain: f32| {
            with_values(
                EffectKind::EqBand,
                &[
                    (ParamKey::Frequency, hz),
                    (ParamKey::Gain, gain),
                    (ParamKey::Width, 1.0),
                ],
            )
        };
        match self {
            Self::Voice => vec![
                with_values(EffectKind::HighPass, &[(ParamKey::Frequency, 80.0)]),
                Effect::new(EffectKind::Denoise),
                band(3_500.0, 3.0),
                Effect::new(EffectKind::Compressor),
            ],
            Self::ThreeBandEq => vec![band(120.0, 0.0), band(1_000.0, 0.0), band(8_000.0, 0.0)],
        }
    }
}

fn with_values(kind: EffectKind, values: &[(ParamKey, f32)]) -> Effect {
    let mut effect = Effect::new(kind);
    for (key, value) in values {
        set_value(&mut effect, *key, *value);
    }
    effect
}

fn set_value(effect: &mut Effect, key: ParamKey, value: f32) {
    effect.params.retain(|param| param.key != key);
    effect.params.push(ParamValue { key, value });
}

/// What is given to a clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Given {
    Kind(EffectKind),
    Preset(Preset),
}

impl Given {
    fn effects(self) -> Vec<Effect> {
        match self {
            Self::Kind(kind) => vec![Effect::new(kind)],
            Self::Preset(preset) => preset.effects(),
        }
    }
}

/// The clip on an audio track that takes audio effects, or why `clip_id` is
/// not one: sound on an audio track whose track is not locked. A picture
/// whose sound is already detached is pointed to its linked clip, not told
/// to detach it again.
pub fn target(project: &Project, clip_id: ClipId) -> Result<&Clip, String> {
    let (track, clip) = (project.clip(clip_id)).ok_or("The clip no longer exists.")?;
    let has_sound = fades::has_sound(project, clip);
    if track.kind != TrackKind::Audio {
        return Err(if sound::owned(project, clip) {
            ON_LINKED.to_owned()
        } else if has_sound {
            DETACH_FIRST.to_owned()
        } else {
            "Only a sound on an audio track takes an audio effect.".to_owned()
        });
    }
    if !has_sound {
        return Err("That clip has no sound.".to_owned());
    }
    if track.locked {
        return Err("Unlock the clip's track to change its audio effects.".to_owned());
    }
    Ok(clip)
}

/// `stack` with what is given at its end, or why it has no room: a clip
/// holds sixteen effects, and a preset goes whole or not at all.
pub fn with_given(stack: &[Effect], given: Given) -> Result<Vec<Effect>, String> {
    let effects = given.effects();
    let room = look::MAX_EFFECTS.saturating_sub(stack.len());
    if effects.len() > room {
        return Err(match given {
            Given::Kind(_) => format!(
                "That clip holds {} effects, the most it takes.",
                look::MAX_EFFECTS
            ),
            Given::Preset(preset) => format!(
                "{} adds {} effects and that clip has room for {room}; a clip holds {} at most.",
                preset.name(),
                effects.len(),
                look::MAX_EFFECTS
            ),
        });
    }
    let mut stack = stack.to_vec();
    stack.extend(effects);
    Ok(stack)
}

/// The one step a preset is, of the stack `effects` of `clip_id`.
pub fn preset_command(clip_id: ClipId, preset: Preset, effects: Vec<Effect>) -> Command {
    Command::Batch {
        label: format!("Add {}", preset.name()),
        commands: vec![Command::SetEffects { clip_id, effects }],
    }
}

/// One step that gives `given` to each of `clips` that takes it. A clip that
/// does not is passed over; when none does, the first one's reason is said.
pub fn give_command(project: &Project, clips: &[ClipId], given: Given) -> Result<Command, String> {
    each(project, clips, |clip| {
        with_given(&clip.effects, given).map(|effects| Command::SetEffects {
            clip_id: clip.id,
            effects,
        })
    })
    .map(|mut commands| match given {
        Given::Preset(preset) => Command::Batch {
            label: format!("Add {}", preset.name()),
            commands,
        },
        Given::Kind(_) if commands.len() == 1 => commands.remove(0),
        Given::Kind(_) => Command::Batch {
            label: "Add effect".to_owned(),
            commands,
        },
    })
}

/// One step that takes `amount` (`0..=1`) of the noise away in each of
/// `clips` that takes audio effects: the first enabled noise reduction is
/// set to it, and a clip with none is given one with its noise floor at the
/// default.
pub fn noise_command(project: &Project, clips: &[ClipId], amount: f32) -> Result<Command, String> {
    if !(0.0..=1.0).contains(&amount) {
        return Err("Noise reduction is from 0% to 100%.".to_owned());
    }
    let commands = each(project, clips, |clip| {
        let mut effects = clip.effects.clone();
        let reading = (effects.iter_mut())
            .find(|effect| effect.enabled && effect.kind == EffectKind::Denoise);
        match reading {
            Some(effect) => {
                let held = (effect.param(ParamKey::Amount))
                    .or_else(|| {
                        let target = editor_core::AnimTarget::Effect(EffectKind::Denoise);
                        look::param_spec(target, ParamKey::Amount).map(|spec| spec.default)
                    })
                    .unwrap_or_default();
                if held == amount {
                    return Err(format!(
                        "That clip already reduces noise by {:.0}%.",
                        amount * 100.0
                    ));
                }
                set_value(effect, ParamKey::Amount, amount);
            }
            None => {
                effects = with_given(&effects, Given::Kind(EffectKind::Denoise))?;
                if let Some(added) = effects.last_mut() {
                    set_value(added, ParamKey::Amount, amount);
                }
            }
        }
        Ok(Command::SetEffects {
            clip_id: clip.id,
            effects,
        })
    })?;
    Ok(Command::Batch {
        label: "Reduce noise".to_owned(),
        commands,
    })
}

/// The commands `make` gives for each of `clips` that takes audio effects,
/// or the first reason none was made.
fn each(
    project: &Project,
    clips: &[ClipId],
    mut make: impl FnMut(&Clip) -> Result<Command, String>,
) -> Result<Vec<Command>, String> {
    if clips.is_empty() {
        return Err("Select a clip first.".to_owned());
    }
    let mut commands = Vec::new();
    let mut refused = None;
    for id in clips {
        match target(project, *id).and_then(&mut make) {
            Ok(command) => commands.push(command),
            Err(reason) => {
                refused.get_or_insert(reason);
            }
        }
    }
    if commands.is_empty() {
        return Err(refused.unwrap_or_else(|| "Select a clip first.".to_owned()));
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Asset, Editor, MediaKind, Time, TrackId};

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    fn track(project: &Project, kind: TrackKind) -> TrackId {
        (project.tracks.iter())
            .find(|track| track.kind == kind)
            .unwrap()
            .id
    }

    /// A sound on the audio track, a video with sound on the video track
    /// after it, and a title: the three clips' ids, in that order.
    fn project() -> (Project, [ClipId; 3]) {
        let mut editor = Editor::default();
        let mut voice = Asset::new("/media/voice.wav".into(), MediaKind::Audio, seconds(6.0));
        voice.name = "voice.wav".to_owned();
        let mut shot = Asset::new("/media/shot.mp4".into(), MediaKind::Video, seconds(6.0));
        shot.name = "shot.mp4".to_owned();
        shot.has_audio = true;
        shot.width = 1920;
        shot.height = 1080;
        editor
            .dispatch(Command::AddAssets {
                assets: vec![voice.clone(), shot.clone()],
            })
            .unwrap();
        let sound = Clip::from_asset(&voice, Time::ZERO);
        let picture = Clip::from_asset(&shot, Time::ZERO);
        let title = Clip::text("Hi", Time::ZERO, seconds(2.0));
        let ids = [sound.id, picture.id, title.id];
        for (kind, clip) in [
            (TrackKind::Audio, sound),
            (TrackKind::Video, picture),
            (TrackKind::Text, title),
        ] {
            let track_id = track(editor.project(), kind);
            editor
                .dispatch(Command::AddClip { track_id, clip })
                .unwrap();
        }
        (editor.project().clone(), ids)
    }

    fn stack(project: &Project, id: ClipId) -> &[Effect] {
        &project.clip(id).unwrap().1.effects
    }

    fn values(effect: &Effect) -> Vec<(ParamKey, f32)> {
        (look::spec(effect.kind).iter())
            .map(|spec| (spec.key, effect.param(spec.key).unwrap_or(spec.default)))
            .collect()
    }

    #[test]
    fn the_presets_write_the_effects_the_spec_names_with_fresh_ids() {
        use EffectKind as K;
        use ParamKey as P;
        let voice = Preset::Voice.effects();
        let read: Vec<(EffectKind, Vec<(ParamKey, f32)>)> =
            voice.iter().map(|e| (e.kind, values(e))).collect();
        assert_eq!(
            read,
            [
                (K::HighPass, vec![(P::Frequency, 80.0)]),
                (K::Denoise, vec![(P::Amount, 0.4), (P::Threshold, -40.0)]),
                (
                    K::EqBand,
                    vec![(P::Frequency, 3_500.0), (P::Gain, 3.0), (P::Width, 1.0)]
                ),
                (
                    K::Compressor,
                    vec![(P::Threshold, -18.0), (P::Amount, 3.0), (P::Gain, 0.0)]
                ),
            ]
        );
        let bands: Vec<Vec<(ParamKey, f32)>> =
            Preset::ThreeBandEq.effects().iter().map(values).collect();
        assert_eq!(
            bands,
            [120.0, 1_000.0, 8_000.0].map(|hz| vec![
                (P::Frequency, hz),
                (P::Gain, 0.0),
                (P::Width, 1.0)
            ])
        );
        for preset in Preset::ALL {
            for effect in preset.effects() {
                assert!(effect.kind.is_audio() && effect.enabled && effect.mix == 1.0);
                assert!(effect.validate(), "{effect:?}");
            }
        }
        // Made again, every id is new.
        let again = Preset::Voice.effects();
        assert!(voice.iter().zip(&again).all(|(a, b)| a.id != b.id));
        // Pitch is the Playback card's.
        assert_eq!(added().count(), 6);
        assert!(added().all(|kind| kind.is_audio() && kind != EffectKind::Pitch));
    }

    #[test]
    fn a_preset_is_one_step_undone_whole_and_saved_as_ordinary_effects() {
        let (project, [sound, ..]) = project();
        let mut editor = Editor::new(project.clone()).unwrap();
        editor
            .dispatch(give_command(&project, &[sound], Given::Kind(EffectKind::Limiter)).unwrap())
            .unwrap();
        let steps = editor.history().count();
        let command = give_command(editor.project(), &[sound], Given::Preset(Preset::Voice));
        editor.dispatch(command.unwrap()).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        assert_eq!(editor.undo_label(), Some("Add Voice"));
        let kinds: Vec<EffectKind> = (stack(editor.project(), sound).iter())
            .map(|effect| effect.kind)
            .collect();
        use EffectKind as K;
        assert_eq!(
            kinds,
            [
                K::Limiter,
                K::HighPass,
                K::Denoise,
                K::EqBand,
                K::Compressor
            ]
        );
        // Saved and read back, the stack is what was written; nothing says
        // a preset made it.
        let text = serde_json::to_string(editor.project()).unwrap();
        assert!(!text.contains("Voice") && !text.contains("preset"));
        let read: Project = serde_json::from_str(&text).unwrap();
        read.validate().unwrap();
        assert_eq!(stack(&read, sound), stack(editor.project(), sound));
        // Twice is twice: nothing remembers the first.
        let twice = give_command(editor.project(), &[sound], Given::Preset(Preset::Voice));
        editor.dispatch(twice.unwrap()).unwrap();
        assert_eq!(stack(editor.project(), sound).len(), 9);
        assert!(editor.undo() && editor.undo());
        assert_eq!(stack(editor.project(), sound).len(), 1);
        assert!(editor.undo());
        assert_eq!(editor.project(), &project);
    }

    #[test]
    fn a_clip_that_takes_no_audio_effect_is_refused_in_words() {
        let (project, [sound, picture, title]) = project();
        let give = |project: &Project, clips: &[ClipId], given| {
            give_command(project, clips, given).unwrap_err()
        };
        let compressor = Given::Kind(EffectKind::Compressor);
        assert_eq!(give(&project, &[picture], compressor), DETACH_FIRST);
        assert_eq!(
            give(&project, &[title], Given::Preset(Preset::Voice)),
            "Only a sound on an audio track takes an audio effect."
        );
        assert_eq!(give(&project, &[], compressor), "Select a clip first.");
        // Of several, the ones that take it are given it.
        let both = give_command(&project, &[picture, sound], compressor).unwrap();
        assert!(matches!(&both, Command::SetEffects { clip_id, .. } if *clip_id == sound));

        let mut editor = Editor::new(project.clone()).unwrap();
        let track_id = track(&project, TrackKind::Audio);
        editor
            .dispatch(Command::SetTrackState {
                track_id,
                locked: Some(true),
                hidden: None,
                muted: None,
                name: None,
                gain: None,
                solo: None,
            })
            .unwrap();
        assert_eq!(
            give(editor.project(), &[sound], compressor),
            "Unlock the clip's track to change its audio effects."
        );

        // Thirteen leave room for three: a 3-band EQ, not a Voice.
        let thirteen: Vec<Effect> = (0..13).map(|_| Effect::new(EffectKind::Limiter)).collect();
        let mut full = Editor::new(project.clone()).unwrap();
        full.dispatch(Command::SetEffects {
            clip_id: sound,
            effects: thirteen,
        })
        .unwrap();
        let before = full.project().clone();
        assert_eq!(
            give(&before, &[sound], Given::Preset(Preset::Voice)),
            "Voice adds 4 effects and that clip has room for 3; a clip holds 16 at most."
        );
        let eq = give_command(&before, &[sound], Given::Preset(Preset::ThreeBandEq)).unwrap();
        full.dispatch(eq).unwrap();
        assert_eq!(stack(full.project(), sound).len(), 16);
        assert_eq!(
            give(full.project(), &[sound], compressor),
            "That clip holds 16 effects, the most it takes."
        );
        // Whatever the project would make of a stack past sixteen, it is
        // refused before it gets there and leaves the project as it was.
        let mut past = stack(full.project(), sound).to_vec();
        past.push(Effect::new(EffectKind::Limiter));
        let steps = full.history().count();
        let after = full.project().clone();
        assert!(
            full.dispatch(Command::SetEffects {
                clip_id: sound,
                effects: past,
            })
            .is_err()
        );
        assert_eq!(full.project(), &after);
        assert_eq!(full.history().count(), steps);
    }

    #[test]
    fn a_picture_whose_sound_is_detached_is_pointed_to_the_linked_clip() {
        let (project, [_, picture, _]) = project();
        let mut editor = Editor::new(project).unwrap();
        let detach = crate::rules::detach_audio(editor.project(), picture).unwrap();
        editor.dispatch(detach).unwrap();
        let detached = editor.project().clone();
        assert!(ON_LINKED.starts_with(fades::OWNED));
        // Not "Detach audio first": detaching it again is refused.
        assert!(crate::rules::detach_audio(&detached, picture).is_err());
        for given in [
            Given::Kind(EffectKind::Compressor),
            Given::Preset(Preset::Voice),
            Given::Preset(Preset::ThreeBandEq),
        ] {
            assert_eq!(
                give_command(&detached, &[picture], given).unwrap_err(),
                ON_LINKED,
                "{given:?}"
            );
        }
        assert_eq!(
            noise_command(&detached, &[picture], 0.6).unwrap_err(),
            ON_LINKED
        );
        // The linked sound, selected with it, is the one given them.
        let sound = (detached.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .find(|clip| clip.linked_id.is_some())
            .unwrap()
            .id;
        let both = give_command(
            &detached,
            &[picture, sound],
            Given::Kind(EffectKind::Limiter),
        );
        assert!(matches!(both, Ok(Command::SetEffects { clip_id, .. }) if clip_id == sound));
    }

    #[test]
    fn noise_reduction_is_set_where_it_is_and_added_where_it_is_not() {
        let (project, [sound, picture, _]) = project();
        let amount_of = |project: &Project| -> Vec<Option<f32>> {
            (stack(project, sound).iter())
                .filter(|effect| effect.kind == EffectKind::Denoise)
                .map(|effect| effect.param(ParamKey::Amount))
                .collect()
        };
        let mut editor = Editor::new(project.clone()).unwrap();
        editor
            .dispatch(noise_command(&project, &[sound], 0.6).unwrap())
            .unwrap();
        assert_eq!(editor.undo_label(), Some("Reduce noise"));
        assert_eq!(amount_of(editor.project()), [Some(0.6)]);
        let denoise = &stack(editor.project(), sound)[0];
        assert_eq!(values(denoise)[1], (ParamKey::Threshold, -40.0));
        // Said again, the one there is set: no second is added.
        let again = noise_command(editor.project(), &[sound], 0.25).unwrap();
        editor.dispatch(again).unwrap();
        assert_eq!(amount_of(editor.project()), [Some(0.25)]);
        assert_eq!(
            noise_command(editor.project(), &[sound], 0.25).unwrap_err(),
            "That clip already reduces noise by 25%."
        );
        assert!(editor.undo() && editor.undo());
        assert_eq!(editor.project(), &project);
        for (amount, refused) in [
            (1.5, "Noise reduction is from 0% to 100%."),
            (f32::NAN, "Noise reduction is from 0% to 100%."),
        ] {
            assert_eq!(
                noise_command(&project, &[sound], amount).unwrap_err(),
                refused
            );
        }
        assert_eq!(
            noise_command(&project, &[picture], 0.6).unwrap_err(),
            DETACH_FIRST
        );
    }

    #[test]
    fn a_kind_is_unavailable_only_once_the_probe_says_so() {
        for kind in EffectKind::ALL.into_iter().filter(|kind| kind.is_audio()) {
            assert!(!unavailable(kind, None), "{kind:?}");
            assert!(!unavailable(kind, Some(&[])), "{kind:?}");
        }
        let missing = [EffectKind::Denoise];
        assert!(unavailable(EffectKind::Denoise, Some(&missing)));
        assert!(!unavailable(EffectKind::Compressor, Some(&missing)));
    }
}
