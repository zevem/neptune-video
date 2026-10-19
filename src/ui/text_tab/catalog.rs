//! The looks Neptune Video comes with: title and caption styles, graphic templates,
//! stickers and emoji. Each is plain title or graphic data, made once.
//!
//! Sizes are canvas pixels on a canvas 1080 pixels on its short side; a
//! look given to another canvas is scaled with it. Everything is drawn in
//! the bundled face, so a look is the same on every computer, apart from an
//! emoji, which is the system's.
use super::{Anchor, Group, Look};
use editor_core::{
    BackgroundShape, ClipContent, Element, ElementKind, Highlight, HighlightMode, ShapeStroke,
    TextAlign, TextAnimation, TextAnimationKind as K, TextBackground, TextCase, TextOutline,
    TextShadow, TextStyle, TextUnit, Time,
};
use std::sync::{Arc, LazyLock};

const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const INK: [u8; 4] = [18, 18, 26, 255];
const YELLOW: [u8; 4] = [255, 214, 0, 255];
const PINK: [u8; 4] = [255, 61, 127, 255];
const CYAN: [u8; 4] = [0, 224, 255, 255];
const RED: [u8; 4] = [236, 38, 56, 255];
const GREEN: [u8; 4] = [38, 196, 110, 255];
const MIST: [u8; 4] = [206, 208, 220, 255];
const NONE: [u8; 4] = [0, 0, 0, 0];

/// What every caption style says in its tile.
pub const CAPTION_SAMPLE: &str = "This is how your captions look";

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn anim(kind: K, unit: TextUnit, duration: f64) -> TextAnimation {
    TextAnimation {
        kind,
        unit,
        duration: seconds(duration),
        delay: Time::ZERO,
    }
}

fn after(mut animation: TextAnimation, delay: f64) -> TextAnimation {
    animation.delay = seconds(delay);
    animation
}

fn styled(weight: u16, change: impl FnOnce(&mut TextStyle)) -> TextStyle {
    let mut style = TextStyle::default();
    style.font.weight = weight;
    change(&mut style);
    style
}

fn outline(width: f32) -> Option<TextOutline> {
    Some(TextOutline {
        color: BLACK,
        width,
    })
}

fn shadow(color: [u8; 4], offset: [f32; 2], blur: f32) -> Option<TextShadow> {
    Some(TextShadow {
        color,
        offset,
        blur,
    })
}

fn background(
    color: [u8; 4],
    padding: [f32; 2],
    radius: f32,
    shape: BackgroundShape,
) -> Option<TextBackground> {
    Some(TextBackground {
        color,
        padding,
        radius,
        shape,
    })
}

fn highlight(mode: HighlightMode, color: [u8; 4], scale: f32, upcoming: f32) -> Option<Highlight> {
    Some(Highlight {
        mode,
        color,
        scale,
        upcoming,
    })
}

fn title(
    text: &str,
    font_size: f32,
    color: [u8; 4],
    style: TextStyle,
    enter: Option<TextAnimation>,
    exit: Option<TextAnimation>,
) -> ClipContent {
    ClipContent::Text {
        text: text.into(),
        font_size,
        color,
        style,
        enter,
        exit,
        words: Vec::new(),
    }
}

fn text(
    name: &str,
    words: &str,
    font_size: f32,
    color: [u8; 4],
    style: TextStyle,
    frame: [f32; 4],
) -> Element {
    Element::new(
        name,
        ElementKind::Text {
            text: words.into(),
            font_size,
            color,
            style,
        },
        frame,
    )
}

fn rect(name: &str, color: [u8; 4], radius: f32, frame: [f32; 4]) -> Element {
    Element::new(
        name,
        ElementKind::Rect {
            color,
            radius,
            stroke: None,
        },
        frame,
    )
}

fn framed(
    name: &str,
    color: [u8; 4],
    stroke: [u8; 4],
    width: f32,
    radius: f32,
    frame: [f32; 4],
) -> Element {
    Element::new(
        name,
        ElementKind::Rect {
            color,
            radius,
            stroke: Some(ShapeStroke {
                color: stroke,
                width,
            }),
        },
        frame,
    )
}

fn ellipse(name: &str, color: [u8; 4], stroke: Option<ShapeStroke>, frame: [f32; 4]) -> Element {
    Element::new(name, ElementKind::Ellipse { color, stroke }, frame)
}

