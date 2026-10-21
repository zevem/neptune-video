//! Transitions, and the library's effects, transitions and looks, driven as
//! a person drives them.
//!
//! [`Bench`] paints the library, the timeline and the inspector side by side
//! in a real egui pass and hands them pointer and key events. A control is
//! found by the name accessibility is given for it. The bench ends a frame
//! as the application does: an [`Action::Edit`] is dispatched, an
//! [`Action::AddTransitions`] is resolved by [`asked`], a draft is committed
//! when nothing holds it, and an Escape gives up what is held before the
//! frame is drawn. Every edit below is one `Editor::dispatch` accepted.
use super::*;
use crate::{
    textures::Textures,
    theme::{self, Accent, Appearance, Palette},
    ui::{
        Action, ClipDraft, EASINGS, Selection, browser,
        inspector::{self, InspectorInput, InspectorState, ProjectDraft},
        library::{self, LibraryInput, LibraryState, Tab},
        timeline::{self, TimelineInput, TimelineState},
    },
};
use editor_core::{Asset, Clip, Editor, EffectKind, MediaKind, ParamKey, TrackId, look};
use eframe::egui::{
    self, Event, Key, Modifiers, PointerButton, Pos2, Rect, Vec2, accesskit::Role, pos2, vec2,
};
use std::{path::PathBuf, time::Duration};

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

struct Seen {
    node: egui::accesskit::NodeId,
    name: String,
    rect: Rect,
    role: Role,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    textures: Textures,
    editor: Editor,
    library: LibraryState,
    timeline: TimelineState,
    inspector: InspectorState,
    draft: Option<ClipDraft>,
    project_draft: ProjectDraft,
    hold: crate::ui::DraftHold,
    selection: Selection,
    playhead: Time,
    /// The defaults the person kept, as the settings hold them.
    saved: Vec<Transition>,
    size: Vec2,
    /// The widths of the library and of the inspector.
    sides: (f32, f32),
    time: f64,
    seen: Vec<Seen>,
    refused: Vec<String>,
    said: Vec<String>,
    wait: Duration,
    titles: crate::titles::Titles,
    presets: std::sync::Arc<crate::presets::Presets>,
    /// What the last frame painted.
    shapes: Vec<egui::epaint::ClippedShape>,
}

const SHOT_START: &str = "Dissolve at the start of shot.mp4";

impl Bench {
    /// Two clips that meet on the first video track: `shot.mp4` from one
    /// second to five with a half second dissolve at its start, then
    /// `next.mp4` to eight. The first is selected and the playhead is at
    /// its start.
    fn new() -> (Self, ClipId, ClipId) {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut bench = Self {
            ctx,
            palette,
            textures: Textures::default(),
            editor: Editor::new(Project::new("Transitions")).expect("a new project is valid"),
            library: LibraryState::default(),
            timeline: TimelineState::default(),
            inspector: InspectorState::default(),
            draft: None,
            project_draft: ProjectDraft::default(),
            hold: crate::ui::DraftHold::default(),
            selection: Selection::default(),
            playhead: seconds(1.0),
            saved: Vec::new(),
            size: vec2(1400.0, 760.0),
            sides: (300.0, 300.0),
            time: 0.0,
            seen: Vec::new(),
            refused: Vec::new(),
            said: Vec::new(),
            wait: Duration::ZERO,
            titles: Default::default(),
            presets: Default::default(),
            shapes: Vec::new(),
        };
        let track_id = bench.track(TrackKind::Video);
        let mut ids = Vec::new();
        for (name, start, length) in [("shot.mp4", 1.0, 4.0), ("next.mp4", 5.0, 3.0)] {
            let mut asset = Asset::new(
                PathBuf::from(format!("/media/{name}")),
                MediaKind::Video,
                seconds(10.0),
            );
            asset.name = name.to_owned();
            asset.has_audio = false;
            bench.edit(Command::AddAssets {
                assets: vec![asset.clone()],
            });
            let mut clip = Clip::from_asset(&asset, seconds(start));
            clip.duration = seconds(length);
            ids.push(clip.id);
            bench.edit(Command::AddClip { track_id, clip });
        }
        bench.edit(set_command(
            ids[0],
            TransitionEdge::In,
            Some(default_of(&[], DEFAULT_KIND)),
        ));
        bench.selection.only(ids[0]);
        bench.frame(Vec::new());
        bench.frame(Vec::new());
        (bench, ids[0], ids[1])
    }

    fn track(&self, kind: TrackKind) -> TrackId {
        (self.editor.project().tracks.iter())
            .find(|track| track.kind == kind)
            .map(|track| track.id)
            .expect("FIXTURE: a new project has a track of each kind")
    }

    /// A fixture's own edit: not one the panes made.
    fn edit(&mut self, command: Command) {
        let label = command.label().to_owned();
        if let Err(error) = self.editor.dispatch(command) {
            panic!("FIXTURE: \"{label}\" was refused: {error}");
        }
    }

    fn clip(&self, id: ClipId) -> &Clip {
        self.editor.project().clip(id).expect("the clip exists").1
    }

    fn coming_in(&self, id: ClipId) -> Option<Transition> {
        self.clip(id).transition_in.clone()
    }

    fn steps(&self) -> usize {
        self.editor.history().count()
    }

    fn last(&self) -> String {
        self.editor.undo_label().unwrap_or_default().to_owned()
    }

    fn marked(&self) -> Option<(ClipId, TransitionEdge)> {
        (self.timeline).marked(self.editor.project(), &self.selection)
    }

    fn commit(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        if let Some(command) = draft.command(self.editor.project())
            && let Err(error) = self.editor.dispatch(command)
        {
            self.refused.push(error.to_string());
        }
    }

    fn dispatch(&mut self, command: Command) -> bool {
        self.commit();
        match self.editor.dispatch(command) {
            Ok(_) => true,
            Err(error) => {
                self.refused.push(error.to_string());
                false
            }
        }
    }

