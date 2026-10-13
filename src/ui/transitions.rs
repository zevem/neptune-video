//! Transitions as the interface handles them: what a kind is called, the
//! one a new edge is given, and the clip and edge a request belongs to.
//!
//! Nothing here draws. The timeline's markings, the library's tiles, the
//! menus and the command line all make their commands from these.
use crate::rules;
use editor_core::{
    Clip, ClipId, Command, Ease, Project, Time, Track, TrackKind, Transition, TransitionEdge,
    TransitionKind, look,
};

pub const KINDS: [TransitionKind; 14] = [
    TransitionKind::Dissolve,
    TransitionKind::Fade,
    TransitionKind::SlideLeft,
    TransitionKind::SlideRight,
    TransitionKind::SlideUp,
    TransitionKind::SlideDown,
    TransitionKind::WipeLeft,
    TransitionKind::WipeRight,
    TransitionKind::WipeUp,
    TransitionKind::WipeDown,
    TransitionKind::ZoomIn,
    TransitionKind::ZoomOut,
    TransitionKind::Blur,
    TransitionKind::Glitch,
];

/// The transition an edge is given when none is named.
pub const DEFAULT_KIND: TransitionKind = TransitionKind::Dissolve;
/// How long a transition is until the person keeps another length.
pub const DEFAULT_DURATION: Time = Time(500_000);

pub const EDGES: [TransitionEdge; 2] = [TransitionEdge::In, TransitionEdge::Out];

pub fn name(kind: TransitionKind) -> &'static str {
    match kind {
        TransitionKind::Fade => "Fade",
        TransitionKind::Dissolve => "Dissolve",
        TransitionKind::SlideLeft => "Slide left",
        TransitionKind::SlideRight => "Slide right",
        TransitionKind::SlideUp => "Slide up",
        TransitionKind::SlideDown => "Slide down",
        TransitionKind::ZoomIn => "Zoom in",
        TransitionKind::ZoomOut => "Zoom out",
        TransitionKind::WipeLeft => "Wipe left",
        TransitionKind::WipeRight => "Wipe right",
        TransitionKind::WipeUp => "Wipe up",
        TransitionKind::WipeDown => "Wipe down",
        TransitionKind::Glitch => "Glitch",
        TransitionKind::Blur => "Blur",
    }
}

/// The kind a person names: its name here, or the project's own word for it.
pub fn named(text: &str) -> Option<TransitionKind> {
    let wanted = text.trim().to_lowercase().replace(['_', '-'], " ");
    let wanted = match wanted.as_str() {
        "cross dissolve" | "crossfade" | "cross fade" => "dissolve",
        other => other,
    };
    (KINDS.into_iter()).find(|kind| name(*kind).to_lowercase() == wanted)
}

/// The end of a clip an edge is, in words.
pub fn edge_name(edge: TransitionEdge) -> &'static str {
    match edge {
        TransitionEdge::In => "start",
        TransitionEdge::Out => "end",
    }
}

pub fn slot(edge: TransitionEdge) -> usize {
    match edge {
        TransitionEdge::In => 0,
        TransitionEdge::Out => 1,
    }
}

pub fn of(clip: &Clip, edge: TransitionEdge) -> Option<&Transition> {
    match edge {
        TransitionEdge::In => clip.transition_in.as_ref(),
        TransitionEdge::Out => clip.transition_out.as_ref(),
    }
}

/// The edge of `clip` that `at` is nearer.
pub fn nearest_edge(clip: &Clip, at: Time) -> TransitionEdge {
    if (at - clip.start).0 <= (clip.end() - at).0 {
        TransitionEdge::In
    } else {
        TransitionEdge::Out
    }
}

/// The lengths a transition of `clip` may have: a canvas frame to the whole
/// clip or ten seconds. None when the clip is shorter than a frame.
pub fn lengths(project: &Project, clip: &Clip) -> Option<(Time, Time)> {
    let frame = rules::frame(project.settings.frame_rate);
    let longest = clip.duration.min(look::MAX_TRANSITION);
    (longest >= frame).then_some((frame, longest))
}

/// `wanted` as a length a transition of `clip` may have, in whole frames
/// where the clip has room for them.
pub fn fitted(project: &Project, clip: &Clip, wanted: Time) -> Option<Time> {
    let (shortest, longest) = lengths(project, clip)?;
    let whole = rules::quantize(wanted, project.settings.frame_rate);
    Some(whole.clamp(shortest, longest))
}

/// The clip that starts where `clip` ends on its track.
fn following<'a>(track: &'a Track, clip: &Clip) -> Option<&'a Clip> {
    (track.clips.iter()).find(|other| other.id != clip.id && other.start == clip.end())
}

