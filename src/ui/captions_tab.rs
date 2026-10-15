//! The library's Captions tab: the project's caption tracks and their
//! languages, the cues of one of them to read, retype and retime, and
//! subtitle files in and out.
//!
//! A caption is a title on a caption track (a text track with a language),
//! so every edit here is one of the editor's commands and one step of its
//! history, and the timeline's gestures work on the same cues. The list
//! draws only the rows in view, so a track of thousands of cues costs what
//! a screenful does.
use super::{
    Action, Selection,
    text_tab::{self, Group, Look},
    widgets::{self, Kind},
};
use crate::{
    icons::{self, Icon},
    languages,
    presets::Presets,
    rules::{self, Edge},
    theme::{self, Palette, metrics},
    timefmt,
};
use editor_core::{
    Clip, ClipContent, ClipId, Command, Project, Time, Track, TrackId, TrackKind,
    captions::{self, Cue, SubtitleFormat},
    title,
};
use eframe::egui::{
    self, CornerRadius, Id, Key, KeyboardShortcut, Modifiers, Rect, Sense, Stroke, Ui, Vec2,
    WidgetInfo, WidgetType, pos2, vec2,
};
use std::{path::Path, sync::Arc};

mod generate;
pub use generate::{SpeechInput, Stretch};

/// The height of one cue's row.
const ROW: f32 = 62.0;
/// How long a caption added by hand lasts, unless the next one comes first.
const NEW_CUE: Time = Time(2_000_000);
/// What a caption added by hand says until it is typed over.
pub const NEW_TEXT: &str = "New caption";

// ---- What the tab asks of the editor ----------------------------------------

/// The project's caption tracks, in the project's order.
pub fn caption_tracks(project: &Project) -> Vec<&Track> {
    (project.tracks.iter())
        .map(AsRef::as_ref)
        .filter(|track| track.kind == TrackKind::Text && track.language.is_some())
        .collect()
}

/// The caption track in view: the one chosen while it is one, else that of
/// a selected caption, else the first.
pub fn in_view(
    project: &Project,
    chosen: Option<TrackId>,
    selection: &Selection,
) -> Option<TrackId> {
    let tracks = caption_tracks(project);
    let is_caption = |id: TrackId| tracks.iter().any(|track| track.id == id);
    chosen
        .filter(|id| is_caption(*id))
        .or_else(|| {
            (selection.clips.iter())
                .filter_map(|clip| project.clip(*clip))
                .map(|(track, _)| track.id)
                .find(|id| is_caption(*id))
        })
        .or_else(|| tracks.first().map(|track| track.id))
}

/// What a caption track in `language` is called when it is made.
pub fn track_name(language: &str) -> String {
    match languages::name(language) {
        Some(name) => format!("{name} captions"),
        None => format!("Captions ({language})"),
    }
}

/// The name a new caption track in `language` takes in `project`: its
/// language's, numbered when another track is already called that, so the
/// track menu and the command panel can tell them apart. `own` is left out
/// of the count, for a track being renamed.
pub fn free_track_name(project: &Project, language: &str, own: Option<TrackId>) -> String {
    let base = track_name(language);
    let taken = |name: &str| {
        (project.tracks.iter()).any(|track| Some(track.id) != own && track.name == name)
    };
    std::iter::once(base.clone())
        .chain((2..).map(|number| format!("{base} {number}")))
        .find(|name| !taken(name))
        .unwrap_or(base)
}

/// Whether `name` is what a caption track in `language` is called when it
/// is made, numbered or not.
fn named_for(name: &str, language: &str) -> bool {
    let base = track_name(language);
    name == base
        || (name.strip_prefix(&base))
            .and_then(|rest| rest.strip_prefix(' '))
            .is_some_and(|number| number.parse::<u32>().is_ok_and(|number| number >= 2))
}

/// What the AI provider is asked for a copy of `track` in `language`, and
/// the captions it is to see whole. Refused for a track it cannot copy.
pub fn translation(
    project: &Project,
    track: &Track,
    language: &str,
) -> Result<(String, Vec<ClipId>), String> {
    let from = (track.language.as_deref()).ok_or("Only a caption track can be translated.")?;
    if from.eq_ignore_ascii_case(language) {
        return Err(format!(
            "These captions are already in {}.",
            languages::label(language)
        ));
    }
    if track.clips.is_empty() {
        return Err("There are no captions to translate.".to_owned());
    }
    if !(track.clips.iter()).all(|clip| matches!(clip.content, ClipContent::Text { .. })) {
        return Err("Only captions are translated, and this track holds graphics too.".to_owned());
    }
    let count = track.clips.len();
    let prompt = format!(
        "Translate the {count} {noun} of the track \"{name}\" ({id}) from {from} into {into}. \
         Propose one copy_captions command: from_track_id {id}, a new track_id, name \"{named}\", \
         language \"{language}\", and texts with one translation for each caption in timeline \
         order, keeping each caption's line breaks.",
        noun = if count == 1 { "caption" } else { "captions" },
        name = track.name,
        id = track.id,
        from = languages::label(from),
        into = languages::label(language),
        named = free_track_name(project, language, None),
    );
    Ok((prompt, track.clips.iter().map(|clip| clip.id).collect()))
}

/// A new, empty caption track in `language`, and its ID.
pub fn new_track(project: &Project, language: &str) -> (Command, TrackId) {
    let mut track = Track::new(free_track_name(project, language, None), TrackKind::Text);
    track.language = Some(language.to_owned());
    let id = track.id;
    (
        Command::Batch {
            label: "Add caption track".into(),
            commands: vec![Command::AddTrack { track }],
        },
        id,
    )
}

