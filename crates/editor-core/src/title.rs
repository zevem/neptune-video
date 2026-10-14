//! Titles: styled text on the canvas, its enter and exit animation, the
//! word times that make a title a timed caption, and graphics made of named
//! elements.
//!
//! Everything here is durable data and pure evaluation. Lengths in a style
//! are fractions of the font size (em), so a style scales with its title;
//! times are integer microseconds from the clip's start, as keyframes are,
//! so moving a clip touches none of them. Drawing belongs to the text
//! renderer, which reads the progress and word state [`crate::resolve`]
//! works out here.
use crate::Time;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::ops::Range;
use uuid::Uuid;

/// Words a title may time.
pub const MAX_WORDS: usize = 4096;
/// Elements a graphic may hold.
pub const MAX_ELEMENTS: usize = 32;
/// The longest enter or exit animation.
pub const MAX_ANIMATION: Time = Time(10_000_000);
/// The family a plain style draws with: the bundled interface face.
pub const DEFAULT_FAMILY: &str = "Geist";
/// How long the active word takes to grow to its highlight scale.
pub const POP: Time = Time(80_000);

fn finite_in(value: f32, min: f32, max: f32) -> bool {
    value.is_finite() && (min..=max).contains(&value)
}

/// A font by family, weight and slant. It never names a file: the family is
/// looked up in the project's font assets, the bundled faces and the
/// system's, and a family none of them has is drawn in the fallback face.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FontFace {
    pub family: String,
    /// CSS weight, 100 (thin) to 900 (black); 400 is regular, 700 bold.
    pub weight: u16,
    pub italic: bool,
}
impl Default for FontFace {
    fn default() -> Self {
        Self {
            family: DEFAULT_FAMILY.into(),
            weight: 600,
            italic: false,
        }
    }
}
impl FontFace {
    pub fn validate(&self) -> bool {
        let family = self.family.trim();
        !family.is_empty()
            && family.len() == self.family.len()
            && self.family.len() <= 128
            && !self.family.chars().any(char::is_control)
            && (100..=900).contains(&self.weight)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextCase {
    #[default]
    AsTyped,
    Upper,
    Lower,
}
impl TextCase {
    /// `text` as this case draws it. Upper and lower case can change a
    /// word's length but never the count of words.
    pub fn apply(self, text: &str) -> String {
        match self {
            Self::AsTyped => text.to_owned(),
            Self::Upper => text.to_uppercase(),
            Self::Lower => text.to_lowercase(),
        }
    }
}

/// A stroke around every glyph, drawn under the fill.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextOutline {
    pub color: [u8; 4],
    /// Em, 0.005..=0.5.
    pub width: f32,
}

/// A copy of the glyphs and their outline under them, moved and blurred.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextShadow {
    pub color: [u8; 4],
    /// Em, each -2..=2; positive is right and down.
    pub offset: [f32; 2],
    /// Em, 0..=1: the Gaussian's deviation is half of it.
    pub blur: f32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundShape {
    /// One box around the whole text.
    #[default]
    Block,
    /// A box around each line, as broadcast captions have.
    Lines,
}

/// A rounded box behind the text.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextBackground {
    pub color: [u8; 4],
    /// Em, each 0..=4: room left and right, then above and below.
    pub padding: [f32; 2],
    /// Em, 0..=4.
    pub radius: f32,
    #[serde(default)]
    pub shape: BackgroundShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HighlightMode {
    /// The active word takes the colour.
    Word,
    /// A rounded box of the colour sits behind the active word.
    Box,
    /// Every spoken word takes the colour, the active one from left to right.
    Fill,
}

/// How a timed title shows the word being said. It has effect only on a
/// title that has word times.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Highlight {
    pub mode: HighlightMode,
    pub color: [u8; 4],
    /// The active word's size, 1..=2 times, grown to over [`POP`].
    pub scale: f32,
    /// The opacity of words not yet said, 0..=1; 0 shows them as they come.
    pub upcoming: f32,
}