fn line(name: &str, color: [u8; 4], width: f32, arrow: bool, frame: [f32; 4]) -> Element {
    Element::new(
        name,
        ElementKind::Line {
            color,
            width,
            arrow,
        },
        frame,
    )
}

/// An element with its entrance and exit.
fn moving(mut element: Element, enter: TextAnimation, exit: Option<TextAnimation>) -> Element {
    element.enter = Some(enter);
    element.exit = exit;
    element
}

fn turned(mut element: Element, degrees: f32) -> Element {
    element.rotation = degrees;
    element
}

fn left(weight: u16) -> TextStyle {
    styled(weight, |style| style.align = TextAlign::Left)
}

fn look(group: Group, anchor: Anchor, name: &str, tags: &str, content: ClipContent) -> Look {
    Look::builtin(group, anchor, name, tags, content)
}

fn titles() -> Vec<Look> {
    let fade_out = Some(anim(K::Fade, TextUnit::Whole, 0.3));
    let at =
        |name: &str, tags: &str, content| look(Group::Titles, Anchor::Centre, name, tags, content);
    vec![
        at(
            "Simple",
            "plain basic",
            title(
                "Your text",
                96.0,
                WHITE,
                TextStyle::default(),
                Some(anim(K::Fade, TextUnit::Whole, 0.4)),
                fade_out,
            ),
        ),
        at(
            "Bold outline",
            "stroke pop",
            title(
                "Make it pop",
                120.0,
                WHITE,
                styled(800, |style| {
                    style.case = TextCase::Upper;
                    style.outline = outline(0.09);
                }),
                Some(anim(K::Pop, TextUnit::Word, 0.6)),
                fade_out,
            ),
        ),
        at(
            "Hook",
            "yellow bounce tiktok shorts",
            title(
                "Wait for it…",
                110.0,
                YELLOW,
                styled(900, |style| {
                    style.outline = outline(0.1);
                    style.shadow = shadow(BLACK, [0.0, 0.08], 0.0);
                }),
                Some(anim(K::Bounce, TextUnit::Word, 0.8)),
                fade_out,
            ),
        ),
        at(
            "Boxed",
            "background label news",
            title(
                "Breaking news",
                84.0,
                INK,
                styled(700, |style| {
                    style.background = background(WHITE, [0.35, 0.15], 0.2, BackgroundShape::Lines);
                }),
                Some(anim(K::SlideUp, TextUnit::Whole, 0.5)),
                fade_out,
            ),
        ),
        at(
            "Neon",
            "glow",
            title(
                "Neon nights",
                110.0,
                [255, 236, 248, 255],
                styled(600, |style| {
                    style.outline = Some(TextOutline {
                        color: PINK,
                        width: 0.02,
                    });
                    style.shadow = shadow(PINK, [0.0, 0.0], 0.4);
                }),
                Some(anim(K::Fade, TextUnit::Letter, 0.8)),
                fade_out,
            ),
        ),
        at(
            "Typewriter",
            "typing",
            title(
                "Once upon a time…",
                80.0,
                WHITE,
                styled(500, |style| {
                    style.shadow = shadow([0, 0, 0, 160], [0.02, 0.04], 0.1);
                }),
                Some(anim(K::Typewriter, TextUnit::Letter, 1.2)),
                fade_out,
            ),
        ),
        at(
            "Headline",
            "big reveal",
            title(
                "The big reveal",
                150.0,
                WHITE,
                styled(900, |style| {
                    style.case = TextCase::Upper;
                    style.tracking = -0.02;
                    style.line_height = 1.0;
                    style.shadow = shadow([0, 0, 0, 170], [0.03, 0.05], 0.15);
                }),
                Some(anim(K::SlideUp, TextUnit::Word, 0.7)),
                fade_out,
            ),
        ),
        at(
            "Label",
            "sale offer tag",
            title(
                "Limited offer",
                64.0,
                WHITE,
                styled(700, |style| {
                    style.case = TextCase::Upper;
                    style.tracking = 0.12;
                    style.background = background(RED, [0.6, 0.3], 0.1, BackgroundShape::Block);
                }),
                Some(anim(K::Wipe, TextUnit::Whole, 0.5)),
                fade_out,
            ),
        ),
        at(
            "Quote",
            "italic",
            title(
                "“Make it simple, but significant.”",
                72.0,
                WHITE,
                styled(400, |style| {
                    style.font.italic = true;
                    style.wrap = 1100.0;
                    style.line_height = 1.3;
                }),
                Some(anim(K::Fade, TextUnit::Word, 1.2)),
                fade_out,
            ),
        ),
        at(
            "Soft shadow",
            "drop",
            title(
                "Good morning",
                100.0,
                WHITE,
                styled(700, |style| {
                    style.shadow = shadow([0, 0, 0, 180], [0.04, 0.06], 0.2);
                }),
                Some(anim(K::SlideLeft, TextUnit::Whole, 0.5)),
                fade_out,
            ),
        ),
    ]
}

