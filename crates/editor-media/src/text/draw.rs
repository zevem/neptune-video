//! A title's look made into a scene: its background, shadow, outline and
//! fill, each unit moved by its animation and each word lit by its
//! highlight; and a graphic's elements, each placed in its design box.
use super::{
    fonts::FontBook,
    layout::{Laid, Setting, lay_out},
    scene::{Group, Item, Node, Paint, Scene, Shadow, Shape},
};
use editor_core::{
    TextStyle,
    title::{
        self, BackgroundShape, ElementKind, ElementLayer, HighlightMode, Motion, TextAlign,
        TextUnit, UnitLook, WordState,
    },
};
use vello_cpu::{
    Glyph,
    kurbo::{Affine, BezPath, Ellipse, Line, Point, Rect, RoundedRect, Shape as _},
};

/// One text to draw.
pub(crate) struct TextSpec<'a> {
    pub text: &'a str,
    pub font_size: f32,
    pub color: [u8; 4],
    pub style: &'a TextStyle,
    pub enter: Option<Motion>,
    pub exit: Option<Motion>,
    pub words: Option<WordState>,
    /// Layout units; 0 breaks lines only at line breaks.
    pub wrap: f32,
}

/// The text as it is drawn: in its case, with plain breaks and spaces.
fn drawn_text(spec: &TextSpec<'_>) -> String {
    spec.style
        .case
        .apply(spec.text)
        .replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{2029}'], "\n")
        .replace('\t', " ")
}

/// How one unit of text looks under the motions at once.
#[derive(Clone, Copy)]
struct Look {
    transform: Affine,
    opacity: f32,
    /// The fraction of each moved unit shown from its left, with its box.
    reveal: Option<(f32, Rect, Affine)>,
}

impl Look {
    const REST: Self = Self {
        transform: Affine::IDENTITY,
        opacity: 1.0,
        reveal: None,
    };
}

/// A unit look about `rect`, with `em` its size: moved, scaled about its
/// centre.
fn look_transform(look: &UnitLook, rect: Rect, em: f32) -> Affine {
    let centre = rect.center().to_vec2();
    Affine::translate((
        f64::from(look.offset[0] * em),
        f64::from(look.offset[1] * em),
    )) * Affine::translate(centre)
        * Affine::scale(f64::from(look.scale))
        * Affine::translate(-centre)
}

fn combine(motions: &[(Motion, usize, usize, Rect)], em: f32) -> Look {
    let mut look = Look::REST;
    for (motion, index, count, rect) in motions {
        let unit = title::unit_look(
            motion.kind,
            title::unit_progress(motion, *index, *count),
            motion.leaving,
        );
        let transform = look_transform(&unit, *rect, em);
        look.transform = transform * look.transform;
        look.opacity *= unit.opacity.clamp(0.0, 1.0);
        if unit.reveal < 1.0 {
            look.reveal = Some((unit.reveal.max(0.0), *rect, transform));
        }
    }
    look
}

/// The part of `rect` (through `transform`) shown by a reveal of `shown`,
/// as a clip in title space: everything left of its edge.
fn reveal_clip(shown: f32, rect: Rect, transform: Affine, em: f32) -> BezPath {
    let em = f64::from(em);
    let edge = rect.x0 + rect.width() * f64::from(shown);
    let clip = Rect::new(
        rect.x0 - em * 4.0,
        rect.y0 - em * 2.0,
        edge,
        rect.y1 + em * 2.0,
    );
    transform * clip.to_path(0.1)
}

fn rounded(rect: Rect, radius: f64) -> BezPath {
    let radius = radius.min(rect.width().min(rect.height()) / 2.0).max(0.0);
    RoundedRect::from_rect(rect, radius).to_path(0.1)
}

/// The unit a glyph is in for a motion: its index, how many there are, and
/// the box it moves about.
fn unit_of(
    laid: &Laid,
    unit: TextUnit,
    glyph: &super::layout::Placed,
) -> Option<(usize, usize, Rect)> {
    match unit {
        TextUnit::Whole => Some((0, 1, laid.bounds())),
        TextUnit::Line => {
            let line = laid.lines.get(glyph.line)?;
            Some((glyph.line, laid.lines.len(), laid.line_box(line)))
        }
        TextUnit::Word => {
            let word = glyph.word?;
            Some((word, laid.words.len(), *laid.words.get(word)?))
        }
        TextUnit::Letter => {
            let letter = glyph.letter?;
            Some((letter, laid.letters.len(), *laid.letters.get(letter)?))
        }
    }
}

