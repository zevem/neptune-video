//! The Captions tab: what its edits ask of the editor, the subtitle files
//! it reads and writes, and the tab driven as a person drives it.
//!
//! [`Bench`] paints the library with the Captions tab in view, in a real
//! egui pass, and ends each frame as the application does: an edit is
//! dispatched, a seek moves the playhead, and what the tab asks of the
//! application's dialogs is counted. A control is found by the name
//! accessibility is given for it.
use super::*;
use crate::speech::{CueLength, Inventory, ModelChoice, SpeechRequest, SpeechSettings, SpeechView};
use crate::{
    textures::Textures,
    theme::{Accent, Appearance},
    titles::Titles,
    ui::library::{self, LibraryInput, LibraryState, Tab},
};
use editor_core::{Editor, WordTime};
use eframe::egui::{Event, PointerButton, Pos2, accesskit::Role};
use std::path::PathBuf;

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn editor() -> Editor {
    Editor::new(Project::new("Talk")).expect("a new project is valid")
}

fn cue(start: f64, end: f64, text: &str) -> Cue {
    Cue {
        start: seconds(start),
        end: seconds(end),
        text: text.to_owned(),
        words: Vec::new(),
    }
}

/// A caption track in `language` holding `cues`, made as an import makes
/// it.
fn captioned(editor: &mut Editor, language: &str, cues: &[Cue]) -> TrackId {
    let (command, track, count) = imported(editor.project(), cues, language).unwrap();
    assert_eq!(count, cues.len());
    editor.dispatch(command).unwrap();
    track
}

fn clips(editor: &Editor, track: TrackId) -> Vec<Clip> {
    let track = (editor.project().tracks.iter())
        .find(|other| other.id == track)
        .expect("the track is there");
    track.clips.clone()
}

fn texts(editor: &Editor, track: TrackId) -> Vec<String> {
    (clips(editor, track).iter())
        .map(|clip| text_of(clip).to_owned())
        .collect()
}

fn times(editor: &Editor, track: TrackId) -> Vec<(Time, Time)> {
    (clips(editor, track).iter())
        .map(|clip| (clip.start, clip.end()))
        .collect()
}

fn words(clip: &Clip) -> Vec<WordTime> {
    match &clip.content {
        ClipContent::Text { words, .. } => words.clone(),
        _ => Vec::new(),
    }
}

/// A folder of the test's own, gone when it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "neptune-video-captions-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- What the tab asks of the editor -----------------------------------------

#[test]
fn a_caption_track_is_named_for_its_language_and_one_is_in_view() {
    let mut editor = editor();
    let none = Selection::default();
    assert!(caption_tracks(editor.project()).is_empty());
    assert_eq!(in_view(editor.project(), None, &none), None);

    let (command, english) = new_track(editor.project(), "en");
    editor.dispatch(command).unwrap();
    let (command, brazilian) = new_track(editor.project(), "pt-BR");
    editor.dispatch(command).unwrap();
    let (command, klingon) = new_track(editor.project(), "x-klingon");
    editor.dispatch(command).unwrap();
    assert_eq!(editor.history().count(), 3, "each track is one step");
    let names: Vec<(&str, Option<&str>)> = (caption_tracks(editor.project()).iter())
        .map(|track| (track.name.as_str(), track.language.as_deref()))
        .collect();
    assert_eq!(
        names,
        [
            ("English captions", Some("en")),
            ("Portuguese captions", Some("pt-BR")),
            ("Captions (x-klingon)", Some("x-klingon")),
        ],
        "the plain text track a project starts with is not a caption track"
    );

    // The one chosen while it is one, else that of a selected caption,
    // else the first.
    let project = editor.project();
    assert_eq!(in_view(project, Some(klingon), &none), Some(klingon));
    let plain = (project.tracks.iter())
        .find(|track| track.kind == TrackKind::Text && track.language.is_none())
        .unwrap()
        .id;
    assert_eq!(in_view(project, Some(plain), &none), Some(english));
    let line = captioned(&mut editor, "pt-BR", &[cue(1.0, 2.0, "Olá")]);
    let said = clips(&editor, line)[0].id;
    let mut selection = Selection::default();
    selection.only(said);
    assert_ne!(line, brazilian, "an import makes a track of its own");
    assert_eq!(in_view(editor.project(), None, &selection), Some(line));
    assert_eq!(
        in_view(editor.project(), Some(plain), &selection),
        Some(line)
    );
}

#[test]
fn a_new_language_renames_only_a_track_named_for_its_last() {
    let mut editor = editor();
    let (command, track) = new_track(editor.project(), "en");
    editor.dispatch(command).unwrap();
    assert!(
        relabel(editor.project(), track, "en").is_none(),
        "the language it has is no edit"
    );

    editor
        .dispatch(relabel(editor.project(), track, "es").unwrap())
        .unwrap();
    let named = |editor: &Editor| {
        let track = editor
            .project()
            .tracks
            .iter()
            .find(|t| t.id == track)
            .unwrap();
        (track.name.clone(), track.language.clone())
    };
    assert_eq!(
        named(&editor),
        ("Spanish captions".into(), Some("es".into()))
    );
    assert_eq!(
        editor.history().count(),
        2,
        "the language and its name are one step"
    );

    // A name the person gave stays.
    editor
        .dispatch(Command::SetTrackState {
            track_id: track,
            locked: None,
            hidden: None,
            muted: None,
            name: Some("Dub".into()),
            gain: None,
            solo: None,
        })
        .unwrap();
    editor
        .dispatch(relabel(editor.project(), track, "fr").unwrap())
        .unwrap();
    assert_eq!(named(&editor), ("Dub".into(), Some("fr".into())));
    assert!(editor.undo());
    assert_eq!(named(&editor), ("Dub".into(), Some("es".into())));
}