fn captions() -> Vec<Look> {
    let caption = |weight: u16, change: &dyn Fn(&mut TextStyle)| {
        styled(weight, |style| {
            style.wrap = 1400.0;
            change(style);
        })
    };
    let at = |name: &str, tags: &str, color, style| {
        look(
            Group::Captions,
            Anchor::Lower,
            name,
            tags,
            title(CAPTION_SAMPLE, 72.0, color, style, None, None),
        )
    };
    vec![
        at(
            "Clean",
            "subtitle plain",
            WHITE,
            caption(600, &|style| {
                style.shadow = shadow([0, 0, 0, 200], [0.03, 0.04], 0.12);
            }),
        ),
        at(
            "Subtitle bar",
            "background broadcast",
            WHITE,
            caption(500, &|style| {
                style.background =
                    background([0, 0, 0, 170], [0.3, 0.1], 0.12, BackgroundShape::Lines);
            }),
        ),
        at(
            "Karaoke box",
            "highlight words pink",
            WHITE,
            caption(800, &|style| {
                style.case = TextCase::Upper;
                style.outline = outline(0.06);
                style.highlight = highlight(HighlightMode::Box, PINK, 1.1, 1.0);
            }),
        ),
        at(
            "Word pop",
            "highlight words yellow karaoke",
            WHITE,
            caption(900, &|style| {
                style.case = TextCase::Upper;
                style.outline = outline(0.1);
                style.shadow = shadow(BLACK, [0.0, 0.06], 0.0);
                style.highlight = highlight(HighlightMode::Word, YELLOW, 1.3, 1.0);
            }),
        ),
        at(
            "Fill sweep",
            "highlight karaoke green",
            WHITE,
            caption(700, &|style| {
                style.outline = outline(0.05);
                style.highlight = highlight(HighlightMode::Fill, GREEN, 1.0, 1.0);
            }),
        ),
        at(
            "Reveal",
            "word by word highlight karaoke",
            WHITE,
            caption(800, &|style| {
                style.shadow = shadow([0, 0, 0, 200], [0.0, 0.05], 0.1);
                style.highlight = highlight(HighlightMode::Word, CYAN, 1.15, 0.0);
            }),
        ),
    ]
}