/// The line a word is set on.
fn active_line(laid: &Laid, word: usize) -> usize {
    laid.glyphs
        .iter()
        .find(|glyph| glyph.word == Some(word))
        .map_or(usize::MAX, |glyph| glyph.line)
}

/// Adds one text to the scene: placed in title space by `place`, which is
/// given the text as set, and faded by `opacity`. Returns the text's box
/// in title space.
pub(crate) fn text_scene(
    scene: &mut Scene,
    book: &mut FontBook,
    spec: &TextSpec<'_>,
    place: impl Fn(&Laid) -> Affine,
    opacity: f32,
) -> Rect {
    let text = drawn_text(spec);
    let style = spec.style;
    let laid = lay_out(
        book,
        &text,
        &Setting {
            font: &style.font,
            size: spec.font_size,
            align: style.align,
            line_height: style.line_height,
            tracking: style.tracking,
            wrap: spec.wrap,
        },
    );
    let place = place(&laid);
    let text_box = place.transform_rect_bbox(laid.bounds());
    if laid.faces.is_empty() || opacity <= 0.0 {
        return text_box;
    }
    let em = spec.font_size;
    let face_base = scene.faces.len();
    scene.faces.extend(laid.faces.iter().cloned());
    let motions: Vec<Motion> = [spec.enter, spec.exit].into_iter().flatten().collect();
    let highlight = style.highlight.filter(|_| spec.words.is_some());
    let state = spec.words;
    let outline = style
        .outline
        .filter(|outline| outline.color[3] > 0)
        .map(|outline| (outline.color, outline.width * em * 2.0));
    let bolder = laid
        .faces
        .iter()
        .map(|face| face.embolden)
        .fold(0.0, f32::max)
        * em;
    let reach = f64::from(em * 0.35 + bolder + outline.map_or(0.0, |(_, width)| width / 2.0));

    let mut under = Group::new(opacity);
    // The background takes the look of the whole text when the text moves
    // as one, each line's when lines move with boxes of their own, and
    // otherwise fades in with the motion.
    if let Some(background) = style
        .background
        .filter(|background| background.color[3] > 0)
    {
        let pad = (
            f64::from(background.padding[0] * em),
            f64::from(background.padding[1] * em),
        );
        let radius = f64::from(background.radius * em);
        let inked: Vec<(usize, Rect)> = (laid.lines.iter().enumerate())
            .filter(|(_, line)| line.width > 0.0)
            .map(|(index, line)| (index, laid.line_ink(line)))
            .collect();
        let boxes: Vec<(Option<usize>, Rect)> = match background.shape {
            BackgroundShape::Lines => inked
                .iter()
                .map(|(index, rect)| (Some(*index), rect.inflate(pad.0, pad.1)))
                .collect(),
            BackgroundShape::Block => inked
                .iter()
                .map(|(_, rect)| *rect)
                .reduce(|a, b| a.union(b))
                .map(|rect| (None, rect.inflate(pad.0, pad.1)))
                .into_iter()
                .collect(),
        };
        // Boxes are drawn solid and faded together, so where two meet
        // they are no darker.
        let [r, g, b, alpha] = background.color;
        let mut backdrop = Group::new(f32::from(alpha) / 255.0);
        for (line, rect) in boxes {
            let mut look = Look::REST;
            for motion in &motions {
                let moved = match (motion.unit, line) {
                    (TextUnit::Whole, _) => Some((*motion, 0, 1, laid.bounds())),
                    (TextUnit::Line, Some(line)) => Some((
                        *motion,
                        line,
                        laid.lines.len(),
                        laid.line_box(&laid.lines[line]),
                    )),
                    _ => None,
                };
                match moved {
                    Some(moved) => {
                        let one = combine(&[moved], em);
                        look.transform = one.transform * look.transform;
                        look.opacity *= one.opacity;
                        look.reveal = one.reveal.or(look.reveal);
                    }
                    None => {
                        look.opacity *= title::unit_look(
                            title::TextAnimationKind::Fade,
                            motion.progress,
                            motion.leaving,
                        )
                        .opacity;
                    }
                }
            }
            let transform = place * look.transform;
            let mut group = Group::new(look.opacity);
            if let Some((shown, unit, moved)) = look.reveal {
                group
                    .clips
                    .push(reveal_clip(shown, unit, place * moved, em));
            }
            group.item(Item {
                transform,
                bounds: transform.transform_rect_bbox(rect),
                shape: Shape::Path(rounded(rect, radius)),
                paint: Paint::Fill([r, g, b, 255]),
            });
            backdrop.push(Node::Group(group));
        }
        under.push(Node::Group(backdrop));
    }

    // Glyphs in spans that move, fade and light up together.
    let mut strokes = Group::new(1.0);
    let mut fills = Group::new(1.0);
    let mut apart: Vec<Node> = Vec::new();
    let glyphs = &laid.glyphs;
    let mut start = 0;
    while start < glyphs.len() {
        let first = &glyphs[start];
        if first.word.is_none() && first.letter.is_none() {
            start += 1;
            continue;
        }
        let units: Vec<Option<(usize, usize, Rect)>> = motions
            .iter()
            .map(|motion| unit_of(&laid, motion.unit, first))
            .collect();
        let key = |glyph: &super::layout::Placed| {
            (
                glyph.face,
                glyph.word,
                motions
                    .iter()
                    .map(|motion| unit_of(&laid, motion.unit, glyph).map(|unit| unit.0))
                    .collect::<Vec<_>>(),
            )
        };
        let first_key = key(first);
        let mut end = start + 1;
        while end < glyphs.len()
            && (glyphs[end].word.is_some() || glyphs[end].letter.is_some())
            && key(&glyphs[end]) == first_key
        {
            end += 1;
        }
        let span = &glyphs[start..end];
        start = end;
        let moved: Vec<(Motion, usize, usize, Rect)> = (motions.iter().zip(&units))
            .filter_map(|(motion, unit)| {
                unit.map(|(index, count, rect)| (*motion, index, count, rect))
            })
            .collect();
        let look = combine(&moved, em);
        let mut group_opacity = look.opacity;
        let mut color = spec.color;
        let mut sweep = None;
        let mut grow = Affine::IDENTITY;
        let word = first.word;
        if let (Some(highlight), Some(state), Some(word)) = (highlight, state, word) {
            let active = state.active;
            let upcoming = active.is_none_or(|active| word > active);
            if upcoming {
                group_opacity *= highlight.upcoming;
            }
            let is_active = active == Some(word);
            match highlight.mode {
                HighlightMode::Word if is_active => color = highlight.color,
                HighlightMode::Fill if active.is_some_and(|active| word < active) => {
                    color = highlight.color
                }
                HighlightMode::Fill if is_active => sweep = Some(state.sweep),
                _ => {}
            }
            if let Some(active) = active.filter(|_| highlight.scale > 1.0)
                && let Some(&rect) = laid.words.get(active)
            {
                let grown = 1.0 + (highlight.scale - 1.0) * state.grown;
                if is_active {
                    grow = look_transform(
                        &UnitLook {
                            scale: grown,
                            ..UnitLook::SHOWN
                        },
                        rect,
                        em,
                    );
                } else if first.line == active_line(&laid, active) {
                    // The words beside it on its line step aside.
                    let room = rect.width() * f64::from(grown - 1.0) / 2.0;
                    let side = if word < active { -room } else { room };
                    grow = Affine::translate((side, 0.0));
                }
            }
            if is_active && highlight.mode == HighlightMode::Box && highlight.color[3] > 0 {
                let rect = laid.words[word].inflate(f64::from(em * 0.16), f64::from(em * 0.06));
                let transform = place * look.transform * grow;
                let mut group = Group::new(group_opacity);
                group.item(Item {
                    transform,
                    bounds: transform.transform_rect_bbox(rect),
                    shape: Shape::Path(rounded(rect, f64::from(em * 0.2))),
                    paint: Paint::Fill(highlight.color),
                });
                under.push(Node::Group(group));
            }
        }
        if group_opacity <= 0.0 || look.reveal.is_some_and(|(shown, ..)| shown <= 0.0) {
            continue;
        }
        let transform = place * look.transform * grow;
        let face = face_base + first.face;
        let colored = laid.faces[first.face].face.color;
        let positioned: Vec<Glyph> = span
            .iter()
            .map(|glyph| Glyph {
                id: glyph.id,
                x: glyph.x,
                y: glyph.y,
            })
            .collect();
        let local = span
            .iter()
            .map(|glyph| {
                let line = &laid.lines[glyph.line];
                Rect::new(
                    f64::from(glyph.x),
                    f64::from(line.baseline - laid.ascent),
                    f64::from(glyph.x + glyph.advance),
                    f64::from(line.baseline + laid.descent),
                )
                .inflate(reach, reach)
            })
            .reduce(|a, b| a.union(b))
            .unwrap_or(Rect::ZERO);
        let bounds = transform.transform_rect_bbox(local);
        let glyph_item = |paint| Item {
            transform,
            shape: Shape::Glyphs {
                face,
                size: em,
                glyphs: positioned.clone(),
            },
            paint,
            bounds,
        };
        // A weight the family lacks is made up by a stroke of the fill's
        // own colour; an outline goes as far past it.
        let bold = laid.faces[first.face].embolden * em * 2.0;
        let filled = |color: [u8; 4]| {
            let mut items = vec![glyph_item(Paint::Fill(color))];
            if bold > 0.0 {
                items.push(glyph_item(Paint::Stroke(color, bold)));
            }
            items
        };
        let stroke = outline
            .filter(|_| !colored)
            .map(|(color, width)| glyph_item(Paint::Stroke(color, width + bold)));
        let fill = filled(color);
        let swept = sweep.filter(|sweep| *sweep > 0.0).map(|sweep| {
            let word = laid.words[word.unwrap_or_default()];
            let mut group = Group::new(1.0);
            group.clips.push(reveal_clip(sweep, word, transform, em));
            for item in filled(highlight.map_or(color, |highlight| highlight.color)) {
                group.item(item);
            }
            group
        });
        let together = group_opacity < 1.0 || look.reveal.is_some();
        if together {
            let mut group = Group::new(group_opacity);
            if let Some((shown, unit, moved)) = look.reveal {
                group
                    .clips
                    .push(reveal_clip(shown, unit, place * moved, em));
            }
            group.children.extend(stroke.map(Node::Item));
            group.children.extend(fill.into_iter().map(Node::Item));
            group.children.extend(swept.map(Node::Group));
            apart.push(Node::Group(group));
        } else {
            strokes.children.extend(stroke.map(Node::Item));
            fills.children.extend(fill.into_iter().map(Node::Item));
            apart.extend(swept.map(Node::Group));
        }
    }
    let mut ink = Group::new(opacity);
    // Every outline under every fill, so a letter's outline never covers
    // its neighbour; units on their way in or out are drawn whole.
    for group in [strokes, fills] {
        if !group.is_empty() {
            ink.push(Node::Group(group));
        }
    }
    ink.children.extend(apart);
    if !under.is_empty() {
        scene.plain(under);
    }
    match style.shadow {
        Some(shadow) if shadow.color[3] > 0 => scene.shadowed(
            ink,
            Shadow {
                color: shadow.color,
                offset: [shadow.offset[0] * em, shadow.offset[1] * em],
                sigma: shadow.blur * em / 2.0,
            },
        ),
        _ => scene.plain(ink),
    }
    text_box
}

