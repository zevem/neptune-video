//! Text shaped and set in lines: each glyph placed, and the letter, word
//! and line it belongs to, so animation and karaoke can move them apart.
use super::fonts::{Chosen, FontBook, clings, is_emoji};
use editor_core::{
    FontFace,
    title::{TextAlign, stands_alone, word_ranges},
};
use harfrust::{Direction, ShapeOptions, ShaperInstance, UnicodeBuffer};
use std::ops::Range;
use vello_cpu::kurbo::Rect;

/// One glyph where it is drawn, in layout units from the text's left top:
/// `x` and `y` are its pen position on the baseline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    /// Index into [`Laid::faces`].
    pub face: usize,
    pub line: usize,
    /// None for whitespace, which has no ink.
    pub word: Option<usize>,
    pub letter: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct Line {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub baseline: f32,
}

/// Text as it is set.
pub(crate) struct Laid {
    pub faces: Vec<Chosen>,
    pub glyphs: Vec<Placed>,
    pub lines: Vec<Line>,
    /// Each word's box: its glyphs across, its line's ink from top to
    /// bottom.
    pub words: Vec<Rect>,
    pub letters: Vec<Rect>,
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
    /// Above and below the baseline, layout units.
    pub ascent: f32,
    pub descent: f32,
}

impl Laid {
    pub fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height))
    }

    /// A line's box from its ascent to its descent.
    pub fn line_ink(&self, line: &Line) -> Rect {
        Rect::new(
            f64::from(line.left),
            f64::from(line.baseline - self.ascent),
            f64::from(line.left + line.width),
            f64::from(line.baseline + self.descent),
        )
    }

    /// The box a line's units scale and wipe about.
    pub fn line_box(&self, line: &Line) -> Rect {
        Rect::new(
            f64::from(line.left),
            f64::from(line.top),
            f64::from(line.left + line.width),
            f64::from(line.top + self.line_height),
        )
    }
}

/// How text is set.
pub(crate) struct Setting<'a> {
    pub font: &'a FontFace,
    pub size: f32,
    pub align: TextAlign,
    /// Times the size between baselines.
    pub line_height: f32,
    /// Em between letters.
    pub tracking: f32,
    /// The widest a line may be, layout units; 0 breaks only at line breaks.
    pub wrap: f32,
}

/// One shaped glyph before it is placed.
struct Shaped {
    id: u32,
    cluster: usize,
    advance: f32,
    dx: f32,
    dy: f32,
    face: usize,
}

