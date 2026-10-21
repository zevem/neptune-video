//! The handles of a clip's fades, driven as a person drives them.
//!
//! [`Bench`] paints the timeline in a real egui pass and hands it pointer
//! and key events. A handle and a menu row are found by the name
//! accessibility is given for them. The bench ends a frame as the
//! application does: an [`Action::Edit`] is dispatched, and an Escape gives
//! up what is held before the frame is drawn. Every edit below is one
//! `Editor::dispatch` accepted.
use super::*;
use crate::theme::{Accent, Appearance};
use editor_core::{Editor, MediaKind};
use eframe::egui::{Event, Key, Modifiers, PointerButton, accesskit::Role};
use std::sync::Arc;

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

struct Seen {
    name: String,
    rect: Rect,
    role: Role,
    disabled: bool,
}

struct Bench {
    ctx: egui::Context,
    palette: Palette,
    editor: Editor,
    timeline: TimelineState,
    selection: Selection,
    playhead: Time,
    snapping: bool,
    size: Vec2,
    time: f64,
    seen: Vec<Seen>,
    /// The bounds of each mesh of bare colour the last frame drew. No
    /// peaks are held here, so no waveform is one: each is a fade.
    shaded: Vec<Rect>,
    refused: Vec<String>,
    said: Vec<String>,
    /// The clips the menu asked to have measured and normalized.
    normalized: Vec<Vec<ClipId>>,
    /// The clips the menu asked to have ducked, and how deep.
    ducked: Vec<(Vec<ClipId>, f32)>,
    /// The sounds the menu asked to have saved as files.
    saved: Vec<crate::extract::Source>,
    /// Whether the session has a stem separator, and the clips the menu
    /// asked to have separated.
    separator: crate::stems::Tool,
    separated: Vec<ClipId>,
}

const VOICE_IN: &str = "Fade in of voice.wav";
const VOICE_OUT: &str = "Fade out of voice.wav";

impl Bench {
    /// Three rows, each with a clip from one second to five: `shot.mp4`, a
    /// video with sound, on a video track, `voice.wav` on an audio track and
    /// `bed.wav` on a second audio track. Nothing is selected and the
    /// playhead is at the start.
    fn new() -> (Self, [ClipId; 3]) {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut project = Project::new("Fades");
        project.tracks.clear();
        let mut ids = Vec::new();
        for (name, media, track, kind) in [
            ("shot.mp4", MediaKind::Video, "Video 1", TrackKind::Video),
            ("voice.wav", MediaKind::Audio, "Audio 1", TrackKind::Audio),
            ("bed.wav", MediaKind::Audio, "Audio 2", TrackKind::Audio),
        ] {
            let mut asset = Asset::new(format!("/media/{name}").into(), media, seconds(10.0));
            asset.name = name.to_owned();
            asset.has_audio = true;
            let mut clip = Clip::from_asset(&asset, seconds(1.0));
            clip.duration = seconds(4.0);
            ids.push(clip.id);
            let mut track = Track::new(track, kind);
            track.clips.push(clip);
            Arc::make_mut(&mut project.assets).insert(asset.id, asset);
            project.tracks.push(Arc::new(track));
        }
        let mut bench = Self {
            ctx,
            palette,
            editor: Editor::new(project).expect("FIXTURE: the project is valid"),
            timeline: TimelineState::default(),
            selection: Selection::default(),
            playhead: Time::ZERO,
            snapping: false,
            size: vec2(900.0, 300.0),
            time: 0.0,
            seen: Vec::new(),
            shaded: Vec::new(),
            refused: Vec::new(),
            said: Vec::new(),
            normalized: Vec::new(),
            ducked: Vec::new(),
            saved: Vec::new(),
            separator: crate::stems::Tool::Missing,
            separated: Vec::new(),
        };
        bench.frame(Vec::new());
        (bench, [ids[0], ids[1], ids[2]])
    }

    /// The bench with `voice.wav` selected, so that its handles are shown.
    fn selected() -> (Self, ClipId) {
        let (mut bench, [_, voice, _]) = Self::new();
        bench.selection.only(voice);
        bench.frame(Vec::new());
        (bench, voice)
    }

    /// A fixture's own edit: not one the timeline made.
    fn edit(&mut self, command: Command) {
        let label = command.label().to_owned();
        if let Err(error) = self.editor.dispatch(command) {
            panic!("FIXTURE: \"{label}\" was refused: {error}");
        }
        self.frame(Vec::new());
    }

    fn lock(&mut self, clip: ClipId) {
        let track_id = self.editor.project().clip(clip).expect("the clip").0.id;
        self.edit(Command::SetTrackState {
            track_id,
            locked: Some(true),
            hidden: None,
            muted: None,
            name: None,
            gain: None,
            solo: None,
        });
    }

    fn fades(&self, clip: ClipId) -> (Time, Time) {
        let clip = self.editor.project().clip(clip).expect("the clip").1;
        (clip.fade_in, clip.fade_out)
    }

    fn steps(&self) -> usize {
        self.editor.history().count()
    }

    fn last(&self) -> String {
        self.editor.undo_label().unwrap_or_default().to_owned()
    }

