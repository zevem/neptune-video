//! The keyframe lane and the graph, driven as a person drives them.
//!
//! [`Bench`] paints the whole timeline in a real egui pass and hands it
//! pointer and key events. A key, a handle or a button is found by the name
//! accessibility is given for it. The bench begins and ends a frame as the
//! application does: an Escape gives up what the timeline holds before the
//! frame is drawn, the keys the timeline's shortcuts would take are counted
//! when the keyframes do not own them, and an [`Action::Edit`] is dispatched,
//! so every edit below is what `Editor::dispatch` accepted and its undo is
//! the editor's.
use super::*;
use crate::{
    textures::Textures,
    theme::{Accent, Appearance},
    ui::timeline::{self, TimelineState},
};
use editor_core::{Asset, Editor, Effect, EffectKind, MediaKind, TrackId, TrackKind, resolve};
use eframe::egui::{Event, Key, Modifiers, PointerButton, accesskit::Role};
use std::{path::PathBuf, time::Duration};

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

fn key(at: f64, value: f32) -> Keyframe {
    Keyframe {
        at: seconds(at),
        value,
        ease: Ease::Linear,
    }
}

const EASE_IN: Ease = Ease::Bezier {
    x1: 0.42,
    y1: 0.0,
    x2: 1.0,
    y2: 1.0,
};

/// A control accessibility was told of in the last frame.
struct Seen {
    node: egui::accesskit::NodeId,
    name: String,
    rect: Rect,
    role: Role,
    disabled: bool,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    textures: Textures,
    editor: Editor,
    state: TimelineState,
    selection: Selection,
    playhead: Time,
    snapping: bool,
    size: Vec2,
    time: f64,
    seen: Vec<Seen>,
    /// What the editor refused, in the words of the refusal.
    refused: Vec<String>,
    said: Vec<String>,
    /// The frames the timeline's own arrows stepped the playhead by, and
    /// the times its Delete and its clipboard chords were heard.
    stepped: i64,
    clip_deletes: usize,
    clip_chords: usize,
    /// How long the last frame said it could wait before the next.
    wait: Duration,
}

impl Bench {
    /// A project with one video clip of four seconds a second in, selected.
    /// Its opacity goes from nothing at its start to whole two seconds on,
    /// and the amount of its glow is keyed a second and three seconds in.
    fn new() -> (Self, ClipId, Uuid) {
        let mut asset = Asset::new(
            PathBuf::from("/media/shot.mp4"),
            MediaKind::Video,
            seconds(10.0),
        );
        asset.name = "shot.mp4".to_owned();
        asset.has_audio = false;
        let mut bench = Self::empty();
        bench.edit(Command::AddAssets {
            assets: vec![asset.clone()],
        });
        let mut clip = Clip::from_asset(&asset, seconds(1.0));
        clip.duration = seconds(4.0);
        let id = clip.id;
        let track_id = bench.track(TrackKind::Video);
        bench.edit(Command::AddClip { track_id, clip });
        let glow = Effect::new(EffectKind::Glow);
        bench.edit(Command::SetEffects {
            clip_id: id,
            effects: vec![glow.clone()],
        });
        bench.keys(
            id,
            None,
            ParamKey::Opacity,
            vec![key(0.0, 0.0), key(2.0, 1.0)],
        );
        bench.keys(
            id,
            Some(glow.id),
            ParamKey::Amount,
            vec![key(1.0, 0.25), key(3.0, 0.75)],
        );
        bench.selection.only(id);
        (bench, id, glow.id)
    }

