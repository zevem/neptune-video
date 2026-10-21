//! The Text tab: what its looks make, where they go, and the tab driven as
//! a person drives it.
//!
//! [`Bench`] paints the library with the Text tab in view beside the
//! timeline, in a real egui pass, and ends each frame as the application
//! does: a look to add goes through [`adding`] and the editor, an edit is
//! dispatched, and a change of presets is made to the bench's presets. A
//! control is found by the name accessibility is given for it.
use super::*;
use crate::{
    presets::{SavedLook, keepable},
    textures::Textures,
    theme::{Accent, Appearance},
    ui::{
        library::{self, LibraryInput, LibraryState, Tab},
        timeline::{self, TimelineInput, TimelineState},
    },
};
use editor_core::{Editor, FrameRate, TextAnimation, TextAnimationKind, WordTime};
use eframe::egui::{Event, Modifiers, PointerButton, Pos2, accesskit::Role};
use std::{collections::HashSet, time::Duration};

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn look(group: Group, name: &str) -> &'static Arc<Look> {
    (builtin().iter())
        .find(|look| look.group == group && look.name == name)
        .unwrap_or_else(|| panic!("FIXTURE: no look is named {name}"))
}

fn editor() -> Editor {
    Editor::new(Project::new("Titles")).expect("a new project is valid")
}

/// Adds a look as the application does and hands back the new clip.
fn add(editor: &mut Editor, look: &Look, track: Option<TrackId>, at: Time) -> ClipId {
    let (command, clip) = adding(editor.project(), look, track, at);
    if let Err(error) = editor.dispatch(command) {
        panic!("{} was refused: {error}", look.name);
    }
    clip
}

fn placed(editor: &Editor, clip: ClipId) -> (TrackId, String, Time) {
    let (track, clip) = editor.project().clip(clip).expect("the clip is there");
    (track.id, track.name.clone(), clip.start)
}

/// The text track a new project has.
fn first_text(editor: &Editor) -> TrackId {
    (editor.project().tracks.iter())
        .find(|track| track.kind == TrackKind::Text)
        .expect("a new project has a text track")
        .id
}

fn text_track(editor: &mut Editor, name: &str, language: Option<&str>) -> TrackId {
    let track = Track::new(name, TrackKind::Text);
    let id = track.id;
    editor.dispatch(Command::AddTrack { track }).unwrap();
    if let Some(language) = language {
        editor
            .dispatch(Command::SetTrackLanguage {
                track_id: id,
                language: Some(language.to_owned()),
            })
            .unwrap();
    }
    id
}

/// A title of `duration` from `start` on `track`, its two words timed.
fn spoken(editor: &mut Editor, track: TrackId, start: Time, duration: Time) -> ClipId {
    let mut clip = Clip::text("Hello there", start, duration);
    if let ClipContent::Text { words, .. } = &mut clip.content {
        let half = Time(duration.0 / 2);
        *words = vec![
            WordTime {
                at: Time::ZERO,
                end: half,
            },
            WordTime {
                at: half,
                end: duration,
            },
        ];
    }
    let id = clip.id;
    editor
        .dispatch(Command::AddClip {
            track_id: track,
            clip,
        })
        .unwrap();
    id
}

fn content(editor: &Editor, clip: ClipId) -> ClipContent {
    editor
        .project()
        .clip(clip)
        .expect("the clip")
        .1
        .content
        .clone()
}

