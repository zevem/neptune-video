//! The command line of the agent panel.
//!
//! This is a small, exact language over the same actions the interface uses,
//! not a language model: every sentence it accepts maps to one documented
//! edit, and anything else is refused with what it expected. A line that
//! starts with `{` is a serialized core `Command` and is applied as is.
//! External agents do not come through here; they use the live session the
//! application hosts (see `editor_agent::AgentHost`).
use crate::{
    languages, rules, speed, timefmt,
    ui::{
        Action, Selection,
        browser::{self, Entry},
        captions_tab, inspector,
        library::Tab,
        timeline::marker_at,
        transitions, with_key,
    },
};
use editor_core::{
    BlendMode, CanvasSettings, Clip, ClipContent, Command, Effect, EffectKind, ParamKey,
    ParamValue, Project, TextAnimation, TextAnimationKind, Time, Track, TrackKind, Transform,
    Transition, TransitionEdge,
};

pub struct Context<'a> {
    pub project: &'a Project,
    pub selection: &'a Selection,
    pub playhead: Time,
    /// How the canvas read the picture of each layer it drew steadied: a
    /// point tracked on one is where that reading shows it.
    pub readings: &'a [editor_core::ClipOverride],
}

/// What the panel lists as examples. Each one is accepted by [`interpret`].
pub const EXAMPLES: [(&str, &str); 24] = [
    (
        "split",
        "Split the selected clips, or every clip, at the playhead",
    ),
    ("delete · ripple delete", "Remove the selected clips"),
    ("go to 1:30", "Move the playhead"),
    ("move 2s · move -10f", "Shift the selected clips"),
    ("trim to 5s", "Set the selected clip's length"),
    (
        "set opacity 50% · set scale 120%",
        "Change the selected clip",
    ),
    (
        "set rotation 15 · set position 100 -50",
        "Rotate or place it",
    ),
    ("set speed 2x · set volume 80%", "Change how it plays"),
    (
        "add effect glow · remove effect glow",
        "Effects on the selected clips",
    ),
    (
        "apply look cinematic 60% · set blend screen",
        "A look, or how the clip mixes with what is under it",
    ),
    (
        "add transition wipe left at start · remove transition",
        "On the edge nearest the playhead unless one is named",
    ),
    (
        "reverse · freeze frame for 2s",
        "Play it backwards, or hold the frame at the playhead",
    ),
    (
        "add keyframe opacity 50% · chroma key green",
        "Animate a value at the playhead, or key out a screen",
    ),
    (
        "follow Tracker 1 · stop following",
        "Go where a tracked region goes, from where the clip is",
    ),
    ("add text Hello for 3s", "Put a title at the playhead"),
    (
        "animate in pop · animate out fade for 1s",
        "How the selected titles come in or go; “none” takes it off",
    ),
    (
        "generate captions · translate captions to Spanish",
        "Captions from speech, or a copy the AI translates for review",
    ),
    (
        "preview shot.mp4 · close preview",
        "Play library media without adding it",
    ),
    ("add marker Intro · add track audio", "Markers and tracks"),
    ("mute track Audio 1 · lock track Video 1", "Track switches"),
    ("canvas 9:16 · rename project Film", "Project settings"),
    (
        "checkpoint Rough cut · restore Rough cut",
        "Keep a named checkpoint, or go back to one",
    ),
    (
        "duplicate project /path/Film copy.aster",
        "Carry on in a copy; this file stays as it is",
    ),
    (
        "export · render queue",
        "Open the export sheet, or the exports rendering and done",
    ),
];

fn percent(text: &str) -> Option<f32> {
    let text = text.trim().trim_end_matches('%').trim();
    text.parse::<f32>().ok().filter(|value| value.is_finite())
}

fn number(text: &str) -> Option<f32> {
    text.trim()
        .trim_end_matches(['x', '×', '°'])
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

/// A signed span such as `2s`, `-10f` or `+1:30`.
fn signed_time(text: &str, project: &Project) -> Option<Time> {
    let text = text.trim();
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    timefmt::parse(rest, project.settings.frame_rate).map(|time| Time(time.0 * sign))
}

fn one_selected<'a>(context: &Context<'a>) -> Result<&'a editor_core::Clip, String> {
    match context.selection.clips.as_slice() {
        [id] => context
            .project
            .clip(*id)
            .map(|(_, clip)| clip)
            .ok_or_else(|| "The selected clip no longer exists.".to_owned()),
        [] => Err("Select a clip first.".to_owned()),
        _ => Err("Select one clip; several are selected.".to_owned()),
    }
}

fn transform(
    context: &Context,
    change: impl FnOnce(&mut Transform) -> Result<(), String>,
) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    let mut next = clip.transform.clone();
    change(&mut next)?;
    if !next.validate() {
        return Err("That value is out of range.".to_owned());
    }
    Ok(vec![Action::Edit(Command::SetTransform {
        clip_id: clip.id,
        transform: next,
    })])
}

/// A name as it is compared: lower case, with the project's own
/// underscores, and an ampersand, read as the words they stand for.
fn plain(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .replace(['_', '-'], " ")
        .replace('&', "and")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The project's own word for a value: the one it is serialized as.
fn word<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(plain))
        .unwrap_or_default()
}

fn effect_named(text: &str) -> Result<EffectKind, String> {
    let wanted = plain(text);
    let mut offered = (EffectKind::ALL.into_iter()).filter(|kind| browser::offered(*kind));
    offered
        .find(|kind| plain(inspector::effect_title(*kind)) == wanted || word(kind) == wanted)
        .ok_or_else(|| {
            let names: Vec<String> = (EffectKind::ALL.into_iter())
                .filter(|kind| browser::offered(*kind))
                .map(|kind| inspector::effect_title(kind).to_lowercase())
                .collect();
            format!(
                "No effect is called “{}”. Effects: {}.",
                text.trim(),
                names.join(", ")
            )
        })
}

/// What the library would do with `entry` for the selected clips, as the
/// command line answers: an edit, or why there is none.
fn given(entry: Entry, context: &Context) -> Result<Vec<Action>, String> {
    let clips = &context.selection.clips;
    match browser::apply(context.project, entry, clips, context.playhead, None) {
        Action::Notify(reason) => Err(reason),
        action => Ok(vec![action]),
    }
}

/// `apply look cinematic 60%`: the look's effects, at full strength unless
/// a percentage follows the name.
fn apply_look(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let (name, strength) = match rest.rsplit_once(' ') {
        Some((name, last)) if last.ends_with('%') => (
            name.trim_end().trim_end_matches(" at"),
            percent(last).ok_or("Give a strength such as 60%.")? / 100.0,
        ),
        _ => (rest, 1.0),
    };
    if !(0.0..=1.0).contains(&strength) {
        return Err("A look's strength is 0% to 100%.".to_owned());
    }
    let wanted = plain(name);
    let (preset, _) = (inspector::looks().iter())
        .find(|(preset, name)| plain(name) == wanted || word(preset) == wanted)
        .ok_or_else(|| {
            let names: Vec<String> = (inspector::looks().iter())
                .map(|(_, name)| name.to_lowercase())
                .collect();
            format!(
                "No look is called “{}”. Looks: {}.",
                name.trim(),
                names.join(", ")
            )
        })?;
    let mut actions = given(Entry::Look(*preset), context)?;
    if let [Action::Edit(Command::ApplyPreset { strength: full, .. })] = actions.as_mut_slice() {
        *full = strength;
    }
    Ok(actions)
}

const BLENDS: [BlendMode; 9] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::Add,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
];

fn set_blend(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    let wanted = plain(rest.trim_start_matches("mode "));
    let mode = (BLENDS.into_iter())
        .find(|mode| word(mode) == wanted)
        .ok_or_else(|| {
            let names: Vec<String> = BLENDS.iter().map(word).collect();
            format!("A blend mode is one of: {}.", names.join(", "))
        })?;
    Ok(vec![Action::Edit(Command::SetBlendMode {
        clip_id: clip.id,
        mode,
    })])
}