fn lower_thirds() -> Vec<Look> {
    let out = Some(anim(K::Fade, TextUnit::Whole, 0.3));
    let at = |name: &str, tags: &str, size: [f32; 2], elements| {
        look(
            Group::LowerThirds,
            Anchor::LowerLeft,
            name,
            tags,
            ClipContent::Graphic { size, elements },
        )
    };
    vec![
        at(
            "Name bar",
            "lower third name role",
            [580.0, 130.0],
            vec![
                moving(
                    rect("Bar", YELLOW, 6.0, [0.0, 0.0, 14.0, 130.0]),
                    anim(K::SlideUp, TextUnit::Whole, 0.3),
                    out,
                ),
                moving(
                    rect("Panel", [16, 16, 24, 220], 10.0, [24.0, 0.0, 556.0, 130.0]),
                    after(anim(K::Wipe, TextUnit::Whole, 0.4), 0.1),
                    out,
                ),
                moving(
                    text(
                        "Name",
                        "Ada Lovelace",
                        46.0,
                        WHITE,
                        left(700),
                        [52.0, 12.0, 520.0, 60.0],
                    ),
                    after(anim(K::Fade, TextUnit::Word, 0.4), 0.3),
                    out,
                ),
                moving(
                    text(
                        "Role",
                        "First programmer",
                        30.0,
                        MIST,
                        left(400),
                        [52.0, 68.0, 520.0, 48.0],
                    ),
                    after(anim(K::Fade, TextUnit::Whole, 0.4), 0.45),
                    out,
                ),
            ],
        ),
        at(
            "Underline",
            "lower third name line",
            [560.0, 124.0],
            vec![
                moving(
                    text(
                        "Name",
                        "Grace Hopper",
                        52.0,
                        WHITE,
                        left(700),
                        [0.0, 0.0, 560.0, 66.0],
                    ),
                    anim(K::SlideUp, TextUnit::Word, 0.5),
                    out,
                ),
                moving(
                    line("Line", YELLOW, 6.0, false, [0.0, 74.0, 220.0, 0.0]),
                    after(anim(K::Wipe, TextUnit::Whole, 0.4), 0.2),
                    out,
                ),
                moving(
                    text(
                        "Role",
                        "Computer scientist",
                        30.0,
                        MIST,
                        left(400),
                        [0.0, 84.0, 560.0, 40.0],
                    ),
                    after(anim(K::Fade, TextUnit::Whole, 0.4), 0.4),
                    out,
                ),
            ],
        ),
        at(
            "Pill",
            "lower third handle follow social",
            [520.0, 96.0],
            vec![
                moving(
                    rect("Pill", PINK, 48.0, [0.0, 0.0, 520.0, 96.0]),
                    anim(K::Pop, TextUnit::Whole, 0.5),
                    out,
                ),
                moving(
                    text(
                        "Handle",
                        "Follow @yourname",
                        40.0,
                        WHITE,
                        styled(700, |_| {}),
                        [24.0, 0.0, 472.0, 96.0],
                    ),
                    after(anim(K::Fade, TextUnit::Whole, 0.3), 0.25),
                    out,
                ),
            ],
        ),
        at(
            "Two tone",
            "lower third name role",
            [600.0, 140.0],
            vec![
                moving(
                    rect("Top", YELLOW, 6.0, [0.0, 0.0, 600.0, 80.0]),
                    anim(K::Wipe, TextUnit::Whole, 0.4),
                    out,
                ),
                moving(
                    text(
                        "Name",
                        "Katherine Johnson",
                        42.0,
                        INK,
                        left(800),
                        [24.0, 0.0, 560.0, 80.0],
                    ),
                    after(anim(K::Fade, TextUnit::Whole, 0.3), 0.3),
                    out,
                ),
                moving(
                    rect("Bottom", WHITE, 6.0, [0.0, 80.0, 420.0, 60.0]),
                    after(anim(K::Wipe, TextUnit::Whole, 0.4), 0.15),
                    out,
                ),
                moving(
                    text(
                        "Role",
                        "Mathematician",
                        28.0,
                        INK,
                        left(500),
                        [24.0, 80.0, 380.0, 60.0],
                    ),
                    after(anim(K::Fade, TextUnit::Whole, 0.3), 0.45),
                    out,
                ),
            ],
        ),
    ]
}

