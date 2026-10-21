//! The Text, Graphic and Animate sections: each gesture is one step that
//! changes the title's content and nothing else of it.
use super::*;
use crate::ui::PresetChange;
use editor_core::{
    ClipContent, Element, ElementKind, HighlightMode, TextAnimationKind, TextUnit, WordTime,
};

/// A bench with one title of four seconds selected, the playhead a second
/// into it, and the section `section` shown.
fn titled(section: Section) -> (Bench, ClipId) {
    let mut bench = Bench::empty();
    let title = bench.add(
        TrackKind::Text,
        Clip::text("Hello there", Time::ZERO, seconds(4.0)),
    );
    bench.selection.only(title);
    bench.playhead = seconds(1.0);
    bench.state.section = section;
    bench.frame(Vec::new());
    (bench, title)
}

fn text_of(bench: &Bench, clip: ClipId) -> (String, editor_core::TextStyle) {
    match &bench.clip(clip).content {
        ClipContent::Text { text, style, .. } => (text.clone(), style.clone()),
        other => panic!("not a title: {other:?}"),
    }
}

fn picked(bench: &mut Bench, menu: &str, option: &str) {
    bench.click(menu);
    bench.click(option);
}

#[test]
fn a_switched_look_is_one_step_with_its_rows() {
    let (mut bench, title) = titled(Section::Text);
    for row in [
        "Font",
        "Weight",
        "Italic",
        "Size",
        "Outline",
        "Shadow",
        "Background",
    ] {
        assert!(bench.has(row), "the Text section has {row}");
    }
    assert!(
        !bench.has("Width"),
        "an outline's rows wait for the outline"
    );
    let before = bench.steps();

    bench.click("Outline");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    let (_, style) = text_of(&bench, title);
    assert!(style.outline.is_some());
    assert!(bench.has("Width"), "and then it has a width");

    bench.click("Background");
    bench.click("Each line");
    assert_eq!(bench.steps(), before + 3, "{:?}", bench.refused);
    let background = text_of(&bench, title).1.background.expect("a background");
    assert_eq!(background.shape, editor_core::BackgroundShape::Lines);

    bench.click("Italic");
    picked(&mut bench, "Weight", "Black");
    let (text, style) = text_of(&bench, title);
    assert!(style.font.italic);
    assert_eq!(style.font.weight, 900);
    assert_eq!(text, "Hello there", "the words are as they were");
    assert_eq!(bench.steps(), before + 5);

    // Undone, it is as it was before the outline.
    for _ in 0..5 {
        assert!(bench.editor.undo(), "a step to undo");
    }
    assert!(text_of(&bench, title).1.is_plain());
}

#[test]
fn a_length_in_ems_is_shown_and_written_in_pixels() {
    let (mut bench, title) = titled(Section::Text);
    bench.click("Outline");
    let shown = bench.find("Width").and_then(|seen| seen.value.clone());
    // 0.06 em of a 64 px title.
    assert_eq!(shown.as_deref(), Some("3.8 px"), "{shown:?}");
    let before = bench.steps();
    bench.focus("Width");
    for character in "8".chars() {
        bench.frame(vec![Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }]);
        bench.frame(vec![Event::Text(character.to_string())]);
    }
    bench.enter(Modifiers::NONE);
    let outline = text_of(&bench, title).1.outline.expect("an outline");
    assert!((outline.width - 0.125).abs() < 1e-4, "{}", outline.width);
    assert_eq!(bench.steps(), before + 1);
}

#[test]
fn a_missing_family_is_said_and_a_listed_one_is_chosen() {
    let (mut bench, title) = titled(Section::Text);
    assert!(
        bench.state.take_families_wanted(),
        "a section that names fonts asks for them"
    );
    bench.families = Some(vec!["Geist".to_owned(), "Inter".to_owned()]);
    let before = bench.steps();
    picked(&mut bench, "Font", "Inter");
    assert_eq!(text_of(&bench, title).1.font.family, "Inter");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);

    bench.families = Some(vec!["Geist".to_owned()]);
    bench.frame(Vec::new());
    assert!(
        (bench.seen.iter()).any(|seen| seen.name.contains("“Inter” is not on this computer")),
        "a family this computer lacks is said"
    );
}