    /// One frame of the application with these events in it.
    fn frame(&mut self, events: Vec<Event>) {
        fn meshes(shape: &egui::Shape, found: &mut Vec<Rect>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| meshes(shape, found)),
                egui::Shape::Mesh(mesh) if mesh.texture_id == egui::TextureId::default() => {
                    found.push(mesh.calc_bounds());
                }
                _ => {}
            }
        }
        self.time += 0.05;
        let ctx = self.ctx.clone();
        let project = self.editor.snapshot();
        let mut actions: Vec<Action> = Vec::new();
        // The application's Escape, before the frame is drawn: what is held
        // is given up, and only then the selection.
        let escaped = events.iter().any(|event| {
            matches!(
                event,
                Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                }
            )
        });
        if escaped && !egui::Popup::is_any_open(&ctx) && !self.timeline.cancel_drag() {
            self.selection.clear();
        }
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            show(
                ui,
                ui.max_rect(),
                &mut self.timeline,
                &mut self.selection,
                &mut Textures::default(),
                &mut HeldPeaks::default(),
                &TimelineInput {
                    p: &self.palette,
                    project: &project,
                    playhead: self.playhead,
                    snapping: self.snapping,
                    media: None,
                    can_paste: false,
                    freeze_hold: seconds(2.0),
                    separator: self.separator,
                },
                &mut actions,
            );
        });
        output.textures_delta.clear();
        self.shaded.clear();
        for clipped in &output.shapes {
            meshes(&clipped.shape, &mut self.shaded);
        }
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
                Action::Normalize(clips) => self.normalized.push(clips),
                Action::Duck { clips, depth } => self.ducked.push((clips, depth)),
                Action::SaveAudio(source) => self.saved.push(source),
                Action::SeparateStems(clip) => self.separated.push(clip),
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

    /// Where a time of the project is in the lanes.
    fn x(&self, at: f64) -> f32 {
        HEAD_WIDTH + at as f32 * self.timeline.zoom - self.timeline.scroll.x
    }

    /// The tile of the clip on the row of this track, as the lanes lay it
    /// out.
    fn tile(&self, clip: ClipId) -> Rect {
        let project = self.editor.project();
        let lanes = Rect::from_min_max(pos2(HEAD_WIDTH, STRIP + RULER), self.size.to_pos2());
        let geometry = Geometry {
            lanes,
            zoom: self.timeline.zoom,
            scroll: self.timeline.scroll.x,
        };
        let rows = lay_out(display_order(project), lanes.top() + LANE_GAP);
        let (row, clip) = rows
            .iter()
            .find_map(|row| {
                let clip = row.track.clips.iter().find(|held| held.id == clip)?;
                Some((row, clip))
            })
            .expect("FIXTURE: the clip is on a row");
        clip_rect(&geometry, row, clip.start, clip.end())
    }

    fn button(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn move_to(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
    }

    fn press(&mut self, pos: Pos2) {
        self.move_to(pos);
        self.frame(vec![Self::button(pos, PointerButton::Primary, true)]);
    }

    fn release(&mut self, pos: Pos2) {
        self.frame(vec![Self::button(pos, PointerButton::Primary, false)]);
        // The frame after the release shows what it committed.
        self.frame(Vec::new());
    }

    /// Takes hold at `from` and carries to `to`, without letting go.
    fn carry(&mut self, from: Pos2, to: Pos2) {
        self.press(from);
        for step in 1..=5 {
            self.move_to(from + (to - from) * (step as f32 / 5.0));
        }
    }

    /// Where the pointer takes the handle with this name, taken hold of at
    /// its middle, for its fade to end at this time of the project. A
    /// handle stands clear of the grips, so that of a short fade is not
    /// where the fade ends, and the fade follows the travel.
    fn reach(&self, name: &str, to: f64) -> Pos2 {
        let project = self.editor.project();
        let (edge, clip) = (fades::EDGES.into_iter())
            .find_map(|edge| {
                let of = name.strip_prefix(fades::name(edge))?.strip_prefix(" of ")?;
                let mut clips = project.tracks.iter().flat_map(|track| &track.clips);
                Some((edge, clips.find(|clip| clip.name == of)?))
            })
            .expect("FIXTURE: the handle of a fade of a clip");
        let from = self.at(name);
        let end = fade_end(
            self.tile(clip.id),
            self.timeline.zoom,
            edge,
            fades::of(clip, edge),
        );
        pos2(self.x(to) + from.x - end, from.y)
    }

    /// Carries the handle with this name until its fade ends at this time
    /// of the project and lets it go there.
    fn drag(&mut self, name: &str, to: f64) {
        self.frame(Vec::new());
        let (from, to) = (self.at(name), self.reach(name, to));
        self.carry(from, to);
        self.release(to);
    }

    fn click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = self.at(name);
        self.press(pos);
        self.release(pos);
    }

    fn secondary_click_at(&mut self, pos: Pos2) {
        self.move_to(pos);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, true)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, false)]);
        self.frame(Vec::new());
    }

    /// Opens the clip menu at `pos` and, where it has one, its Audio menu:
    /// the rows of a clip's sound are in it (§5.7.3).
    fn audio_menu_at(&mut self, pos: Pos2) {
        self.secondary_click_at(pos);
        if let Some(audio) = self.find("Audio").map(|row| row.rect.center()) {
            for _ in 0..4 {
                self.frame(vec![Event::PointerMoved(audio)]);
            }
        }
    }

    fn escape(&mut self) {
        self.frame(vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        self.frame(Vec::new());
    }

    /// Closes the menus open, one Escape each (§5.7.1): the Audio menu,
    /// then the clip menu it opened from.
    fn close_menus(&mut self) {
        self.escape();
        if self.has("Split at playhead") {
            self.escape();
        }
    }
}

