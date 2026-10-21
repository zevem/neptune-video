//! Issue 14: play and speed in a transport of any width.
//!
//! Each test paints the viewer or the source monitor with [`show`] or
//! [`show_source`] inside real egui passes, finds the play disc and the speed
//! control among what was painted, and presses them with the pointer. What is
//! asserted is where they were painted and what the presses asked for.
//!
//! `FIXTURE:` is a precondition of the test itself and proves nothing about
//! the transport. `ISSUE14:` is the behavior the issue describes.
use super::{GROUP_GAP, SPEED, SourceInput, ViewerInput, ViewerState, show, show_source};
use crate::{
    record::RecordView,
    textures::Textures,
    theme::{self, Accent, Appearance, Palette},
    ui::{Action, ClipDraft, Selection, Transport},
};
use editor_core::{Asset, Clip, FrameRate, MediaKind, Project, Time};
use eframe::egui::{self, Pos2, Rect, Shape, Vec2, pos2, vec2};
use std::{path::PathBuf, sync::Arc};

/// The viewer's pane in an 800×600 window with the library and the inspector
/// both shown and the library at its widest, as in the capture the issue
/// names; in a 760×560 window, where the library has given way; and in a
/// window of ordinary size.
const PANES: [Vec2; 3] = [vec2(168.0, 300.0), vec2(498.0, 280.0), vec2(900.0, 420.0)];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Deck {
    Timeline,
    Source,
}

/// What one pass painted that a transport is found by.
#[derive(Default)]
struct Painted {
    /// Filled circles: centre, radius and fill.
    discs: Vec<(Pos2, f32, egui::Color32)>,
    /// Laid-out text and where it is.
    texts: Vec<(String, Rect)>,
    /// What accessibility is told: each name, where it is, and whether it
    /// can be used.
    names: Vec<(String, Rect, bool)>,
}

impl Painted {
    fn collect(&mut self, shape: &Shape) {
        match shape {
            Shape::Vec(shapes) => {
                for shape in shapes {
                    self.collect(shape);
                }
            }
            Shape::Circle(circle) => self.discs.push((circle.center, circle.radius, circle.fill)),
            Shape::Text(text) => self.texts.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            _ => {}
        }
    }

    fn text(&self, wanted: &str) -> Option<Rect> {
        let mut found = self.texts.iter().filter(|(text, _)| text == wanted);
        let first = found.next()?;
        assert!(
            found.next().is_none(),
            "FIXTURE: \"{wanted}\" is painted more than once"
        );
        Some(first.1)
    }

    /// Where the one control or label named `wanted` is, and whether it
    /// can be used.
    fn named(&self, wanted: &str) -> Option<(Rect, bool)> {
        let mut found = self.names.iter().filter(|(name, ..)| name == wanted);
        let (_, rect, enabled) = found.next()?;
        assert!(
            found.next().is_none(),
            "FIXTURE: \"{wanted}\" is named more than once"
        );
        Some((*rect, *enabled))
    }
}

/// A pane the viewer or the source monitor is painted in.
struct Pane {
    ctx: egui::Context,
    size: Vec2,
    deck: Deck,
    palette: Palette,
    project: Project,
    asset: Asset,
    textures: Textures,
    viewer: ViewerState,
    selection: Selection,
    draft: Option<ClipDraft>,
    playing: bool,
    record: RecordView,
}

