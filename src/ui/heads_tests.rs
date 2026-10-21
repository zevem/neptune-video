//! The track heads' mix controls, driven as a person drives them.
//!
//! [`Bench`] paints the timeline in a real egui pass and hands it pointer
//! and key events. A toggle, a menu row and the volume field are found by
//! the name accessibility is given for them, a toggle among those of its own
//! head. An [`Action::Edit`] is dispatched as the application does it, so
//! every edit below is one `Editor::dispatch` accepted.
use super::*;
use crate::theme::{Accent, Appearance};
use editor_core::{Editor, MediaKind};
use eframe::egui::{Event, Key, Modifiers, PointerButton, accesskit::Role};
use std::sync::Arc;

struct Seen {
    node: egui::accesskit::NodeId,
    name: String,
    rect: Rect,
    role: Role,
    disabled: bool,
    /// Whether a check box is checked.
    toggled: Option<egui::accesskit::Toggled>,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    editor: Editor,
    timeline: TimelineState,
    size: Vec2,
    time: f64,
    seen: Vec<Seen>,
    /// Every line of text the last frame painted, and where.
    written: Vec<(String, Rect)>,
    /// The colour of every line of text and glyph the last frame painted,
    /// and where.
    inked: Vec<(Rect, Color32)>,
    /// Every rectangle the last frame filled, and with what.
    filled: Vec<(Rect, Color32)>,
    /// Every rectangle the last frame stroked, and with what.
    stroked: Vec<(Rect, Color32)>,
    /// How many meshes the last frame painted.
    meshes: usize,
    /// What the timeline asked of the application that is not an edit,
    /// where a test lets it ask; anywhere else such a request fails it.
    asked: Option<Vec<Action>>,
    refused: Vec<String>,
    /// Whether the timeline is told snapping is on.
    snapping: bool,
    /// The node that held the keyboard focus after the last frame.
    focused: Option<egui::accesskit::NodeId>,
    /// Whether Left and Right were a control's own in the last frame, as
    /// the application is told: then they do not step the playhead.
    arrows_kept: bool,
}

const NAMES: [&str; 4] = ["Video 1", "Audio 1", "Audio 2", "Text 1"];

impl Bench {
    /// A video track, two audio tracks and a text track, the first three
    /// with a clip of media that has sound.
    fn new() -> Self {
        Self::of(Self::project(true))
    }

    /// The tracks of [`Bench::new`], with their clips or with none.
    fn project(clips: bool) -> Project {
        let mut project = Project::new("Heads");
        project.tracks.clear();
        for (name, kind) in NAMES.into_iter().zip([
            TrackKind::Video,
            TrackKind::Audio,
            TrackKind::Audio,
            TrackKind::Text,
        ]) {
            let mut track = Track::new(name, kind);
            if clips && kind != TrackKind::Text {
                let path = format!("/media/{name}.mp4");
                let mut asset = Asset::new(path.into(), MediaKind::Video, Time(4_000_000));
                asset.has_audio = true;
                track.clips.push(Clip::from_asset(&asset, Time::ZERO));
                Arc::make_mut(&mut project.assets).insert(asset.id, asset);
            }
            project.tracks.push(Arc::new(track));
        }
        project
    }

