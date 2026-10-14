use editor_core::{
    title::{self, Motion},
    *,
};
use std::sync::Arc;
use uuid::Uuid;

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn words(times: &[(f64, f64)]) -> Vec<WordTime> {
    times
        .iter()
        .map(|&(at, end)| WordTime {
            at: seconds(at),
            end: seconds(end),
        })
        .collect()
}

fn animation(kind: TextAnimationKind, duration: f64) -> TextAnimation {
    TextAnimation {
        kind,
        unit: kind.usual_unit(),
        duration: seconds(duration),
        delay: Time::ZERO,
    }
}

fn text_track(editor: &Editor) -> TrackId {
    editor
        .project()
        .tracks
        .iter()
        .find(|track| track.kind == TrackKind::Text)
        .unwrap()
        .id
}

/// An editor with one four-second title on the text track.
fn titled(text: &str) -> (Editor, TrackId, ClipId) {
    let mut editor = Editor::default();
    let track = text_track(&editor);
    let clip = Clip::text(text, Time::ZERO, seconds(4.0));
    let id = clip.id;
    editor
        .dispatch(Command::AddClip {
            track_id: track,
            clip,
        })
        .unwrap();
    (editor, track, id)
}

fn content(editor: &Editor, id: ClipId) -> ClipContent {
    editor.project().clip(id).unwrap().1.content.clone()
}

fn text_of(content: &ClipContent) -> (&str, &[WordTime]) {
    match content {
        ClipContent::Text { text, words, .. } => (text, words),
        _ => panic!("not a title"),
    }
}

#[test]
fn a_plain_title_serializes_as_before_and_old_titles_load() {
    let clip = Clip::text("Hello", Time::ZERO, seconds(2.0));
    let json = serde_json::to_value(&clip.content).unwrap();
    let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["color", "font_size", "text", "type"]);
    let old: ClipContent = serde_json::from_str(
        r#"{"type":"text","text":"Hi","font_size":48.0,"color":[255,0,0,255]}"#,
    )
    .unwrap();
    let ClipContent::Text {
        style,
        enter,
        exit,
        words,
        ..
    } = old
    else {
        panic!("not a title");
    };
    assert!(style.is_plain() && enter.is_none() && exit.is_none() && words.is_empty());
    assert_eq!(style.font.family, "Geist");
    assert_eq!(style.font.weight, 600);
}

#[test]
fn style_bounds_and_nan_are_refused_without_changing_the_project() {
    let (mut editor, _, id) = titled("Hello");
    let before = editor.snapshot();
    let mut bad = Vec::new();
    let changes: [fn(&mut TextStyle); 5] = [
        |style: &mut TextStyle| style.line_height = f32::NAN,
        |style: &mut TextStyle| style.tracking = 9.0,
        |style: &mut TextStyle| style.font.weight = 950,
        |style: &mut TextStyle| style.font.family = " Geist".into(),
        |style: &mut TextStyle| {
            style.outline = Some(TextOutline {
                color: [0; 4],
                width: f32::INFINITY,
            })
        },
    ];
    for change in changes {
        let mut style = TextStyle::default();
        change(&mut style);
        bad.push(style);
    }
    for style in bad {
        assert!(
            editor
                .dispatch(Command::SetTextStyle { clip_id: id, style })
                .is_err()
        );
        assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    }
    let style = TextStyle {
        outline: Some(TextOutline {
            color: [0, 0, 0, 255],
            width: 0.08,
        }),
        ..TextStyle::default()
    };
    editor
        .dispatch(Command::SetTextStyle {
            clip_id: id,
            style: style.clone(),
        })
        .unwrap();
    match content(&editor, id) {
        ClipContent::Text { style: stored, .. } => assert_eq!(stored, style),
        _ => panic!("not a title"),
    }
    assert!(editor.undo());
    assert!(Arc::ptr_eq(&before, &editor.snapshot()) || *before == *editor.snapshot());
}