/// The edge a line names at its end (`at start`, `at end`, `in`, `out`) and
/// the line without it.
fn named_edge(rest: &str) -> (&str, Option<TransitionEdge>) {
    for (suffix, edge) in [
        ("at start", TransitionEdge::In),
        ("at the start", TransitionEdge::In),
        ("at end", TransitionEdge::Out),
        ("at the end", TransitionEdge::Out),
        ("in", TransitionEdge::In),
        ("out", TransitionEdge::Out),
    ] {
        if rest == suffix {
            return ("", Some(edge));
        }
        if let Some(head) = rest.strip_suffix(suffix)
            && head.ends_with(' ')
        {
            return (head.trim_end(), Some(edge));
        }
    }
    (rest, None)
}

/// The edges of the selected clips a line is about: the named one, or each
/// clip's nearest the playhead.
fn edges(
    edge: Option<TransitionEdge>,
    context: &Context,
) -> Result<Vec<(editor_core::ClipId, TransitionEdge)>, String> {
    let clips = &context.selection.clips;
    if clips.is_empty() {
        return Err("Select a clip first.".to_owned());
    }
    Ok(match edge {
        Some(edge) => clips.iter().map(|clip| (*clip, edge)).collect(),
        None => transitions::at_playhead(context.project, clips, context.playhead),
    })
}

/// `add transition wipe left at start for 1s`: every part after the first
/// two words may be left out.
fn add_transition(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let project = context.project;
    let lower = rest.trim().to_lowercase();
    let (lower, length) = match lower.rsplit_once(" for ") {
        Some((head, span)) => (head.trim(), Some(span)),
        None => match lower.strip_prefix("for ") {
            Some(span) => ("", Some(span)),
            None => (lower.as_str(), None),
        },
    };
    let length = length
        .map(|span| {
            timefmt::parse(span, project.settings.frame_rate)
                .filter(|length| *length > Time::ZERO)
                .ok_or("Give a length such as 1s or 12f after “for”.")
        })
        .transpose()?;
    let (name, edge) = named_edge(lower);
    let kind = match name {
        "" => None,
        name => Some(transitions::named(name).ok_or_else(|| {
            let names: Vec<String> = (transitions::KINDS.iter())
                .map(|kind| transitions::name(*kind).to_lowercase())
                .collect();
            format!(
                "No transition is called “{name}”. Transitions: {}.",
                names.join(", ")
            )
        })?),
    };
    let places = edges(edge, context)?;
    let Some(length) = length else {
        // The application gives it what the person kept as the default.
        return Ok(vec![Action::AddTransitions { places, kind }]);
    };
    // A length that is asked for is the one it gets, cut to its clip.
    let kind = kind.unwrap_or(transitions::DEFAULT_KIND);
    let mut commands = Vec::new();
    for (clip, edge) in places {
        let (id, edge) = transitions::place(project, clip, edge)?;
        let (_, clip) = project.clip(id).ok_or("The clip no longer exists.")?;
        let duration = transitions::fitted(project, clip, length)
            .ok_or("That clip is too short for a transition.")?;
        let transition = Transition {
            duration,
            ..transitions::with_kind(
                transitions::of(clip, edge).unwrap_or(&transitions::default_of(&[], kind)),
                kind,
            )
        };
        commands.push(transitions::set_command(id, edge, Some(transition)));
    }
    Ok(vec![Action::Edit(batch("Add transitions", commands))])
}

fn remove_transition(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let project = context.project;
    let (extra, edge) = named_edge(rest.trim());
    if !extra.is_empty() {
        return Err("Say “remove transition”, with “at start” or “at end” if you mean one.".into());
    }
    // What is removed is a transition of the selected clip itself: with no
    // edge named, the one on its edge nearer the playhead, or its other one
    // when that edge has none. A cut's transition belongs to the clip it
    // brings in, so only an end named in words reaches the next clip's.
    let has = |id, edge| {
        (project.clip(id)).is_some_and(|(_, clip)| transitions::of(clip, edge).is_some())
    };
    let mut removed: Vec<(editor_core::ClipId, TransitionEdge)> = Vec::new();
    for (clip, nearest) in edges(edge, context)? {
        let own = match edge {
            Some(edge) => [Some(edge), None],
            None => [
                Some(nearest),
                transitions::EDGES
                    .into_iter()
                    .find(|other| *other != nearest),
            ],
        };
        let found = (own.into_iter().flatten())
            .find(|edge| has(clip, *edge))
            .map(|edge| (clip, edge))
            .or_else(|| {
                let cut = transitions::place(project, clip, edge?).ok()?;
                has(cut.0, cut.1).then_some(cut)
            });
        if let Some(found) = found
            && !removed.contains(&found)
        {
            removed.push(found);
        }
    }
    let commands: Vec<Command> = (removed.into_iter())
        .map(|(id, edge)| transitions::set_command(id, edge, None))
        .collect();
    if commands.is_empty() {
        return Err("There is no transition there.".to_owned());
    }
    Ok(vec![Action::Edit(batch("Remove transitions", commands))])
}

/// One command, or several as one undo step.
fn batch(label: &str, mut commands: Vec<Command>) -> Command {
    match commands.len() {
        1 => commands.remove(0),
        _ => Command::Batch {
            label: label.to_owned(),
            commands,
        },
    }
}

fn remove_effect(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    let kind = effect_named(rest)?;
    let index = (clip.effects.iter())
        .rposition(|effect| effect.kind == kind)
        .ok_or_else(|| format!("The selected clip has no {}.", plain(rest)))?;
    let mut effects = clip.effects.clone();
    effects.remove(index);
    Ok(vec![Action::Edit(Command::SetEffects {
        clip_id: clip.id,
        effects,
    })])
}

/// `chroma key blue`: a key of that screen colour where the Color section
/// would put one. A clip keeps one key.
fn chroma_key(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    let colour = match rest.trim().to_lowercase().as_str() {
        "" | "green" => [0.0, 1.0, 0.0],
        "blue" => [0.0, 0.0, 1.0],
        _ => return Err("A screen is green or blue; pick any other colour in Color.".to_owned()),
    };
    if (clip.effects.iter()).any(|effect| effect.kind == EffectKind::ChromaKey) {
        return Err("The selected clip already has a chroma key; adjust it in Color.".to_owned());
    }
    let mut effect = Effect::new(EffectKind::ChromaKey);
    effect.params = [ParamKey::KeyR, ParamKey::KeyG, ParamKey::KeyB]
        .into_iter()
        .zip(colour)
        .map(|(key, value)| ParamValue { key, value })
        .collect();
    let mut effects = clip.effects.clone();
    effects.insert(inspector::place_effect(&effects, effect.kind), effect);
    Ok(vec![Action::Edit(Command::SetEffects {
        clip_id: clip.id,
        effects,
    })])
}

/// One of the clip's own values, as stored.
fn stored(clip: &Clip, key: ParamKey) -> f32 {
    let t = &clip.transform;
    match key {
        ParamKey::PositionX => t.position[0],
        ParamKey::PositionY => t.position[1],
        ParamKey::ScaleX => t.scale[0],
        ParamKey::ScaleY => t.scale[1],
        ParamKey::Rotation => t.rotation,
        ParamKey::Opacity => t.opacity,
        _ => clip.gain,
    }
}

/// `add keyframe opacity 50%`: a keyframe at the playhead, held inside the
/// clip, with the value given or the one the clip shows there.
fn add_keyframe(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    let rest = rest.trim();
    let (name, value) = match rest.split_once(' ') {
        Some((name, value)) => (name, Some(value.trim())),
        None => (rest, None),
    };
    // What the value is called, the tracks it is, and what a typed value
    // is divided by.
    let (keys, scale): (&[ParamKey], f32) = match name.to_lowercase().as_str() {
        "opacity" => (&[ParamKey::Opacity], 100.0),
        "scale" => (&[ParamKey::ScaleX, ParamKey::ScaleY], 100.0),
        "rotation" => (&[ParamKey::Rotation], 1.0),
        "volume" => (&[ParamKey::Gain], 100.0),
        "position" => (&[ParamKey::PositionX, ParamKey::PositionY], 1.0),
        _ => {
            return Err(
                "A keyframe is of opacity, scale, rotation, position or volume.".to_owned(),
            );
        }
    };
    let typed = match value {
        Some(_) if name.eq_ignore_ascii_case("position") => {
            return Err("Place the clip first, then “add keyframe position”.".to_owned());
        }
        Some(text) => Some(
            number(text.trim_end_matches('%')).ok_or("Give a value such as 50% or 15.")? / scale,
        ),
        None => None,
    };
    let at = (context.playhead - clip.start).clamp(Time::ZERO, clip.duration);
    let commands = (keys.iter())
        .map(|key| {
            let track =
                (clip.animation.iter()).find(|track| track.target.is_none() && track.key == *key);
            let value = typed.unwrap_or_else(|| match track {
                Some(track) if !track.keys.is_empty() => track.sample(at),
                _ => stored(clip, *key),
            });
            let existing = track.map_or(&[][..], |track| track.keys.as_slice());
            Command::SetKeyframes {
                clip_id: clip.id,
                target: None,
                key: *key,
                keys: with_key(existing, at, value),
            }
        })
        .collect();
    Ok(vec![Action::Edit(batch("Add keyframe", commands))])
}

