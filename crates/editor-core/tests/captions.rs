use editor_core::{
    Clip, TextAnimation, TextAnimationKind, TextUnit, Time, WordTime,
    captions::{self, Chunking, Cue, SpokenWord, SubtitleFormat},
    title,
};

fn ms(value: i64) -> Time {
    Time(value * 1000)
}

const SRT: &str = "\u{feff}1\r\n00:00:01,000 --> 00:00:03,500\r\n<i>Hello</i> there,\r\n{\\an8}friend\r\n\r\n2\r\n00:00:04,000 --> 00:00:05,250 X1:10\r\nSecond\r\n";

#[test]
fn srt_reads_times_and_plain_text() {
    let cues = captions::parse(SRT, SubtitleFormat::Srt).unwrap();
    assert_eq!(cues.len(), 2);
    assert_eq!((cues[0].start, cues[0].end), (ms(1000), ms(3500)));
    assert_eq!(cues[0].text, "Hello there,\nfriend");
    assert_eq!(cues[1].text, "Second");
    assert_eq!(cues[1].end, ms(5250));
    assert!(cues.iter().all(|cue| cue.words.is_empty()));
}

#[test]
fn srt_round_trips() {
    let cues = captions::parse(SRT, SubtitleFormat::Srt).unwrap();
    let written = captions::format_srt(&cues);
    assert!(written.starts_with("1\n00:00:01,000 --> 00:00:03,500\nHello there,\nfriend\n\n2\n"));
    assert_eq!(
        captions::parse(&written, SubtitleFormat::Srt).unwrap(),
        cues
    );
}

#[test]
fn malformed_files_are_refused_whole() {
    for (source, line) in [
        ("1\n00:00:01,000 -> 00:00:02,000\nA\n", 2),
        ("1\n00:00:02,000 --> 00:00:01,000\nA\n", 2),
        (
            "1\n00:00:01,000 --> 00:00:02,000\nA\n\n2\n00:61:00,000 --> 00:62:00,000\nB\n",
            6,
        ),
        ("1\n00:00:01,00 --> 00:00:02,000\nA\n", 2),
        ("", 1),
    ] {
        let error = captions::parse(source, SubtitleFormat::Srt).unwrap_err();
        assert_eq!(error.line, line, "{source:?}: {error}");
    }
    for source in [
        "WEBVTT\n00:01.000 --> 00:02.000\nA\n",
        "VTT\n\n00:01.000 --> 00:02.000\nA\n",
        "WEBVTT\n\n00:01,000 --> 00:02.000\nA\n",
        "WEBVTT\n\nidentifier\nnot times\n",
    ] {
        assert!(
            captions::parse(source, SubtitleFormat::Vtt).is_err(),
            "{source:?}"
        );
    }
    let huge = "x".repeat(captions::MAX_SUBTITLE_BYTES + 1);
    assert!(captions::parse(&huge, SubtitleFormat::Srt).is_err());
}

#[test]
fn vtt_reads_short_times_skips_notes_and_takes_inline_word_times() {
    let source = "WEBVTT - a talk\nKind: captions\n\nNOTE written by hand\n--> not a cue\n\nSTYLE\n::cue { color: lime }\n\nintro\n00:01.000 --> 00:04.000 align:start line:90%\n<v Ada>Hello <00:00:02.000>big <00:00:03.000>world &amp; you</v>\n\n01:00:00.000 --> 01:00:01.000\nOne &lt;tag&gt;\n";
    let cues = captions::parse(source, SubtitleFormat::Vtt).unwrap();
    assert_eq!(cues.len(), 2);
    assert_eq!(cues[0].text, "Hello big world & you");
    assert_eq!((cues[0].start, cues[0].end), (ms(1000), ms(4000)));
    let starts: Vec<_> = cues[0].words.iter().map(|word| word.at).collect();
    assert_eq!(starts[..3], [ms(1000), ms(2000), ms(3000)]);
    assert_eq!(cues[0].words.len(), 5);
    assert_eq!(cues[0].words[4].end, ms(4000));
    assert_eq!(cues[1].text, "One <tag>");
    assert_eq!(cues[1].start, ms(3_600_000));
}

#[test]
fn vtt_round_trips_word_times() {
    let cue = Cue {
        start: ms(500),
        end: ms(2500),
        text: "Tom & Jerry\n\nrun -->".into(),
        words: vec![
            WordTime {
                at: ms(500),
                end: ms(900),
            },
            WordTime {
                at: ms(900),
                end: ms(1100),
            },
            WordTime {
                at: ms(1100),
                end: ms(1600),
            },
            WordTime {
                at: ms(1700),
                end: ms(2000),
            },
            WordTime {
                at: ms(2000),
                end: ms(2500),
            },
        ],
    };
    let written = captions::format_vtt(std::slice::from_ref(&cue), Some("en"));
    assert!(written.starts_with("WEBVTT\nLanguage: en\n\n00:00:00.500 --> 00:00:02.500\nTom <00:00:00.900>&amp; <00:00:01.100>Jerry\n<00:00:01.700>run <00:00:02.000>--&gt;\n\n"), "{written}");
    let read = captions::parse(&written, SubtitleFormat::Vtt).unwrap();
    assert_eq!(read[0].text, "Tom & Jerry\nrun -->");
    let starts: Vec<_> = read[0].words.iter().map(|word| word.at).collect();
    let expected: Vec<_> = cue.words.iter().map(|word| word.at).collect();
    assert_eq!(starts, expected);
}