#[test]
fn every_look_is_its_own_and_adds_as_one_step_that_undoes() {
    let looks = builtin();
    let mut ids = HashSet::new();
    for look in looks {
        assert!(ids.insert(look.id), "{} is listed once", look.name);
        assert!(
            keepable(&look.content),
            "{} could be kept as it is",
            look.name
        );
    }
    let emoji = looks.iter().filter(|look| look.group == Group::Emoji);
    assert_eq!(emoji.count(), catalog::EMOJI.len());
    for group in [
        Group::Titles,
        Group::Captions,
        Group::LowerThirds,
        Group::TitleCards,
        Group::Callouts,
        Group::Stickers,
    ] {
        assert!(
            looks.iter().filter(|look| look.group == group).count() >= 3,
            "{} has looks to choose from",
            group.heading()
        );
    }
    let rate = FrameRate::default();
    for look in looks {
        let mut editor = editor();
        let tracks = editor.project().tracks.len();
        let steps = editor.history().count();
        let clip = add(&mut editor, look, None, seconds(1.0));
        let (track, clip_now) = editor.project().clip(clip).expect("added");
        assert_eq!(track.kind, TrackKind::Text, "{}", look.name);
        assert_eq!(clip_now.start, seconds(1.0), "{}", look.name);
        assert_eq!(clip_now.duration, look.duration(rate), "{}", look.name);
        assert!(clip_now.duration >= seconds(3.0), "{}", look.name);
        assert_eq!(
            editor.history().count(),
            steps + 1,
            "{} is one step",
            look.name
        );
        assert!(editor.undo(), "{} undoes", look.name);
        assert_eq!(editor.project().tracks.len(), tracks, "{}", look.name);
        assert!(
            editor.project().clip(clip).is_none(),
            "{} undoes",
            look.name
        );
    }
}

#[test]
fn a_look_goes_where_there_is_room_and_never_onto_captions_unasked() {
    let mut editor = editor();
    let simple = look(Group::Titles, "Simple");
    let first = add(&mut editor, simple, None, Time::ZERO);
    let (text_1, name, start) = placed(&editor, first);
    assert_eq!((name.as_str(), start), ("Text 1", Time::ZERO));

    // The same time is taken there: a new track.
    let second = add(&mut editor, simple, None, Time::ZERO);
    let (text_2, name, start) = placed(&editor, second);
    assert_eq!((name.as_str(), start), ("Text 2", Time::ZERO));
    assert_ne!(text_1, text_2);

    // Later, the first track has room again.
    let third = add(&mut editor, simple, None, seconds(10.0));
    assert_eq!(
        placed(&editor, third),
        (text_1, "Text 1".to_owned(), seconds(10.0))
    );

    // On the track it was let go on, at the first room from there.
    let fourth = add(&mut editor, simple, Some(text_2), seconds(1.0));
    assert_eq!(
        placed(&editor, fourth),
        (text_2, "Text 2".to_owned(), seconds(3.0))
    );

    // A time between frames is the frame's.
    let fifth = add(&mut editor, simple, Some(text_2), Time(20_010_000));
    assert_eq!(placed(&editor, fifth).2, seconds(20.0));

    // A caption track is a language's: nothing lands there unless it is
    // the track it was let go on.
    let mut editor = self::editor();
    let captions = first_text(&editor);
    editor
        .dispatch(Command::SetTrackLanguage {
            track_id: captions,
            language: Some("en".into()),
        })
        .unwrap();
    let title = add(&mut editor, simple, None, Time::ZERO);
    let (track, name, _) = placed(&editor, title);
    assert_ne!(track, captions);
    assert_eq!(name, "Text 2");
    let caption = add(
        &mut editor,
        look(Group::Captions, "Clean"),
        Some(captions),
        Time::ZERO,
    );
    assert_eq!(placed(&editor, caption).0, captions);
}