fn title_cards() -> Vec<Look> {
    let out = Some(anim(K::Fade, TextUnit::Whole, 0.4));
    let at = |name: &str, tags: &str, size: [f32; 2], elements| {
        look(
            Group::TitleCards,
            Anchor::Centre,
            name,
            tags,
            ClipContent::Graphic { size, elements },
        )
    };
    vec![
        at(
            "Big title",
            "title card intro subtitle",
            [1200.0, 340.0],
            vec![
                moving(
                    text(
                        "Title",
                        "Your title",
                        140.0,
                        WHITE,
                        styled(800, |_| {}),
                        [0.0, 20.0, 1200.0, 180.0],
                    ),
                    anim(K::SlideUp, TextUnit::Word, 0.6),
                    out,
                ),
                moving(
                    line(
                        "Divider",
                        [255, 255, 255, 200],
                        4.0,
                        false,
                        [450.0, 220.0, 300.0, 0.0],
                    ),
                    after(anim(K::Wipe, TextUnit::Whole, 0.4), 0.3),
                    out,
                ),
                moving(
                    text(
                        "Subtitle",
                        "A short subtitle",
                        48.0,
                        MIST,
                        styled(400, |_| {}),
                        [0.0, 240.0, 1200.0, 70.0],
                    ),
                    after(anim(K::Fade, TextUnit::Whole, 0.5), 0.5),
                    out,
                ),
            ],
        ),
        at(
            "Chapter",
            "title card section kicker frame",
            [1000.0, 300.0],
            vec![
                moving(
                    framed("Frame", NONE, WHITE, 3.0, 0.0, [0.0, 0.0, 1000.0, 300.0]),
                    anim(K::Fade, TextUnit::Whole, 0.6),
                    out,
                ),
                moving(
                    text(
                        "Kicker",
                        "Chapter one",
                        36.0,
                        YELLOW,
                        styled(600, |style| {
                            style.case = TextCase::Upper;
                            style.tracking = 0.3;
                        }),
                        [0.0, 36.0, 1000.0, 50.0],
                    ),
                    after(anim(K::Typewriter, TextUnit::Letter, 0.6), 0.2),
                    out,
                ),
                moving(
                    text(
                        "Title",
                        "The beginning",
                        110.0,
                        WHITE,
                        styled(700, |_| {}),
                        [0.0, 100.0, 1000.0, 150.0],
                    ),
                    after(anim(K::Fade, TextUnit::Word, 0.6), 0.5),
                    out,
                ),
            ],
        ),
        at(
            "Boxed title",
            "title card episode panel",
            [900.0, 260.0],
            vec![
                moving(
                    rect("Panel", [16, 16, 24, 235], 24.0, [0.0, 0.0, 900.0, 260.0]),
                    anim(K::Pop, TextUnit::Whole, 0.5),
                    out,
                ),
                moving(
                    text(
                        "Title",
                        "Episode 12",
                        96.0,
                        WHITE,
                        styled(800, |_| {}),
                        [40.0, 34.0, 820.0, 124.0],
                    ),
                    after(anim(K::SlideUp, TextUnit::Whole, 0.4), 0.25),
                    out,
                ),
                moving(
                    text(
                        "Subtitle",
                        "Season finale",
                        40.0,
                        YELLOW,
                        styled(600, |_| {}),
                        [40.0, 160.0, 820.0, 60.0],
                    ),
                    after(anim(K::SlideUp, TextUnit::Whole, 0.4), 0.4),
                    out,
                ),
            ],
        ),
    ]
}

fn callouts() -> Vec<Look> {
    let out = Some(anim(K::Fade, TextUnit::Whole, 0.3));
    let at = |name: &str, tags: &str, size: [f32; 2], elements| {
        look(
            Group::Callouts,
            Anchor::Centre,
            name,
            tags,
            ClipContent::Graphic { size, elements },
        )
    };
    vec![
        at(
            "Arrow label",
            "callout pointer",
            [520.0, 230.0],
            vec![
                moving(
                    rect("Label", YELLOW, 14.0, [0.0, 0.0, 300.0, 84.0]),
                    anim(K::Pop, TextUnit::Whole, 0.4),
                    out,
                ),
                moving(
                    text(
                        "Text",
                        "Look here",
                        40.0,
                        INK,
                        styled(700, |_| {}),
                        [0.0, 0.0, 300.0, 84.0],
                    ),
                    after(anim(K::Fade, TextUnit::Whole, 0.3), 0.15),
                    out,
                ),
                moving(
                    line("Arrow", YELLOW, 8.0, true, [300.0, 64.0, 200.0, 146.0]),
                    after(anim(K::Wipe, TextUnit::Whole, 0.4), 0.25),
                    out,
                ),
            ],
        ),
        at(
            "Circle",
            "callout ring highlight",
            [400.0, 300.0],
            vec![moving(
                ellipse(
                    "Ring",
                    NONE,
                    Some(ShapeStroke {
                        color: RED,
                        width: 10.0,
                    }),
                    [10.0, 10.0, 380.0, 280.0],
                ),
                anim(K::Pop, TextUnit::Whole, 0.4),
                out,
            )],
        ),
        at(
            "Speech bubble",
            "callout talk",
            [520.0, 240.0],
            vec![
                moving(
                    turned(rect("Tail", WHITE, 6.0, [80.0, 150.0, 64.0, 64.0]), 45.0),
                    anim(K::Pop, TextUnit::Whole, 0.4),
                    out,
                ),
                moving(
                    rect("Bubble", WHITE, 40.0, [0.0, 0.0, 520.0, 190.0]),
                    anim(K::Pop, TextUnit::Whole, 0.4),
                    out,
                ),
                moving(
                    text(
                        "Text",
                        "Hello there!",
                        48.0,
                        INK,
                        styled(600, |_| {}),
                        [30.0, 0.0, 460.0, 190.0],
                    ),
                    after(anim(K::Typewriter, TextUnit::Letter, 0.6), 0.3),
                    out,
                ),
            ],
        ),
        at(
            "Subscribe",
            "callout button youtube",
            [440.0, 110.0],
            vec![
                moving(
                    rect("Button", RED, 16.0, [0.0, 0.0, 440.0, 110.0]),
                    anim(K::Pop, TextUnit::Whole, 0.5),
                    out,
                ),
                moving(
                    text(
                        "Text",
                        "Subscribe",
                        46.0,
                        WHITE,
                        styled(800, |style| {
                            style.case = TextCase::Upper;
                            style.tracking = 0.05;
                        }),
                        [0.0, 0.0, 440.0, 110.0],
                    ),
                    after(anim(K::Fade, TextUnit::Whole, 0.3), 0.2),
                    out,
                ),
            ],
        ),
        at(
            "Highlight box",
            "callout frame",
            [600.0, 200.0],
            vec![moving(
                framed(
                    "Box",
                    [255, 214, 0, 56],
                    YELLOW,
                    6.0,
                    12.0,
                    [0.0, 0.0, 600.0, 200.0],
                ),
                anim(K::Wipe, TextUnit::Whole, 0.4),
                out,
            )],
        ),
    ]
}