    fn empty() -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        Self {
            ctx,
            palette,
            textures: Textures::default(),
            editor: Editor::new(Project::new("Keyframes")).expect("a new project is valid"),
            state: TimelineState::default(),
            selection: Selection::default(),
            playhead: Time::ZERO,
            snapping: true,
            size: vec2(1000.0, 420.0),
            time: 0.0,
            seen: Vec::new(),
            refused: Vec::new(),
            said: Vec::new(),
            stepped: 0,
            clip_deletes: 0,
            clip_chords: 0,
            wait: Duration::ZERO,
        }
    }

    fn track(&self, kind: TrackKind) -> TrackId {
        (self.editor.project().tracks.iter())
            .find(|track| track.kind == kind)
            .map(|track| track.id)
            .expect("FIXTURE: a new project has a track of each kind")
    }

    /// A fixture's own edit: not one the timeline made.
    fn edit(&mut self, command: Command) {
        let label = command.label().to_owned();
        if let Err(error) = self.editor.dispatch(command) {
            panic!("FIXTURE: \"{label}\" was refused: {error}");
        }
    }

    fn keys(&mut self, clip: ClipId, target: Option<Uuid>, key: ParamKey, keys: Vec<Keyframe>) {
        self.edit(Command::SetKeyframes {
            clip_id: clip,
            target,
            key,
            keys,
        });
    }

    fn clip(&self, id: ClipId) -> &Clip {
        self.editor.project().clip(id).expect("the clip exists").1
    }

    /// The keys of one of the clip's tracks, as seconds and values.
    fn track_keys(&self, id: ClipId, target: Option<Uuid>, key: ParamKey) -> Vec<(f64, f32)> {
        track_of(self.clip(id), (target, key)).map_or_else(Vec::new, |track| {
            (track.keys.iter())
                .map(|key| (key.at.as_seconds(), key.value))
                .collect()
        })
    }

    fn opacity(&self, id: ClipId) -> Vec<(f64, f32)> {
        self.track_keys(id, None, ParamKey::Opacity)
    }

    fn steps(&self) -> usize {
        self.editor.history().count()
    }

    fn last(&self) -> String {
        self.editor.undo_label().unwrap_or_default().to_owned()
    }

    /// One frame of the application with these events in it.
    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let ctx = self.ctx.clone();
        let project = self.editor.snapshot();
        let rate = project.settings.frame_rate;
        let mut actions: Vec<Action> = Vec::new();
        // The application's shortcuts, before the frame is drawn.
        let keyed = self.state.keys.owns_keys(&ctx, &self.selection);
        for event in &events {
            match event {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => match key {
                    Key::Escape if !egui::Popup::is_any_open(&ctx) => {
                        if !self.state.cancel_drag() {
                            self.selection.clear();
                        }
                    }
                    Key::ArrowLeft | Key::ArrowRight if !keyed && modifiers.is_none() => {
                        self.stepped += if *key == Key::ArrowLeft { -1 } else { 1 };
                    }
                    Key::Delete | Key::Backspace if !keyed => self.clip_deletes += 1,
                    Key::G if modifiers.is_none() => {
                        self.state.keys.graph = !self.state.keys.graph;
                    }
                    Key::U if modifiers.is_none() => {
                        self.state.keys.toggle_lane(&project, &self.selection);
                    }
                    _ => {}
                },
                Event::Copy | Event::Cut if !keyed => self.clip_chords += 1,
                Event::Paste(text) if !self.state.keys.pastes(text) => {
                    self.clip_chords += 1;
                    self.state.keys.forget_copied();
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
        let mut output = ctx.run_ui(input, |ui| {
            let rect = ui.max_rect();
            timeline::show(
                ui,
                rect,
                &mut self.state,
                &mut self.selection,
                &mut self.textures,
                &mut crate::peaks::HeldPeaks::default(),
                &TimelineInput {
                    p: &self.palette,
                    project: &project,
                    playhead: self.playhead,
                    snapping: self.snapping,
                    media: None,
                    can_paste: false,
                    freeze_hold: seconds(2.0),
                    separator: crate::stems::Tool::Missing,
                },
                &mut actions,
            );
        });
        output.textures_delta.clear();
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
                Action::Seek(time) => self.playhead = time,
                Action::Notify(text) => self.said.push(text),
                Action::Step(frames) => {
                    self.playhead = rate.frame_time(rate.frame_at(self.playhead) + frames);
                }
                other => panic!("FIXTURE: the timeline asked for {other:?}"),
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

    fn at(&self, name: &str) -> Pos2 {
        match self.find(name) {
            Some(seen) => seen.rect.center(),
            None => {
                let names: Vec<&str> = self.seen.iter().map(|seen| seen.name.as_str()).collect();
                panic!("FIXTURE: no control is named \"{name}\" among {names:?}")
            }
        }
    }

    fn button(pos: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        }
    }

    fn press(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        let down = Self::button(pos, PointerButton::Primary, true, Modifiers::NONE);
        self.frame(vec![down]);
    }

    fn move_to(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
    }

    fn release(&mut self, pos: Pos2) {
        let up = Self::button(pos, PointerButton::Primary, false, Modifiers::NONE);
        self.frame(vec![up]);
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

    /// A click with Shift held, which adds to what is selected.
    fn shift_click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.frame(vec![Event::PointerMoved(pos)]);
        let shift = Modifiers::SHIFT;
        self.frame(vec![
            Event::ModifiersChanged(shift),
            Self::button(pos, PointerButton::Primary, true, shift),
        ]);
        self.frame(vec![Self::button(
            pos,
            PointerButton::Primary,
            false,
            shift,
        )]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    fn secondary_click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.frame(vec![Event::PointerMoved(pos)]);
        let none = Modifiers::NONE;
        self.frame(vec![Self::button(
            pos,
            PointerButton::Secondary,
            true,
            none,
        )]);
        self.frame(vec![Self::button(
            pos,
            PointerButton::Secondary,
            false,
            none,
        )]);
        self.frame(Vec::new());
    }

    /// Takes hold of the control and moves it `by`, without letting go.
    fn drag(&mut self, name: &str, by: Vec2) -> Pos2 {
        self.frame(Vec::new());
        let from = self.at(name);
        self.press(from);
        for step in 1..=4 {
            self.move_to(from + by * (step as f32 / 4.0));
        }
        from + by
    }

    fn key(&mut self, key: Key) {
        self.keys_down(key, 1);
    }

    /// `count` presses of one key in a single frame.
    fn keys_down(&mut self, key: Key, count: usize) {
        let press = Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![press; count]);
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

    /// The opacity the canvas would draw the clip with now, held keys and
    /// all.
    fn drawn_opacity(&self, clip: ClipId, at: Time) -> f32 {
        let project = self.editor.project();
        let overrides: Vec<_> = self.state.keys.overriding(project).into_iter().collect();
        (resolve(project, at, &overrides).layers.iter())
            .find(|layer| layer.clip == clip)
            .expect("the clip is on screen")
            .transform
            .opacity
    }
}

/// A bench with the lane of its clip open.
fn lane() -> (Bench, ClipId, Uuid) {
    let (mut bench, clip, glow) = Bench::new();
    bench.click("Keyframes of shot.mp4");
    assert_eq!(bench.state.keys.lane_clip(), Some(clip));
    (bench, clip, glow)
}

/// A bench with its clip in the graph.
fn graph() -> (Bench, ClipId, Uuid) {
    let (mut bench, clip, glow) = Bench::new();
    bench.click("Graph editor (G)");
    assert!(bench.state.keys.graph);
    bench.frame(Vec::new());
    (bench, clip, glow)
}

const OPACITY_START: &str = "Opacity keyframe at 00:00:01:00";
const OPACITY_END: &str = "Opacity keyframe at 00:00:03:00";
const GLOW_START: &str = "Glow Amount keyframe at 00:00:02:00";
const GLOW_END: &str = "Glow Amount keyframe at 00:00:04:00";

// ---- The edits, apart from any gesture ------------------------------------

#[test]
fn a_moved_key_takes_the_place_of_one_it_lands_on_and_stays_in_the_clip() {
    let (bench, clip, glow) = Bench::new();
    let clip = bench.clip(clip);
    let first = KeyRef {
        target: None,
        key: ParamKey::Opacity,
        at: Time::ZERO,
    };
    let tracks = moved(clip, &[first], seconds(2.0), 0.0);
    assert_eq!(tracks.len(), 1, "only the track of the moved key changes");
    assert_eq!(tracks[0].keys, vec![key(2.0, 0.0)]);
    assert!(tracks[0].validate(clip.duration, &look::clip_spec()[5]));

    // No further than the clip's ends, whichever key is nearest one.
    let last = KeyRef {
        at: seconds(2.0),
        ..first
    };
    assert_eq!(reach(clip, &[first, last], seconds(9.0)), seconds(2.0));
    assert_eq!(reach(clip, &[first, last], seconds(-9.0)), Time::ZERO);
    assert_eq!(reach(clip, &[last], seconds(-9.0)), seconds(-2.0));

    // A value is held to its parameter's range, from the tables.
    let raised = moved(clip, &[last], Time::ZERO, 5.0);
    assert_eq!(raised[0].keys[1].value, 1.0);
    let amount = KeyRef {
        target: Some(glow),
        key: ParamKey::Amount,
        at: seconds(1.0),
    };
    let spec = look::param_spec(AnimTarget::Effect(EffectKind::Glow), ParamKey::Amount).unwrap();
    let lowered = moved(clip, &[amount], Time::ZERO, -50.0);
    assert_eq!(lowered[0].keys[0].value, spec.min);
}

#[test]
fn a_command_is_one_batch_of_the_tracks_that_change_and_none_when_nothing_does() {
    let (bench, clip, _) = Bench::new();
    let clip = bench.clip(clip);
    assert!(command(clip, "Move keyframes", clip.animation.clone()).is_none());
    let all: Vec<KeyRef> = (clip.animation.iter())
        .flat_map(|track| track.keys.iter().map(|key| KeyRef::of(track, key)))
        .collect();
    let tracks = moved(clip, &all, seconds(0.5), 0.0);
    let Some(Command::Batch { label, commands }) = command(clip, "Move keyframes", tracks) else {
        panic!("a batch");
    };
    assert_eq!(label, "Move keyframes");
    assert_eq!(commands.len(), 2, "one SetKeyframes for each track");
    assert!(
        (commands.iter()).all(|command| matches!(command, Command::SetKeyframes { .. })),
        "{commands:?}"
    );
}

// ---- The lane -------------------------------------------------------------

#[test]
fn the_chevron_of_a_selected_animated_clip_opens_a_lane_of_its_keys() {
    let (mut bench, clip, _) = Bench::new();
    bench.frame(Vec::new());
    assert!(!bench.has(OPACITY_START), "the lane starts closed");
    bench.click("Keyframes of shot.mp4");
    assert_eq!(bench.state.keys.lane_clip(), Some(clip));
    // A key is drawn at its time on the timeline: the clip's start and on.
    let (start, end) = (bench.at(OPACITY_START), bench.at(OPACITY_END));
    assert!((end.x - start.x - 2.0 * bench.state.zoom).abs() < 0.01);
    assert!(bench.has(GLOW_START) && bench.has(GLOW_END));
    let row = bench.at(GLOW_START).y - start.y;
    assert!((row - ROW).abs() < 0.01, "a row for each parameter: {row}");
    // The rows are named under the track's head, clear of the keys.
    let named = bench.find("Select Opacity keyframes").expect("named").rect;
    assert!(named.right() < start.x && (named.center().y - start.y).abs() < 1.0);

    bench.click("Keyframes of shot.mp4");
    assert_eq!(bench.state.keys.lane_clip(), None);
    assert!(!bench.has(OPACITY_START));

    // The key U and the tool strip open it too.
    bench.key(Key::U);
    assert!(bench.has(OPACITY_START));
    bench.click("Keyframes of the selected clip (U)");
    assert!(!bench.has(OPACITY_START));
}

#[test]
fn a_clip_without_animation_has_no_chevron_and_no_lane() {
    let (mut bench, clip, glow) = Bench::new();
    bench.keys(clip, None, ParamKey::Opacity, Vec::new());
    bench.keys(clip, Some(glow), ParamKey::Amount, Vec::new());
    bench.frame(Vec::new());
    assert!(!bench.has("Keyframes of shot.mp4"));
    let strip = bench
        .find("Keyframes of the selected clip (U)")
        .expect("named");
    assert!(strip.disabled);
    bench.key(Key::U);
    assert_eq!(bench.state.keys.lane_clip(), None);
}

#[test]
fn a_dragged_key_is_previewed_and_is_one_step_when_let_go() {
    let (mut bench, clip, _) = lane();
    let before = bench.steps();
    let stored = bench.editor.snapshot();
    // A second and a bit: it lands on a frame.
    let end = bench.drag(OPACITY_END, vec2(83.0, 0.0));

    assert_eq!(bench.steps(), before, "a held drag is no step");
    assert_eq!(*bench.editor.project(), *stored);
    // The canvas draws the clip as the drag would leave it: half way up at
    // a second and a half in, where it was three quarters.
    let held = bench.drawn_opacity(clip, seconds(2.5));
    assert!((held - 0.5).abs() < 0.02, "previewed: {held}");

    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "the whole drag is one step");
    assert_eq!(bench.last(), "Move keyframes");
    let keys = bench.opacity(clip);
    assert_eq!(keys.len(), 2);
    let frame = 1.0 / 30.0;
    let landed = keys[1].0;
    assert!((landed - 3.0).abs() < 0.05, "{landed}");
    assert!(
        ((landed / frame).round() * frame - landed).abs() < 2e-6,
        "on a frame: {landed}"
    );
    assert!(
        bench
            .state
            .keys
            .overriding(bench.editor.project())
            .is_none()
    );
    // The key that was moved is still the selected one.
    assert_eq!(bench.state.keys.selected().len(), 1);
    assert_eq!(bench.state.keys.selected()[0].at.as_seconds(), landed);

    assert!(bench.editor.undo());
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0), (2.0, 1.0)]);
}