#[test]
fn a_look_is_made_for_the_canvas_it_goes_on() {
    let mut project = Project::new("4K");
    project.settings.width = 3840;
    project.settings.height = 2160;
    let simple = look(Group::Titles, "Simple");
    let (ClipContent::Text { font_size: was, .. }, ClipContent::Text { font_size, .. }) =
        (&simple.content, &simple.clip(Time::ZERO, &project).content)
    else {
        panic!("a title");
    };
    assert_eq!(*font_size, was * 2.0);

    let bar = look(Group::LowerThirds, "Name bar");
    let clip = bar.clip(Time::ZERO, &project);
    let (ClipContent::Graphic { size: was, .. }, ClipContent::Graphic { size, elements }) =
        (&bar.content, &clip.content)
    else {
        panic!("a graphic");
    };
    assert_eq!(*size, [was[0] * 2.0, was[1] * 2.0]);
    // Its lower left corner a margin in from the canvas's.
    let margin = 2160.0 * 0.07;
    let [x, y] = clip.transform.position;
    assert!((x - size[0] / 2.0 - (-1920.0 + margin)).abs() < 0.01, "{x}");
    assert!(
        (y + size[1] / 2.0 - (1080.0 - margin * 1.6)).abs() < 0.01,
        "{y}"
    );
    // Each element is its own, not the look's.
    let ClipContent::Graphic {
        elements: theirs, ..
    } = &bar.content
    else {
        unreachable!()
    };
    assert!(
        elements
            .iter()
            .zip(theirs)
            .all(|(mine, theirs)| mine.id != theirs.id)
    );

    // A kept look goes back where it was kept, at the size it was.
    let saved = Look::saved(&SavedLook {
        id: Uuid::new_v4(),
        name: "Mine".into(),
        content: simple.content.clone(),
        position: Some([10.0, -20.0]),
    });
    let clip = saved.clip(Time::ZERO, &project);
    assert_eq!(clip.transform.position, [10.0, -20.0]);
    assert_eq!(clip.content, simple.content);
}

#[test]
fn a_look_lasts_as_long_as_its_animations_need() {
    let rate = FrameRate::default();
    let mut long = look(Group::Titles, "Simple").as_ref().clone();
    let ClipContent::Text { enter, exit, .. } = &mut long.content else {
        panic!("a title");
    };
    let slow = TextAnimation {
        kind: TextAnimationKind::Fade,
        unit: TextAnimationKind::Fade.usual_unit(),
        duration: seconds(2.0),
        delay: Time::ZERO,
    };
    *enter = Some(slow);
    *exit = Some(slow);
    assert!(long.duration(rate) >= seconds(4.0));
    let mut editor = editor();
    let clip = add(&mut editor, &long, None, Time::ZERO);
    assert!(placed(&editor, clip).2 == Time::ZERO);
}

#[test]
fn a_title_given_a_look_keeps_its_words_and_their_times_in_one_step() {
    let mut editor = editor();
    let track = text_track(&mut editor, "Text 1", None);
    // A second is shorter than the typewriter's entrance and exit.
    let title = spoken(&mut editor, track, Time::ZERO, seconds(1.0));
    let before = content(&editor, title);
    let typewriter = look(Group::Titles, "Typewriter");
    let Action::Edit(command) = restyle(editor.project(), typewriter, &[title]) else {
        panic!("a title is given the look");
    };
    assert!(matches!(&command, Command::Batch { label, .. } if label == "Style title"));
    let steps = editor.history().count();
    editor.dispatch(command).expect("the look fits the title");
    assert_eq!(editor.history().count(), steps + 1);

    let (
        ClipContent::Text {
            text,
            words,
            style,
            enter,
            exit,
            ..
        },
        ClipContent::Text {
            text: had_text,
            words: had_words,
            ..
        },
        ClipContent::Text {
            style: its_style, ..
        },
    ) = (content(&editor, title), &before, &typewriter.content)
    else {
        panic!("titles");
    };
    assert_eq!(
        (&text, &words),
        (had_text, had_words),
        "the words are as they were"
    );
    assert_eq!(&style, its_style);
    let span = |animation: Option<TextAnimation>| animation.map_or(Time::ZERO, |a| a.span());
    assert!(enter.is_some(), "the entrance is kept, shortened");
    assert!(span(enter) + span(exit) <= seconds(1.0));

    assert!(editor.undo());
    assert_eq!(content(&editor, title), before);
}