/// The clip and edge a transition asked for at `edge` of `clip` belongs to.
/// A transition between two clips is the incoming clip's: the end of a clip
/// another one follows is that one's start.
pub fn place(
    project: &Project,
    clip: ClipId,
    edge: TransitionEdge,
) -> Result<(ClipId, TransitionEdge), String> {
    let (track, clip) = (project.clip(clip)).ok_or("That clip is no longer in the project.")?;
    if track.kind == TrackKind::Audio {
        return Err("A transition is for pictures and titles; sound has none.".to_owned());
    }
    Ok(match (edge, following(track, clip)) {
        (TransitionEdge::Out, Some(next)) => (next.id, TransitionEdge::In),
        _ => (clip.id, edge),
    })
}

/// The edge of each of `clips` nearest `playhead`: where a transition asked
/// for without pointing at an edge goes.
pub fn at_playhead(
    project: &Project,
    clips: &[ClipId],
    playhead: Time,
) -> Vec<(ClipId, TransitionEdge)> {
    (clips.iter())
        .filter_map(|id| project.clip(*id))
        .map(|(_, clip)| (clip.id, nearest_edge(clip, playhead)))
        .collect()
}

/// The transition a kind starts as: what the person kept as its default, or
/// half a second in a straight line with the kind's own values.
pub fn default_of(saved: &[Transition], kind: TransitionKind) -> Transition {
    (saved.iter())
        .find(|saved| saved.kind == kind)
        .cloned()
        .unwrap_or(Transition {
            kind,
            duration: DEFAULT_DURATION,
            ease: Ease::Linear,
            params: Vec::new(),
        })
}

/// `transition` as another kind: its length and easing stay, and so do the
/// values the new kind has too.
pub fn with_kind(transition: &Transition, kind: TransitionKind) -> Transition {
    let specs = look::transition_spec(kind);
    Transition {
        kind,
        duration: transition.duration,
        ease: transition.ease,
        params: (transition.params.iter())
            .filter(|param| {
                (specs.iter()).any(|spec| spec.key == param.key && spec.contains(param.value))
            })
            .copied()
            .collect(),
    }
}

pub fn set_command(clip: ClipId, edge: TransitionEdge, transition: Option<Transition>) -> Command {
    Command::SetTransition {
        clip_id: clip,
        edge,
        transition,
    }
}

/// What asking for transitions comes to.
#[derive(Debug)]
pub enum Asked {
    /// The one edge asked about has a transition already, and no kind was
    /// named: its marking is selected instead.
    Show(ClipId, TransitionEdge),
    /// The edit, and the marking to select once it is made: the one made,
    /// when there is only one.
    Make(Box<Command>, Option<(ClipId, TransitionEdge)>),
}

/// What [`super::Action::AddTransitions`] does with the project as it is.
pub fn asked(
    project: &Project,
    saved: &[Transition],
    places: &[(ClipId, TransitionEdge)],
    kind: Option<TransitionKind>,
) -> Result<Asked, String> {
    // A cut is its incoming clip's start, wherever it was asked for.
    let mut resolved = (places.iter()).filter_map(|(clip, edge)| place(project, *clip, *edge).ok());
    let only = match (resolved.next(), resolved.next()) {
        (Some(place), None) => Some(place),
        _ => None,
    };
    let there = only.filter(|(clip, edge)| {
        kind.is_none() && (project.clip(*clip)).is_some_and(|(_, clip)| of(clip, *edge).is_some())
    });
    match there {
        Some((clip, edge)) => Ok(Asked::Show(clip, edge)),
        None => Ok(Asked::Make(
            Box::new(add_command(project, saved, places, kind)?),
            only,
        )),
    }
}