#[test]
fn an_escape_gives_up_a_held_key_and_leaves_the_project_and_the_clip_selected() {
    let (mut bench, clip, _) = lane();
    let before = bench.steps();
    let stored = bench.editor.snapshot();
    let end = bench.drag(OPACITY_END, vec2(80.0, 0.0));
    assert!(
        bench
            .state
            .keys
            .overriding(bench.editor.project())
            .is_some()
    );

    bench.frame(vec![Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(
        bench
            .state
            .keys
            .overriding(bench.editor.project())
            .is_none()
    );
    assert_eq!(
        bench.selection.single(),
        Some(clip),
        "the clip stays selected"
    );
    // The pointer is still down and still moving: nothing it does is kept.
    bench.move_to(end + vec2(40.0, 0.0));
    bench.release(end + vec2(40.0, 0.0));
    assert_eq!(bench.steps(), before, "a cancelled drag is no step");
    assert_eq!(*bench.editor.project(), *stored);
    assert!(
        bench.has(OPACITY_END),
        "the key is drawn where it is stored"
    );
}

#[test]
fn a_dragged_key_snaps_to_the_playhead_when_snapping_is_on() {
    let (mut bench, clip, _) = lane();
    // Off the frame grid, so that landing on it is the snap's doing.
    bench.playhead = Time(3_512_345);
    let by = vec2(0.5 * bench.state.zoom + 5.0, 0.0);
    let end = bench.drag(OPACITY_END, by);
    bench.release(end);
    assert_eq!(bench.clip(clip).animation[0].keys[1].at, Time(2_512_345));

    assert!(bench.editor.undo());
    bench.snapping = false;
    let end = bench.drag(OPACITY_END, by);
    bench.release(end);
    let at = bench.clip(clip).animation[0].keys[1].at;
    assert_ne!(at, Time(2_512_345), "no snap with snapping off");
    assert_eq!(at, rules::quantize(at, FrameRate::default()), "a frame");
}

#[test]
fn shift_adds_keys_and_those_of_two_tracks_move_as_one_step() {
    let (mut bench, clip, glow) = lane();
    bench.click(OPACITY_END);
    assert_eq!(bench.state.keys.selected().len(), 1);
    bench.shift_click(GLOW_START);
    assert_eq!(bench.state.keys.selected().len(), 2);
    // Shift on a selected key lets it go again.
    bench.shift_click(GLOW_START);
    assert_eq!(bench.state.keys.selected().len(), 1);
    bench.shift_click(GLOW_START);

    let before = bench.steps();
    let end = bench.drag(OPACITY_END, vec2(-40.0, 0.0));
    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "two tracks, one step");
    assert_eq!(bench.last(), "Move keyframes");
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0), (1.5, 1.0)]);
    let amount = bench.track_keys(clip, Some(glow), ParamKey::Amount);
    assert_eq!(amount, vec![(0.5, 0.25), (3.0, 0.75)]);

    assert!(bench.editor.undo());
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0), (2.0, 1.0)]);
    let amount = bench.track_keys(clip, Some(glow), ParamKey::Amount);
    assert_eq!(amount, vec![(1.0, 0.25), (3.0, 0.75)]);
}

#[test]
fn a_press_and_its_release_inside_one_frame_is_still_a_click() {
    let (mut bench, _, _) = lane();
    bench.frame(Vec::new());
    let none = Modifiers::NONE;
    let both = |pos: Pos2| {
        vec![
            Event::PointerMoved(pos),
            Bench::button(pos, PointerButton::Primary, true, none),
            Bench::button(pos, PointerButton::Primary, false, none),
        ]
    };
    let key = bench.at(GLOW_START);
    bench.frame(both(key));
    bench.frame(Vec::new());
    assert_eq!(bench.state.keys.selected().len(), 1);
    assert_eq!(bench.state.keys.selected()[0].key, ParamKey::Amount);
    // And on the lane beside it: the key is let go and the playhead moved.
    bench.frame(both(key + vec2(40.0, 0.0)));
    bench.frame(Vec::new());
    assert!(bench.state.keys.selected().is_empty());
    assert!((bench.playhead.as_seconds() - 2.5).abs() < 0.02);
    assert_eq!(bench.steps(), 5);
}

#[test]
fn delete_removes_the_selected_keys_and_not_the_clip() {
    let (mut bench, clip, _) = lane();
    bench.click(OPACITY_END);
    let before = bench.steps();
    bench.key(Key::Delete);
    assert_eq!(bench.clip_deletes, 0, "the clip was not asked to go");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Delete keyframes");
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0)]);
    assert!(bench.state.keys.selected().is_empty());

    // The last key of a track takes the track with it.
    bench.click(OPACITY_START);
    bench.key(Key::Backspace);
    assert!(bench.opacity(clip).is_empty());
    assert_eq!(bench.clip(clip).animation.len(), 1);
    assert!(bench.editor.undo() && bench.editor.undo());
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0), (2.0, 1.0)]);

    // With no key selected, Delete is the timeline's again.
    bench.frame(Vec::new());
    bench.key(Key::Delete);
    assert_eq!(bench.clip_deletes, 1);

    // A press on a clip lets selected keys go: Delete is then the clip's.
    bench.click(OPACITY_START);
    assert_eq!(bench.state.keys.selected().len(), 1);
    let tile = bench.at("Keyframes of shot.mp4") + vec2(60.0, 20.0);
    bench.click_at(tile);
    assert!(bench.state.keys.selected().is_empty());
    bench.key(Key::Delete);
    assert_eq!(bench.clip_deletes, 2);
}