    fn of(project: Project) -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut bench = Self {
            ctx,
            palette,
            editor: Editor::new(project).expect("FIXTURE: the project is valid"),
            timeline: TimelineState::default(),
            size: vec2(900.0, 420.0),
            time: 0.0,
            seen: Vec::new(),
            written: Vec::new(),
            inked: Vec::new(),
            filled: Vec::new(),
            stroked: Vec::new(),
            meshes: 0,
            asked: None,
            refused: Vec::new(),
            snapping: false,
            focused: None,
            arrows_kept: false,
        };
        bench.frame(Vec::new());
        bench
    }

    fn track(&self, name: &str) -> &Track {
        (self.editor.project().tracks.iter())
            .find(|track| track.name == name)
            .expect("FIXTURE: the track")
    }

    /// A fixture's own edit: not one the timeline made.
    fn edit(&mut self, command: Command) {
        let label = command.label().to_owned();
        if let Err(error) = self.editor.dispatch(command) {
            panic!("FIXTURE: \"{label}\" was refused: {error}");
        }
        self.frame(Vec::new());
    }

    fn steps(&self) -> usize {
        self.editor.history().count()
    }

    /// One frame of the application with these events in it.
    fn frame(&mut self, events: Vec<Event>) {
        fn texts(shape: &egui::Shape, found: &mut Vec<(String, Rect)>, bench: &mut Bench) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    (shapes.iter()).for_each(|shape| texts(shape, found, bench))
                }
                egui::Shape::Text(text) => {
                    let rect = Rect::from_min_size(text.pos, text.galley.size());
                    found.push((text.galley.text().to_owned(), rect));
                    bench.inked.push((rect, text.fallback_color));
                }
                egui::Shape::Rect(rect) => {
                    bench.filled.push((rect.rect, rect.fill));
                    if rect.stroke.width > 0.0 {
                        bench.stroked.push((rect.rect, rect.stroke.color));
                    }
                }
                egui::Shape::Mesh(_) => bench.meshes += 1,
                _ => {}
            }
        }
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
        let mut arrows_kept = false;
        let mut output = ctx.run_ui(input, |ui| {
            // As the application reads them, before and after its widgets.
            arrows_kept = crate::ui::ChoiceArrows::begin(ui.ctx());
            show(
                ui,
                ui.max_rect(),
                &mut self.timeline,
                &mut Selection::default(),
                &mut Textures::default(),
                &mut HeldPeaks::default(),
                &TimelineInput {
                    p: &self.palette,
                    project: &project,
                    playhead: Time::ZERO,
                    snapping: self.snapping,
                    media: None,
                    can_paste: false,
                    freeze_hold: Time(2_000_000),
                    separator: crate::stems::Tool::Missing,
                },
                &mut actions,
            );
            crate::ui::ChoiceArrows::end(ui.ctx());
        });
        self.arrows_kept = arrows_kept;
        output.textures_delta.clear();
        let mut written = Vec::new();
        self.inked.clear();
        self.filled.clear();
        self.stroked.clear();
        self.meshes = 0;
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut written, self);
        }
        self.written = written;
        if let Some(update) = &output.platform_output.accesskit_update {
            self.focused = Some(update.focus);
            self.seen = (update.nodes.iter())
                .filter_map(|(id, node)| {
                    let bounds = node.bounds()?;
                    Some(Seen {
                        node: *id,
                        name: node.label().or_else(|| node.value())?.to_owned(),
                        rect: Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                        role: node.role(),
                        disabled: node.is_disabled(),
                        toggled: node.toggled(),
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
                other => match &mut self.asked {
                    Some(asked) => asked.push(other),
                    None => panic!("FIXTURE: the timeline asked for {other:?}"),
                },
            }
        }
    }

    /// The head of the track with this name, as the timeline lays it out.
    fn head(&self, track: &str) -> Rect {
        let heads = Rect::from_min_max(pos2(0.0, STRIP + RULER), pos2(HEAD_WIDTH, self.size.y));
        let rows = lay_out(display_order(self.editor.project()), heads.top() + LANE_GAP);
        let row = (rows.iter())
            .find(|row| row.track.name == track)
            .expect("FIXTURE: the track has a row");
        head_rect(heads, row)
    }

    fn controls(&self) -> impl Iterator<Item = &Seen> {
        (self.seen.iter()).filter(|seen| !matches!(seen.role, Role::Label | Role::TextRun))
    }

    /// The names of the toggles in the head of this track, the trailing one
    /// first and the upper of two in a column before the lower.
    fn toggles(&self, track: &str) -> Vec<&str> {
        let head = self.head(track);
        let mut found: Vec<&Seen> = (self.controls())
            .filter(|seen| seen.rect.width() == HEAD_CELL && head.contains_rect(seen.rect))
            .collect();
        found.sort_by(|a, b| {
            (b.rect.left().total_cmp(&a.rect.left())).then(a.rect.top().total_cmp(&b.rect.top()))
        });
        found.iter().map(|seen| seen.name.as_str()).collect()
    }

    /// The toggle with this name in the head of this track.
    fn toggle(&self, track: &str, name: &str) -> Option<&Seen> {
        let head = self.head(track);
        (self.controls()).find(|seen| seen.name == name && head.contains_rect(seen.rect))
    }

    fn find(&self, name: &str) -> Option<&Seen> {
        self.controls().find(|seen| seen.name == name)
    }

    fn has(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    fn button(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn click_at(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Self::button(pos, PointerButton::Primary, true)]);
        self.frame(vec![Self::button(pos, PointerButton::Primary, false)]);
        // The frame after the release shows what it committed.
        self.frame(Vec::new());
    }

    /// Clicks the toggle with this name in the head of this track.
    fn switch(&mut self, track: &str, name: &str) {
        self.frame(Vec::new());
        let Some(seen) = self.toggle(track, name) else {
            panic!(
                "FIXTURE: {track} has no \"{name}\" among {:?}",
                self.toggles(track)
            );
        };
        self.click_at(seen.rect.center());
    }

    fn click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = (self.find(name))
            .unwrap_or_else(|| panic!("FIXTURE: no control is named \"{name}\""))
            .rect
            .center();
        self.click_at(pos);
    }

    /// Opens the menu of this track's head, from the icon of its kind.
    fn open_menu(&mut self, track: &str) {
        let head = self.head(track);
        let pos = pos2(head.left() + 15.0, head.center().y);
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, true)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, false)]);
        self.frame(Vec::new());
        assert!(self.has("Add track of this kind"), "the menu is open");
    }

    /// Gives the control the keyboard focus, as a screen reader does.
    fn focus(&mut self, name: &str) {
        self.frame(Vec::new());
        let target_node = match self.find(name) {
            Some(seen) => seen.node,
            None => panic!("FIXTURE: no control is named \"{name}\""),
        };
        self.frame(vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Focus,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node,
                data: None,
            },
        )]);
        self.frame(Vec::new());
    }

    fn key(&mut self, key: Key) {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        self.frame(Vec::new());
    }

    /// Types this into the volume field of the open menu, without entering
    /// it.
    fn type_volume(&mut self, text: &str) {
        self.focus("Track volume");
        self.frame(vec![Event::Text(text.to_owned())]);
    }

    /// What the head of this track writes, top down.
    /// The words in a head, top first. Icons are glyphs of a font in the
    /// private use area and are not words.
    fn written_in(&self, track: &str) -> Vec<(&str, Rect)> {
        let head = self.head(track);
        let icon = |text: &str| {
            text.chars()
                .all(|c| matches!(c, '\u{e000}'..='\u{f8ff}' | '\u{f0000}'..='\u{10ffff}'))
        };
        let mut found: Vec<(&str, Rect)> = (self.written.iter())
            .filter(|(text, rect)| head.contains_rect(*rect) && !icon(text))
            .map(|(text, rect)| (text.as_str(), *rect))
            .collect();
        found.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
        found
    }
}