/// Sets `text` (as drawn: cased, with `\n` for breaks).
pub(crate) fn lay_out(book: &mut FontBook, text: &str, setting: &Setting<'_>) -> Laid {
    let size = setting.size;
    let line_height = setting.line_height * size;
    let mut laid = Laid {
        faces: Vec::new(),
        glyphs: Vec::new(),
        lines: Vec::new(),
        words: Vec::new(),
        letters: Vec::new(),
        width: 0.0,
        height: 0.0,
        line_height,
        ascent: size * 0.8,
        descent: size * 0.2,
    };
    let Some(primary) = book.select(setting.font) else {
        return laid;
    };
    laid.ascent = primary.face.ascent / primary.face.upem * size;
    laid.descent = -primary.face.descent / primary.face.upem * size;
    laid.faces.push(primary);
    let words = word_ranges(text);
    // The letters: each character that is not whitespace and does not
    // cling to the one before.
    let letter_starts: Vec<usize> = text
        .char_indices()
        .filter(|(_, c)| !c.is_whitespace() && !clings(*c))
        .map(|(index, _)| index)
        .collect();
    let content = laid.ascent + laid.descent;
    let mut from = 0;
    for paragraph in text.split('\n') {
        let start = from;
        from += paragraph.len() + 1;
        let shaped = shape(book, &mut laid.faces, setting.font, paragraph, size);
        // Each cluster's advance, at its first byte, with tracking.
        let mut advance = vec![0.0_f32; paragraph.len() + 1];
        let mut starts = vec![false; paragraph.len() + 1];
        for glyph in &shaped.glyphs {
            advance[glyph.cluster] += glyph.advance;
            starts[glyph.cluster] = true;
        }
        let tracking = setting.tracking * size;
        if tracking != 0.0 {
            for (index, start) in starts.iter().enumerate() {
                if *start {
                    advance[index] += tracking;
                }
            }
        }
        for range in wrap(paragraph, &advance, &starts, setting.wrap) {
            let index = laid.lines.len();
            let top = index as f32 * line_height;
            let baseline = top + (line_height - content) / 2.0 + laid.ascent;
            // Trailing whitespace takes no room.
            let ink_end = paragraph[range.clone()]
                .trim_end()
                .len()
                .saturating_add(range.start);
            let width: f32 = advance[range.start..ink_end].iter().sum::<f32>()
                - if ink_end > range.start { tracking } else { 0.0 };
            let mut pen = 0.0;
            let mut runs: Vec<&Run> = shaped.runs.iter().collect();
            if shaped.backward {
                runs.reverse();
            }
            for run in runs {
                let mut last_cluster = None;
                for glyph in &shaped.glyphs[run.glyphs.clone()] {
                    if !range.contains(&glyph.cluster) {
                        continue;
                    }
                    if last_cluster.is_some_and(|last| last != glyph.cluster) {
                        pen += tracking;
                    }
                    last_cluster = Some(glyph.cluster);
                    let cluster = start + glyph.cluster;
                    let c = text[cluster..].chars().next().unwrap_or(' ');
                    let inked = !c.is_whitespace();
                    let word = inked
                        .then(|| words.partition_point(|word| word.end <= cluster))
                        .filter(|&word| word < words.len() && words[word].start <= cluster);
                    let letter = inked
                        .then(|| letter_starts.partition_point(|&at| at <= cluster))
                        .and_then(|after| after.checked_sub(1));
                    laid.glyphs.push(Placed {
                        id: glyph.id,
                        x: pen + glyph.dx,
                        y: baseline + glyph.dy,
                        advance: glyph.advance,
                        face: glyph.face,
                        line: index,
                        word,
                        letter,
                    });
                    pen += glyph.advance;
                }
                if last_cluster.is_some() {
                    pen += tracking;
                }
            }
            laid.lines.push(Line {
                left: 0.0,
                top,
                width: width.max(0.0),
                baseline,
            });
        }
    }
    laid.width = laid.lines.iter().map(|line| line.width).fold(0.0, f32::max);
    laid.height = laid.lines.len() as f32 * line_height;
    // Lines aligned in the widest.
    let mut first_glyph = 0;
    for (index, line) in laid.lines.iter_mut().enumerate() {
        line.left = match setting.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (laid.width - line.width) / 2.0,
            TextAlign::Right => laid.width - line.width,
        };
        let count = laid.glyphs[first_glyph..]
            .iter()
            .take_while(|glyph| glyph.line == index)
            .count();
        for glyph in &mut laid.glyphs[first_glyph..first_glyph + count] {
            glyph.x += line.left;
        }
        first_glyph += count;
    }
    laid.words = boxes(&laid, words.len(), |glyph| glyph.word);
    laid.letters = boxes(&laid, letter_starts.len(), |glyph| glyph.letter);
    laid
}

/// The box of each unit `of` names, from its glyphs.
fn boxes(laid: &Laid, count: usize, of: impl Fn(&Placed) -> Option<usize>) -> Vec<Rect> {
    let mut out: Vec<Option<Rect>> = vec![None; count];
    for glyph in &laid.glyphs {
        let Some(unit) = of(glyph).filter(|&unit| unit < count) else {
            continue;
        };
        let line = &laid.lines[glyph.line];
        let rect = Rect::new(
            f64::from(glyph.x),
            f64::from(line.baseline - laid.ascent),
            f64::from(glyph.x + glyph.advance),
            f64::from(line.baseline + laid.descent),
        );
        out[unit] = Some(out[unit].map_or(rect, |known| known.union(rect)));
    }
    out.into_iter()
        .map(|rect| rect.unwrap_or(Rect::ZERO))
        .collect()
}

struct Run {
    glyphs: Range<usize>,
}

struct ShapedParagraph {
    glyphs: Vec<Shaped>,
    runs: Vec<Run>,
    /// Set right to left: its runs go from the right.
    backward: bool,
}