#[test]
fn a_second_caption_track_in_a_language_is_numbered() {
    let mut editor = editor();
    let first = captioned(&mut editor, "en", &[cue(0.0, 1.0, "Hi")]);
    let second = captioned(&mut editor, "en", &[cue(0.0, 1.0, "Hi")]);
    let (command, third) = new_track(editor.project(), "en");
    editor.dispatch(command).unwrap();
    let name = |editor: &Editor, id: TrackId| {
        (editor.project().tracks.iter())
            .find(|track| track.id == id)
            .unwrap()
            .name
            .clone()
    };
    assert_eq!(
        [first, second, third].map(|id| name(&editor, id)),
        [
            "English captions",
            "English captions 2",
            "English captions 3"
        ]
    );

    // A translation names a free track, and a numbered name still counts
    // as named for its language.
    let source = (editor.project().tracks.iter())
        .find(|track| track.id == first)
        .unwrap()
        .clone();
    let (prompt, _) = translation(editor.project(), &source, "es").unwrap();
    assert!(prompt.contains("name \"Spanish captions\""), "{prompt}");
    editor
        .dispatch(relabel(editor.project(), second, "es").unwrap())
        .unwrap();
    assert_eq!(name(&editor, second), "Spanish captions");
    let (prompt, _) = translation(editor.project(), &source, "es").unwrap();
    assert!(prompt.contains("name \"Spanish captions 2\""), "{prompt}");
    editor
        .dispatch(relabel(editor.project(), third, "es").unwrap())
        .unwrap();
    assert_eq!(name(&editor, third), "Spanish captions 2");

    // A track keeps its own name when its new language is called the same.
    editor
        .dispatch(relabel(editor.project(), first, "en-GB").unwrap())
        .unwrap();
    assert_eq!(name(&editor, first), "English captions");
    // A free number is taken again once its track is gone.
    editor
        .dispatch(Command::RemoveTrack { track_id: second })
        .unwrap();
    let (command, again) = new_track(editor.project(), "es");
    editor.dispatch(command).unwrap();
    assert_eq!(name(&editor, again), "Spanish captions");
}

#[test]
fn a_subtitle_file_becomes_one_caption_track_in_one_step() {
    let mut editor = editor();
    let source = "1\n00:00:01,000 --> 00:00:02,500\nHello there\n\n\
                  2\n00:00:03,000 --> 00:00:04,000\n<i>General</i> Kenobi\n\n\
                  3\n00:00:05,000 --> 00:00:05,001\nToo short to be seen\n";
    let cues = captions::parse(source, SubtitleFormat::Srt).unwrap();
    let tracks = editor.project().tracks.len();
    let (command, track, count) = imported(editor.project(), &cues, "en").unwrap();
    assert_eq!(count, 2, "a cue shorter than a frame is left out");
    editor.dispatch(command).unwrap();
    assert_eq!(editor.history().count(), 1);
    assert_eq!(editor.project().tracks.len(), tracks + 1);
    assert_eq!(texts(&editor, track), ["Hello there", "General Kenobi"]);
    assert_eq!(
        times(&editor, track),
        [(seconds(1.0), seconds(2.5)), (seconds(3.0), seconds(4.0))],
        "the file's times are kept as they are, to come back out the same"
    );
    // Each takes the caption look, scaled to the canvas.
    let look = text_tab::caption_look().clip(Time::ZERO, editor.project());
    for clip in clips(&editor, track) {
        let (
            ClipContent::Text {
                font_size, style, ..
            },
            ClipContent::Text {
                font_size: size,
                style: kind,
                ..
            },
        ) = (&clip.content, &look.content)
        else {
            panic!("a caption is a title");
        };
        assert_eq!((font_size, style), (size, kind));
        assert_eq!(clip.transform, look.transform);
    }
    assert!(editor.undo());
    assert_eq!(editor.project().tracks.len(), tracks);

    let none = imported(editor.project(), &[cue(1.0, 1.001, "Blink")], "en");
    assert!(none.is_err(), "a file with nothing to see is refused");
}

#[test]
fn a_caption_added_by_hand_fits_before_the_next_and_takes_its_neighbours_look() {
    let mut editor = editor();
    let track = captioned(
        &mut editor,
        "en",
        &[cue(0.0, 1.0, "First"), cue(4.0, 5.0, "Last")],
    );
    // The first is made larger, so that the look it gives is its own.
    let first = clips(&editor, track)[0].id;
    let ClipContent::Text { color, .. } = &clips(&editor, track)[0].content else {
        unreachable!()
    };
    editor
        .dispatch(Command::SetText {
            clip_id: first,
            text: "First".into(),
            font_size: 120.0,
            color: *color,
        })
        .unwrap();
    let steps = editor.history().count();

    let (command, added_id) = added(editor.project(), track, seconds(2.5)).unwrap();
    editor.dispatch(command).unwrap();
    assert_eq!(editor.history().count(), steps + 1);
    let (_, clip) = editor.project().clip(added_id).unwrap();
    assert_eq!(
        (clip.start, clip.end()),
        (seconds(2.5), seconds(4.0)),
        "it stops at the next"
    );
    assert_eq!(text_of(clip), NEW_TEXT);
    assert!(matches!(clip.content, ClipContent::Text { font_size, .. } if font_size == 120.0));

    let refused = added(editor.project(), track, seconds(4.5));
    assert!(refused.is_err(), "a caption is not added over another");
    let (command, _) = added(editor.project(), track, seconds(6.0)).unwrap();
    editor.dispatch(command).unwrap();
    let last = clips(&editor, track).last().unwrap().clone();
    assert_eq!((last.start, last.duration), (seconds(6.0), NEW_CUE));

    editor
        .dispatch(Command::SetTrackState {
            track_id: track,
            locked: Some(true),
            hidden: None,
            muted: None,
            name: None,
            gain: None,
            solo: None,
        })
        .unwrap();
    assert!(added(editor.project(), track, seconds(10.0)).is_err());
    assert!(retyped(editor.project(), first, "Changed").is_err());
}

