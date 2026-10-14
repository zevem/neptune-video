//! Fades as the interface handles them: the clips that take one, the room an
//! edge has, and the edit a handle, a field, a menu row or a line of the
//! command panel asks for.
//!
//! Nothing here draws. The timeline's handles, the inspector's rows, the
//! clip menu and the command line all make their commands from these.
use editor_core::{Clip, ClipId, Command, MediaKind, Project, Time, sound};

/// The edge of a clip a fade is held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FadeEdge {
    In,
    Out,
}

pub const EDGES: [FadeEdge; 2] = [FadeEdge::In, FadeEdge::Out];

/// How long a fade asked for without a length is.
pub const QUICK: Time = Time(500_000);

/// What stands where a picture clip's sound would be edited when its linked
/// audio clip owns that sound, and what a fade asked of such a clip is told.
pub const OWNED: &str = "Sound is on the linked audio clip";

/// What is asked of the fades of some clips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// This edge fades for this long, held to the room the other leaves.
    Set(FadeEdge, Time),
    /// Neither edge fades.
    Remove,
}

/// The rows of the clip menu.
pub const MENU: [(&str, Asked); 3] = [
    ("Fade in 0.5 s", Asked::Set(FadeEdge::In, QUICK)),
    ("Fade out 0.5 s", Asked::Set(FadeEdge::Out, QUICK)),
    ("Remove fades", Asked::Remove),
];

pub fn name(edge: FadeEdge) -> &'static str {
    match edge {
        FadeEdge::In => "Fade in",
        FadeEdge::Out => "Fade out",
    }
}

pub fn of(clip: &Clip, edge: FadeEdge) -> Time {
    match edge {
        FadeEdge::In => clip.fade_in,
        FadeEdge::Out => clip.fade_out,
    }
}

fn other(edge: FadeEdge) -> FadeEdge {
    match edge {
        FadeEdge::In => FadeEdge::Out,
        FadeEdge::Out => FadeEdge::In,
    }
}

/// The longest the fade at `edge` may be: the clip, less its other fade.
pub fn room(clip: &Clip, edge: FadeEdge) -> Time {
    (clip.duration - of(clip, other(edge))).max(Time::ZERO)
}

/// Whether the clip's media has sound, so that the project lets it fade.
pub fn has_sound(project: &Project, clip: &Clip) -> bool {
    (clip.asset_id())
        .and_then(|asset| project.assets.get(&asset))
        .is_some_and(|asset| asset.has_audio || asset.kind == MediaKind::Audio)
}

/// Why `clip` is given no fade, when it is given none. A picture clip whose
/// sound its linked audio clip owns is refused though the project would take
/// the fade: it would enter the history and never be heard.
pub fn refusal(project: &Project, clip: ClipId) -> Option<String> {
    let Some((track, clip)) = project.clip(clip) else {
        return Some("The clip no longer exists.".to_owned());
    };
    if !has_sound(project, clip) {
        return Some("That clip has no sound to fade.".to_owned());
    }
    if sound::owned(project, clip) {
        return Some(format!("{OWNED}."));
    }
    track
        .locked
        .then(|| "Unlock the clip's track to fade its sound.".to_owned())
}

/// The command that gives one edge of `clip` a fade of this length and
/// leaves the other as it is.
pub fn set_command(clip: &Clip, edge: FadeEdge, length: Time) -> Command {
    let (fade_in, fade_out) = match edge {
        FadeEdge::In => (length, clip.fade_out),
        FadeEdge::Out => (clip.fade_in, length),
    };
    Command::SetFades {
        clip_id: clip.id,
        fade_in,
        fade_out,
    }
}