/// Adds a graphic's elements to the scene, its design box centred on the
/// title's anchor.
pub(crate) fn graphic_scene(
    scene: &mut Scene,
    book: &mut FontBook,
    size: [f32; 2],
    elements: &[ElementLayer],
) {
    let centre = Affine::translate((-f64::from(size[0]) / 2.0, -f64::from(size[1]) / 2.0));
    for layer in elements {
        let element = &layer.element;
        let [x, y, width, height] = element.frame.map(f64::from);
        let frame = Rect::new(x, y, x + width, y + height);
        let pivot = frame.center().to_vec2();
        let rotate = centre
            * Affine::translate(pivot)
            * Affine::rotate(f64::from(element.rotation).to_radians())
            * Affine::translate(-pivot);
        let motions: Vec<Motion> = [layer.enter, layer.exit].into_iter().flatten().collect();
        match &element.kind {
            ElementKind::Text {
                text,
                font_size,
                color,
                style,
            } => {
                let spec = TextSpec {
                    text,
                    font_size: *font_size,
                    color: *color,
                    style,
                    enter: layer.enter,
                    exit: layer.exit,
                    words: None,
                    wrap: width.abs() as f32,
                };
                text_scene(
                    scene,
                    book,
                    &spec,
                    |laid| {
                        let across = f64::from(laid.width);
                        let left = match style.align {
                            TextAlign::Left => frame.x0,
                            TextAlign::Center => frame.x0 + (frame.width() - across) / 2.0,
                            TextAlign::Right => frame.x1 - across,
                        };
                        let top = frame.y0 + (frame.height() - f64::from(laid.height)) / 2.0;
                        rotate * Affine::translate((left, top))
                    },
                    element.opacity,
                );
            }
            kind => {
                // A shape's em is a quarter of its longer side: what a
                // slide moves it by.
                let em = (width.abs().max(height.abs()) / 4.0).max(1.0) as f32;
                let moved: Vec<(Motion, usize, usize, Rect)> = motions
                    .iter()
                    .map(|motion| (*motion, 0, 1, frame))
                    .collect();
                let look = combine(&moved, em);
                let transform = rotate * look.transform;
                let mut group = Group::new(element.opacity * look.opacity);
                if let Some((shown, unit, moved)) = look.reveal {
                    if shown <= 0.0 {
                        continue;
                    }
                    group
                        .clips
                        .push(reveal_clip(shown, unit, rotate * moved, em));
                }
                shape_items(&mut group, kind, frame, transform);
                scene.plain(group);
            }
        }
    }
}