#[test]
fn a_caption_is_retimed_split_and_merged_with_its_words_in_place() {
    let mut editor = editor();
    let mut spoken = cue(1.0, 3.0, "Hello there");
    spoken.words = vec![
        WordTime {
            at: seconds(1.0),
            end: seconds(2.0),
        },
        WordTime {
            at: seconds(2.0),
            end: seconds(3.0),
        },
    ];
    let track = captioned(
        &mut editor,
        "en",
        &[cue(0.0, 0.5, "Before"), spoken, cue(4.0, 5.0, "After")],
    );
    let id = clips(&editor, track)[1].id;
    let said = |editor: &Editor| {
        let (_, clip) = editor.project().clip(id).unwrap();
        (words(clip).iter())
            .map(|word| (clip.start + word.at, clip.start + word.end))
            .collect::<Vec<_>>()
    };
    let spoken_at = said(&editor);

    // An earlier start stops at the caption before, and the words stay
    // where they are said.
    editor
        .dispatch(retimed(editor.project(), id, Edge::Start, seconds(0.2)).unwrap())
        .unwrap();
    assert_eq!(times(&editor, track)[1], (seconds(0.5), seconds(3.0)));
    assert_eq!(said(&editor), spoken_at);
    // A later end stops at the next.
    editor
        .dispatch(retimed(editor.project(), id, Edge::End, seconds(9.0)).unwrap())
        .unwrap();
    assert_eq!(times(&editor, track)[1], (seconds(0.5), seconds(4.0)));
    assert!(
        retimed(editor.project(), id, Edge::End, seconds(9.0)).is_err(),
        "nothing to move is said, not done"
    );
    assert!(retimed(editor.project(), id, Edge::Start, seconds(4.5)).is_err());
    editor
        .dispatch(retimed(editor.project(), id, Edge::End, seconds(3.0)).unwrap())
        .unwrap();
    editor
        .dispatch(retimed(editor.project(), id, Edge::Start, seconds(1.0)).unwrap())
        .unwrap();
    assert_eq!(times(&editor, track)[1], (seconds(1.0), seconds(3.0)));
    let steps = editor.history().count();

    // Split between the words, each half keeps its word.
    assert!(
        split(editor.project(), id, seconds(1.0)).is_err(),
        "not on its edge"
    );
    editor
        .dispatch(split(editor.project(), id, seconds(2.0)).unwrap())
        .unwrap();
    assert_eq!(texts(&editor, track), ["Before", "Hello", "there", "After"]);
    assert_eq!(editor.history().count(), steps + 1);

    // And merged back, one caption with both words at their times.
    editor
        .dispatch(merged(editor.project(), id).unwrap())
        .unwrap();
    assert_eq!(texts(&editor, track), ["Before", "Hello there", "After"]);
    assert_eq!(times(&editor, track)[1], (seconds(1.0), seconds(3.0)));
    assert_eq!(said(&editor), spoken_at);
    assert_eq!(editor.history().count(), steps + 2, "a merge is one step");
    let last = clips(&editor, track)[2].id;
    assert!(
        merged(editor.project(), last).is_err(),
        "the last has none after it"
    );
    assert!(editor.undo());
    assert_eq!(texts(&editor, track), ["Before", "Hello", "there", "After"]);
}

#[test]
fn words_are_kept_as_typed_and_an_empty_caption_is_refused() {
    let mut editor = editor();
    let track = captioned(&mut editor, "en", &[cue(0.0, 1.0, "One")]);
    let id = clips(&editor, track)[0].id;
    assert!(retyped(editor.project(), id, "One").unwrap().is_none());
    assert!(
        retyped(editor.project(), id, "One\n").unwrap().is_none(),
        "a trailing line is not words"
    );
    assert!(retyped(editor.project(), id, "  \n ").is_err());
    editor
        .dispatch(retyped(editor.project(), id, "One\nTwo").unwrap().unwrap())
        .unwrap();
    assert_eq!(texts(&editor, track), ["One\nTwo"]);
}

#[test]
fn a_track_is_written_as_srt_or_vtt_and_read_back() {
    let scratch = Scratch::new("export");
    let mut editor = editor();
    let mut spoken = cue(1.0, 2.0, "Hello there");
    spoken.words = vec![
        WordTime {
            at: seconds(1.0),
            end: seconds(1.5),
        },
        WordTime {
            at: seconds(1.5),
            end: seconds(2.0),
        },
    ];
    let track = captioned(
        &mut editor,
        "pt-BR",
        &[spoken.clone(), cue(3.0, 4.0, "Tchau")],
    );
    let project = editor.project();
    assert_eq!(
        export_name(project, track, SubtitleFormat::Srt),
        "Talk.pt-BR.srt"
    );

    let (srt, count) = exported(project, track, SubtitleFormat::Srt).unwrap();
    assert_eq!(count, 2);
    let path = scratch
        .0
        .join(export_name(project, track, SubtitleFormat::Srt));
    write_file(&path, &srt).unwrap();
    let (read, language) = read_file(&path).unwrap();
    assert_eq!(language.as_deref(), Some("pt-BR"), "named by the file");
    assert_eq!(read.len(), 2);
    assert_eq!(
        (read[0].start, read[0].end, read[0].text.as_str()),
        (spoken.start, spoken.end, "Hello there")
    );

    let (vtt, _) = exported(project, track, SubtitleFormat::Vtt).unwrap();
    assert!(vtt.starts_with("WEBVTT\nLanguage: pt-BR\n"));
    let path = scratch.0.join("plain.vtt");
    write_file(&path, &vtt).unwrap();
    let (read, language) = read_file(&path).unwrap();
    assert_eq!(language.as_deref(), Some("pt-BR"), "named by its header");
    assert_eq!(
        read[0].words, spoken.words,
        "word times go out and come back"
    );
    let leftovers: Vec<_> = std::fs::read_dir(&scratch.0).unwrap().collect();
    assert_eq!(
        leftovers.len(),
        2,
        "nothing is left beside what was written"
    );

    let (command, empty) = new_track(editor.project(), "en");
    editor.dispatch(command).unwrap();
    assert!(exported(editor.project(), empty, SubtitleFormat::Srt).is_err());
}