/// Gives `track` the language `language`, and that language's name when
/// it was still called by its last one. `None` when it already has it.
pub fn relabel(project: &Project, track_id: TrackId, language: &str) -> Option<Command> {
    let track = project.tracks.iter().find(|track| track.id == track_id)?;
    if track.language.as_deref() == Some(language) {
        return None;
    }
    let mut commands = vec![Command::SetTrackLanguage {
        track_id,
        language: Some(language.to_owned()),
    }];
    let named = (track.language.as_deref()).is_none_or(|old| named_for(&track.name, old));
    if named {
        commands.push(Command::SetTrackState {
            track_id,
            locked: None,
            hidden: None,
            muted: None,
            name: Some(free_track_name(project, language, Some(track_id))),
            gain: None,
            solo: None,
        });
    }
    Some(Command::Batch {
        label: "Set caption language".into(),
        commands,
    })
}

/// The cues of a caption track: its titles, in order.
fn cues_of(track: &Track) -> Vec<&Clip> {
    (track.clips.iter())
        .filter(|clip| matches!(clip.content, ClipContent::Text { .. }))
        .collect()
}

/// The title a new caption of `track` at `at` copies its look from: the
/// last before it, else the first, else the caption look made for the
/// canvas.
fn look_at(project: &Project, track: &Track, at: Time) -> Clip {
    let cues = cues_of(track);
    (cues.iter().rev().find(|clip| clip.start <= at))
        .or_else(|| cues.first())
        .map(|clip| (*clip).clone())
        .unwrap_or_else(|| text_tab::caption_look().clip(Time::ZERO, project))
}

/// A track of `cues` in `language`, each a title with the caption look, as
/// one step. Cues too short for a frame are left out; none left is a
/// refusal.
pub fn imported(
    project: &Project,
    cues: &[Cue],
    language: &str,
) -> Result<(Command, TrackId, usize), String> {
    from_cues(project, cues, language, "Import captions")
}

/// A track of `cues` as [`imported`] makes it, as the step named `label`.
pub fn from_cues(
    project: &Project,
    cues: &[Cue],
    language: &str,
    label: &str,
) -> Result<(Command, TrackId, usize), String> {
    let frame = rules::frame(project.settings.frame_rate);
    let mut track = Track::new(free_track_name(project, language, None), TrackKind::Text);
    let track_id = track.id;
    track.language = Some(language.to_owned());
    let look = text_tab::caption_look().clip(Time::ZERO, project);
    track.clips = (cues.iter())
        .filter_map(|cue| captions::cue_clip(cue, &look, frame))
        .collect();
    if track.clips.is_empty() {
        return Err("The file has no caption long enough to be seen.".into());
    }
    let count = track.clips.len();
    Ok((
        Command::Batch {
            label: label.into(),
            commands: vec![Command::AddTrack { track }],
        },
        track_id,
        count,
    ))
}

/// A caption at `at` on `track`, as long as [`NEW_CUE`] or until the next
/// one, in the look of the one before it.
pub fn added(project: &Project, track_id: TrackId, at: Time) -> Result<(Command, ClipId), String> {
    let track = (project.tracks.iter())
        .find(|track| track.id == track_id)
        .ok_or("That caption track is gone.")?;
    if track.locked {
        return Err("The caption track is locked.".into());
    }
    let frame = rules::frame(project.settings.frame_rate);
    let at = rules::quantize(at.max(Time::ZERO), project.settings.frame_rate);
    if (track.clips.iter()).any(|clip| clip.start <= at && at < clip.end()) {
        return Err(
            "There is a caption at the playhead already: split it, or move the playhead.".into(),
        );
    }
    let next = (track.clips.iter())
        .map(|clip| clip.start)
        .filter(|start| *start > at)
        .min();
    let end = next.map_or(at + NEW_CUE, |next| next.min(at + NEW_CUE));
    let cue = Cue {
        start: at,
        end,
        text: NEW_TEXT.to_owned(),
        words: Vec::new(),
    };
    let clip = captions::cue_clip(&cue, &look_at(project, track, at), frame)
        .ok_or("There is no room for a caption before the next one.")?;
    let id = clip.id;
    Ok((
        Command::Batch {
            label: "Add caption".into(),
            commands: vec![Command::AddClip { track_id, clip }],
        },
        id,
    ))
}

/// A caption of a track that takes edits.
fn editable(project: &Project, clip_id: ClipId) -> Result<(&Track, &Clip), String> {
    let (track, clip) = project.clip(clip_id).ok_or("That caption is gone.")?;
    if track.locked {
        return Err("The caption track is locked.".into());
    }
    Ok((track, clip))
}

/// The caption with its start or end at `at`, on a frame, stopping at its
/// neighbours as a trim on the timeline does. Its words and animations keep
/// their places in time.
pub fn retimed(
    project: &Project,
    clip_id: ClipId,
    edge: Edge,
    at: Time,
) -> Result<Command, String> {
    let (track, clip) = editable(project, clip_id)?;
    let at = rules::quantize(at.max(Time::ZERO), project.settings.frame_rate);
    match edge {
        Edge::Start if at >= clip.end() => return Err("A caption starts before it ends.".into()),
        Edge::End if at <= clip.start => return Err("A caption ends after it starts.".into()),
        _ => {}
    }
    let wanted = match edge {
        Edge::Start => at - clip.start,
        Edge::End => at - clip.end(),
    };
    let (least, most) = rules::trim_range(project, track, clip, edge);
    let delta = wanted.clamp(least, most);
    if delta == Time::ZERO {
        let why = match edge {
            _ if wanted == Time::ZERO => "The caption is already there.",
            Edge::Start if wanted < least => "It would cover the caption before it.",
            Edge::End if wanted > most => "It would cover the caption after it.",
            _ => "A caption lasts at least a frame.",
        };
        return Err(why.into());
    }
    let (source_in, duration) = match edge {
        Edge::Start => (clip.source_in + delta, clip.duration - delta),
        Edge::End => (clip.source_in, clip.duration + delta),
    };
    Ok(Command::Batch {
        label: "Time caption".into(),
        commands: vec![Command::TrimClip {
            clip_id,
            source_in,
            duration,
            ripple: false,
        }],
    })
}