#[test]
fn an_entrance_takes_its_style_s_usual_order_and_fits_the_clip() {
    let (mut bench, title) = titled(Section::Animate);
    for row in ["In", "Out", "Highlight words"] {
        assert!(bench.has(row), "the Animate section has {row}");
    }
    let before = bench.steps();
    picked(&mut bench, "In", "Typewriter");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    let ClipContent::Text { enter, exit, .. } = &bench.clip(title).content else {
        panic!("a title");
    };
    let enter = enter.expect("an entrance");
    assert_eq!(enter.kind, TextAnimationKind::Typewriter);
    assert_eq!(enter.unit, TextUnit::Letter);
    assert_eq!(enter.duration, seconds(0.5));
    assert!(exit.is_none());

    picked(&mut bench, "Out", "Slide down");
    let ClipContent::Text { exit, .. } = &bench.clip(title).content else {
        panic!("a title");
    };
    assert_eq!(
        exit.map(|exit| exit.kind),
        Some(TextAnimationKind::SlideDown)
    );

    picked(&mut bench, "In", "None");
    let ClipContent::Text { enter, .. } = &bench.clip(title).content else {
        panic!("a title");
    };
    assert!(enter.is_none());
    assert_eq!(bench.steps(), before + 3);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn karaoke_is_offered_on_any_title_and_said_to_need_word_times() {
    let (mut bench, title) = titled(Section::Animate);
    assert!(
        (bench.seen.iter()).any(|seen| seen.name.contains("captions with word times")),
        "a title without word times says what lights its words"
    );
    bench.click("Highlight words");
    bench.click("Fill");
    let ClipContent::Text { style, .. } = &bench.clip(title).content else {
        panic!("a title");
    };
    let highlight = style.highlight.expect("a highlight");
    assert_eq!(highlight.mode, HighlightMode::Fill);

    // A caption with word times says nothing of them.
    bench.edit(Command::SetWords {
        clip_id: title,
        words: vec![
            WordTime {
                at: seconds(0.0),
                end: seconds(1.0),
            },
            WordTime {
                at: seconds(1.0),
                end: seconds(2.0),
            },
        ],
    });
    bench.frame(Vec::new());
    assert!(
        !(bench.seen.iter()).any(|seen| seen.name.contains("captions with word times")),
        "{:?}",
        bench.seen.iter().map(|seen| &seen.name).collect::<Vec<_>>()
    );
}

fn graphic(bench: &mut Bench) -> ClipId {
    let gold = [255, 196, 0, 255];
    let bar = Element::new(
        "Bar",
        ElementKind::Rect {
            color: gold,
            radius: 0.0,
            stroke: None,
        },
        [0.0, 0.0, 12.0, 100.0],
    );
    let name = Element::new(
        "Name",
        ElementKind::Text {
            text: "Ada Lovelace".into(),
            font_size: 40.0,
            color: [255; 4],
            style: Default::default(),
        },
        [20.0, 0.0, 400.0, 50.0],
    );
    let role = Element::new(
        "Role",
        ElementKind::Text {
            text: "Programmer".into(),
            font_size: 24.0,
            color: gold,
            style: Default::default(),
        },
        [20.0, 50.0, 400.0, 50.0],
    );
    let clip = Clip::graphic(
        "Lower third",
        [420.0, 100.0],
        vec![bar, name, role],
        Time::ZERO,
        seconds(4.0),
    );
    bench.add(TrackKind::Text, clip)
}

#[test]
fn a_graphic_offers_its_words_colours_and_one_font() {
    let mut bench = Bench::empty();
    let clip = graphic(&mut bench);
    bench.selection.only(clip);
    bench.state.section = Section::Graphic;
    bench.frame(Vec::new());
    for row in [
        "Graphic", "Animate", "Name", "Role", "Bar", "Font", "Weight",
    ] {
        assert!(bench.has(row), "a graphic has {row}");
    }
    assert!(
        !(bench.seen.iter()).any(|seen| seen.name == "Text" && seen.role == Role::RadioButton),
        "a graphic has no Text section of a title"
    );
    // Two colours: the bar's gold, which the role shares, and the name's white.
    let colours = (bench.seen.iter())
        .filter(|seen| seen.role == Role::ColorWell || seen.name == "Bar")
        .count();
    assert!(colours >= 1);

    let before = bench.steps();
    picked(&mut bench, "Weight", "Bold");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    let ClipContent::Graphic { elements, .. } = &bench.clip(clip).content else {
        panic!("a graphic");
    };
    let weights: Vec<u16> = (elements.iter())
        .filter_map(|element| match &element.kind {
            ElementKind::Text { style, .. } => Some(style.font.weight),
            _ => None,
        })
        .collect();
    assert_eq!(weights, vec![700, 700], "every text of it");
}

#[test]
fn a_graphic_s_parts_come_in_one_after_another() {
    let mut bench = Bench::empty();
    let clip = graphic(&mut bench);
    bench.selection.only(clip);
    bench.state.section = Section::Animate;
    bench.frame(Vec::new());
    let before = bench.steps();
    picked(&mut bench, "In", "Slide up");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    let delays = |bench: &Bench| -> Vec<Option<Time>> {
        let ClipContent::Graphic { elements, .. } = &bench.clip(clip).content else {
            panic!("a graphic");
        };
        (elements.iter())
            .map(|element| element.enter.map(|enter| enter.delay))
            .collect()
    };
    assert_eq!(delays(&bench), vec![Some(Time::ZERO); 3]);

    // A stagger: each part a tenth after the one before it.
    bench.focus("Between parts");
    bench.frame(vec![Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    for character in "0.1".chars() {
        bench.frame(vec![Event::Text(character.to_string())]);
    }
    bench.enter(Modifiers::NONE);
    let rate = bench.editor.project().settings.frame_rate;
    assert_eq!(
        delays(&bench),
        vec![
            Some(Time::ZERO),
            Some(rate.frame_time(3)),
            Some(rate.frame_time(6))
        ]
    );
    assert_eq!(bench.steps(), before + 2, "{:?}", bench.refused);
}

#[test]
fn a_title_typed_is_one_step_when_its_field_is_left() {
    let (mut bench, title) = titled(Section::Text);
    let field = (bench.seen.iter())
        .find(|seen| seen.value.as_deref() == Some("Hello there") && !Bench::is_text(seen))
        .map(|seen| seen.node)
        .expect("the title's words are in a field");
    bench.frame(vec![Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Focus,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: field,
            data: None,
        },
    )]);
    bench.frame(vec![Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    let before = bench.steps();
    for character in "Goodbye".chars() {
        bench.frame(vec![Event::Text(character.to_string())]);
    }
    assert_eq!(bench.steps(), before, "nothing is kept while it is typed");
    match bench.layer(title).source {
        editor_core::LayerSource::Text { text, .. } => {
            assert_eq!(text, "Goodbye", "the canvas draws what is typed");
        }
        other => panic!("a title layer: {other:?}"),
    }
    // Away from the field: a press on nothing.
    let away = Pos2::new(bench.size.x - 4.0, bench.size.y - 4.0);
    bench.press(away);
    bench.release(away);
    assert_eq!(text_of(&bench, title).0, "Goodbye");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
}

#[test]
fn a_look_is_kept_under_a_name_and_a_brand_colour_is_one_click() {
    let (mut bench, title) = titled(Section::Text);
    bench.brand = vec![[255, 0, 0, 255], [0, 0, 255, 255]];
    bench.click("Keep…");
    let named = bench
        .find("Preset name")
        .and_then(|seen| seen.value.clone());
    assert_eq!(
        named.as_deref(),
        Some("Hello there"),
        "its first words name it"
    );
    bench.click("Keep");
    let [
        PresetChange::Keep {
            name,
            content,
            position,
        },
    ] = bench.kept.as_slice()
    else {
        panic!("one look was kept: {:?}", bench.kept);
    };
    assert_eq!(name, "Hello there");
    assert_eq!(**content, bench.clip(title).content);
    assert_eq!(*position, Some(bench.clip(title).transform.position));

    let before = bench.steps();
    bench.click("Colour");
    assert!(
        bench.has("Brand colour #0000FF"),
        "the brand colours are offered"
    );
    bench.click("Brand colour #FF0000");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    let ClipContent::Text { color, .. } = &bench.clip(title).content else {
        panic!("a title");
    };
    assert_eq!(*color, [255, 0, 0, 255]);
}

#[test]
fn a_graphic_is_kept_as_a_template_by_its_name() {
    let mut bench = Bench::empty();
    let clip = graphic(&mut bench);
    bench.selection.only(clip);
    bench.state.section = Section::Graphic;
    bench.frame(Vec::new());
    bench.click("Keep…");
    bench.click("Keep");
    let [PresetChange::Keep { name, content, .. }] = bench.kept.as_slice() else {
        panic!("one template was kept: {:?}", bench.kept);
    };
    assert_eq!(name, "Lower third");
    assert_eq!(**content, bench.clip(clip).content);
}

#[test]
fn a_colour_well_opens_its_own_picker_and_edits_its_own_colour() {
    // Spec §5.4 colour well; AGENTS.md: an action keeps its stable target.
    // The text's colour and its outline's and shadow's are wells of one
    // name in three cards, and each opens and moves only its own.
    let (mut bench, title) = titled(Section::Text);
    bench.size = vec2(300.0, 2400.0);
    bench.click("Outline");
    bench.click("Shadow");
    bench.frame(Vec::new());
    let mut wells: Vec<Rect> = (bench.seen.iter())
        .filter(|seen| seen.name == "Colour" && seen.role == Role::ColorWell)
        .map(|seen| seen.rect)
        .collect();
    wells.sort_by(|a, b| a.top().total_cmp(&b.top()));
    assert!(
        wells.len() >= 3,
        "a well each for the text, outline and shadow"
    );
    let colours = |bench: &Bench| {
        let ClipContent::Text { color, style, .. } = &bench.clip(title).content else {
            panic!("not a title");
        };
        let outline = style.outline.as_ref().map(|outline| outline.color);
        let shadow = style.shadow.as_ref().map(|shadow| shadow.color);
        (*color, outline, shadow)
    };
    let (text, outline, shadow) = colours(&bench);
    let before = bench.steps();

    // The text's own well is the first of them.
    bench.press(wells[0].center());
    bench.release(wells[0].center());
    bench.focus("Saturation and brightness");
    bench.key(Key::ArrowDown, Modifiers::NONE);
    bench.frame(Vec::new());
    bench.commit();
    let (text_now, outline_now, shadow_now) = colours(&bench);
    assert_ne!(text_now, text, "the text's colour moved");
    assert_eq!(outline_now, outline, "the outline's did not");
    assert_eq!(shadow_now, shadow, "nor the shadow's");
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
}