/// How a title's text looks. A plain style is how a title was always drawn:
/// Geist semibold, centred, with nothing around it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TextStyle {
    pub font: FontFace,
    pub align: TextAlign,
    /// Times the font size from one line's baseline to the next, 0.5..=4.
    pub line_height: f32,
    /// Em added between letters, -0.5..=2.
    pub tracking: f32,
    /// The widest a line may be, in canvas pixels; 0 breaks lines only at
    /// line breaks.
    pub wrap: f32,
    pub case: TextCase,
    pub outline: Option<TextOutline>,
    pub shadow: Option<TextShadow>,
    pub background: Option<TextBackground>,
    pub highlight: Option<Highlight>,
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: FontFace::default(),
            align: TextAlign::Center,
            line_height: 1.2,
            tracking: 0.0,
            wrap: 0.0,
            case: TextCase::AsTyped,
            outline: None,
            shadow: None,
            background: None,
            highlight: None,
        }
    }
}
impl TextStyle {
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    pub fn validate(&self) -> bool {
        self.font.validate()
            && finite_in(self.line_height, 0.5, 4.0)
            && finite_in(self.tracking, -0.5, 2.0)
            && finite_in(self.wrap, 0.0, 16384.0)
            && self
                .outline
                .is_none_or(|outline| finite_in(outline.width, 0.005, 0.5))
            && self.shadow.is_none_or(|shadow| {
                shadow.offset.iter().all(|v| finite_in(*v, -2.0, 2.0))
                    && finite_in(shadow.blur, 0.0, 1.0)
            })
            && self.background.is_none_or(|background| {
                background.padding.iter().all(|v| finite_in(*v, 0.0, 4.0))
                    && finite_in(background.radius, 0.0, 4.0)
            })
            && self.highlight.is_none_or(|highlight| {
                finite_in(highlight.scale, 1.0, 2.0) && finite_in(highlight.upcoming, 0.0, 1.0)
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAnimationKind {
    Fade,
    /// Grows from nothing past its size and settles.
    Pop,
    /// Drops in from above and bounces twice.
    Bounce,
    /// Appears unit by unit, with no motion.
    Typewriter,
    SlideUp,
    SlideDown,
    SlideLeft,
    SlideRight,
    /// Revealed from left to right.
    Wipe,
}
impl TextAnimationKind {
    pub const ALL: [Self; 9] = [
        Self::Fade,
        Self::Pop,
        Self::Bounce,
        Self::Typewriter,
        Self::SlideUp,
        Self::SlideDown,
        Self::SlideLeft,
        Self::SlideRight,
        Self::Wipe,
    ];

    /// The unit the kind is usually given: a typewriter types letters, a
    /// pop and a bounce go word by word, the rest move the whole text.
    pub fn usual_unit(self) -> TextUnit {
        match self {
            Self::Typewriter => TextUnit::Letter,
            Self::Pop | Self::Bounce => TextUnit::Word,
            _ => TextUnit::Whole,
        }
    }
}

/// What an animation moves one at a time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextUnit {
    #[default]
    Whole,
    Line,
    Word,
    Letter,
}

/// A title's entrance or exit: one kind over its units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextAnimation {
    pub kind: TextAnimationKind,
    #[serde(default)]
    pub unit: TextUnit,
    /// One canvas frame to ten seconds, for all units together.
    pub duration: Time,
    /// From the clip's start to the entrance, or from the exit's end to the
    /// clip's end.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub delay: Time,
}

fn is_zero(time: &Time) -> bool {
    *time == Time::ZERO
}

impl TextAnimation {
    /// The time it takes from the edge of its clip.
    pub fn span(&self) -> Time {
        self.delay + self.duration
    }

    /// `frame` is one canvas frame.
    pub fn validate(&self, frame: Time, clip_duration: Time) -> bool {
        self.duration >= frame
            && self.duration > Time::ZERO
            && self.duration <= MAX_ANIMATION
            && self.delay >= Time::ZERO
            && self.delay.is_valid()
            && self.span() <= clip_duration
    }

    /// This animation held to a clip of `clip_duration`: shortened, then
    /// its delay, so that it fits; `None` when not even one frame does.
    pub fn held(mut self, frame: Time, clip_duration: Time) -> Option<Self> {
        if clip_duration < frame.max(Time(1)) {
            return None;
        }
        self.duration = self.duration.min(clip_duration).max(frame);
        self.delay = self.delay.min(clip_duration - self.duration);
        Some(self)
    }

    /// The same animation on a clip `factor` times as long.
    pub fn scaled(mut self, factor: f64, frame: Time) -> Self {
        self.duration = self.duration.scaled(factor).max(frame).min(MAX_ANIMATION);
        self.delay = self.delay.scaled(factor);
        self
    }
}

/// Whether an entrance and an exit fit one clip without overlapping.
pub fn animations_fit(
    enter: Option<&TextAnimation>,
    exit: Option<&TextAnimation>,
    clip_duration: Time,
) -> bool {
    let (Some(enter), Some(exit)) = (enter, exit) else {
        return true;
    };
    enter.span() + exit.span() <= clip_duration
}

/// The enter and exit of a clip that has been given a new duration, made
/// to fit it: each held inside it, and the exit dropped if they would meet.
pub fn fit_animations(
    enter: Option<TextAnimation>,
    exit: Option<TextAnimation>,
    frame: Time,
    clip_duration: Time,
) -> (Option<TextAnimation>, Option<TextAnimation>) {
    let enter = enter.and_then(|animation| animation.held(frame, clip_duration));
    let exit = exit.and_then(|animation| animation.held(frame, clip_duration));
    if animations_fit(enter.as_ref(), exit.as_ref(), clip_duration) {
        (enter, exit)
    } else {
        // Each takes half the clip; one that cannot is let go.
        let half = Time(clip_duration.0 / 2);
        let enter = enter.and_then(|animation| animation.held(frame, half));
        let exit = exit.and_then(|animation| animation.held(frame, clip_duration - half));
        match (enter, exit) {
            (Some(enter), Some(exit))
                if animations_fit(Some(&enter), Some(&exit), clip_duration) =>
            {
                (Some(enter), Some(exit))
            }
            (enter, _) => (enter, None),
        }
    }
}

/// When one word of a timed title is said, from the clip's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WordTime {
    pub at: Time,
    pub end: Time,
}

/// Whether `words` time the words of `text` in a clip of `clip_duration`.
pub fn words_valid(words: &[WordTime], text: &str, clip_duration: Time) -> bool {
    if words.is_empty() {
        return true;
    }
    words.len() <= MAX_WORDS
        && word_count(text) == words.len()
        && words
            .iter()
            .all(|word| word.at >= Time::ZERO && word.at <= word.end && word.end <= clip_duration)
        && words.windows(2).all(|pair| pair[0].at <= pair[1].at)
}

/// Whether `c` is a word of its own: scripts written without spaces.
pub fn stands_alone(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF       // Hiragana, Katakana
        | 0x31F0..=0x31FF     // Katakana phonetic extensions
        | 0x3400..=0x4DBF     // CJK extension A
        | 0x4E00..=0x9FFF     // CJK unified ideographs
        | 0xF900..=0xFAFF     // CJK compatibility ideographs
        | 0xFF66..=0xFF9D     // Half-width Katakana
        | 0x20000..=0x2A6DF   // CJK extension B
        | 0x2A700..=0x2EBEF   // CJK extensions C to F
        | 0x30000..=0x3134F) // CJK extension G
}