#[test]
fn the_arrows_nudge_the_selected_keys_a_frame_and_leave_the_playhead() {
    let (mut bench, clip, _) = lane();
    bench.click(OPACITY_END);
    let before = bench.steps();
    bench.key(Key::ArrowRight);
    assert_eq!(bench.stepped, 0, "the playhead was not stepped");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Move keyframes");
    let frame = FrameRate::default().frame_time(1);
    assert_eq!(
        bench.clip(clip).animation[0].keys[1].at,
        seconds(2.0) + frame
    );

    // Every press of a frame counts, and the key that moved is still held.
    bench.keys_down(Key::ArrowLeft, 3);
    let at = bench.clip(clip).animation[0].keys[1].at;
    assert_eq!(
        at,
        seconds(2.0) + frame - FrameRate::default().frame_time(3)
    );
    assert_eq!(bench.state.keys.selected()[0].at, at);

    // A key stops at the end of its clip.
    bench.click(GLOW_END);
    for _ in 0..40 {
        bench.key(Key::ArrowRight);
    }
    let last = bench.clip(clip).animation[1].keys[1].at;
    assert_eq!(last, seconds(4.0));

    // An Escape lets the keys go, and the arrows are the playhead's again.
    bench.key(Key::Escape);
    assert!(bench.state.keys.selected().is_empty());
    assert_eq!(bench.selection.single(), Some(clip));
    bench.key(Key::ArrowRight);
    assert_eq!(bench.stepped, 1);
}

#[test]
fn copied_keys_are_pasted_at_the_playhead_as_one_step() {
    let (mut bench, clip, glow) = lane();
    bench.click(GLOW_START);
    bench.shift_click(GLOW_END);
    bench.frame(vec![Event::Copy]);
    assert_eq!(bench.clip_chords, 0, "the clip was not copied");
    assert_eq!(bench.said, vec!["Copied keyframes".to_owned()]);

    // Half a second into the clip: the second key would fall past its end
    // only if it were pasted later.
    bench.playhead = seconds(1.5);
    let before = bench.steps();
    bench.frame(vec![Event::Paste(
        bench.state.keys.copied_text().to_owned(),
    )]);
    bench.frame(Vec::new());
    assert_eq!(bench.clip_chords, 0);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Paste keyframes");
    let amount = bench.track_keys(clip, Some(glow), ParamKey::Amount);
    assert_eq!(
        amount,
        vec![(0.5, 0.25), (1.0, 0.25), (2.5, 0.75), (3.0, 0.75)]
    );
    assert_eq!(bench.state.keys.selected().len(), 2, "the pasted keys");

    assert!(bench.editor.undo());
    let amount = bench.track_keys(clip, Some(glow), ParamKey::Amount);
    assert_eq!(amount, vec![(1.0, 0.25), (3.0, 0.75)]);

    // Off the clip there is nowhere to paste, and the project is told so.
    bench.playhead = seconds(8.0);
    bench.frame(vec![Event::Paste(
        bench.state.keys.copied_text().to_owned(),
    )]);
    assert_eq!(bench.steps(), before);
    assert!(
        bench
            .said
            .last()
            .is_some_and(|said| said.contains("playhead"))
    );
}

#[test]
fn the_menu_of_a_key_sets_the_easing_of_the_selected_keys() {
    let (mut bench, clip, _) = lane();
    let before = bench.steps();
    bench.secondary_click(OPACITY_START);
    assert_eq!(bench.state.keys.selected().len(), 1, "the key is selected");
    assert!(bench.has("Ease in") && bench.has("Hold"));
    bench.click("Ease in");
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Change easing");
    assert_eq!(bench.clip(clip).animation[0].keys[0].ease, EASE_IN);
    assert_eq!(bench.clip(clip).animation[0].keys[1].ease, Ease::Linear);
    assert!(bench.editor.undo());
    assert_eq!(bench.clip(clip).animation[0].keys[0].ease, Ease::Linear);
}

#[test]
fn every_edit_of_a_key_is_reached_without_a_pointer() {
    let (mut bench, clip, _) = lane();
    // The key is given the keyboard, and Enter selects it.
    bench.focus(OPACITY_END);
    assert!(bench.state.keys.selected().is_empty());
    bench.key(Key::Enter);
    assert_eq!(bench.state.keys.selected().len(), 1);
    // The arrows move it and do not move the focus off it.
    bench.key(Key::ArrowLeft);
    let frame = FrameRate::default().frame_time(1);
    assert_eq!(
        bench.clip(clip).animation[0].keys[1].at,
        seconds(2.0) - frame
    );
    assert_eq!(bench.stepped, 0);

    // The lane's menu holds the rest. The track's last key leaves no
    // stretch to ease, and the menu says so instead of making a step.
    bench.focus("Keyframe options");
    bench.key(Key::Enter);
    assert!(bench.has("Delete keyframes") && bench.has("Paste at playhead"));
    assert!(bench.find("Hold").expect("listed").disabled);
    bench.key(Key::Escape);
    bench.focus(OPACITY_START);
    bench.key(Key::Enter);
    bench.focus("Keyframe options");
    bench.key(Key::Enter);
    bench.focus("Hold");
    bench.key(Key::Enter);
    assert_eq!(bench.clip(clip).animation[0].keys[0].ease, Ease::Hold);
    assert_eq!(bench.clip(clip).animation[0].keys[1].ease, Ease::Linear);

    // A row's name selects every key of its parameter.
    bench.focus("Select Opacity keyframes");
    bench.key(Key::Enter);
    assert_eq!(bench.state.keys.selected().len(), 2);
}

#[test]
fn a_marquee_selects_keys_and_a_click_on_the_lane_moves_the_playhead() {
    let (mut bench, _, _) = lane();
    bench.frame(Vec::new());
    let (first, last) = (bench.at(OPACITY_START), bench.at(GLOW_END));
    let from = first + vec2(20.0, -5.0);
    bench.press(from);
    bench.move_to(last + vec2(10.0, 6.0));
    // Everything right of the first opacity key.
    assert_eq!(bench.state.keys.selected().len(), 3);
    bench.release(last + vec2(10.0, 6.0));
    assert_eq!(bench.state.keys.selected().len(), 3);
    assert_eq!(bench.steps(), 5, "selecting is no step");

    // A press that does not travel lets them go and seeks.
    let empty = pos2(bench.at(OPACITY_END).x + 40.0, bench.at(OPACITY_END).y);
    bench.click_at(empty);
    assert!(bench.state.keys.selected().is_empty());
    let sought = bench.playhead.as_seconds();
    assert!((sought - 3.5).abs() < 0.02, "{sought}");
}

#[test]
fn a_locked_track_holds_its_keys_and_the_project_refuses_what_is_asked_of_them() {
    let (mut bench, clip, _) = lane();
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
    let before = bench.steps();
    let stored = bench.editor.snapshot();
    let end = bench.drag(OPACITY_END, vec2(80.0, 0.0));
    assert!(
        bench
            .state
            .keys
            .overriding(bench.editor.project())
            .is_none()
    );
    bench.release(end);
    assert_eq!(bench.steps(), before);

    // The key is selected all the same; what the keyboard asks is refused
    // by the project, in its own words, and nothing changes.
    assert_eq!(bench.state.keys.selected().len(), 1);
    bench.key(Key::Delete);
    assert_eq!(bench.refused.len(), 1, "{:?}", bench.refused);
    assert_eq!(*bench.editor.project(), *stored);
    assert_eq!(bench.opacity(clip).len(), 2);
    bench.secondary_click(OPACITY_END);
    assert!(bench.find("Ease in").is_some_and(|item| item.disabled));
}

#[test]
fn the_lane_asks_for_no_frame_when_nothing_is_done_in_it() {
    let (mut bench, _, _) = lane();
    bench.click(OPACITY_END);
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert_eq!(bench.wait, Duration::MAX, "an idle lane sleeps");
}

// ---- The graph ------------------------------------------------------------

#[test]
fn the_graph_takes_the_tracks_place_and_shows_a_curve_for_each_parameter() {
    let (mut bench, _, _) = graph();
    assert!(bench.has(OPACITY_START) && bench.has(GLOW_END));
    assert!(!bench.has("Lock track"), "the track heads are not drawn");
    assert!(bench.has("Show Opacity") && bench.has("Solo Glow Amount"));
    // Time is the timeline's own: a key is under its time on the ruler.
    let (start, end) = (bench.at(OPACITY_START), bench.at(OPACITY_END));
    assert!((end.x - start.x - 2.0 * bench.state.zoom).abs() < 0.01);
    // And value is up: the opacity rises, the glow sits between.
    assert!(end.y < start.y);
    let glow = bench.at(GLOW_START).y;
    assert!(glow < start.y && glow > end.y);

    // The key G and the button leave it again.
    bench.key(Key::G);
    assert!(bench.has("Lock track") && !bench.has("Show Opacity"));
}