fn stickers() -> Vec<Look> {
    let pop = anim(K::Pop, TextUnit::Whole, 0.5);
    let out = Some(anim(K::Pop, TextUnit::Whole, 0.3));
    let at = |name: &str, tags: &str, size: [f32; 2], elements| {
        look(
            Group::Stickers,
            Anchor::Centre,
            name,
            tags,
            ClipContent::Graphic { size, elements },
        )
    };
    let tick = |name: &str, frame| moving(line(name, WHITE, 22.0, false, frame), pop, out);
    vec![
        at("Starburst", "sticker burst wow star", [360.0, 360.0], {
            let mut parts: Vec<Element> = [0.0, 22.5, 45.0, 67.5]
                .into_iter()
                .enumerate()
                .map(|(index, degrees)| {
                    moving(
                        turned(
                            rect(
                                &format!("Ray {}", index + 1),
                                YELLOW,
                                0.0,
                                [80.0, 80.0, 200.0, 200.0],
                            ),
                            degrees,
                        ),
                        pop,
                        out,
                    )
                })
                .collect();
            parts.push(moving(
                turned(
                    text(
                        "Text",
                        "Wow!",
                        72.0,
                        INK,
                        styled(900, |_| {}),
                        [60.0, 130.0, 240.0, 100.0],
                    ),
                    -10.0,
                ),
                after(pop, 0.15),
                out,
            ));
            parts
        }),
        at(
            "New badge",
            "sticker badge",
            [260.0, 260.0],
            vec![
                moving(
                    ellipse("Badge", RED, None, [0.0, 0.0, 260.0, 260.0]),
                    pop,
                    out,
                ),
                moving(
                    turned(
                        text(
                            "Text",
                            "New",
                            76.0,
                            WHITE,
                            styled(900, |style| style.case = TextCase::Upper),
                            [0.0, 80.0, 260.0, 100.0],
                        ),
                        -12.0,
                    ),
                    after(pop, 0.15),
                    out,
                ),
            ],
        ),
        at(
            "Sale tag",
            "sticker discount price",
            [360.0, 180.0],
            vec![
                moving(
                    turned(rect("Tag", YELLOW, 20.0, [20.0, 20.0, 320.0, 140.0]), -8.0),
                    anim(K::Bounce, TextUnit::Whole, 0.6),
                    out,
                ),
                moving(
                    turned(
                        text(
                            "Text",
                            "-50%",
                            84.0,
                            INK,
                            styled(900, |_| {}),
                            [20.0, 20.0, 320.0, 140.0],
                        ),
                        -8.0,
                    ),
                    anim(K::Bounce, TextUnit::Whole, 0.6),
                    out,
                ),
            ],
        ),
        at(
            "Check",
            "sticker tick yes done",
            [240.0, 240.0],
            vec![
                moving(
                    ellipse("Disc", GREEN, None, [0.0, 0.0, 240.0, 240.0]),
                    pop,
                    out,
                ),
                tick("Short stroke", [62.0, 124.0, 42.0, 42.0]),
                tick("Long stroke", [104.0, 166.0, 76.0, -82.0]),
            ],
        ),
        at(
            "Cross",
            "sticker no wrong",
            [240.0, 240.0],
            vec![
                moving(
                    ellipse("Disc", RED, None, [0.0, 0.0, 240.0, 240.0]),
                    pop,
                    out,
                ),
                tick("Stroke", [74.0, 74.0, 92.0, 92.0]),
                tick("Back stroke", [166.0, 74.0, -92.0, 92.0]),
            ],
        ),
        at(
            "Arrow",
            "sticker pointer",
            [300.0, 120.0],
            vec![moving(
                line("Arrow", YELLOW, 18.0, true, [10.0, 60.0, 270.0, 0.0]),
                anim(K::Wipe, TextUnit::Whole, 0.4),
                out,
            )],
        ),
        at(
            "Like",
            "sticker thumbs button",
            [280.0, 110.0],
            vec![
                moving(
                    rect("Button", WHITE, 55.0, [0.0, 0.0, 280.0, 110.0]),
                    pop,
                    out,
                ),
                moving(
                    text(
                        "Text",
                        "👍 Like",
                        46.0,
                        INK,
                        styled(600, |_| {}),
                        [0.0, 0.0, 280.0, 110.0],
                    ),
                    after(pop, 0.1),
                    out,
                ),
            ],
        ),
    ]
}