#[test]
fn a_caption_s_look_goes_to_every_caption_of_its_track() {
    let mut editor = editor();
    let captions = text_track(&mut editor, "Captions", Some("en"));
    let plain = text_track(&mut editor, "Text 1", None);
    let cues: Vec<ClipId> = (0..3)
        .map(|n| {
            spoken(
                &mut editor,
                captions,
                seconds(2.0 * f64::from(n)),
                seconds(2.0),
            )
        })
        .collect();
    let lone = spoken(&mut editor, plain, Time::ZERO, seconds(2.0));
    assert_eq!(restyled_with(editor.project(), cues[1]), cues);
    assert_eq!(restyled_with(editor.project(), lone), vec![lone]);

    let karaoke = look(Group::Captions, "Karaoke box");
    let Action::Edit(command) = restyle(editor.project(), karaoke, &cues) else {
        panic!("captions are given the look");
    };
    assert!(matches!(&command, Command::Batch { label, .. } if label == "Style titles"));
    editor.dispatch(command).unwrap();
    let ClipContent::Text { style: theirs, .. } = &karaoke.content else {
        panic!("a title");
    };
    for cue in &cues {
        let ClipContent::Text { style, words, .. } = content(&editor, *cue) else {
            panic!("a caption");
        };
        assert_eq!(&style, theirs);
        assert_eq!(words.len(), 2);
    }
    let ClipContent::Text { style, .. } = content(&editor, lone) else {
        panic!("a title");
    };
    assert!(style.is_plain(), "a title of another track is left alone");

    // Given again, it is said that nothing would change.
    let again = restyle(editor.project(), karaoke, &cues);
    assert!(matches!(again, Action::Notify(said) if said.contains("already look like")));
}

#[test]
fn a_look_with_nothing_to_go_to_says_why() {
    let mut editor = editor();
    let track = text_track(&mut editor, "Text 1", None);
    let title = spoken(&mut editor, track, Time::ZERO, seconds(2.0));
    let simple = look(Group::Titles, "Simple");
    assert!(matches!(
        restyle(editor.project(), simple, &[]),
        Action::Notify(said) if said.starts_with("Select a title")
    ));
    assert!(matches!(
        restyle(editor.project(), look(Group::Stickers, "Check"), &[title]),
        Action::Notify(said) if said.starts_with("Only a title's look")
    ));
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
    assert!(matches!(
        restyle(editor.project(), look(Group::Titles, "Neon"), &[title]),
        Action::Notify(said) if said.starts_with("Select a title")
    ));
}

/// A control accessibility was told of in the last frame.
struct Seen {
    name: String,
    rect: Rect,
    role: Role,
    selected: Option<bool>,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    textures: Textures,
    titles: Titles,
    editor: Editor,
    library: LibraryState,
    timeline: TimelineState,
    selection: Selection,
    presets: Arc<Presets>,
    playhead: Time,
    size: Vec2,
    /// The width of the library.
    side: f32,
    time: f64,
    seen: Vec<Seen>,
    /// The rects filled with the stage's dark in the last frame.
    staged: Vec<Rect>,
    refused: Vec<String>,
    said: Vec<String>,
    wait: Duration,
    /// How often the tab asked for the font picker.
    fonts: usize,
}

