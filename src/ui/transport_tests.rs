//! Issue 14: play and speed in a transport of any width.
//!
//! Each test paints the viewer or the source monitor with [`show`] or
//! [`show_source`] inside real egui passes, finds the play disc and the speed
//! control among what was painted, and presses them with the pointer. What is
//! asserted is where they were painted and what the presses asked for.
//!
//! `FIXTURE:` is a precondition of the test itself and proves nothing about
//! the transport. `ISSUE14:` is the behavior the issue describes.
use super::{
    GROUP_GAP, PICK_PROMPT, PLAY, SPEED, SourceInput, TRANSPORT, ViewerInput, ViewerState, show,
    show_source, stage_palette,
};
use crate::{
    record::RecordView,
    textures::Textures,
    theme::{self, Accent, Appearance, Palette},
    ui::{Action, ClipDraft, ColourPick, Selection, Transport},
};
use editor_core::{Asset, Clip, FrameRate, MediaKind, Project, Time};
use editor_media::{MediaEvent, MediaService};
use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Shape, Stroke, Vec2, accesskit, pos2, vec2,
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

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
    /// Stroked circles: centre, radius and stroke.
    rings: Vec<(Pos2, f32, Stroke)>,
    /// Rectangles: where, their corners, fill and stroke.
    rects: Vec<(Rect, CornerRadius, Color32, Stroke)>,
    /// The place in the paint order of each of `rects`, and of each closed
    /// four-point path: the frames drawn round layers.
    rects_at: Vec<usize>,
    frames_at: Vec<usize>,
    /// How many shapes were painted.
    shapes: usize,
    /// Laid-out text and where it is.
    texts: Vec<(String, Rect)>,
    /// What accessibility is told: each name, where it is, and whether it
    /// can be used.
    names: Vec<(String, Rect, bool)>,
    /// The name of what has the keyboard.
    focused: Option<String>,
    /// The rows of a choice: each name, and whether it is the one chosen.
    choices: Vec<(String, bool)>,
}

