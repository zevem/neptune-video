//! Captions as text: SubRip and WebVTT files read and written whole, and
//! spoken words grouped into cues. Pure: files are read and written by the
//! desktop, and speech is recognized by `editor-media`.
use crate::{
    Clip, ClipContent, Time, Track,
    title::{self, WordTime},
};
use std::{fmt::Write as _, ops::Range};
use uuid::Uuid;

/// The largest subtitle file read.
pub const MAX_SUBTITLE_BYTES: usize = 8 * 1024 * 1024;
/// The most cues one file or one transcript makes.
pub const MAX_CUES: usize = 20_000;
/// The longest text one cue may hold, in bytes.
pub const MAX_CUE_TEXT: usize = 4096;

/// One caption: its text from `start` to `end` on the timeline, and the
/// times of its words when they are known, also on the timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start: Time,
    pub end: Time,
    pub text: String,
    /// One per word of `text` by the title word rule, or none.
    pub words: Vec<WordTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleFormat {
    Srt,
    Vtt,
}

impl SubtitleFormat {
    /// The format a file name's extension names.
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "srt" => Some(Self::Srt),
            "vtt" => Some(Self::Vtt),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
        }
    }
}

/// Why a subtitle file was not read; nothing of it is used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Line {line}: {message}")]
pub struct SubtitleError {
    pub line: usize,
    pub message: String,
}

fn refuse<T>(line: usize, message: impl Into<String>) -> Result<T, SubtitleError> {
    Err(SubtitleError {
        line,
        message: message.into(),
    })
}

/// The cues of a subtitle file, in time order, overlaps settled.
pub fn parse(source: &str, format: SubtitleFormat) -> Result<Vec<Cue>, SubtitleError> {
    if source.len() > MAX_SUBTITLE_BYTES {
        return refuse(1, "The file is larger than 8 MB");
    }
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut cues = match format {
        SubtitleFormat::Srt => parse_srt(source)?,
        SubtitleFormat::Vtt => parse_vtt(source)?,
    };
    if cues.is_empty() {
        return refuse(1, "The file has no captions");
    }
    settle(&mut cues);
    Ok(cues)
}

// The file's lines, numbered from one, with line endings of any platform.
fn lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .enumerate()
        .map(|(index, line)| (index + 1, line))
}

// Blocks of non-blank lines.
fn blocks(source: &str) -> Vec<Vec<(usize, &str)>> {
    let mut blocks = Vec::new();
    let mut block = Vec::new();
    for (number, line) in lines(source) {
        if line.trim().is_empty() {
            if !block.is_empty() {
                blocks.push(std::mem::take(&mut block));
            }
        } else {
            block.push((number, line));
        }
    }
    if !block.is_empty() {
        blocks.push(block);
    }
    blocks
}

fn parse_srt(source: &str) -> Result<Vec<Cue>, SubtitleError> {
    let mut cues = Vec::new();
    for block in blocks(source) {
        let mut rest = &block[..];
        // The cue number is optional in practice; its value is not used.
        if let [(_, first), tail @ ..] = rest
            && !first.contains("-->")
            && first.trim().chars().all(|c| c.is_ascii_digit())
        {
            rest = tail;
        }
        let [(number, timing), text @ ..] = rest else {
            return refuse(block[0].0, "A caption has a number but no times");
        };
        let (start, end) =
            timing_line(timing, SubtitleFormat::Srt).map_err(|message| SubtitleError {
                line: *number,
                message,
            })?;
        let text = text
            .iter()
            .map(|(_, line)| strip_markup(line))
            .collect::<Vec<_>>()
            .join("\n");
        push_cue(&mut cues, *number, start, end, text, Vec::new())?;
    }
    Ok(cues)
}