fn reverse(context: &Context) -> Result<Vec<Action>, String> {
    let project = context.project;
    let clip = one_selected(context)?;
    let (track, _) = project.clip(clip.id).ok_or("The clip no longer exists.")?;
    if !speed::timed(project, track, clip) {
        return Err("Only a video clip plays backwards.".to_owned());
    }
    let mut curve = speed::Curve::of(clip);
    curve.reverse();
    speed::curve_command(project, clip.id, &curve, "Reverse clip")
        .map(|command| vec![Action::Edit(command)])
        .ok_or_else(|| "That clip cannot be played backwards.".to_owned())
}

/// `freeze frame for 2s`: the selected clip under the playhead, or the top
/// one there, as the F key has it.
fn freeze(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let project = context.project;
    let hold = match rest.trim() {
        "" => speed::HOLD,
        rest => (rest.strip_prefix("for "))
            .and_then(|span| timefmt::parse(span, project.settings.frame_rate))
            .ok_or("Say how long, such as “freeze frame for 2s”.")?,
    };
    let clip = speed::freeze_target(project, context.selection.single(), context.playhead)
        .ok_or("Put the playhead on a video clip to freeze a frame.")?;
    speed::freeze_command(project, clip, context.playhead, hold)
        .map(|command| vec![Action::Edit(command)])
}

/// `follow Tracker 1`: the selected clip goes where the tracker's point
/// goes, from where it is at the playhead, as the inspector's list has it.
fn follow(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let project = context.project;
    let clip = one_selected(context)?;
    let (track, _) = project.clip(clip.id).ok_or("The clip no longer exists.")?;
    if track.kind == TrackKind::Audio {
        return Err("Only a picture or a title follows a tracker.".to_owned());
    }
    let tracker = tracker_named(project, rest)?;
    // The clip keeps its place at the playhead and moves with the point
    // from there.
    let offset = editor_core::render::tracked_place(
        project,
        context.playhead,
        clip.id,
        tracker,
        context.readings,
    )
    .map_or([0.0; 2], |point| point.map(|value| -value.round() + 0.0));
    Ok(vec![Action::Edit(Command::SetFollow {
        clip_id: clip.id,
        follow: Some(editor_core::Follow {
            tracker_id: tracker.id,
            position: true,
            scale: false,
            rotation: false,
            offset,
        }),
    })])
}

/// The one tracker called `name`, in any case of letters.
fn tracker_named<'a>(project: &'a Project, name: &str) -> Result<&'a editor_core::Tracker, String> {
    let wanted = name.trim().to_lowercase();
    let mut named =
        (project.trackers.values()).filter(|tracker| tracker.name.to_lowercase() == wanted);
    match (named.next(), named.next()) {
        (Some(tracker), None) => Ok(tracker),
        (Some(_), Some(_)) => Err(format!(
            "More than one tracker is called {}; rename one in the inspector's Motion section.",
            name.trim()
        )),
        (None, _) if project.trackers.is_empty() => Err(
            "There are no trackers yet. Track a region in the inspector's Motion section."
                .to_owned(),
        ),
        (None, _) => {
            let names: Vec<&str> = (project.trackers.values())
                .map(|tracker| tracker.name.as_str())
                .collect();
            Err(format!(
                "There is no tracker with that name. There are: {}.",
                names.join(", ")
            ))
        }
    }
}

/// `remove tracker Tracker 1`: the tracker goes, and whatever followed it
/// stays where it is put. It needs no clip selected, so a tracker of media
/// no clip shows any more can still be removed.
fn remove_tracker(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let tracker = tracker_named(context.project, rest)?;
    Ok(vec![Action::Edit(Command::RemoveTracker {
        tracker_id: tracker.id,
    })])
}

fn stop_following(context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    if clip.follow.is_none() {
        return Err("The selected clip follows no tracker.".to_owned());
    }
    Ok(vec![Action::Edit(Command::SetFollow {
        clip_id: clip.id,
        follow: None,
    })])
}

fn track_named<'a>(project: &'a Project, name: &str) -> Result<&'a editor_core::Track, String> {
    let wanted = name.trim().to_lowercase();
    project
        .tracks
        .iter()
        .find(|track| track.name.to_lowercase() == wanted)
        .map(AsRef::as_ref)
        .ok_or_else(|| {
            let names: Vec<&str> = project.tracks.iter().map(|t| t.name.as_str()).collect();
            format!(
                "No track is named “{}”. Tracks: {}.",
                name.trim(),
                names.join(", ")
            )
        })
}