/// The caption split in two at `at`, its words going with their halves.
pub fn split(project: &Project, clip_id: ClipId, at: Time) -> Result<Command, String> {
    let (_, clip) = editable(project, clip_id)?;
    let at = rules::quantize(at, project.settings.frame_rate);
    if at <= clip.start || at >= clip.end() {
        return Err("Put the playhead inside the caption to split it there.".into());
    }
    Ok(Command::Batch {
        label: "Split caption".into(),
        commands: vec![Command::SplitClip { clip_id, at }],
    })
}

/// The caption and the next one of its track as one, from the first's
/// start to the second's end, their texts joined and their word times kept
/// when both have them.
pub fn merged(project: &Project, clip_id: ClipId) -> Result<Command, String> {
    let (track, clip) = editable(project, clip_id)?;
    let next = (cues_of(track).into_iter())
        .find(|other| other.start >= clip.end())
        .ok_or("There is no caption after this one.")?;
    let (
        ClipContent::Text {
            text,
            font_size,
            color,
            words,
            ..
        },
        ClipContent::Text {
            text: next_text,
            words: next_words,
            ..
        },
    ) = (&clip.content, &next.content)
    else {
        return Err("Only captions are merged.".into());
    };
    let joined = format!("{} {}", text.trim_end(), next_text.trim_start());
    let mut commands = vec![
        Command::DeleteClips {
            clip_ids: vec![next.id],
            ripple: false,
        },
        Command::TrimClip {
            clip_id,
            source_in: clip.source_in,
            duration: next.end() - clip.start,
            ripple: false,
        },
        Command::SetText {
            clip_id,
            text: joined.clone(),
            font_size: *font_size,
            color: *color,
        },
    ];
    let shift = next.start - clip.start;
    let both: Vec<title::WordTime> = (words.iter().copied())
        .chain(next_words.iter().map(|word| title::WordTime {
            at: word.at + shift,
            end: word.end + shift,
        }))
        .collect();
    if !words.is_empty() && !next_words.is_empty() && both.len() == title::word_count(&joined) {
        commands.push(Command::SetWords {
            clip_id,
            words: both,
        });
    }
    Ok(Command::Batch {
        label: "Merge captions".into(),
        commands,
    })
}

/// The caption with `text`, its size and colour as they are. An empty
/// caption is refused: deleting it is what is meant.
pub fn retyped(project: &Project, clip_id: ClipId, text: &str) -> Result<Option<Command>, String> {
    let (_, clip) = editable(project, clip_id)?;
    let ClipContent::Text {
        text: had,
        font_size,
        color,
        ..
    } = &clip.content
    else {
        return Ok(None);
    };
    let text = text.trim_end();
    if text.trim().is_empty() {
        return Err("A caption needs words: delete it instead.".into());
    }
    if text == had {
        return Ok(None);
    }
    Ok(Some(Command::SetText {
        clip_id,
        text: text.to_owned(),
        font_size: *font_size,
        color: *color,
    }))
}

/// A subtitle file of `track`, and how many cues it holds.
pub fn exported(
    project: &Project,
    track_id: TrackId,
    format: SubtitleFormat,
) -> Result<(String, usize), String> {
    let track = (project.tracks.iter())
        .find(|track| track.id == track_id)
        .ok_or("That caption track is gone.")?;
    let cues = captions::track_cues(track);
    if cues.is_empty() {
        return Err("The caption track has no captions to export.".into());
    }
    let text = match format {
        SubtitleFormat::Srt => captions::format_srt(&cues),
        SubtitleFormat::Vtt => captions::format_vtt(&cues, track.language.as_deref()),
    };
    Ok((text, cues.len()))
}

/// The name a track's subtitle file is offered under: `Project.en.srt`.
pub fn export_name(project: &Project, track_id: TrackId, format: SubtitleFormat) -> String {
    let language = (project.tracks.iter())
        .find(|track| track.id == track_id)
        .and_then(|track| track.language.clone());
    match language {
        Some(language) => format!("{}.{language}.{}", project.name, format.extension()),
        None => format!("{}.{}", project.name, format.extension()),
    }
}

/// The cues of a subtitle file and the language it says it is in: a VTT
/// `Language:` header, or a known tag before the extension
/// (`talk.pt-BR.srt`). Read whole or not at all.
pub fn read_file(path: &Path) -> Result<(Vec<Cue>, Option<String>), String> {
    let shown = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let format = (path.extension())
        .and_then(|extension| extension.to_str())
        .and_then(SubtitleFormat::from_extension)
        .ok_or_else(|| format!("{shown} is not an .srt or .vtt file."))?;
    let size = std::fs::metadata(path)
        .map_err(|error| format!("{shown} could not be read: {error}"))?
        .len();
    if size > captions::MAX_SUBTITLE_BYTES as u64 {
        return Err(format!(
            "{shown} is larger than a subtitle file can be (8 MB)."
        ));
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("{shown} could not be read: {error}"))?;
    let source = decoded(&bytes);
    let cues = captions::parse(&source, format).map_err(|error| format!("{shown}: {error}"))?;
    let header = (format == SubtitleFormat::Vtt)
        .then(|| {
            (source.lines())
                .take_while(|line| !line.trim().is_empty())
                .find_map(|line| line.trim().strip_prefix("Language:"))
                .and_then(languages::tag)
        })
        .flatten();
    let named = (path.file_stem())
        .and_then(|stem| Path::new(stem).extension())
        .and_then(|tag| tag.to_str())
        .and_then(languages::tag)
        .filter(|tag| languages::name(tag).is_some());
    Ok((cues, header.or(named)))
}