#[test]
fn overlapping_cues_are_settled() {
    let source = "1\n00:00:01,000 --> 00:00:05,000\nA\n\n2\n00:00:03,000 --> 00:00:06,000\nB\n\n3\n00:00:03,000 --> 00:00:04,000\nC\n\n4\n00:00:00,500 --> 00:00:00,900\nD\n";
    let cues = captions::parse(source, SubtitleFormat::Srt).unwrap();
    let shape: Vec<_> = cues
        .iter()
        .map(|cue| (cue.start, cue.end, cue.text.as_str()))
        .collect();
    assert_eq!(
        shape,
        [
            (ms(500), ms(900), "D"),
            (ms(1000), ms(3000), "A"),
            (ms(3000), ms(6000), "B\nC"),
        ]
    );
}

fn spoken(words: &[(&str, i64, i64)]) -> Vec<SpokenWord> {
    words
        .iter()
        .map(|&(text, at, end)| SpokenWord {
            text: text.into(),
            at: ms(at),
            end: ms(end),
        })
        .collect()
}

#[test]
fn chunking_breaks_on_sentences_pauses_and_size() {
    let words = spoken(&[
        (" Hello", 0, 300),
        (" there.", 300, 600),
        (" This", 700, 900),
        (" is", 900, 1000),
        (" a", 1000, 1100),
        (" test", 1100, 1400),
        (" after", 2500, 2800),
        (" a", 2800, 2900),
        (" pause", 2900, 3300),
    ]);
    let cues = captions::cues(&words, &Chunking::LINES);
    let texts: Vec<_> = cues.iter().map(|cue| cue.text.as_str()).collect();
    assert_eq!(texts, ["Hello there.", "This is a test", "after a pause"]);
    assert_eq!(cues[0].start, ms(0));
    // A cue lingers, but never into the next one.
    assert_eq!(cues[0].end, ms(700));
    assert_eq!(cues[1].end, ms(1800));
    for cue in &cues {
        assert_eq!(cue.words.len(), title::word_count(&cue.text));
        assert!(title::words_valid(
            &cue.words
                .iter()
                .map(|word| WordTime {
                    at: word.at - cue.start,
                    end: word.end - cue.start
                })
                .collect::<Vec<_>>(),
            &cue.text,
            cue.end - cue.start
        ));
    }
    let short = captions::cues(&words, &Chunking::PHRASES);
    assert!(short.iter().all(|cue| cue.words.len() <= 4));
    let single = captions::cues(&words, &Chunking::WORDS);
    assert_eq!(single.len(), 9);
}

#[test]
fn word_pieces_join_and_unspaced_scripts_split() {
    let words = spoken(&[
        (" Un", 0, 200),
        ("believ", 200, 400),
        ("able", 400, 600),
        (" 你好", 1000, 1400),
        ("世界", 1400, 1800),
    ]);
    let cues = captions::cues(&words, &Chunking::LINES);
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].text, "Unbelievable 你好世界");
    assert_eq!(
        cues[0].words,
        [
            WordTime {
                at: ms(0),
                end: ms(600)
            },
            WordTime {
                at: ms(1000),
                end: ms(1200)
            },
            WordTime {
                at: ms(1200),
                end: ms(1400)
            },
            WordTime {
                at: ms(1400),
                end: ms(1600)
            },
            WordTime {
                at: ms(1600),
                end: ms(1800)
            },
        ]
    );
    // A cue never breaks inside a written word.
    let one_each = captions::cues(&words[..3], &Chunking::WORDS);
    assert_eq!(one_each.len(), 1);
}

#[test]
fn cues_become_titles_and_titles_become_cues() {
    let mut look = Clip::text("", Time::ZERO, ms(1000));
    if let editor_core::ClipContent::Text { enter, exit, .. } = &mut look.content {
        *enter = Some(TextAnimation {
            kind: TextAnimationKind::Pop,
            unit: TextUnit::Word,
            duration: ms(800),
            delay: Time::ZERO,
        });
        *exit = Some(TextAnimation {
            kind: TextAnimationKind::Fade,
            unit: TextUnit::Whole,
            duration: ms(800),
            delay: Time::ZERO,
        });
    }
    let frame = ms(33);
    let cue = Cue {
        start: ms(2000),
        end: ms(3000),
        text: "two words".into(),
        words: vec![
            WordTime {
                at: ms(2000),
                end: ms(2400),
            },
            WordTime {
                at: ms(2500),
                end: ms(3000),
            },
        ],
    };
    let clip = captions::cue_clip(&cue, &look, frame).unwrap();
    assert_eq!((clip.start, clip.duration), (ms(2000), ms(1000)));
    let editor_core::ClipContent::Text {
        text,
        words,
        enter,
        exit,
        ..
    } = &clip.content
    else {
        panic!("not a title");
    };
    assert_eq!(text, "two words");
    assert_eq!(
        words[1],
        WordTime {
            at: ms(500),
            end: ms(1000)
        }
    );
    // Animations are held inside the cue.
    assert!(title::animations_fit(
        enter.as_ref(),
        exit.as_ref(),
        clip.duration
    ));
    assert!(
        captions::cue_clip(
            &Cue {
                end: ms(2010),
                ..cue.clone()
            },
            &look,
            frame
        )
        .is_none()
    );

    let mut track = editor_core::Track::new("Captions", editor_core::TrackKind::Text);
    track.clips.push(clip);
    assert_eq!(captions::track_cues(&track), [cue]);
}