#[test]
fn animations_must_fit_their_clip_and_reverse() {
    let (mut editor, _, id) = titled("Hello");
    let set = |edge, animation| Command::SetTextAnimation {
        clip_id: id,
        edge,
        animation,
    };
    // Shorter than a frame, longer than the clip, or meeting the entrance.
    assert!(
        editor
            .dispatch(set(
                TransitionEdge::In,
                Some(animation(TextAnimationKind::Fade, 0.001))
            ))
            .is_err()
    );
    assert!(
        editor
            .dispatch(set(
                TransitionEdge::In,
                Some(animation(TextAnimationKind::Fade, 5.0))
            ))
            .is_err()
    );
    editor
        .dispatch(set(
            TransitionEdge::In,
            Some(animation(TextAnimationKind::Pop, 2.5)),
        ))
        .unwrap();
    assert!(
        editor
            .dispatch(set(
                TransitionEdge::Out,
                Some(animation(TextAnimationKind::Fade, 2.0))
            ))
            .is_err()
    );
    editor
        .dispatch(set(
            TransitionEdge::Out,
            Some(animation(TextAnimationKind::SlideUp, 1.0)),
        ))
        .unwrap();
    assert_eq!(editor.undo_label(), Some("Animate text"));
    editor.dispatch(set(TransitionEdge::In, None)).unwrap();
    assert_eq!(editor.undo_label(), Some("Remove text animation"));
    match content(&editor, id) {
        ClipContent::Text { enter, exit, .. } => {
            assert!(enter.is_none());
            assert_eq!(exit.unwrap().kind, TextAnimationKind::SlideUp);
        }
        _ => panic!("not a title"),
    }
}

#[test]
fn word_times_follow_the_word_rule() {
    assert_eq!(title::word_count("Hello,  big world!"), 3);
    assert_eq!(title::word_count("  "), 0);
    assert_eq!(title::word_count("你好世界"), 4);
    assert_eq!(title::word_count("你好。"), 2);
    assert_eq!(title::word_count("カタカナ ok"), 5);
    let (mut editor, _, id) = titled("one two three");
    let good = words(&[(0.0, 0.5), (0.5, 1.0), (1.2, 2.0)]);
    for bad in [
        words(&[(0.0, 0.5), (0.5, 1.0)]),
        words(&[(0.0, 0.5), (1.5, 1.0), (1.2, 2.0)]),
        words(&[(1.0, 1.5), (0.5, 1.0), (1.2, 2.0)]),
        words(&[(0.0, 0.5), (0.5, 1.0), (1.2, 4.5)]),
    ] {
        assert!(
            editor
                .dispatch(Command::SetWords {
                    clip_id: id,
                    words: bad
                })
                .is_err()
        );
    }
    editor
        .dispatch(Command::SetWords {
            clip_id: id,
            words: good.clone(),
        })
        .unwrap();
    assert_eq!(text_of(&content(&editor, id)).1, good);
}

#[test]
fn editing_text_keeps_the_times_of_words_it_keeps() {
    let (mut editor, _, id) = titled("one two three four");
    let timed = words(&[(0.0, 0.4), (0.5, 0.9), (1.0, 1.4), (1.5, 2.0)]);
    editor
        .dispatch(Command::SetWords {
            clip_id: id,
            words: timed.clone(),
        })
        .unwrap();
    let edit = |editor: &mut Editor, text: &str| {
        editor
            .dispatch(Command::SetText {
                clip_id: id,
                text: text.into(),
                font_size: 64.0,
                color: [255; 4],
            })
            .unwrap();
        text_of(&content(editor, id)).1.to_vec()
    };
    // A word fixed in place takes its time; one taken out takes its time away.
    let fixed = edit(&mut editor, "one too three four");
    assert_eq!(fixed, timed);
    let shorter = edit(&mut editor, "one three four");
    assert_eq!(shorter, [timed[0], timed[2], timed[3]]);
    // A word put in is spread between its neighbours.
    let longer = edit(&mut editor, "one three and four");
    assert_eq!(longer.len(), 4);
    assert_eq!(longer[0], timed[0]);
    assert_eq!(longer[1], timed[2]);
    assert_eq!(longer[3], timed[3]);
    assert!(longer[2].at >= timed[2].end && longer[2].end <= timed[3].at);
    // Case and punctuation around a word keep it.
    let cased = edit(&mut editor, "One three, and four!");
    assert_eq!(cased[0], timed[0]);
    assert_eq!(cased[3], timed[3]);
    // Clearing the text clears the times; style and size stay.
    assert!(edit(&mut editor, "  ").is_empty());
    assert_eq!(editor.project().clip(id).unwrap().1.name, "  ");
}