    /// One frame of the application with these events in it.
    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let ctx = self.ctx.clone();
        let project = self.editor.snapshot();
        let mut actions: Vec<Action> = Vec::new();
        // The application's shortcuts, before the frame is drawn.
        for event in &events {
            let Event::Key {
                key, pressed: true, ..
            } = event
            else {
                continue;
            };
            match key {
                // A field that was being typed in is left, and no more.
                Key::Escape if self.inspector.typed(&ctx) => {
                    self.inspector.cancel(&mut self.draft);
                }
                Key::Escape if !egui::Popup::is_any_open(&ctx) => {
                    let held = InspectorState::holds(&ctx, &self.draft);
                    if held {
                        self.inspector.cancel(&mut self.draft);
                    }
                    if !(self.timeline.cancel_drag() | held) && !self.timeline.unmark() {
                        self.selection.clear();
                    }
                }
                Key::Delete if !ctx.text_edit_focused() => {
                    if let Some((clip, edge)) = self.marked() {
                        actions.push(Action::Edit(set_command(clip, edge, None)));
                    }
                }
                _ => {}
            }
        }
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (left, right) = self.sides;
        let mut output = ctx.run_ui(input, |ui| {
            let all = ui.max_rect();
            let library = Rect::from_min_max(all.min, pos2(all.left() + left, all.bottom()));
            let inspector = Rect::from_min_max(pos2(all.right() - right, all.top()), all.max);
            let lanes = Rect::from_min_max(
                pos2(library.right(), all.center().y),
                pos2(inspector.left(), all.bottom()),
            );
            library::show(
                ui,
                library,
                &mut self.library,
                &mut self.selection,
                &mut self.textures,
                &mut self.titles,
                &mut crate::peaks::HeldPeaks::default(),
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
            let transition = (self.timeline.marked(&project, &self.selection)).map(|(_, e)| e);
            inspector::show(
                ui,
                inspector,
                &mut self.inspector,
                &self.selection,
                &mut self.draft,
                &mut self.project_draft,
                &InspectorInput {
                    p: &self.palette,
                    project: &project,
                    textures: &self.textures,
                    playhead: self.playhead,
                    transition,
                    missing: None,
                    families: None,
                    brand: &[],
                },
                &mut actions,
            );
        });
        output.textures_delta.clear();
        self.shapes = std::mem::take(&mut output.shapes);
        self.wait = (output.viewport_output.values())
            .map(|viewport| viewport.repaint_delay)
            .min()
            .unwrap_or(Duration::MAX);
        if let Some(update) = &output.platform_output.accesskit_update {
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
                    })
                })
                .collect();
        }
        // The end of the application's frame.
        for action in actions {
            match action {
                Action::Edit(command) => {
                    self.dispatch(command);
                }
                Action::AddTransitions { places, kind } => {
                    self.commit();
                    match asked(self.editor.project(), &self.saved, &places, kind) {
                        Ok(Asked::Show(clip, edge)) => {
                            self.timeline.mark(&mut self.selection, clip, edge);
                        }
                        Ok(Asked::Make(command, made)) => {
                            if self.dispatch(*command)
                                && let Some((clip, edge)) = made
                            {
                                self.timeline.mark(&mut self.selection, clip, edge);
                            }
                        }
                        Err(reason) => self.refused.push(reason),
                    }
                }
                Action::SaveTransitionDefault(transition) => {
                    self.saved = saved_with(&self.saved, transition);
                }
                Action::Seek(time) => self.playhead = time,
                Action::Notify(text) => self.said.push(text),
                other => panic!("FIXTURE: the panes asked for {other:?}"),
            }
        }
        if !(self.hold).holds(&ctx, &self.draft, &self.project_draft) {
            self.hold.forget();
            self.commit();
        }
    }

    fn find(&self, name: &str) -> Option<&Seen> {
        (self.seen.iter())
            .find(|seen| seen.name == name && !matches!(seen.role, Role::Label | Role::TextRun))
    }

    fn has(&self, name: &str) -> bool {
        self.find(name).is_some()
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

    fn press(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Self::button(pos, PointerButton::Primary, true)]);
    }

    fn move_to(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
    }

    fn release(&mut self, pos: Pos2) {
        self.frame(vec![Self::button(pos, PointerButton::Primary, false)]);
        // The frame after the release shows what it committed.
        self.frame(Vec::new());
    }

    fn click_at(&mut self, pos: Pos2) {
        self.press(pos);
        self.release(pos);
    }

    fn click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.click_at(pos);
    }

    /// Two clicks as close as a hand makes them.
    fn double_click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.frame(vec![Event::PointerMoved(pos)]);
        for _ in 0..2 {
            self.frame(vec![Self::button(pos, PointerButton::Primary, true)]);
            self.frame(vec![Self::button(pos, PointerButton::Primary, false)]);
        }
        self.frame(Vec::new());
    }

    fn secondary_click_at(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, true)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, false)]);
        self.frame(Vec::new());
    }

    /// Takes hold at `from` and carries to `to`, without letting go.
    fn carry(&mut self, from: Pos2, to: Pos2) {
        self.press(from);
        for step in 1..=5 {
            self.move_to(from + (to - from) * (step as f32 / 5.0));
        }
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

    fn typed(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.to_owned())]);
        self.frame(Vec::new());
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

    /// Where a time of the project is in the lanes, and the middle of the
    /// clips' row: worked out from the handle of the fixture's dissolve,
    /// which stands half a second into a clip that starts at one.
    fn spot(&self, at: f64) -> Pos2 {
        let handle = self.at(SHOT_START);
        pos2(handle.x + (at - 1.5) as f32 * self.timeline.zoom, handle.y)
    }

    /// The library's search field.
    fn search(&self) -> Pos2 {
        (self.seen.iter())
            .find(|seen| seen.role == Role::TextInput && seen.rect.right() <= self.sides.0)
            .expect("FIXTURE: the library has a search field")
            .rect
            .center()
    }

    fn tab(&mut self, tab: Tab) {
        self.click(tab.label());
        assert_eq!(self.library.tab, tab);
    }
}

// ---- The marking and its handle -------------------------------------------

#[test]
fn dragging_a_marking_s_handle_changes_the_length_in_one_step() {
    let (mut bench, shot, _) = Bench::new();
    let steps = bench.steps();
    let from = bench.at(SHOT_START);
    // A second further into the clip.
    bench.carry(from, from + vec2(bench.timeline.zoom, 0.0));
    bench.frame(Vec::new());
    // Held, nothing is edited yet, and the handle is where it is held.
    assert_eq!(bench.steps(), steps);
    assert_eq!(bench.coming_in(shot).unwrap().duration, DEFAULT_DURATION);
    assert!((bench.at(SHOT_START).x - (from.x + bench.timeline.zoom)).abs() < 1.5);
    bench.release(from + vec2(bench.timeline.zoom, 0.0));
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Set transition");
    assert_eq!(bench.coming_in(shot).unwrap().duration, seconds(1.5));
    // Taking hold of it selected it, and with it the clip.
    assert_eq!(bench.marked(), Some((shot, TransitionEdge::In)));
    assert!(bench.editor.undo());
    assert_eq!(bench.coming_in(shot).unwrap().duration, DEFAULT_DURATION);
}