impl Pane {
    fn new(deck: Deck, size: Vec2) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        // FIXTURE: a title ten seconds long, so the timeline has something
        // to play; an empty one offers no play (spec §5.3).
        let mut project = Project::new("Transport");
        Arc::make_mut(&mut project.tracks[2]).clips.push(Clip::text(
            "Transport",
            Time::ZERO,
            Time::from_seconds(10.0),
        ));
        let mut asset = Asset::new(
            PathBuf::from("/media/transport.mp4"),
            MediaKind::Video,
            Time::from_seconds(10.0),
        );
        asset.width = 1920;
        asset.height = 1080;
        asset.frame_rate = Some(FrameRate::default());
        Self {
            ctx,
            size,
            deck,
            palette,
            project,
            asset,
            textures: Textures::default(),
            viewer: ViewerState::default(),
            selection: Selection::default(),
            draft: None,
            playing: false,
            record: RecordView::Idle,
        }
    }

    /// One pass with `events`. Returns what it asked for and what it painted.
    fn pass(&mut self, events: Vec<egui::Event>) -> (Vec<Action>, Painted) {
        self.textures.begin_frame();
        let ctx = self.ctx.clone();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            events,
            ..Default::default()
        };
        let mut actions: Vec<Action> = Vec::new();
        let output = ctx.run_ui(input, |ui| {
            let rect = ui.max_rect();
            match self.deck {
                Deck::Timeline => show(
                    ui,
                    rect,
                    &mut self.viewer,
                    &mut self.selection,
                    &mut self.draft,
                    &mut self.textures,
                    &ViewerInput {
                        p: &self.palette,
                        project: &self.project,
                        query: None,
                        playhead: Time::ZERO,
                        playing: self.playing,
                        speed: 1.0,
                        looping: false,
                        media: None,
                        record: self.record,
                        library: true,
                        quality: Default::default(),
                    },
                    &mut actions,
                ),
                Deck::Source => show_source(
                    ui,
                    rect,
                    &mut self.textures,
                    &SourceInput {
                        p: &self.palette,
                        asset: &self.asset,
                        rate: FrameRate::default(),
                        background: [0, 0, 0, 255],
                        playhead: Time::ZERO,
                        playing: self.playing,
                        speed: 1.0,
                        looping: false,
                        media: None,
                    },
                    &mut actions,
                ),
            }
        });
        let mut painted = Painted::default();
        for clipped in &output.shapes {
            painted.collect(&clipped.shape);
        }
        if let Some(update) = &output.platform_output.accesskit_update {
            painted.names = (update.nodes.iter())
                .filter_map(|(_, node)| {
                    let bounds = node.bounds()?;
                    Some((
                        node.label().or_else(|| node.value())?.to_owned(),
                        Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                        !node.is_disabled(),
                    ))
                })
                .collect();
        }
        // No renderer takes this pass's texture changes, the font atlas among
        // them, and epaint refuses to drop them unless told it is meant.
        output.drop_without_applying_deltas();
        (actions, painted)
    }

    /// The pane at rest: painted until nothing in it is still arriving.
    fn rest(&mut self) -> Painted {
        self.pass(Vec::new());
        self.pass(Vec::new());
        let (actions, painted) = self.pass(Vec::new());
        assert!(
            actions.is_empty(),
            "FIXTURE: a pane nobody touched asked for {actions:?}"
        );
        painted
    }

    /// A press and release of the primary button at `at`, a pass for each
    /// step. Returns everything asked for on the way and what was painted
    /// after.
    fn click(&mut self, at: Pos2) -> (Vec<Action>, Painted) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut asked = Vec::new();
        for events in [
            vec![egui::Event::PointerMoved(at)],
            vec![button(true)],
            vec![button(false)],
        ] {
            asked.extend(self.pass(events).0);
        }
        let (actions, painted) = self.pass(Vec::new());
        asked.extend(actions);
        (asked, painted)
    }

    fn bounds(&self) -> Rect {
        Rect::from_min_size(Pos2::ZERO, self.size)
    }

    /// Where the play control is: the button named for what it does now.
    /// Its disc is neutral, so it is found by its name (spec §4.4.2).
    fn play(&self, painted: &Painted) -> Rect {
        let named = ["Play (Space)", "Pause (Space)"]
            .into_iter()
            .find_map(|name| painted.named(name));
        let Some((rect, _)) = named else {
            panic!(
                "ISSUE14: no play control is painted in a pane of {:?}",
                self.size
            );
        };
        // Its disc fills it, and is not the accent: nothing is at rest.
        let disc = (painted.discs.iter())
            .find(|(centre, radius, _)| *radius >= 12.0 && centre.distance(rect.center()) < 0.5);
        assert!(
            disc.is_some_and(|(_, radius, fill)| *radius * 2.0 >= rect.width() - 0.5
                && *fill != self.palette.accent),
            "play at {rect:?} is not a neutral disc: {disc:?}"
        );
        rect
    }

    /// Where the speed control was painted: its place around what it reads,
    /// which a button centres in itself. This is where it reads, not where
    /// it is pressed; the presses on its edges are the check on that.
    fn speed(&self, painted: &Painted) -> Rect {
        let Some(text) = painted.text("1×") else {
            panic!(
                "ISSUE14: no speed control is painted in a pane of {:?}",
                self.size
            );
        };
        Rect::from_center_size(text.center(), SPEED)
    }
}

fn is(deck: Deck, action: &Action, wanted: Transport) -> bool {
    match (deck, action) {
        (Deck::Source, Action::Source(transport)) => *transport == wanted,
        (Deck::Timeline, Action::TogglePlay) => wanted == Transport::TogglePlay,
        (Deck::Timeline, Action::SetSpeed(speed)) => wanted == Transport::SetSpeed(*speed),
        _ => false,
    }
}

fn each_pane(mut check: impl FnMut(Pane)) {
    for deck in [Deck::Timeline, Deck::Source] {
        for size in PANES {
            check(Pane::new(deck, size));
        }
    }
}