#[test]
fn subtitle_files_are_read_in_their_encoding_and_whole_or_not_at_all() {
    let scratch = Scratch::new("read");
    let srt = "1\n00:00:01,000 --> 00:00:02,000\nCafé ñandú\n";
    let expected = "Café ñandú";
    let read =
        |path: &Path| read_file(path).map(|(cues, language)| (cues[0].text.clone(), language));

    let mut marked = vec![0xEF, 0xBB, 0xBF];
    marked.extend_from_slice(srt.as_bytes());
    let path = scratch.file("marked.srt", &marked);
    assert_eq!(read(&path).unwrap(), (expected.to_owned(), None));

    let mut wide = vec![0xFF, 0xFE];
    wide.extend(srt.encode_utf16().flat_map(u16::to_le_bytes));
    let path = scratch.file("wide.es.srt", &wide);
    assert_eq!(
        read(&path).unwrap(),
        (expected.to_owned(), Some("es".to_owned()))
    );

    let mut big = vec![0xFE, 0xFF];
    big.extend(srt.encode_utf16().flat_map(u16::to_be_bytes));
    let path = scratch.file("big.srt", &big);
    assert_eq!(read(&path).unwrap().0, expected);

    // Old SubRip files are Windows-1252.
    let old: Vec<u8> = (srt.chars())
        .map(|c| match c {
            'é' => 0xE9,
            'ñ' => 0xF1,
            'ú' => 0xFA,
            other => other as u8,
        })
        .collect();
    let path = scratch.file("old.final.srt", &old);
    assert_eq!(
        read(&path).unwrap(),
        (expected.to_owned(), None),
        "a part of the name that is no language is not one"
    );

    let path = scratch.file("notes.txt", srt.as_bytes());
    assert!(
        read_file(&path)
            .unwrap_err()
            .contains("not an .srt or .vtt")
    );
    let path = scratch.file("broken.srt", b"1\nnot a time\nWords\n");
    assert!(read_file(&path).unwrap_err().starts_with("broken.srt:"));
    let path = scratch.file("huge.srt", b"");
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(captions::MAX_SUBTITLE_BYTES as u64 + 1)
        .unwrap();
    assert!(read_file(&path).unwrap_err().contains("larger than"));
}

#[test]
fn times_read_as_minutes_seconds_and_hundredths() {
    assert_eq!(clock(Time::ZERO), "0:00.00");
    assert_eq!(clock(seconds(1.2)), "0:01.20");
    assert_eq!(clock(seconds(83.456)), "1:23.46");
    assert_eq!(clock(seconds(3723.4)), "1:02:03.40");
    let rate = editor_core::FrameRate::default();
    for value in [0.0, 1.2, 83.46, 3723.4] {
        let typed = timefmt::parse(&clock(seconds(value)), rate).unwrap();
        assert_eq!(typed, seconds(value), "{value} reads back as it is shown");
    }
}

// ---- The tab as a person drives it ---------------------------------------------

struct Seen {
    name: String,
    value: Option<String>,
    rect: Rect,
    role: Role,
    disabled: bool,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    textures: Textures,
    titles: Titles,
    editor: Editor,
    library: LibraryState,
    selection: Selection,
    presets: Arc<Presets>,
    playhead: Time,
    size: Vec2,
    time: f64,
    seen: Vec<Seen>,
    refused: Vec<String>,
    said: Vec<String>,
    imports: usize,
    exports: Vec<(TrackId, SubtitleFormat)>,
    /// The speech worker as the application shows it, and what the tab
    /// asked of it.
    speech: SpeechView,
    speech_settings: SpeechSettings,
    inventory: Option<Inventory>,
    asked: Vec<Asked>,
}

/// What the tab asked of the speech worker and its files.
#[derive(Debug, PartialEq)]
enum Asked {
    Generate(SpeechRequest),
    Cancel,
    Add,
    Discard,
    Pick,
    Look,
    Translate(TrackId, String),
}

impl Bench {
    fn new() -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut library = LibraryState::default();
        library.tab = Tab::Captions;
        let mut bench = Self {
            ctx,
            palette,
            textures: Textures::default(),
            titles: Titles::default(),
            editor: editor(),
            library,
            selection: Selection::default(),
            presets: Arc::default(),
            playhead: Time::ZERO,
            size: vec2(300.0, 900.0),
            time: 0.0,
            seen: Vec::new(),
            refused: Vec::new(),
            said: Vec::new(),
            imports: 0,
            exports: Vec::new(),
            speech: SpeechView::Idle,
            speech_settings: SpeechSettings::default(),
            inventory: None,
            asked: Vec::new(),
        };
        bench.frame(Vec::new());
        bench
    }

    fn steps(&self) -> usize {
        self.editor.history().count()
    }

    /// One frame of the application with these events in it.
    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let ctx = self.ctx.clone();
        let project = self.editor.snapshot();
        let mut actions: Vec<Action> = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        self.titles.begin_frame();
        let mut output = ctx.run_ui(input, |ui| {
            library::show(
                ui,
                ui.max_rect(),
                &mut self.library,
                &mut self.selection,
                &mut self.textures,
                &mut self.titles,
                &LibraryInput {
                    p: &self.palette,
                    project: &project,
                    media: None,
                    grid: true,
                    sort: Default::default(),
                    importing: 0,
                    problem: None,
                    previewing: None,
                    playhead: self.playhead,
                    presets: &self.presets,
                    speech: SpeechInput {
                        view: &self.speech,
                        settings: &self.speech_settings,
                        inventory: self.inventory.as_ref(),
                    },
                },
                &mut actions,
            );
        });
        output.textures_delta.clear();
        if let Some(update) = &output.platform_output.accesskit_update {
            self.seen = (update.nodes.iter())
                .filter_map(|(_, node)| {
                    let bounds = node.bounds()?;
                    Some(Seen {
                        name: node.label().or_else(|| node.value())?.to_owned(),
                        value: node.value().map(str::to_owned),
                        rect: Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                        role: node.role(),
                        disabled: node.is_disabled(),
                    })
                })
                .collect();
        }
        for action in actions {
            match action {
                Action::Edit(command) => {
                    if let Err(error) = self.editor.dispatch(command) {
                        self.refused.push(error.to_string());
                    }
                }
                Action::Notify(text) => self.said.push(text),
                Action::Seek(time) => self.playhead = time,
                Action::ImportCaptions => self.imports += 1,
                Action::ExportCaptions { track, format } => self.exports.push((track, format)),
                Action::SetSpeech(settings) => self.speech_settings = settings,
                Action::GenerateCaptions(request) => self.asked.push(Asked::Generate(request)),
                Action::CancelSpeech => self.asked.push(Asked::Cancel),
                Action::AddHeardCaptions => self.asked.push(Asked::Add),
                Action::DiscardHeardCaptions => self.asked.push(Asked::Discard),
                Action::PickSpeechModel => self.asked.push(Asked::Pick),
                Action::LookAtModels => self.asked.push(Asked::Look),
                Action::TranslateCaptions { track_id, language } => {
                    self.asked.push(Asked::Translate(track_id, language))
                }
                other => panic!("FIXTURE: the tab asked for {other:?}"),
            }
        }
    }

    fn find(&self, name: &str) -> Option<&Seen> {
        (self.seen.iter())
            .find(|seen| seen.name == name && !matches!(seen.role, Role::Label | Role::TextRun))
    }

    fn has(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    fn value(&self, name: &str) -> Option<String> {
        self.find(name).and_then(|seen| seen.value.clone())
    }

    fn says(&self, text: &str) -> bool {
        self.seen.iter().any(|seen| seen.name.contains(text))
    }

    fn at(&self, name: &str) -> Pos2 {
        match self.find(name) {
            Some(seen) => seen.rect.center(),
            None => {
                let names: Vec<&str> = self.seen.iter().map(|seen| seen.name.as_str()).collect();
                panic!("FIXTURE: no control is named \"{name}\" among {names:?}")
            }
        }
    }

    fn press(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn click_at(&mut self, pos: Pos2, button: PointerButton) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Self::press(pos, button, true)]);
        self.frame(vec![Self::press(pos, button, false)]);
        self.frame(Vec::new());
    }

    fn click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.click_at(pos, PointerButton::Primary);
    }

    fn secondary_click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.click_at(pos, PointerButton::Secondary);
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        self.frame(Vec::new());
    }

    /// Replaces what the focused field holds with `text`.
    fn retype(&mut self, text: &str) {
        self.key(Key::A, Modifiers::COMMAND);
        self.frame(vec![Event::Text(text.to_owned())]);
    }

    fn track(&self) -> TrackId {
        caption_tracks(self.editor.project())
            .first()
            .expect("FIXTURE: a caption track")
            .id
    }
}