/// Emoji, each with the name it is found by.
pub const EMOJI: [(&str, &str); 48] = [
    ("😂", "Laughing"),
    ("😍", "Heart eyes"),
    ("🥰", "Smiling with hearts"),
    ("😎", "Cool"),
    ("🤯", "Mind blown"),
    ("😱", "Scream"),
    ("😭", "Crying"),
    ("🥳", "Party face"),
    ("🤔", "Thinking"),
    ("😅", "Sweat smile"),
    ("🙃", "Upside down"),
    ("😴", "Sleepy"),
    ("🔥", "Fire"),
    ("✨", "Sparkles"),
    ("💯", "Hundred"),
    ("💥", "Boom"),
    ("⚡", "Lightning"),
    ("⭐", "Star"),
    ("❤️", "Red heart"),
    ("💔", "Broken heart"),
    ("👍", "Thumbs up"),
    ("👎", "Thumbs down"),
    ("👏", "Clapping"),
    ("🙌", "Raised hands"),
    ("🙏", "Thank you"),
    ("💪", "Strong"),
    ("👀", "Eyes"),
    ("👉", "Pointing right"),
    ("👇", "Pointing down"),
    ("✅", "Check mark"),
    ("❌", "Cross mark"),
    ("❗", "Exclamation"),
    ("❓", "Question"),
    ("🎉", "Celebration"),
    ("🎁", "Gift"),
    ("🚀", "Rocket"),
    ("💡", "Idea"),
    ("📌", "Pin"),
    ("📈", "Chart rising"),
    ("💰", "Money"),
    ("🛒", "Shopping cart"),
    ("⏰", "Alarm clock"),
    ("🎵", "Music"),
    ("📣", "Megaphone"),
    ("🍕", "Pizza"),
    ("☕", "Coffee"),
    ("🌈", "Rainbow"),
    ("🌟", "Glowing star"),
];

fn emoji() -> Vec<Look> {
    EMOJI
        .iter()
        .map(|(emoji, name)| {
            look(
                Group::Emoji,
                Anchor::Centre,
                name,
                "emoji",
                title(
                    emoji,
                    200.0,
                    WHITE,
                    TextStyle::default(),
                    Some(anim(K::Pop, TextUnit::Whole, 0.4)),
                    Some(anim(K::Fade, TextUnit::Whole, 0.3)),
                ),
            )
        })
        .collect()
}

/// Every look Neptune Video comes with, in the order the tab lists them.
pub fn builtin() -> &'static [Arc<Look>] {
    static LOOKS: LazyLock<Vec<Arc<Look>>> = LazyLock::new(|| {
        [
            titles(),
            captions(),
            lower_thirds(),
            title_cards(),
            callouts(),
            stickers(),
            emoji(),
        ]
        .into_iter()
        .flatten()
        .map(Arc::new)
        .collect()
    });
    &LOOKS
}