// ---- Where the toggles stand ---------------------------------------------

/// A head of this height in a column of heads as wide as the timeline's.
fn head_of(height: f32) -> Rect {
    let heads = Rect::from_min_size(Pos2::ZERO, vec2(HEAD_WIDTH, 400.0));
    let track = Track::new("Any", TrackKind::Audio);
    head_rect(
        heads,
        &Row {
            track: &track,
            top: 10.0,
            height,
            lane: 0.0,
        },
    )
}

/// How wide the head's own font writes this.
fn width_of(ctx: &egui::Context, text: &str, font: egui::FontId) -> f32 {
    let mut width = 0.0;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        width = (ui.painter())
            .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x;
    });
    // No renderer takes the font texture: epaint is told it is meant.
    output.textures_delta.clear();
    width
}

#[test]
fn the_toggles_of_every_head_fit_a_156_point_head_and_leave_its_name_room() {
    assert_eq!(HEAD_WIDTH, 156.0);
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    for (kind, name, toggles, columns) in [
        (TrackKind::Video, "Video 1", 4, 2),
        (TrackKind::Audio, "Audio 1", 3, 3),
        (TrackKind::Text, "Text 1", 2, 2),
    ] {
        let head = head_of(lane_height(kind));
        assert_eq!(head.width(), 146.0);
        assert_eq!(head_toggles(kind), toggles, "{kind:?}");
        let cells: Vec<Rect> = (0..toggles)
            .map(|index| head_cell(head, toggles, index))
            .collect();
        for (index, cell) in cells.iter().enumerate() {
            // A whole 22-point target, inside the head.
            assert_eq!(cell.size(), Vec2::splat(22.0), "{kind:?} {index}");
            assert!(head.contains_rect(*cell), "{kind:?} {index}: {cell:?}");
            for other in &cells[index + 1..] {
                assert!(!cell.intersects(*other), "{kind:?}: {cell:?} {other:?}");
            }
        }
        // The first is four points from the trailing edge, and the columns
        // are 23 apart.
        assert_eq!(cells[0].right(), head.right() - 4.0, "{kind:?}");
        let leftmost = (cells.iter().map(|cell| cell.left())).fold(f32::INFINITY, f32::min);
        assert_eq!(
            leftmost,
            head.right() - 4.0 - 22.0 - (columns - 1) as f32 * 23.0,
            "{kind:?}"
        );
        // The name has what is between the icon of the kind and the cells,
        // less two points: the name a new track is given is written whole.
        let room = head_name_room(head, toggles);
        assert_eq!(room.min, head.left() + 28.0);
        assert_eq!(room.max, leftmost - 1.0 - 2.0, "{kind:?}");
        let name_width = width_of(&ctx, name, theme::medium(12.0));
        assert!(
            name_width <= room.span(),
            "{name}: {name_width} in {}",
            room.span()
        );
        // And the longest volume a head says under it, in Regular 11 (§4.4.5).
        let figure = width_of(&ctx, "800%", theme::regular(11.0));
        assert!(figure <= room.span(), "{kind:?}: {figure}");
    }

    // An audio head: one row, solo beside mute beside lock, 43 points of
    // name as it had with two toggles less one cell.
    let head = head_of(lane_height(TrackKind::Audio));
    let [lock, mute, solo] = [0, 1, 2].map(|index| head_cell(head, 3, index));
    assert_eq!((lock.top(), mute.top(), solo.top()), (21.0, 21.0, 21.0));
    assert_eq!(lock.center().y, head.center().y);
    assert_eq!(mute.right() + 1.0, lock.left());
    assert_eq!(solo.right() + 1.0, mute.left());
    assert_eq!(head_name_room(head, 3).span(), 43.0);

    // A video head has four: side by side they would leave its name twenty
    // points, which does not hold "Video 1".
    let head = head_of(lane_height(TrackKind::Video));
    let side_by_side = 146.0 - 4.0 - 4.0 * 23.0 - 28.0 - 2.0;
    assert_eq!(side_by_side, 20.0);
    assert!(width_of(&ctx, "Video 1", theme::medium(12.0)) > side_by_side);
    // So they stand in two columns, each two high and centred in the head:
    // hide under lock, solo under mute.
    let [lock, hide, mute, solo] = [0, 1, 2, 3].map(|index| head_cell(head, 4, index));
    assert_eq!(head_lines(head, 4), 2);
    assert_eq!(
        (lock.left(), hide.left()),
        (mute.right() + 1.0, mute.right() + 1.0)
    );
    assert_eq!(mute.left(), solo.left());
    assert_eq!(
        (hide.top(), solo.top()),
        (lock.bottom() + 1.0, mute.bottom() + 1.0)
    );
    assert_eq!(lock.top() - head.top(), head.bottom() - hide.bottom());
    assert_eq!(head_name_room(head, 4).span(), 66.0);

    // A head too low for two rows keeps four in one, still inside it.
    let low = head_of(44.0);
    assert_eq!(head_lines(low, 4), 1);
    let cells = [0, 1, 2, 3].map(|index| head_cell(low, 4, index));
    assert!(cells.iter().all(|cell| low.contains_rect(*cell)));
    assert!(
        cells
            .windows(2)
            .all(|pair| pair[1].right() + 1.0 == pair[0].left())
    );
    assert_eq!(head_name_room(low, 4).span(), 20.0);
    // And the room is never less than nothing.
    assert_eq!(head_name_room(low, 9).span(), 0.0);
}