#[test]
fn realignment_is_bounded_and_monotonic() {
    let text: String = (0..4000).map(|i| format!("w{i} ")).collect();
    let timed: Vec<_> = (0..4000)
        .map(|i| WordTime {
            at: Time(i * 1000),
            end: Time(i * 1000 + 900),
        })
        .collect();
    let replaced: String = (0..4000).map(|i| format!("x{i} ")).collect();
    let started = std::time::Instant::now();
    let moved = title::realign(&text, &timed, &replaced, Time(4_000_000));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(moved.len(), 4000);
    assert!(moved.windows(2).all(|pair| pair[0].at <= pair[1].at));
    assert!(title::words_valid(&moved, &replaced, Time(4_000_000)));
}

#[test]
fn split_parts_a_timed_title_by_word_and_its_animations_by_edge() {
    let (mut editor, track, id) = titled("one two three four");
    editor
        .dispatch(Command::Batch {
            label: "Time".into(),
            commands: vec![
                Command::SetWords {
                    clip_id: id,
                    words: words(&[(0.0, 0.4), (0.5, 0.9), (2.1, 2.5), (2.6, 3.0)]),
                },
                Command::SetTextAnimation {
                    clip_id: id,
                    edge: TransitionEdge::In,
                    animation: Some(animation(TextAnimationKind::Pop, 0.5)),
                },
                Command::SetTextAnimation {
                    clip_id: id,
                    edge: TransitionEdge::Out,
                    animation: Some(animation(TextAnimationKind::Fade, 0.5)),
                },
            ],
        })
        .unwrap();
    editor
        .dispatch(Command::SplitClip {
            clip_id: id,
            at: seconds(2.0),
        })
        .unwrap();
    let clips = &editor.project().track(track).unwrap().clips;
    assert_eq!(clips.len(), 2);
    let (left, right) = (&clips[0], &clips[1]);
    assert_eq!(
        text_of(&left.content),
        ("one two", &words(&[(0.0, 0.4), (0.5, 0.9)])[..])
    );
    assert_eq!(
        text_of(&right.content),
        ("three four", &words(&[(0.1, 0.5), (0.6, 1.0)])[..])
    );
    assert_eq!(left.name, "one two");
    let animations = |clip: &Clip| match &clip.content {
        ClipContent::Text { enter, exit, .. } => (enter.is_some(), exit.is_some()),
        _ => panic!("not a title"),
    };
    assert_eq!(animations(left), (true, false));
    assert_eq!(animations(right), (false, true));
    editor.project().validate().unwrap();
}

#[test]
fn split_before_the_first_word_keeps_the_title_whole() {
    let (mut editor, track, id) = titled("one two");
    editor
        .dispatch(Command::SetWords {
            clip_id: id,
            words: words(&[(2.0, 2.5), (2.5, 3.0)]),
        })
        .unwrap();
    editor
        .dispatch(Command::SplitClip {
            clip_id: id,
            at: seconds(1.0),
        })
        .unwrap();
    let clips = &editor.project().track(track).unwrap().clips;
    // Every word is held inside the half that has none of its own.
    assert_eq!(
        text_of(&clips[0].content),
        ("one two", &words(&[(1.0, 1.0), (1.0, 1.0)])[..])
    );
    assert_eq!(
        text_of(&clips[1].content),
        ("one two", &words(&[(1.0, 1.5), (1.5, 2.0)])[..])
    );
}