#[test]
fn with_no_caption_track_the_tab_says_what_captions_are_and_starts_one() {
    let mut bench = Bench::new();
    assert!(bench.says("one track for each language"));
    assert!(!bench.has("Export captions"), "there is nothing to export");
    bench.click("Import SRT or VTT…");
    bench.click("Import SRT or VTT");
    assert_eq!(bench.imports, 2, "from the tab and from its header");

    bench.click("New caption track");
    assert!(bench.has("English") && bench.has("Language or tag"));
    bench.click("Spanish");
    assert_eq!(bench.steps(), 1);
    let track = bench.track();
    let made = (bench.editor.project().tracks.iter())
        .find(|t| t.id == track)
        .unwrap();
    assert_eq!(
        (made.name.as_str(), made.language.as_deref()),
        ("Spanish captions", Some("es"))
    );
    bench.frame(Vec::new());
    assert!(bench.has("Add caption") && bench.has("Caption language"));
    assert!(bench.says("No captions yet"));
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}

#[test]
fn a_caption_is_added_at_the_playhead_and_typed_into_the_list() {
    let mut bench = Bench::new();
    let (command, track) = new_track(bench.editor.project(), "en");
    bench.editor.dispatch(command).unwrap();
    bench.playhead = seconds(1.0);
    bench.click("Add caption");
    assert_eq!(bench.steps(), 2);
    assert_eq!(times(&bench.editor, track), [(seconds(1.0), seconds(3.0))]);
    let id = clips(&bench.editor, track)[0].id;
    assert_eq!(bench.selection.clips, [id], "the new caption is selected");

    // Its field takes the keyboard: what is typed replaces the words, and
    // Return keeps them as one step.
    bench.retype("Hello world");
    assert_eq!(bench.steps(), 2, "nothing is kept while typing");
    bench.key(Key::Enter, Modifiers::NONE);
    assert_eq!(texts(&bench.editor, track), ["Hello world"]);
    assert_eq!(bench.steps(), 3);
    bench.frame(Vec::new());
    assert_eq!(bench.value("Caption 1").as_deref(), Some("Hello world"));

    // Shift+Return starts a line; Escape gives up what was typed.
    bench.click("Caption 1");
    bench.retype("Two");
    bench.key(Key::Enter, Modifiers::SHIFT);
    bench.frame(vec![Event::Text("lines".into())]);
    bench.key(Key::Escape, Modifiers::NONE);
    assert_eq!(texts(&bench.editor, track), ["Hello world"]);
    assert_eq!(bench.steps(), 3);
    bench.click("Caption 1");
    bench.retype("Two");
    bench.key(Key::Enter, Modifiers::SHIFT);
    bench.frame(vec![Event::Text("lines".into())]);
    // Clicking elsewhere keeps the words too.
    let away = pos2(150.0, 860.0);
    bench.click_at(away, PointerButton::Primary);
    assert_eq!(texts(&bench.editor, track), ["Two\nlines"]);
    assert_eq!(bench.steps(), 4);

    // An emptied caption is not kept: deleting it is what is meant.
    bench.click("Caption 1");
    bench.retype(" ");
    bench.key(Key::Enter, Modifiers::NONE);
    assert_eq!(texts(&bench.editor, track), ["Two\nlines"]);
    assert!(
        bench
            .said
            .iter()
            .any(|said| said.contains("delete it instead"))
    );

    // A caption is not added over another.
    bench.playhead = seconds(2.0);
    bench.click("Add caption");
    assert!(
        bench
            .said
            .iter()
            .any(|said| said.contains("at the playhead already"))
    );
    assert_eq!(bench.steps(), 4);
}