/// The byte ranges of the words of `text`, the one count every timed title
/// uses: runs without whitespace, cut before each Han, Hiragana or Katakana
/// character and after it unless punctuation follows.
pub fn word_ranges(text: &str) -> Vec<Range<usize>> {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    let mut after_alone = false;
    for (index, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(from) = start.take() {
                words.push(from..index);
            }
            after_alone = false;
            continue;
        }
        let alone = stands_alone(c);
        let cut = alone || (after_alone && c.is_alphanumeric());
        if cut && let Some(from) = start.take() {
            words.push(from..index);
        }
        if start.is_none() {
            start = Some(index);
        }
        after_alone = alone || (after_alone && !c.is_alphanumeric());
    }
    if let Some(from) = start {
        words.push(from..text.len());
    }
    words
}

pub fn word_count(text: &str) -> usize {
    word_ranges(text).len()
}

/// The words of `text`, as compared when it is edited.
fn words_of(text: &str) -> Vec<String> {
    word_ranges(text)
        .into_iter()
        .map(|range| text[range].to_lowercase())
        .collect()
}

/// Word times for `new` text when a title timed for `old` text is edited:
/// the words both texts have in order keep their times, and the others are
/// spread evenly over the time between their neighbours. Empty when the old
/// text had no times or the new one has no words.
pub fn realign(old: &str, words: &[WordTime], new: &str, clip_duration: Time) -> Vec<WordTime> {
    let before = words_of(old);
    let after = words_of(new);
    if words.is_empty() || words.len() != before.len() || after.is_empty() {
        return Vec::new();
    }
    if after.len() > MAX_WORDS {
        return Vec::new();
    }
    // Words kept at both ends cost nothing to match; what lies between is
    // matched by longest common subsequence while its table stays small,
    // and spread out like new words otherwise.
    let (n, m) = (before.len(), after.len());
    let prefix = (0..n.min(m)).take_while(|&k| before[k] == after[k]).count();
    let suffix = (0..(n - prefix).min(m - prefix))
        .take_while(|&k| before[n - 1 - k] == after[m - 1 - k])
        .count();
    // The old word each new word kept, by index.
    let mut kept: Vec<Option<usize>> = vec![None; m];
    for (k, slot) in kept.iter_mut().enumerate().take(prefix) {
        *slot = Some(k);
    }
    for k in 0..suffix {
        kept[m - 1 - k] = Some(n - 1 - k);
    }
    let (old_mid, new_mid) = (prefix..n - suffix, prefix..m - suffix);
    let (rows, columns) = (old_mid.len(), new_mid.len());
    if rows > 0 && columns > 0 && rows * columns <= 250_000 {
        let width = columns + 1;
        let mut table = vec![0_u16; (rows + 1) * width];
        for i in (0..rows).rev() {
            for j in (0..columns).rev() {
                table[i * width + j] = if before[old_mid.start + i] == after[new_mid.start + j] {
                    table[(i + 1) * width + j + 1] + 1
                } else {
                    table[(i + 1) * width + j].max(table[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < rows && j < columns {
            if before[old_mid.start + i] == after[new_mid.start + j] {
                kept[new_mid.start + j] = Some(old_mid.start + i);
                i += 1;
                j += 1;
            } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    // A run of new words in place of as many old ones (words fixed or
    // reworded one for one) takes their times. Any other run between two
    // kept words (or the clip's ends) shares the time from the end of the
    // one before to the start of the next; when nothing matched, the old
    // words' span is shared out.
    let first = words.first().map_or(Time::ZERO, |word| word.at);
    let last = words.last().map_or(clip_duration, |word| word.end);
    let mut out = Vec::with_capacity(m);
    let mut index = 0;
    let mut old_next = 0;
    while index < m {
        if let Some(old) = kept[index] {
            out.push(words[old]);
            old_next = old + 1;
            index += 1;
            continue;
        }
        let run_end = (index..m).find(|&k| kept[k].is_some()).unwrap_or(m);
        let old_end = kept.get(run_end).copied().flatten().unwrap_or(n);
        if old_end.saturating_sub(old_next) == run_end - index {
            out.extend_from_slice(&words[old_next..old_end]);
            index = run_end;
            continue;
        }
        let from = out.last().map_or(first, |word: &WordTime| word.end);
        let to = kept
            .get(run_end)
            .copied()
            .flatten()
            .map_or(last, |old| words[old].at);
        let to = to.max(from);
        let count = (run_end - index) as i64;
        let step = (to.0 - from.0) / count;
        for k in 0..count {
            let at = Time(from.0 + step * k);
            let end = if k + 1 == count {
                to
            } else {
                Time(from.0 + step * (k + 1))
            };
            out.push(WordTime { at, end });
        }
        index = run_end;
    }
    // Words recognised as overlapping can leave a run starting after the
    // word it leads to: keep every start in order and every word inside.
    let mut floor = Time::ZERO;
    for word in &mut out {
        word.at = word.at.max(floor).min(clip_duration);
        word.end = word.end.max(word.at).min(clip_duration);
        floor = word.at;
    }
    out
}

/// Word times for `text` spread over `[from, to)` by the length of each
/// word: an estimate for text no one has timed, such as a translation.
pub fn spread(text: &str, from: Time, to: Time) -> Vec<WordTime> {
    let ranges = word_ranges(text);
    if ranges.is_empty() || ranges.len() > MAX_WORDS || to <= from {
        return Vec::new();
    }
    let weights: Vec<i64> = ranges
        .iter()
        .map(|range| text[range.clone()].chars().count().max(1) as i64)
        .collect();
    let total: i64 = weights.iter().sum();
    let span = to.0 - from.0;
    let mut out = Vec::with_capacity(ranges.len());
    let mut sum = 0;
    for weight in weights {
        let at = Time(from.0 + (i128::from(span) * i128::from(sum) / i128::from(total)) as i64);
        sum += weight;
        let end = Time(from.0 + (i128::from(span) * i128::from(sum) / i128::from(total)) as i64);
        out.push(WordTime { at, end });
    }
    out
}

/// One of a clip's animations at one time: how far the units have come in
/// (1 shown, 0 not), and whether it is the exit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub kind: TextAnimationKind,
    pub unit: TextUnit,
    pub progress: f32,
    pub leaving: bool,
}

/// The entrance at clip time `local`, while it is under way. Before its
/// delay it has not begun (progress 0); once done it is `None`, so a title
/// at rest resolves to the same layer frame after frame.
pub fn entering(animation: Option<&TextAnimation>, local: Time) -> Option<Motion> {
    let animation = animation?;
    if local >= animation.span() {
        return None;
    }
    let progress = (local - animation.delay).0 as f64 / animation.duration.0.max(1) as f64;
    Some(Motion {
        kind: animation.kind,
        unit: animation.unit,
        progress: progress.clamp(0.0, 1.0) as f32,
        leaving: false,
    })
}

/// The exit at clip time `local` of a clip of `clip_duration`, once begun.
pub fn leaving(
    animation: Option<&TextAnimation>,
    local: Time,
    clip_duration: Time,
) -> Option<Motion> {
    let animation = animation?;
    let start = clip_duration - animation.span();
    if local <= start {
        return None;
    }
    let done = (local - start).0 as f64 / animation.duration.0.max(1) as f64;
    Some(Motion {
        kind: animation.kind,
        unit: animation.unit,
        progress: (1.0 - done).clamp(0.0, 1.0) as f32,
        leaving: true,
    })
}

/// How far unit `index` of `count` has come at the motion's progress. Units
/// start evenly spaced and each takes `2 / (count + 1)` of the time, so
/// neighbours overlap by half; a typewriter's unit appears whole at its
/// turn. An exit is its entrance backwards, so the last unit leaves first.
pub fn unit_progress(motion: &Motion, index: usize, count: usize) -> f32 {
    let p = motion.progress.clamp(0.0, 1.0);
    // Every unit is at rest at either end, exactly.
    if p <= 0.0 || p >= 1.0 {
        return p;
    }
    let count = count.max(1);
    let index = index.min(count - 1);
    if motion.kind == TextAnimationKind::Typewriter {
        return if p * count as f32 > index as f32 {
            1.0
        } else {
            0.0
        };
    }
    if count == 1 {
        return p;
    }
    let length = 2.0 / (count as f32 + 1.0);
    let start = index as f32 * (1.0 - length) / (count as f32 - 1.0);
    ((p - start) / length).clamp(0.0, 1.0)
}

/// How one unit is drawn at its own progress: multiplied into its opacity,
/// scaled about its centre, moved (em) and revealed from the left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitLook {
    pub opacity: f32,
    pub scale: f32,
    pub offset: [f32; 2],
    /// The fraction of the unit's width shown, from its left.
    pub reveal: f32,
}

impl UnitLook {
    pub const SHOWN: Self = Self {
        opacity: 1.0,
        scale: 1.0,
        offset: [0.0, 0.0],
        reveal: 1.0,
    };
}

fn ease_out(u: f32) -> f32 {
    1.0 - (1.0 - u).powi(3)
}

fn ease_in_out(u: f32) -> f32 {
    if u < 0.5 {
        4.0 * u * u * u
    } else {
        1.0 - (-2.0 * u + 2.0).powi(3) / 2.0
    }
}

// A back-out curve: past 1 and back.
fn overshoot(u: f32) -> f32 {
    let c = 1.70158_f32;
    1.0 + (c + 1.0) * (u - 1.0).powi(3) + c * (u - 1.0).powi(2)
}

// The bounce of a ball dropped onto the line: 0 at the top, 1 at rest.
fn bounce(u: f32) -> f32 {
    let (n, d) = (7.5625_f32, 2.75_f32);
    if u < 1.0 / d {
        n * u * u
    } else if u < 2.0 / d {
        let u = u - 1.5 / d;
        n * u * u + 0.75
    } else if u < 2.5 / d {
        let u = u - 2.25 / d;
        n * u * u + 0.9375
    } else {
        let u = u - 2.625 / d;
        n * u * u + 0.984375
    }
}

/// The look of a unit of `kind` at its progress `u`. A slide travels one em
/// and fades; leaving, it goes on in the direction it is named for.
pub fn unit_look(kind: TextAnimationKind, u: f32, leaving: bool) -> UnitLook {
    let u = u.clamp(0.0, 1.0);
    let mut look = UnitLook::SHOWN;
    let slide = |dx: f32, dy: f32| {
        // Entering it comes from behind the direction; leaving it goes on.
        let travel = 1.0 - ease_out(u);
        let sign = if leaving { 1.0 } else { -1.0 };
        UnitLook {
            opacity: ease_out(u),
            offset: [dx * travel * sign, dy * travel * sign],
            ..UnitLook::SHOWN
        }
    };
    match kind {
        TextAnimationKind::Fade => look.opacity = ease_in_out(u),
        TextAnimationKind::Pop => {
            look.scale = overshoot(u).max(0.0);
            look.opacity = (u * 4.0).min(1.0);
        }
        TextAnimationKind::Bounce => {
            look.offset[1] = -(1.0 - bounce(u)) * 1.5;
            look.opacity = (u * 5.0).min(1.0);
        }
        TextAnimationKind::Typewriter => look.opacity = if u > 0.0 { 1.0 } else { 0.0 },
        TextAnimationKind::SlideUp => look = slide(0.0, -1.0),
        TextAnimationKind::SlideDown => look = slide(0.0, 1.0),
        TextAnimationKind::SlideLeft => look = slide(-1.0, 0.0),
        TextAnimationKind::SlideRight => look = slide(1.0, 0.0),
        TextAnimationKind::Wipe => look.reveal = ease_in_out(u),
    }
    look
}

/// The words of a timed title at one time, as its highlight draws them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WordState {
    /// The word being said, or the last one said; none before the first.
    pub active: Option<usize>,
    /// Words whose time is over.
    pub spoken: usize,
    /// How much of the active word a fill has swept, 0..=1; 1 for the
    /// other modes, so a word at rest resolves the same frame after frame.
    pub sweep: f32,
    /// How far the active word has grown to its highlight scale, 0..=1.
    pub grown: f32,
}

/// The word state at clip time `local`, for a title with a highlight.
pub fn word_state(words: &[WordTime], highlight: &Highlight, local: Time) -> WordState {
    let reached = words.partition_point(|word| word.at <= local);
    let spoken = words.iter().filter(|word| word.end <= local).count();
    let Some(index) = reached.checked_sub(1) else {
        return WordState {
            active: None,
            spoken,
            sweep: 0.0,
            grown: 0.0,
        };
    };
    let word = words[index];
    let since = (local - word.at).0 as f64;
    let sweep = if highlight.mode == HighlightMode::Fill {
        let length = (word.end - word.at).0.max(1) as f64;
        (since / length).clamp(0.0, 1.0) as f32
    } else {
        1.0
    };
    let grown = if highlight.scale > 1.0 {
        (since / POP.0 as f64).clamp(0.0, 1.0) as f32
    } else {
        1.0
    };
    WordState {
        active: Some(index),
        spoken,
        sweep,
        grown,
    }
}

/// A stroke around a shape, in design pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShapeStroke {
    pub color: [u8; 4],
    /// 0.5..=1000 design pixels.
    pub width: f32,
}

/// What one element of a graphic draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ElementKind {
    /// Text laid out in the element's frame: wrapped at its width, aligned
    /// by its style and centred from top to bottom. The style's own wrap is
    /// not used.
    Text {
        text: String,
        font_size: f32,
        color: [u8; 4],
        #[serde(default, skip_serializing_if = "TextStyle::is_plain")]
        style: TextStyle,
    },
    Rect {
        color: [u8; 4],
        /// Corner radius, design pixels.
        #[serde(default)]
        radius: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<ShapeStroke>,
    },
    Ellipse {
        color: [u8; 4],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<ShapeStroke>,
    },
    /// From the frame's left top to its right bottom; either side may be
    /// negative. An arrow has its head at the end.
    Line {
        color: [u8; 4],
        width: f32,
        #[serde(default)]
        arrow: bool,
    },
}

impl ElementKind {
    /// The colours the element draws with, in order: what a shared colour
    /// parameter changes.
    pub fn colors(&self) -> Vec<[u8; 4]> {
        match self {
            Self::Text { color, style, .. } => {
                let mut colors = vec![*color];
                colors.extend(style.outline.map(|outline| outline.color));
                colors.extend(style.shadow.map(|shadow| shadow.color));
                colors.extend(style.background.map(|background| background.color));
                colors.extend(style.highlight.map(|highlight| highlight.color));
                colors
            }
            Self::Rect { color, stroke, .. } | Self::Ellipse { color, stroke } => {
                let mut colors = vec![*color];
                colors.extend(stroke.map(|stroke| stroke.color));
                colors
            }
            Self::Line { color, .. } => vec![*color],
        }
    }

    /// Every colour equal to `from` made `to`.
    pub fn recolor(&mut self, from: [u8; 4], to: [u8; 4]) {
        let swap = |color: &mut [u8; 4]| {
            if *color == from {
                *color = to;
            }
        };
        match self {
            Self::Text { color, style, .. } => {
                swap(color);
                if let Some(outline) = &mut style.outline {
                    swap(&mut outline.color);
                }
                if let Some(shadow) = &mut style.shadow {
                    swap(&mut shadow.color);
                }
                if let Some(background) = &mut style.background {
                    swap(&mut background.color);
                }
                if let Some(highlight) = &mut style.highlight {
                    swap(&mut highlight.color);
                }
            }
            Self::Rect { color, stroke, .. } | Self::Ellipse { color, stroke } => {
                swap(color);
                if let Some(stroke) = stroke {
                    swap(&mut stroke.color);
                }
            }
            Self::Line { color, .. } => swap(color),
        }
    }
}

/// One named part of a graphic, placed in its design box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Element {
    pub id: Uuid,
    /// What the inspector calls it: "Name", "Title", "Bar".
    pub name: String,
    pub kind: ElementKind,
    /// Left, top, width and height in the design box's pixels, from its
    /// left top.
    pub frame: [f32; 4],
    /// Degrees about the frame's centre.
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enter: Option<TextAnimation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<TextAnimation>,
}

fn one() -> f32 {
    1.0
}

impl Element {
    pub fn new(name: impl Into<String>, kind: ElementKind, frame: [f32; 4]) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            frame,
            rotation: 0.0,
            opacity: 1.0,
            enter: None,
            exit: None,
        }
    }

    fn validate(&self, frame: Time, clip_duration: Time) -> bool {
        let shape_ok = match &self.kind {
            ElementKind::Text {
                text,
                font_size,
                style,
                ..
            } => text.len() <= 65_536 && finite_in(*font_size, 1.0, 4096.0) && style.validate(),
            ElementKind::Rect { radius, stroke, .. } => {
                finite_in(*radius, 0.0, 8192.0) && stroke_ok(stroke)
            }
            ElementKind::Ellipse { stroke, .. } => stroke_ok(stroke),
            ElementKind::Line { width, .. } => finite_in(*width, 0.5, 1000.0),
        };
        let [_, _, width, height] = self.frame;
        let sized = match self.kind {
            ElementKind::Line { .. } => width != 0.0 || height != 0.0,
            _ => width > 0.0 && height > 0.0,
        };
        shape_ok
            && sized
            && self.frame.iter().all(|v| finite_in(*v, -1.0e6, 1.0e6))
            && !self.name.trim().is_empty()
            && self.name.len() <= 64
            && finite_in(self.rotation, -36_000.0, 36_000.0)
            && finite_in(self.opacity, 0.0, 1.0)
            && [&self.enter, &self.exit]
                .into_iter()
                .flatten()
                .all(|animation| animation.validate(frame, clip_duration))
            && animations_fit(self.enter.as_ref(), self.exit.as_ref(), clip_duration)
    }
}