#[test]
fn the_graph_says_what_it_needs_when_it_has_nothing_to_draw() {
    let (mut bench, clip, glow) = graph();
    bench.selection.clear();
    bench.frame(Vec::new());
    assert!(!bench.has(OPACITY_START) && !bench.has("Fit values"));
    bench.selection.only(clip);
    bench.keys(clip, None, ParamKey::Opacity, Vec::new());
    bench.keys(clip, Some(glow), ParamKey::Amount, Vec::new());
    bench.frame(Vec::new());
    assert!(!bench.has("Fit values"));
    assert_eq!(bench.wait, Duration::MAX);
}

#[test]
fn a_key_in_the_graph_is_dragged_in_time_and_value_as_one_step() {
    let (mut bench, clip, _) = graph();
    let before = bench.steps();
    let stored = bench.editor.snapshot();
    let (low, high) = (bench.at(OPACITY_START).y, bench.at(OPACITY_END).y);
    // Half way down to the first key's value, and half a second earlier.
    let by = vec2(-0.5 * bench.state.zoom, (low - high) * 0.5);
    let end = bench.drag(OPACITY_END, by);
    assert_eq!(*bench.editor.project(), *stored, "held, not written");
    let held = bench.drawn_opacity(clip, seconds(2.5));
    assert!((held - 0.5).abs() < 0.02, "previewed: {held}");

    bench.release(end);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Move keyframes");
    let keys = bench.opacity(clip);
    assert_eq!(keys[1].0, 1.5);
    assert!((keys[1].1 - 0.5).abs() < 0.02, "{keys:?}");
    assert!(bench.editor.undo());
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0), (2.0, 1.0)]);
}

#[test]
fn a_value_dragged_in_the_graph_stops_at_its_parameters_range() {
    let (mut bench, clip, glow) = graph();
    let end = bench.drag(OPACITY_END, vec2(0.0, -400.0));
    bench.release(end);
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0), (2.0, 1.0)]);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);

    let end = bench.drag(GLOW_START, vec2(0.0, 900.0));
    bench.release(end);
    let spec = look::param_spec(AnimTarget::Effect(EffectKind::Glow), ParamKey::Amount).unwrap();
    let amount = bench.track_keys(clip, Some(glow), ParamKey::Amount);
    assert_eq!(amount[0], (1.0, spec.min));
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn an_escape_gives_up_a_key_held_in_the_graph() {
    let (mut bench, _, _) = graph();
    let before = bench.steps();
    let stored = bench.editor.snapshot();
    let end = bench.drag(OPACITY_END, vec2(-40.0, 30.0));
    assert!(
        bench
            .state
            .keys
            .overriding(bench.editor.project())
            .is_some()
    );
    bench.key(Key::Escape);
    bench.move_to(end + vec2(0.0, 20.0));
    bench.release(end + vec2(0.0, 20.0));
    assert_eq!(bench.steps(), before);
    assert_eq!(*bench.editor.project(), *stored);
    assert!(bench.state.keys.graph, "an Escape does not leave the graph");
}

#[test]
fn the_curve_is_the_tracks_own_evaluation_of_its_easing() {
    let (mut bench, clip, _) = graph();
    let eased = vec![
        Keyframe {
            ease: EASE_IN,
            ..key(0.0, 0.0)
        },
        key(2.0, 1.0),
    ];
    bench.keys(clip, None, ParamKey::Opacity, eased);
    bench.frame(Vec::new());
    let (a, b) = (bench.at(OPACITY_START), bench.at(OPACITY_END));
    let curve = bench.state.keys.curve(0).expect("the opacity's curve");
    // At each sample between the two keys, the height the ease gives.
    let mut checked = 0;
    for point in curve.iter().filter(|p| p.x > a.x + 1.0 && p.x < b.x - 1.0) {
        let t = f64::from((point.x - a.x) / (b.x - a.x));
        let y = a.y + (b.y - a.y) * EASE_IN.eval(t) as f32;
        assert!((point.y - y).abs() < 0.2, "{point:?} for {y}");
        checked += 1;
    }
    assert!(checked > 40, "sampled along the segment: {checked}");
    // An ease in starts level: a quarter of the way it has hardly risen.
    let early = curve.iter().find(|p| p.x >= a.x + (b.x - a.x) * 0.25);
    assert!(early.is_some_and(|p| (a.y - p.y) < (a.y - b.y) * 0.15));

    // A held key steps: the curve stays level and rises under the next key.
    let held = vec![
        Keyframe {
            ease: Ease::Hold,
            ..key(0.0, 0.0)
        },
        key(2.0, 1.0),
    ];
    bench.keys(clip, None, ParamKey::Opacity, held);
    bench.frame(Vec::new());
    let (a, b) = (bench.at(OPACITY_START), bench.at(OPACITY_END));
    let curve = bench.state.keys.curve(0).expect("the opacity's curve");
    assert!(
        (curve.iter())
            .filter(|p| p.x > a.x && p.x < b.x - 0.01)
            .all(|p| (p.y - a.y).abs() < 0.01)
    );
    let under: Vec<f32> = (curve.iter())
        .filter(|p| (p.x - b.x).abs() < 0.01)
        .map(|p| p.y)
        .collect();
    assert!(under.len() >= 2 && under.iter().any(|y| (y - b.y).abs() < 0.01));
}

#[test]
fn the_curve_is_sampled_when_something_changes_and_not_in_every_frame() {
    let (mut bench, _, _) = graph();
    let sampled = bench.state.keys.samplings();
    assert!(sampled >= 1);
    for _ in 0..5 {
        bench.frame(Vec::new());
    }
    assert_eq!(
        bench.state.keys.samplings(),
        sampled,
        "idle frames reuse it"
    );
    assert_eq!(bench.wait, Duration::MAX, "an idle graph sleeps");
    // The pointer over the graph changes nothing that is drawn of a curve.
    let over = bench.at(OPACITY_END) + vec2(30.0, 30.0);
    bench.move_to(over);
    bench.move_to(over + vec2(5.0, 0.0));
    assert_eq!(bench.state.keys.samplings(), sampled);

    bench.state.zoom_centered(1.5);
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert_eq!(bench.state.keys.samplings(), sampled + 1, "a zoom is one");
    bench.size.y += 40.0;
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert_eq!(bench.state.keys.samplings(), sampled + 2, "a size is one");
}

#[test]
fn the_legend_shows_a_curve_alone_or_not_at_all_and_the_values_fit_what_is_shown() {
    let (mut bench, _, _) = graph();
    let spread = bench.at(OPACITY_START).y - bench.at(OPACITY_END).y;
    bench.click("Solo Glow Amount");
    assert!(!bench.has(OPACITY_START), "the other curve is gone");
    // The glow's half of the range now has the height opacity's whole had.
    let alone = bench.at(GLOW_START).y - bench.at(GLOW_END).y;
    assert!((alone - spread).abs() < 1.0, "{alone} against {spread}");
    bench.click("Solo Glow Amount");
    assert!(bench.has(OPACITY_START));

    bench.click("Show Glow Amount");
    assert!(!bench.has(GLOW_START) && bench.has(OPACITY_START));
    bench.click("Show Glow Amount");
    assert!(bench.has(GLOW_START));
    assert_eq!(bench.steps(), 5, "what is shown is not the project's");
}

#[test]
fn the_wheel_moves_the_values_and_the_fit_button_brings_them_back() {
    let (mut bench, _, _) = graph();
    let fitted = bench.at(OPACITY_END);
    assert!(bench.find("Fit values").is_some_and(|fit| fit.disabled));
    bench.move_to(fitted + vec2(20.0, 20.0));
    bench.frame(vec![Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, 30.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    }]);
    for _ in 0..12 {
        bench.frame(Vec::new());
    }
    let moved = bench.at(OPACITY_END);
    assert!(moved.y > fitted.y + 10.0, "{moved:?} from {fitted:?}");
    assert_eq!(moved.x, fitted.x);
    bench.click("Fit values");
    bench.frame(Vec::new());
    assert!((bench.at(OPACITY_END).y - fitted.y).abs() < 0.01);
}