fn parse_vtt(source: &str) -> Result<Vec<Cue>, SubtitleError> {
    let mut blocks = blocks(source).into_iter();
    let Some(header) = blocks.next() else {
        return refuse(1, "The file is empty");
    };
    let signature = header[0].1;
    if !(signature == "WEBVTT"
        || signature.starts_with("WEBVTT ")
        || signature.starts_with("WEBVTT\t"))
    {
        return refuse(header[0].0, "A WebVTT file starts with WEBVTT");
    }
    // A header block may also hold a first cue when no blank line follows
    // the signature's metadata; WebVTT requires one, so it is refused.
    if header.iter().any(|(_, line)| line.contains("-->")) {
        return refuse(header[0].0, "A blank line must follow the WEBVTT header");
    }
    let mut cues = Vec::new();
    for block in blocks {
        let first = block[0].1;
        let keyword = |word: &str| {
            first
                .strip_prefix(word)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
        };
        // Comments, styles and regions are not captions.
        if !first.contains("-->") && (keyword("NOTE") || keyword("STYLE") || keyword("REGION")) {
            continue;
        }
        let timing_at = usize::from(!first.contains("-->"));
        let Some((number, timing)) = block.get(timing_at) else {
            return refuse(block[0].0, "A caption has an identifier but no times");
        };
        if !timing.contains("-->") {
            return refuse(
                *number,
                "Expected caption times such as 00:01.000 --> 00:04.000",
            );
        }
        let (start, end) =
            timing_line(timing, SubtitleFormat::Vtt).map_err(|message| SubtitleError {
                line: *number,
                message,
            })?;
        let payload: Vec<&str> = block[timing_at + 1..]
            .iter()
            .map(|(_, line)| *line)
            .collect();
        let (text, words) = vtt_payload(&payload.join("\n"), start, end);
        push_cue(&mut cues, *number, start, end, text, words)?;
    }
    Ok(cues)
}

fn push_cue(
    cues: &mut Vec<Cue>,
    line: usize,
    start: Time,
    end: Time,
    text: String,
    words: Vec<WordTime>,
) -> Result<(), SubtitleError> {
    if cues.len() >= MAX_CUES {
        return refuse(line, "The file has more than 20,000 captions");
    }
    if text.len() > MAX_CUE_TEXT {
        return refuse(line, "A caption is longer than 4,096 bytes");
    }
    // A cue with no text shows nothing and is not kept.
    if text.trim().is_empty() {
        return Ok(());
    }
    cues.push(Cue {
        start,
        end,
        text,
        words,
    });
    Ok(())
}

// `start --> end` and, in WebVTT, settings after the end, which are not used.
fn timing_line(line: &str, format: SubtitleFormat) -> Result<(Time, Time), String> {
    let (start, rest) = line
        .split_once("-->")
        .ok_or_else(|| "Expected caption times such as 00:00:01,000 --> 00:00:04,000".to_owned())?;
    let end = rest.split_whitespace().next().unwrap_or("");
    let (start, end) = (timestamp(start.trim(), format), timestamp(end, format));
    match (start, end) {
        (Some(start), Some(end)) if end > start => Ok((start, end)),
        (Some(_), Some(_)) => Err("A caption ends before it starts".into()),
        _ => Err(format!(
            "Times must look like {}",
            match format {
                SubtitleFormat::Srt => "00:00:01,000",
                SubtitleFormat::Vtt => "00:00:01.000 or 00:01.000",
            }
        )),
    }
}

// `hh:mm:ss,mmm` (SubRip, which also takes a full stop) or `[hh:]mm:ss.mmm`
// (WebVTT).
fn timestamp(text: &str, format: SubtitleFormat) -> Option<Time> {
    let (clock, millis) = match format {
        SubtitleFormat::Srt => text.split_once([',', '.'])?,
        SubtitleFormat::Vtt => text.split_once('.')?,
    };
    if millis.len() != 3 || !millis.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parts: Vec<&str> = clock.split(':').collect();
    let (hours, minutes, seconds) = match (format, parts.as_slice()) {
        (_, [h, m, s]) => (*h, *m, *s),
        (SubtitleFormat::Vtt, [m, s]) => ("0", *m, *s),
        _ => return None,
    };
    let number = |part: &str, width: Option<usize>| -> Option<i64> {
        let ok = !part.is_empty()
            && part.len() <= 6
            && part.bytes().all(|b| b.is_ascii_digit())
            && width.is_none_or(|width| part.len() == width);
        ok.then(|| part.parse().ok()).flatten()
    };
    let hours = number(hours, None)?;
    let minutes = number(minutes, Some(2)).filter(|m| *m < 60)?;
    let seconds = number(seconds, Some(2)).filter(|s| *s < 60)?;
    let millis: i64 = millis.parse().ok()?;
    let total = ((hours * 60 + minutes) * 60 + seconds) * 1000 + millis;
    Some(Time(total * 1000))
}

