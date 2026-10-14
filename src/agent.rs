//! The command line of the agent panel.
//!
//! This is a small, exact language over the same actions the interface uses,
//! not a language model: every sentence it accepts maps to one documented
//! edit, and anything else is refused with what it expected. A line that
//! starts with `{` is a serialized core `Command` and is applied as is.
//! External agents do not come through here; they use the live session the
//! application hosts (see `editor_agent::AgentHost`).
use crate::{
    audio, duck, languages, normalize, rules, speed, timefmt,
    ui::{
        Action, Selection,
        browser::{self, Entry},
        captions_tab,
        fades::{self, Asked, FadeEdge},
        inspector,
        library::Tab,
        timeline::marker_at,
        tracks, transitions, with_key,
    },
};
use editor_core::{
    BlendMode, CanvasSettings, Clip, ClipContent, Command, Effect, EffectKind, MediaKind, ParamKey,
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
pub const EXAMPLES: [(&str, &str); 33] = [
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
        "detach audio",
        "Put the selected video clip's sound on an audio track, linked",
    ),
    (
        "separate stems",
        "Split the selected clip's sound into vocals and instruments with Demucs",
    ),
    (
        "fade in 1s · fade out 2s · remove fades",
        "Fade the selected clips' sound at their edges",
    ),
    (
        "normalize",
        "Measure the selected clips and set each one's volume to -14 LUFS",
    ),
    (
        "duck under speech · duck 18 db",
        "Lower the selected clips wherever other clips are heard",
    ),
    (
        "show beats · markers from beats",
        "Ticks at the beats of the selected music, or a marker on each",
    ),
    (
        "pitch up 2 · pitch down 3 · keep pitch",
        "Shift a sound's pitch in semitones, or let its speed keep it",
    ),
    (
        "enhance voice · reduce noise 60% · add eq",
        "Audio effects on the selected clips on audio tracks",
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
    (
        "add background music song.wav",
        "Repeat library sound under the whole project, on a new track",
    ),
    ("add marker Intro · add track audio", "Markers and tracks"),
    ("mute track Audio 1 · lock track Video 1", "Track switches"),
    (
        "solo track Audio 1 · set track volume Audio 1 50%",
        "Hear a track by itself, or set how loud it is",
    ),
    ("canvas 9:16 · rename project Film", "Project settings"),
    (
        "checkpoint Rough cut · restore Rough cut",
        "Keep a named checkpoint, or go back to one",
    ),
    (
        "duplicate project /path/Film copy.aster",
        "Carry on in a copy; this file stays as it is",
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

/// `detach audio`: the selected picture clip's sound becomes a linked clip
/// on an audio track. A clip with nothing to detach is refused here, in the
/// words the application would use.
fn detach_audio(context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    match rules::detach_refusal(context.project, clip.id) {
        Some(reason) => Err(reason),
        None => Ok(vec![Action::DetachAudio(clip.id)]),
    }
}

/// `separate stems`: the whole sound of the selected clip's media is split
/// by the stem separator, and the stems placed under the clip when they
/// come. Nothing is edited here; a clip that is not heard is refused in the
/// words the clip menu uses, and the application refuses it without a
/// separator or a saved project.
fn separate_stems(context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    match crate::stems::refusal(context.project, clip.id) {
        Some(reason) => Err(reason),
        None => Ok(vec![Action::SeparateStems(clip.id)]),
    }
}

/// `add background music song.wav`: that sound of the library, or the one
/// selected there when none is named, under the whole project on a new
/// track. Of several media of that name it is the one a bed can be made of,
/// then a sound file, then the first. A bed that cannot be made is refused
/// here, in the words the application would use.
fn background_music(name: &str, context: &Context) -> Result<Vec<Action>, String> {
    let project = context.project;
    let wanted = name.trim().to_lowercase();
    let asset = if wanted.is_empty() {
        (context.selection.asset).ok_or(
            "Say which sound, such as “add background music song.wav”, or select one in the library.",
        )?
    } else {
        let named =
            || (project.assets.values()).filter(|asset| asset.name.to_lowercase() == wanted);
        named()
            .find(|asset| rules::bed_refusal(project, asset.id).is_none())
            .or_else(|| named().find(|asset| asset.kind == MediaKind::Audio))
            .or_else(|| named().next())
            .ok_or("There is no media with that name in the library.")?
            .id
    };
    match rules::bed_refusal(project, asset) {
        Some(reason) => Err(reason),
        None => Ok(vec![Action::AddBackgroundMusic(asset)]),
    }
}

/// `fade in 1s`, `fade out 2s`, `remove fades`: the sound of the selected
/// clips fades at that edge for that long, held to the room the clip's other
/// fade leaves, or at neither. A clip without sound, and a picture clip
/// whose sound its linked audio clip owns, is refused in words.
fn fade(asked: Asked, context: &Context) -> Result<Vec<Action>, String> {
    fades::command(context.project, &context.selection.clips, asked)
        .map(|command| vec![Action::Edit(command)])
}

/// `normalize`: the sound of each selected clip that is heard is measured,
/// and its volume set from the figures when they come. Nothing is edited
/// here; a clip that would not be measured is refused in the words the
/// clip menu uses.
fn normalize(context: &Context) -> Result<Vec<Action>, String> {
    normalize::takers(context.project, &context.selection.clips)
        .map(|clips| vec![Action::Normalize(clips)])
}

/// `duck under speech`, `duck 18 db`: each selected clip that is heard is
/// lowered wherever the clips not selected are, by 12 dB or the depth
/// given, as volume keyframes once their sound is read. Nothing is edited
/// here; a clip that would not be ducked is refused in the words the clip
/// menu uses.
fn duck_under(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let rest = rest.trim().to_lowercase();
    let depth = if rest.is_empty() {
        duck::DEPTH
    } else {
        let amount = rest.strip_suffix("db").unwrap_or(&rest).trim();
        (amount.parse::<f32>().ok())
            .filter(|db| db.is_finite())
            .and_then(duck::depth_of_db)
            .ok_or_else(|| {
                format!(
                    "Give the depth in dB, from {} to {}, such as “duck under speech 18 db”.",
                    duck::SHALLOWEST_DB,
                    duck::DEEPEST_DB
                )
            })?
    };
    // The whole selection goes: a clip passed over is still no voice.
    duck::takers(context.project, &context.selection.clips)?;
    Ok(vec![Action::Duck {
        clips: context.selection.clips.clone(),
        depth,
    }])
}

/// `show beats`, `hide beats`: the beats of the media of the selected clips
/// on audio tracks are shown on its tiles, or no longer, for the session.
/// Nothing is edited; a selection with no such clip is refused in words.
fn show_beats(shown: bool, context: &Context) -> Result<Vec<Action>, String> {
    let project = context.project;
    let mut assets = Vec::new();
    for id in &context.selection.clips {
        let asset = (project.clip(*id))
            .filter(|(track, _)| track.kind == TrackKind::Audio)
            .and_then(|(_, clip)| clip.asset_id());
        if let Some(asset) = asset.filter(|asset| !assets.contains(asset)) {
            assets.push(asset);
        }
    }
    if assets.is_empty() {
        return Err("Select a clip on an audio track: beats are shown on those.".to_owned());
    }
    Ok(vec![Action::ShowBeats { assets, shown }])
}

/// `markers from beats`: a marker on each beat the selected clip on an
/// audio track shows, as one step, once its media's beats are found. The
/// beats are the window's alone, so the application makes the markers.
fn markers_from_beats(context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    let on_audio =
        (context.project.clip(clip.id)).is_some_and(|(track, _)| track.kind == TrackKind::Audio);
    if !on_audio {
        return Err("Select a clip on an audio track: beats are shown on those.".to_owned());
    }
    Ok(vec![Action::MarkersFromBeats(clip.id)])
}

/// `pitch up 2`, `pitch down 3`: the selected clip on an audio track is
/// shifted that many semitones from its own pitch, which its speed then
/// leaves alone. The shift is set, not added to: said twice it is the same.
fn pitch(rest: &str, sign: f32, context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    let rest = rest.trim().to_lowercase();
    let amount = (["semitones", "semitone", "st"].iter())
        .find_map(|unit| rest.strip_suffix(unit))
        .unwrap_or(&rest)
        .trim();
    let semitones = (amount.parse::<f32>().ok())
        .filter(|semitones| semitones.is_finite() && *semitones >= 0.0)
        .ok_or("Give semitones, such as “pitch up 2”.")?;
    // Down by none is no shift, not a shift of -0.
    speed::pitch_command(context.project, clip.id, sign * semitones + 0.0)
        .map(|command| vec![Action::Edit(command)])
}

/// `keep pitch`: the selected clip on an audio track keeps its pitch
/// whatever its speed, as the inspector's switch has it.
fn keep_pitch(context: &Context) -> Result<Vec<Action>, String> {
    let clip = one_selected(context)?;
    speed::keep_pitch_command(context.project, clip.id, true)
        .map(|command| vec![Action::Edit(command)])
}

/// `enhance voice`, `add eq`: a preset's effects at the end of the stack of
/// each selected clip on an audio track, as one step. Said twice, they are
/// added twice.
fn audio_preset(preset: audio::Preset, context: &Context) -> Result<Vec<Action>, String> {
    let clips = &context.selection.clips;
    audio::give_command(context.project, clips, audio::Given::Preset(preset))
        .map(|command| vec![Action::Edit(command)])
}

/// `reduce noise 60%`: that much of the noise is taken out of the selected
/// clips on audio tracks, by the noise reduction they have or one added at
/// its own noise floor. Without a percentage it is the effect's default.
fn reduce_noise(rest: &str, context: &Context) -> Result<Vec<Action>, String> {
    let rest = rest.trim();
    let rest = rest.strip_prefix("by ").unwrap_or(rest);
    let amount = if rest.is_empty() {
        let target = editor_core::AnimTarget::Effect(EffectKind::Denoise);
        editor_core::look::param_spec(target, ParamKey::Amount).map_or(0.4, |spec| spec.default)
    } else {
        percent(rest).ok_or("Give a percentage, such as “reduce noise 60%”.")? / 100.0
    };
    audio::noise_command(context.project, &context.selection.clips, amount)
        .map(|command| vec![Action::Edit(command)])
}

/// The length a line gives a fade: half a second when it gives none.
fn fade_length(rest: &str, project: &Project) -> Result<Time, String> {
    let rest = rest.trim();
    let rest = rest.strip_prefix("for ").unwrap_or(rest);
    if rest.is_empty() {
        return Ok(fades::QUICK);
    }
    timefmt::parse(rest, project.settings.frame_rate)
        .filter(|length| *length >= Time::ZERO)
        .ok_or_else(|| "Give a length such as 1s or 12f.".to_owned())
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
        "detach audio" => detach_audio(context)?,
        "separate stems" | "separate" | "split stems" => separate_stems(context)?,
        "add background music" => background_music("", context)?,
        "fade in" => fade(Asked::Set(FadeEdge::In, fades::QUICK), context)?,
        "fade out" => fade(Asked::Set(FadeEdge::Out, fades::QUICK), context)?,
        "remove fades" | "remove fade" => fade(Asked::Remove, context)?,
        "normalize" | "normalize loudness" | "normalise" | "normalise loudness" => {
            normalize(context)?
        }
        "duck" | "duck under speech" => duck_under("", context)?,
        "keep pitch" => keep_pitch(context)?,
        "show beats" => show_beats(true, context)?,
        "hide beats" => show_beats(false, context)?,
        "markers from beats" | "mark beats" => markers_from_beats(context)?,
        "enhance voice" => audio_preset(audio::Preset::Voice, context)?,
        "add eq" | "add 3-band eq" | "add 3 band eq" => {
            audio_preset(audio::Preset::ThreeBandEq, context)?
        }
        "add compressor" => given(Entry::Effect(EffectKind::Compressor), context)?,
        "add limiter" => given(Entry::Effect(EffectKind::Limiter), context)?,
        "reduce noise" => reduce_noise("", context)?,
        "pitch up" | "pitch down" => {
            return Err("Say how many semitones, such as “pitch up 2”.".to_owned());
        }
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
            } else if let Some(rest) = after("fade in ") {
                let length = fade_length(&rest.to_lowercase(), project)?;
                fade(Asked::Set(FadeEdge::In, length), context)?
            } else if let Some(rest) = after("fade out ") {
                let length = fade_length(&rest.to_lowercase(), project)?;
                fade(Asked::Set(FadeEdge::Out, length), context)?
            } else if let Some(rest) = after("reduce noise ") {
                reduce_noise(rest, context)?
            } else if let Some(rest) = after("duck under speech ").or_else(|| after("duck ")) {
                duck_under(rest, context)?
            } else if let Some(rest) = after("pitch up ") {
                pitch(rest, 1.0, context)?
            } else if let Some(rest) = after("pitch down ") {
                pitch(rest, -1.0, context)?
            } else if let Some(rest) = after("remove tracker ").or_else(|| after("delete tracker "))
            {
                remove_tracker(rest, context)?
            } else if let Some(rest) = after("follow ") {
                follow(rest, context)?
            } else if let Some(rest) = after("add background music ") {
                background_music(rest, context)?
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
            } else if let Some(rest) = after("set track volume ") {
                vec![Action::Edit(track_volume(rest, project)?)]
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

/// `mute track Audio 1`, `unlock track Video 1`, `hide track Text 1`,
/// `solo track Audio 1`…
fn track_switch(lower: &str, line: &str, project: &Project) -> Result<Option<Command>, String> {
    for (verb, locked, hidden, muted, solo) in [
        ("lock", Some(true), None, None, None),
        ("unlock", Some(false), None, None, None),
        ("hide", None, Some(true), None, None),
        ("show", None, Some(false), None, None),
        ("mute", None, None, Some(true), None),
        ("unmute", None, None, Some(false), None),
        ("solo", None, None, None, Some(true)),
        ("unsolo", None, None, None, Some(false)),
    ] {
        let prefix = format!("{verb} track ");
        if lower.starts_with(&prefix) {
            let track = track_named(project, line.get(prefix.len()..).unwrap_or_default())?;
            // The mix ignores a text track's solo, and its head offers none.
            if solo.is_some() && !tracks::carries_sound(track) {
                return Err("A text track has no sound to solo.".to_owned());
            }
            return Ok(Some(Command::SetTrackState {
                track_id: track.id,
                locked,
                hidden,
                muted,
                name: None,
                gain: None,
                solo,
            }));
        }
    }
    Ok(None)
}

/// `set track volume Audio 1 50%`: the track named before the last word,
/// at that many percent. A volume the project would refuse is refused here
/// in words, not held to the bound.
fn track_volume(rest: &str, project: &Project) -> Result<Command, String> {
    let expected = "Name the track, then a percentage: “set track volume Audio 1 50%”.";
    let (name, value) = rest.trim().rsplit_once(' ').ok_or(expected)?;
    let value = percent(value).ok_or(expected)?;
    // "Audio 1" alone is a name that ends in a number, not Audio at 1%.
    if track_named(project, rest).is_ok() {
        return Err(expected.to_owned());
    }
    let track = track_named(project, name)?;
    if !tracks::carries_sound(track) {
        return Err("A text track has no sound to set the volume of.".to_owned());
    }
    if !(0.0..=tracks::MOST).contains(&value) {
        return Err(format!("A track's volume is from 0% to {}%.", tracks::MOST));
    }
    tracks::volume(track, value).ok_or_else(|| expected.to_owned())
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
    const OF_A_PICTURE: [&str; 15] = [
        "add effect",
        "remove effect",
        "apply look",
        "set blend",
        "add transition",
        "remove transition",
        "reverse",
        "freeze frame",
        "chroma key",
        "detach audio",
        "separate stems",
        "fade ",
        "remove fades",
        "normalize",
        "duck",
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
                // A bed needs a sound in the library: it has a test of its own.
                if example.starts_with("add background music") {
                    continue;
                }
                // A pitch needs a clip on an audio track: so does its test.
                if example.starts_with("pitch ") || example == "keep pitch" {
                    continue;
                }
                // So do beats.
                if example.ends_with(" beats") {
                    continue;
                }
                // So does an audio effect.
                let audio = ["enhance voice", "reduce noise", "add eq"];
                if audio.iter().any(|phrase| example.starts_with(phrase)) {
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
    /// selected, with a glow and a dissolve at its start. Its media has
    /// sound, which fades out over its last quarter second.
    fn project_with_shot() -> (Project, Selection, editor_core::ClipId) {
        let mut editor = editor_core::Editor::default();
        let mut shot = editor_core::Asset::new(
            "/media/shot.mp4".into(),
            editor_core::MediaKind::Video,
            Time::from_seconds(6.0),
        );
        shot.name = "shot.mp4".to_owned();
        shot.has_audio = true;
        let track = (editor.project().tracks.iter())
            .find(|track| track.kind == TrackKind::Video)
            .unwrap()
            .id;
        let mut clip = Clip::from_asset(&shot, Time::ZERO);
        clip.duration = Time::from_seconds(4.0);
        clip.effects.push(Effect::new(EffectKind::Glow));
        clip.transition_in = Some(transitions::default_of(&[], transitions::DEFAULT_KIND));
        clip.fade_out = Time::from_seconds(0.25);
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
                Action::DetachAudio(clip) => rules::detach_audio(editor.project(), clip)?,
                Action::AddBackgroundMusic(asset) => {
                    rules::background_music(editor.project(), asset)?
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
        // A table, a plate and a stabilization are not added by name, nor
        // is a pitch, which is kept or shifted by its own phrases.
        assert!(run("add effect lut", &project, &selection).is_err());
        assert!(unknown.contains("compressor") && !unknown.contains("pitch"));
        let refused = run("add effect pitch", &project, &selection).unwrap_err();
        assert!(refused.starts_with("No effect is called"), "{refused}");
        // An audio kind is named, and a picture clip is told why it takes
        // none.
        for phrase in [
            "add effect compressor",
            "add effect high_pass",
            "add limiter",
        ] {
            let refused = run(phrase, &project, &selection).unwrap_err();
            assert_eq!(refused, audio::DETACH_FIRST, "{phrase}");
        }
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
    fn a_sound_is_laid_under_the_project_by_name() {
        let (project, selection, _) = project_with_shot();
        let mut editor = editor_core::Editor::new(project).unwrap();
        let mut song = editor_core::Asset::new(
            "/media/song.wav".into(),
            editor_core::MediaKind::Audio,
            Time::from_seconds(3.0),
        );
        song.name = "Song.wav".to_owned();
        let id = song.id;
        editor
            .dispatch(Command::AddAssets { assets: vec![song] })
            .unwrap();
        let project = editor.project().clone();
        // The example the panel lists, in any case.
        let asked = run("add background music song.wav", &project, &selection).unwrap();
        assert!(matches!(asked.as_slice(), [Action::AddBackgroundMusic(asset)] if *asset == id));
        let after = applied("Add Background Music SONG.WAV", &project, &selection).unwrap();
        let bed = after.tracks.last().unwrap();
        assert_eq!((bed.name.as_str(), bed.kind), ("Music", TrackKind::Audio));
        // Four seconds of project under a three second sound.
        assert_eq!(bed.clips.len(), 2);
        assert!(bed.clips.iter().all(|clip| clip.gain == 0.5));
        assert_eq!(bed.clips[1].fade_out, Time::from_seconds(1.0));

        // Without a name it is the sound selected in the library.
        let chosen = Selection {
            asset: Some(id),
            ..Selection::default()
        };
        let asked = run("add background music", &project, &chosen).unwrap();
        assert!(matches!(asked.as_slice(), [Action::AddBackgroundMusic(asset)] if *asset == id));
        let unnamed = run("add background music", &project, &selection).unwrap_err();
        assert!(unnamed.starts_with("Say which sound"), "{unnamed}");
        let unknown = run("add background music nothing.wav", &project, &selection).unwrap_err();
        assert_eq!(unknown, "There is no media with that name in the library.");
        // A video is not a sound file, and an empty project has no length.
        let picture = run("add background music shot.mp4", &project, &selection).unwrap_err();
        assert_eq!(picture, "Background music is made of a sound file.");
        let mut empty = editor_core::Editor::default();
        let song = project.assets[&id].clone();
        empty
            .dispatch(Command::AddAssets { assets: vec![song] })
            .unwrap();
        let refused = run(
            "add background music song.wav",
            empty.project(),
            &Selection::default(),
        )
        .unwrap_err();
        assert!(
            refused.starts_with("Put a clip on the timeline first"),
            "{refused}"
        );
    }

    #[test]
    fn of_media_with_one_name_the_bed_is_of_the_sound() {
        // The same file name from two folders, and a name that differs in
        // case alone: whichever the library holds first, the phrase takes the
        // sound a bed can be made of. Ids are random, so the order is tried
        // both ways over the runs.
        for _ in 0..16 {
            let (project, selection, _) = project_with_shot();
            let mut editor = editor_core::Editor::new(project).unwrap();
            let named = |name: &str, kind, length| {
                let mut asset = editor_core::Asset::new(
                    format!("/media/{}/{name}", uuid::Uuid::new_v4()).into(),
                    kind,
                    Time::from_seconds(length),
                );
                asset.name = name.to_owned();
                asset
            };
            let video = named("Theme", editor_core::MediaKind::Video, 8.0);
            let short = named("theme", editor_core::MediaKind::Audio, 0.05);
            let song = named("THEME", editor_core::MediaKind::Audio, 3.0);
            let (short_id, id) = (short.id, song.id);
            let assets = vec![video, short, song];
            editor.dispatch(Command::AddAssets { assets }).unwrap();
            let asked = run("add background music theme", editor.project(), &selection).unwrap();
            assert!(
                matches!(asked.as_slice(), [Action::AddBackgroundMusic(asset)] if *asset == id)
            );
            // With no sound of that name that a bed can be made of, the
            // refusal is the sound file's, not the video's.
            editor
                .dispatch(Command::RemoveAssets {
                    asset_ids: vec![id],
                })
                .unwrap();
            let refused = run("add background music theme", editor.project(), &selection);
            assert!(
                refused
                    .as_ref()
                    .is_err_and(|why| why.starts_with("That sound is shorter")),
                "{refused:?} {short_id}"
            );
        }
    }

    #[test]
    fn a_clips_sound_is_detached_by_name_once() {
        let (project, selection, id) = project_with_shot();
        let asked = run("Detach audio", &project, &selection).unwrap();
        assert!(matches!(asked.as_slice(), [Action::DetachAudio(clip)] if *clip == id));
        let after = applied("detach audio", &project, &selection).unwrap();
        let link = after.clip(id).unwrap().1.linked_id;
        let sound: Vec<&Clip> = (after.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .collect();
        assert!(link.is_some() && sound.len() == 1 && sound[0].linked_id == link);
        // The sound is on the linked audio clip now, and the line says so.
        let again = run("detach audio", &after, &selection).unwrap_err();
        assert!(again.contains("linked audio clip"), "{again}");
        // A title has no sound, and nothing selected is no clip.
        let (titled, title) = project_with_title();
        let silent = run("detach audio", &titled, &title).unwrap_err();
        assert_eq!(silent, "That clip has no sound to detach.");
        let nothing = run("detach audio", &project, &Selection::default()).unwrap_err();
        assert!(nothing.starts_with("Select a clip"), "{nothing}");
    }

    #[test]
    fn stems_are_asked_of_the_one_selected_clip_that_is_heard() {
        let (project, selection, id) = project_with_shot();
        for line in ["separate stems", "Separate", "split stems"] {
            let asked = run(line, &project, &selection).unwrap();
            assert!(
                matches!(asked.as_slice(), [Action::SeparateStems(clip)] if *clip == id),
                "{line}: {asked:?}"
            );
        }
        // Detached, the picture's sound is the audio clip's: asked there.
        let after = applied("detach audio", &project, &selection).unwrap();
        let owned = run("separate stems", &after, &selection).unwrap_err();
        assert_eq!(
            owned,
            "Sound is on the linked audio clip: separate it there."
        );
        // A title has no sound, and nothing selected is no clip.
        let (titled, title) = project_with_title();
        let silent = run("separate stems", &titled, &title).unwrap_err();
        assert_eq!(silent, "That clip has no sound to separate.");
        let nothing = run("separate stems", &project, &Selection::default()).unwrap_err();
        assert!(nothing.starts_with("Select a clip"), "{nothing}");
    }

    #[test]
    fn the_sound_of_the_selected_clips_fades_by_name() {
        let (project, selection, id) = project_with_shot();
        let fades = |project: &Project| {
            let clip = project.clip(id).unwrap().1;
            (clip.fade_in, clip.fade_out)
        };
        let quarter = Time::from_seconds(0.25);
        let after = applied("Fade in 1s", &project, &selection).unwrap();
        assert_eq!(fades(&after), (Time::from_seconds(1.0), quarter));
        let after = applied("fade out 2s", &project, &selection).unwrap();
        assert_eq!(fades(&after), (Time::ZERO, Time::from_seconds(2.0)));
        // Frames, a length after "for", and half a second when none is given.
        let after = applied("fade in for 15f", &project, &selection).unwrap();
        assert_eq!(fades(&after).0, Time::from_seconds(0.5));
        let after = applied("fade in", &project, &selection).unwrap();
        assert_eq!(fades(&after).0, fades::QUICK);
        // Longer than the clip has room for beside its other fade is held
        // to that room: four seconds less the quarter that fades out.
        let after = applied("fade in 9s", &project, &selection).unwrap();
        assert_eq!(fades(&after), (Time::from_seconds(3.75), quarter));
        // A length of nothing takes that fade away, and both go by name.
        let after = applied("fade out 0", &project, &selection).unwrap();
        assert_eq!(fades(&after), (Time::ZERO, Time::ZERO));
        let none = run("remove fades", &after, &selection).unwrap_err();
        assert_eq!(none, "There are no fades to remove.");
        let faded = applied("fade in 1s", &project, &selection).unwrap();
        let after = applied("remove fades", &faded, &selection).unwrap();
        assert_eq!(fades(&after), (Time::ZERO, Time::ZERO));
        assert!(run("fade in soon", &project, &selection).is_err());
        assert!(run("fade in -1s", &project, &selection).is_err());
        let nothing = run("fade in 1s", &project, &Selection::default()).unwrap_err();
        assert!(nothing.starts_with("Select a clip"), "{nothing}");

        // A clip without sound is refused in words: a title here.
        let (titled, title) = project_with_title();
        for line in ["fade in 1s", "fade out 2s", "remove fades"] {
            let silent = run(line, &titled, &title).unwrap_err();
            assert_eq!(silent, "That clip has no sound to fade.", "{line}");
        }
        // So is a picture clip whose sound its linked audio clip owns, in
        // the words the inspector has in place of its rows.
        let detached = applied("detach audio", &project, &selection).unwrap();
        for line in ["fade in 1s", "fade out 2s", "remove fades"] {
            let owned = run(line, &detached, &selection).unwrap_err();
            assert_eq!(owned, "Sound is on the linked audio clip.", "{line}");
        }
        // The audio clip that owns the sound fades, and carries the fade
        // the picture had when its sound was detached.
        let sound = (detached.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .next()
            .unwrap();
        assert_eq!(sound.fade_out, quarter);
        let mut owner = Selection::default();
        owner.only(sound.id);
        let after = applied("fade in 1s", &detached, &owner).unwrap();
        let faded = after.clip(sound.id).unwrap().1;
        assert_eq!(faded.fade_in, Time::from_seconds(1.0));
        assert_eq!(fades(&after), fades(&detached), "the picture's are left");
        // Unlinked, the picture's sound is its own again.
        let unlinked = applied("unlink", &detached, &selection).unwrap();
        let after = applied("fade in 1s", &unlinked, &selection).unwrap();
        assert_eq!(fades(&after).0, Time::from_seconds(1.0));
    }

    #[test]
    fn normalize_asks_for_the_selected_clips_that_are_heard_and_refuses_the_rest_in_words() {
        let (project, selection, id) = project_with_shot();
        // Nothing is edited by the line: the clips are measured first.
        for line in ["normalize", "Normalize loudness", "normalise"] {
            let asked = run(line, &project, &selection).unwrap();
            assert!(
                matches!(asked.as_slice(), [Action::Normalize(clips)] if *clips == [id]),
                "{line}: {asked:?}"
            );
        }
        let (titled, title) = project_with_title();
        let silent = run("normalize", &titled, &title).unwrap_err();
        assert_eq!(silent, "That clip has no sound to normalize.");
        let nothing = run("normalize", &project, &Selection::default()).unwrap_err();
        assert_eq!(nothing, "Select a clip first.");
        // A picture whose sound its linked audio clip owns is told so, and
        // the audio clip is the one measured.
        let detached = applied("detach audio", &project, &selection).unwrap();
        let owned = run("normalize", &detached, &selection).unwrap_err();
        assert_eq!(owned, "Sound is on the linked audio clip.");
        let sound = (detached.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .next()
            .unwrap()
            .id;
        let both = Selection {
            clips: vec![id, sound],
            ..Selection::default()
        };
        let asked = run("normalize", &detached, &both).unwrap();
        assert!(matches!(asked.as_slice(), [Action::Normalize(clips)] if *clips == [sound]));
        // A muted track's clip is not heard.
        let track = (detached.tracks.iter())
            .find(|track| track.kind == TrackKind::Audio)
            .unwrap()
            .name
            .clone();
        let mut only = Selection::default();
        only.only(sound);
        let muted = applied(&format!("mute track {track}"), &detached, &only).unwrap();
        let unheard = run("normalize", &muted, &only).unwrap_err();
        assert_eq!(unheard, "That clip is not heard: its track is muted.");
    }

    #[test]
    fn duck_asks_for_the_selected_clips_that_are_heard_at_the_depth_given() {
        let (project, selection, id) = project_with_shot();
        let depth_of = |line: &str| match run(line, &project, &selection).unwrap().as_slice() {
            [Action::Duck { clips, depth }] if *clips == [id] => *depth,
            other => panic!("{line}: {other:?}"),
        };
        // Nothing is edited by the line: the others' sound is read first.
        for line in ["duck", "duck under speech", "Duck Under Speech"] {
            assert_eq!(depth_of(line), duck::DEPTH, "{line}");
        }
        for line in ["duck 18 db", "duck under speech -18dB", "duck 18"] {
            assert!((depth_of(line) - 0.125_89).abs() < 1e-4, "{line}");
        }
        for line in ["duck 0.5 db", "duck 80 db", "duck deeply"] {
            let refused = run(line, &project, &selection).unwrap_err();
            assert!(
                refused.starts_with("Give the depth in dB, from 1 to 60"),
                "{line}"
            );
        }
        let (titled, title) = project_with_title();
        let silent = run("duck", &titled, &title).unwrap_err();
        assert_eq!(silent, "That clip has no sound to duck.");
        let nothing = run("duck", &project, &Selection::default()).unwrap_err();
        assert_eq!(nothing, "Select a clip first.");
        // A picture whose sound its linked audio clip owns is told so, and
        // the audio clip is the one ducked. The whole selection is asked
        // for: a clip passed over is no voice either.
        let detached = applied("detach audio", &project, &selection).unwrap();
        let owned = run("duck under speech", &detached, &selection).unwrap_err();
        assert_eq!(owned, "Sound is on the linked audio clip.");
        let sound = (detached.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .next()
            .unwrap()
            .id;
        let both = Selection {
            clips: vec![id, sound],
            ..Selection::default()
        };
        let asked = run("duck", &detached, &both).unwrap();
        assert!(matches!(asked.as_slice(), [Action::Duck { clips, .. }] if *clips == [id, sound]));
    }

    #[test]
    fn beats_are_shown_and_marked_for_the_selected_clips_on_audio_tracks() {
        let (project, selection, id) = project_with_shot();
        // A picture shows none: its sound goes to an audio track first.
        for line in ["show beats", "hide beats", "markers from beats"] {
            assert_eq!(
                run(line, &project, &selection).unwrap_err(),
                "Select a clip on an audio track: beats are shown on those.",
                "{line}"
            );
        }
        assert_eq!(
            run("markers from beats", &project, &Selection::default()).unwrap_err(),
            "Select a clip first."
        );
        let detached = applied("detach audio", &project, &selection).unwrap();
        let sound = (detached.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .next()
            .unwrap()
            .id;
        let asset = detached.clip(sound).unwrap().1.asset_id().unwrap();
        // Of the picture and its sound, the media is shown once.
        let both = Selection {
            clips: vec![id, sound],
            ..Selection::default()
        };
        for (line, on) in [("show beats", true), ("Hide Beats", false)] {
            let asked = run(line, &detached, &both).unwrap();
            assert!(
                matches!(asked.as_slice(), [Action::ShowBeats { assets, shown }]
                    if *assets == [asset] && *shown == on),
                "{line}: {asked:?}"
            );
        }
        // Markers are asked for of one clip, and made where the beats are.
        let mut owner = Selection::default();
        owner.only(sound);
        for line in ["markers from beats", "mark beats"] {
            let asked = run(line, &detached, &owner).unwrap();
            assert!(
                matches!(asked.as_slice(), [Action::MarkersFromBeats(clip)] if *clip == sound),
                "{line}"
            );
        }
        assert_eq!(
            run("markers from beats", &detached, &both).unwrap_err(),
            "Select one clip; several are selected."
        );
    }

    #[test]
    fn a_sound_on_an_audio_track_is_pitched_in_semitones_and_keeps_its_pitch() {
        let (project, selection, _) = project_with_shot();
        // Only a clip on an audio track carries a pitch: the picture says so.
        for (line, refused) in [
            ("keep pitch", "Detach audio to keep pitch."),
            ("pitch up 2", "Detach audio to change its pitch."),
        ] {
            assert_eq!(run(line, &project, &selection).unwrap_err(), refused);
        }
        let detached = applied("detach audio", &project, &selection).unwrap();
        let sound = (detached.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .next()
            .unwrap()
            .id;
        let mut owner = Selection::default();
        owner.only(sound);
        let read = |project: &Project| editor_core::sound::pitch(project.clip(sound).unwrap().1);

        let kept = applied("keep pitch", &detached, &owner).unwrap();
        assert_eq!(read(&kept), Some(0.0));
        assert_eq!(
            run("keep pitch", &kept, &owner).unwrap_err(),
            "That clip already keeps its pitch."
        );
        // The shift is set, not added to, and the `Pitch` that keeps the
        // pitch is the one that is changed.
        let up = applied("pitch up 2", &kept, &owner).unwrap();
        assert_eq!(read(&up), Some(2.0));
        assert_eq!(up.clip(sound).unwrap().1.effects.len(), 1);
        let down = applied("Pitch down 3 semitones", &up, &owner).unwrap();
        assert_eq!(read(&down), Some(-3.0));
        assert_eq!(
            read(&applied("pitch up 12st", &detached, &owner).unwrap()),
            Some(12.0)
        );
        assert_eq!(
            read(&applied("pitch down 0", &down, &owner).unwrap()),
            Some(0.0)
        );
        assert_eq!(
            run("pitch down 3", &down, &owner).unwrap_err(),
            "That clip is already at that pitch."
        );

        for (line, refused) in [
            ("pitch up 13", "A pitch is from -12 to 12 semitones."),
            ("pitch up -2", "Give semitones, such as “pitch up 2”."),
            ("pitch down a lot", "Give semitones, such as “pitch up 2”."),
            ("pitch up", "Say how many semitones, such as “pitch up 2”."),
        ] {
            assert_eq!(run(line, &detached, &owner).unwrap_err(), refused, "{line}");
        }
        let nothing = run("keep pitch", &detached, &Selection::default()).unwrap_err();
        assert!(nothing.starts_with("Select a clip"), "{nothing}");
        // A clip without sound is refused in words.
        let (titled, title) = project_with_title();
        assert_eq!(
            run("pitch up 2", &titled, &title).unwrap_err(),
            "That clip has no sound."
        );
    }

    #[test]
    fn a_sound_on_an_audio_track_is_given_audio_effects_by_phrase() {
        use EffectKind as K;
        let (project, selection, _) = project_with_shot();
        // A picture with sound is told to detach it first, by every phrase.
        for line in [
            "enhance voice",
            "add eq",
            "add compressor",
            "add limiter",
            "reduce noise 60%",
        ] {
            assert_eq!(
                run(line, &project, &selection).unwrap_err(),
                audio::DETACH_FIRST,
                "{line}"
            );
        }
        let detached = applied("detach audio", &project, &selection).unwrap();
        // Detached, the picture is still selected; it is pointed to its
        // linked sound, not told to detach it again, and nothing changes.
        for line in [
            "enhance voice",
            "add eq",
            "add compressor",
            "reduce noise 60%",
            "add effect high-pass",
        ] {
            assert_eq!(
                run(line, &detached, &selection).unwrap_err(),
                audio::ON_LINKED,
                "{line}"
            );
        }
        let sound = (detached.tracks.iter())
            .filter(|track| track.kind == TrackKind::Audio)
            .flat_map(|track| &track.clips)
            .next()
            .unwrap()
            .id;
        let mut owner = Selection::default();
        owner.only(sound);
        let kinds = |project: &Project| -> Vec<EffectKind> {
            (project.clip(sound).unwrap().1.effects.iter())
                .map(|effect| effect.kind)
                .collect()
        };

        let voiced = applied("Enhance voice", &detached, &owner).unwrap();
        assert_eq!(
            kinds(&voiced),
            [K::HighPass, K::Denoise, K::EqBand, K::Compressor]
        );
        let eq = applied("add eq", &detached, &owner).unwrap();
        let bands: Vec<Option<f32>> = (eq.clip(sound).unwrap().1.effects.iter())
            .map(|effect| effect.param(ParamKey::Frequency))
            .collect();
        assert_eq!(bands, [Some(120.0), Some(1_000.0), Some(8_000.0)]);
        for (line, kind) in [
            ("add compressor", K::Compressor),
            ("add limiter", K::Limiter),
        ] {
            assert_eq!(kinds(&applied(line, &detached, &owner).unwrap()), [kind]);
        }
        // By the effect's own name too, and taken away by it.
        let high = applied("add effect high-pass", &detached, &owner).unwrap();
        assert_eq!(kinds(&high), [K::HighPass]);
        let none = applied("remove effect high pass", &high, &owner).unwrap();
        assert!(kinds(&none).is_empty());

        // Noise reduction is set where it is and added where it is not.
        let amount = |project: &Project| {
            (project.clip(sound).unwrap().1.effects.iter())
                .find(|effect| effect.kind == K::Denoise)
                .and_then(|effect| effect.param(ParamKey::Amount))
        };
        let reduced = applied("reduce noise 60%", &detached, &owner).unwrap();
        assert_eq!(amount(&reduced), Some(0.6));
        assert_eq!(
            run("reduce noise 60%", &reduced, &owner).unwrap_err(),
            "That clip already reduces noise by 60%."
        );
        let reduced = applied("reduce noise by 25%", &voiced, &owner).unwrap();
        assert_eq!(amount(&reduced), Some(0.25));
        assert_eq!(kinds(&reduced).len(), 4, "the one there is set");
        let plain = applied("reduce noise", &detached, &owner).unwrap();
        assert_eq!(amount(&plain), Some(0.4));
        for (line, refused) in [
            ("reduce noise 140%", "Noise reduction is from 0% to 100%."),
            (
                "reduce noise a lot",
                "Give a percentage, such as “reduce noise 60%”.",
            ),
        ] {
            assert_eq!(run(line, &detached, &owner).unwrap_err(), refused, "{line}");
        }
        let nothing = run("enhance voice", &detached, &Selection::default()).unwrap_err();
        assert_eq!(nothing, "Select a clip first.");
        let (titled, title) = project_with_title();
        assert_eq!(
            run("add eq", &titled, &title).unwrap_err(),
            "Only a sound on an audio track takes an audio effect."
        );
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
    fn a_track_is_soloed_and_given_a_volume_by_name() {
        let project = Project::default();
        let none = Selection::default();
        let track = |project: &Project, name: &str| {
            let track = project.tracks.iter().find(|track| track.name == name);
            track.map(|track| (track.solo, track.gain)).unwrap()
        };

        // Each phrase is one step, which `applied` counts; names are found
        // whatever their case.
        let soloed = applied("Solo track audio 1", &project, &none).unwrap();
        assert_eq!(track(&soloed, "Audio 1"), (true, 1.0));
        assert_eq!(track(&soloed, "Video 1"), (false, 1.0));
        let after = applied("unsolo track Audio 1", &soloed, &none).unwrap();
        assert_eq!(after, project);
        // A video track has sound to solo; a text track has none.
        let after = applied("solo track Video 1", &project, &none).unwrap();
        assert!(track(&after, "Video 1").0);
        for line in ["solo track Text 1", "unsolo track Text 1"] {
            let refused = run(line, &project, &none).unwrap_err();
            assert_eq!(refused, "A text track has no sound to solo.", "{line}");
        }

        let after = applied("set track volume Audio 1 50%", &project, &none).unwrap();
        assert_eq!(track(&after, "Audio 1"), (false, 0.5));
        assert_eq!(track(&after, "Video 1"), (false, 1.0));
        // The sign is not needed, and the bounds are the project's own.
        let after = applied("set track volume video 1 800", &project, &none).unwrap();
        assert_eq!(track(&after, "Video 1").1, 8.0);
        let after = applied("set track volume Audio 1 0%", &project, &none).unwrap();
        assert_eq!(track(&after, "Audio 1").1, 0.0);
        for line in [
            "set track volume Audio 1 801%",
            "set track volume Audio 1 -1%",
        ] {
            let refused = run(line, &project, &none).unwrap_err();
            assert_eq!(refused, "A track's volume is from 0% to 800%.", "{line}");
        }
        // Without a number, without a name and of a track that is not there.
        for line in [
            "set track volume Audio 1",
            "set track volume 50%",
            "set track volume Audio 1 loud",
            "set track volume Audio 1 inf",
        ] {
            let refused = run(line, &project, &none).unwrap_err();
            assert!(refused.starts_with("Name the track"), "{line}: {refused}");
        }
        let missing = run("set track volume Nope 50%", &project, &none).unwrap_err();
        assert!(missing.contains("Audio 1"), "{missing}");
        let silent = run("set track volume Text 1 50%", &project, &none).unwrap_err();
        assert_eq!(silent, "A text track has no sound to set the volume of.");
        // The clip's own volume is still its own phrase.
        assert!(run("set volume 80%", &project, &none).is_err());
    }

    #[test]
    fn a_track_switch_names_the_track_it_could_not_find() {
        let project = Project::default();
        let error = run("mute track Nope", &project, &Selection::default()).unwrap_err();
        assert!(error.contains("Audio 1"));
    }
}