// ---- The handles ----------------------------------------------------------

#[test]
fn a_handle_drag_previews_the_fade_and_is_one_step_on_release() {
    let (mut bench, voice) = Bench::selected();
    let steps = bench.steps();
    assert!(bench.shaded.is_empty(), "a clip without fades draws none");
    let from = bench.at(VOICE_IN);
    // To two seconds of the project: a second into the clip.
    let (to, end) = (bench.reach(VOICE_IN, 2.0), bench.x(2.0));
    bench.carry(from, to);
    bench.frame(Vec::new());
    // Held, nothing is edited yet: the handle and the curve are where the
    // fade is held.
    assert_eq!(bench.steps(), steps);
    assert_eq!(bench.fades(voice), (Time::ZERO, Time::ZERO));
    assert!((bench.at(VOICE_IN).x - end).abs() < 0.01);
    let tile = bench.tile(voice);
    let [curve] = bench.shaded.as_slice() else {
        panic!("one fade is drawn: {:?}", bench.shaded);
    };
    assert!(
        (curve.left() - (tile.left() + 1.0)).abs() < 0.01,
        "{curve:?}"
    );
    assert!((curve.right() - end).abs() < 0.01, "{curve:?}");
    assert!(sound_area(tile).contains_rect(*curve), "{curve:?}");

    bench.release(to);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Change fades");
    assert_eq!(bench.fades(voice), (seconds(1.0), Time::ZERO));
    assert_eq!(bench.shaded.len(), 1, "the stored fade is drawn");

    // The other edge, to half a second before the clip's end.
    bench.drag(VOICE_OUT, 4.5);
    assert_eq!(bench.steps(), steps + 2, "{:?}", bench.refused);
    assert_eq!(bench.fades(voice), (seconds(1.0), seconds(0.5)));
    assert_eq!(bench.shaded.len(), 2);

    // Each drag undoes by itself.
    assert!(bench.editor.undo());
    assert_eq!(bench.fades(voice), (seconds(1.0), Time::ZERO));
    assert!(bench.editor.undo());
    assert_eq!(bench.fades(voice), (Time::ZERO, Time::ZERO));
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}

#[test]
fn escape_gives_up_a_held_handle_and_leaves_the_project_unchanged() {
    let (mut bench, voice) = Bench::selected();
    bench.drag(VOICE_OUT, 4.0);
    let (steps, before) = (bench.steps(), bench.editor.project().clone());
    let from = bench.at(VOICE_IN);
    let to = pos2(bench.x(3.0), from.y);
    bench.carry(from, to);
    assert!(matches!(bench.timeline.drag, Some(Drag::Fade { .. })));
    bench.escape();
    assert!(bench.timeline.drag.is_none());
    // The pointer is still down and still moving: nothing follows it.
    bench.move_to(to + vec2(40.0, 0.0));
    bench.release(to + vec2(40.0, 0.0));
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
    // The Escape let go of the handle, not of the clip, and what is drawn
    // is the stored fade again.
    assert_eq!(bench.selection.clips, [voice]);
    assert_eq!(bench.shaded.len(), 1);
    // The next drag is its own, and is kept.
    bench.drag(VOICE_IN, 2.0);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.fades(voice), (seconds(1.0), seconds(1.0)));
}