/// Writes a subtitle file whole: to a new hidden file beside it first, then
/// renamed into its place, so nothing already there is followed or
/// clobbered on the way.
pub fn write_file(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let shown = path.display();
    let name = path.file_name().map_or_else(
        || "captions".into(),
        |name| name.to_string_lossy().into_owned(),
    );
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    written.map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("Captions were not saved to {shown}: {error}")
    })
}

/// Text of a subtitle file: UTF-8 (its mark dropped), UTF-16 with a mark,
/// or else Windows-1252, as old SubRip files are.
fn decoded(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], big: bool| {
        let units: Vec<u16> = (rest.chunks_exact(2))
            .map(|pair| {
                if big {
                    u16::from_be_bytes([pair[0], pair[1]])
                } else {
                    u16::from_le_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, false);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, true);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|byte| windows_1252(*byte)).collect(),
    }
}

fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

/// A time as the list shows it: `0:01.20`, `1:02:03.40`.
pub fn clock(time: Time) -> String {
    let hundredths = (time.0.max(0) + 5_000) / 10_000;
    let (seconds, hundredths) = (hundredths / 100, hundredths % 100);
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}.{hundredths:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}.{hundredths:02}", seconds / 60, seconds % 60)
    }
}

// ---- The tab ------------------------------------------------------------------

#[derive(Default)]
pub struct CaptionsTabState {
    /// The caption track the person chose to see.
    pub track: Option<TrackId>,
    /// The caption whose words are being typed, and what is typed.
    typing: Option<(ClipId, String)>,
    /// The caption whose start or end is being typed, and what is typed.
    timing: Option<(ClipId, Edge, String)>,
    /// A caption whose field is to take the keyboard once it is in view.
    focus: Option<ClipId>,
    /// What is typed in a language menu.
    language: String,
    /// Whether the track menu is choosing the language of a new track.
    naming: bool,
    /// Whether the track menu is choosing the language to translate into.
    translating: bool,
    /// The caption track of the selected caption, as last seen: choosing
    /// a caption of another track shows that track.
    selected: Option<TrackId>,
    /// The cue under the playhead last frame, and where the list was
    /// scrolled to.
    followed: Option<ClipId>,
    offset: f32,
    /// The choices of captions from speech are shown.
    pub speech_open: bool,
    /// What captions from speech hear.
    stretch: Stretch,
}

impl CaptionsTabState {
    /// Whether a field of the tab holds the keyboard: shortcuts wait.
    pub fn typing(&self) -> bool {
        self.typing.is_some() || self.timing.is_some()
    }
}

pub struct CaptionsTabInput<'a> {
    pub p: &'a Palette,
    pub project: &'a Project,
    pub playhead: Time,
    pub presets: &'a Arc<Presets>,
    pub speech: SpeechInput<'a>,
}

/// A button that opens a menu, as wide as `rect`: what is chosen, and a
/// chevron.
fn chooser(
    ui: &mut Ui,
    p: &Palette,
    rect: Rect,
    id: Id,
    name: &str,
    shown: &str,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::ComboBox, ui.is_enabled(), name);
        info.current_text_value = Some(shown.to_owned());
        info
    });
    let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
    let fill = if response.hovered() {
        p.hover
    } else {
        p.control
    };
    ui.painter().rect_filled(rect, radius, fill);
    let label = widgets::truncated(ui, shown, theme::medium(12.0), p.fg, rect.width() - 34.0);
    ui.painter().galley(
        pos2(rect.left() + 10.0, rect.center().y - label.size().y * 0.5),
        label,
        p.fg,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.right() - 14.0, rect.center().y),
            Vec2::splat(12.0),
        ),
        Icon::ChevronDown,
        p.secondary,
    );
    response
}

/// The languages to choose from, filtered by what is typed, and a typed
/// tag that is not among them. Gives the tag chosen.
fn language_menu(
    ui: &mut Ui,
    p: &Palette,
    typed: &mut String,
    current: Option<&str>,
) -> Option<String> {
    widgets::menu_style(ui, 220.0);
    let field = ui.add(
        egui::TextEdit::singleline(typed)
            .hint_text("Language or tag")
            .desired_width(f32::INFINITY)
            .font(theme::regular(12.5)),
    );
    field.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, "Language or tag"));
    if !field.has_focus() && typed.is_empty() && ui.memory(|memory| memory.focused().is_none()) {
        field.request_focus();
    }
    ui.add_space(4.0);
    let query = typed.trim().to_lowercase();
    let mut chosen = None;
    if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        chosen = languages::tag(typed);
    }
    // A menu that turns into this list offers only the height it had, and
    // never grows by itself: the list asks for its own, and shrinks only to
    // what it holds.
    egui::ScrollArea::vertical()
        .max_height(240.0)
        .min_scrolled_height(240.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let listed = (languages::LANGUAGES.iter()).filter(|(code, name)| {
                query.is_empty() || name.to_lowercase().contains(&query) || *code == query
            });
            let mut any = false;
            // A tag of its own, such as pt-BR, is checked above the list.
            if let Some(current) = current
                && query.is_empty()
                && !(languages::LANGUAGES.iter()).any(|(code, _)| *code == current)
            {
                any = true;
                let name = languages::name(current).unwrap_or(current);
                if widgets::menu_item(ui, p, Some(Icon::Check), name, current).clicked() {
                    chosen = Some(current.to_owned());
                }
                ui.separator();
            }
            for (code, name) in listed {
                any = true;
                let icon = (current == Some(*code)).then_some(Icon::Check);
                if widgets::menu_item(ui, p, icon, name, code).clicked() {
                    chosen = Some((*code).to_owned());
                }
            }
            if let Some(tag) = languages::tag(typed)
                && !(languages::LANGUAGES.iter()).any(|(code, _)| *code == tag)
            {
                any = true;
                if widgets::menu_item(ui, p, None, &format!("Use “{tag}”"), "").clicked() {
                    chosen = Some(tag);
                }
            }
            if !any {
                ui.label(
                    egui::RichText::new("Type a language tag, such as pt-BR.")
                        .font(theme::regular(11.5))
                        .color(p.muted),
                );
            }
        });
    if chosen.is_some() {
        typed.clear();
        ui.close();
    }
    chosen
}