#[test]
fn a_selected_keys_segment_has_handles_that_shape_its_ease_as_one_step() {
    let (mut bench, clip, _) = graph();
    let out = "Opacity ease out of 00:00:01:00";
    let into = "Opacity ease into 00:00:03:00";
    assert!(!bench.has(out), "no handle until a key is selected");
    bench.click(OPACITY_START);
    let (a, b) = (bench.at(OPACITY_START), bench.at(OPACITY_END));
    // A straight segment's handles lie a third along it.
    let first = bench.at(out);
    assert!((first.x - (a.x + (b.x - a.x) / 3.0)).abs() < 0.6);
    assert!((first.y - (a.y + (b.y - a.y) / 3.0)).abs() < 0.6);

    let before = bench.steps();
    let stored = bench.editor.snapshot();
    // Flat out of the key, and two thirds of the way along.
    let to = pos2(a.x + (b.x - a.x) * 2.0 / 3.0, a.y);
    let end = bench.drag(out, to - first);
    assert_eq!(*bench.editor.project(), *stored);
    assert!(
        bench
            .state
            .keys
            .overriding(bench.editor.project())
            .is_some()
    );
    bench.release(end);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Change easing");
    let Ease::Bezier { x1, y1, x2, y2 } = bench.clip(clip).animation[0].keys[0].ease else {
        panic!("a cubic");
    };
    assert!(
        (x1 - 2.0 / 3.0).abs() < 0.02 && y1.abs() < 0.02,
        "{x1} {y1}"
    );
    assert!((x2 - 2.0 / 3.0).abs() < 0.01 && (y2 - 2.0 / 3.0).abs() < 0.01);

    // A handle never leaves what an ease can hold. The travel is past the
    // limit of a graph made taller by the 34-point strip and 24-point ruler
    // (§5.5.1, §5.5.2).
    let end = bench.drag(into, vec2(400.0, -1200.0));
    bench.release(end);
    let Ease::Bezier { x2, y2, .. } = bench.clip(clip).animation[0].keys[0].ease else {
        panic!("a cubic");
    };
    assert_eq!((x2, y2), (1.0, 4.0));
    assert!(bench.clip(clip).animation[0].keys[0].ease.validate());
    assert!(bench.refused.is_empty());

    assert!(bench.editor.undo() && bench.editor.undo());
    assert_eq!(bench.clip(clip).animation[0].keys[0].ease, Ease::Linear);
}

#[test]
fn an_escape_gives_up_a_held_handle_and_the_arrows_move_a_focused_one() {
    let (mut bench, clip, _) = graph();
    bench.click(OPACITY_START);
    let out = "Opacity ease out of 00:00:01:00";
    let before = bench.steps();
    let end = bench.drag(out, vec2(40.0, 10.0));
    bench.key(Key::Escape);
    bench.release(end);
    assert_eq!(bench.steps(), before);
    assert_eq!(bench.clip(clip).animation[0].keys[0].ease, Ease::Linear);

    // Selected again, the handle takes the keyboard and the arrows.
    bench.click(OPACITY_START);
    bench.focus(out);
    bench.key(Key::ArrowRight);
    assert_eq!(bench.steps(), before + 1);
    assert_eq!(bench.last(), "Change easing");
    let Ease::Bezier { x1, .. } = bench.clip(clip).animation[0].keys[0].ease else {
        panic!("a cubic");
    };
    assert!((x1 - (1.0 / 3.0 + 0.05)).abs() < 0.002, "{x1}");
    // The keys did not move with it.
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0), (2.0, 1.0)]);
    assert_eq!(bench.stepped, 0);
}

#[test]
fn the_arrows_nudge_a_key_in_the_graph_in_time_and_in_value() {
    let (mut bench, clip, glow) = graph();
    bench.click(GLOW_START);
    let before = bench.steps();
    bench.key(Key::ArrowUp);
    bench.key(Key::ArrowRight);
    assert_eq!(bench.steps(), before + 2);
    let amount = bench.track_keys(clip, Some(glow), ParamKey::Amount);
    assert!(amount[0].1 > 0.25 && amount[0].1 < 0.3, "{amount:?}");
    assert!((amount[0].0 - (1.0 + 1.0 / 30.0)).abs() < 1e-5);
    assert_eq!(bench.stepped, 0);
}

#[test]
fn a_click_on_the_graph_moves_the_playhead_there() {
    let (mut bench, _, _) = graph();
    let key = bench.at(OPACITY_END);
    bench.click_at(key + vec2(0.5 * bench.state.zoom, 25.0));
    assert!((bench.playhead.as_seconds() - 3.5).abs() < 0.02);
}

// ---- A short and narrow timeline ------------------------------------------

#[test]
fn the_lane_and_the_graph_fit_a_narrow_windows_timeline() {
    for size in [vec2(748.0, 170.0), vec2(748.0, 132.0)] {
        let (mut bench, _, _) = Bench::new();
        bench.size = size;
        bench.state.fit = true;
        bench.frame(Vec::new());
        let window = Rect::from_min_size(Pos2::ZERO, size);
        bench.click("Keyframes of shot.mp4");
        bench.frame(Vec::new());
        // The lane is as tall as its rows, whatever the timeline's height;
        // what does not fit is reached by scrolling the tracks.
        let first = bench.find(OPACITY_START).expect("named").rect;
        assert!(window.contains_rect(first), "{first:?} in {size:?}");
        assert!(first.height() >= ROW - 2.0);

        bench.click("Graph editor (G)");
        bench.frame(Vec::new());
        for name in [OPACITY_START, OPACITY_END, GLOW_START, GLOW_END] {
            let rect = bench.find(name).expect("named").rect;
            assert!(window.contains_rect(rect), "{name} {rect:?} in {size:?}");
        }
        // The keys keep their spread: the values are fitted to the height.
        let spread = bench.at(OPACITY_START).y - bench.at(OPACITY_END).y;
        assert!(spread >= 24.0, "{spread} in {size:?}");
        for name in [
            "Fit values",
            "Keyframe options",
            "Show Opacity",
            "Solo Opacity",
        ] {
            let rect = bench.find(name).expect("named").rect;
            assert!(window.contains_rect(rect), "{name} {rect:?} in {size:?}");
        }
        // The strip's two toggles are still on it.
        for name in ["Graph editor (G)", "Keyframes of the selected clip (U)"] {
            let rect = bench.find(name).expect("named").rect;
            assert!(window.contains_rect(rect) && rect.width() >= 28.0);
        }
    }
}

// ---- Review of wave 3 -------------------------------------------------------

#[test]
fn a_held_key_is_written_into_the_clip_as_it_is_when_it_is_let_go() {
    let (mut bench, clip, _) = lane();
    // A third key, which an undo takes away while the first is held.
    bench.keys(
        clip,
        None,
        ParamKey::Opacity,
        vec![key(0.0, 0.0), key(2.0, 1.0), key(3.5, 0.5)],
    );
    bench.frame(Vec::new());
    let end = bench.drag(OPACITY_START, vec2(0.5 * bench.state.zoom, 0.0));
    assert!(
        bench.editor.undo(),
        "the third key is undone under the drag"
    );
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0), (2.0, 1.0)]);

    let before = bench.steps();
    bench.release(end);
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    assert_eq!(
        bench.opacity(clip),
        vec![(0.5, 0.0), (2.0, 1.0)],
        "the key that was undone is not written back"
    );
}