#[test]
fn a_dragged_fade_stops_at_the_other_fade_and_at_the_clip() {
    let (mut bench, voice) = Bench::selected();
    let steps = bench.steps();
    // Past the clip's end: the whole clip, and no more.
    bench.drag(VOICE_IN, 9.0);
    assert_eq!(bench.fades(voice), (seconds(4.0), Time::ZERO));
    // The fade out has no room left: dragged, it stays at nothing.
    bench.drag(VOICE_OUT, 2.0);
    assert_eq!(bench.fades(voice), (seconds(4.0), Time::ZERO));
    assert_eq!(
        bench.steps(),
        steps + 1,
        "a drag that changes nothing is no step"
    );
    // Before the clip's start: no fade, never less.
    bench.drag(VOICE_IN, 0.25);
    assert_eq!(bench.fades(voice), (Time::ZERO, Time::ZERO));
    assert_eq!(bench.steps(), steps + 2);

    // With a second of fade out, the fade in has three seconds of room.
    bench.drag(VOICE_OUT, 4.0);
    bench.drag(VOICE_IN, 4.75);
    assert_eq!(bench.fades(voice), (seconds(3.0), seconds(1.0)));
    // The two meet: each keeps a handle, side by side, and the fade out
    // dragged across the fade in stays where the fade in ends.
    let (start, end) = (bench.rect(VOICE_IN), bench.rect(VOICE_OUT));
    assert!(start.right() <= end.left() + 0.01, "{start:?} {end:?}");
    let before = bench.steps();
    bench.drag(VOICE_OUT, 1.5);
    assert_eq!(bench.fades(voice), (seconds(3.0), seconds(1.0)));
    assert_eq!(bench.steps(), before);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_fade_lands_on_a_frame_and_on_the_playhead_when_snapping_is_on() {
    let (mut bench, voice) = Bench::selected();
    let rate = bench.editor.project().settings.frame_rate;
    // Between two frames: the nearer one.
    let loose = 2.02;
    assert_ne!(rules::quantize(seconds(loose), rate), seconds(loose));
    bench.drag(VOICE_IN, loose);
    let whole = rules::quantize(seconds(loose), rate);
    assert_eq!(bench.fades(voice).0, whole - seconds(1.0));

    // The playhead off any frame of the project, and the handle let go
    // three points from it: with snapping on the fade ends on the playhead.
    bench.playhead = Time(3_210_000);
    bench.snapping = true;
    let near = 3.21 + 3.0 / f64::from(bench.timeline.zoom);
    bench.drag(VOICE_IN, near);
    assert_eq!(bench.fades(voice).0, Time(2_210_000));
    // With it off the same place is a frame again.
    bench.snapping = false;
    bench.drag(VOICE_IN, near + 0.5);
    bench.drag(VOICE_IN, near);
    let whole = rules::quantize(seconds(near), rate);
    assert_eq!(bench.fades(voice).0, whole - seconds(1.0));
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_fade_follows_the_travel_of_a_handle_that_stands_clear_of_its_end() {
    // Twelve points to the second: a clip without fades has its handles
    // twelve points inside the tile, a second from where each fade ends.
    let (mut bench, voice) = Bench::selected();
    bench.timeline.zoom = 12.0;
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    let rate = bench.editor.project().settings.frame_rate;
    let (steps, tile) = (bench.steps(), bench.tile(voice));
    let handle = bench.at(VOICE_IN);
    assert!(handle.x - tile.left() > 11.0, "{handle:?} {tile:?}");
    // Towards the clip's start, where a fade is shorter: there is none to
    // shorten, and none is made.
    let to = handle - vec2(7.0, 0.0);
    bench.carry(handle, to);
    assert!(matches!(bench.timeline.drag, Some(Drag::Fade { .. })));
    bench.release(to);
    assert_eq!(bench.fades(voice), (Time::ZERO, Time::ZERO));
    assert_eq!(bench.steps(), steps);
    // Away from it, the fade is as long as the handle travelled: eight
    // points, to the frame.
    let travel = rules::quantize(seconds(1.0 + 8.0 / 12.0), rate) - seconds(1.0);
    let to = handle + vec2(8.0, 0.0);
    bench.carry(handle, to);
    bench.release(to);
    assert_eq!(bench.fades(voice), (travel, Time::ZERO));
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    // The fade out the same, the other way.
    let handle = bench.at(VOICE_OUT);
    assert!(tile.right() - handle.x > 11.0, "{handle:?} {tile:?}");
    let to = handle + vec2(7.0, 0.0);
    bench.carry(handle, to);
    bench.release(to);
    assert_eq!(bench.fades(voice), (travel, Time::ZERO));
    let to = handle - vec2(8.0, 0.0);
    bench.carry(handle, to);
    bench.release(to);
    let back = seconds(5.0) - rules::quantize(seconds(5.0 - 8.0 / 12.0), rate);
    assert_eq!(bench.fades(voice), (travel, back));
    assert_eq!(bench.steps(), steps + 2, "{:?}", bench.refused);

    // A handle carried away and back to where it was taken hold of leaves
    // the fade it has, though that is on no frame.
    let typed = Time(410_000);
    assert_ne!(rules::quantize(typed, rate), typed);
    bench.edit(Command::SetFades {
        clip_id: voice,
        fade_in: typed,
        fade_out: Time::ZERO,
    });
    let steps = bench.steps();
    let handle = bench.at(VOICE_IN);
    bench.carry(handle, handle + vec2(20.0, 0.0));
    bench.move_to(handle);
    bench.release(handle);
    assert_eq!(bench.fades(voice), (typed, Time::ZERO));
    assert_eq!(bench.steps(), steps);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_locked_track_shows_no_handle_and_takes_no_drag() {
    let (mut bench, [_, voice, bed]) = Bench::new();
    bench.selection.only(bed);
    bench.frame(Vec::new());
    let handle = bench.rect("Fade in of bed.wav");
    bench.lock(bed);
    assert!(!bench.has("Fade in of bed.wav") && !bench.has("Fade out of bed.wav"));
    // A drag from where the handle stood moves nothing, the clip neither.
    let (steps, before) = (bench.steps(), bench.editor.project().clone());
    let from = handle.center();
    let to = from + vec2(bench.timeline.zoom, 0.0);
    bench.carry(from, to);
    assert!(bench.timeline.drag.is_none());
    bench.release(to);
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);

    // A track locked while its clip's fade is held: the project refuses
    // what the release asks, and nothing is changed.
    bench.selection.only(voice);
    bench.frame(Vec::new());
    let from = bench.at(VOICE_IN);
    let to = pos2(bench.x(2.0), from.y);
    bench.carry(from, to);
    let (steps, track_id) = (
        bench.steps(),
        bench.editor.project().clip(voice).unwrap().0.id,
    );
    bench
        .editor
        .dispatch(Command::SetTrackState {
            track_id,
            locked: Some(true),
            hidden: None,
            muted: None,
            name: None,
            gain: None,
            solo: None,
        })
        .unwrap();
    let locked = bench.editor.project().clone();
    bench.release(to);
    assert_eq!(bench.steps(), steps + 1, "the lock alone is a step");
    assert_eq!(*bench.editor.project(), locked);
    assert_eq!(bench.fades(voice), (Time::ZERO, Time::ZERO));
}

#[test]
fn handles_are_on_a_sound_tile_that_is_selected_or_under_the_pointer() {
    let (mut bench, [shot, voice, bed]) = Bench::new();
    assert!(!bench.has(VOICE_IN) && !bench.has(VOICE_OUT));
    // Under the pointer, and still there with the pointer on the handle.
    let body = bench.tile(voice).center();
    bench.move_to(body);
    bench.frame(Vec::new());
    assert!(bench.has(VOICE_IN) && bench.has(VOICE_OUT));
    assert!(!bench.has("Fade in of bed.wav"), "only the tile under it");
    let handle = bench.at(VOICE_OUT);
    for _ in 0..3 {
        bench.move_to(handle);
        assert!(bench.has(VOICE_OUT), "a handle under the pointer stays");
    }
    // Taken hold of there, it fades the clip and selects it.
    let to = bench.reach(VOICE_OUT, 4.0);
    bench.carry(handle, to);
    bench.release(to);
    assert_eq!(bench.fades(voice), (Time::ZERO, seconds(1.0)));
    assert_eq!(bench.selection.clips, [voice]);
    // Away from the lanes the selected tile keeps its handles and the
    // other has none.
    bench.move_to(pos2(400.0, 10.0));
    bench.frame(Vec::new());
    assert!(bench.has(VOICE_IN));
    assert!(!bench.has("Fade in of bed.wav"));
    bench.selection.only(bed);
    bench.frame(Vec::new());
    assert!(!bench.has(VOICE_IN) && bench.has("Fade in of bed.wav"));

    // A picture tile has none, though its sound may fade.
    bench.selection.only(shot);
    bench.move_to(bench.tile(shot).center());
    bench.frame(Vec::new());
    assert!(!bench.has("Fade in of shot.mp4") && !bench.has("Fade out of shot.mp4"));

    // Nor has a tile too narrow for them: four seconds at eight points.
    bench.selection.only(voice);
    bench.timeline.zoom = 8.0;
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(bench.tile(voice).width() < FADE_TILE);
    assert!(!bench.has(VOICE_IN) && !bench.has(VOICE_OUT));
    bench.timeline.zoom = FADE_TILE / 4.0;
    bench.frame(Vec::new());
    bench.frame(Vec::new());
    assert!(bench.has(VOICE_IN) && bench.has(VOICE_OUT));
}

#[test]
fn a_tile_carried_or_trimmed_shows_its_handles_only_once_let_go() {
    let (mut bench, voice) = Bench::selected();
    let second = vec2(bench.timeline.zoom, 0.0);
    // Pressed and not yet moved: the handles stay.
    let body = bench.tile(voice).center();
    bench.press(body);
    assert!(bench.has(VOICE_IN) && bench.has(VOICE_OUT));
    // Carried a second on: none stands where the tile was.
    for step in 1..=5 {
        bench.move_to(body + second * (step as f32 / 5.0));
    }
    assert!(matches!(
        bench.timeline.drag,
        Some(Drag::Move { moved: true, .. })
    ));
    assert!(!bench.has(VOICE_IN) && !bench.has(VOICE_OUT));
    // Let go, they are on the tile where it now is.
    bench.release(body + second);
    let tile = bench.tile(voice);
    assert_eq!(tile.left(), bench.x(2.0));
    assert!(tile.contains(bench.at(VOICE_IN)) && tile.contains(bench.at(VOICE_OUT)));

    // Its end held and drawn in a second: none until it is let go.
    let grip = pos2(tile.right() - 2.0, tile.center().y);
    bench.carry(grip, grip - second);
    assert!(matches!(bench.timeline.drag, Some(Drag::Trim { .. })));
    assert!(!bench.has(VOICE_IN) && !bench.has(VOICE_OUT));
    bench.release(grip - second);
    let trimmed = bench.tile(voice);
    // About a second shorter: the end lands on a frame.
    assert!((trimmed.right() - (tile.right() - second.x)).abs() < 4.0);
    assert!(trimmed.contains(bench.at(VOICE_OUT)));
}

#[test]
fn a_handle_never_overlaps_a_trim_grip_at_any_width_that_shows_it() {
    // A clip of one second from the start of an audio track: its tile is as
    // many points wide as the zoom.
    let voice = Asset::new("/media/voice.wav".into(), MediaKind::Audio, seconds(1.0));
    let clip = Clip::from_asset(&voice, Time::ZERO);
    let mut track = Track::new("Audio 1", TrackKind::Audio);
    track.clips.push(clip);
    let mut project = Project::new("Grips");
    Arc::make_mut(&mut project.assets).insert(voice.id, voice);
    project.tracks = vec![Arc::new(track)];
    let fades = [0.0, 0.01, 0.1, 0.25, 0.5, 0.75, 0.9, 0.99, 1.0];
    let mut shown = 0;
    for tenth in 20..6400 {
        let width = tenth as f32 / 10.0;
        let geometry = Geometry {
            lanes: Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 200.0)),
            zoom: width,
            scroll: 0.0,
        };
        let rows = lay_out(display_order(&project), geometry.lanes.top() + LANE_GAP);
        let tile = clip_rect(&geometry, &rows[0], Time::ZERO, seconds(1.0));
        assert!((tile.width() - width).abs() < 0.01);
        for fade_in in fades {
            for fade_out in fades.into_iter().filter(|out| fade_in + out <= 1.0) {
                let (fade_in, fade_out) = (seconds(fade_in), seconds(fade_out));
                let Some([start, end]) = fade_handles(tile, width, fade_in, fade_out) else {
                    assert!(width < FADE_TILE, "{width}: a tile this wide has handles");
                    continue;
                };
                assert!(width >= FADE_TILE, "{width}: a tile this narrow has none");
                shown += 1;
                let said = format!("{width}: {fade_in:?} {fade_out:?}: {start:?} {end:?}");
                // The grips are seven points of each edge, as `hit` has
                // them: no point of a handle trims the clip.
                for handle in [start, end] {
                    assert!(handle.left() >= tile.left() + EDGE_GRIP - 0.01, "{said}");
                    assert!(handle.right() <= tile.right() - EDGE_GRIP + 0.01, "{said}");
                    for corner in [handle.left_top(), handle.right_bottom(), handle.center()] {
                        // Inside by a hair: an edge shared with the grip is
                        // the grip's.
                        let inside = corner + (handle.center() - corner) * 0.01;
                        let Hit::Clip { edge, .. } = hit(&rows, &geometry, inside) else {
                            panic!("{said}: off the tile");
                        };
                        assert!(edge.is_none(), "{said}: {inside:?} trims");
                    }
                    // Under the name, in the sound.
                    assert!(
                        sound_area(tile).expand(0.01).contains_rect(handle),
                        "{said}"
                    );
                    let spare = handle.size() - vec2(MARK_HANDLE, FADE_HANDLE);
                    assert!(spare.length() < 0.01, "{said}");
                }
                // And neither covers the other.
                assert!(start.right() <= end.left() + 0.01, "{said}");
                // Where there is room each stands where its fade ends.
                let ends = [
                    fade_end(tile, width, FadeEdge::In, fade_in),
                    fade_end(tile, width, FadeEdge::Out, fade_out),
                ];
                let clear = EDGE_GRIP + MARK_HANDLE * 0.5;
                let free = ends[1] - ends[0] >= MARK_HANDLE
                    && ends[0] >= tile.left() + clear
                    && ends[1] <= tile.right() - clear;
                if free {
                    assert!((start.center().x - ends[0]).abs() < 0.01, "{said}");
                    assert!((end.center().x - ends[1]).abs() < 0.01, "{said}");
                }
            }
        }
    }
    assert!(shown > 100_000);
}