/// The tab in `rect`: the caption track in view, its language and look,
/// and its cues; or, with no caption track yet, how to make one.
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    search: &str,
    state: &mut CaptionsTabState,
    selection: &mut Selection,
    input: &CaptionsTabInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let project = input.project;
    // Captions being made, or made and waiting, are above everything.
    let used = generate::status(ui, p, rect, input.speech.view, actions);
    let rect = Rect::from_min_max(pos2(rect.left(), rect.top() + used), rect.max);
    if state.speech_open {
        generate::panel(
            ui,
            p,
            rect,
            project,
            selection,
            state,
            input.speech,
            actions,
        );
        return;
    }
    let area = rect.shrink2(vec2(10.0, 0.0));
    let selected = (selection.clips.iter())
        .filter_map(|id| project.clip(*id))
        .map(|(track, _)| track)
        .find(|track| track.kind == TrackKind::Text && track.language.is_some())
        .map(|track| track.id);
    if selected != state.selected {
        state.selected = selected;
        if selected.is_some() {
            state.track = selected;
        }
    }
    let Some(track_id) = in_view(project, state.track, selection) else {
        empty(ui, p, area, project, state, actions);
        return;
    };
    let Some(track) = project.tracks.iter().find(|track| track.id == track_id) else {
        return;
    };
    let language = track.language.clone().unwrap_or_default();

    // The track, whether it is shown, and a new one.
    let mut top = area.top();
    let line = |top: f32| {
        Rect::from_min_size(
            pos2(area.left(), top),
            vec2(area.width(), metrics::CONTROL_HEIGHT),
        )
    };
    let first = line(top);
    let eye = Rect::from_min_max(
        pos2(first.right() - metrics::CONTROL_HEIGHT, first.top()),
        first.max,
    );
    let picker = Rect::from_min_max(first.min, pos2(eye.left() - 4.0, first.bottom()));
    let tracks = chooser(
        ui,
        p,
        picker,
        Id::new("caption-track"),
        "Caption track",
        &track.name,
    );
    egui::Popup::menu(&tracks)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            if state.naming {
                if let Some(tag) = language_menu(ui, p, &mut state.language, None) {
                    let (command, id) = new_track(project, &tag);
                    actions.push(Action::Edit(command));
                    state.track = Some(id);
                    state.naming = false;
                }
                return;
            }
            if state.translating {
                if let Some(tag) = language_menu(ui, p, &mut state.language, None) {
                    actions.push(Action::TranslateCaptions {
                        track_id,
                        language: tag,
                    });
                    state.translating = false;
                }
                return;
            }
            widgets::menu_style(ui, 240.0);
            for other in caption_tracks(project) {
                let icon = (other.id == track_id).then_some(Icon::Check);
                let count = cues_of(other).len();
                let tag = other.language.as_deref().unwrap_or_default();
                if widgets::menu_item(ui, p, icon, &other.name, &format!("{tag} · {count}"))
                    .clicked()
                {
                    state.track = Some(other.id);
                    ui.close();
                }
            }
            ui.separator();
            if widgets::menu_item(ui, p, Some(Icon::Plus), "New caption track…", "").clicked() {
                state.naming = true;
                state.language.clear();
            }
            let copyable = !track.clips.is_empty()
                && (track.clips.iter())
                    .all(|clip| matches!(clip.content, ClipContent::Text { .. }));
            let translate = ui.add_enabled_ui(copyable, |ui| {
                widgets::menu_item(ui, p, Some(Icon::Sparkles), "Translate into…", "AI")
            });
            if translate
                .inner
                .on_hover_text("A copy in another language, written by the AI and shown for review")
                .clicked()
            {
                state.translating = true;
                state.language.clear();
            }
        });
    if !tracks.contains_pointer()
        && !egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&tracks))
    {
        state.naming = false;
        state.translating = false;
    }
    let hidden = track.hidden;
    let shown = ui.interact(eye, Id::new("caption-track-shown"), Sense::click());
    let label = if hidden {
        "Show captions"
    } else {
        "Hide captions"
    };
    shown.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    ui.painter().rect_filled(
        eye,
        CornerRadius::same(metrics::CONTROL_RADIUS),
        if shown.hovered() { p.hover } else { p.control },
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(eye.center(), Vec2::splat(14.0)),
        if hidden { Icon::EyeOff } else { Icon::Eye },
        if hidden { p.muted } else { p.secondary },
    );
    if shown.on_hover_text(label).clicked() {
        actions.push(Action::Edit(Command::SetTrackState {
            track_id,
            locked: None,
            hidden: Some(!hidden),
            muted: None,
            name: None,
            gain: None,
            solo: None,
        }));
    }
    top = first.bottom() + 6.0;

    // Its language and its look.
    let second = line(top);
    let half = (second.width() - 6.0) * 0.5;
    let left = Rect::from_min_size(second.min, vec2(half, second.height()));
    let right = Rect::from_min_size(
        pos2(left.right() + 6.0, second.top()),
        vec2(half, second.height()),
    );
    let tongue = chooser(
        ui,
        p,
        left,
        Id::new("caption-language"),
        "Caption language",
        &languages::label(&language),
    );
    egui::Popup::menu(&tongue)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            if let Some(tag) = language_menu(ui, p, &mut state.language, Some(&language))
                && let Some(command) = relabel(project, track_id, &tag)
            {
                actions.push(Action::Edit(command));
            }
        });
    let cues = cues_of(track);
    let ids: Vec<ClipId> = cues.iter().map(|clip| clip.id).collect();
    let look = chooser(
        ui,
        p,
        right,
        Id::new("caption-style"),
        "Caption style",
        "Style",
    );
    egui::Popup::menu(&look)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            widgets::menu_style(ui, 220.0);
            let picked = (selection.clips.iter())
                .filter_map(|id| project.clip(*id))
                .find(|(track, clip)| {
                    track.id == track_id && matches!(clip.content, ClipContent::Text { .. })
                });
            let mut given: Option<Look> = None;
            if let Some((_, clip)) = picked
                && widgets::menu_item(ui, p, Some(Icon::Text), "The selected caption's", "")
                    .clicked()
            {
                given = Some(Look::of_title("the selected caption", clip.content.clone()));
            }
            for kept in (input.presets.looks.iter())
                .filter(|kept| matches!(kept.content, ClipContent::Text { .. }))
            {
                if widgets::menu_item(ui, p, None, &kept.name, "Yours").clicked() {
                    given = Some(Look::saved(kept));
                }
            }
            for builtin in text_tab::builtin()
                .iter()
                .filter(|look| look.group == Group::Captions)
            {
                if widgets::menu_item(ui, p, None, &builtin.name, "").clicked() {
                    given = Some(builtin.as_ref().clone());
                }
            }
            if let Some(given) = given {
                actions.push(text_tab::restyle(project, &given, &ids));
                ui.close();
            }
        });
    look.on_hover_text("Give every caption of the track one look");
    top = second.bottom() + 8.0;

    // The cues, and a new one at the playhead.
    let foot = Rect::from_min_max(
        pos2(area.left(), rect.bottom() - metrics::CONTROL_HEIGHT - 10.0),
        pos2(area.right(), rect.bottom() - 10.0),
    );
    let list = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), foot.top() - 6.0));
    let query = search.trim().to_lowercase();
    let shown: Vec<(usize, &Clip)> = (cues.iter().copied().enumerate())
        .filter(|(_, clip)| query.is_empty() || text_of(clip).to_lowercase().contains(&query))
        .collect();
    if cues.is_empty() {
        let hint = Rect::from_min_max(list.min + vec2(10.0, 0.0), list.max - vec2(10.0, 0.0));
        ui.put(
            hint,
            egui::Label::new(
                egui::RichText::new("No captions yet. Add one at the playhead, import a subtitle file, or let a look of the Text tab go on this track.")
                    .font(theme::regular(11.5))
                    .color(p.muted),
            )
            .wrap(),
        );
    } else if shown.is_empty() {
        let hint = Rect::from_min_max(list.min + vec2(10.0, 0.0), list.max - vec2(10.0, 0.0));
        ui.put(
            hint,
            egui::Label::new(
                egui::RichText::new(format!("No caption says “{}”.", search.trim()))
                    .font(theme::regular(11.5))
                    .color(p.muted),
            )
            .wrap(),
        );
    } else {
        rows(ui, list, &shown, track, state, selection, input, actions);
    }

    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(foot)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    if widgets::button(&mut bar, p, Some(Icon::Plus), "Add caption", Kind::Plain)
        .on_hover_text("Add a caption at the playhead")
        .clicked()
    {
        match added(project, track_id, input.playhead) {
            Ok((command, clip)) => {
                actions.push(Action::Edit(command));
                selection.only(clip);
                state.focus = Some(clip);
            }
            Err(why) => actions.push(Action::Notify(why)),
        }
    }
    bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let count = match (cues.len(), shown.len()) {
            (1, _) if query.is_empty() => "1 caption".to_owned(),
            (all, _) if query.is_empty() => format!("{all} captions"),
            (all, found) => format!("{found} of {all}"),
        };
        ui.label(
            egui::RichText::new(count)
                .font(theme::regular(11.5))
                .color(p.muted),
        );
    });
}