impl Painted {
    fn collect(&mut self, shape: &Shape) {
        match shape {
            Shape::Vec(shapes) => {
                for shape in shapes {
                    self.collect(shape);
                }
                return;
            }
            Shape::Circle(circle) => {
                if circle.fill != Color32::TRANSPARENT {
                    self.discs.push((circle.center, circle.radius, circle.fill));
                }
                if circle.stroke.width > 0.0 {
                    self.rings
                        .push((circle.center, circle.radius, circle.stroke));
                }
            }
            Shape::Rect(rect) => {
                self.rects
                    .push((rect.rect, rect.corner_radius, rect.fill, rect.stroke));
                self.rects_at.push(self.shapes);
            }
            Shape::Path(path) if path.closed && path.points.len() == 4 => {
                self.frames_at.push(self.shapes);
            }
            Shape::Text(text) => self.texts.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            _ => {}
        }
        self.shapes += 1;
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
    /// Whether the library is shown beside the viewer.
    library: bool,
    /// The media workers, when the test starts them, and how many failures
    /// for the layers drawn they have published.
    service: Option<MediaService>,
    failures: usize,
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
            library: true,
            service: None,
            failures: 0,
        }
    }

    /// One pass with `events`. Returns what it asked for and what it painted.
    fn pass(&mut self, events: Vec<egui::Event>) -> (Vec<Action>, Painted) {
        // What the workers published is handed over as the window does: a
        // frame that could not be decoded is given up by its layer.
        while let Some(event) = self.service.as_ref().and_then(MediaService::poll) {
            match event {
                MediaEvent::Error {
                    asset_id: Some(asset),
                    consumer,
                    size,
                    ..
                } => {
                    if self.textures.failed(consumer, asset, size) {
                        self.failures += 1;
                    }
                }
                other => panic!("FIXTURE: the worker published {other:?}"),
            }
        }
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
                        media: self.service.as_ref(),
                        record: self.record,
                        library: self.library,
                        quality: Default::default(),
                        cards: &[],
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
            painted.focused = (update.nodes.iter())
                .find(|(id, _)| *id == update.focus)
                .and_then(|(_, node)| node.label())
                .map(str::to_owned);
            painted.choices = (update.nodes.iter())
                .filter(|(_, node)| node.role() == accesskit::Role::RadioButton)
                .filter_map(|(_, node)| {
                    let chosen = node.toggled() == Some(accesskit::Toggled::True);
                    Some((node.label()?.to_owned(), chosen))
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
/// `Icon::Disc` (Tabler's filled `circle` glyph) in the transport, which took
/// the microphone's place (spec §4.4.3). The stage's status dots are the
/// same glyph (§3.8), and are not looked at.
fn record_disc(pane: &Pane, painted: &Painted) -> Option<Pos2> {
    let transport = Rect::from_min_max(pos2(0.0, pane.size.y - TRANSPORT), pane.size.to_pos2());
    let mut found = (painted.texts.iter())
        .filter(|(text, rect)| text == "\u{f671}" && transport.contains(rect.center()));
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

impl Pane {
    /// The pointer moved to `at` and left there for a while: long enough
    /// for a hover to light fully and a tooltip to show.
    fn hover(&mut self, at: Pos2) -> Painted {
        self.pass(vec![egui::Event::PointerMoved(at)]);
        for _ in 0..90 {
            self.pass(Vec::new());
        }
        self.pass(Vec::new()).1
    }

    /// The primary button pressed at `at` and still held.
    fn hold(&mut self, at: Pos2) -> Painted {
        self.pass(vec![egui::Event::PointerMoved(at)]);
        self.pass(vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        self.pass(Vec::new()).1
    }

    /// Tab pressed until the control named `wanted` has the keyboard.
    fn focus(&mut self, wanted: &str) -> Painted {
        let tab = egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        for _ in 0..40 {
            self.pass(vec![tab.clone()]);
            let (_, painted) = self.pass(Vec::new());
            if painted.focused.as_deref() == Some(wanted) {
                return painted;
            }
        }
        panic!("FIXTURE: Tab never reached \"{wanted}\"");
    }

    /// The stage's palette, which the transport is drawn in.
    fn stage(&self) -> Palette {
        stage_palette(&self.palette)
    }
}

/// Whether a rectangle filled `fill` was painted over `place`.
fn filled(painted: &Painted, place: Rect, fill: Color32) -> bool {
    (painted.rects.iter()).any(|(rect, _, shown, _)| *shown == fill && rect.contains_rect(place))
}

#[test]
fn play_speed_and_record_light_on_hover_press_and_focus() {
    // Spec §4.4.2–§4.4.4 and §9.12: hover fills `hover`, a press `pressed`,
    // and the keyboard rings what it is on.
    let (hover, pressed, ring) = {
        let pane = Pane::new(Deck::Timeline, PANES[2]);
        let p = pane.stage();
        (p.hover, p.pressed, theme::tint(p.accent, 0.55))
    };
    let accent = Pane::new(Deck::Timeline, PANES[2]).stage().accent;
    let disc_at = |painted: &Painted, at: Pos2, fill: Color32| {
        (painted.discs.iter()).any(|(centre, radius, shown)| {
            *shown == fill && centre.distance(at) < 0.5 && *radius >= PLAY * 0.5 - 0.5
        })
    };

    // Play.
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    let rested = pane.rest();
    let play = pane.play(&rested).center();
    let painted = pane.hover(play);
    assert!(
        disc_at(&painted, play, hover),
        "play does not light on hover"
    );
    let painted = pane.hold(play);
    assert!(
        disc_at(&painted, play, pressed),
        "play does not darken while pressed"
    );
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    let painted = pane.focus("Play (Space)");
    assert!(
        (painted.rings.iter()).any(|(centre, radius, stroke)| centre.distance(play) < 0.5
            && (*radius - (PLAY * 0.5 + 2.0)).abs() < 0.01
            && *stroke == Stroke::new(2.0, ring)),
        "play has no focus ring: {:?}",
        painted.rings
    );

    // Speed, which reads in its place.
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    // Found by what it reads, which is centred in it to a point.
    let rested = pane.rest();
    let speed = pane.speed(&rested).shrink(1.0);
    let painted = pane.hover(speed.center());
    assert!(
        filled(&painted, speed, hover),
        "speed does not light on hover"
    );
    let painted = pane.hold(speed.center());
    assert!(
        filled(&painted, speed, pressed),
        "speed does not darken while pressed"
    );
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    let painted = pane.focus("Playback speed, 1×");
    assert!(
        (painted.rects.iter())
            .any(|(rect, _, _, stroke)| *stroke == Stroke::new(2.0, ring)
                && rect.contains_rect(speed)),
        "speed has no focus ring"
    );

    // Record, whose surface is its target's less a point all round.
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    let painted = pane.rest();
    let disc = record_disc(&pane, &painted).unwrap();
    let surface = Rect::from_center_size(disc, Vec2::splat(24.0));
    let painted = pane.hover(disc);
    assert!(
        filled(&painted, surface, hover),
        "record does not light on hover"
    );
    let painted = pane.hold(disc);
    assert!(
        filled(&painted, surface, pressed),
        "record does not darken while pressed"
    );
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    let painted = pane.focus("Record voiceover (R)");
    assert!(
        (painted.rects.iter()).any(|(rect, _, _, stroke)| *stroke == Stroke::new(1.5, accent)
            && rect.contains_rect(surface)),
        "record has no focus ring"
    );
}

#[test]
fn a_take_is_a_pill_with_its_level_in_the_colour_of_its_loudness() {
    // Spec §4.4.3: an 80 × 28 pill at r14, and a level bar green below -12
    // dBFS, yellow to -1 dB and red within 1 dB of full scale.
    for (level, ink) in [(0.1, "green"), (0.5, "yellow"), (0.95, "red")] {
        let mut pane = Pane::new(Deck::Timeline, PANES[2]);
        pane.record = RecordView::Recording {
            elapsed: Time::from_seconds(3.6),
            level,
        };
        let p = pane.stage();
        let painted = pane.rest();
        let pill = (painted.rects.iter())
            .find(|(_, _, fill, _)| *fill == theme::tint(p.red, 0.14))
            .unwrap_or_else(|| panic!("no take pill at level {level}"));
        assert_eq!(pill.0.size(), vec2(80.0, 28.0), "the pill's size");
        assert_eq!(pill.1, CornerRadius::same(14), "the pill's corners");
        let fill = match ink {
            "green" => p.green,
            "yellow" => p.yellow,
            _ => p.red,
        };
        let bar = |fill: Color32| {
            (painted.rects.iter()).find(|(rect, _, shown, _)| {
                *shown == fill && rect.height() == 4.0 && pill.0.contains_rect(*rect)
            })
        };
        let track = bar(theme::tint(p.fg, 0.14))
            .unwrap_or_else(|| panic!("no level track at level {level}"));
        assert_eq!(track.0.width(), 24.0);
        let lit = bar(fill).unwrap_or_else(|| panic!("the level at {level} is not {ink}"));
        assert!(lit.0.left() == track.0.left() && lit.0.width() <= track.0.width());
        // The time and the level are one block in the middle of what the
        // stop square leaves.
        let time = painted.text("0:03").unwrap();
        let middle = (pill.0.left() + 28.0 + pill.0.right() - 12.0) * 0.5;
        assert!((time.center().x - middle).abs() < 0.5);
        assert!((track.0.center().x - middle).abs() < 0.5);
    }
}

#[test]
fn a_deck_with_nothing_to_play_says_why_on_each_control() {
    // Spec §3.10 F12: a disabled control gives its reason on hover, in the
    // words of its deck.
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    pane.project = Project::new("Empty");
    let painted = pane.rest();
    for name in ["Play (Space)", "Next frame (→)", "Go to end (End)"] {
        let (rect, _) = painted.named(name).unwrap();
        let painted = pane.hover(rect.center());
        assert!(
            painted
                .text("Nothing to play: the timeline is empty")
                .is_some(),
            "{name} gives no reason on an empty timeline"
        );
    }
    let mut pane = Pane::new(Deck::Source, PANES[2]);
    pane.asset.duration = Time::ZERO;
    let painted = pane.rest();
    for name in ["Play (Space)", "Previous frame (←)"] {
        let (rect, enabled) = painted.named(name).unwrap();
        assert!(!enabled, "{name} can be used on a source of no length");
        let painted = pane.hover(rect.center());
        assert!(
            painted
                .text("Nothing to play: the timeline is empty")
                .is_none()
                && painted
                    .text("Nothing to play: the file's length is not known")
                    .is_some(),
            "{name} on the source deck does not say why it has nothing to play"
        );
    }
}

#[test]
fn an_empty_timeline_offers_the_import_only_where_the_library_does_not() {
    // Spec §5.3: "Import media…" shows only when the library is empty or
    // hidden.
    let media = Asset::new(
        PathBuf::from("/media/beside.mp4"),
        MediaKind::Video,
        Time::from_seconds(4.0),
    );
    for (library, holds, offered) in [
        (true, false, true),
        (true, true, false),
        (false, true, true),
        (false, false, true),
    ] {
        let mut pane = Pane::new(Deck::Timeline, PANES[2]);
        pane.project = Project::new("Empty");
        if holds {
            Arc::make_mut(&mut pane.project.assets).insert(media.id, media.clone());
        }
        pane.library = library;
        let painted = pane.rest();
        assert_eq!(
            painted.named("Import media…").is_some(),
            offered,
            "library shown {library}, holding media {holds}"
        );
    }
}

#[test]
fn without_a_media_service_the_stage_never_says_it_waits() {
    // Spec §5.3 and §8.1 #9: no frame is on its way without a service, so
    // the stage does not claim one is, however long it is looked at.
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    let clip = Clip::from_asset(&pane.asset, Time::ZERO);
    Arc::make_mut(&mut pane.project.assets).insert(pane.asset.id, pane.asset.clone());
    Arc::make_mut(&mut pane.project.tracks[0]).clips.push(clip);
    for _ in 0..90 {
        pane.pass(Vec::new());
    }
    let painted = pane.rest();
    assert!(painted.text("Waiting for frames…").is_none());
    let repaint = pane.ctx.has_requested_repaint();
    assert!(!repaint, "the stage keeps the window awake for no frame");
}

#[test]
fn a_missing_file_is_never_said_to_be_waited_for() {
    // Spec §5.3 and §8.1 #9: waiting and failed are not the same. A frame
    // the worker gave up is not on its way, so the stage does not say it
    // waits for it, however long it is looked at.
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    pane.service = Some(MediaService::new(|| {}).expect("FIXTURE: the media workers start"));
    pane.asset.path = std::env::temp_dir().join(format!(
        "neptune-video-missing-{}.mp4",
        uuid::Uuid::new_v4()
    ));
    let clip = Clip::from_asset(&pane.asset, Time::ZERO);
    Arc::make_mut(&mut pane.project.assets).insert(pane.asset.id, pane.asset.clone());
    Arc::make_mut(&mut pane.project.tracks[0]).clips.push(clip);
    let until = Instant::now() + Duration::from_secs(10);
    while pane.failures == 0 {
        assert!(
            Instant::now() < until,
            "FIXTURE: the worker never said the file is missing"
        );
        pane.pass(Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..90 {
        pane.pass(Vec::new());
    }
    let painted = pane.rest();
    assert!(
        painted.text("Waiting for frames…").is_none(),
        "the stage says it waits for a frame the worker gave up"
    );
}

#[test]
fn the_speeds_are_a_choice_that_tells_the_one_playing() {
    // Spec §5.7.1: a menu's choice rows are radio buttons, the one chosen
    // selected, with the check in the icon's place.
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    let painted = pane.rest();
    let (_, painted) = pane.click(pane.speed(&painted).center());
    let choices = painted.choices;
    // The nodes come in no particular order.
    let mut names: Vec<&str> = choices.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["0.25×", "0.5×", "1.5×", "1×", "2×", "4×"]);
    let chosen: Vec<&str> = (choices.iter())
        .filter(|(_, chosen)| *chosen)
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(chosen, ["1×"], "the speed playing is not the one chosen");
}

#[test]
fn the_source_header_is_reached_by_the_keyboard_as_it_reads() {
    // Spec §7.4: Tab follows reading order, so Add to timeline, left of
    // Close, comes first.
    for size in [PANES[1], PANES[2]] {
        let mut pane = Pane::new(Deck::Source, size);
        let painted = pane.rest();
        let (add, _) = painted
            .named("Add to timeline")
            .unwrap_or_else(|| panic!("FIXTURE: no Add to timeline at {size:?}"));
        let (close, _) = painted.named("Close preview (Esc)").unwrap();
        assert!(add.right() <= close.left(), "FIXTURE: Add reads first");
        let painted = pane.focus("Add to timeline");
        assert_eq!(painted.focused.as_deref(), Some("Add to timeline"));
        let tab = egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        pane.pass(vec![tab]);
        let (_, painted) = pane.pass(Vec::new());
        assert_eq!(
            painted.focused.as_deref(),
            Some("Close preview (Esc)"),
            "at {size:?}: Tab from Add to timeline does not go on to Close"
        );
    }
}

#[test]
fn a_canvas_modes_prompt_is_painted_over_the_frame_it_draws() {
    // Spec §5.3: the prompt is the canvas's top layer, over the picked
    // layer's frame.
    let mut pane = Pane::new(Deck::Timeline, PANES[2]);
    let clip = Clip::from_asset(&pane.asset, Time::ZERO);
    let id = clip.id;
    Arc::make_mut(&mut pane.project.assets).insert(pane.asset.id, pane.asset.clone());
    Arc::make_mut(&mut pane.project.tracks[0]).clips.push(clip);
    pane.viewer.pick_colour(Some(ColourPick {
        clip: id,
        effect: uuid::Uuid::new_v4(),
    }));
    let painted = pane.rest();
    let words = painted
        .text(PICK_PROMPT)
        .unwrap_or_else(|| panic!("no prompt among {:?}", painted.texts));
    let elevated = pane.stage().elevated;
    let capsule = (painted.rects.iter().zip(&painted.rects_at))
        .find(|((rect, _, fill, _), _)| *fill == elevated && rect.contains_rect(words))
        .map(|(_, at)| *at)
        .expect("the prompt has no capsule");
    let frame = *painted.frames_at.last().expect("FIXTURE: no frame drawn");
    assert!(
        capsule > frame,
        "the prompt (shape {capsule}) is under the frame (shape {frame})"
    );
}