#[test]
fn a_paste_of_something_copied_since_is_not_a_paste_of_keyframes() {
    let (mut bench, clip, glow) = lane();
    bench.click(GLOW_START);
    bench.frame(vec![Event::Copy]);
    let marker = bench.state.keys.copied_text().to_owned();
    assert!(!marker.is_empty());
    bench.playhead = seconds(1.5);
    let before = bench.steps();

    // The clipboard now holds what another window copied.
    bench.frame(vec![Event::Paste("clips of another window".to_owned())]);
    bench.frame(Vec::new());
    assert_eq!(bench.clip_chords, 1, "the paste is the timeline's");
    assert_eq!(bench.steps(), before, "and no keyframe is written");
    let amount = bench.track_keys(clip, Some(glow), ParamKey::Amount);
    assert_eq!(amount, vec![(1.0, 0.25), (3.0, 0.75)]);

    // And the keyframes copied before it are no longer what a paste is of.
    bench.frame(vec![Event::Paste(marker)]);
    bench.frame(Vec::new());
    assert_eq!(bench.clip_chords, 2);
    assert_eq!(bench.steps(), before);
}

#[test]
fn keys_are_let_go_when_another_clip_is_selected_away_from_the_tracks() {
    let (mut bench, clip, _) = lane();
    let asset = (bench.editor.project().assets.values().next())
        .expect("FIXTURE: the shot's asset")
        .clone();
    let mut other = Clip::from_asset(&asset, seconds(6.0));
    other.duration = seconds(2.0);
    let other_id = other.id;
    let track_id = bench.track(TrackKind::Video);
    bench.edit(Command::AddClip {
        track_id,
        clip: other,
    });
    bench.click(OPACITY_END);
    assert_eq!(bench.state.keys.selected().len(), 1);
    assert_eq!(bench.selection.single(), Some(clip));

    // As a click on the canvas selects it: no press on the tracks.
    bench.selection.only(other_id);
    let before = bench.steps();
    bench.key(Key::Delete);
    assert_eq!(bench.clip_deletes, 1, "Delete is the selected clip's");
    assert_eq!(bench.steps(), before, "and no keyframe went");
    assert_eq!(bench.opacity(clip), vec![(0.0, 0.0), (2.0, 1.0)]);
    assert!(bench.state.keys.selected().is_empty());
    bench.key(Key::ArrowRight);
    assert_eq!(bench.stepped, 1, "the arrows step the playhead");
    bench.frame(vec![Event::Copy]);
    assert_eq!(bench.clip_chords, 1, "and a copy is of the clip");

    // A key taken hold of in the open lane selects its clip again.
    bench.click(OPACITY_END);
    assert_eq!(bench.selection.single(), Some(clip));
    assert_eq!(bench.state.keys.selected().len(), 1);
}

#[test]
fn a_key_moved_along_in_the_graph_keeps_its_value_beside_a_far_larger_one() {
    let (mut bench, clip, _) = Bench::new();
    // A place in the hundreds on the same axis as an opacity of 0..=1.
    bench.keys(
        clip,
        None,
        ParamKey::PositionX,
        vec![key(0.0, 0.0), key(2.0, 500.0)],
    );
    bench.click("Graph editor (G)");
    bench.frame(Vec::new());

    // Half a second earlier, with the little up and down a hand has.
    let end = bench.drag(OPACITY_END, vec2(-0.5 * bench.state.zoom, 2.0));
    bench.release(end);
    assert_eq!(bench.last(), "Move keyframes");
    assert_eq!(
        bench.opacity(clip),
        vec![(0.0, 0.0), (1.5, 1.0)],
        "moved in time, and its value is the one it had"
    );

    // An arrow moves it by a hundredth of its own range, not of the axis.
    bench.click("Opacity keyframe at 00:00:02:15");
    bench.key(Key::ArrowDown);
    let value = bench.opacity(clip)[1].1;
    assert!((value - 0.99).abs() < 1e-4, "{value}");
}

#[test]
fn a_dragged_value_lands_no_coarser_than_a_hundredth_of_its_range() {
    // An opacity drawn at a quarter of a point to the unit.
    assert_eq!(unit(0.24, Some((0.0, 1.0))), 0.01);
    // A place in pixels on the same axis lands on whole pixels.
    assert_eq!(unit(0.24, Some((-100_000.0, 100_000.0))), 1.0);
    // Alone on the axis it lands as finely as a point tells apart.
    assert_eq!(unit(250.0, Some((0.0, 1.0))), 0.001);
    assert_eq!(unit(0.24, None), 1.0);
}

// ---- The empty Volume row --------------------------------------------------

/// A bench with one clip of four seconds a second in and no keyframes,
/// selected: of `media` on a track of `kind`, with sound or without.
fn unkeyed(name: &str, media: MediaKind, sound: bool, kind: TrackKind) -> (Bench, ClipId) {
    let mut asset = Asset::new(
        PathBuf::from(format!("/media/{name}")),
        media,
        seconds(10.0),
    );
    asset.name = name.to_owned();
    asset.has_audio = sound;
    let mut bench = Bench::empty();
    bench.edit(Command::AddAssets {
        assets: vec![asset.clone()],
    });
    let mut clip = Clip::from_asset(&asset, seconds(1.0));
    clip.duration = seconds(4.0);
    let id = clip.id;
    let track_id = bench.track(kind);
    bench.edit(Command::AddClip { track_id, clip });
    bench.selection.only(id);
    bench.frame(Vec::new());
    (bench, id)
}

const STRIP_TOGGLE: &str = "Keyframes of the selected clip (U)";
const VOLUME_ROW: &str = "Select Volume keyframes";

#[test]
fn a_sound_clip_without_keys_opens_a_lane_of_one_empty_volume_row() {
    let (mut bench, voice) = unkeyed("voice.wav", MediaKind::Audio, true, TrackKind::Audio);
    // Closed, the tile has no chevron: the tool strip and U open the lane.
    assert!(!bench.has("Keyframes of voice.wav"));
    assert!(
        bench
            .find(STRIP_TOGGLE)
            .is_some_and(|strip| !strip.disabled)
    );
    assert!(!bench.has(VOLUME_ROW));
    bench.key(Key::U);
    assert_eq!(bench.state.keys.lane_clip(), Some(voice));
    let project = bench.editor.project();
    let track = project.clip(voice).unwrap().0.id;
    assert_eq!(bench.state.keys.lane_rows(project), Some((track, 1)));
    // One row, named, with no key in it, and nothing was edited to show it.
    assert!(bench.has(VOLUME_ROW));
    assert!(
        !bench
            .seen
            .iter()
            .any(|seen| seen.name.contains("keyframe at"))
    );
    assert!(bench.clip(voice).animation.is_empty());
    assert_eq!(bench.steps(), 2, "the fixture's two edits and no more");
    // Open, the tile carries the chevron that closes it.
    bench.click("Keyframes of voice.wav");
    assert_eq!(bench.state.keys.lane_clip(), None);
    assert!(!bench.has(VOLUME_ROW));
    bench.click(STRIP_TOGGLE);
    assert!(bench.has(VOLUME_ROW));

    // A key written into the row is drawn in it, and with the last key
    // deleted the row is empty again and stays.
    bench.keys(voice, None, ParamKey::Gain, vec![key(1.0, 0.5)]);
    bench.frame(Vec::new());
    const KEY: &str = "Volume keyframe at 00:00:02:00";
    assert!(bench.has(KEY) && bench.has(VOLUME_ROW));
    bench.click(KEY);
    bench.frame(vec![Event::Copy]);
    bench.key(Key::Delete);
    assert!(
        bench.clip(voice).animation.is_empty(),
        "{:?}",
        bench.refused
    );
    assert_eq!(bench.clip_deletes, 0, "the key was deleted, not the clip");
    assert!(bench.has(VOLUME_ROW) && !bench.has(KEY));
    // The empty row takes a paste of the copied key at the playhead.
    bench.playhead = seconds(3.0);
    let before = bench.steps();
    bench.frame(vec![Event::Paste(
        bench.state.keys.copied_text().to_owned(),
    )]);
    bench.frame(Vec::new());
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    assert_eq!(
        bench.clip_chords, 0,
        "the paste was the keys', not the clips'"
    );
    assert_eq!(bench.track_keys(voice, None, ParamKey::Gain), [(2.0, 0.5)]);
    assert!(bench.editor.undo());
    bench.frame(Vec::new());
    assert!(bench.has(VOLUME_ROW));
    bench.said.clear();
    // A click in the empty row moves the playhead, as in any lane.
    let row = bench.find(VOLUME_ROW).expect("named").rect.center();
    bench.click_at(pos2(400.0, row.y));
    assert!(bench.playhead > Time::ZERO);
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}