impl Bench {
    fn new() -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut library = LibraryState::default();
        library.tab = Tab::Text;
        let mut bench = Self {
            ctx,
            palette,
            textures: Textures::default(),
            titles: Titles::default(),
            editor: editor(),
            library,
            timeline: TimelineState::default(),
            selection: Selection::default(),
            presets: Arc::default(),
            playhead: Time::ZERO,
            // Tall, so that every tile of the tab is on screen.
            size: vec2(1400.0, 4600.0),
            side: 300.0,
            time: 0.0,
            seen: Vec::new(),
            staged: Vec::new(),
            refused: Vec::new(),
            said: Vec::new(),
            wait: Duration::ZERO,
            fonts: 0,
        };
        bench.frame(Vec::new());
        bench
    }

    fn steps(&self) -> usize {
        self.editor.history().count()
    }

    fn dispatch(&mut self, command: Command) {
        if let Err(error) = self.editor.dispatch(command) {
            self.refused.push(error.to_string());
        }
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
            let all = ui.max_rect();
            let side = Rect::from_min_max(all.min, pos2(all.left() + self.side, all.bottom()));
            let lanes = Rect::from_min_max(pos2(side.right(), all.top()), pos2(all.right(), 400.0));
            library::show(
                ui,
                side,
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
                    speech: crate::ui::captions_tab::SpeechInput {
                        view: &crate::speech::SpeechView::Idle,
                        settings: &crate::speech::SpeechSettings::default(),
                        inventory: None,
                    },
                },
                &mut actions,
            );
            timeline::show(
                ui,
                lanes,
                &mut self.timeline,
                &mut self.selection,
                &mut self.textures,
                &mut crate::peaks::HeldPeaks::default(),
                &TimelineInput {
                    p: &self.palette,
                    project: &project,
                    playhead: self.playhead,
                    snapping: false,
                    media: None,
                    can_paste: false,
                    freeze_hold: seconds(2.0),
                    separator: crate::stems::Tool::Missing,
                },
                &mut actions,
            );
        });
        output.textures_delta.clear();
        let stage = self.palette.stage();
        self.staged = (output.shapes.iter())
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == stage => Some(rect.rect),
                _ => None,
            })
            .collect();
        self.wait = (output.viewport_output.values())
            .map(|viewport| viewport.repaint_delay)
            .min()
            .unwrap_or(Duration::MAX);
        if let Some(update) = &output.platform_output.accesskit_update {
            self.seen = (update.nodes.iter())
                .filter_map(|(_, node)| {
                    let bounds = node.bounds()?;
                    Some(Seen {
                        name: node.label().or_else(|| node.value())?.to_owned(),
                        rect: Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                        role: node.role(),
                        selected: (node.toggled())
                            .map(|toggled| toggled == egui::accesskit::Toggled::True),
                    })
                })
                .collect();
        }
        // The end of the application's frame.
        for action in actions {
            match action {
                Action::Edit(command) => self.dispatch(command),
                Action::AddLook { look, track, at } => {
                    let at = at.unwrap_or(self.playhead);
                    let (command, clip) = adding(self.editor.project(), &look, track, at);
                    let steps = self.steps();
                    self.dispatch(command);
                    if self.steps() > steps {
                        self.selection.only(clip);
                    }
                }
                Action::Presets(change) => {
                    let presets = Arc::make_mut(&mut self.presets);
                    match change {
                        PresetChange::Keep {
                            name,
                            content,
                            position,
                        } => {
                            presets.keep(&name, *content, position).unwrap();
                        }
                        PresetChange::Rename(id, name) => {
                            presets.rename(id, &name);
                        }
                        PresetChange::Remove(id) => {
                            presets.remove(id);
                        }
                        PresetChange::AddColor(color) => {
                            presets.add_color(color);
                        }
                        PresetChange::RemoveColor(color) => {
                            presets.colors.retain(|kept| *kept != color);
                        }
                    }
                }
                Action::ImportFonts => self.fonts += 1,
                Action::Notify(text) => self.said.push(text),
                Action::Seek(time) => self.playhead = time,
                other => panic!("FIXTURE: the panes asked for {other:?}"),
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

    /// Whether a label says this.
    fn says(&self, text: &str) -> bool {
        self.seen.iter().any(|seen| seen.name.contains(text))
    }

    fn rect(&self, name: &str) -> Rect {
        match self.find(name) {
            Some(seen) => seen.rect,
            None => {
                let names: Vec<&str> = self.seen.iter().map(|seen| seen.name.as_str()).collect();
                panic!("FIXTURE: no control is named \"{name}\" among {names:?}")
            }
        }
    }

    fn at(&self, name: &str) -> Pos2 {
        self.rect(name).center()
    }

    fn button(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Self::button(pos, PointerButton::Primary, true)]);
        self.frame(vec![Self::button(pos, PointerButton::Primary, false)]);
        self.frame(Vec::new());
    }

    /// Clicks as close together as a hand makes them.
    fn clicks(&mut self, name: &str, count: usize) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.frame(vec![Event::PointerMoved(pos)]);
        for _ in 0..count {
            self.frame(vec![Self::button(pos, PointerButton::Primary, true)]);
            self.frame(vec![Self::button(pos, PointerButton::Primary, false)]);
        }
        self.frame(Vec::new());
    }

    fn secondary_click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, true)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, false)]);
        self.frame(Vec::new());
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

    /// Takes hold of the tile and lets it go at `to`, by way of the lanes.
    fn carry(&mut self, name: &str, to: Pos2) {
        self.frame(Vec::new());
        let from = self.at(name);
        self.frame(vec![Event::PointerMoved(from)]);
        self.frame(vec![Self::button(from, PointerButton::Primary, true)]);
        for step in 1..=6 {
            self.frame(vec![Event::PointerMoved(from.lerp(to, step as f32 / 6.0))]);
        }
        self.frame(vec![Event::PointerMoved(to)]);
        self.frame(vec![Self::button(to, PointerButton::Primary, false)]);
        self.frame(Vec::new());
    }

    /// The selected clip, which is the one added last.
    fn added(&self) -> (TrackId, Clip) {
        let [clip] = self.selection.clips.as_slice() else {
            panic!("one clip is selected: {:?}", self.selection.clips);
        };
        let (track, clip) = self.editor.project().clip(*clip).expect("the clip");
        (track.id, clip.clone())
    }
}