/// Turns one line into the actions it asks for, or says what was expected.
pub fn interpret(line: &str, context: &Context) -> Result<Vec<Action>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Err("Type a command. “help” lists them.".to_owned());
    }
    if line.starts_with('{') {
        return serde_json::from_str::<Command>(line)
            .map(|command| vec![Action::Edit(command)])
            .map_err(|error| format!("That is not a command the editor knows: {error}"));
    }
    let project = context.project;
    let lower = line.to_lowercase();
    // The prefixes are ASCII, but a letter that only lowercases to ASCII can
    // be longer than one byte, so the original is sliced with a check.
    let after = |prefix: &str| {
        lower
            .strip_prefix(prefix)
            .and_then(|_| line.get(prefix.len()..))
            .map(str::trim)
    };

    Ok(match lower.as_str() {
        "split" | "split at playhead" | "cut here" => vec![Action::SplitAtPlayhead],
        "delete" | "remove" => vec![Action::Delete { ripple: false }],
        "ripple delete" => vec![Action::Delete { ripple: true }],
        "undo" => vec![Action::Undo],
        "redo" => vec![Action::Redo],
        "copy" => vec![Action::Copy],
        "cut" => vec![Action::Cut],
        "paste" => vec![Action::Paste],
        "duplicate" => vec![Action::Duplicate],
        "select all" => vec![Action::SelectAll],
        "close preview" => vec![Action::CloseSource],
        "play" | "pause" | "play/pause" => vec![Action::TogglePlay],
        "go to start" | "start" => vec![Action::JumpStart],
        "go to end" | "end" => vec![Action::JumpEnd],
        "fit" | "fit timeline" => vec![Action::FitTimeline],
        "save" => vec![Action::Save],
        "checkpoints" => vec![Action::OpenCheckpoints],
        "export" | "export video" => vec![Action::OpenExport],
        "render queue" | "renders" => vec![Action::OpenRenderQueue],
        "duplicate project" => vec![Action::DuplicateProject],
        "add marker" => vec![Action::AddMarker],
        "add text" => vec![Action::AddText],
        "group" => vec![Action::Edit(Command::GroupClips {
            clip_ids: context.selection.clips.clone(),
        })],
        "ungroup" => vec![Action::Edit(Command::UngroupClips {
            clip_ids: context.selection.clips.clone(),
        })],
        "link" => vec![Action::Edit(Command::LinkClips {
            clip_ids: context.selection.clips.clone(),
        })],
        "unlink" => vec![Action::Edit(Command::UnlinkClips {
            clip_ids: context.selection.clips.clone(),
        })],
        "reset transform" => transform(context, |t| {
            *t = Transform::default();
            Ok(())
        })?,
        "flip horizontal" | "flip h" => transform(context, |t| {
            t.flip_x = !t.flip_x;
            Ok(())
        })?,
        "flip vertical" | "flip v" => transform(context, |t| {
            t.flip_y = !t.flip_y;
            Ok(())
        })?,
        "reverse" | "reverse clip" => reverse(context)?,
        "freeze" | "freeze frame" => freeze("", context)?,
        "chroma key" | "green screen" | "remove background" => chroma_key("", context)?,
        "add transition" => add_transition("", context)?,
        "remove transition" => remove_transition("", context)?,
        "stop following" | "unfollow" => stop_following(context)?,
        "generate captions" | "captions from speech" => {
            vec![
                Action::ShowLibrary(Tab::Captions),
                Action::CaptionsFromSpeech,
            ]
        }
        "follow" => {
            return Err("Say which tracker, such as “follow Tracker 1”.".to_owned());
        }
        _ => {
            if let Some(rest) = after("add effect ") {
                given(Entry::Effect(effect_named(rest)?), context)?
            } else if let Some(rest) = after("remove effect ") {
                remove_effect(rest, context)?
            } else if let Some(rest) = after("apply look ").or_else(|| after("look ")) {
                apply_look(rest, context)?
            } else if let Some(rest) = after("set blend ").or_else(|| after("blend ")) {
                set_blend(rest, context)?
            } else if let Some(rest) = after("add transition ") {
                add_transition(rest, context)?
            } else if let Some(rest) = after("remove transition ") {
                remove_transition(&rest.to_lowercase(), context)?
            } else if let Some(rest) = after("add keyframe ").or_else(|| after("keyframe ")) {
                add_keyframe(rest, context)?
            } else if let Some(rest) = after("chroma key ") {
                chroma_key(rest, context)?
            } else if let Some(rest) = after("freeze frame ").or_else(|| after("freeze ")) {
                freeze(&rest.to_lowercase(), context)?
            } else if let Some(rest) = after("remove tracker ").or_else(|| after("delete tracker "))
            {
                remove_tracker(rest, context)?
            } else if let Some(rest) = after("follow ") {
                follow(rest, context)?
            } else if let Some(rest) = after("preview ") {
                let wanted = rest.to_lowercase();
                let asset = project
                    .assets
                    .values()
                    .find(|asset| asset.name.to_lowercase() == wanted)
                    .ok_or("There is no media with that name in the library.")?;
                vec![Action::PreviewAsset(asset.id)]
            } else if let Some(rest) = after("go to ").or_else(|| after("seek ")) {
                let time = timefmt::parse(rest, project.settings.frame_rate)
                    .ok_or("Give a time such as 12s, 1:30 or 48f.")?;
                vec![Action::Seek(time)]
            } else if let Some(rest) = after("split at ") {
                let time = timefmt::parse(rest, project.settings.frame_rate)
                    .ok_or("Give a time such as 12s, 1:30 or 48f.")?;
                vec![Action::Seek(time), Action::SplitAtPlayhead]
            } else if let Some(rest) = after("move ") {
                if context.selection.clips.is_empty() {
                    return Err("Select the clips to move first.".to_owned());
                }
                let delta =
                    signed_time(rest, project).ok_or("Give a span such as 2s, -10f or +1:30.")?;
                let delta = rules::resolve_move(project, &context.selection.clips, delta, None)
                    .ok_or("There is no room to move the selection there.")?;
                vec![Action::Edit(Command::MoveClips {
                    clip_ids: context.selection.clips.clone(),
                    delta,
                    target_track: None,
                })]
            } else if let Some(rest) = after("trim to ") {
                let clip = one_selected(context)?;
                let (track, _) = project.clip(clip.id).ok_or("The clip no longer exists.")?;
                let wanted = timefmt::parse(rest, project.settings.frame_rate)
                    .ok_or("Give a length such as 5s or 120f.")?;
                let (least, most) = rules::trim_range(project, track, clip, rules::Edge::End);
                let delta = (wanted - clip.duration).clamp(least, most);
                vec![Action::Edit(Command::TrimClip {
                    clip_id: clip.id,
                    source_in: clip.source_in,
                    duration: clip.duration + delta,
                    ripple: false,
                })]
            } else if let Some(rest) = after("set opacity ") {
                let value = percent(rest).ok_or("Give a percentage, such as 50%.")?;
                transform(context, |t| {
                    t.opacity = value / 100.0;
                    Ok(())
                })?
            } else if let Some(rest) = after("set scale ") {
                let value = percent(rest).ok_or("Give a percentage, such as 120%.")?;
                transform(context, |t| {
                    t.scale = [value / 100.0, value / 100.0];
                    Ok(())
                })?
            } else if let Some(rest) = after("set rotation ") {
                let value = number(rest).ok_or("Give degrees, such as 15.")?;
                transform(context, |t| {
                    t.rotation = value;
                    Ok(())
                })?
            } else if let Some(rest) = after("set position ") {
                let mut parts = rest.split([' ', ',']).filter(|part| !part.is_empty());
                let (x, y) = parts
                    .next()
                    .and_then(number)
                    .zip(parts.next().and_then(number))
                    .ok_or("Give two numbers in canvas pixels, such as 100 -50.")?;
                transform(context, |t| {
                    t.position = [x, y];
                    Ok(())
                })?
            } else if let Some(rest) = after("set speed ") {
                let clip = one_selected(context)?;
                let rate = number(rest).ok_or("Give a speed, such as 2x.")?;
                vec![Action::Edit(Command::SetPlaybackRate {
                    clip_id: clip.id,
                    rate: f64::from(rate),
                })]
            } else if let Some(rest) = after("set volume ") {
                let clip = one_selected(context)?;
                let value = percent(rest).ok_or("Give a percentage, such as 80%.")?;
                vec![Action::Edit(Command::SetGain {
                    clip_id: clip.id,
                    gain: value / 100.0,
                })]
            } else if let Some(rest) = after("set text ") {
                let clip = one_selected(context)?;
                let ClipContent::Text {
                    font_size, color, ..
                } = &clip.content
                else {
                    return Err("The selected clip is not text.".to_owned());
                };
                vec![Action::Edit(Command::SetText {
                    clip_id: clip.id,
                    text: rest.trim_matches('"').to_owned(),
                    font_size: *font_size,
                    color: *color,
                })]
            } else if let Some(rest) = after("add marker ") {
                let Command::AddMarker { mut marker } = marker_at(context.playhead) else {
                    unreachable!("marker_at builds a marker command");
                };
                marker.label = rest.to_owned();
                vec![Action::Edit(Command::AddMarker { marker })]
            } else if let Some(rest) = after("add track ") {
                let kind = match rest.to_lowercase().as_str() {
                    "video" => TrackKind::Video,
                    "audio" => TrackKind::Audio,
                    "text" => TrackKind::Text,
                    _ => return Err("A track is video, audio or text.".to_owned()),
                };
                vec![Action::AddTrack(kind)]
            } else if let Some(rest) = after("add text ") {
                add_text(rest, context)?
            } else if let Some(rest) = after("animate in ") {
                animate(TransitionEdge::In, rest, context)?
            } else if let Some(rest) = after("animate out ") {
                animate(TransitionEdge::Out, rest, context)?
            } else if let Some(rest) =
                after("translate captions to ").or_else(|| after("translate captions into "))
            {
                translate(rest, context)?
            } else if let Some(rest) = after("checkpoint ").filter(|rest| !rest.is_empty()) {
                vec![Action::SaveCheckpoint(rest.to_owned())]
            } else if let Some(rest) = after("restore ").filter(|rest| !rest.is_empty()) {
                vec![Action::RestoreCheckpoint(rest.to_owned())]
            } else if let Some(rest) = after("duplicate project ").filter(|rest| !rest.is_empty()) {
                let path = std::path::PathBuf::from(rest);
                if !path.is_absolute() {
                    return Err(
                        "Give the duplicate a full path, such as /films/Copy.aster.".to_owned()
                    );
                }
                vec![Action::DuplicateTo(path)]
            } else if let Some(rest) = after("rename project ") {
                vec![Action::Edit(Command::RenameProject {
                    name: rest.to_owned(),
                })]
            } else if let Some(rest) = after("canvas ") {
                let settings = match rest {
                    "16:9" | "9:16" | "1:1" => CanvasSettings {
                        frame_rate: project.settings.frame_rate,
                        background: project.settings.background,
                        audio_sample_rate: project.settings.audio_sample_rate,
                        ..CanvasSettings::preset(rest)
                    },
                    _ => return Err("A canvas is 16:9, 9:16 or 1:1.".to_owned()),
                };
                vec![Action::Edit(Command::SetSettings { settings })]
            } else if let Some(command) = track_switch(&lower, line, project)? {
                vec![Action::Edit(command)]
            } else {
                return Err(format!(
                    "“{line}” is not a command. “help” lists what this panel understands."
                ));
            }
        }
    })
}