// Drops SubRip's HTML-like and override tags; captions keep plain text.
fn strip_markup(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' if line.contains('>') => {
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                }
            }
            '{' if chars.peek() == Some(&'\\') => {
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", "\u{a0}")
        .replace("&lrm;", "\u{200e}")
        .replace("&rlm;", "\u{200f}")
        .replace("&amp;", "&")
}

// A WebVTT cue's text without tags, and its word times when inline
// timestamps (`<00:00:01.500>`) time each part of it.
fn vtt_payload(payload: &str, start: Time, end: Time) -> (String, Vec<WordTime>) {
    // Each part is the text after a timestamp, with the time it starts.
    let mut parts: Vec<(Time, String)> = vec![(start, String::new())];
    let mut rest = payload;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>').map(|close| open + close) else {
            break;
        };
        if let Some(part) = parts.last_mut() {
            part.1.push_str(&rest[..open]);
        }
        let tag = &rest[open + 1..close];
        if let Some(at) = timestamp(tag, SubtitleFormat::Vtt) {
            parts.push((at.clamp(start, end), String::new()));
        }
        rest = &rest[close + 1..];
    }
    if let Some(part) = parts.last_mut() {
        part.1.push_str(rest);
    }
    let parts: Vec<(Time, String)> = parts
        .into_iter()
        .map(|(at, text)| (at, unescape(&text)))
        .collect();
    let text: String = parts.iter().map(|(_, text)| text.as_str()).collect();
    if parts.len() < 2 {
        return (text, Vec::new());
    }
    // The words of each part share the time until the next part starts.
    let mut words = Vec::new();
    for (index, (at, part)) in parts.iter().enumerate() {
        let until = parts.get(index + 1).map_or(end, |next| next.0).max(*at);
        words.extend(title::spread(part, *at, until));
    }
    let words = if title::word_count(&text) == words.len()
        && words.windows(2).all(|pair| pair[0].at <= pair[1].at)
    {
        words
    } else {
        Vec::new()
    };
    (text, words)
}

/// Puts cues in time order and settles overlaps: cues that start together
/// become one, their texts on separate lines, and a cue that runs into the
/// next one ends where it starts.
pub fn settle(cues: &mut Vec<Cue>) {
    cues.sort_by_key(|cue| cue.start);
    let mut settled: Vec<Cue> = Vec::with_capacity(cues.len());
    for cue in cues.drain(..) {
        match settled.last_mut() {
            Some(last) if last.start == cue.start => {
                last.text.push('\n');
                last.text.push_str(&cue.text);
                last.end = last.end.max(cue.end);
                last.words.clear();
            }
            _ => settled.push(cue),
        }
    }
    for index in 1..settled.len() {
        let next = settled[index].start;
        let cue = &mut settled[index - 1];
        if cue.end > next {
            cue.end = next;
            for word in &mut cue.words {
                word.at = word.at.min(next);
                word.end = word.end.min(next);
            }
        }
    }
    *cues = settled;
}