fn text_of(clip: &Clip) -> &str {
    match &clip.content {
        ClipContent::Text { text, .. } => text,
        _ => "",
    }
}

/// With no caption track yet: what captions are here and how to begin.
fn empty(
    ui: &mut Ui,
    p: &Palette,
    area: Rect,
    project: &Project,
    state: &mut CaptionsTabState,
    actions: &mut Vec<Action>,
) {
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(area.shrink2(vec2(4.0, 12.0))));
    body.spacing_mut().item_spacing = vec2(0.0, 8.0);
    body.label(
        egui::RichText::new("Captions")
            .font(theme::medium(13.0))
            .color(p.fg),
    );
    body.add(
        egui::Label::new(
            egui::RichText::new(
                "Captions are titles on a caption track, one track for each language. Generate them from what is said, import a subtitle file, or start a track and type them.",
            )
            .font(theme::regular(11.5))
            .color(p.muted),
        )
        .wrap(),
    );
    body.add_space(4.0);
    if widgets::button(
        &mut body,
        p,
        Some(Icon::Speech),
        "Generate from speech",
        Kind::Primary,
    )
    .on_hover_text("Write captions from what is said on the timeline")
    .clicked()
    {
        state.speech_open = true;
        actions.push(Action::LookAtModels);
    }
    let start = widgets::button(
        &mut body,
        p,
        Some(Icon::Plus),
        "New caption track",
        Kind::Plain,
    );
    egui::Popup::menu(&start)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            if let Some(tag) = language_menu(ui, p, &mut state.language, None) {
                let (command, id) = new_track(project, &tag);
                actions.push(Action::Edit(command));
                state.track = Some(id);
            }
        });
    if widgets::button(
        &mut body,
        p,
        Some(Icon::Import),
        "Import SRT or VTT…",
        Kind::Plain,
    )
    .clicked()
    {
        actions.push(Action::ImportCaptions);
    }
}