#[test]
fn a_typed_time_moves_its_edge_and_stops_at_the_neighbours() {
    let mut bench = Bench::new();
    let track = captioned(
        &mut bench.editor,
        "en",
        &[cue(0.0, 2.0, "One"), cue(3.0, 5.0, "Two")],
    );
    bench.frame(Vec::new());
    assert_eq!(bench.value("End of caption 1"), None);
    assert!(bench.has("End of caption 1"));

    bench.click("End of caption 1");
    assert_eq!(bench.value("End of caption 1").as_deref(), Some("0:02.00"));
    bench.retype("2.5");
    bench.key(Key::Enter, Modifiers::NONE);
    assert_eq!(times(&bench.editor, track)[0], (seconds(0.0), seconds(2.5)));

    bench.click("End of caption 1");
    bench.retype("0:04.00");
    bench.key(Key::Enter, Modifiers::NONE);
    assert_eq!(
        times(&bench.editor, track)[0],
        (seconds(0.0), seconds(3.0)),
        "it stops at the next"
    );

    bench.click("Start of caption 2");
    bench.retype("soon");
    bench.key(Key::Enter, Modifiers::NONE);
    assert!(
        bench
            .said
            .iter()
            .any(|said| said == "“soon” is not a time.")
    );
    assert_eq!(bench.steps(), 3, "the import and two times");

    // Escape leaves the time as it was.
    bench.click("Start of caption 2");
    bench.retype("4");
    bench.key(Key::Escape, Modifiers::NONE);
    assert_eq!(times(&bench.editor, track)[1], (seconds(3.0), seconds(5.0)));
    assert!(bench.refused.is_empty());
}

#[test]
fn the_row_menu_splits_merges_retimes_and_deletes_a_caption() {
    let mut bench = Bench::new();
    let mut spoken = cue(0.0, 2.0, "Hello there");
    spoken.words = vec![
        WordTime {
            at: seconds(0.0),
            end: seconds(1.0),
        },
        WordTime {
            at: seconds(1.0),
            end: seconds(2.0),
        },
    ];
    let track = captioned(&mut bench.editor, "en", &[spoken]);
    bench.playhead = seconds(1.0);
    bench.secondary_click("Caption 1");
    bench.click("Split at playhead");
    assert_eq!(texts(&bench.editor, track), ["Hello", "there"]);

    bench.secondary_click("Start of caption 1");
    bench.click("Merge with next");
    assert_eq!(texts(&bench.editor, track), ["Hello there"]);
    assert_eq!(words(&clips(&bench.editor, track)[0]).len(), 2);

    bench.playhead = seconds(1.5);
    bench.secondary_click("Caption 1");
    bench.click("End at playhead");
    assert_eq!(times(&bench.editor, track), [(seconds(0.0), seconds(1.5))]);
    bench.playhead = seconds(0.5);
    bench.secondary_click("Caption 1");
    bench.click("Start at playhead");
    assert_eq!(times(&bench.editor, track), [(seconds(0.5), seconds(1.5))]);

    bench.secondary_click("Caption 1");
    bench.click("Delete caption");
    assert!(clips(&bench.editor, track).is_empty());
    assert_eq!(bench.steps(), 6, "the import and five edits");
    assert!(bench.refused.is_empty());
}

#[test]
fn the_tab_shows_the_selected_captions_track_and_gives_it_a_language() {
    let mut bench = Bench::new();
    let english = captioned(&mut bench.editor, "en", &[cue(0.0, 1.0, "Hi")]);
    let spanish = captioned(&mut bench.editor, "es", &[cue(0.0, 1.0, "Hola")]);
    bench.frame(Vec::new());
    assert_eq!(bench.value("Caption 1").as_deref(), Some("Hi"));
    bench.selection.only(clips(&bench.editor, spanish)[0].id);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert_eq!(bench.value("Caption 1").as_deref(), Some("Hola"));

    // The track menu goes back to the other.
    bench.click("Caption track");
    bench.click("English captions");
    assert_eq!(bench.value("Caption 1").as_deref(), Some("Hi"));

    bench.click("Caption language");
    bench.click("French");
    let named = (bench.editor.project().tracks.iter())
        .find(|t| t.id == english)
        .unwrap();
    assert_eq!(
        (named.name.as_str(), named.language.as_deref()),
        ("French captions", Some("fr"))
    );

    // A tag the list does not have is taken as typed.
    bench.click("Caption language");
    bench.frame(vec![Event::Text("pt_br".into())]);
    bench.click("Use “pt-BR”");
    let named = (bench.editor.project().tracks.iter())
        .find(|t| t.id == english)
        .unwrap();
    assert_eq!(named.language.as_deref(), Some("pt-BR"));
    assert_eq!(named.name, "Portuguese captions");
    // The menu checks it above the list, which has Portuguese at large.
    bench.click("Caption language");
    let portuguese = (bench.seen.iter())
        .filter(|seen| seen.name == "Portuguese")
        .count();
    assert_eq!(portuguese, 2, "pt-BR above the list and pt in it");
    bench.key(Key::Escape, Modifiers::NONE);
    assert!(!bench.has("Dutch"));

    // Hidden and shown again.
    bench.click("Hide captions");
    assert!((bench.editor.project().tracks.iter()).any(|t| t.id == english && t.hidden));
    bench.click("Show captions");
    assert!((bench.editor.project().tracks.iter()).any(|t| t.id == english && !t.hidden));
    assert!(bench.refused.is_empty());
}

#[test]
fn a_translation_asks_for_one_copy_with_a_text_for_each_caption() {
    let mut editor = editor();
    let track = captioned(
        &mut editor,
        "en",
        &[cue(0.0, 1.0, "Hi"), cue(1.0, 2.0, "Two\nlines")],
    );
    let source = (editor.project().tracks.iter())
        .find(|other| other.id == track)
        .unwrap()
        .clone();
    let (prompt, captions) = translation(editor.project(), &source, "es").unwrap();
    assert_eq!(
        captions,
        clips(&editor, track)
            .iter()
            .map(|clip| clip.id)
            .collect::<Vec<_>>()
    );
    for said in [
        "Translate the 2 captions of the track \"English captions\"",
        &format!("({track})"),
        "from English into Spanish",
        "one copy_captions command",
        &format!("from_track_id {track}"),
        "name \"Spanish captions\"",
        "language \"es\"",
        "line breaks",
    ] {
        assert!(prompt.contains(said), "{said} in {prompt}");
    }

    // What the model would send back is one step that keeps the timing.
    let copy = uuid::Uuid::new_v4();
    editor
        .dispatch(Command::CopyCaptions {
            from_track_id: track,
            track_id: copy,
            name: "Spanish captions".into(),
            language: "es".into(),
            texts: vec!["Hola".into(), "Dos\nlíneas".into()],
        })
        .unwrap();
    assert_eq!(texts(&editor, copy), ["Hola", "Dos\nlíneas"]);
    assert_eq!(times(&editor, copy), times(&editor, track));

    let refused = translation(editor.project(), &source, "EN").unwrap_err();
    assert!(refused.contains("already in English"), "{refused}");
    let (command, empty) = new_track(editor.project(), "de");
    editor.dispatch(command).unwrap();
    let empty = (editor.project().tracks.iter())
        .find(|other| other.id == empty)
        .unwrap();
    assert!(
        translation(editor.project(), empty, "es")
            .unwrap_err()
            .contains("no captions")
    );
    let mut plain = (**empty).clone();
    plain.language = None;
    assert!(translation(editor.project(), &plain, "es").is_err());
}