#[test]
fn a_dragged_length_stays_within_the_clip_and_a_frame() {
    let (mut bench, shot, _) = Bench::new();
    let from = bench.at(SHOT_START);
    let far = from + vec2(bench.timeline.zoom * 9.0, 0.0);
    bench.carry(from, far);
    bench.release(far);
    // No longer than its clip.
    assert_eq!(bench.coming_in(shot).unwrap().duration, seconds(4.0));
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
    let from = bench.at(SHOT_START);
    let back = from - vec2(bench.timeline.zoom * 9.0, 0.0);
    bench.carry(from, back);
    bench.release(back);
    // And never none: a frame of the project.
    let frame = crate::rules::frame(bench.editor.project().settings.frame_rate);
    assert_eq!(bench.coming_in(shot).unwrap().duration, frame);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn escape_gives_up_a_held_handle_and_then_the_marking() {
    let (mut bench, shot, _) = Bench::new();
    let steps = bench.steps();
    let before = bench.editor.project().clone();
    let from = bench.at(SHOT_START);
    let to = from + vec2(bench.timeline.zoom, 0.0);
    bench.carry(from, to);
    bench.key(Key::Escape);
    bench.release(to);
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
    // The first Escape let go of the handle; the marking is still the one
    // selected, and the next gives that up, then the clip.
    assert_eq!(bench.marked(), Some((shot, TransitionEdge::In)));
    bench.key(Key::Escape);
    assert_eq!(bench.marked(), None);
    assert_eq!(bench.selection.clips, [shot]);
    bench.key(Key::Escape);
    assert!(bench.selection.clips.is_empty());
}

#[test]
fn a_click_on_a_marking_shows_its_transition_and_delete_removes_only_it() {
    let (mut bench, shot, _) = Bench::new();
    assert!(!bench.has("Transition duration"));
    // The body of the marking, clear of the handle and of the trim grip.
    let body = bench.spot(1.0) + vec2(12.0, 0.0);
    bench.click_at(body);
    assert_eq!(bench.marked(), Some((shot, TransitionEdge::In)));
    assert!(bench.has("Transition duration") && bench.has("Transition kind"));
    // A press elsewhere on the clip lets the marking go.
    let elsewhere = bench.spot(3.0);
    bench.click_at(elsewhere);
    assert_eq!(bench.marked(), None);
    assert!(!bench.has("Transition duration"));
    bench.click_at(body);
    let steps = bench.steps();
    bench.key(Key::Delete);
    assert_eq!(bench.steps(), steps + 1);
    assert!(bench.coming_in(shot).is_none());
    assert_eq!(bench.selection.clips, [shot], "the clip stays");
    assert_eq!(bench.marked(), None);
    assert!(bench.editor.undo());
    assert!(bench.coming_in(shot).is_some());
}

#[test]
fn the_handle_is_reached_by_the_keyboard_and_selects_its_marking() {
    let (mut bench, shot, _) = Bench::new();
    bench.focus(SHOT_START);
    bench.key(Key::Enter);
    assert_eq!(bench.marked(), Some((shot, TransitionEdge::In)));
    assert!(bench.has("Transition duration"));
}

// ---- Adding one ------------------------------------------------------------

#[test]
fn a_cut_under_the_pointer_offers_a_transition_for_the_incoming_clip() {
    let (mut bench, shot, next) = Bench::new();
    let steps = bench.steps();
    let cut = bench.spot(5.0);
    bench.move_to(cut + vec2(-4.0, 0.0));
    bench.frame(Vec::new());
    // A cut is its incoming clip's start, whichever side the pointer is.
    const OFFER: &str = "Add a transition at the start of next.mp4";
    assert!(bench.has(OFFER));
    bench.click(OFFER);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    let made = bench.coming_in(next).expect("the cut has a transition");
    assert_eq!((made.kind, made.duration), (DEFAULT_KIND, DEFAULT_DURATION));
    assert!(bench.clip(shot).transition_out.is_none());
    // What was made is what the inspector shows.
    assert_eq!(bench.marked(), Some((next, TransitionEdge::In)));
    assert!(bench.has("Transition duration"));
    // And the edge offers no second one.
    bench.move_to(cut + vec2(-4.0, 0.0));
    bench.frame(Vec::new());
    assert!(!bench.has(OFFER));
    assert!(bench.editor.undo());
    assert!(bench.coming_in(next).is_none());
}

#[test]
fn a_free_edge_offers_a_transition_and_a_locked_track_offers_none() {
    let (mut bench, _, next) = Bench::new();
    let end = bench.spot(8.0);
    bench.move_to(end + vec2(-5.0, 0.0));
    bench.frame(Vec::new());
    const OFFER: &str = "Add a transition at the end of next.mp4";
    assert!(bench.has(OFFER));
    bench.click(OFFER);
    assert!(bench.clip(next).transition_out.is_some());
    assert_eq!(bench.marked(), Some((next, TransitionEdge::Out)));

    let (mut bench, ..) = Bench::new();
    let end = bench.spot(8.0);
    let track_id = bench.track(TrackKind::Video);
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    });
    bench.move_to(end + vec2(-5.0, 0.0));
    bench.frame(Vec::new());
    assert!(!bench.has(OFFER));
    // Nor a handle to drag.
    assert!(!bench.has(SHOT_START));
}

#[test]
fn the_clip_menu_adds_edits_and_removes_the_transition_of_the_nearer_edge() {
    let (mut bench, shot, next) = Bench::new();
    let steps = bench.steps();
    // Near the end of the last clip: its own end, which nothing follows.
    let late = bench.spot(7.6);
    bench.secondary_click_at(late);
    bench.click("Add transition at end");
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert!(bench.clip(next).transition_out.is_some());
    // Where there is one, the menu shows it or takes it away.
    bench.key(Key::Escape);
    let early = bench.spot(2.0);
    bench.secondary_click_at(early);
    assert!(!bench.has("Add transition at start"));
    bench.click("Edit transition");
    assert_eq!(bench.marked(), Some((shot, TransitionEdge::In)));
    assert_eq!(bench.steps(), steps + 1);
    bench.secondary_click_at(early);
    bench.click("Remove transition");
    assert!(bench.coming_in(shot).is_none());
    assert_eq!(bench.steps(), steps + 2);
}