#[test]
fn a_picture_with_sound_opens_the_volume_row_and_one_muted_or_silenced_keeps_it() {
    let (mut bench, shot) = unkeyed("shot.mp4", MediaKind::Video, true, TrackKind::Video);
    bench.key(Key::U);
    assert_eq!(bench.state.keys.lane_clip(), Some(shot));
    assert!(bench.has(VOLUME_ROW));
    // At volume 0 on a muted track it is not in the mix, and its volume is
    // keyed all the same: that is how it is brought back in.
    let track_id = bench.track(TrackKind::Video);
    bench.edit(Command::SetTrackState {
        track_id,
        locked: None,
        hidden: None,
        muted: Some(true),
        name: None,
        gain: None,
        solo: None,
    });
    bench.edit(Command::SetGain {
        clip_id: shot,
        gain: 0.0,
    });
    bench.frame(Vec::new());
    let project = bench.editor.project();
    let (track, clip) = project.clip(shot).unwrap();
    assert!(!editor_core::sound::sounds(project, track, clip));
    assert!(bench.has(VOLUME_ROW));
}

#[test]
fn a_still_a_silent_video_and_an_owned_picture_have_no_lane_to_open() {
    for (name, media, sound) in [
        ("still.png", MediaKind::Image, false),
        ("mute.mp4", MediaKind::Video, false),
    ] {
        let (mut bench, _) = unkeyed(name, media, sound, TrackKind::Video);
        assert!(bench.find(STRIP_TOGGLE).is_some_and(|strip| strip.disabled));
        let project = bench.editor.project();
        assert!(!bench.state.keys.can_toggle_lane(project, &bench.selection));
        bench.key(Key::U);
        assert_eq!(bench.state.keys.lane_clip(), None, "{name}");
        assert!(!bench.has(VOLUME_ROW), "{name}");
    }

    // A picture whose sound its linked audio clip owns: the audio clip has
    // the row, the picture has none.
    let (mut bench, shot) = unkeyed("shot.mp4", MediaKind::Video, true, TrackKind::Video);
    bench.key(Key::U);
    assert!(bench.has(VOLUME_ROW));
    let detach = rules::detach_audio(bench.editor.project(), shot).unwrap();
    bench.edit(detach);
    bench.frame(Vec::new());
    // The lane that was open has no row left, and takes no room.
    assert!(!bench.has(VOLUME_ROW));
    assert_eq!(bench.state.keys.lane_rows(bench.editor.project()), None);
    bench.state.keys.set_lane(None);
    bench.frame(Vec::new());
    assert!(bench.find(STRIP_TOGGLE).is_some_and(|strip| strip.disabled));
    bench.key(Key::U);
    assert_eq!(bench.state.keys.lane_clip(), None);
    let sound = (bench.editor.project().tracks.iter())
        .filter(|track| track.kind == TrackKind::Audio)
        .flat_map(|track| &track.clips)
        .find(|clip| clip.linked_id.is_some())
        .expect("the detached sound")
        .id;
    bench.selection.only(sound);
    bench.key(Key::U);
    assert_eq!(bench.state.keys.lane_clip(), Some(sound));
    assert!(bench.has(VOLUME_ROW));
    // Unlinked, the picture's sound is its own again.
    bench.edit(Command::UnlinkClips {
        clip_ids: vec![shot],
    });
    bench.selection.only(shot);
    bench.key(Key::U);
    assert_eq!(bench.state.keys.lane_clip(), Some(shot));
    assert!(bench.has(VOLUME_ROW));
    // The graph draws curves, and says a clip without keyframes has none.
    bench.click("Graph editor (G)");
    assert!(bench.state.keys.graph);
    bench.frame(Vec::new());
    assert!(!bench.has(VOLUME_ROW));
}

#[test]
fn a_volume_key_is_written_at_the_volume_the_clip_has_there_or_refused_in_words() {
    let (mut bench, voice) = unkeyed("voice.wav", MediaKind::Audio, true, TrackKind::Audio);
    bench.edit(Command::SetGain {
        clip_id: voice,
        gain: 0.25,
    });
    let ask = |bench: &Bench, at: f64| volume_key(bench.editor.project(), voice, seconds(at));
    // Off the clip, before it and after it.
    for at in [0.5, 5.5] {
        let off = ask(&bench, at).unwrap_err();
        assert_eq!(off, "Put the playhead on the clip to key its volume.");
    }
    // On it, at its first and last times too: one batch, one step.
    let steps = bench.steps();
    bench.edit(ask(&bench, 1.0).unwrap());
    bench.edit(ask(&bench, 5.0).unwrap());
    assert_eq!(bench.steps(), steps + 2);
    assert_eq!(bench.last(), "Key volume");
    let keys = |bench: &Bench| bench.track_keys(voice, None, ParamKey::Gain);
    assert_eq!(keys(&bench), [(0.0, 0.25), (4.0, 0.25)]);
    let there = ask(&bench, 5.0).unwrap_err();
    assert_eq!(there, "The volume is keyed there already.");
    // Between keys the new one has the value the track has there, the keys
    // beside it keep theirs, and the eased stretch it divides its shape.
    bench.keys(
        voice,
        None,
        ParamKey::Gain,
        vec![
            Keyframe {
                at: Time::ZERO,
                value: 0.0,
                ease: EASE_IN,
            },
            key(4.0, 2.0),
        ],
    );
    let before = gain_at(bench.clip(voice), seconds(2.0));
    let eased = bench.clip(voice).clone();
    assert!(
        (before - 0.5).abs() > 0.1,
        "the stretch is not a straight one"
    );
    bench.edit(ask(&bench, 2.0).unwrap());
    assert_eq!(keys(&bench), [(0.0, 0.0), (1.0, before), (4.0, 2.0)]);
    for step in 0..=400 {
        let at = seconds(1.0) + Time(step * 10_000);
        let (was, is) = (gain_at(&eased, at), gain_at(bench.clip(voice), at));
        assert!((was - is).abs() <= 2e-4, "{at:?}: {was} became {is}");
    }
    assert!(bench.editor.undo());
    assert_eq!(keys(&bench), [(0.0, 0.0), (4.0, 2.0)]);

    // A locked track is refused in words, and by the project if asked.
    let (track, clip) = bench.editor.project().clip(voice).unwrap();
    let (track_id, direct) = (track.id, key_volume(clip, seconds(2.0)).unwrap());
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    });
    let locked = ask(&bench, 3.0).unwrap_err();
    assert_eq!(locked, "Unlock the clip's track to key its volume.");
    let held = bench.editor.project().clone();
    assert!(bench.editor.dispatch(direct).is_err());
    assert_eq!(*bench.editor.project(), held);

    // A clip without sound, and one whose sound its audio clip owns.
    let (bench, still) = unkeyed("still.png", MediaKind::Image, false, TrackKind::Video);
    let silent = volume_key(bench.editor.project(), still, seconds(2.0)).unwrap_err();
    assert_eq!(silent, "That clip has no sound to key the volume of.");
    let (mut bench, shot) = unkeyed("shot.mp4", MediaKind::Video, true, TrackKind::Video);
    let detach = rules::detach_audio(bench.editor.project(), shot).unwrap();
    bench.edit(detach);
    let owned = volume_key(bench.editor.project(), shot, seconds(2.0)).unwrap_err();
    assert_eq!(owned, "Sound is on the linked audio clip.");
}