/// The rows of `shown` in `list`, the one under the playhead marked and
/// kept in view while the playhead moves.
#[expect(clippy::too_many_arguments, reason = "the tab's one call")]
fn rows(
    ui: &mut Ui,
    list: Rect,
    shown: &[(usize, &Clip)],
    track: &Track,
    state: &mut CaptionsTabState,
    selection: &mut Selection,
    input: &CaptionsTabInput,
    actions: &mut Vec<Action>,
) {
    let playhead = input.playhead;
    let current =
        (shown.iter()).position(|(_, clip)| clip.start <= playhead && playhead < clip.end());
    let current_id = current.map(|index| shown[index].1.id);
    let wanted = state
        .focus
        .and_then(|id| shown.iter().position(|(_, clip)| clip.id == id));
    // The list follows the playhead from cue to cue, and goes to a new one.
    let mut scroll = None;
    let height = list.height();
    let keep_in_view = |index: usize, offset: f32| {
        let top = index as f32 * ROW;
        (top < offset || top + ROW > offset + height).then(|| (top - ROW).max(0.0))
    };
    if let Some(index) = wanted {
        scroll = keep_in_view(index, state.offset);
    } else if current_id != state.followed
        && !state.typing()
        && let Some(index) = current
    {
        scroll = keep_in_view(index, state.offset);
    }
    state.followed = current_id;

    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    // Rows touch: a row's place in the list is its index times its height.
    child.spacing_mut().item_spacing = Vec2::ZERO;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("caption-rows", track.id))
        .auto_shrink([false, false]);
    if let Some(offset) = scroll {
        area = area.vertical_scroll_offset(offset);
    }
    let mut drawn = 0..0;
    let output = area.show_rows(&mut child, ROW, shown.len(), |ui, range| {
        drawn = range.clone();
        for index in range {
            let (number, clip) = shown[index];
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
            cue_row(
                ui,
                row,
                number + 1,
                clip,
                track,
                current == Some(index),
                state,
                selection,
                input,
                actions,
            );
        }
    });
    state.offset = output.state.offset.y;
    // Words typed for a caption scrolled out of view are kept as typed.
    let in_view = |id: ClipId| (shown[drawn.clone()].iter()).any(|(_, clip)| clip.id == id);
    if let Some((id, typed)) = state.typing.take_if(|(id, _)| !in_view(*id)) {
        retype(input.project, id, &typed, actions);
    }
    state.timing.take_if(|(id, _, _)| !in_view(*id));
}

/// Whether Return was pressed without Shift, as the key's own event says.
fn plain_return(ui: &Ui) -> bool {
    ui.input(|i| {
        i.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.shift
            )
        })
    })
}

/// Gives a caption the words typed for it.
fn retype(project: &Project, clip: ClipId, typed: &str, actions: &mut Vec<Action>) {
    match retyped(project, clip, typed) {
        Ok(Some(command)) => actions.push(Action::Edit(command)),
        Ok(None) => {}
        Err(why) => actions.push(Action::Notify(why)),
    }
}

