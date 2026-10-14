//! What Whisper heard, put back on the timeline: the stretches of speech it
//! was given end to end, the quiet moment a long sound is cut at, and its
//! text tokens as spoken words with their times.
use editor_core::{Time, captions::SpokenWord};
use std::ops::Range;

/// The stretches of speech of a piece of sound, laid end to end with a
/// short silence between, and where each came from.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Condensed {
    /// The start of each stretch in what is heard, in the piece, and its
    /// length, in samples.
    spans: Vec<(usize, usize, usize)>,
}

impl Condensed {
    /// The samples of `spans` of `samples`, each followed by `gap` samples
    /// of silence so that Whisper hears where speech stopped.
    pub(super) fn build(samples: &[f32], spans: &[Range<usize>], gap: usize) -> (Vec<f32>, Self) {
        let mut heard = Vec::new();
        let mut map = Self::default();
        for span in spans {
            let span = span.start.min(samples.len())..span.end.min(samples.len());
            if span.is_empty() {
                continue;
            }
            map.spans.push((heard.len(), span.start, span.len()));
            heard.extend_from_slice(&samples[span]);
            heard.extend(std::iter::repeat_n(0.0, gap));
        }
        (heard, map)
    }

    /// The sample of the piece that `heard` is. A sample of a silence put
    /// between two stretches is the end of the stretch before it.
    pub(super) fn original(&self, heard: usize) -> usize {
        let index = self
            .spans
            .partition_point(|(start, _, _)| *start <= heard)
            .saturating_sub(1);
        match self.spans.get(index) {
            Some((start, from, length)) => from + heard.saturating_sub(*start).min(*length),
            None => 0,
        }
    }
}

/// Where to cut `samples` before sample `at`: the middle of its quietest
/// `window` in the `search` samples before `at`, so that a word is rarely
/// split between two pieces.
pub(super) fn quiet_cut(samples: &[f32], at: usize, search: usize, window: usize) -> usize {
    let at = at.min(samples.len());
    let from = at.saturating_sub(search);
    if window == 0 || at - from < window {
        return at;
    }
    let energy = |range: Range<usize>| -> f64 {
        samples[range]
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum()
    };
    let step = (window / 2).max(1);
    let mut best = (f64::INFINITY, at);
    let mut start = from;
    while start + window <= at {
        let quiet = energy(start..start + window);
        // The latest of equally quiet windows keeps the piece long.
        if quiet <= best.0 {
            best = (quiet, start + window / 2);
        }
        start += step;
    }
    best.1
}

/// A text token as whisper.cpp gives it: its bytes, which can end inside a
/// character, and its time and its segment's end in hundredths of a second
/// of what was heard.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Piece {
    pub bytes: Vec<u8>,
    pub at: i64,
    pub segment: usize,
    pub segment_end: i64,
}

/// The longest a word is heard for, by its length: Whisper times when a
/// word starts, not when it ends.
fn longest(characters: usize) -> i64 {
    (15 + 8 * characters as i64).min(150)
}

/// Text pieces as spoken words on the timeline. `offset` is where the piece
/// of sound starts and `rate` its sample rate; times are kept in order and
/// within `bounds`. A character split between tokens is joined first.
pub(super) fn spoken(
    pieces: &[Piece],
    map: &Condensed,
    offset: Time,
    rate: u32,
    bounds: Range<Time>,
) -> Vec<SpokenWord> {
    // Whole characters first: a token can end inside one.
    let mut whole: Vec<(String, &Piece)> = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut first: Option<&Piece> = None;
    for piece in pieces {
        if let Some(start) = first
            && start.segment != piece.segment
        {
            // A segment never ends inside a character it can still finish.
            whole.push((String::from_utf8_lossy(&pending).into_owned(), start));
            pending.clear();
            first = None;
        }
        pending.extend_from_slice(&piece.bytes);
        let first_piece = *first.get_or_insert(piece);
        match std::str::from_utf8(&pending) {
            Ok(text) => {
                whole.push((text.to_owned(), first_piece));
                pending.clear();
                first = None;
            }
            // An unfinished character waits for the token that ends it.
            Err(error) if error.error_len().is_none() => {}
            Err(_) => {
                whole.push((String::from_utf8_lossy(&pending).into_owned(), first_piece));
                pending.clear();
                first = None;
            }
        }
    }
    if let Some(first) = first {
        whole.push((String::from_utf8_lossy(&pending).into_owned(), first));
    }
    whole.retain(|(text, _)| !text.is_empty());

    let time = |hundredths: i64| -> Time {
        let heard = (hundredths.max(0) as u64 * u64::from(rate) / 100) as usize;
        let sample = map.original(heard);
        let at = offset + Time::from_seconds(sample as f64 / f64::from(rate));
        at.clamp(bounds.start, bounds.end)
    };
    let mut words = Vec::with_capacity(whole.len());
    let mut last = bounds.start;
    for (index, (text, piece)) in whole.iter().enumerate() {
        let next = whole
            .get(index + 1)
            .filter(|(_, next)| next.segment == piece.segment)
            .map_or(piece.segment_end, |(_, next)| next.at);
        let characters = text.trim().chars().count();
        let end = next.min(piece.at + longest(characters)).max(piece.at);
        let at = time(piece.at).max(last);
        let end = time(end).max(at);
        last = at;
        words.push(SpokenWord {
            text: text.clone(),
            at,
            end,
        });
    }
    words
}

/// Whether a segment is a description of sound rather than speech, such
/// as "[Music]" or "(applause)".
pub(super) fn annotation(text: &str) -> bool {
    let text = text.trim();
    let wrapped = |open: char, close: char| text.starts_with(open) && text.ends_with(close);
    text.is_empty()
        || wrapped('[', ']')
        || wrapped('(', ')')
        || wrapped('*', '*')
        || text
            .chars()
            .all(|c| matches!(c, '♪' | '♫' | ' ' | '.' | '…'))
}