#[test]
fn a_fade_is_drawn_as_the_curve_the_sound_follows() {
    let voice = Asset::new("/media/voice.wav".into(), MediaKind::Audio, seconds(10.0));
    let mut clip = Clip::from_asset(&voice, seconds(2.0));
    clip.duration = seconds(4.0);
    clip.fade_in = seconds(1.0);
    clip.fade_out = seconds(2.0);
    // A hundred points to the second.
    let tile = Rect::from_min_max(pos2(200.0, 10.0), pos2(600.0, 54.0));
    let area = sound_area(tile);
    let whole = (tile.left(), tile.right());
    let level = |point: &Pos2| (area.bottom() - point.y) / area.height();

    let rising = fade_curve(&clip, tile, area, 100.0, FadeEdge::In, whole);
    assert_eq!(rising.len(), 51, "a point every two, and the end");
    assert_eq!((rising[0].x, rising[50].x), (200.0, 300.0));
    assert_eq!(level(&rising[0]), 0.0);
    assert!((level(&rising[50]) - 1.0).abs() < 1e-6);
    // Equal power: half way along, the sound is at the root of a half.
    assert!((level(&rising[25]) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    assert!(rising.windows(2).all(|pair| pair[0].y > pair[1].y));
    for point in &rising {
        let local = Time::from_seconds(f64::from(point.x - tile.left()) / 100.0);
        assert!((level(point) - sound::fade_at(&clip, local)).abs() < 1e-6);
    }

    let falling = fade_curve(&clip, tile, area, 100.0, FadeEdge::Out, whole);
    assert_eq!(falling.len(), 101);
    assert_eq!((falling[0].x, falling[100].x), (400.0, 600.0));
    assert!((level(&falling[0]) - 1.0).abs() < 1e-6);
    assert!(level(&falling[100]).abs() < 1e-6);
    assert!((level(&falling[50]) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    assert!(falling.windows(2).all(|pair| pair[0].y < pair[1].y));

    // Only the part in view is worked out, from where the view starts.
    let part = fade_curve(&clip, tile, area, 100.0, FadeEdge::Out, (450.0, 520.0));
    assert_eq!((part[0].x, part[part.len() - 1].x), (450.0, 520.0));
    assert_eq!(part.len(), 36);
    assert!((level(&part[0]) - level(&falling[25])).abs() < 1e-6);
    let unseen = fade_curve(&clip, tile, area, 100.0, FadeEdge::In, (450.0, 520.0));
    assert!(unseen.is_empty());
    // A clip without a fade has no curve.
    clip.fade_in = Time::ZERO;
    assert!(fade_curve(&clip, tile, area, 100.0, FadeEdge::In, whole).is_empty());
}

// ---- The clip menu --------------------------------------------------------

// §5.7.3: the rows of a clip's sound are in its Audio menu.
#[test]
fn the_clip_menu_fades_the_selected_clips_and_removes_their_fades() {
    let (mut bench, [shot, voice, bed]) = Bench::new();
    let steps = bench.steps();
    bench.audio_menu_at(bench.tile(voice).center());
    bench.click("Fade in 0.5 s");
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Change fades");
    assert_eq!(bench.fades(voice), (fades::QUICK, Time::ZERO));
    bench.audio_menu_at(bench.tile(voice).center());
    bench.click("Fade out 0.5 s");
    assert_eq!(bench.fades(voice), (fades::QUICK, fades::QUICK));
    // Asked for again it is there already: shown, and not to be chosen.
    bench.audio_menu_at(bench.tile(voice).center());
    assert!(bench.find("Fade out 0.5 s").is_some_and(|row| row.disabled));
    assert!(bench.find("Remove fades").is_some_and(|row| !row.disabled));
    bench.click("Remove fades");
    assert_eq!(bench.fades(voice), (Time::ZERO, Time::ZERO));
    assert_eq!(bench.steps(), steps + 3);
    bench.audio_menu_at(bench.tile(voice).center());
    assert!(bench.find("Remove fades").is_some_and(|row| row.disabled));
    bench.close_menus();

    // Several selected clips fade in one step: a picture clip's sound too.
    bench.selection.clips = vec![shot, voice, bed];
    bench.audio_menu_at(bench.tile(bed).center());
    bench.click("Fade out 0.5 s");
    assert_eq!(bench.steps(), steps + 4, "{:?}", bench.refused);
    for clip in [shot, voice, bed] {
        assert_eq!(bench.fades(clip), (Time::ZERO, fades::QUICK));
    }
    assert!(bench.editor.undo());
    for clip in [shot, voice, bed] {
        assert_eq!(bench.fades(clip), (Time::ZERO, Time::ZERO));
    }
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}

// §5.7.1 and P5-RR3: one Escape in the open Audio menu closes it alone,
// and the clip menu it opened from stays; the next closes that.
#[test]
fn an_escape_in_the_audio_menu_closes_it_and_leaves_the_clip_menu() {
    let (mut bench, [_, voice, _]) = Bench::new();
    bench.audio_menu_at(bench.tile(voice).center());
    assert!(
        bench.has("Fade in 0.5 s"),
        "FIXTURE: the Audio menu is open"
    );
    // The pointer leaves the row that opened it, as for the keyboard.
    bench.frame(vec![Event::PointerGone]);
    bench.escape();
    assert!(!bench.has("Fade in 0.5 s"), "the Audio menu closed");
    assert!(bench.has("Split at playhead"), "the clip menu stayed");
    bench.escape();
    assert!(!bench.has("Split at playhead"), "the next Escape closed it");
}

// §5.7.3: the rows of a clip's sound are in its Audio menu.
#[test]
fn the_clip_menu_offers_no_fade_of_a_title_or_of_sound_the_linked_clip_owns() {
    let (mut bench, [shot, ..]) = Bench::new();
    // A title has no sound: the rows are not there.
    let mut text = Track::new("Text 1", TrackKind::Text);
    let title = Clip::text("Title", seconds(1.0), seconds(4.0));
    let title_id = title.id;
    text.clips.push(title);
    bench.edit(Command::AddTrack { track: text });
    bench.audio_menu_at(bench.tile(title_id).center());
    assert!(bench.has("Split at playhead"), "the menu is open");
    assert!(!bench.has("Fade in 0.5 s") && !bench.has("Remove fades"));
    bench.close_menus();

    // A picture whose sound is on its linked audio clip: shown, and not to
    // be chosen.
    let detach = rules::detach_audio(bench.editor.project(), shot).unwrap();
    bench.edit(detach);
    let steps = bench.steps();
    bench.selection.clear();
    bench.audio_menu_at(bench.tile(shot).center());
    for (label, _) in fades::MENU {
        assert!(bench.find(label).is_some_and(|row| row.disabled), "{label}");
    }
    bench.click("Fade in 0.5 s");
    assert_eq!(bench.steps(), steps);
    assert_eq!(bench.fades(shot), (Time::ZERO, Time::ZERO));
}

// §5.7.3: the rows of a clip's sound are in its Audio menu.
#[test]
fn the_clip_menu_normalizes_the_selected_clips_that_are_heard() {
    const NORMALIZE: &str = "Normalize loudness";
    let (mut bench, [shot, voice, bed]) = Bench::new();
    // The selected clips, captured when the row is chosen; nothing is
    // edited until each is measured.
    bench.selection.clips = vec![voice, bed];
    let steps = bench.steps();
    bench.audio_menu_at(bench.tile(bed).center());
    bench.click(NORMALIZE);
    assert_eq!(bench.normalized, [vec![voice, bed]]);
    assert_eq!(bench.steps(), steps);

    // A title has no sound: no row.
    let mut text = Track::new("Text 1", TrackKind::Text);
    let title = Clip::text("Title", seconds(1.0), seconds(4.0));
    let title_id = title.id;
    text.clips.push(title);
    bench.edit(Command::AddTrack { track: text });
    bench.selection.clear();
    bench.audio_menu_at(bench.tile(title_id).center());
    assert!(bench.has("Split at playhead"), "the menu is open");
    assert!(!bench.has(NORMALIZE));
    bench.close_menus();

    // A picture whose sound is on its linked audio clip: shown, and not to
    // be chosen.
    let detach = rules::detach_audio(bench.editor.project(), shot).unwrap();
    bench.edit(detach);
    bench.selection.clear();
    bench.audio_menu_at(bench.tile(shot).center());
    assert!(bench.find(NORMALIZE).is_some_and(|row| row.disabled));
    bench.click(NORMALIZE);
    assert_eq!(bench.normalized.len(), 1);
}

// §5.7.3: the rows of a clip's sound are in its Audio menu.
#[test]
fn the_menu_of_an_audio_lane_clip_ducks_the_selected_clips() {
    use crate::duck::{DEPTH, LABEL};
    let (mut bench, [shot, voice, bed]) = Bench::new();
    // The selected clips at the default depth, captured when the row is
    // chosen; nothing is edited until the others' sound is read.
    bench.selection.only(bed);
    let steps = bench.steps();
    bench.audio_menu_at(bench.tile(bed).center());
    bench.click(LABEL);
    assert_eq!(bench.ducked, [(vec![bed], DEPTH)]);
    assert_eq!(bench.steps(), steps);

    // A clip on a video lane has no row, though its sound is heard.
    bench.selection.only(shot);
    bench.audio_menu_at(bench.tile(shot).center());
    assert!(bench.has("Normalize loudness"), "the menu is open");
    assert!(!bench.has(LABEL));
    bench.close_menus();

    // On a locked track the row is shown and not to be chosen.
    bench.lock(voice);
    bench.selection.only(voice);
    bench.audio_menu_at(bench.tile(voice).center());
    assert!(bench.find(LABEL).is_some_and(|row| row.disabled));
    bench.click(LABEL);
    assert_eq!(bench.ducked.len(), 1);

    // Beside a clip that is ducked, the locked one is passed over and still
    // asked for: a selected clip is no voice.
    bench.selection.clips = vec![voice, bed];
    bench.audio_menu_at(bench.tile(bed).center());
    bench.click(LABEL);
    assert_eq!(bench.ducked[1], (vec![voice, bed], DEPTH));
}

// §5.7.3: the rows of a clip's sound are in its Audio menu.
#[test]
fn the_clip_menu_saves_the_span_of_sound_the_clip_plays() {
    use crate::extract::{LABEL, Source};
    let (mut bench, [shot, voice, _]) = Bench::new();
    // The clip under the menu, captured when the row is chosen, on a video
    // lane and an audio lane alike; nothing is edited.
    let steps = bench.steps();
    for clip in [shot, voice] {
        bench.selection.only(clip);
        bench.audio_menu_at(bench.tile(clip).center());
        bench.click(LABEL);
    }
    assert_eq!(bench.saved, [Source::Clip(shot), Source::Clip(voice)]);
    assert_eq!(bench.steps(), steps);

    // A title has no sound: no row.
    let mut text = Track::new("Text 1", TrackKind::Text);
    let title = Clip::text("Title", seconds(1.0), seconds(4.0));
    let title_id = title.id;
    text.clips.push(title);
    bench.edit(Command::AddTrack { track: text });
    bench.selection.clear();
    bench.audio_menu_at(bench.tile(title_id).center());
    assert!(bench.has("Split at playhead"), "the menu is open");
    assert!(!bench.has(LABEL));
    bench.close_menus();

    // A picture whose sound its linked audio clip owns still plays that
    // sound's span: its file is offered as the audio clip's is.
    let detach = rules::detach_audio(bench.editor.project(), shot).unwrap();
    bench.edit(detach);
    bench.selection.clear();
    bench.audio_menu_at(bench.tile(shot).center());
    assert!(bench.find(LABEL).is_some_and(|row| !row.disabled));
}

// §5.7.3: the rows of a clip's sound are in its Audio menu.
#[test]
fn the_clip_menu_separates_a_clip_that_is_heard_once_a_separator_is_found() {
    use crate::stems::{LABEL, LOOKING, NEEDS_TOOL, Tool};
    let (mut bench, [shot, voice, _]) = Bench::new();
    // Room under the audio lane for the whole of its clip's menu.
    bench.size = vec2(900.0, 700.0);
    bench.frame(Vec::new());
    // Without a separator, or before the probe has answered, the row is
    // shown without being able to be chosen, and says why on hover.
    for (tool, words) in [(Tool::Missing, NEEDS_TOOL), (Tool::Looking, LOOKING)] {
        bench.separator = tool;
        bench.selection.only(voice);
        bench.audio_menu_at(bench.tile(voice).center());
        let row = bench.find(LABEL).expect("the row is shown");
        assert!(row.disabled);
        let at = row.rect.center();
        // Resting on it says why, once the pointer has been still a while.
        bench.frame(vec![Event::PointerMoved(at)]);
        let mut said = false;
        for _ in 0..90 {
            bench.frame(Vec::new());
            said |= (bench.seen.iter()).any(|seen| seen.name.replace('\n', " ") == words);
        }
        assert!(said, "{words} on hover");
        bench.close_menus();
    }
    assert!(bench.separated.is_empty());

    // Found: the clip under the menu, captured when the row is chosen, on a
    // video lane and an audio lane alike; nothing is edited yet.
    bench.separator = Tool::Found;
    let steps = bench.steps();
    for clip in [shot, voice] {
        bench.selection.only(clip);
        bench.audio_menu_at(bench.tile(clip).center());
        bench.click(LABEL);
    }
    assert_eq!(bench.separated, [shot, voice]);
    assert_eq!(bench.steps(), steps);

    // A picture whose sound its linked audio clip owns is not heard: the
    // row says where the sound is.
    let detach = rules::detach_audio(bench.editor.project(), shot).unwrap();
    bench.edit(detach);
    bench.selection.clear();
    bench.audio_menu_at(bench.tile(shot).center());
    assert!(bench.find(LABEL).is_some_and(|row| row.disabled));
    bench.close_menus();

    // A locked track keeps its clip from being turned down.
    bench.lock(voice);
    bench.audio_menu_at(bench.tile(voice).center());
    assert!(bench.find(LABEL).is_some_and(|row| row.disabled));
}