#[test]
fn asking_where_there_is_one_shows_it_and_a_kept_default_is_what_is_made() {
    let (mut bench, shot, next) = Bench::new();
    let project = bench.editor.project().clone();
    // The shortcut, with the playhead at the start of the selected clip.
    let places = at_playhead(&project, &[shot], seconds(1.0));
    assert_eq!(places, [(shot, TransitionEdge::In)]);
    assert!(matches!(
        asked(&project, &[], &places, None),
        Ok(Asked::Show(clip, TransitionEdge::In)) if clip == shot
    ));
    // The end of a clip another follows is that one's start.
    let mut kept = default_of(&[], DEFAULT_KIND);
    kept.duration = seconds(1.0);
    kept.ease = EASINGS[3].1;
    let places = at_playhead(&project, &[shot], seconds(4.9));
    let Ok(Asked::Make(command, made)) = asked(&project, &[kept.clone()], &places, None) else {
        panic!("an edit");
    };
    assert_eq!(made, Some((next, TransitionEdge::In)));
    bench.edit(*command);
    assert_eq!(bench.coming_in(next), Some(kept));
    // Nothing selected is said, not done.
    let nothing = asked(&project, &[], &[], None).unwrap_err();
    assert!(!nothing.is_empty());
}

// ---- The inspector ----------------------------------------------------------

/// A bench with the fixture's dissolve shown in the inspector.
fn shown() -> (Bench, ClipId) {
    let (mut bench, shot, _) = Bench::new();
    bench
        .timeline
        .mark(&mut bench.selection, shot, TransitionEdge::In);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(bench.has("Transition duration"));
    (bench, shot)
}

#[test]
fn a_scrubbed_duration_is_one_step_and_escape_gives_it_up() {
    let (mut bench, shot) = shown();
    let steps = bench.steps();
    let field = bench.at("Transition duration");
    bench.carry(field, field + vec2(60.0, 0.0));
    // Held: a draft, and no edit.
    assert_eq!(bench.steps(), steps);
    let held = (bench.draft.as_ref())
        .and_then(|draft| draft.transitions.clone())
        .and_then(|both| both[0].clone())
        .expect("a held transition");
    assert!(held.duration > DEFAULT_DURATION);
    bench.release(field + vec2(60.0, 0.0));
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.coming_in(shot).unwrap().duration, held.duration);
    assert!(bench.editor.undo());
    assert_eq!(bench.coming_in(shot).unwrap().duration, DEFAULT_DURATION);

    let before = bench.editor.project().clone();
    let steps = bench.steps();
    let field = bench.at("Transition duration");
    bench.carry(field, field + vec2(60.0, 0.0));
    bench.key(Key::Escape);
    bench.move_to(field + vec2(90.0, 0.0));
    bench.release(field + vec2(90.0, 0.0));
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
    // The Escape gave up the scrub, not the marking it was scrubbed in.
    assert_eq!(bench.marked(), Some((shot, TransitionEdge::In)));
}

#[test]
fn the_kind_s_own_values_are_listed_from_the_tables_and_edited() {
    let (mut bench, shot) = shown();
    // A dissolve has none.
    assert!(!bench.has("Transition softness") && !bench.has("Transition amount"));
    let steps = bench.steps();
    bench.click("Transition kind");
    bench.click("Wipe left");
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    let wipe = bench.coming_in(shot).unwrap();
    assert_eq!(wipe.kind, TransitionKind::WipeLeft);
    assert_eq!(wipe.duration, DEFAULT_DURATION, "its length is kept");
    for spec in look::transition_spec(TransitionKind::WipeLeft) {
        assert_eq!(spec.key, ParamKey::Softness);
    }
    assert!(bench.has("Transition softness"));
    let field = bench.at("Transition softness");
    bench.carry(field, field + vec2(40.0, 0.0));
    bench.release(field + vec2(40.0, 0.0));
    assert_eq!(bench.steps(), steps + 2, "{:?}", bench.refused);
    let softness = bench.coming_in(shot).unwrap().param(ParamKey::Softness);
    assert!(softness.is_some_and(|value| value > 0.05), "{softness:?}");
    // Its reset puts back the kind's own.
    bench.click("Reset softness");
    assert_eq!(bench.steps(), steps + 3);
    assert!(bench.coming_in(shot).unwrap().params.is_empty());

    bench.click("Transition easing");
    bench.click("Ease out");
    assert_eq!(bench.coming_in(shot).unwrap().ease, EASINGS[3].1);
    assert_eq!(bench.steps(), steps + 4);
}

#[test]
fn a_transition_is_removed_and_kept_as_its_kind_s_default_from_the_inspector() {
    let (mut bench, shot) = shown();
    bench.click("Transition easing");
    bench.click("Ease in and out");
    let steps = bench.steps();
    bench.click("Save as default");
    // Keeping it is the person's setting, not an edit of the project.
    assert_eq!(bench.steps(), steps);
    assert_eq!(bench.saved, [bench.coming_in(shot).unwrap()]);
    assert_eq!(default_of(&bench.saved, DEFAULT_KIND).ease, EASINGS[4].1);
    bench.click("Remove");
    assert_eq!(bench.steps(), steps + 1);
    assert!(bench.coming_in(shot).is_none());
    // With its transition gone the clip is what the inspector shows.
    assert!(!bench.has("Transition duration"));
    assert_eq!(bench.selection.clips, [shot]);
}

#[test]
fn a_locked_track_s_transition_is_shown_and_not_editable() {
    let (mut bench, shot) = shown();
    let track_id = bench.track(TrackKind::Video);
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    });
    bench.frame(Vec::new());
    let steps = bench.steps();
    let field = bench.at("Transition duration");
    bench.carry(field, field + vec2(60.0, 0.0));
    bench.release(field + vec2(60.0, 0.0));
    bench.click("Remove");
    assert_eq!(bench.steps(), steps);
    assert!(bench.coming_in(shot).is_some());
}

#[test]
fn the_transition_pane_fits_a_narrow_inspector() {
    let (mut bench, _) = shown();
    bench.click("Transition kind");
    bench.click("Zoom in");
    bench.sides = (236.0, 236.0);
    bench.size = vec2(760.0, 560.0);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    let pane = Rect::from_min_max(pos2(760.0 - 236.0, 0.0), pos2(760.0, 560.0));
    for name in [
        "Transition kind",
        "Transition duration",
        "Transition easing",
        "Transition amount",
        "Remove",
        "Save as default",
    ] {
        let rect = bench.rect(name);
        assert!(
            pane.contains_rect(rect),
            "{name} at {rect:?} leaves {pane:?}"
        );
    }
    // And the library's switch its panel.
    let side = Rect::from_min_max(Pos2::ZERO, pos2(236.0, 560.0));
    for tab in Tab::ALL {
        let rect = bench.rect(tab.label());
        assert!(side.contains_rect(rect), "{} at {rect:?}", tab.label());
    }
}