fn stroke_ok(stroke: &Option<ShapeStroke>) -> bool {
    stroke.is_none_or(|stroke| finite_in(stroke.width, 0.5, 1000.0))
}

/// Whether a graphic's design box and elements are within bounds for a
/// clip of `clip_duration`.
pub fn graphic_valid(
    size: [f32; 2],
    elements: &[Element],
    frame: Time,
    clip_duration: Time,
) -> bool {
    let mut ids = std::collections::HashSet::with_capacity(elements.len());
    size.iter().all(|side| finite_in(*side, 1.0, 16384.0))
        && (1..=MAX_ELEMENTS).contains(&elements.len())
        && elements
            .iter()
            .all(|element| ids.insert(element.id) && element.validate(frame, clip_duration))
}

/// The colours a graphic's elements share, in the order first used: its
/// colour parameters.
pub fn graphic_colors(elements: &[Element]) -> Vec<[u8; 4]> {
    let mut colors: Vec<[u8; 4]> = Vec::new();
    for color in elements.iter().flat_map(|element| element.kind.colors()) {
        if !colors.contains(&color) {
            colors.push(color);
        }
    }
    colors
}

/// Whether a language tag is one a caption track may carry: 1 to 35 ASCII
/// letters, digits and hyphens, starting with a letter. It is not checked
/// against the registry.
pub fn language_valid(tag: &str) -> bool {
    (1..=35).contains(&tag.len())
        && tag.starts_with(|c: char| c.is_ascii_alphabetic())
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A graphic's element at one time, as the renderer draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementLayer {
    pub element: Element,
    pub enter: Option<Motion>,
    pub exit: Option<Motion>,
}