#[test]
fn a_head_has_solo_where_it_has_mute_and_a_text_track_has_neither() {
    let bench = Bench::new();
    // The trailing column first, top down.
    assert_eq!(
        bench.toggles("Video 1"),
        ["Lock track", "Hide track", "Mute track", "Solo track"]
    );
    for audio in ["Audio 1", "Audio 2"] {
        assert_eq!(
            bench.toggles(audio),
            ["Lock track", "Mute track", "Solo track"]
        );
    }
    assert_eq!(bench.toggles("Text 1"), ["Lock track", "Hide track"]);
    // Each stands where the arithmetic puts it.
    for (track, kind) in [
        ("Video 1", TrackKind::Video),
        ("Audio 2", TrackKind::Audio),
        ("Text 1", TrackKind::Text),
    ] {
        let head = bench.head(track);
        let count = head_toggles(kind);
        for (index, name) in bench.toggles(track).into_iter().enumerate() {
            let seen = bench.toggle(track, name).unwrap();
            assert_eq!(seen.rect, head_cell(head, count, index), "{track} {name}");
        }
    }
}

// ---- Solo ----------------------------------------------------------------

#[test]
fn each_toggle_of_a_head_is_one_step_that_undoes() {
    let mut bench = Bench::new();
    let before = bench.editor.project().clone();
    let read = |bench: &Bench| {
        let track = bench.track("Video 1");
        [track.locked, track.hidden, track.muted, track.solo]
    };
    for (index, (press, again)) in [
        ("Lock track", "Unlock track"),
        ("Hide track", "Show track"),
        ("Mute track", "Unmute track"),
        ("Solo track", "Unsolo track"),
    ]
    .into_iter()
    .enumerate()
    {
        let steps = bench.steps();
        bench.switch("Video 1", press);
        assert_eq!(bench.steps(), steps + 1, "{press}: {:?}", bench.refused);
        let mut wanted = [false; 4];
        wanted[index] = true;
        assert_eq!(read(&bench), wanted, "{press} changes that and no more");
        // The cell now offers the way back, and that is a step too.
        assert!(bench.toggle("Video 1", again).is_some(), "{again}");
        bench.switch("Video 1", again);
        assert_eq!(bench.steps(), steps + 2, "{again}");
        assert_eq!(read(&bench), [false; 4]);
        assert!(bench.editor.undo());
        assert_eq!(read(&bench), wanted, "{again} undoes by itself");
        assert!(bench.editor.undo());
        assert_eq!(*bench.editor.project(), before);
        bench.frame(Vec::new());
    }
    assert!(bench.refused.is_empty());
}

#[test]
fn a_locked_track_is_still_soloed_from_its_head() {
    let mut bench = Bench::new();
    bench.switch("Audio 1", "Lock track");
    bench.switch("Audio 1", "Solo track");
    let track = bench.track("Audio 1");
    assert!(track.locked && track.solo, "{:?}", bench.refused);
}

#[test]
fn a_head_silenced_by_a_solo_says_so_where_the_mix_leaves_its_track_out() {
    let mut bench = Bench::new();
    const SILENCED: &str = "Mute track (silenced by a solo)";
    // What each head with sound shows agrees with what the mix admits.
    let agrees = |bench: &Bench| {
        let project = bench.editor.project();
        for name in &NAMES[..3] {
            let track = bench.track(name);
            let sounds = sound::sounds(project, track, &track.clips[0]);
            let dimmed = bench.toggle(name, SILENCED).is_some();
            assert_eq!(dimmed, !track.muted && !sounds, "{name}");
            let muted = bench.toggle(name, "Unmute track").is_some();
            assert_eq!(muted, track.muted, "{name}");
            let open = bench.toggle(name, "Mute track").is_some();
            assert_eq!(open, sounds, "{name}");
        }
    };
    agrees(&bench);
    assert!(!bench.has(SILENCED));

    bench.switch("Audio 1", "Solo track");
    agrees(&bench);
    assert!(bench.toggle("Audio 1", "Mute track").is_some());
    assert!(bench.toggle("Video 1", SILENCED).is_some());
    assert!(bench.toggle("Audio 2", SILENCED).is_some());
    // A second solo is heard beside the first.
    bench.switch("Video 1", "Solo track");
    agrees(&bench);
    assert!(bench.toggle("Video 1", "Mute track").is_some());
    // A dimmed cell still mutes, and a muted track shows its mute.
    let steps = bench.steps();
    bench.switch("Audio 2", SILENCED);
    assert_eq!(bench.steps(), steps + 1);
    assert!(bench.track("Audio 2").muted);
    agrees(&bench);
    // Mute wins over solo: a soloed track that is muted shows its mute, and
    // still silences the rest.
    bench.switch("Video 1", "Unsolo track");
    bench.switch("Audio 1", "Mute track");
    agrees(&bench);
    assert!(bench.toggle("Video 1", SILENCED).is_some());
    // With the last solo gone no head is dimmed.
    bench.switch("Audio 1", "Unsolo track");
    agrees(&bench);
    assert!(!bench.has(SILENCED));
    // The solo of a text track, which only a file or an agent can set,
    // silences nothing and dims nothing.
    let text = bench.track("Text 1").clone();
    bench.edit(tracks::solo(&text, true));
    agrees(&bench);
    assert!(!bench.has(SILENCED));
    assert!(bench.refused.is_empty());
}