#[test]
fn trim_and_rate_rebase_words_and_animations() {
    let (mut editor, _, id) = titled("one two");
    editor
        .dispatch(Command::Batch {
            label: "Time".into(),
            commands: vec![
                Command::SetWords {
                    clip_id: id,
                    words: words(&[(1.0, 2.0), (2.0, 3.0)]),
                },
                Command::SetTextAnimation {
                    clip_id: id,
                    edge: TransitionEdge::In,
                    animation: Some(animation(TextAnimationKind::Fade, 1.0)),
                },
                Command::SetTextAnimation {
                    clip_id: id,
                    edge: TransitionEdge::Out,
                    animation: Some(animation(TextAnimationKind::Fade, 1.0)),
                },
            ],
        })
        .unwrap();
    // Trimming the head moves the words with the text.
    editor
        .dispatch(Command::TrimClip {
            clip_id: id,
            source_in: seconds(0.5),
            duration: seconds(3.5),
            ripple: false,
        })
        .unwrap();
    let clip = editor.project().clip(id).unwrap().1.clone();
    assert_eq!(text_of(&clip.content).1, words(&[(0.5, 1.5), (1.5, 2.5)]));
    // Trimming short holds both animations inside the clip.
    editor
        .dispatch(Command::TrimClip {
            clip_id: id,
            source_in: seconds(0.5),
            duration: seconds(1.0),
            ripple: false,
        })
        .unwrap();
    let clip = editor.project().clip(id).unwrap().1.clone();
    let ClipContent::Text {
        enter,
        exit,
        words: held,
        ..
    } = &clip.content
    else {
        panic!("not a title");
    };
    assert!(held.iter().all(|word| word.end <= seconds(1.0)));
    let span = enter.map_or(Time::ZERO, |a| a.span()) + exit.map_or(Time::ZERO, |a| a.span());
    assert!(span <= seconds(1.0));
    assert!(enter.is_some());
    editor.project().validate().unwrap();
    assert!(editor.undo());
    // Playing twice as fast halves word times and animations.
    editor
        .dispatch(Command::SetPlaybackRate {
            clip_id: id,
            rate: 2.0,
        })
        .unwrap();
    let clip = editor.project().clip(id).unwrap().1.clone();
    assert_eq!(clip.duration, seconds(1.75));
    let ClipContent::Text {
        enter, words: fast, ..
    } = &clip.content
    else {
        panic!("not a title");
    };
    assert_eq!(*fast, words(&[(0.25, 0.75), (0.75, 1.25)]));
    assert_eq!(enter.unwrap().duration, seconds(0.5));
}

#[test]
fn evaluation_steps_motion_once_a_frame() {
    let (mut editor, _, id) = titled("one two three");
    editor
        .dispatch(Command::SetTextAnimation {
            clip_id: id,
            edge: TransitionEdge::In,
            animation: Some(animation(TextAnimationKind::Pop, 1.0)),
        })
        .unwrap();
    let motion = |at: f64| {
        let plan = resolve(editor.project(), seconds(at), &[]);
        match &plan.layers[0].source {
            LayerSource::Text { enter, exit, .. } => (*enter, *exit),
            _ => panic!("not a title"),
        }
    };
    let (enter, exit) = motion(0.5);
    let Some(Motion {
        progress, leaving, ..
    }) = enter
    else {
        panic!("no entrance");
    };
    assert!(!leaving && exit.is_none());
    assert!((progress - 0.5).abs() < 0.02, "{progress}");
    // Within one frame the plan is the same.
    assert_eq!(motion(0.5001), motion(0.5));
    assert!(motion(1.5).0.is_none());
}

#[test]
fn unit_spread_staggers_and_typewriter_appears() {
    let motion = |kind, progress| Motion {
        kind,
        unit: TextUnit::Letter,
        progress,
        leaving: false,
    };
    let fade = motion(TextAnimationKind::Fade, 0.5);
    let first = title::unit_progress(&fade, 0, 4);
    let last = title::unit_progress(&fade, 3, 4);
    assert!(first > last);
    assert_eq!(
        title::unit_progress(&motion(TextAnimationKind::Fade, 1.0), 3, 4),
        1.0
    );
    assert_eq!(
        title::unit_progress(&motion(TextAnimationKind::Fade, 0.0), 0, 4),
        0.0
    );
    let typed = motion(TextAnimationKind::Typewriter, 0.5);
    assert_eq!(title::unit_progress(&typed, 1, 4), 1.0);
    assert_eq!(title::unit_progress(&typed, 2, 4), 0.0);
    // Whole text is one unit.
    assert_eq!(title::unit_progress(&fade, 0, 1), 0.5);
}