fn shape_items(group: &mut Group, kind: &ElementKind, frame: Rect, transform: Affine) {
    let item = |path: BezPath, paint: Paint, reach: f64| Item {
        transform,
        bounds: transform.transform_rect_bbox(path.bounding_box().inflate(reach, reach)),
        shape: Shape::Path(path),
        paint,
    };
    let rect = frame.abs();
    match kind {
        ElementKind::Rect {
            color,
            radius,
            stroke,
        } => {
            let path = rounded(rect, f64::from(*radius));
            group.item(item(path.clone(), Paint::Fill(*color), 1.0));
            if let Some(stroke) = stroke {
                let width = f64::from(stroke.width);
                group.item(item(path, Paint::Stroke(stroke.color, stroke.width), width));
            }
        }
        ElementKind::Ellipse { color, stroke } => {
            let path = Ellipse::from_rect(rect).to_path(0.1);
            group.item(item(path.clone(), Paint::Fill(*color), 1.0));
            if let Some(stroke) = stroke {
                let width = f64::from(stroke.width);
                group.item(item(path, Paint::Stroke(stroke.color, stroke.width), width));
            }
        }
        ElementKind::Line {
            color,
            width,
            arrow,
        } => {
            let from = Point::new(frame.x0, frame.y0);
            let to = Point::new(frame.x1, frame.y1);
            let stroke = f64::from(*width);
            let along = to - from;
            let length = along.hypot();
            let mut end = to;
            if *arrow && length > 0.0 {
                // A head three widths long, filled; the shaft stops at its
                // base.
                let head = (stroke * 3.5).min(length);
                let unit = along / length;
                let normal = unit.turn_90();
                let base = to - unit * head;
                let mut path = BezPath::new();
                path.move_to(to);
                path.line_to(base + normal * head * 0.6);
                path.line_to(base - normal * head * 0.6);
                path.close_path();
                group.item(item(path, Paint::Fill(*color), stroke));
                end = base + unit * (stroke * 0.5).min(head);
            }
            let path = Line::new(from, end).to_path(0.1);
            group.item(item(path, Paint::Stroke(*color, *width), stroke));
        }
        ElementKind::Text { .. } => {}
    }
}