// ---- Volume --------------------------------------------------------------

#[test]
fn a_click_on_the_volume_field_keeps_the_menu_and_takes_what_is_typed() {
    let mut bench = Bench::new();
    let steps = bench.steps();
    bench.open_menu("Audio 1");
    // Clicked as a pointer clicks it: the menu stays and the field is
    // typed into.
    bench.click("Track volume");
    assert!(bench.has("Track volume"), "a click in the menu keeps it");
    bench.frame(vec![Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    bench.frame(vec![Event::Text("70".to_owned())]);
    bench.key(Key::Enter);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.track("Audio 1").gain, 0.7);
    assert!(!bench.has("Track volume"), "entered, the menu is done");
    // A click outside it still closes it, and edits nothing.
    bench.open_menu("Audio 1");
    let away = bench.head("Audio 1").right_bottom() + vec2(300.0, -4.0);
    bench.click_at(away);
    assert!(!bench.has("Track volume"));
    assert_eq!(bench.steps(), steps + 1);
}

#[test]
fn a_volume_entered_in_the_head_menu_is_one_step_held_to_what_a_track_takes() {
    let mut bench = Bench::new();
    let steps = bench.steps();
    bench.open_menu("Audio 1");
    assert!(bench.find("Reset volume").is_some_and(|row| row.disabled));
    // Typed, nothing is edited until it is entered.
    bench.type_volume("999");
    assert_eq!(bench.steps(), steps);
    assert_eq!(bench.track("Audio 1").gain, 1.0);
    bench.key(Key::Enter);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.track("Audio 1").gain, 8.0, "held to the bound");
    assert!(!bench.has("Track volume"), "entered, the menu is done");
    // One track's volume, and nothing else of it.
    let track = bench.track("Audio 1");
    assert!(!track.locked && !track.muted && !track.solo);
    assert!(
        NAMES
            .iter()
            .all(|name| *name == "Audio 1" || bench.track(name).gain == 1.0)
    );

    // Half, nothing, and a part of a percent: what is typed is what is
    // kept, and what the field and the head then read.
    for (typed, gain) in [("50", 0.5), ("0", 0.0), ("12.4", 0.124)] {
        let steps = bench.steps();
        bench.open_menu("Audio 1");
        bench.type_volume(typed);
        bench.key(Key::Enter);
        assert_eq!(bench.steps(), steps + 1, "{typed}: {:?}", bench.refused);
        assert_eq!(bench.track("Audio 1").gain, gain, "{typed}");
    }
    assert_eq!(bench.written_in("Audio 1")[1].0, "12.4%");
    // What the field already reads is no step, and neither is what is typed
    // and given up.
    let steps = bench.steps();
    bench.open_menu("Audio 1");
    bench.type_volume("12.4");
    bench.key(Key::Enter);
    bench.key(Key::Escape);
    bench.open_menu("Audio 1");
    bench.type_volume("300");
    bench.key(Key::Escape);
    bench.key(Key::Escape);
    assert!(!bench.has("Track volume"));
    assert_eq!(bench.steps(), steps);
    assert_eq!(bench.track("Audio 1").gain, 0.124);

    // Each volume undoes by itself.
    assert!(bench.editor.undo());
    assert_eq!(bench.track("Audio 1").gain, 0.0);
    bench.frame(Vec::new());

    // An arrow in the field is a percent and a step, and the menu stays for
    // the next.
    let steps = bench.steps();
    bench.open_menu("Audio 1");
    bench.focus("Track volume");
    bench.key(Key::ArrowUp);
    bench.key(Key::ArrowUp);
    assert_eq!(bench.steps(), steps + 2, "{:?}", bench.refused);
    assert_eq!(bench.track("Audio 1").gain, 0.02);
    assert!(bench.has("Track volume"));
    // Under nothing there is nothing.
    bench.key(Key::ArrowDown);
    bench.key(Key::ArrowDown);
    bench.key(Key::ArrowDown);
    assert_eq!(bench.steps(), steps + 4);
    assert_eq!(bench.track("Audio 1").gain, 0.0);
    bench.key(Key::Escape);
    bench.key(Key::Escape);
    assert!(!bench.has("Track volume"));

    // Back to the volume a track starts with, from the menu.
    let steps = bench.steps();
    bench.open_menu("Audio 1");
    assert!(bench.find("Reset volume").is_some_and(|row| !row.disabled));
    bench.click("Reset volume");
    assert_eq!(bench.steps(), steps + 1);
    assert_eq!(bench.track("Audio 1").gain, 1.0);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_volume_that_reads_whole_is_set_to_the_whole_it_reads() {
    let mut bench = Bench::new();
    // A part of a percent still sounds, and the head says so.
    bench.open_menu("Audio 1");
    bench.type_volume("0.4");
    bench.key(Key::Enter);
    assert_eq!(bench.track("Audio 1").gain, 0.004);
    assert_eq!(bench.written_in("Audio 1")[1].0, "0.4%");
    // Nothing, entered over it, is nothing.
    let steps = bench.steps();
    bench.open_menu("Audio 1");
    bench.type_volume("0");
    bench.key(Key::Enter);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.track("Audio 1").gain, 0.0);
    assert!(!bench.has("Track volume"), "entered, the menu is done");
    assert_eq!(bench.written_in("Audio 1")[1].0, "0%");

    // A volume finer than the field writes, as a file or an agent leaves
    // it: the field reads "0" and "100" of them, and takes either typed.
    for (held, typed, gain) in [(0.000_04, "0", 0.0), (1.000_04, "100", 1.0)] {
        let track = bench.track("Audio 2").clone();
        bench.edit(Command::SetTrackState {
            track_id: track.id,
            locked: None,
            hidden: None,
            muted: None,
            name: None,
            gain: Some(held),
            solo: None,
        });
        let said = bench.written_in("Audio 2")[1].0;
        assert!(said != "0%" && said != "100%", "{held}: {said}");
        let steps = bench.steps();
        bench.open_menu("Audio 2");
        bench.type_volume(typed);
        bench.key(Key::Enter);
        assert_eq!(bench.steps(), steps + 1, "{held}: {:?}", bench.refused);
        assert_eq!(bench.track("Audio 2").gain, gain, "{held}");
    }
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn escape_gives_up_a_volume_being_dragged_in_the_head_menu() {
    let mut bench = Bench::new();
    let steps = bench.steps();
    bench.open_menu("Video 1");
    let from = bench.find("Track volume").unwrap().rect.center();
    bench.frame(vec![Event::PointerMoved(from)]);
    bench.frame(vec![Bench::button(from, PointerButton::Primary, true)]);
    for step in 1..=6 {
        bench.frame(vec![Event::PointerMoved(
            from + vec2(step as f32 * 10.0, 0.0),
        )]);
    }
    // Escape with the button still down, then the frames after it and the
    // release that comes too late to be one.
    bench.key(Key::Escape);
    let to = from + vec2(60.0, 0.0);
    bench.frame(vec![Bench::button(to, PointerButton::Primary, false)]);
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), steps, "{:?}", bench.refused);
    assert_eq!(bench.track("Video 1").gain, 1.0);
    assert!(bench.refused.is_empty());
}