#[test]
fn karaoke_state_tracks_the_spoken_word() {
    let timed = words(&[(0.0, 0.5), (0.5, 1.0), (1.5, 2.0)]);
    let highlight = Highlight {
        mode: HighlightMode::Fill,
        color: [255, 220, 0, 255],
        scale: 1.2,
        upcoming: 0.5,
    };
    let at = |time| title::word_state(&timed, &highlight, seconds(time));
    assert_eq!(at(-0.1).active, None);
    let middle = at(0.75);
    assert_eq!(middle.active, Some(1));
    assert_eq!(middle.spoken, 1);
    assert!((middle.sweep - 0.5).abs() < 1e-3);
    assert_eq!(middle.grown, 1.0);
    // Between words the last one said stays active, swept.
    let pause = at(1.2);
    assert_eq!((pause.active, pause.spoken, pause.sweep), (Some(1), 2, 1.0));
    let boxed = Highlight {
        mode: HighlightMode::Box,
        scale: 1.0,
        ..highlight
    };
    let state = title::word_state(&timed, &boxed, seconds(0.75));
    assert_eq!((state.sweep, state.grown), (1.0, 1.0));
}

#[test]
fn graphics_validate_their_elements() {
    let (mut editor, track, _) = titled("x");
    let element = |kind| Element::new("Box", kind, [0.0, 0.0, 200.0, 100.0]);
    let good = vec![
        element(ElementKind::Rect {
            color: [20, 20, 20, 255],
            radius: 8.0,
            stroke: None,
        }),
        element(ElementKind::Text {
            text: "Name".into(),
            font_size: 40.0,
            color: [255; 4],
            style: TextStyle::default(),
        }),
    ];
    let graphic = Clip::graphic(
        "Lower third",
        [800.0, 200.0],
        good.clone(),
        seconds(5.0),
        seconds(3.0),
    );
    let graphic_id = graphic.id;
    editor
        .dispatch(Command::AddClip {
            track_id: track,
            clip: graphic,
        })
        .unwrap();
    let before = editor.snapshot();
    let mut nan = good.clone();
    nan[0].frame[2] = f32::NAN;
    let mut twins = good.clone();
    twins[1].id = twins[0].id;
    let many = vec![good[0].clone(); 33]
        .into_iter()
        .map(|mut e| {
            e.id = Uuid::new_v4();
            e
        })
        .collect();
    for elements in [nan, twins, many] {
        assert!(
            editor
                .dispatch(Command::SetGraphic {
                    clip_id: graphic_id,
                    size: [800.0, 200.0],
                    elements,
                })
                .is_err()
        );
        assert!(Arc::ptr_eq(&before, &editor.snapshot()));
    }
    assert!(
        editor
            .dispatch(Command::SetGraphic {
                clip_id: graphic_id,
                size: [0.0, 200.0],
                elements: good.clone(),
            })
            .is_err()
    );
    // Graphics refuse looks a title refuses.
    let mut look = editor.project().clip(graphic_id).unwrap().1.clone();
    look.blend = BlendMode::Multiply;
    let mut project = editor.project().clone();
    let slot = Arc::make_mut(project.tracks.iter_mut().find(|t| t.id == track).unwrap());
    *slot.clips.iter_mut().find(|c| c.id == graphic_id).unwrap() = look;
    assert!(project.validate().is_err());
    // Shared colours are what a template's colour parameters change.
    assert_eq!(title::graphic_colors(&good), [[20, 20, 20, 255], [255; 4]]);
    let plan = resolve(editor.project(), seconds(6.0), &[]);
    assert!(matches!(
        &plan.layers[0].source,
        LayerSource::Graphic { size, elements } if *size == [800.0, 200.0] && elements.len() == 2
    ));
}

