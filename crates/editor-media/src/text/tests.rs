use super::*;
use editor_core::{
    Element, ElementKind, FontFace, Highlight, HighlightMode, ShapeStroke, TextAnimationKind,
    TextBackground, TextOutline, TextShadow, TextStyle, TextUnit,
    title::{ElementLayer, Motion, WordState},
};
use std::{path::PathBuf, sync::mpsc, time::Duration};

const REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/Geist-Regular.ttf");
const SEMIBOLD: &[u8] = include_bytes!("../../../../assets/fonts/Geist-SemiBold.ttf");

/// A book of the bundled faces alone: no system face changes a picture.
fn book() -> FontBook {
    FontBook::new(&[REGULAR, SEMIBOLD]).with_system_roots(Vec::new())
}

fn title(text: &str, style: TextStyle) -> LayerSource {
    LayerSource::Text {
        text: text.into(),
        font_size: 100.0,
        color: [255, 255, 255, 255],
        style,
        enter: None,
        exit: None,
        words: None,
    }
}

/// Where a picture has ink, in canvas pixels from the anchor.
struct Ink {
    pixels: usize,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

fn ink(picture: &TitlePicture) -> Ink {
    let mut found = Ink {
        pixels: 0,
        left: f32::MAX,
        top: f32::MAX,
        right: f32::MIN,
        bottom: f32::MIN,
    };
    for y in 0..picture.height {
        for x in 0..picture.width {
            let alpha = picture.rgba[((y * picture.width + x) * 4 + 3) as usize];
            if alpha > 16 {
                found.pixels += 1;
                let cx = picture.origin[0] + x as f32 / picture.scale;
                let cy = picture.origin[1] + y as f32 / picture.scale;
                found.left = found.left.min(cx);
                found.top = found.top.min(cy);
                found.right = found.right.max(cx);
                found.bottom = found.bottom.max(cy);
            }
        }
    }
    found
}

fn pixel(picture: &TitlePicture, canvas: [f32; 2]) -> [u8; 4] {
    let x = ((canvas[0] - picture.origin[0]) * picture.scale) as u32;
    let y = ((canvas[1] - picture.origin[1]) * picture.scale) as u32;
    assert!(
        x < picture.width && y < picture.height,
        "{canvas:?} is off the picture"
    );
    let at = ((y * picture.width + x) * 4) as usize;
    picture.rgba[at..at + 4].try_into().unwrap()
}

/// Pixels of a colour, near enough, counted by how red, green or blue they
/// lean.
fn count(picture: &TitlePicture, test: impl Fn([u8; 4]) -> bool) -> usize {
    picture
        .rgba
        .chunks_exact(4)
        .filter(|pixel| test([pixel[0], pixel[1], pixel[2], pixel[3]]))
        .count()
}

#[test]
fn a_plain_title_is_drawn_inside_its_box_about_its_anchor() {
    let mut book = book();
    let picture = draw_title(&mut book, &title("Hello", TextStyle::default()), 1.0).unwrap();
    let ink = ink(&picture);
    assert!(ink.pixels > 2000, "{} pixels", ink.pixels);
    let [left, top, right, bottom] = picture.bounds;
    // The box is centred on the anchor, as wide as the text and one line
    // of 1.2 em tall.
    assert!((left + right).abs() < 0.01 && (top + bottom).abs() < 0.01);
    assert!((bottom - top - 120.0).abs() < 0.5, "{:?}", picture.bounds);
    assert!(
        right - left > 150.0 && right - left < 300.0,
        "{:?}",
        picture.bounds
    );
    // Ink inside the box, give or take an overhang.
    assert!(ink.left >= left - 4.0 && ink.right <= right + 4.0);
    assert!(ink.top >= top && ink.bottom <= bottom);
    // Premultiplied white: no channel above its alpha.
    assert!(
        picture
            .rgba
            .chunks_exact(4)
            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
    );
}

#[test]
fn a_picture_follows_its_scale_and_keeps_its_place() {
    let mut book = book();
    let source = title("Scale", TextStyle::default());
    let one = draw_title(&mut book, &source, 1.0).unwrap();
    let two = draw_title(&mut book, &source, 2.0).unwrap();
    assert_eq!(two.scale, 2.0);
    assert!(two.width >= one.width * 2 - 2 && two.width <= one.width * 2 + 2);
    let (a, b) = (ink(&one), ink(&two));
    assert!((a.left - b.left).abs() < 1.5 && (a.top - b.top).abs() < 1.5);
    assert_eq!(one.bounds, two.bounds);
}

#[test]
fn scale_steps_by_eighths_of_an_octave_and_pictures_stay_in_bounds() {
    assert_eq!(quantize_scale(1.03), 1.0);
    assert_eq!(quantize_scale(2.0), 2.0);
    assert!((quantize_scale(1.5) - 2_f32.powf(0.625)).abs() < 1e-5);
    assert_eq!(quantize_scale(f32::NAN), 1.0);
    assert_eq!(quantize_scale(1000.0), MAX_TITLE_SCALE);
    let mut book = book();
    let wide = "W".repeat(200);
    let picture = draw_title(&mut book, &title(&wide, TextStyle::default()), 4.0).unwrap();
    assert!(picture.width <= MAX_TITLE_SIDE && picture.height <= MAX_TITLE_SIDE);
    assert!(picture.scale < 4.0, "drawn smaller to fit");
}

#[test]
fn an_outline_grows_the_ink_in_its_own_colour() {
    let mut book = book();
    let plain = draw_title(&mut book, &title("Outline", TextStyle::default()), 1.0).unwrap();
    let style = TextStyle {
        outline: Some(TextOutline {
            color: [255, 0, 0, 255],
            width: 0.08,
        }),
        ..TextStyle::default()
    };
    let outlined = draw_title(&mut book, &title("Outline", style), 1.0).unwrap();
    let (a, b) = (ink(&plain), ink(&outlined));
    assert!(
        b.pixels > a.pixels * 13 / 10,
        "{} vs {}",
        b.pixels,
        a.pixels
    );
    // Eight hundredths of an em reach about eight pixels further.
    assert!(b.left < a.left - 5.0 && b.right > a.right + 5.0);
    let red = count(&outlined, |[r, g, _, a]| a > 200 && r > 200 && g < 40);
    let white = count(&outlined, |[r, g, b, a]| {
        a > 200 && r > 200 && g > 200 && b > 200
    });
    assert!(red > 500 && white > 500, "{red} red, {white} white");
}

#[test]
fn a_shadow_lands_where_it_is_cast_and_blurs() {
    let mut book = book();
    let plain = draw_title(&mut book, &title("Shade", TextStyle::default()), 1.0).unwrap();
    let cast = |blur| TextStyle {
        shadow: Some(TextShadow {
            color: [0, 0, 255, 255],
            offset: [0.2, 0.3],
            blur,
        }),
        ..TextStyle::default()
    };
    let sharp = draw_title(&mut book, &title("Shade", cast(0.0)), 1.0).unwrap();
    let soft = draw_title(&mut book, &title("Shade", cast(0.2)), 1.0).unwrap();
    let (a, b, c) = (ink(&plain), ink(&sharp), ink(&soft));
    assert!(
        (b.right - a.right - 20.0).abs() < 3.0,
        "{} {}",
        b.right,
        a.right
    );
    assert!((b.bottom - a.bottom - 30.0).abs() < 3.0);
    assert!((b.left - a.left).abs() < 2.0, "the text stays put");
    assert!(
        c.right > b.right + 3.0,
        "a blur spreads past a sharp shadow"
    );
    let blue = count(&sharp, |[r, _, b, a]| a > 200 && b > 200 && r < 40);
    assert!(blue > 500, "{blue}");
    // The picture's box is the text's, not the shadow's.
    assert_eq!(plain.bounds, sharp.bounds);
}

#[test]
fn a_background_box_lies_behind_the_text_with_its_padding() {
    let mut book = book();
    let style = TextStyle {
        background: Some(TextBackground {
            color: [0, 160, 0, 255],
            padding: [0.3, 0.1],
            radius: 0.0,
            shape: Default::default(),
        }),
        ..TextStyle::default()
    };
    let picture = draw_title(&mut book, &title("Box", style), 1.0).unwrap();
    let [left, _, right, _] = picture.bounds;
    // Thirty pixels of padding beside the text, all green.
    assert_eq!(pixel(&picture, [left - 15.0, 0.0]), [0, 160, 0, 255]);
    assert_eq!(pixel(&picture, [right + 15.0, 0.0]), [0, 160, 0, 255]);
    let ink = ink(&picture);
    assert!(ink.left < left - 28.0 && ink.right > right + 28.0);
}

#[test]
fn lines_break_at_the_wrap_width_and_at_line_breaks() {
    let mut book = book();
    let one = draw_title(
        &mut book,
        &title("one two three four", TextStyle::default()),
        1.0,
    )
    .unwrap();
    let style = TextStyle {
        wrap: 320.0,
        ..TextStyle::default()
    };
    let wrapped = draw_title(&mut book, &title("one two three four", style), 1.0).unwrap();
    let height = |picture: &TitlePicture| picture.bounds[3] - picture.bounds[1];
    let width = |picture: &TitlePicture| picture.bounds[2] - picture.bounds[0];
    assert!((height(&one) - 120.0).abs() < 0.5);
    assert!(height(&wrapped) >= 240.0 - 0.5, "{}", height(&wrapped));
    assert!(width(&wrapped) <= 320.0, "{}", width(&wrapped));
    let broken = draw_title(
        &mut book,
        &title("one\ntwo\r\nthree", TextStyle::default()),
        1.0,
    )
    .unwrap();
    assert!((height(&broken) - 360.0).abs() < 0.5);
    // A word wider than the wrap breaks between its letters.
    let style = TextStyle {
        wrap: 150.0,
        ..TextStyle::default()
    };
    let long = draw_title(&mut book, &title("Supercalifragilistic", style), 1.0).unwrap();
    assert!(width(&long) <= 150.0 && height(&long) > 240.0);
}

#[test]
fn upper_case_and_tracking_change_the_set_text() {
    let mut book = book();
    let width = |picture: &TitlePicture| picture.bounds[2] - picture.bounds[0];
    let plain = draw_title(&mut book, &title("track", TextStyle::default()), 1.0).unwrap();
    let tracked = TextStyle {
        tracking: 0.2,
        ..TextStyle::default()
    };
    let spaced = draw_title(&mut book, &title("track", tracked), 1.0).unwrap();
    // Four gaps between five letters, twenty pixels each.
    assert!((width(&spaced) - width(&plain) - 80.0).abs() < 1.0);
    let upper = TextStyle {
        case: editor_core::TextCase::Upper,
        ..TextStyle::default()
    };
    let shouted = draw_title(&mut book, &title("track", upper), 1.0).unwrap();
    assert!(width(&shouted) > width(&plain));
}

fn karaoke(mode: HighlightMode, active: Option<usize>, upcoming: f32) -> LayerSource {
    LayerSource::Text {
        text: "red red".into(),
        font_size: 100.0,
        color: [255, 255, 255, 255],
        style: TextStyle {
            highlight: Some(Highlight {
                mode,
                color: [255, 0, 0, 255],
                scale: 1.0,
                upcoming,
            }),
            ..TextStyle::default()
        },
        enter: None,
        exit: None,
        words: Some(WordState {
            active,
            spoken: active.unwrap_or(0),
            sweep: 1.0,
            grown: 1.0,
        }),
    }
}

#[test]
fn karaoke_lights_the_word_being_said() {
    let mut book = book();
    let red = |picture: &TitlePicture, left: bool| {
        let mid = picture.origin[0] + picture.width as f32 / picture.scale / 2.0;
        let mut found = 0;
        for y in 0..picture.height {
            for x in 0..picture.width {
                let at = ((y * picture.width + x) * 4) as usize;
                let p = &picture.rgba[at..at + 4];
                let cx = picture.origin[0] + x as f32 / picture.scale;
                if p[3] > 200 && p[0] > 200 && p[1] < 40 && (cx < mid) == left {
                    found += 1;
                }
            }
        }
        found
    };
    let first = draw_title(&mut book, &karaoke(HighlightMode::Word, Some(0), 1.0), 1.0).unwrap();
    assert!(red(&first, true) > 300 && red(&first, false) == 0);
    let second = draw_title(&mut book, &karaoke(HighlightMode::Word, Some(1), 1.0), 1.0).unwrap();
    assert!(red(&second, true) == 0 && red(&second, false) > 300);
    // A fill keeps what was said lit.
    let filled = draw_title(&mut book, &karaoke(HighlightMode::Fill, Some(1), 1.0), 1.0).unwrap();
    assert!(red(&filled, true) > 300 && red(&filled, false) > 300);
    // A box lies behind the active word.
    let boxed = draw_title(&mut book, &karaoke(HighlightMode::Box, Some(0), 1.0), 1.0).unwrap();
    assert!(red(&boxed, true) > red(&first, true) * 2);
    // Words not yet said are hidden at an upcoming opacity of 0.
    let shown = draw_title(&mut book, &karaoke(HighlightMode::Word, Some(0), 0.0), 1.0).unwrap();
    let all = ink(&first);
    let part = ink(&shown);
    assert!(
        part.right < all.right - 100.0,
        "{} {}",
        part.right,
        all.right
    );
}

#[test]
fn a_fill_sweeps_the_active_word_from_its_left() {
    let mut book = book();
    let mut source = karaoke(HighlightMode::Fill, Some(0), 1.0);
    if let LayerSource::Text { words, .. } = &mut source {
        words.as_mut().unwrap().sweep = 0.5;
    }
    let picture = draw_title(&mut book, &source, 1.0).unwrap();
    let (mut red_right, mut white_right) = (f32::MIN, f32::MIN);
    for y in 0..picture.height {
        for x in 0..picture.width {
            let at = ((y * picture.width + x) * 4) as usize;
            let p = &picture.rgba[at..at + 4];
            let cx = picture.origin[0] + x as f32 / picture.scale;
            if p[3] > 200 && p[0] > 200 && p[1] < 40 {
                red_right = red_right.max(cx);
            }
            if p[3] > 200 && p[1] > 200 {
                white_right = white_right.max(cx);
            }
        }
    }
    let all = ink(&picture);
    assert!(red_right > all.left + 20.0, "part of the first word is lit");
    assert!(red_right < white_right, "and not all of it");
}

fn animated(kind: TextAnimationKind, unit: TextUnit, progress: f32) -> LayerSource {
    let mut source = title("Hello world", TextStyle::default());
    if let LayerSource::Text { enter, .. } = &mut source {
        *enter = Some(Motion {
            kind,
            unit,
            progress,
            leaving: false,
        });
    }
    source
}

#[test]
fn an_entrance_fades_moves_and_types_its_units() {
    let mut book = book();
    let rest =
        ink(&draw_title(&mut book, &title("Hello world", TextStyle::default()), 1.0).unwrap());
    // Halfway through a fade of the whole, the ink is half there.
    let faded = draw_title(
        &mut book,
        &animated(TextAnimationKind::Fade, TextUnit::Whole, 0.5),
        1.0,
    )
    .unwrap();
    let most = faded.rgba.chunks_exact(4).map(|p| p[3]).max().unwrap();
    assert!((120..=136).contains(&most), "{most}");
    // A slide up starts below and comes up.
    let sliding = ink(&draw_title(
        &mut book,
        &animated(TextAnimationKind::SlideUp, TextUnit::Whole, 0.3),
        1.0,
    )
    .unwrap());
    assert!(
        sliding.top > rest.top + 10.0,
        "{} {}",
        sliding.top,
        rest.top
    );
    // A typewriter a third of the way has typed about a third.
    let typed = ink(&draw_title(
        &mut book,
        &animated(TextAnimationKind::Typewriter, TextUnit::Letter, 0.3),
        1.0,
    )
    .unwrap());
    assert!(typed.right < rest.left + (rest.right - rest.left) * 0.5);
    assert!((typed.left - rest.left).abs() < 2.0);
    // Before it begins, nothing shows; once done, all of it.
    let none = draw_title(
        &mut book,
        &animated(TextAnimationKind::Pop, TextUnit::Word, 0.0),
        1.0,
    )
    .unwrap();
    assert_eq!(ink(&none).pixels, 0);
    let wiped = ink(&draw_title(
        &mut book,
        &animated(TextAnimationKind::Wipe, TextUnit::Whole, 0.5),
        1.0,
    )
    .unwrap());
    assert!(wiped.right < rest.right - 50.0 && (wiped.left - rest.left).abs() < 2.0);
}

#[test]
fn a_heavier_weight_than_the_family_has_is_emboldened_and_italic_slants() {
    let mut book = book();
    let face = |weight, italic| TextStyle {
        font: FontFace {
            family: "Geist".into(),
            weight,
            italic,
        },
        ..TextStyle::default()
    };
    let regular = ink(&draw_title(&mut book, &title("Heavy", face(400, false)), 1.0).unwrap());
    let semibold = ink(&draw_title(&mut book, &title("Heavy", face(600, false)), 1.0).unwrap());
    let black = ink(&draw_title(&mut book, &title("Heavy", face(900, false)), 1.0).unwrap());
    assert!(
        semibold.pixels > regular.pixels,
        "the semibold face is chosen"
    );
    assert!(
        black.pixels > semibold.pixels * 11 / 10,
        "{} {}",
        black.pixels,
        semibold.pixels
    );
    let picture = draw_title(&mut book, &title("l", face(600, true)), 1.0).unwrap();
    // The stem leans right: its top row starts right of its bottom row.
    let row_start = |row: u32| {
        (0..picture.width)
            .find(|x| picture.rgba[((row * picture.width + x) * 4 + 3) as usize] > 128)
    };
    let rows: Vec<u32> = (0..picture.height)
        .filter(|row| row_start(*row).is_some())
        .collect();
    let (top, bottom) = (rows[2], rows[rows.len() - 3]);
    assert!(row_start(top).unwrap() > row_start(bottom).unwrap() + 5);
}

#[test]
fn a_graphic_draws_its_elements_in_its_design_box() {
    let mut book = book();
    let bar = Element::new(
        "Bar",
        ElementKind::Rect {
            color: [0, 0, 200, 255],
            radius: 8.0,
            stroke: Some(ShapeStroke {
                color: [255, 255, 0, 255],
                width: 4.0,
            }),
        },
        [0.0, 0.0, 600.0, 120.0],
    );
    let name = Element::new(
        "Name",
        ElementKind::Text {
            text: "Ada Lovelace".into(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
            style: TextStyle::default(),
        },
        [20.0, 0.0, 560.0, 120.0],
    );
    let arrow = Element::new(
        "Arrow",
        ElementKind::Line {
            color: [0, 255, 0, 255],
            width: 6.0,
            arrow: true,
        },
        [0.0, 140.0, 300.0, 0.0],
    );
    let layer = |element| ElementLayer {
        element,
        enter: None,
        exit: None,
    };
    let source = LayerSource::Graphic {
        size: [600.0, 160.0],
        elements: vec![layer(bar), layer(name), layer(arrow)],
    };
    let picture = draw_title(&mut book, &source, 1.0).unwrap();
    assert_eq!(picture.bounds, [-300.0, -80.0, 300.0, 80.0]);
    // The bar's fill near its left top, inside its stroke.
    assert_eq!(pixel(&picture, [-280.0, -60.0]), [0, 0, 200, 255]);
    assert_eq!(pixel(&picture, [-300.0, -20.0])[0..2], [255, 255]);
    // The name in white over the bar's middle.
    let white = count(&picture, |[r, g, b, a]| {
        a > 250 && r > 240 && g > 240 && b > 240
    });
    assert!(white > 1000, "{white}");
    // The arrow along the bottom, its head at the right.
    assert_eq!(pixel(&picture, [-200.0, 60.0])[1], 255);
    assert_eq!(pixel(&picture, [-5.0, 60.0])[1], 255);
}

#[test]
fn a_graphic_element_enters_on_its_own() {
    let mut book = book();
    let bar = Element::new(
        "Bar",
        ElementKind::Rect {
            color: [0, 0, 200, 255],
            radius: 0.0,
            stroke: None,
        },
        [0.0, 0.0, 400.0, 100.0],
    );
    let source = |progress| LayerSource::Graphic {
        size: [400.0, 100.0],
        elements: vec![ElementLayer {
            element: bar.clone(),
            enter: Some(Motion {
                kind: TextAnimationKind::SlideLeft,
                unit: TextUnit::Whole,
                progress,
                leaving: false,
            }),
            exit: None,
        }],
    };
    let rest = ink(&draw_title(&mut book, &source(1.0), 1.0).unwrap());
    let coming = ink(&draw_title(&mut book, &source(0.4), 1.0).unwrap());
    // Sliding left, it comes from the right.
    assert!(
        coming.left > rest.left + 10.0,
        "{} {}",
        coming.left,
        rest.left
    );
}

#[test]
fn empty_text_draws_nothing_but_keeps_a_box() {
    let mut book = book();
    let picture = draw_title(&mut book, &title("   ", TextStyle::default()), 1.0).unwrap();
    assert_eq!(ink(&picture).pixels, 0);
    assert!(picture.bounds[3] - picture.bounds[1] > 100.0);
    let media = LayerSource::Media {
        asset_id: uuid::Uuid::new_v4(),
        source_time: editor_core::Time::ZERO,
        speed: 1.0,
    };
    assert!(draw_title(&mut book, &media, 1.0).is_none());
}

#[test]
fn a_family_nobody_has_is_drawn_in_geist() {
    let mut book = book();
    let style = TextStyle {
        font: FontFace {
            family: "No Such Family".into(),
            ..FontFace::default()
        },
        ..TextStyle::default()
    };
    let missing = draw_title(&mut book, &title("Same", style), 1.0).unwrap();
    let geist = draw_title(&mut book, &title("Same", TextStyle::default()), 1.0).unwrap();
    assert_eq!(missing.rgba, geist.rgba);
}

#[test]
fn project_and_system_faces_are_found_by_family() {
    let folder = tempfile::tempdir().unwrap();
    let nested = folder.path().join("a/b");
    std::fs::create_dir_all(&nested).unwrap();
    let path = nested.join("Geist-Regular.ttf");
    std::fs::write(&path, REGULAR).unwrap();
    std::fs::write(nested.join("broken.ttf"), b"not a font").unwrap();
    // A system of one font, found from its naming tables.
    let mut system = FontBook::new(&[]).with_system_roots(vec![folder.path().to_owned()]);
    assert_eq!(system.families(), vec!["Geist".to_owned()]);
    assert!(draw_title(&mut system, &title("Found", TextStyle::default()), 1.0).is_some());
    // A project's font.
    let mut project = FontBook::new(&[]).with_system_roots(Vec::new());
    assert!(project.families().is_empty());
    project.set_project_fonts(&[path.clone(), nested.join("broken.ttf")]);
    assert_eq!(project.families(), vec!["Geist".to_owned()]);
    let picture = draw_title(&mut project, &title("Found", TextStyle::default()), 1.0).unwrap();
    assert!(ink(&picture).pixels > 1000);
    assert_eq!(font_family(&path).unwrap(), "Geist");
    assert!(font_family(&nested.join("broken.ttf")).is_err());
    assert!(is_font_file(&path) && !is_font_file(&PathBuf::from("a.png")));
}

#[test]
fn an_emoji_is_drawn_in_colour_when_the_system_has_a_colour_face() {
    let roots: Vec<PathBuf> = [
        "/usr/share/fonts",
        "/System/Library/Fonts",
        "C:\\Windows\\Fonts",
    ]
    .iter()
    .map(PathBuf::from)
    .filter(|root| root.exists())
    .collect();
    let mut book = FontBook::new(&[REGULAR, SEMIBOLD]).with_system_roots(roots);
    let families = book.families();
    let colour = ["Noto Color Emoji", "Apple Color Emoji", "Segoe UI Emoji"]
        .iter()
        .any(|family| families.iter().any(|known| known == family));
    if !colour {
        eprintln!("SKIP: no colour emoji face on this system");
        return;
    }
    let picture = draw_title(&mut book, &title("😀", TextStyle::default()), 1.0).unwrap();
    // A yellow face, not white.
    let yellow = count(&picture, |[r, g, b, a]| {
        a > 200 && r > 180 && g > 120 && b < 90
    });
    assert!(yellow > 200, "{yellow}");
}

#[test]
fn the_lane_answers_the_newest_request_for_a_title() {
    let (sender, woken) = mpsc::channel();
    let sender = std::sync::Mutex::new(sender);
    let service = TextService::with_book(
        book,
        std::sync::Arc::new(move || {
            let _ = sender.lock().unwrap().send(());
        }),
    )
    .unwrap();
    let clip = uuid::Uuid::new_v4();
    let fonts: std::sync::Arc<[PathBuf]> = Vec::new().into();
    for serial in 1..=3 {
        service.request(TitleRequest {
            clip,
            serial,
            source: title(&format!("Draft {serial}"), TextStyle::default()),
            scale: 1.0,
            fonts: fonts.clone(),
        });
    }
    let mut newest = 0;
    while newest < 3 {
        woken
            .recv_timeout(Duration::from_secs(20))
            .expect("the lane answers");
        while let Some(event) = service.poll() {
            if let TextEvent::Drawn(drawn) = event {
                assert!(drawn.serial > newest, "answers come in order");
                assert_eq!(drawn.clip, clip);
                newest = drawn.serial;
            }
        }
    }
    service.want_families();
    let families = loop {
        woken.recv_timeout(Duration::from_secs(20)).unwrap();
        if let Some(TextEvent::Families(families)) = service.poll() {
            break families;
        }
    };
    assert!(families.iter().any(|family| family == "Geist"));
    assert!(!service.busy());
}