#[test]
fn a_volume_dragged_in_the_head_menu_is_one_step_when_it_is_let_go() {
    let mut bench = Bench::new();
    let steps = bench.steps();
    bench.open_menu("Video 1");
    let from = bench.find("Track volume").unwrap().rect.center();
    bench.frame(vec![Event::PointerMoved(from)]);
    bench.frame(vec![Bench::button(from, PointerButton::Primary, true)]);
    for step in 1..=6 {
        let at = from + vec2(step as f32 * 10.0, 0.0);
        bench.frame(vec![Event::PointerMoved(at)]);
        // Held, the project is as it was.
        assert_eq!(bench.steps(), steps);
        assert_eq!(bench.track("Video 1").gain, 1.0);
    }
    let to = from + vec2(60.0, 0.0);
    bench.frame(vec![Bench::button(to, PointerButton::Primary, false)]);
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    let gain = bench.track("Video 1").gain;
    assert!(gain > 1.0 && gain <= 8.0, "{gain}");
    // The menu stays for another drag, and reads what was let go.
    assert!(bench.has("Track volume"));
    assert!(bench.find("Reset volume").is_some_and(|row| !row.disabled));
    assert!(bench.editor.undo());
    assert_eq!(bench.track("Video 1").gain, 1.0);
    assert!(bench.refused.is_empty());
}

#[test]
fn a_text_track_has_no_volume_in_its_menu() {
    let mut bench = Bench::new();
    bench.open_menu("Text 1");
    assert!(bench.has("Remove track"));
    assert!(!bench.has("Track volume") && !bench.has("Reset volume"));
    bench.key(Key::Escape);
    bench.open_menu("Video 1");
    assert!(bench.has("Track volume") && bench.has("Reset volume"));
}