// ---- The library's tiles ----------------------------------------------------

#[test]
fn each_tab_lists_its_tiles_from_the_tables_and_the_search_narrows_them() {
    let (mut bench, ..) = Bench::new();
    bench.tab(Tab::Transitions);
    for kind in KINDS {
        assert!(
            bench.has(name(kind)) || !bench.rect("Dissolve").is_positive(),
            "{kind:?}"
        );
    }
    bench.tab(Tab::Looks);
    assert_eq!(inspector::looks().len(), 7);
    for (_, look) in inspector::looks() {
        assert!(bench.has(look), "{look}");
    }
    bench.tab(Tab::Effects);
    assert!(bench.has("Glow") && bench.has("Grain"));
    // Nothing that needs a file, and nothing the canvas does not draw.
    let offered = browser::entries(Tab::Effects);
    assert!(offered.iter().all(|(_, entry)| match entry {
        browser::Entry::Effect(kind) => !kind.needs_asset() && *kind != EffectKind::Stabilize,
        _ => false,
    }));
    let search = bench.search();
    bench.click_at(search);
    bench.typed("glo");
    assert!(bench.has("Glow") && !bench.has("Grain"));
    bench.typed("zzz");
    assert!(!bench.has("Glow"));
    // Each tab keeps its own search: the media are not narrowed by it.
    bench.tab(Tab::Media);
    assert!(bench.library.search.is_empty());
}

#[test]
fn the_effects_tab_lists_the_picture_kinds_it_listed_before_then_the_audio_kinds() {
    use EffectKind as K;
    // Under their headings, in the order of the table within each.
    let listed: Vec<(&str, EffectKind)> = (browser::entries(Tab::Effects).into_iter())
        .map(|(heading, entry)| match entry {
            browser::Entry::Effect(kind) => (heading, kind),
            other => panic!("{other:?}"),
        })
        .collect();
    let before = [
        ("Stylize", K::Glow),
        ("Stylize", K::Grain),
        ("Stylize", K::Shadow),
        ("Stylize", K::Vignette),
        ("Stylize", K::Glitch),
        ("Blur and sharpen", K::MotionBlur),
        ("Blur and sharpen", K::Sharpen),
        ("Blur and sharpen", K::Blur),
        ("Distort", K::Shake),
        ("Distort", K::Distort),
        ("Key", K::ChromaKey),
        ("Colour", K::Grade),
        ("Colour", K::HslBand),
        ("Colour", K::CurveMaster),
        ("Colour", K::CurveRed),
        ("Colour", K::CurveGreen),
        ("Colour", K::CurveBlue),
    ];
    // The audio kinds come last, under their own heading, and Pitch is
    // kept or shifted in the Playback card, not given from here.
    let audio = [
        K::HighPass,
        K::EqBand,
        K::Compressor,
        K::Limiter,
        K::Denoise,
        K::CenterCut,
    ]
    .map(|kind| ("Audio", kind));
    assert_eq!(listed, [before.as_slice(), &audio].concat());
    assert!(!browser::offered(K::Pitch));
    for kind in EffectKind::ALL.into_iter().filter(|kind| kind.is_audio()) {
        assert_eq!(inspector::effect_family(kind).1, "Audio");
        assert!(!inspector::effect_title(kind).is_empty());
    }
    // A search for the heading finds its tiles, by their names.
    let (mut bench, ..) = Bench::new();
    bench.tab(Tab::Effects);
    assert!(bench.has("Glow"));
    let search = bench.search();
    bench.click_at(search);
    bench.typed("audio");
    for name in [
        "High-pass",
        "EQ band",
        "Compressor",
        "Limiter",
        "Noise reduction",
    ] {
        assert!(bench.has(name), "{name}");
    }
    assert!(!bench.has("Glow") && !bench.has("Pitch"));
}

#[test]
fn an_audio_tile_goes_to_a_sound_and_a_picture_is_told_why_not() {
    let (mut bench, shot, _) = Bench::new();
    // A video with sound before the two silent ones, and a sound on the
    // audio track.
    let mut talk = Asset::new(
        PathBuf::from("/media/talk.mp4"),
        MediaKind::Video,
        seconds(10.0),
    );
    talk.name = "talk.mp4".to_owned();
    talk.has_audio = true;
    let mut voice = Asset::new(
        PathBuf::from("/media/voice.wav"),
        MediaKind::Audio,
        seconds(10.0),
    );
    voice.name = "voice.wav".to_owned();
    bench.edit(Command::AddAssets {
        assets: vec![talk.clone(), voice.clone()],
    });
    let mut picture = Clip::from_asset(&talk, Time::ZERO);
    picture.duration = seconds(0.9);
    let picture_id = picture.id;
    let track_id = bench.track(TrackKind::Video);
    bench.edit(Command::AddClip {
        track_id,
        clip: picture,
    });
    let mut sound = Clip::from_asset(&voice, Time::ZERO);
    sound.duration = seconds(3.0);
    let sound_id = sound.id;
    let track_id = bench.track(TrackKind::Audio);
    bench.edit(Command::AddClip {
        track_id,
        clip: sound,
    });
    bench.selection.clear();
    bench.tab(Tab::Effects);
    let search = bench.search();
    bench.click_at(search);
    bench.typed("audio");

    // Let go over a picture, it changes nothing and says why: one with
    // sound is told to detach it, one without that it has none to take one.
    let before = bench.editor.project().clone();
    let steps = bench.steps();
    for (at, said) in [
        (0.45, crate::audio::DETACH_FIRST),
        (2.5, "Only a sound on an audio track takes an audio effect."),
    ] {
        let tile = bench.at("Compressor");
        let over = bench.spot(at);
        bench.carry(tile, over);
        bench.release(over);
        assert_eq!(bench.steps(), steps);
        assert_eq!(*bench.editor.project(), before);
        assert_eq!(bench.said.last().map(String::as_str), Some(said));
    }
    assert!(bench.clip(picture_id).effects.is_empty() && bench.clip(shot).effects.is_empty());

    // Given to the selected clips, it goes to the sound among them alone.
    bench.selection.clips = vec![shot, picture_id, sound_id];
    bench.double_click("Limiter");
    assert_eq!(
        bench.steps(),
        steps + 1,
        "{:?} {:?}",
        bench.refused,
        bench.said
    );
    let kinds: Vec<EffectKind> = (bench.clip(sound_id).effects.iter())
        .map(|effect| effect.kind)
        .collect();
    assert_eq!(kinds, [EffectKind::Limiter]);
    assert_eq!(*bench.clip(picture_id), *before.clip(picture_id).unwrap().1);
    assert!(bench.editor.undo());
    assert_eq!(*bench.editor.project(), before);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_double_click_or_enter_gives_an_effect_to_the_selected_clips() {
    let (mut bench, shot, next) = Bench::new();
    bench.selection.clips = vec![shot, next];
    bench.tab(Tab::Effects);
    let steps = bench.steps();
    bench.double_click("Glow");
    assert_eq!(
        bench.steps(),
        steps + 1,
        "{:?} {:?}",
        bench.refused,
        bench.said
    );
    assert_eq!(bench.last(), "Add effect");
    for clip in [shot, next] {
        let kinds: Vec<_> = bench.clip(clip).effects.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [EffectKind::Glow]);
    }
    assert!(bench.editor.undo());
    assert!(bench.clip(shot).effects.is_empty() && bench.clip(next).effects.is_empty());
    // A click picks the tile; Return gives it.
    bench.selection.clips = vec![shot];
    bench.click("Grain");
    assert_eq!(bench.steps(), steps);
    bench.key(Key::Enter);
    assert_eq!(
        bench.steps(),
        steps + 1,
        "{:?} {:?}",
        bench.refused,
        bench.said
    );
    assert_eq!(bench.clip(shot).effects[0].kind, EffectKind::Grain);
}