#[test]
fn the_track_menu_translates_into_a_language_chosen_or_typed() {
    let mut bench = Bench::new();
    let english = captioned(&mut bench.editor, "en", &[cue(0.0, 1.0, "Hi")]);
    bench.frame(Vec::new());
    bench.click("Caption track");
    bench.click("Translate into…");
    bench.click("Spanish");
    assert_eq!(bench.asked, [Asked::Translate(english, "es".into())]);
    assert_eq!(
        bench.steps(),
        1,
        "nothing is edited until the copy is reviewed"
    );
    // The list is as tall in the menu it replaces as in a menu of its own.
    bench.click("Caption track");
    bench.click("Translate into…");
    bench.click("German");
    assert_eq!(
        bench.asked.last(),
        Some(&Asked::Translate(english, "de".into()))
    );

    bench.click("Caption track");
    bench.click("Translate into…");
    bench.frame(vec![Event::Text("pt_br".into())]);
    bench.click("Use “pt-BR”");
    assert_eq!(
        bench.asked.last(),
        Some(&Asked::Translate(english, "pt-BR".into()))
    );

    // An empty track has nothing to translate.
    bench.click("Caption track");
    bench.click("New caption track…");
    bench.click("German");
    assert_eq!(
        bench.value("Caption track").as_deref(),
        Some("German captions")
    );
    bench.click("Caption track");
    assert!(
        bench
            .find("Translate into…")
            .is_some_and(|seen| seen.disabled)
    );
}

#[test]
fn the_style_menu_gives_every_caption_of_the_track_one_look() {
    let mut bench = Bench::new();
    let track = captioned(
        &mut bench.editor,
        "en",
        &[cue(0.0, 1.0, "One"), cue(2.0, 3.0, "Two")],
    );
    let other = (text_tab::builtin().iter())
        .filter(|look| look.group == Group::Captions)
        .nth(1)
        .expect("FIXTURE: a second caption look")
        .clone();
    bench.click("Caption style");
    bench.click(&other.name);
    assert_eq!(bench.steps(), 2, "every caption takes it in one step");
    let made = other.clip(Time::ZERO, bench.editor.project());
    let ClipContent::Text { style: wanted, .. } = &made.content else {
        unreachable!()
    };
    for clip in clips(&bench.editor, track) {
        assert!(matches!(&clip.content, ClipContent::Text { style, .. } if style == wanted));
    }
    assert_eq!(
        texts(&bench.editor, track),
        ["One", "Two"],
        "the words are kept"
    );
}

#[test]
fn the_header_exports_the_track_in_view_and_imports_files() {
    let mut bench = Bench::new();
    let track = captioned(&mut bench.editor, "en", &[cue(0.0, 1.0, "One")]);
    bench.click("Export captions");
    bench.click("WebVTT (.vtt)…");
    bench.click("Export captions");
    bench.click("SubRip (.srt)…");
    assert_eq!(
        bench.exports,
        [(track, SubtitleFormat::Vtt), (track, SubtitleFormat::Srt)]
    );
    bench.click("Import SRT or VTT");
    assert_eq!(bench.imports, 1);
}

#[test]
fn the_list_draws_the_rows_in_view_and_follows_the_playhead() {
    let mut bench = Bench::new();
    let many: Vec<Cue> = (0..2000)
        .map(|index| {
            cue(
                index as f64,
                index as f64 + 0.5,
                &format!("Line {}", index + 1),
            )
        })
        .collect();
    let track = captioned(&mut bench.editor, "en", &many);
    bench.frame(Vec::new());
    let drawn = (bench.seen.iter())
        .filter(|seen| seen.name.starts_with("Caption ") && seen.role != Role::Label)
        .count();
    assert!((5..40).contains(&drawn), "{drawn} rows are drawn of 2000");
    assert!(bench.says("2000 captions"));

    bench.playhead = seconds(1500.2);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert_eq!(bench.value("Caption 1501").as_deref(), Some("Line 1501"));
    assert!(!bench.has("Caption 1"));

    // Searching lists what says it.
    bench.library.browser.search = "Line 77".into();
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(bench.says("11 of 2000"), "77 and 770 to 779");
    assert_eq!(bench.value("Caption 77").as_deref(), Some("Line 77"));
    bench.library.browser.search = "nothing like it".into();
    bench.frame(Vec::new());
    assert!(bench.says("No caption says"));
    assert_eq!(clips(&bench.editor, track).len(), 2000);
}

// ---- Captions from speech ------------------------------------------------------

/// A bench whose timeline holds ten seconds of sound from 2 s.
fn spoken() -> (Bench, ClipId) {
    let mut bench = Bench::new();
    let mut project = Project::new("Talk");
    let asset = editor_core::Asset::new(
        PathBuf::from("talk.wav"),
        editor_core::MediaKind::Audio,
        seconds(10.0),
    );
    Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
    let mut voice = Track::new("Voice", TrackKind::Audio);
    let clip = Clip::from_asset(&asset, seconds(2.0));
    let id = clip.id;
    voice.clips.push(clip);
    project.tracks.push(Arc::new(voice));
    bench.editor = Editor::new(project).unwrap();
    bench.frame(Vec::new());
    (bench, id)
}

fn generated(bench: &Bench) -> Vec<&SpeechRequest> {
    (bench.asked.iter())
        .filter_map(|asked| match asked {
            Asked::Generate(request) => Some(request),
            _ => None,
        })
        .collect()
}