#[test]
fn the_tab_lists_its_groups_and_a_search_narrows_them() {
    let mut bench = Bench::new();
    for heading in [
        "Titles",
        "Captions",
        "Lower thirds",
        "Title cards",
        "Callouts",
        "Stickers",
        "Emoji",
        "Yours",
    ] {
        assert!(bench.says(heading), "the tab has {heading}");
    }
    for tile in [
        "Simple",
        "Karaoke box",
        "Name bar",
        "Big title",
        "Speech bubble",
        "Like",
        "Fire",
    ] {
        assert!(bench.has(tile), "a tile is named {tile}");
    }
    assert!(bench.says("Keep a title's look or a graphic from the inspector"));
    assert!(bench.has("Add brand colour"));
    let titles = bench.rect("Simple");
    let yours = bench.rect("Add brand colour");
    assert!(
        yours.top() > titles.bottom(),
        "nothing kept yet: Yours is last"
    );

    bench.library.browser.search = "FIRE".into();
    bench.frame(Vec::new());
    assert!(bench.has("Fire"));
    assert!(
        !bench.has("Simple"),
        "{:?}",
        bench.find("Simple").map(|s| s.rect)
    );
    assert!(
        !bench.says("Yours"),
        "an empty group is not listed while searching"
    );

    bench.library.browser.search = "lower".into();
    bench.frame(Vec::new());
    assert!(
        bench.has("Name bar") && bench.has("Pill"),
        "a group is found by its name"
    );

    bench.library.browser.search = "qqqq".into();
    bench.frame(Vec::new());
    assert!(
        !bench.has("Fire") && !bench.says("Lower thirds"),
        "nothing is listed"
    );

    bench.library.browser.search.clear();
    bench.click("Import fonts");
    assert_eq!(bench.fonts, 1);
}

#[test]
fn a_double_click_or_return_adds_at_the_playhead() {
    let mut bench = Bench::new();
    bench.playhead = seconds(2.0);
    let before = bench.steps();
    bench.clicks("Simple", 2);
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    let (_, clip) = bench.added();
    assert_eq!(clip.start, seconds(2.0));
    assert!(matches!(&clip.content, ClipContent::Text { .. }));

    // A third click of the same hand adds nothing more.
    let before = bench.steps();
    bench.time += 1.0;
    bench.clicks("Fire", 3);
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);

    // Return adds the tile last clicked.
    bench.time += 1.0;
    bench.click("Name bar");
    assert_eq!(
        bench.find("Name bar").and_then(|seen| seen.selected),
        Some(true)
    );
    let before = bench.steps();
    bench.key(Key::Enter, Modifiers::NONE);
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    let (_, clip) = bench.added();
    assert!(matches!(&clip.content, ClipContent::Graphic { .. }));
    assert_eq!(clip.name, "Name bar");
}