/// One undo step that does what is asked to each of `clips` that takes a
/// fade. Says why when none of them is changed.
pub fn command(project: &Project, clips: &[ClipId], asked: Asked) -> Result<Command, String> {
    let mut commands: Vec<Command> = Vec::new();
    let mut refused = None;
    for id in clips {
        if let Some(reason) = refusal(project, *id) {
            refused.get_or_insert(reason);
            continue;
        }
        let Some((_, clip)) = project.clip(*id) else {
            continue;
        };
        let command = match asked {
            Asked::Set(edge, length) => {
                let room = room(clip, edge);
                if room.is_zero() && length > Time::ZERO {
                    refused.get_or_insert_with(|| {
                        "The clip's other fade leaves no room for that one.".to_owned()
                    });
                    continue;
                }
                set_command(clip, edge, length.clamp(Time::ZERO, room))
            }
            Asked::Remove => Command::SetFades {
                clip_id: clip.id,
                fade_in: Time::ZERO,
                fade_out: Time::ZERO,
            },
        };
        let unchanged = matches!(
            &command,
            Command::SetFades { fade_in, fade_out, .. }
                if (*fade_in, *fade_out) == (clip.fade_in, clip.fade_out)
        );
        if !unchanged {
            commands.push(command);
        }
    }
    match commands.len() {
        0 => Err(refused.unwrap_or_else(|| {
            match asked {
                _ if clips.is_empty() => "Select a clip first.",
                Asked::Set(..) => "That fade is already there.",
                Asked::Remove => "There are no fades to remove.",
            }
            .to_owned()
        })),
        1 => Ok(commands.remove(0)),
        _ => Ok(Command::Batch {
            label: match asked {
                Asked::Set(..) => "Change fades",
                Asked::Remove => "Remove fades",
            }
            .to_owned(),
            commands,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules;
    use editor_core::{Asset, Editor, TrackKind};

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    /// A four second clip of a video with sound on the first video track,
    /// a four second sound on the first audio track and a title.
    fn project() -> (Editor, [ClipId; 3]) {
        let mut editor = Editor::default();
        let mut shot = Asset::new("/media/shot.mp4".into(), MediaKind::Video, seconds(6.0));
        shot.has_audio = true;
        let voice = Asset::new("/media/voice.wav".into(), MediaKind::Audio, seconds(6.0));
        let track = |editor: &Editor, kind| {
            (editor.project().tracks.iter())
                .find(|track| track.kind == kind)
                .unwrap()
                .id
        };
        let mut clips = [
            Clip::from_asset(&shot, Time::ZERO),
            Clip::from_asset(&voice, Time::ZERO),
            Clip::text("Title", Time::ZERO, seconds(4.0)),
        ];
        for clip in &mut clips {
            clip.duration = seconds(4.0);
        }
        let ids = [clips[0].id, clips[1].id, clips[2].id];
        editor
            .dispatch(Command::AddAssets {
                assets: vec![shot, voice],
            })
            .unwrap();
        for (clip, kind) in
            clips
                .into_iter()
                .zip([TrackKind::Video, TrackKind::Audio, TrackKind::Text])
        {
            let track_id = track(&editor, kind);
            editor
                .dispatch(Command::AddClip { track_id, clip })
                .unwrap();
        }
        (editor, ids)
    }

    fn fades(editor: &Editor, clip: ClipId) -> (Time, Time) {
        let clip = editor.project().clip(clip).unwrap().1;
        (clip.fade_in, clip.fade_out)
    }

    #[test]
    fn a_fade_is_held_to_the_room_the_other_leaves_and_undoes() {
        let (mut editor, [_, voice, _]) = project();
        let ask = |editor: &Editor, asked| command(editor.project(), &[voice], asked);
        let made = ask(&editor, Asked::Set(FadeEdge::Out, seconds(1.0))).unwrap();
        editor.dispatch(made).unwrap();
        // Longer than the three seconds the fade out leaves.
        let made = ask(&editor, Asked::Set(FadeEdge::In, seconds(9.0))).unwrap();
        let steps = editor.history().count();
        editor.dispatch(made).unwrap();
        assert_eq!(fades(&editor, voice), (seconds(3.0), seconds(1.0)));
        assert_eq!(editor.history().count(), steps + 1);
        // Asked for again it is there already, and the fade out has no room.
        let again = ask(&editor, Asked::Set(FadeEdge::In, seconds(9.0)));
        assert_eq!(again.unwrap_err(), "That fade is already there.");
        let made = ask(&editor, Asked::Set(FadeEdge::In, seconds(4.0))).unwrap_err();
        assert_eq!(made, "That fade is already there.");
        assert!(editor.undo());
        assert_eq!(fades(&editor, voice), (Time::ZERO, seconds(1.0)));
        // A fade of the whole clip leaves the other no room, in words.
        let whole = ask(&editor, Asked::Set(FadeEdge::Out, seconds(4.0))).unwrap();
        editor.dispatch(whole).unwrap();
        let none = ask(&editor, Asked::Set(FadeEdge::In, QUICK)).unwrap_err();
        assert_eq!(none, "The clip's other fade leaves no room for that one.");
        let removed = ask(&editor, Asked::Remove).unwrap();
        editor.dispatch(removed).unwrap();
        assert_eq!(fades(&editor, voice), (Time::ZERO, Time::ZERO));
        let nothing = ask(&editor, Asked::Remove).unwrap_err();
        assert_eq!(nothing, "There are no fades to remove.");
    }

    #[test]
    fn several_clips_fade_in_one_step_and_those_without_sound_are_left() {
        let (mut editor, [shot, voice, title]) = project();
        let asked = Asked::Set(FadeEdge::In, QUICK);
        let made = command(editor.project(), &[shot, voice, title], asked).unwrap();
        assert!(matches!(&made, Command::Batch { commands, .. } if commands.len() == 2));
        let steps = editor.history().count();
        editor.dispatch(made).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        assert_eq!(fades(&editor, shot).0, QUICK);
        assert_eq!(fades(&editor, voice).0, QUICK);
        // A title alone, and nothing at all, are refused in words.
        let silent = command(editor.project(), &[title], asked).unwrap_err();
        assert_eq!(silent, "That clip has no sound to fade.");
        let nothing = command(editor.project(), &[], asked).unwrap_err();
        assert_eq!(nothing, "Select a clip first.");
    }

    #[test]
    fn a_picture_whose_sound_its_audio_clip_owns_is_refused_until_it_is_unlinked() {
        let (mut editor, [shot, ..]) = project();
        let detach = rules::detach_audio(editor.project(), shot).unwrap();
        editor.dispatch(detach).unwrap();
        let asked = Asked::Set(FadeEdge::In, QUICK);
        let refused = command(editor.project(), &[shot], asked).unwrap_err();
        assert_eq!(refused, "Sound is on the linked audio clip.");
        // The audio clip that owns the sound takes the fade.
        let sound = (editor.project().tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .find(|clip| clip.linked_id.is_some())
            .unwrap()
            .id;
        assert_eq!(refusal(editor.project(), sound), None);
        // The project itself would take the picture's: it is the interface
        // that does not ask.
        let (_, picture) = editor.project().clip(shot).unwrap();
        let direct = set_command(picture, FadeEdge::In, QUICK);
        editor.dispatch(direct).unwrap();
        assert!(editor.undo());
        editor
            .dispatch(Command::UnlinkClips {
                clip_ids: vec![shot],
            })
            .unwrap();
        assert_eq!(refusal(editor.project(), shot), None);
    }

    #[test]
    fn a_locked_track_is_refused_in_words_and_by_the_project() {
        let (mut editor, [_, voice, _]) = project();
        let (track, clip) = editor.project().clip(voice).unwrap();
        let (track_id, direct) = (track.id, set_command(clip, FadeEdge::Out, QUICK));
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
        let asked = Asked::Set(FadeEdge::Out, QUICK);
        let refused = command(editor.project(), &[voice], asked).unwrap_err();
        assert_eq!(refused, "Unlock the clip's track to fade its sound.");
        let before = editor.project().clone();
        assert!(editor.dispatch(direct).is_err());
        assert_eq!(*editor.project(), before);
    }

    #[test]
    fn the_menu_rows_say_the_length_they_give() {
        for (label, asked) in MENU {
            match asked {
                Asked::Set(edge, length) => {
                    assert_eq!(length, QUICK);
                    let said = format!("{} {:.1} s", name(edge), length.as_seconds());
                    assert_eq!(label, said);
                }
                Asked::Remove => assert_eq!(label, "Remove fades"),
            }
        }
    }
}