#[test]
fn play_and_speed_are_painted_apart_and_inside_the_pane() {
    each_pane(|mut pane| {
        let painted = pane.rest();
        let (play, speed) = (pane.play(&painted), pane.speed(&painted));
        let (deck, size) = (pane.deck, pane.size);
        assert!(
            pane.bounds().contains_rect(play) && pane.bounds().contains_rect(speed),
            "ISSUE14: {deck:?} at {size:?}: play {play:?} or speed {speed:?} is \
             painted outside the pane"
        );
        assert!(
            speed.left() - play.right() >= GROUP_GAP - 0.5,
            "ISSUE14: {deck:?} at {size:?}: play {play:?} and speed {speed:?} are \
             not apart"
        );
        assert!(
            (play.center().y - speed.center().y).abs() <= 1.5,
            "ISSUE14: {deck:?} at {size:?}: play {play:?} and speed {speed:?} are not \
             on one line; centres {:?} and {:?}",
            play.center(),
            speed.center()
        );
    });
}

#[test]
fn a_press_on_play_asks_only_to_play() {
    each_pane(|mut pane| {
        let painted = pane.rest();
        let play = pane.play(&painted);
        let (deck, size) = (pane.deck, pane.size);
        // Its centre, and just inside the edge nearest the speed control.
        for at in [play.center(), play.right_center() - vec2(3.0, 0.0)] {
            let (asked, _) = pane.click(at);
            assert!(
                matches!(asked.as_slice(), [one] if is(deck, one, Transport::TogglePlay)),
                "ISSUE14: {deck:?} at {size:?}: a press on play at {at:?} asked for \
                 {asked:?}"
            );
            pane.rest();
        }
    });
}

#[test]
fn a_press_on_speed_opens_its_menu_and_a_choice_sets_the_speed() {
    each_pane(|mut pane| {
        let painted = pane.rest();
        let (play, speed) = (pane.play(&painted), pane.speed(&painted));
        let (deck, size) = (pane.deck, pane.size);
        assert!(
            painted.text("2×").is_none(),
            "FIXTURE: the speed menu is open before it was asked for"
        );
        // Its centre, just inside the edge nearest play, and just inside its
        // top and bottom: the control is pressed over all of its place.
        for at in [
            speed.center(),
            speed.left_center() + vec2(3.0, 0.0),
            speed.center_top() + vec2(0.0, 3.0),
            speed.center_bottom() - vec2(0.0, 3.0),
        ] {
            fixture_apart(play, at);
            let (asked, painted) = pane.click(at);
            assert!(
                asked.is_empty(),
                "ISSUE14: {deck:?} at {size:?}: a press on speed at {at:?} asked for \
                 {asked:?}"
            );
            let Some(double) = painted.text("2×") else {
                panic!(
                    "ISSUE14: {deck:?} at {size:?}: a press on speed at {at:?} opened \
                     no menu"
                );
            };
            let (asked, painted) = pane.click(double.center());
            assert!(
                matches!(asked.as_slice(), [one] if is(deck, one, Transport::SetSpeed(2.0))),
                "ISSUE14: {deck:?} at {size:?}: choosing 2× asked for {asked:?}"
            );
            assert!(
                painted.text("2×").is_none(),
                "ISSUE14: {deck:?} at {size:?}: the speed menu stayed open after a \
                 choice"
            );
            pane.rest();
        }
    });
}

fn fixture_apart(play: Rect, at: Pos2) {
    assert!(
        !play.contains(at),
        "FIXTURE: the point pressed for speed is on the play control"
    );
}

#[test]
fn play_reads_as_pause_while_playing_and_still_asks_to_toggle() {
    each_pane(|mut pane| {
        let play = {
            let painted = pane.rest();
            pane.play(&painted)
        };
        pane.playing = true;
        // The cross-fade from one glyph to the other has its passes.
        for _ in 0..30 {
            pane.pass(Vec::new());
        }
        let painted = pane.rest();
        let (deck, size) = (pane.deck, pane.size);
        assert_eq!(
            pane.play(&painted),
            play,
            "ISSUE14: {deck:?} at {size:?}: play moved when playback started"
        );
        let (asked, _) = pane.click(play.center());
        assert!(
            matches!(asked.as_slice(), [one] if is(deck, one, Transport::TogglePlay)),
            "ISSUE14: {deck:?} at {size:?}: a press on pause asked for {asked:?}"
        );
    });
}

