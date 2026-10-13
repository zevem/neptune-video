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
    textures::Textures,
    theme::{self, Accent, Appearance, Palette},
    ui::{Action, ClipDraft, Selection, Transport},
};
use editor_core::{Asset, FrameRate, MediaKind, Project, Time};
use eframe::egui::{self, Pos2, Rect, Shape, Vec2, vec2};
use std::path::PathBuf;

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
}

impl Pane {
    fn new(deck: Deck, size: Vec2) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
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
            project: Project::new("Transport"),
            asset,
            textures: Textures::default(),
            viewer: ViewerState::default(),
            selection: Selection::default(),
            draft: None,
            playing: false,
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

    /// Where the play disc was painted: the one large disc in the accent.
    fn play(&self, painted: &Painted) -> Rect {
        let mut found = painted
            .discs
            .iter()
            .filter(|(_, radius, fill)| *radius >= 12.0 && *fill == self.palette.accent);
        let Some((centre, radius, _)) = found.next() else {
            panic!(
                "ISSUE14: no play control is painted in a pane of {:?}",
                self.size
            );
        };
        assert!(
            found.next().is_none(),
            "FIXTURE: more than one disc looks like the play control"
        );
        Rect::from_center_size(*centre, Vec2::splat(radius * 2.0))
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