#[test]
fn a_head_says_under_its_name_a_volume_that_is_not_the_one_it_starts_with() {
    let mut bench = Bench::new();
    for name in NAMES {
        let written = bench.written_in(name);
        let [(only, at)] = written.as_slice() else {
            panic!("{name}: {written:?}");
        };
        assert_eq!(*only, name);
        // Alone, the name is at the middle of the head.
        let head = bench.head(name);
        assert!((at.center().y - head.center().y).abs() < 0.01, "{name}");
    }
    for (name, percent, said) in [("Video 1", 150.0, "150%"), ("Audio 2", 0.0, "0%")] {
        let track = bench.track(name).clone();
        bench.edit(tracks::volume(&track, percent).unwrap());
        let (head, written) = (bench.head(name), bench.written_in(name));
        let [(first, above), (second, below)] = written.as_slice() else {
            panic!("{name}: {written:?}");
        };
        assert_eq!((*first, *second), (name, said));
        // Under the name, from where the name starts, clear of the toggles
        // and inside the head.
        assert!(above.bottom() <= below.top() + 2.0, "{above:?} {below:?}");
        assert_eq!(above.left(), below.left());
        let room = head_name_room(head, head_toggles(track.kind));
        assert!(below.right() <= room.max, "{below:?}");
        assert!(head.contains_rect(*above) && head.contains_rect(*below));
    }
    // The others say nothing.
    assert_eq!(bench.written_in("Audio 1").len(), 1);
    // A head too low for two lines would keep to its name: the lowest head
    // that carries sound is tall enough.
    assert!(lane_height(TrackKind::Audio) >= HEAD_FIGURE);
    assert!(lane_height(TrackKind::Video) >= HEAD_FIGURE);
}

// §4.4.5, with the owner's decision on Solo: a soloed head's Solo cell is
// pressed and its Headphones glyph is in the accent, not yellow. Off, the
// cell is bare and its glyph muted.
#[test]
fn a_soloed_head_shows_its_solo_cell_pressed_with_its_glyph_in_the_accent() {
    let mut bench = Bench::new();
    let p = bench.palette;
    let cell = (bench.toggle("Audio 1", "Solo track"))
        .expect("FIXTURE: Audio 1 has a solo")
        .rect;
    let glyphs = |bench: &Bench| -> Vec<Color32> {
        (bench.inked.iter())
            .filter(|(rect, _)| cell.contains_rect(*rect))
            .map(|(_, ink)| *ink)
            .collect()
    };
    let pressed = |bench: &Bench| {
        (bench.filled.iter()).any(|(rect, fill)| cell.contains_rect(*rect) && *fill == p.pressed)
    };
    assert_eq!(glyphs(&bench), [p.muted]);
    assert!(!pressed(&bench));
    bench.switch("Audio 1", "Solo track");
    assert!(bench.track("Audio 1").solo, "{:?}", bench.refused);
    assert_eq!(glyphs(&bench), [p.accent]);
    assert!(pressed(&bench));
}

// §7.2: the tool strip's snapping toggle is "Snapping (N)", a check box that
// says whether snapping is on, and its zoom is the slider "Timeline zoom".
#[test]
fn the_tool_strip_names_its_snapping_toggle_and_its_zoom() {
    use egui::accesskit::Toggled;
    let mut bench = Bench::new();
    let snapping = bench.find("Snapping (N)").expect("the snapping toggle");
    assert_eq!(snapping.role, Role::CheckBox);
    assert_eq!(snapping.toggled, Some(Toggled::False));
    assert!(snapping.rect.bottom() <= STRIP, "{:?}", snapping.rect);
    let zoom = bench.find("Timeline zoom").expect("the zoom slider");
    assert_eq!(zoom.role, Role::Slider);
    assert!(zoom.rect.bottom() <= STRIP, "{:?}", zoom.rect);

    // Turned on, it keeps its name: only the check box says so.
    bench.snapping = true;
    bench.frame(Vec::new());
    let snapping = bench.find("Snapping (N)").expect("the snapping toggle, on");
    assert_eq!(snapping.role, Role::CheckBox);
    assert_eq!(snapping.toggled, Some(Toggled::True));
    assert!(!bench.has("Snapping is on (N)"));
    assert!(!bench.has("Snapping is off (N)"));
}

// §7.3: the arrows are the zoom slider's while it has the keyboard. Each one
// steps the zoom, and the focus stays on the slider for the next.
#[test]
fn the_arrows_step_the_zoom_slider_and_leave_the_focus_on_it() {
    let mut bench = Bench::new();
    bench.focus("Timeline zoom");
    let slider = bench.find("Timeline zoom").expect("the zoom slider").node;
    assert_eq!(bench.focused, Some(slider));
    let mut zoom = bench.timeline.zoom;
    for press in 0..2 {
        bench.key(Key::ArrowRight);
        assert!(bench.timeline.zoom > zoom, "press {press}: the zoom rose");
        assert_eq!(
            bench.focused,
            Some(slider),
            "press {press}: the focus stayed"
        );
        zoom = bench.timeline.zoom;
    }
    bench.key(Key::ArrowLeft);
    assert!(bench.timeline.zoom < zoom, "the left arrow lowers it");
    assert_eq!(bench.focused, Some(slider));
}