#[test]
fn a_look_is_given_whole_and_nothing_selected_is_explained() {
    let (mut bench, shot, _) = Bench::new();
    bench.tab(Tab::Looks);
    let steps = bench.steps();
    bench.double_click("Noir");
    assert_eq!(
        bench.steps(),
        steps + 1,
        "{:?} {:?}",
        bench.refused,
        bench.said
    );
    assert!(!bench.clip(shot).effects.is_empty());
    assert!(bench.editor.undo());
    assert!(bench.clip(shot).effects.is_empty());

    bench.selection.clear();
    let before = bench.editor.project().clone();
    bench.double_click("Noir");
    assert_eq!(*bench.editor.project(), before);
    assert_eq!(
        bench.said,
        ["Select a clip on the timeline to give a look to, or drag it onto one."]
    );
}

#[test]
fn an_activated_transition_goes_on_the_edge_nearest_the_playhead() {
    let (mut bench, shot, next) = Bench::new();
    bench.tab(Tab::Transitions);
    // The playhead is at the start of the selected clip, which has a
    // dissolve: it becomes a wipe of the same length.
    let steps = bench.steps();
    bench.double_click("Wipe left");
    assert_eq!(
        bench.steps(),
        steps + 1,
        "{:?} {:?}",
        bench.refused,
        bench.said
    );
    let wipe = bench.coming_in(shot).unwrap();
    assert_eq!(
        (wipe.kind, wipe.duration),
        (TransitionKind::WipeLeft, DEFAULT_DURATION)
    );
    // Nearer its end, where the next clip starts: the cut is that clip's.
    bench.selection.only(shot);
    bench.playhead = seconds(4.8);
    bench.double_click("Fade");
    assert_eq!(
        bench.coming_in(next).map(|made| made.kind),
        Some(TransitionKind::Fade)
    );
    assert!(bench.clip(shot).transition_out.is_none());
    assert_eq!(bench.steps(), steps + 2);
}

#[test]
fn a_tile_let_go_over_a_clip_is_given_to_that_clip_and_its_nearer_edge() {
    let (mut bench, shot, next) = Bench::new();
    // Nothing is selected: the clip is the one under the pointer.
    bench.selection.clear();
    bench.tab(Tab::Transitions);
    let steps = bench.steps();
    let tile = bench.at("Zoom in");
    // Late in the last clip: its end.
    let late = bench.spot(7.5);
    bench.carry(tile, late);
    assert_eq!(bench.steps(), steps);
    bench.release(late);
    assert_eq!(
        bench.steps(),
        steps + 1,
        "{:?} {:?}",
        bench.refused,
        bench.said
    );
    let made = bench.clip(next).transition_out.clone().expect("at its end");
    assert_eq!(made.kind, TransitionKind::ZoomIn);
    assert!(bench.coming_in(next).is_none());

    // An effect goes to the clip, wherever on it.
    bench.tab(Tab::Effects);
    let tile = bench.at("Glow");
    let over = bench.spot(2.5);
    bench.carry(tile, over);
    bench.release(over);
    assert_eq!(
        bench.steps(),
        steps + 2,
        "{:?} {:?}",
        bench.refused,
        bench.said
    );
    assert_eq!(bench.clip(shot).effects.len(), 1);
    assert!(bench.clip(next).effects.is_empty());

    // Let go over no clip, or with Escape, nothing is given.
    let tile = bench.at("Glow");
    let empty = bench.spot(9.5);
    bench.carry(tile, empty);
    bench.release(empty);
    assert_eq!(bench.steps(), steps + 2);
}

#[test]
fn a_tile_let_go_over_a_locked_track_is_refused_and_changes_nothing() {
    let (mut bench, ..) = Bench::new();
    bench.tab(Tab::Transitions);
    let over = bench.spot(7.5);
    let track_id = bench.track(TrackKind::Video);
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    });
    let before = bench.editor.project().clone();
    let steps = bench.steps();
    let tile = bench.at("Zoom in");
    bench.carry(tile, over);
    bench.release(over);
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
    assert_eq!(bench.refused.len(), 1, "{:?}", bench.refused);
}

#[test]
fn nothing_asks_for_another_frame_while_idle() {
    let (mut bench, shot, _) = Bench::new();
    bench
        .timeline
        .mark(&mut bench.selection, shot, TransitionEdge::In);
    for tab in [Tab::Effects, Tab::Transitions, Tab::Looks] {
        bench.tab(tab);
        bench.move_to(pos2(700.0, 100.0));
        for _ in 0..12 {
            bench.time += 1.0;
            bench.frame(Vec::new());
        }
        assert!(
            bench.wait > Duration::from_secs(3600),
            "{tab:?}: {:?}",
            bench.wait
        );
    }
}

// ---- Review of wave 3 -------------------------------------------------------

#[test]
fn an_escape_in_a_typed_duration_gives_it_up_and_keeps_the_marking_selected() {
    let (mut bench, shot) = shown();
    let steps = bench.steps();
    let stored = bench.coming_in(shot);
    bench.focus("Transition duration");
    bench.typed("2");
    assert!(bench.draft.is_some(), "what is typed is held");
    bench.key(Key::Escape);
    bench.frame(Vec::new());
    assert!(bench.draft.is_none());
    assert_eq!(bench.steps(), steps, "a value given up is no step");
    assert_eq!(bench.coming_in(shot), stored);
    // The Escape left the field and did no more: the marking is still the
    // one selected, with its clip.
    assert_eq!(bench.marked(), Some((shot, TransitionEdge::In)));
    assert_eq!(bench.selection.clips, [shot]);
}