/// The paragraph cut into runs of one face each and shaped.
fn shape(
    book: &mut FontBook,
    faces: &mut Vec<Chosen>,
    font: &FontFace,
    paragraph: &str,
    size: f32,
) -> ShapedParagraph {
    let mut out = ShapedParagraph {
        glyphs: Vec::new(),
        runs: Vec::new(),
        backward: false,
    };
    // Each character's face.
    let mut items: Vec<(Range<usize>, usize)> = Vec::new();
    let chars: Vec<(usize, char)> = paragraph.char_indices().collect();
    for (k, &(index, c)) in chars.iter().enumerate() {
        let end = index + c.len_utf8();
        let previous = items.last().map(|(_, face)| *face);
        let next = chars.get(k + 1).map(|(_, next)| *next);
        let emoji_shown = matches!(next, Some('\u{FE0F}' | '\u{20E3}'))
            || (is_emoji(c) && c as u32 >= 0x1F000 && next != Some('\u{FE0E}'));
        let face = match previous {
            Some(previous) if clings(c) => previous,
            // A space is the title's own, so an emoji's face does not set
            // the gap after it; one the title's face lacks keeps the run's.
            Some(previous) if c.is_whitespace() && !faces[0].face.has(c) => previous,
            // The character after a joiner joins the emoji before it.
            Some(previous) if k > 0 && chars[k - 1].1 == '\u{200D}' => previous,
            _ if c.is_whitespace() || c.is_control() => 0,
            _ => {
                let primary = &faces[0];
                let primary_has = primary.face.has(c);
                let wanted = if emoji_shown && !(primary.face.color && primary_has) {
                    book.fallback(c, true, font)
                } else if primary_has {
                    None
                } else {
                    book.fallback(c, is_emoji(c), font)
                };
                match wanted {
                    Some(chosen) => match faces.iter().position(|known| known.same(&chosen)) {
                        Some(known) => known,
                        None => {
                            faces.push(chosen);
                            faces.len() - 1
                        }
                    },
                    // Nothing has it: the title's face draws its box.
                    None => 0,
                }
            }
        };
        match items.last_mut() {
            Some((range, last)) if *last == face => range.end = end,
            _ => items.push((index..end, face)),
        }
    }
    let mut first_strong = None;
    for (range, face) in items {
        let chosen = &faces[face];
        let font = chosen.face.font();
        let instance = (!chosen.coords.is_empty())
            .then(|| ShaperInstance::from_coords(&font, chosen.coords.iter().copied()));
        let shaper = chosen
            .face
            .shaper
            .shaper(&font)
            .instance(instance.as_ref())
            .build();
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(&paragraph[range.clone()]);
        buffer.set_pre_context(&paragraph[..range.start]);
        buffer.set_post_context(&paragraph[range.end..]);
        buffer.guess_segment_properties();
        let direction = buffer.direction();
        let scale = size / chosen.face.upem;
        let glyphs = shaper.shape(buffer, ShapeOptions::new());
        if first_strong.is_none() && paragraph[range.clone()].chars().any(char::is_alphabetic) {
            first_strong = Some(direction == Direction::RightToLeft);
        }
        let begin = out.glyphs.len();
        for (info, position) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
            out.glyphs.push(Shaped {
                id: info.glyph_id,
                cluster: range.start + info.cluster as usize,
                advance: position.x_advance as f32 * scale,
                dx: position.x_offset as f32 * scale,
                dy: -position.y_offset as f32 * scale,
                face,
            });
        }
        out.runs.push(Run {
            glyphs: begin..out.glyphs.len(),
        });
    }
    out.backward = first_strong.unwrap_or(false);
    out
}

/// Where lines may start: after whitespace, and around characters that
/// stand alone unless punctuation follows.
fn may_break(paragraph: &str, at: usize) -> bool {
    let (Some(before), Some(c)) = (
        paragraph[..at].chars().next_back(),
        paragraph[at..].chars().next(),
    ) else {
        return false;
    };
    if c.is_whitespace() || clings(c) {
        return false;
    }
    before.is_whitespace()
        || stands_alone(c)
        || (stands_alone(before) && c.is_alphanumeric())
        || (before == '-' && c.is_alphanumeric())
}

/// Lines of the paragraph as byte ranges, filled greedily to `width`.
fn wrap(paragraph: &str, advance: &[f32], starts: &[bool], width: f32) -> Vec<Range<usize>> {
    if width <= 0.0 || paragraph.is_empty() {
        return std::iter::once(0..paragraph.len()).collect();
    }
    let width_of = |range: Range<usize>| -> f32 { advance[range].iter().sum() };
    let inked = |range: Range<usize>| -> f32 {
        let end = range.start + paragraph[range.clone()].trim_end().len();
        width_of(range.start..end)
    };
    let mut cuts: Vec<usize> = (1..paragraph.len())
        .filter(|&at| paragraph.is_char_boundary(at) && may_break(paragraph, at))
        .collect();
    cuts.push(paragraph.len());
    let mut lines = Vec::new();
    let mut line_start = 0;
    let mut used = 0.0;
    let mut piece_start = 0;
    for cut in cuts {
        let piece = piece_start..cut;
        piece_start = cut;
        let ink = inked(piece.clone());
        if piece.start > line_start && used + ink > width {
            lines.push(line_start..piece.start);
            line_start = piece.start;
            used = 0.0;
        }
        if piece.start == line_start && ink > width {
            // A piece wider than a line breaks between clusters.
            let mut at = piece.start;
            let mut run = 0.0;
            while at < piece.end {
                let next = (at + 1..=piece.end)
                    .find(|&next| next == piece.end || starts[next])
                    .unwrap_or(piece.end);
                let step = width_of(at..next);
                if run > 0.0 && run + step > width && !paragraph[at..next].trim().is_empty() {
                    lines.push(line_start..at);
                    line_start = at;
                    run = 0.0;
                }
                run += step;
                at = next;
            }
            used = run;
            continue;
        }
        used += width_of(piece);
    }
    lines.push(line_start..paragraph.len());
    lines
}