/// One cue: its times, each typed over when clicked, and its words in a
/// field. Clicking the row selects the cue and goes to its start.
#[expect(clippy::too_many_arguments, reason = "one call per row")]
fn cue_row(
    ui: &mut Ui,
    row: Rect,
    number: usize,
    clip: &Clip,
    track: &Track,
    current: bool,
    state: &mut CaptionsTabState,
    selection: &mut Selection,
    input: &CaptionsTabInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let project = input.project;
    let inner = row.shrink2(vec2(10.0, 3.0));
    let response = ui.interact(inner, Id::new(("caption-row", clip.id)), Sense::click());
    let selected = selection.contains(clip.id);
    let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
    if selected {
        ui.painter()
            .rect_filled(inner, radius, theme::tint(p.accent, 0.16));
    } else if response.hovered() {
        ui.painter().rect_filled(inner, radius, p.hover);
    }
    if current {
        let mark =
            Rect::from_min_size(inner.min + vec2(0.0, 6.0), vec2(3.0, inner.height() - 12.0));
        ui.painter()
            .rect_filled(mark, CornerRadius::same(2), p.accent);
    }
    if response.clicked() {
        selection.only(clip.id);
        actions.push(Action::Seek(clip.start));
    }

    // The times.
    let rate = project.settings.frame_rate;
    let times_top = inner.top() + 4.0;
    let mut x = inner.left() + 10.0;
    let mut times = Vec::new();
    for (edge, time) in [(Edge::Start, clip.start), (Edge::End, clip.end())] {
        let name = match edge {
            Edge::Start => format!("Start of caption {number}"),
            Edge::End => format!("End of caption {number}"),
        };
        let text = clock(time);
        let width = if clip.end() >= Time::from_seconds(3600.0) {
            76.0
        } else {
            58.0
        };
        let cell = Rect::from_min_size(pos2(x, times_top), vec2(width, 16.0));
        let typing = matches!(&state.timing, Some((id, typed_edge, _)) if *id == clip.id && *typed_edge == edge);
        if typing {
            let Some((_, _, typed)) = &mut state.timing else {
                unreachable!()
            };
            let mut field =
                ui.new_child(egui::UiBuilder::new().max_rect(cell.expand2(vec2(2.0, 1.0))));
            let edit = field.add(
                egui::TextEdit::singleline(typed)
                    .desired_width(width)
                    .font(theme::mono(11.0))
                    .margin(vec2(2.0, 0.0)),
            );
            edit.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, &name));
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            if edit.lost_focus() {
                let typed = typed.clone();
                state.timing = None;
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    let parsed = timefmt::parse(&typed, rate)
                        .ok_or_else(|| format!("“{typed}” is not a time."));
                    let command = parsed.and_then(|at| retimed(project, clip.id, edge, at));
                    match command {
                        Ok(command) => actions.push(Action::Edit(command)),
                        Err(why) => actions.push(Action::Notify(why)),
                    }
                }
            }
        } else {
            let label = ui.interact(
                cell,
                Id::new(("caption-time", clip.id, edge == Edge::Start)),
                Sense::click(),
            );
            label.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
            let ink = if label.hovered() { p.fg } else { p.muted };
            ui.painter().text(
                cell.left_center(),
                egui::Align2::LEFT_CENTER,
                &text,
                theme::mono(11.0),
                ink,
            );
            let label = label.on_hover_text("Click to type a time");
            if label.clicked() && !track.locked {
                state.timing = Some((clip.id, edge, text));
                selection.only(clip.id);
            }
            times.push(label);
        }
        if edge == Edge::Start {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(x + width + 7.0, times_top + 8.0), Vec2::splat(11.0)),
                Icon::ArrowRight,
                p.muted,
            );
        }
        x += width + 16.0;
    }

    // The words, typed over in a field that grows over the rows below.
    let field_rect = Rect::from_min_max(
        pos2(inner.left() + 6.0, times_top + 17.0),
        pos2(inner.right() - 4.0, inner.bottom() - 2.0),
    );
    let mut typing = matches!(&state.typing, Some((id, _)) if *id == clip.id);
    let fresh = !typing;
    if !typing {
        let text = text_of(clip);
        let sense = if track.locked {
            Sense::hover()
        } else {
            Sense::click()
        };
        let words = ui.interact(field_rect, Id::new(("caption-text", clip.id)), sense);
        words.widget_info(|| {
            let mut info = WidgetInfo::new(WidgetType::TextEdit);
            info.label = Some(format!("Caption {number}"));
            info.current_text_value = Some(text.to_owned());
            info.enabled = !track.locked;
            info
        });
        let mut job = egui::text::LayoutJob::simple(
            text.to_owned(),
            theme::regular(12.0),
            p.fg,
            field_rect.width() - 8.0,
        );
        job.wrap.max_rows = 2;
        job.wrap.overflow_character = Some('…');
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        ui.painter()
            .with_clip_rect(field_rect.intersect(ui.clip_rect()))
            .galley(field_rect.min + vec2(4.0, 2.0), galley, p.fg);
        if words.hovered() && !track.locked {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        let start = state.focus == Some(clip.id) || words.clicked() || words.gained_focus();
        if start && !track.locked {
            state.typing = Some((clip.id, text.to_owned()));
            state.focus = Some(clip.id);
            typing = true;
            if !selection.contains(clip.id) {
                selection.only(clip.id);
            }
            if !(clip.start <= input.playhead && input.playhead < clip.end()) {
                actions.push(Action::Seek(clip.start));
            }
        }
        times.push(words);
    }
    if typing && let Some((_, typed)) = &mut state.typing {
        let editor = Id::new(("caption-editor", clip.id));
        let edit = egui::Area::new(Id::new(("caption-editing", clip.id)))
            .order(egui::Order::Foreground)
            .fixed_pos(field_rect.min - vec2(2.0, 2.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(p.control)
                    .stroke(Stroke::new(1.0, p.accent))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(4, 3))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(typed)
                                .id(editor)
                                .frame(egui::Frame::NONE)
                                .desired_rows(2)
                                .desired_width(field_rect.width() - 4.0)
                                .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                                .font(theme::regular(12.0))
                                .text_color(p.fg)
                                .margin(Vec2::ZERO),
                        )
                    })
                    .inner
            })
            .inner;
        edit.widget_info(|| {
            WidgetInfo::labeled(WidgetType::TextEdit, true, format!("Caption {number}"))
        });
        if state.focus == Some(clip.id) {
            edit.request_focus();
            state.focus = None;
        }
        // Return keeps the words; Shift+Return starts a new line.
        if edit.has_focus() && plain_return(ui) {
            ui.memory_mut(|memory| memory.surrender_focus(editor));
        }
        if !fresh
            && !edit.has_focus()
            && let Some((id, typed)) = state.typing.take()
            && !ui.input(|i| i.key_pressed(Key::Escape))
        {
            retype(project, id, &typed, actions);
        }
    }

    // The row's menu opens from its words and times as well.
    let unlocked = !track.locked;
    let menu = (times.into_iter()).fold(response, |menu, part| menu.union(part));
    menu.context_menu(|ui| {
        widgets::menu_style(ui, 220.0);
        let at = input.playhead;
        let mut asked: Option<Result<Command, String>> = None;
        ui.add_enabled_ui(unlocked, |ui| {
            if widgets::menu_item(ui, p, None, "Start at playhead", "").clicked() {
                asked = Some(retimed(project, clip.id, Edge::Start, at));
            }
            if widgets::menu_item(ui, p, None, "End at playhead", "").clicked() {
                asked = Some(retimed(project, clip.id, Edge::End, at));
            }
            if widgets::menu_item(ui, p, Some(Icon::Scissors), "Split at playhead", "").clicked() {
                asked = Some(split(project, clip.id, at));
            }
            if widgets::menu_item(ui, p, None, "Merge with next", "").clicked() {
                asked = Some(merged(project, clip.id));
            }
            ui.separator();
            if widgets::menu_item(ui, p, Some(Icon::Trash), "Delete caption", "").clicked() {
                asked = Some(Ok(Command::Batch {
                    label: "Delete caption".into(),
                    commands: vec![Command::DeleteClips {
                        clip_ids: vec![clip.id],
                        ripple: false,
                    }],
                }));
            }
        });
        if let Some(asked) = asked {
            match asked {
                Ok(command) => actions.push(Action::Edit(command)),
                Err(why) => actions.push(Action::Notify(why)),
            }
            ui.close();
        }
    });
}

#[cfg(test)]
#[path = "captions_tab_tests.rs"]
mod tests;