#[test]
fn caption_languages_and_translated_copies() {
    let (mut editor, track, id) = titled("hello world");
    editor
        .dispatch(Command::SetWords {
            clip_id: id,
            words: words(&[(0.5, 1.0), (1.0, 2.0)]),
        })
        .unwrap();
    for bad in ["", "1en", "en us", "x".repeat(36).as_str()] {
        assert!(
            editor
                .dispatch(Command::SetTrackLanguage {
                    track_id: track,
                    language: Some(bad.into()),
                })
                .is_err(),
            "{bad:?}"
        );
    }
    let video = editor.project().tracks[0].id;
    assert!(
        editor
            .dispatch(Command::SetTrackLanguage {
                track_id: video,
                language: Some("en".into()),
            })
            .is_err()
    );
    editor
        .dispatch(Command::SetTrackLanguage {
            track_id: track,
            language: Some("en".into()),
        })
        .unwrap();
    let copy = Uuid::new_v4();
    assert!(
        editor
            .dispatch(Command::CopyCaptions {
                from_track_id: track,
                track_id: copy,
                name: "Español".into(),
                language: "es".into(),
                texts: vec![],
            })
            .is_err()
    );
    editor
        .dispatch(Command::CopyCaptions {
            from_track_id: track,
            track_id: copy,
            name: "Español".into(),
            language: "es".into(),
            texts: vec!["hola a todos".into()],
        })
        .unwrap();
    let project = editor.project();
    let index = project.tracks.iter().position(|t| t.id == copy).unwrap();
    let source = project.tracks.iter().position(|t| t.id == track).unwrap();
    assert_eq!(index, source + 1);
    assert!(project.tracks[source].hidden);
    let copied = &project.tracks[index];
    assert_eq!(copied.language.as_deref(), Some("es"));
    assert!(!copied.hidden);
    let (text, timed) = text_of(&copied.clips[0].content);
    assert_eq!(text, "hola a todos");
    assert_eq!(timed.len(), 3);
    assert_eq!(timed[0].at, seconds(0.5));
    assert_eq!(timed[2].end, seconds(2.0));
    assert_ne!(copied.clips[0].id, id);
    assert!(editor.undo());
    assert!(editor.project().track(copy).is_none());
}

#[test]
fn font_assets_are_resources_not_clips() {
    let mut editor = Editor::default();
    let font = Asset::new("/fonts/Brand.ttf".into(), MediaKind::Font, Time(1));
    let video = editor.project().tracks[0].id;
    editor
        .dispatch(Command::AddAssets {
            assets: vec![font.clone()],
        })
        .unwrap();
    assert!(
        editor
            .dispatch(Command::AddClip {
                track_id: video,
                clip: Clip::from_asset(&font, Time::ZERO),
            })
            .is_err()
    );
}

#[test]
fn a_drafted_title_is_what_the_plan_draws_and_media_keeps_its_own() {
    let (mut editor, _, id) = titled("Stored");
    let mut drafted = content(&editor, id);
    if let ClipContent::Text { text, style, .. } = &mut drafted {
        *text = "Typed".into();
        style.outline = Some(TextOutline {
            color: [0, 0, 0, 255],
            width: 0.05,
        });
    }
    let draft = ClipOverride {
        clip: id,
        content: Some(drafted),
        ..ClipOverride::default()
    };
    let plan = resolve(editor.project(), seconds(1.0), std::slice::from_ref(&draft));
    let LayerSource::Text { text, style, .. } = &plan.layers[0].source else {
        panic!("a title layer");
    };
    assert_eq!(text, "Typed");
    assert!(style.outline.is_some());
    // The draft is not the project.
    assert_eq!(text_of(&content(&editor, id)).0, "Stored");

    // A graphic drafted over a media clip changes nothing it draws.
    let mut asset = Asset::new("/media/a.mp4".into(), MediaKind::Video, seconds(10.0));
    asset.width = 1920;
    asset.height = 1080;
    let video = (editor.project().tracks.iter())
        .find(|track| track.kind == TrackKind::Video)
        .unwrap()
        .id;
    editor
        .dispatch(Command::AddAssets {
            assets: vec![asset.clone()],
        })
        .unwrap();
    let clip = Clip::from_asset(&asset, Time::ZERO);
    let media = clip.id;
    editor
        .dispatch(Command::AddClip {
            track_id: video,
            clip,
        })
        .unwrap();
    let draft = ClipOverride {
        clip: media,
        content: Some(ClipContent::Graphic {
            size: [100.0, 100.0],
            elements: Vec::new(),
        }),
        ..ClipOverride::default()
    };
    let plan = resolve(editor.project(), seconds(1.0), std::slice::from_ref(&draft));
    let layer = plan
        .layers
        .iter()
        .find(|layer| layer.clip == media)
        .unwrap();
    assert!(matches!(layer.source, LayerSource::Media { .. }));
}