// P5-R2-1: while the zoom slider has the keyboard, Left and Right are its
// own, so the application does not also step the playhead with them. Once
// the keyboard leaves it they are the application's again.
#[test]
fn the_zoom_sliders_arrows_do_not_also_step_the_playhead() {
    let mut bench = Bench::new();
    let arrow = |key| Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    bench.frame(vec![arrow(Key::ArrowRight)]);
    assert!(!bench.arrows_kept, "nothing has the keyboard");
    bench.focus("Timeline zoom");
    let zoom = bench.timeline.zoom;
    bench.frame(vec![arrow(Key::ArrowRight)]);
    assert!(bench.timeline.zoom > zoom, "the zoom rose");
    assert!(bench.arrows_kept, "the arrow was the slider's alone");
    bench.frame(vec![arrow(Key::ArrowLeft)]);
    assert!(bench.arrows_kept);
    bench.ctx.memory_mut(|memory| {
        if let Some(focused) = memory.focused() {
            memory.surrender_focus(focused);
        }
    });
    bench.frame(Vec::new());
    bench.frame(vec![arrow(Key::ArrowRight)]);
    assert!(!bench.arrows_kept, "the keyboard left the slider");
}

// TL4 and §7.2: in a pane too narrow for the whole tool strip, the Edit and
// Insert groups fold into "More timeline actions", whose rows ask for what
// their buttons did. What never folds keeps a whole target in the strip.
#[test]
fn a_narrow_strip_folds_its_edit_and_insert_groups_into_one_menu() {
    let mut bench = Bench::new();
    bench.size = vec2(300.0, 420.0);
    bench.asked = Some(Vec::new());
    bench.frame(Vec::new());
    assert_eq!(Fold::of(bench.size.x), Fold::Edit);
    for gone in [
        "Split at playhead (S)",
        "Add marker at playhead (M)",
        "Add text at playhead (T)",
        "Add track",
    ] {
        assert!(!bench.has(gone), "\"{gone}\" folded away");
    }
    let kept = [
        "Keyframes of the selected clip (U)",
        "Graph editor (G)",
        "Snapping (N)",
        "Zoom out",
        "Zoom in",
        "Fit timeline to window (Shift+Z)",
        "More timeline actions",
    ];
    for name in kept {
        let seen = (bench.controls())
            .find(|seen| seen.name.starts_with(name))
            .unwrap_or_else(|| panic!("\"{name}\" is in the strip"));
        let rect = seen.rect;
        assert!(rect.width() >= 28.0, "{name}: {rect:?}");
        assert!(
            rect.right() <= bench.size.x && rect.bottom() <= STRIP,
            "{name}: {rect:?}"
        );
    }

    // Its rows are what folded, and one asks for what its button did.
    bench.click("More timeline actions");
    for row in ["Split at playhead", "Add text at playhead", "Add track"] {
        assert!(bench.has(row), "the menu has \"{row}\"");
    }
    bench.click("Add marker at playhead");
    assert!(
        matches!(bench.asked.as_deref(), Some([Action::AddMarker])),
        "{:?}",
        bench.asked
    );
    assert!(!bench.has("Add text at playhead"), "the menu closed");
}

// §9.12: a head cell under the pointer is filled `hover`, and one the
// keyboard reaches with Tab wears the focus ring.
#[test]
fn a_head_cell_is_lit_under_the_pointer_and_ringed_when_tab_reaches_it() {
    let mut bench = Bench::new();
    let p = bench.palette;
    let seen = (bench.toggle("Audio 1", "Solo track")).expect("FIXTURE: Audio 1 has a solo");
    let (cell, node) = (seen.rect, seen.node);
    let lit =
        |bench: &Bench| (bench.filled.iter()).any(|(rect, fill)| *rect == cell && *fill == p.hover);
    assert!(!lit(&bench));
    bench.frame(vec![Event::PointerMoved(cell.center())]);
    for _ in 0..10 {
        bench.frame(Vec::new());
    }
    assert!(lit(&bench), "the cell under the pointer is lit");
    bench.frame(vec![Event::PointerMoved(pos2(400.0, 400.0))]);
    for _ in 0..10 {
        bench.frame(Vec::new());
    }
    assert!(!lit(&bench), "and dark again once the pointer leaves");

    let ring = theme::tint(p.accent, 0.55);
    let ringed = |bench: &Bench| {
        (bench.stroked.iter()).any(|(rect, stroke)| *rect == cell.expand(2.0) && *stroke == ring)
    };
    assert!(!ringed(&bench));
    let mut presses = 0;
    while bench.focused != Some(node) {
        bench.key(Key::Tab);
        presses += 1;
        assert!(presses < 60, "Tab reaches the solo cell of Audio 1");
    }
    assert!(ringed(&bench), "the cell the keyboard is on is ringed");
}

// §5.5.7: an empty timeline shows one ghost row saying where media goes, in
// the muted ink and drawn in lines: no mesh.
#[test]
fn an_empty_timeline_shows_a_ghost_row_and_paints_no_mesh() {
    let bench = Bench::of(Bench::project(false));
    let p = bench.palette;
    let (_, said) = (bench.written.iter())
        .find(|(text, _)| text == "Drag media here")
        .expect("the ghost row says where media goes");
    assert!(said.left() >= HEAD_WIDTH, "in the lanes: {said:?}");
    assert!(
        (bench.inked.iter()).any(|(rect, ink)| rect == said && *ink == p.muted),
        "in the muted ink"
    );
    assert_eq!(bench.meshes, 0);

    // A timeline with a clip has no ghost row.
    let bench = Bench::new();
    assert!(!(bench.written.iter()).any(|(text, _)| text == "Drag media here"));
}