#[test]
fn a_transitions_values_typed_a_key_a_frame_keep_every_character() {
    let (mut bench, shot) = shown();
    bench.click("Transition kind");
    bench.click("Wipe left");
    let steps = bench.steps();
    // The first digit takes the softness off the kind's own: the row gains
    // its reset while the field is being typed in.
    bench.focus("Transition softness");
    for character in ["2", "5"] {
        bench.frame(vec![Event::Text(character.to_owned())]);
    }
    assert_eq!(bench.steps(), steps, "a value being typed is no step");
    bench.key(Key::Enter);
    let softness = bench.coming_in(shot).unwrap().param(ParamKey::Softness);
    assert!(
        softness.is_some_and(|value| (value - 0.25).abs() < 1e-4),
        "typed 25 %, stored {softness:?}"
    );
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);

    bench.focus("Transition duration");
    for character in ["0", ".", "8"] {
        bench.frame(vec![Event::Text(character.to_owned())]);
    }
    bench.key(Key::Enter);
    let duration = bench.coming_in(shot).unwrap().duration;
    assert!(
        (duration.as_seconds() - 0.8).abs() < 0.02,
        "typed 0.8 s, stored {duration:?}"
    );
    assert_eq!(bench.steps(), steps + 2, "{:?}", bench.refused);
}

#[test]
fn a_press_on_the_name_of_a_tall_tile_selects_the_clip_and_not_its_marking() {
    let (mut bench, shot, _) = Bench::new();
    // The marking is drawn under the name: the handle at its inner end is
    // as tall as it is.
    let handle = bench.rect(SHOT_START);
    let body = bench.spot(1.0).x + 12.0;
    bench.click_at(pos2(body, handle.center().y));
    assert_eq!(bench.marked(), Some((shot, TransitionEdge::In)));
    bench.click_at(bench.spot(3.0));
    assert_eq!(bench.marked(), None);
    // The same place along the clip, in the row its name is written in.
    bench.click_at(pos2(body, handle.top() - 8.0));
    assert_eq!(bench.selection.clips, [shot], "the name is the clip's");
    assert_eq!(bench.marked(), None, "and not the marking's");
    assert!(!bench.has("Transition duration"));
}

#[test]
fn a_drag_from_the_edge_under_the_badge_still_trims_and_a_click_adds() {
    let (mut bench, _, next) = Bench::new();
    let end = bench.spot(8.0);
    bench.move_to(end + vec2(-5.0, 0.0));
    bench.frame(Vec::new());
    const OFFER: &str = "Add a transition at the end of next.mp4";
    let badge = bench.rect(OFFER);
    // On the badge and in the grip that trims the clip's end.
    let grip = pos2(end.x - 3.0, badge.center().y);
    assert!(badge.contains(grip), "{badge:?} has {grip:?}");
    bench.move_to(grip);
    bench.frame(Vec::new());
    assert!(bench.has(OFFER));

    let steps = bench.steps();
    let to = grip - vec2(bench.timeline.zoom, 0.0);
    bench.carry(grip, to);
    bench.release(to);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Trim clip");
    let length = bench.clip(next).duration;
    assert!(
        (length.as_seconds() - 2.0).abs() < 0.1,
        "trimmed by the second it was dragged: {length:?}"
    );
    assert!(bench.clip(next).transition_out.is_none(), "and not offered");

    // Let go where it went down, the same press is a click on the badge:
    // the edge is given a transition and nothing is trimmed.
    let end = bench.spot(length.as_seconds() + 5.0);
    bench.move_to(end + vec2(-5.0, 0.0));
    bench.frame(Vec::new());
    let badge = bench.rect(OFFER);
    let grip = pos2(end.x - 3.0, badge.center().y);
    bench.move_to(grip);
    bench.frame(Vec::new());
    let steps = bench.steps();
    bench.press(grip);
    bench.frame(Vec::new());
    bench.release(grip);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert!(bench.clip(next).transition_out.is_some());
    assert_eq!(bench.clip(next).duration, length, "nothing was trimmed");
    assert_eq!(bench.marked(), Some((next, TransitionEdge::Out)));
}

#[test]
fn only_a_click_on_the_badge_that_was_shown_adds_a_transition() {
    let (mut bench, shot, next) = Bench::new();
    const OFFER: &str = "Add a transition at the end of next.mp4";
    let end = bench.spot(8.0);
    bench.move_to(end + vec2(-5.0, 0.0));
    bench.frame(Vec::new());
    let badge = bench.rect(OFFER);
    let steps = bench.steps();

    // The corner of the badge's box is not the disc that is drawn.
    let corner = badge.right_top() + vec2(-1.0, 1.0);
    assert!(corner.distance(badge.center()) > badge.width() * 0.5);
    bench.selection.only(shot);
    bench.click_at(corner);
    assert_eq!(bench.steps(), steps, "{:?}", bench.refused);
    assert!(bench.clip(next).transition_out.is_none());
    assert_eq!(
        bench.selection.single(),
        Some(next),
        "the click is the clip's"
    );

    // A press that arrives with the pointer finds no badge under it yet:
    // nobody saw one there.
    bench.selection.only(shot);
    bench.move_to(bench.spot(6.5));
    bench.frame(Vec::new());
    assert!(!bench.has(OFFER));
    let centre = badge.center();
    bench.frame(vec![
        Event::PointerMoved(centre),
        Bench::button(centre, PointerButton::Primary, true),
    ]);
    bench.release(centre);
    assert_eq!(bench.steps(), steps, "{:?}", bench.refused);
    assert!(bench.clip(next).transition_out.is_none());
    assert_eq!(bench.selection.single(), Some(next));

    // Shown under the pointer and then clicked, it adds one.
    bench.move_to(bench.spot(6.5));
    bench.click_at(centre);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert!(bench.clip(next).transition_out.is_some());
}