/// One undo step that puts a transition on each of `places`. With a `kind`
/// an edge that has a transition keeps its length and easing and becomes
/// that kind; without one such an edge is left as it is. Says why when no
/// edge takes one.
pub fn add_command(
    project: &Project,
    saved: &[Transition],
    places: &[(ClipId, TransitionEdge)],
    kind: Option<TransitionKind>,
) -> Result<Command, String> {
    let mut commands: Vec<Command> = Vec::new();
    let mut placed: Vec<(ClipId, TransitionEdge)> = Vec::new();
    let mut refusal = None;
    for (clip, edge) in places {
        let (id, edge) = match place(project, *clip, *edge) {
            Ok(place) => place,
            Err(reason) => {
                refusal.get_or_insert(reason);
                continue;
            }
        };
        if placed.contains(&(id, edge)) {
            continue;
        }
        placed.push((id, edge));
        let Some((_, clip)) = project.clip(id) else {
            continue;
        };
        let transition = match (of(clip, edge), kind) {
            (Some(_), None) => {
                refusal.get_or_insert_with(|| {
                    format!("The {} of that clip has a transition.", edge_name(edge))
                });
                continue;
            }
            (Some(existing), Some(kind)) if existing.kind == kind => continue,
            (Some(existing), Some(kind)) => with_kind(existing, kind),
            (None, kind) => default_of(saved, kind.unwrap_or(DEFAULT_KIND)),
        };
        let Some(duration) = fitted(project, clip, transition.duration) else {
            refusal.get_or_insert("That clip is too short for a transition.".to_owned());
            continue;
        };
        commands.push(set_command(
            id,
            edge,
            Some(Transition {
                duration,
                ..transition
            }),
        ));
    }
    match commands.len() {
        0 => Err(refusal.unwrap_or_else(|| {
            if places.is_empty() {
                "Select a clip to give a transition to.".to_owned()
            } else {
                "That transition is already there.".to_owned()
            }
        })),
        1 => Ok(commands.remove(0)),
        _ => Ok(Command::Batch {
            label: "Add transitions".to_owned(),
            commands,
        }),
    }
}

/// What the person kept as defaults, with `transition` as its kind's.
pub fn saved_with(saved: &[Transition], transition: Transition) -> Vec<Transition> {
    let mut kept: Vec<Transition> = (saved.iter())
        .filter(|saved| saved.kind != transition.kind)
        .cloned()
        .collect();
    kept.insert(0, transition);
    kept
}