/// A SubRip file of `cues`.
pub fn format_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (index, cue) in cues.iter().enumerate() {
        let _ = writeln!(
            out,
            "{}\n{} --> {}",
            index + 1,
            clock(cue.start, ','),
            clock(cue.end, ',')
        );
        for line in cue.text.lines().filter(|line| !line.trim().is_empty()) {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// A WebVTT file of `cues`, its words timed inline where cues have times.
pub fn format_vtt(cues: &[Cue], language: Option<&str>) -> String {
    let mut out = String::from("WEBVTT\n");
    if let Some(language) = language {
        let _ = writeln!(out, "Language: {language}");
    }
    out.push('\n');
    for cue in cues {
        let _ = writeln!(out, "{} --> {}", clock(cue.start, '.'), clock(cue.end, '.'));
        // A blank line ends a cue, so its text keeps none.
        let text = cue
            .text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let ranges = title::word_ranges(&text);
        let timed = cue.words.len() == ranges.len() && !ranges.is_empty();
        let mut from = 0;
        for (index, range) in ranges.iter().enumerate() {
            out.push_str(&vtt_escape(&text[from..range.start]));
            // The first word starts with the cue.
            if timed && index > 0 {
                let _ = write!(out, "<{}>", clock(cue.words[index].at, '.'));
            }
            out.push_str(&vtt_escape(&text[range.clone()]));
            from = range.end;
        }
        out.push_str(&vtt_escape(&text[from..]));
        out.push_str("\n\n");
    }
    out
}

fn vtt_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// `hh:mm:ss<separator>mmm`, rounded to the millisecond.
fn clock(time: Time, separator: char) -> String {
    let millis = (time.0.max(0) + 500) / 1000;
    let (hours, rest) = (millis / 3_600_000, millis % 3_600_000);
    let (minutes, rest) = (rest / 60_000, rest % 60_000);
    let (seconds, millis) = (rest / 1000, rest % 1000);
    format!("{hours:02}:{minutes:02}:{seconds:02}{separator}{millis:03}")
}

/// The cues of a caption track: its titles in order, their word times on
/// the timeline. Graphics are not captions.
pub fn track_cues(track: &Track) -> Vec<Cue> {
    track
        .clips
        .iter()
        .filter_map(|clip| match &clip.content {
            ClipContent::Text { text, words, .. } => Some(Cue {
                start: clip.start,
                end: clip.end(),
                text: text.clone(),
                words: words
                    .iter()
                    .map(|word| WordTime {
                        at: clip.start + word.at,
                        end: clip.start + word.end,
                    })
                    .collect(),
            }),
            _ => None,
        })
        .collect()
}

/// A title showing `cue`, made from `look`: a title whose size, colour,
/// style, animations and transform it takes. Its animations are held inside
/// the cue; `None` when the cue is shorter than one frame.
pub fn cue_clip(cue: &Cue, look: &Clip, frame: Time) -> Option<Clip> {
    let duration = cue.end - cue.start;
    if duration < frame.max(Time(1)) {
        return None;
    }
    let mut clip = look.clone();
    clip.id = Uuid::new_v4();
    clip.linked_id = None;
    clip.group_id = None;
    clip.start = cue.start;
    clip.duration = duration;
    clip.source_in = Time::ZERO;
    clip.name = cue.text.clone();
    let ClipContent::Text {
        text,
        enter,
        exit,
        words,
        ..
    } = &mut clip.content
    else {
        return None;
    };
    text.clone_from(&cue.text);
    (*enter, *exit) = title::fit_animations(*enter, *exit, frame, duration);
    *words = if cue.words.len() == title::word_count(&cue.text) {
        cue.words
            .iter()
            .map(|word| {
                let at = (word.at - cue.start).clamp(Time::ZERO, duration);
                WordTime {
                    at,
                    end: (word.end - cue.start).clamp(at, duration),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    for track in &mut clip.animation {
        track.keys.retain(|key| key.at <= duration);
    }
    Some(clip)
}

/// One recognized word with its times on the timeline. Leading whitespace
/// in `text` separates it from the word before; without it the two are
/// one written word, as a recognizer's word pieces are.
#[derive(Debug, Clone, PartialEq)]
pub struct SpokenWord {
    pub text: String,
    pub at: Time,
    pub end: Time,
}

/// How spoken words are grouped into cues.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chunking {
    /// The most characters one cue holds.
    pub max_chars: usize,
    /// The most words one cue holds.
    pub max_words: usize,
    /// The longest a cue lasts.
    pub max_duration: Time,
    /// A silence at least this long starts a new cue.
    pub pause: Time,
    /// How long a cue stays after its last word, unless the next one starts.
    pub linger: Time,
    /// Whether the end of a sentence ends a cue.
    pub sentences: bool,
}

impl Chunking {
    /// Two lines of subtitles, as broadcast captions have.
    pub const LINES: Self = Self {
        max_chars: 84,
        max_words: 20,
        max_duration: Time(6_000_000),
        pause: Time(700_000),
        linger: Time(400_000),
        sentences: true,
    };
    /// A few words at a time, as short vertical videos show them.
    pub const PHRASES: Self = Self {
        max_chars: 24,
        max_words: 4,
        max_duration: Time(2_500_000),
        pause: Time(400_000),
        linger: Time(150_000),
        sentences: true,
    };
    /// One word at a time.
    pub const WORDS: Self = Self {
        max_chars: 32,
        max_words: 1,
        max_duration: Time(2_000_000),
        pause: Time(300_000),
        linger: Time(100_000),
        sentences: true,
    };
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', ')', '”', '’', '»'])
        .ends_with(['.', '?', '!', '…', '。', '？', '！'])
}

// Whether a written word ends between `before` and `next`.
fn boundary(before: &str, next: &str) -> bool {
    next.starts_with(char::is_whitespace)
        || before.ends_with(char::is_whitespace)
        || before.chars().next_back().is_some_and(title::stands_alone)
        || next.chars().next().is_some_and(title::stands_alone)
}

/// Spoken words grouped into cues by `chunking`. Each cue's words are the
/// title words of its text, timed from the spoken words that make them.
pub fn cues(words: &[SpokenWord], chunking: &Chunking) -> Vec<Cue> {
    let mut groups: Vec<Range<usize>> = Vec::new();
    let mut start = 0;
    let mut chars = 0;
    let mut count = 0;
    for (index, word) in words.iter().enumerate() {
        let size = word.text.trim().chars().count();
        if index > start {
            let previous = &words[index - 1];
            let breakable = boundary(&previous.text, &word.text);
            let full = chars + 1 + size > chunking.max_chars
                || count + 1 > chunking.max_words.max(1)
                || word.end - words[start].at > chunking.max_duration;
            let paused = word.at - previous.end >= chunking.pause;
            let sentence = chunking.sentences && ends_sentence(&previous.text);
            if breakable && (full || paused || sentence) {
                groups.push(start..index);
                start = index;
                chars = 0;
                count = 0;
            }
        }
        chars += size + usize::from(chars > 0);
        if index == start || boundary(&words[index - 1].text, &word.text) {
            count += 1;
        }
        if groups.len() >= MAX_CUES {
            break;
        }
    }
    if start < words.len() && groups.len() < MAX_CUES {
        groups.push(start..words.len());
    }
    let mut cues: Vec<Cue> = groups
        .into_iter()
        .filter_map(|group| cue_of(&words[group]))
        .collect();
    // A cue stays a little after its last word, never into the next.
    for cue in &mut cues {
        cue.end = cue.end + chunking.linger;
    }
    settle(&mut cues);
    cues
}

// One cue of spoken words: their text joined, and a time for each title
// word of it, from the spoken words it overlaps. A spoken word holding
// several title words shares its time among them by their length.
fn cue_of(words: &[SpokenWord]) -> Option<Cue> {
    let mut text = String::new();
    let mut spans: Vec<(Range<usize>, Time, Time)> = Vec::new();
    for word in words {
        let piece = if text.is_empty() {
            word.text.trim_start()
        } else {
            word.text.as_str()
        };
        let from = text.len();
        text.push_str(piece);
        spans.push((from..text.len(), word.at, word.end.max(word.at)));
    }
    let trimmed = text.trim_end().len();
    text.truncate(trimmed);
    if text.is_empty() || text.len() > MAX_CUE_TEXT {
        return None;
    }
    let ranges = title::word_ranges(&text);
    let mut times: Vec<Option<WordTime>> = vec![None; ranges.len()];
    for (span, at, end) in &spans {
        let inside: Vec<usize> = (0..ranges.len())
            .filter(|&k| ranges[k].start < span.end && span.start < ranges[k].end)
            .collect();
        if inside.is_empty() {
            continue;
        }
        let pieces = if inside.len() == 1 {
            vec![WordTime { at: *at, end: *end }]
        } else {
            let part: String = inside
                .iter()
                .map(|&k| &text[ranges[k].clone()])
                .collect::<Vec<_>>()
                .join(" ");
            title::spread(&part, *at, *end)
        };
        for (&k, piece) in inside.iter().zip(pieces) {
            times[k] = Some(match times[k] {
                Some(time) => WordTime {
                    at: time.at.min(piece.at),
                    end: time.end.max(piece.end),
                },
                None => piece,
            });
        }
    }
    let first = spans.first()?.1;
    let last = spans.iter().map(|span| span.2).max()?;
    let mut words: Vec<WordTime> = times
        .into_iter()
        .map(|time| {
            time.unwrap_or(WordTime {
                at: first,
                end: first,
            })
        })
        .collect();
    let mut floor = first;
    for word in &mut words {
        word.at = word.at.max(floor);
        word.end = word.end.max(word.at);
        floor = word.at;
    }
    Some(Cue {
        start: first,
        end: last.max(first + Time(1)),
        text,
        words,
    })
}