#[test]
fn a_tile_let_go_on_the_timeline_adds_there_or_restyles_a_title() {
    let mut bench = Bench::new();
    let track = first_text(&bench.editor);
    let title = spoken(&mut bench.editor, track, Time::ZERO, seconds(4.0));
    bench.frame(Vec::new());
    let lanes = bench.rect("Tracks");
    let row = bench.rect("Track Text 1");

    // On the title: it takes the look and keeps its words.
    let before = bench.steps();
    bench.carry("Neon", pos2(lanes.left() + 40.0, row.center().y));
    assert_eq!(
        bench.steps(),
        before + 1,
        "{:?} {:?}",
        bench.refused,
        bench.said
    );
    let ClipContent::Text { style, words, .. } = content(&bench.editor, title) else {
        panic!("a title");
    };
    let ClipContent::Text { style: neon, .. } = &look(Group::Titles, "Neon").content else {
        panic!("a title");
    };
    assert_eq!(&style, neon);
    assert_eq!(words.len(), 2);

    // Past it, on the same track, where it was let go.
    let before = bench.steps();
    bench.carry("Hook", pos2(lanes.left() + 640.0, row.center().y));
    assert_eq!(
        bench.steps(),
        before + 1,
        "{:?} {:?}",
        bench.refused,
        bench.said
    );
    let (on, clip) = bench.added();
    assert_eq!(on, track);
    assert!(clip.start > seconds(4.0), "{:?}", clip.start);
}

#[test]
fn the_tab_rests_until_a_tile_is_pointed_at() {
    let mut bench = Bench::new();
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(bench.wait > Duration::from_secs(1), "{:?}", bench.wait);
    let pos = bench.at("Hook");
    bench.frame(vec![Event::PointerMoved(pos)]);
    bench.frame(Vec::new());
    assert_eq!(
        bench.wait,
        Duration::ZERO,
        "the tile under the pointer plays"
    );
    bench.frame(vec![Event::PointerGone]);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(bench.wait > Duration::from_secs(1), "{:?}", bench.wait);
}

#[test]
fn kept_looks_and_brand_colours_come_first_and_can_be_let_go() {
    let mut bench = Bench::new();
    bench.click("Add brand colour");
    bench.click("Add colour");
    assert_eq!(bench.presets.colors, vec![[255, 214, 0, 255]]);
    Arc::make_mut(&mut bench.presets)
        .keep(
            "My hook",
            look(Group::Titles, "Hook").content.clone(),
            Some([0.0, 100.0]),
        )
        .unwrap();
    bench.frame(Vec::new());
    assert!(
        bench.rect("My hook").top() < bench.rect("Simple").top(),
        "Yours is first"
    );
    assert!(bench.has("Brand colour #FFD600"));

    bench.secondary_click("My hook");
    bench.click("Rename…");
    bench.key(Key::A, Modifiers::COMMAND);
    bench.frame(vec![Event::Text("Hook two".into())]);
    bench.key(Key::Enter, Modifiers::NONE);
    assert_eq!(bench.presets.looks[0].name, "Hook two");

    // It adds where it was kept, as it was kept.
    bench.time += 1.0;
    bench.clicks("Hook two", 2);
    let (_, clip) = bench.added();
    assert_eq!(clip.transform.position, [0.0, 100.0]);

    bench.secondary_click("Hook two");
    bench.click("Remove preset");
    assert!(bench.presets.looks.is_empty());
    bench.secondary_click("Brand colour #FFD600");
    bench.click("Remove colour");
    assert!(bench.presets.colors.is_empty());
}

#[test]
fn every_tab_has_room_in_a_narrow_library() {
    let mut bench = Bench::new();
    bench.side = 236.0;
    bench.size = vec2(900.0, 700.0);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    let mut cells = Vec::new();
    for tab in Tab::ALL {
        let cell = bench.rect(tab.label());
        assert!(cell.width() >= 24.0, "{} has room: {cell:?}", tab.label());
        assert!(
            cell.right() <= 236.0 + 0.5,
            "{} is in the library",
            tab.label()
        );
        cells.push(cell);
    }
    for pair in cells.windows(2) {
        assert!(
            pair[0].right() <= pair[1].left() + 0.5,
            "tabs do not overlap"
        );
    }
    assert!(bench.has("Simple"), "the tab still shows its tiles");
    // §5.2: a look's picture sits on a well of the stage's dark, as the
    // viewer's picture does, in place of the tile's own backdrop.
    let tile = bench.rect("Simple");
    assert!(
        (bench.staged.iter())
            .any(|well| tile.contains_rect(*well) && well.width() > tile.width() - 1.0),
        "{tile:?} {:?}",
        bench.staged
    );
}