/// Kept defaults a file may hold: one of each kind at most, each with a
/// length, an easing and values a clip long enough would accept.
pub fn sanitized(saved: Vec<Transition>) -> Vec<Transition> {
    let mut kept: Vec<Transition> = Vec::new();
    for transition in saved {
        let sound = transition.validate(Time(1), look::MAX_TRANSITION)
            && kept.iter().all(|kept| kept.kind != transition.kind);
        if sound {
            kept.push(transition);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Editor, ParamKey, ParamValue};
    use std::sync::Arc;

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    /// Two titles end to end on the text track and a third apart from them.
    fn cut() -> (Project, [ClipId; 3]) {
        let mut project = Project::new("Cut");
        let clips = [(0.0, 2.0), (2.0, 4.0), (6.0, 8.0)]
            .map(|(start, end)| Clip::text("Title", seconds(start), seconds(end - start)));
        let ids = [clips[0].id, clips[1].id, clips[2].id];
        let track = (project.tracks.iter_mut())
            .find(|track| track.kind == TrackKind::Text)
            .unwrap();
        Arc::make_mut(track).clips.extend(clips);
        project.validate().unwrap();
        (project, ids)
    }

    #[test]
    fn every_kind_is_listed_once_and_named_apart() {
        for (index, kind) in KINDS.into_iter().enumerate() {
            assert!(KINDS[..index].iter().all(|other| *other != kind));
            assert_eq!(named(name(kind)), Some(kind));
            let word = serde_json::to_value(kind).unwrap();
            assert_eq!(named(word.as_str().unwrap()), Some(kind), "{word}");
        }
        assert_eq!(named("crossfade"), Some(TransitionKind::Dissolve));
        assert_eq!(named("star wipe"), None);
    }

    #[test]
    fn the_end_of_a_clip_another_follows_is_that_ones_start() {
        let (project, [first, second, apart]) = cut();
        let out = TransitionEdge::Out;
        assert_eq!(
            place(&project, first, out),
            Ok((second, TransitionEdge::In))
        );
        assert_eq!(place(&project, second, out), Ok((second, out)));
        assert_eq!(
            place(&project, apart, TransitionEdge::In),
            Ok((apart, TransitionEdge::In))
        );
    }

    #[test]
    fn a_new_transition_is_the_default_kind_fitted_to_its_clip() {
        let (project, [first, second, _]) = cut();
        let mut editor = Editor::new(project).unwrap();
        let places = [(first, TransitionEdge::Out)];
        let command = add_command(editor.project(), &[], &places, None).unwrap();
        editor.dispatch(command).unwrap();
        let made = editor.project().clip(second).unwrap().1;
        let made = made.transition_in.clone().unwrap();
        assert_eq!(made.kind, DEFAULT_KIND);
        assert_eq!(made.duration, seconds(0.5));
        // Asked for again without a kind, the edge is left as it is.
        let again = add_command(editor.project(), &[], &places, None);
        assert!(again.unwrap_err().contains("has a transition"));
        // With a kind it becomes that kind and keeps its length.
        let wipe = Some(TransitionKind::WipeLeft);
        let command = add_command(editor.project(), &[], &places, wipe).unwrap();
        editor.dispatch(command).unwrap();
        let made = editor.project().clip(second).unwrap().1;
        let made = made.transition_in.clone().unwrap();
        assert_eq!(
            (made.kind, made.duration),
            (TransitionKind::WipeLeft, seconds(0.5))
        );
    }

    #[test]
    fn a_kept_default_is_what_its_kind_starts_as_and_is_cut_to_the_clip() {
        let (project, [_, _, apart]) = cut();
        let kept = Transition {
            kind: TransitionKind::ZoomIn,
            duration: seconds(5.0),
            ease: crate::ui::EASINGS[4].1,
            params: vec![ParamValue {
                key: ParamKey::Amount,
                value: 1.5,
            }],
        };
        let saved = saved_with(&[], kept.clone());
        let places = [(apart, TransitionEdge::In)];
        let zoom = Some(TransitionKind::ZoomIn);
        let command = add_command(&project, &saved, &places, zoom).unwrap();
        let Command::SetTransition {
            transition: Some(made),
            ..
        } = command
        else {
            panic!("one transition is one command");
        };
        // The clip is two seconds long.
        assert_eq!(made.duration, seconds(2.0));
        assert_eq!((made.ease, &made.params), (kept.ease, &kept.params));
        // Another kind is not this default.
        assert_eq!(default_of(&saved, DEFAULT_KIND).duration, DEFAULT_DURATION);
        // A kind is kept once, the newest first.
        let again = saved_with(&saved, default_of(&[], TransitionKind::ZoomIn));
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].duration, DEFAULT_DURATION);
    }

    #[test]
    fn several_edges_are_one_step_and_a_cut_is_given_one_transition() {
        let (project, [first, second, apart]) = cut();
        let mut editor = Editor::new(project).unwrap();
        // The end of the first clip and the start of the second are one cut.
        let places = [
            (first, TransitionEdge::Out),
            (second, TransitionEdge::In),
            (apart, TransitionEdge::Out),
        ];
        let command = add_command(editor.project(), &[], &places, None).unwrap();
        let Command::Batch { commands, .. } = &command else {
            panic!("two edges are a batch");
        };
        assert_eq!(commands.len(), 2);
        let steps = editor.history().count();
        editor.dispatch(command).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        assert!(editor.undo());
        let clip = |id| editor.project().clip(id).unwrap().1;
        assert!(clip(second).transition_in.is_none() && clip(apart).transition_out.is_none());
    }

    #[test]
    fn sound_takes_no_transition_and_says_so() {
        let mut project = Project::new("Sound");
        let asset = editor_core::Asset::new(
            "/media/a.wav".into(),
            editor_core::MediaKind::Audio,
            seconds(4.0),
        );
        let clip = Clip::from_asset(&asset, Time::ZERO);
        let id = clip.id;
        Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        let track = (project.tracks.iter_mut())
            .find(|track| track.kind == TrackKind::Audio)
            .unwrap();
        Arc::make_mut(track).clips.push(clip);
        let refused = add_command(&project, &[], &[(id, TransitionEdge::In)], None);
        assert!(refused.unwrap_err().contains("sound has none"));
        let none = add_command(&project, &[], &[], None);
        assert_eq!(none.unwrap_err(), "Select a clip to give a transition to.");
    }

    #[test]
    fn a_change_of_kind_keeps_only_the_values_the_new_kind_has() {
        let zoom = Transition {
            kind: TransitionKind::ZoomIn,
            duration: seconds(1.0),
            ease: Ease::Linear,
            params: vec![ParamValue {
                key: ParamKey::Amount,
                value: 3.0,
            }],
        };
        // A glitch has an amount too, but none as large as that.
        assert!(with_kind(&zoom, TransitionKind::Glitch).params.is_empty());
        assert_eq!(
            with_kind(&zoom, TransitionKind::ZoomOut).params,
            zoom.params
        );
        assert!(with_kind(&zoom, TransitionKind::Dissolve).params.is_empty());
    }

    #[test]
    fn kept_defaults_that_no_clip_would_take_are_dropped() {
        let sound = default_of(&[], TransitionKind::Blur);
        let mut long = default_of(&[], TransitionKind::Fade);
        long.duration = seconds(11.0);
        let mut foreign = default_of(&[], TransitionKind::Dissolve);
        foreign.params.push(ParamValue {
            key: ParamKey::Amount,
            value: 0.5,
        });
        let kept = sanitized(vec![sound.clone(), sound.clone(), long, foreign]);
        assert_eq!(kept, [sound]);
    }
}

#[cfg(test)]
#[path = "transitions_tests.rs"]
mod panes;