/// `add text Hello for 3s`: the title goes on the first unlocked text track
/// at the playhead, or where there is room after it.
fn add_text(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let project = context.project;
    let (text, duration) = match rest.rsplit_once(" for ") {
        Some((text, span)) => (
            text,
            timefmt::parse(span, project.settings.frame_rate)
                .ok_or("Give a length such as 3s after “for”.")?,
        ),
        None => (rest, Time::from_seconds(3.0)),
    };
    if duration <= Time::ZERO {
        return Err("A title needs some length.".to_owned());
    }
    let text = text.trim().trim_matches('"');
    let track = project
        .tracks
        .iter()
        .find(|track| track.kind == TrackKind::Text && !track.locked)
        .ok_or("Add an unlocked text track first: “add track text”.")?;
    let start = rules::free_start(track, context.playhead, duration, &[]);
    Ok(vec![Action::Edit(Command::AddClip {
        track_id: track.id,
        clip: editor_core::Clip::text(text, start, duration),
    })])
}

/// `animate in pop`, `animate out fade for 1s`, `animate in none`: the
/// entrance or exit of every selected title, as one step. A new one takes
/// half a second unless a length is given, held inside its clip.
fn animate(edge: TransitionEdge, rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let project = context.project;
    let (name, length) = match rest.rsplit_once(" for ") {
        Some((name, span)) => (
            name,
            Some(
                timefmt::parse(span, project.settings.frame_rate)
                    .ok_or("Give a length such as 0.5s after “for”.")?,
            ),
        ),
        None => (rest, None),
    };
    let name = name.trim().to_lowercase();
    let kind = match name.as_str() {
        "none" | "nothing" | "off" => None,
        _ => Some(
            serde_json::from_value::<TextAnimationKind>(serde_json::Value::String(
                name.replace([' ', '-'], "_"),
            ))
            .map_err(|_| {
                "An animation is fade, pop, bounce, typewriter, wipe or slide up, down, left or right."
                    .to_owned()
            })?,
        ),
    };
    let frame = project.settings.frame_rate.frame_time(1);
    let mut commands = Vec::new();
    for (_, clip) in (context.selection.clips.iter()).filter_map(|id| project.clip(*id)) {
        let ClipContent::Text { enter, exit, .. } = &clip.content else {
            continue;
        };
        let (other, current) = match edge {
            TransitionEdge::In => (exit, enter),
            TransitionEdge::Out => (enter, exit),
        };
        let animation = match kind {
            None if current.is_none() => continue,
            None => None,
            Some(kind) => {
                let given = TextAnimation {
                    kind,
                    unit: kind.usual_unit(),
                    duration: length.unwrap_or(Time::from_seconds(0.5)),
                    delay: Time::ZERO,
                };
                // Room the other edge leaves, and no more.
                let room = clip.duration - other.map_or(Time::ZERO, |other| other.span());
                Some(given.held(frame, room).ok_or(format!(
                    "“{}” has no room for that beside its other animation.",
                    clip.name
                ))?)
            }
        };
        commands.push(Command::SetTextAnimation {
            clip_id: clip.id,
            edge,
            animation,
        });
    }
    if commands.is_empty() {
        return Err(match kind {
            Some(_) => "Select the titles to animate first.",
            None => "The selected titles have no such animation.",
        }
        .to_owned());
    }
    let label = match (edge, kind) {
        (TransitionEdge::In, Some(_)) => "Animate in",
        (TransitionEdge::Out, Some(_)) => "Animate out",
        (TransitionEdge::In, None) => "Remove animation in",
        (TransitionEdge::Out, None) => "Remove animation out",
    };
    Ok(vec![Action::Edit(batch(label, commands))])
}

/// `translate captions to Spanish`: the caption track of the selected
/// caption, or else the one caption track that is shown.
fn translate(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let language = languages::tag(rest.trim().trim_end_matches('.'))
        .ok_or("Name a language or give its tag, such as Spanish or pt-BR.")?;
    let project = context.project;
    let captions = |track: &Track| track.kind == TrackKind::Text && track.language.is_some();
    let selected = (context.selection.clips.iter())
        .filter_map(|id| project.clip(*id))
        .map(|(track, _)| track)
        .find(|track| captions(track));
    let track = match selected {
        Some(track) => track,
        None => {
            let mut shown = (project.tracks.iter())
                .map(AsRef::as_ref)
                .filter(|track| captions(track) && !track.hidden);
            match (shown.next(), shown.next()) {
                (Some(track), None) => track,
                (None, _) => return Err("There is no caption track to translate.".to_owned()),
                _ => return Err("Select a caption of the track to translate.".to_owned()),
            }
        }
    };
    captions_tab::translation(project, track, &language)?;
    Ok(vec![Action::TranslateCaptions {
        track_id: track.id,
        language,
    }])
}