#[test]
fn captions_from_speech_are_asked_for_with_what_the_person_chose() {
    let (mut bench, clip) = spoken();
    bench.click("Generate from speech");
    assert_eq!(bench.asked, [Asked::Look], "the models folder is looked at");
    for control in [
        "Speech model",
        "Spoken language",
        "Captions from",
        "Caption length",
        "Translate to English",
        "Download and generate",
    ] {
        assert!(bench.has(control), "{control}");
    }
    assert!(bench.says("Nothing is uploaded"));
    assert!(bench.says("Downloads 60 MB once"));

    // A model that is there needs no download.
    bench.inventory = Some(Inventory {
        installed: vec!["base"],
        detector: true,
    });
    bench.frame(Vec::new());
    assert!(bench.has("Generate captions") && !bench.has("Download and generate"));
    assert!(bench.says("On this computer"));

    bench.click("Speech model");
    assert!(bench.has("Tiny") && bench.has("Large v3 Turbo"));
    bench.click("Small");
    assert_eq!(bench.speech_settings.model, "small");
    assert!(bench.has("Download and generate"));
    bench.click("Spoken language");
    assert!(bench.has("Detect from speech"));
    bench.click("Spanish");
    assert_eq!(bench.speech_settings.language.as_deref(), Some("es"));
    bench.key(Key::Escape, Modifiers::NONE);
    bench.click("Caption length");
    bench.click("One word");
    assert_eq!(bench.speech_settings.length, CueLength::Words);
    bench.click("Translate to English");
    assert!(bench.speech_settings.translate);

    // Selected clips with none selected cannot be heard.
    bench.click("Captions from");
    bench.click("Selected clips");
    assert!(bench.says("Select the clips to caption."));
    bench.click("Download and generate");
    assert!(generated(&bench).is_empty());
    bench.selection.only(clip);
    bench.click("Download and generate");
    assert_eq!(
        generated(&bench),
        [&SpeechRequest {
            start: seconds(2.0),
            end: seconds(12.0),
            model: ModelChoice::Catalog(editor_media::speech::Model::by_id("small").unwrap()),
            language: Some("es".to_owned()),
            translate: true,
            length: CueLength::Words,
        }]
    );
    assert!(!bench.library.captions.speech_open, "the panel gives way");
    assert!(bench.refused.is_empty() && bench.said.is_empty());

    // The header opens it again, and a model file can be chosen.
    bench.click("Captions from speech");
    assert!(bench.has("Speech model"));
    bench.click("Speech model");
    bench.click("Model file…");
    assert_eq!(bench.asked.last(), Some(&Asked::Pick));
    bench.speech_settings.file = Some(PathBuf::from("/models/ggml-mine.bin"));
    bench.frame(Vec::new());
    assert_eq!(
        bench.value("Speech model").as_deref(),
        Some("ggml-mine.bin")
    );
    assert!(bench.has("Generate captions"));
    bench.click("Close");
    assert!(!bench.has("Speech model"));
}

#[test]
fn captions_being_made_show_how_far_they_are_and_stop_when_asked() {
    let (mut bench, _) = spoken();
    bench.speech = SpeechView::Working {
        stage: None,
        done: 0.0,
        stopping: false,
    };
    bench.frame(Vec::new());
    assert!(bench.says("Starting…"));
    bench.speech = SpeechView::Working {
        stage: Some(editor_media::speech::Stage::Recognizing),
        done: 0.42,
        stopping: false,
    };
    bench.frame(Vec::new());
    assert!(bench.says("Writing what is said…") && bench.says("42 %"));
    assert!(bench.has("Captions progress"));
    // What is there stays usable below it.
    assert!(bench.has("Generate from speech"));
    bench.click("Stop making captions");
    assert_eq!(bench.asked, [Asked::Cancel]);

    // Another cannot start while one runs.
    bench.click("Generate from speech");
    bench.click("Download and generate");
    assert!(generated(&bench).is_empty());
    assert!(bench.says("Captions are being made."));

    bench.speech = SpeechView::Working {
        stage: Some(editor_media::speech::Stage::Recognizing),
        done: 0.5,
        stopping: true,
    };
    bench.frame(Vec::new());
    assert!(bench.says("Stopping…") && !bench.says("50 %"));
}

#[test]
fn captions_that_came_while_the_timeline_changed_wait_to_be_added() {
    let (mut bench, _) = spoken();
    bench.speech = SpeechView::Ready {
        count: 12,
        language: "pt-BR".to_owned(),
        why: "The timeline changed while they were made, so some may be off.".to_owned(),
    };
    bench.frame(Vec::new());
    assert!(bench.says("Captions are ready"));
    assert!(bench.says("12 captions in Portuguese (pt-BR). The timeline changed"));
    bench.click("Add captions");
    bench.click("Discard");
    assert_eq!(bench.asked, [Asked::Add, Asked::Discard]);
    assert_eq!(bench.steps(), 0, "the application makes the edit");
}

#[test]
fn the_speech_panel_fits_a_narrow_library() {
    let (mut bench, _) = spoken();
    bench.size = vec2(220.0, 640.0);
    bench.click("Generate from speech");
    bench.frame(Vec::new());
    let right = bench.size.x;
    for control in [
        "Speech model",
        "Spoken language",
        "Captions from",
        "Caption length",
        "Download and generate",
    ] {
        let seen = bench.find(control).unwrap();
        assert!(seen.rect.right() <= right, "{control} at {:?}", seen.rect);
    }
}

#[test]
fn a_short_library_scrolls_the_speech_panel_to_its_button() {
    let (mut bench, _) = spoken();
    bench.size = vec2(240.0, 360.0);
    bench.click("Generate from speech");
    bench.frame(Vec::new());
    let below = bench.find("Download and generate").unwrap().rect;
    assert!(
        below.top() > bench.size.y,
        "the button starts out of sight at {below:?}"
    );

    bench.frame(vec![Event::PointerMoved(pos2(120.0, 240.0))]);
    bench.frame(vec![Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -600.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    }]);
    for _ in 0..12 {
        bench.frame(Vec::new());
    }
    let shown = bench.find("Download and generate").unwrap().rect;
    assert!(shown.bottom() <= bench.size.y, "scrolled to {shown:?}");
    bench.click("Download and generate");
    assert!(
        matches!(bench.asked.last(), Some(Asked::Generate(_))),
        "{:?}",
        bench.asked
    );
}