/// Where the record control's disc was painted, if it was: the one
/// `Icon::Disc` (Tabler's filled `circle` glyph) in the pane, which took the
/// microphone's place (spec §4.4.3).
fn record_disc(_pane: &Pane, painted: &Painted) -> Option<Pos2> {
    let mut found = (painted.texts.iter()).filter(|(text, _)| text == "\u{f671}");
    let first = found.next().map(|(_, rect)| rect.center());
    assert!(found.next().is_none(), "FIXTURE: more than one record disc");
    first
}

#[test]
fn only_the_timeline_records_and_a_press_on_record_asks_for_a_take() {
    each_pane(|mut pane| {
        let painted = pane.rest();
        let (deck, size) = (pane.deck, pane.size);
        let disc = record_disc(&pane, &painted);
        if deck == Deck::Source {
            assert!(
                disc.is_none(),
                "the source monitor at {size:?} offers to record"
            );
            return;
        }
        let Some(disc) = disc else {
            panic!("no record control in the timeline's pane of {size:?}");
        };
        let (play, speed) = (pane.play(&painted), pane.speed(&painted));
        assert!(pane.bounds().contains(disc));
        assert!(
            disc.x - play.right() >= GROUP_GAP && speed.left() > disc.x,
            "at {size:?}: record at {disc:?} is not between play {play:?} and speed {speed:?}"
        );
        let (asked, _) = pane.click(disc);
        assert!(
            matches!(asked.as_slice(), [Action::Record]),
            "at {size:?}: a press on record asked for {asked:?}"
        );
    });
}

#[test]
fn a_running_take_shows_its_time_in_speeds_place_and_a_press_stops_it() {
    for size in PANES {
        let mut pane = Pane::new(Deck::Timeline, size);
        let at_rest = pane.rest();
        let disc = record_disc(&pane, &at_rest).unwrap();
        pane.record = RecordView::Recording {
            elapsed: Time::from_seconds(3.6),
            level: 0.5,
        };
        let painted = pane.rest();
        let Some(time) = painted.text("0:03") else {
            panic!("at {size:?}: no elapsed time while recording");
        };
        assert!(
            pane.bounds().contains_rect(time),
            "at {size:?}: {time:?} is clipped"
        );
        assert!(
            painted.text("1×").is_none(),
            "at {size:?}: the speed control is offered while recording"
        );
        // The stop control is where record was.
        let (asked, _) = pane.click(disc);
        assert!(
            matches!(asked.as_slice(), [Action::StopRecord]),
            "at {size:?}: a press on stop asked for {asked:?}"
        );
        let (asked, _) = pane.click(time.center());
        assert!(
            matches!(asked.as_slice(), [Action::StopRecord]),
            "at {size:?}: a press on the time asked for {asked:?}"
        );
        // Closing, the control asks for nothing.
        pane.record = RecordView::Stopping;
        pane.rest();
        let (asked, _) = pane.click(disc);
        assert!(asked.is_empty(), "at {size:?}: closing asked for {asked:?}");
    }
}

#[test]
fn the_timecode_is_read_out_with_the_duration() {
    // Spec §4.4.1 and §7.2: the timecode is painted and named as one label.
    for (deck, name) in [
        (Deck::Timeline, "Playhead 00:00:00:00 of 00:00:10:00"),
        (Deck::Source, "Source 00:00:00:00 of 00:00:10:00"),
    ] {
        let mut pane = Pane::new(deck, PANES[2]);
        let painted = pane.rest();
        let Some((label, _)) = painted.named(name) else {
            panic!("{deck:?}: no label \"{name}\" among {:?}", painted.names);
        };
        let Some(timecode) = painted.text("00:00:00:00") else {
            panic!("{deck:?}: no timecode painted");
        };
        assert!(
            label.contains_rect(timecode.shrink(0.5)),
            "{deck:?}: the label {label:?} is not where the timecode {timecode:?} is"
        );
    }
}

#[test]
fn an_empty_timeline_offers_nothing_to_play() {
    // Spec §5.3: play and the frame steps wait for something to play.
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    pane.project = Project::new("Empty");
    let painted = pane.rest();
    for name in [
        "Play (Space)",
        "Previous frame (←)",
        "Next frame (→)",
        "Go to start (Home)",
        "Go to end (End)",
    ] {
        let Some((rect, enabled)) = painted.named(name) else {
            panic!("no \"{name}\" among {:?}", painted.names);
        };
        assert!(!enabled, "\"{name}\" can be used on an empty timeline");
        let (asked, _) = pane.click(rect.center());
        assert!(asked.is_empty(), "a press on {name} asked for {asked:?}");
    }
    assert!(
        painted
            .named("No media in the timeline. Import media or drag clips from the library.")
            .is_some(),
        "the stage does not say the timeline is empty: {:?}",
        painted.names
    );
}