/// `mute track Audio 1`, `unlock track Video 1`, `hide track Text 1`…
fn track_switch(lower: &str, line: &str, project: &Project) -> Result<Option<Command>, String> {
    for (verb, locked, hidden, muted) in [
        ("lock", Some(true), None, None),
        ("unlock", Some(false), None, None),
        ("hide", None, Some(true), None),
        ("show", None, Some(false), None),
        ("mute", None, None, Some(true)),
        ("unmute", None, None, Some(false)),
    ] {
        let prefix = format!("{verb} track ");
        if lower.starts_with(&prefix) {
            let track = track_named(project, line.get(prefix.len()..).unwrap_or_default())?;
            return Ok(Some(Command::SetTrackState {
                track_id: track.id,
                locked,
                hidden,
                muted,
                name: None,
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(line: &str, project: &Project, selection: &Selection) -> Result<Vec<Action>, String> {
        interpret(
            line,
            &Context {
                project,
                selection,
                playhead: Time::from_seconds(1.0),
                readings: &[],
            },
        )
    }

    fn project_with_title() -> (Project, Selection) {
        let mut editor = editor_core::Editor::default();
        let track = editor
            .project()
            .tracks
            .iter()
            .find(|track| track.kind == TrackKind::Text)
            .unwrap()
            .id;
        let clip = editor_core::Clip::text("Hi", Time::ZERO, Time::from_seconds(4.0));
        let id = clip.id;
        editor
            .dispatch(Command::AddClip {
                track_id: track,
                clip,
            })
            .unwrap();
        // The title is an English caption, so it can be translated.
        editor
            .dispatch(Command::SetTrackLanguage {
                track_id: track,
                language: Some("en".into()),
            })
            .unwrap();
        let mut shot = editor_core::Asset::new(
            "/media/shot.mp4".into(),
            editor_core::MediaKind::Video,
            Time::from_seconds(6.0),
        );
        shot.name = "shot.mp4".to_owned();
        editor
            .dispatch(Command::AddAssets { assets: vec![shot] })
            .unwrap();
        let mut selection = Selection::default();
        selection.only(id);
        (editor.project().clone(), selection)
    }

    #[test]
    fn a_preview_names_library_media_and_never_edits() {
        let (project, selection) = project_with_title();
        let asset = *project.assets.keys().next().unwrap();
        let asked = run("Preview SHOT.mp4", &project, &selection).unwrap();
        assert!(matches!(asked.as_slice(), [Action::PreviewAsset(id)] if *id == asset));
        assert!(run("preview nothing.mov", &project, &selection).is_err());
        assert!(matches!(
            run("close preview", &project, &selection)
                .unwrap()
                .as_slice(),
            [Action::CloseSource]
        ));
    }

    /// What is done to a picture: these examples are tried on a video clip.
    const OF_A_PICTURE: [&str; 9] = [
        "add effect",
        "remove effect",
        "apply look",
        "set blend",
        "add transition",
        "remove transition",
        "reverse",
        "freeze frame",
        "chroma key",
    ];

    #[test]
    fn every_listed_example_is_understood() {
        let (project, selection) = project_with_title();
        let (shot, shot_selected, _) = project_with_shot();
        for (examples, _) in EXAMPLES {
            for example in examples.split(" · ") {
                // Speed and volume need media; the rest apply to a title.
                if example.starts_with("set speed") || example.starts_with("set volume") {
                    continue;
                }
                // Following needs a tracker: it has a test of its own.
                if example.starts_with("follow ") || example == "stop following" {
                    continue;
                }
                let (project, selection) =
                    match OF_A_PICTURE.iter().any(|start| example.starts_with(start)) {
                        true => (&shot, &shot_selected),
                        false => (&project, &selection),
                    };
                assert!(
                    run(example, project, selection).is_ok(),
                    "{example}: {:?}",
                    run(example, project, selection).err()
                );
            }
        }
    }

    /// A four second video clip from the start of the first video track,
    /// selected, with a glow and a dissolve at its start.
    fn project_with_shot() -> (Project, Selection, editor_core::ClipId) {
        let mut editor = editor_core::Editor::default();
        let mut shot = editor_core::Asset::new(
            "/media/shot.mp4".into(),
            editor_core::MediaKind::Video,
            Time::from_seconds(6.0),
        );
        shot.name = "shot.mp4".to_owned();
        let track = (editor.project().tracks.iter())
            .find(|track| track.kind == TrackKind::Video)
            .unwrap()
            .id;
        let mut clip = Clip::from_asset(&shot, Time::ZERO);
        clip.duration = Time::from_seconds(4.0);
        clip.effects.push(Effect::new(EffectKind::Glow));
        clip.transition_in = Some(transitions::default_of(&[], transitions::DEFAULT_KIND));
        let id = clip.id;
        for command in [
            Command::AddAssets { assets: vec![shot] },
            Command::AddClip {
                track_id: track,
                clip,
            },
        ] {
            editor.dispatch(command).unwrap();
        }
        let mut selection = Selection::default();
        selection.only(id);
        (editor.project().clone(), selection, id)
    }

    /// What a line asks for, applied: the project after it, or why not.
    /// A transition of no named length is made as the application makes
    /// it, with nothing kept as a default.
    fn applied(line: &str, project: &Project, selection: &Selection) -> Result<Project, String> {
        let mut editor = editor_core::Editor::new(project.clone()).unwrap();
        for action in run(line, project, selection)? {
            let command = match action {
                Action::Edit(command) => command,
                Action::AddTransitions { places, kind } => {
                    transitions::add_command(editor.project(), &[], &places, kind)?
                }
                other => panic!("{line} asked for {other:?}"),
            };
            let steps = editor.history().count();
            editor
                .dispatch(command)
                .map_err(|error| error.to_string())?;
            assert_eq!(editor.history().count(), steps + 1, "{line} is one step");
        }
        Ok(editor.project().clone())
    }

    #[test]
    fn effects_are_added_and_removed_by_name() {
        let (project, selection, id) = project_with_shot();
        let kinds = |project: &Project| -> Vec<EffectKind> {
            let clip = project.clip(id).unwrap().1;
            clip.effects.iter().map(|effect| effect.kind).collect()
        };
        let after = applied("add effect Motion Blur", &project, &selection).unwrap();
        assert_eq!(kinds(&after), [EffectKind::Glow, EffectKind::MotionBlur]);
        // The project's own word for a kind is understood too.
        let after = applied("add effect motion_blur", &project, &selection).unwrap();
        assert_eq!(kinds(&after).len(), 2);
        let after = applied("remove effect glow", &project, &selection).unwrap();
        assert!(kinds(&after).is_empty());
        let none = applied("remove effect blur", &project, &selection).unwrap_err();
        assert_eq!(none, "The selected clip has no blur.");
        let unknown = run("add effect sparkle", &project, &selection).unwrap_err();
        assert!(
            unknown.contains("glow") && unknown.contains("chroma key"),
            "{unknown}"
        );
        // A table, a plate and a stabilization are not added by name.
        assert!(run("add effect lut", &project, &selection).is_err());
        let nothing = run("add effect glow", &project, &Selection::default()).unwrap_err();
        assert!(nothing.starts_with("Select a clip"), "{nothing}");
        // A title takes no effect, and the line says so.
        let (titled, title) = project_with_title();
        let refused = run("add effect glow", &titled, &title).unwrap_err();
        assert!(refused.contains("picture clip"), "{refused}");
    }

    #[test]
    fn a_look_is_applied_at_the_strength_given() {
        let (project, selection, id) = project_with_shot();
        let whole = run("apply look teal & orange", &project, &selection).unwrap();
        assert!(matches!(
            whole.as_slice(),
            [Action::Edit(Command::ApplyPreset { clip_ids, preset, strength })]
                if clip_ids == &[id]
                    && *preset == editor_core::LookPreset::TealOrange
                    && *strength == 1.0
        ));
        let part = run("apply look Cinematic at 60%", &project, &selection).unwrap();
        assert!(matches!(
            part.as_slice(),
            [Action::Edit(Command::ApplyPreset { strength, .. })] if (*strength - 0.6).abs() < 1e-6
        ));
        let after = applied("look noir 50%", &project, &selection).unwrap();
        assert!(after.clip(id).unwrap().1.effects.len() > 1);
        assert!(run("apply look noir 150%", &project, &selection).is_err());
        let unknown = run("apply look sepia", &project, &selection).unwrap_err();
        assert!(unknown.contains("vintage"), "{unknown}");
    }

    #[test]
    fn a_blend_mode_is_set_by_name() {
        let (project, selection, id) = project_with_shot();
        let after = applied("set blend soft light", &project, &selection).unwrap();
        assert_eq!(after.clip(id).unwrap().1.blend, BlendMode::SoftLight);
        let after = applied("set blend mode screen", &project, &selection).unwrap();
        assert_eq!(after.clip(id).unwrap().1.blend, BlendMode::Screen);
        let unknown = run("set blend glow", &project, &selection).unwrap_err();
        assert!(unknown.contains("soft light"), "{unknown}");
    }

    #[test]
    fn a_transition_goes_on_the_edge_named_or_the_one_nearest_the_playhead() {
        let (project, selection, id) = project_with_shot();
        let clip = |project: &Project| project.clip(id).unwrap().1.clone();
        // The playhead is a second into four: the start is nearer, and the
        // dissolve there becomes a wipe of the same length.
        let after = applied("add transition wipe left", &project, &selection).unwrap();
        let made = clip(&after).transition_in.unwrap();
        assert_eq!(made.kind, editor_core::TransitionKind::WipeLeft);
        assert_eq!(made.duration, transitions::DEFAULT_DURATION);
        assert!(clip(&after).transition_out.is_none());
        // A named edge and a named length.
        let after = applied("add transition zoom in at end for 1s", &project, &selection).unwrap();
        let made = clip(&after).transition_out.unwrap();
        assert_eq!(made.kind, editor_core::TransitionKind::ZoomIn);
        assert_eq!(made.duration, Time::from_seconds(1.0));
        // Longer than the clip is cut to the clip.
        let after = applied("add transition out for 9s", &project, &selection).unwrap();
        assert_eq!(
            clip(&after).transition_out.unwrap().duration,
            Time::from_seconds(4.0)
        );
        // With no kind, an edge that has one is left as it is, in words.
        let there = applied("add transition", &project, &selection).unwrap_err();
        assert!(there.contains("has a transition"), "{there}");
        let after = applied("remove transition", &project, &selection).unwrap();
        assert!(clip(&after).transition_in.is_none());
        let none = run("remove transition at end", &project, &selection).unwrap_err();
        assert_eq!(none, "There is no transition there.");
        let unknown = run("add transition star wipe", &project, &selection).unwrap_err();
        assert!(unknown.contains("dissolve"), "{unknown}");
        assert!(run("add transition", &project, &Selection::default()).is_err());
    }

    #[test]
    fn remove_transition_takes_the_selected_clips_own_and_not_the_next_clips() {
        let (project, selection, id) = project_with_shot();
        // A second shot from the cut, with a transition of its own.
        let mut editor = editor_core::Editor::new(project).unwrap();
        let (track, shot) = editor.project().clip(id).unwrap();
        let (track, mut next) = (track.id, shot.clone());
        next.id = uuid::Uuid::new_v4();
        next.start = shot.end();
        next.effects.clear();
        let following = next.id;
        editor
            .dispatch(Command::AddClip {
                track_id: track,
                clip: next,
            })
            .unwrap();
        let project = editor.project().clone();
        // The playhead is just past the selected clip's end: its end is the
        // nearer edge and has nothing; its start has the transition.
        let past = Context {
            project: &project,
            selection: &selection,
            playhead: Time::from_seconds(4.1),
            readings: &[],
        };
        let asked = interpret("remove transition", &past).unwrap();
        let [Action::Edit(command)] = asked.as_slice() else {
            panic!("{asked:?}");
        };
        editor.dispatch(command.clone()).unwrap();
        let has = |editor: &editor_core::Editor, clip| {
            editor
                .project()
                .clip(clip)
                .unwrap()
                .1
                .transition_in
                .is_some()
        };
        assert!(!has(&editor, id), "the selected clip's own went");
        assert!(has(&editor, following), "the next clip's stayed");
        // Nothing of its own is left, and the cut is not its own.
        let project = editor.project().clone();
        let past = Context {
            project: &project,
            ..past
        };
        let none = interpret("remove transition", &past).unwrap_err();
        assert_eq!(none, "There is no transition there.");
        // The end named in words is the cut, whose transition the next clip has.
        let asked = interpret("remove transition at end", &past).unwrap();
        let [Action::Edit(command)] = asked.as_slice() else {
            panic!("{asked:?}");
        };
        editor.dispatch(command.clone()).unwrap();
        assert!(!has(&editor, following));
    }

    #[test]
    fn a_clip_follows_a_tracker_by_name_and_stops() {
        let (project, selection) = project_with_title();
        let title = selection.single().unwrap();
        let none = run("follow Tracker 1", &project, &selection).unwrap_err();
        assert!(none.starts_with("There are no trackers yet"), "{none}");
        // A tracker of the shot, and the shot under the title.
        let mut editor = editor_core::Editor::new(project).unwrap();
        let shot = editor.project().assets.values().next().unwrap().clone();
        let video = (editor.project().tracks.iter())
            .find(|track| track.kind == TrackKind::Video)
            .unwrap()
            .id;
        let mut clip = Clip::from_asset(&shot, Time::ZERO);
        clip.duration = Time::from_seconds(4.0);
        let key = |at: f64, x: f32| editor_core::TrackKey {
            source: Time::from_seconds(at),
            center: [x, 0.5],
            scale: 1.0,
            rotation: 0.0,
        };
        let tracker = editor_core::Tracker {
            id: uuid::Uuid::new_v4(),
            name: "Tracker 1".to_owned(),
            asset_id: shot.id,
            keys: vec![key(0.0, 0.25), key(4.0, 0.75)],
        };
        for command in [
            Command::AddClip {
                track_id: video,
                clip,
            },
            Command::SetTracker {
                tracker: tracker.clone(),
            },
        ] {
            editor.dispatch(command).unwrap();
        }
        let project = editor.project().clone();

        // Both listed examples, the first in any case of letters.
        let after = applied("Follow tracker 1", &project, &selection).unwrap();
        let follow = after.clip(title).unwrap().1.follow.clone().unwrap();
        assert_eq!(follow.tracker_id, tracker.id);
        assert!(follow.position && !follow.scale && !follow.rotation);
        // A second in the point is at 0.375 of the width: the title stays
        // where it is and goes with the point from there.
        let width = project.settings.width as f32;
        assert_eq!(follow.offset, [(0.125 * width).round(), 0.0]);
        let place = |project: &Project| {
            let plan = editor_core::resolve(project, Time::from_seconds(1.0), &[]);
            let layer = plan
                .layers
                .iter()
                .find(|layer| layer.clip == title)
                .unwrap();
            layer.transform.position
        };
        assert_eq!(place(&after), place(&project));
        let stopped = applied("stop following", &after, &selection).unwrap();
        assert!(stopped.clip(title).unwrap().1.follow.is_none());
        let stopped = applied("unfollow", &after, &selection).unwrap();
        assert!(stopped.clip(title).unwrap().1.follow.is_none());

        let none = run("stop following", &project, &selection).unwrap_err();
        assert_eq!(none, "The selected clip follows no tracker.");
        // A tracker is removed by name with nothing selected, so one of
        // media no clip shows any more can still go.
        let nothing = Selection::default();
        let removed = applied("Remove tracker tracker 1", &after, &nothing).unwrap();
        assert!(removed.trackers.is_empty());
        assert!(removed.clip(title).unwrap().1.follow.is_none());
        let unknown = run("remove tracker the ball", &project, &nothing).unwrap_err();
        assert!(unknown.contains("There are: Tracker 1"), "{unknown}");
        let unknown = run("follow the ball", &project, &selection).unwrap_err();
        assert!(unknown.contains("Tracker 1"), "{unknown}");
        assert!(run("follow", &project, &selection).is_err());
        let nothing = run("follow Tracker 1", &project, &Selection::default()).unwrap_err();
        assert!(nothing.starts_with("Select a clip"), "{nothing}");
    }

    #[test]
    fn a_clip_is_reversed_and_a_frame_frozen_as_one_step_each() {
        let (project, selection, id) = project_with_shot();
        let after = applied("reverse", &project, &selection).unwrap();
        let reversed = after.clip(id).unwrap().1;
        assert!(speed::Curve::of(reversed).reversed());
        let after = applied("freeze frame for 1s", &project, &selection).unwrap();
        let frozen = after.clip(id).unwrap().1;
        assert_eq!(frozen.duration, Time::from_seconds(5.0));
        assert_eq!(speed::badge(frozen).as_deref(), Some("Freeze"));
        let after = applied("freeze", &project, &selection).unwrap();
        assert_eq!(after.clip(id).unwrap().1.duration, Time::from_seconds(6.0));
        assert!(run("freeze frame soon", &project, &selection).is_err());
        // A title has no frame to hold and nothing to play backwards.
        let (titled, title) = project_with_title();
        assert!(run("reverse", &titled, &title).is_err());
        assert!(run("freeze frame", &titled, &title).is_err());
    }

    #[test]
    fn a_keyframe_is_written_at_the_playhead_inside_the_clip() {
        let (project, selection, id) = project_with_shot();
        let track = |project: &Project, key: ParamKey| -> Vec<(Time, f32)> {
            let clip = project.clip(id).unwrap().1;
            (clip.animation.iter())
                .find(|track| track.target.is_none() && track.key == key)
                .map_or_else(Vec::new, |track| {
                    track.keys.iter().map(|key| (key.at, key.value)).collect()
                })
        };
        let second = Time::from_seconds(1.0);
        let after = applied("add keyframe opacity 50%", &project, &selection).unwrap();
        assert_eq!(track(&after, ParamKey::Opacity), [(second, 0.5)]);
        // With no value it holds what the clip shows there.
        let after = applied("add keyframe rotation", &project, &selection).unwrap();
        assert_eq!(track(&after, ParamKey::Rotation), [(second, 0.0)]);
        // Both axes of a scale or a place are one step.
        let after = applied("add keyframe scale 120", &project, &selection).unwrap();
        assert_eq!(track(&after, ParamKey::ScaleX), [(second, 1.2)]);
        assert_eq!(track(&after, ParamKey::ScaleY), [(second, 1.2)]);
        let after = applied("keyframe position", &project, &selection).unwrap();
        assert_eq!(track(&after, ParamKey::PositionY), [(second, 0.0)]);
        // A value the project has no room for is refused by the project.
        assert!(applied("add keyframe opacity 500%", &project, &selection).is_err());
        assert!(run("add keyframe hue", &project, &selection).is_err());
    }

    #[test]
    fn a_screen_is_keyed_once_before_the_other_effects() {
        let (project, selection, id) = project_with_shot();
        let after = applied("chroma key blue", &project, &selection).unwrap();
        let clip = after.clip(id).unwrap().1;
        // The key is pulled from the picture before the glow is drawn.
        assert_eq!(clip.effects[0].kind, EffectKind::ChromaKey);
        assert_eq!(clip.effects[0].param(ParamKey::KeyB), Some(1.0));
        assert_eq!(clip.effects[0].param(ParamKey::KeyG), Some(0.0));
        assert_eq!(clip.effects[1].kind, EffectKind::Glow);
        let again = run("chroma key", &after, &selection).unwrap_err();
        assert!(again.contains("already has"), "{again}");
        assert!(run("chroma key red", &project, &selection).is_err());
    }

    #[test]
    fn opacity_becomes_a_transform_command_for_the_selected_clip() {
        let (project, selection) = project_with_title();
        let actions = run("set opacity 50%", &project, &selection).unwrap();
        assert!(matches!(
            actions.as_slice(),
            [Action::Edit(Command::SetTransform { transform, .. })] if transform.opacity == 0.5
        ));
    }

    #[test]
    fn a_change_without_a_selection_says_what_is_missing() {
        let project = Project::default();
        let error = run("set opacity 50%", &project, &Selection::default()).unwrap_err();
        assert_eq!(error, "Select a clip first.");
    }

    #[test]
    fn raw_json_is_read_as_a_core_command() {
        let project = Project::default();
        let actions = run(
            r#"{"type":"rename_project","name":"Film"}"#,
            &project,
            &Selection::default(),
        )
        .unwrap();
        assert!(matches!(
            actions.as_slice(),
            [Action::Edit(Command::RenameProject { name })] if name == "Film"
        ));
    }

    #[test]
    fn checkpoints_and_duplicates_are_named_on_the_command_line() {
        let project = Project::default();
        let none = Selection::default();
        assert!(matches!(
            run("checkpoint Rough cut", &project, &none).unwrap().as_slice(),
            [Action::SaveCheckpoint(name)] if name == "Rough cut"
        ));
        assert!(matches!(
            run("Restore Rough cut", &project, &none).unwrap().as_slice(),
            [Action::RestoreCheckpoint(name)] if name == "Rough cut"
        ));
        assert!(matches!(
            run("duplicate project /films/B.aster", &project, &none).unwrap().as_slice(),
            [Action::DuplicateTo(path)] if path == std::path::Path::new("/films/B.aster")
        ));
        assert!(run("duplicate project B.aster", &project, &none).is_err());
        assert!(matches!(
            run("duplicate", &project, &none).unwrap().as_slice(),
            [Action::Duplicate]
        ));
    }

    #[test]
    fn export_and_the_render_queue_are_named_on_the_command_line() {
        let project = Project::default();
        let none = Selection::default();
        assert!(matches!(
            run("Export", &project, &none).unwrap().as_slice(),
            [Action::OpenExport]
        ));
        assert!(matches!(
            run("render queue", &project, &none).unwrap().as_slice(),
            [Action::OpenRenderQueue]
        ));
    }

    #[test]
    fn an_unknown_sentence_is_refused_rather_than_guessed() {
        let project = Project::default();
        assert!(run("make it pop", &project, &Selection::default()).is_err());
    }

    fn animation(project: &Project, clip: editor_core::ClipId) -> [Option<TextAnimation>; 2] {
        match &project.clip(clip).unwrap().1.content {
            ClipContent::Text { enter, exit, .. } => [*enter, *exit],
            _ => unreachable!("FIXTURE: a title"),
        }
    }

    #[test]
    fn titles_are_animated_in_and_out_by_name_as_one_step_each() {
        let (project, mut selection) = project_with_title();
        let first = selection.clips[0];
        let project = applied("animate in typewriter", &project, &selection).unwrap();
        let [enter, exit] = animation(&project, first);
        let enter = enter.unwrap();
        assert_eq!(enter.kind, TextAnimationKind::Typewriter);
        assert_eq!(enter.unit, editor_core::TextUnit::Letter);
        assert_eq!(enter.duration, Time::from_seconds(0.5));
        assert!(exit.is_none());

        let project = applied("Animate out slide up for 1s", &project, &selection).unwrap();
        let [_, exit] = animation(&project, first);
        assert_eq!(exit.unwrap().kind, TextAnimationKind::SlideUp);
        assert_eq!(exit.unwrap().duration, Time::from_seconds(1.0));

        // A length longer than the room the other edge leaves is held to it.
        let project = applied("animate in pop for 9s", &project, &selection).unwrap();
        let [enter, exit] = animation(&project, first);
        assert_eq!(
            enter.unwrap().span() + exit.unwrap().span(),
            Time::from_seconds(4.0)
        );

        let project = applied("animate in none", &project, &selection).unwrap();
        assert!(animation(&project, first)[0].is_none());
        assert!(
            run("animate in none", &project, &selection).is_err(),
            "nothing to take off"
        );
        assert!(run("animate in wobble", &project, &selection).is_err());

        // Two titles are one step.
        let track = project.clip(first).unwrap().0.id;
        let second =
            editor_core::Clip::text("There", Time::from_seconds(5.0), Time::from_seconds(2.0));
        let id = second.id;
        let mut editor = editor_core::Editor::new(project).unwrap();
        editor
            .dispatch(Command::AddClip {
                track_id: track,
                clip: second,
            })
            .unwrap();
        selection.clips.push(id);
        let asked = run("animate in bounce", editor.project(), &selection).unwrap();
        let [Action::Edit(command)] = asked.as_slice() else {
            panic!("{asked:?}");
        };
        let steps = editor.history().count();
        editor.dispatch(command.clone()).unwrap();
        assert_eq!(editor.history().count(), steps + 1);
        for clip in [first, id] {
            assert_eq!(
                animation(editor.project(), clip)[0].unwrap().kind,
                TextAnimationKind::Bounce
            );
        }

        // Media is not a title.
        let (shot, shot_selected, _) = project_with_shot();
        assert!(run("animate in pop", &shot, &shot_selected).is_err());
    }

    #[test]
    fn captions_are_translated_from_the_selected_track_or_the_one_shown() {
        let (project, selection) = project_with_title();
        let track = project.clip(selection.clips[0]).unwrap().0.id;
        let asked = run("translate captions into pt_br", &project, &selection).unwrap();
        assert!(
            matches!(asked.as_slice(), [Action::TranslateCaptions { track_id, language }]
                if *track_id == track && language == "pt-BR"),
            "{asked:?}"
        );
        // With nothing selected, the one caption track shown is meant.
        let nothing = Selection::default();
        let asked = run("translate captions to Spanish", &project, &nothing).unwrap();
        assert!(matches!(asked.as_slice(),
            [Action::TranslateCaptions { track_id, language }] if *track_id == track && language == "es"));
        let refused = run("translate captions to English", &project, &selection).unwrap_err();
        assert!(refused.contains("already in English"), "{refused}");
        assert!(run("translate captions to Klingonese", &project, &selection).is_err());

        // A second caption track shown needs a selection to choose.
        let mut editor = editor_core::Editor::new(project).unwrap();
        let (command, _) = captions_tab::new_track(editor.project(), "de");
        editor.dispatch(command).unwrap();
        let refused = run("translate captions to Spanish", editor.project(), &nothing).unwrap_err();
        assert!(refused.contains("Select a caption"), "{refused}");
        assert!(
            run(
                "translate captions to Spanish",
                editor.project(),
                &selection
            )
            .is_ok()
        );
    }

    #[test]
    fn captions_from_speech_are_asked_for_in_the_captions_tab() {
        let (project, selection) = project_with_title();
        let asked = run("Generate captions", &project, &selection).unwrap();
        assert!(matches!(
            asked.as_slice(),
            [
                Action::ShowLibrary(Tab::Captions),
                Action::CaptionsFromSpeech
            ]
        ));
    }

    #[test]
    fn a_track_switch_names_the_track_it_could_not_find() {
        let project = Project::default();
        let error = run("mute track Nope", &project, &Selection::default()).unwrap_err();
        assert!(error.contains("Audio 1"));
    }
}