#[test]
fn a_click_in_an_edge_grip_selects_the_clip_and_trims_nothing() {
    let (mut bench, _, next) = Bench::new();
    let end = bench.spot(8.0);
    bench.move_to(end + vec2(-5.0, 0.0));
    bench.frame(Vec::new());
    let badge = bench.rect("Add a transition at the end of next.mp4");
    // In the grip that trims the clip's end, clear of the badge: at this
    // zoom the grip is some frames of the clip wide.
    let grip = pos2(end.x - 5.0, badge.top() - 5.0);
    assert!(
        5.0 / bench.timeline.zoom > 1.0 / 30.0,
        "FIXTURE: a frame fits the grip"
    );
    let (steps, length) = (bench.steps(), bench.clip(next).duration);
    bench.press(grip);
    bench.frame(Vec::new());
    // A hand is not still: two points are not a drag.
    bench.move_to(grip + vec2(-2.0, 0.0));
    bench.release(grip + vec2(-2.0, 0.0));
    assert_eq!(bench.steps(), steps, "{:?}", bench.refused);
    assert_eq!(bench.clip(next).duration, length);
    assert_eq!(
        bench.selection.single(),
        Some(next),
        "the click selected it"
    );
}

#[test]
fn the_handle_of_a_transition_as_long_as_its_clip_leaves_both_grips_alone() {
    let (mut bench, _, next) = Bench::new();
    let whole = Transition {
        duration: seconds(3.0),
        ..default_of(&[], DEFAULT_KIND)
    };
    bench.edit(set_command(next, TransitionEdge::In, Some(whole)));
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    let handle = bench.rect("Dissolve at the start of next.mp4");
    let (start, end) = (bench.spot(5.0).x, bench.spot(8.0).x);
    // Seven points at each edge trim the clip, and the clip beside it.
    assert!(handle.right() <= end - 7.0 + 0.01, "{handle:?} at {end}");
    assert!(handle.left() >= start + 7.0 - 0.01, "{handle:?} at {start}");

    // A drag from the far edge trims it, as on any clip.
    let steps = bench.steps();
    let grip = pos2(end - 3.0, handle.center().y);
    let to = grip - vec2(bench.timeline.zoom * 0.5, 0.0);
    bench.carry(grip, to);
    bench.release(to);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Trim clip");
}

#[test]
fn a_wheel_turned_with_ctrl_zooms_the_timeline_about_the_pointer() {
    let (mut bench, _, _) = Bench::new();
    bench.frame(Vec::new());
    let wheel = |delta: f32, modifiers| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, delta),
        phase: egui::TouchPhase::Move,
        modifiers,
    };
    let settle = |bench: &mut Bench| {
        for _ in 0..12 {
            bench.frame(Vec::new());
        }
    };
    // Up over a marking: it stays under the pointer while the time spreads.
    let marked = bench.at(SHOT_START);
    let zoom = bench.timeline.zoom;
    bench.move_to(marked);
    bench.frame(vec![wheel(60.0, Modifiers::CTRL | Modifiers::COMMAND)]);
    settle(&mut bench);
    assert!(
        bench.timeline.zoom > zoom * 1.2,
        "{} from {zoom}",
        bench.timeline.zoom
    );
    assert!((bench.at(SHOT_START).x - marked.x).abs() < 1.0);

    // Down over the tool strip, which is the timeline's too.
    let zoomed = bench.timeline.zoom;
    bench.move_to(bench.at("Fit timeline to window (Shift+Z)"));
    bench.frame(vec![wheel(-60.0, Modifiers::CTRL | Modifiers::COMMAND)]);
    settle(&mut bench);
    assert!(
        bench.timeline.zoom < zoomed / 1.2,
        "{} from {zoomed}",
        bench.timeline.zoom
    );

    // Without the modifier the wheel scrolls and leaves the scale alone.
    let zoom = bench.timeline.zoom;
    bench.move_to(marked);
    bench.frame(vec![wheel(-60.0, Modifiers::NONE)]);
    settle(&mut bench);
    assert_eq!(bench.timeline.zoom, zoom);
}

#[test]
fn a_chosen_dissolve_keeps_its_diagonal_in_a_narrow_ring() {
    // §9.6 and P10 round 4: at the scale of a narrow window a second's
    // dissolve is a ring a few points wide, which frames its lines. The
    // diagonal runs across the ring, and gives up its glyph rather than be
    // broken into stubs round it.
    let (mut bench, shot, _) = Bench::new();
    let second = Transition {
        duration: seconds(1.0),
        ..default_of(&[], DEFAULT_KIND)
    };
    bench.edit(set_command(shot, TransitionEdge::In, Some(second)));
    bench.timeline.zoom = 33.0;
    bench
        .timeline
        .mark(&mut bench.selection, shot, TransitionEdge::In);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    let ink = egui::Color32::from_white_alpha(170);
    // The length of each slanted line of the stage's ink, as much of it as
    // its clip shows: only a dissolve draws one here.
    let shown: Vec<f32> = (bench.shapes.iter())
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == ink && (points[0].x - points[1].x).abs() > 0.5 =>
            {
                let inside = (0..=200)
                    .filter(|step| {
                        let at = points[0].lerp(points[1], *step as f32 / 200.0);
                        clipped.clip_rect.contains(at)
                    })
                    .count();
                Some((points[1] - points[0]).length() * inside as f32 / 201.0)
            }
            _ => None,
        })
        .collect();
    assert_eq!(shown.len(), 1, "one whole diagonal, unbroken: {shown:?}");
    assert!(shown[0] >= 15.0, "{shown:?}");
}

#[test]
fn a_selected_tiles_trim_bars_stand_on_a_keyline_rim_in_either_appearance() {
    // §4.4.6 and P10 round 6: a bar over a poster lies on a frame that may
    // be as dark (in the light) or as light (in the dark) as itself, so
    // each stands on a rim of the selection's keyline, drawn under it.
    let (mut bench, ..) = Bench::new();
    for appearance in [Appearance::Light, Appearance::Dark] {
        bench.palette = Palette::new(appearance, Accent::default());
        theme::apply(&bench.ctx, &bench.palette);
        bench.frame(Vec::new());
        bench.frame(Vec::new());
        let p = bench.palette;
        let rects: Vec<(Rect, egui::Color32)> = (bench.shapes.iter())
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) => Some((rect.rect, rect.fill)),
                _ => None,
            })
            .collect();
        let bar = theme::tint(p.fg, 0.8);
        let bars: Vec<usize> = (0..rects.len())
            .filter(|&at| rects[at].1 == bar && rects[at].0.width() == 3.0)
            .collect();
        assert_eq!(
            bars.len(),
            2,
            "{appearance:?}: the selected tile's two bars"
        );
        for at in bars {
            let (rect, _) = rects[at];
            assert!(
                rects[..at]
                    .iter()
                    .rev()
                    .take(2)
                    .any(|(rim, fill)| { *fill == p.keyline() && *rim == rect.expand(1.0) }),
                "{appearance:?}: the bar at {rect:?} has its rim under it"
            );
        }
    }
}
