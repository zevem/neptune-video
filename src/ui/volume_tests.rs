//! The volume line of a sound tile, driven as a person drives it.
//!
//! [`Bench`] paints the timeline in a real egui pass and hands it pointer
//! and key events. The line is found where the timeline's own arithmetic
//! puts it, a menu row by the name accessibility is given for it. The bench
//! ends a frame as the application does: an [`Action::Edit`] is dispatched,
//! and an Escape gives up what is held before the frame is drawn. Every edit
//! below is one `Editor::dispatch` accepted.
use super::*;
use crate::theme::{Accent, Appearance};
use editor_core::{Ease, Editor, Keyframe, MediaKind};
use eframe::egui::{Event, Key, Modifiers, PointerButton, accesskit::Role};
use std::sync::Arc;

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

/// The volume the held line would give its clip.
fn held_gain(bench: &Bench) -> f32 {
    match bench.timeline.drag {
        Some(Drag::Volume { gain, .. }) => gain,
        _ => panic!("no volume line is held"),
    }
}

fn key(at: f64, value: f32) -> Keyframe {
    Keyframe {
        at: seconds(at),
        value,
        ease: Ease::Linear,
    }
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
    size: Vec2,
    time: f64,
    seen: Vec<Seen>,
    /// Every line of text the last frame painted, and where.
    texts: Vec<(String, Rect)>,
    /// The bounds of each mesh of bare colour the last frame drew. No peaks
    /// are held here, so no waveform is one: each is a fade.
    shaded: Vec<Rect>,
    refused: Vec<String>,
    said: Vec<String>,
}

impl Bench {
    /// Two rows, each with a clip from one second to five: `shot.mp4`, a
    /// video with sound, on a video track and `voice.wav` on an audio
    /// track. Nothing is selected and the playhead is at the start.
    fn new() -> (Self, [ClipId; 2]) {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut project = Project::new("Volume");
        project.tracks.clear();
        let mut ids = Vec::new();
        for (name, media, track, kind) in [
            ("shot.mp4", MediaKind::Video, "Video 1", TrackKind::Video),
            ("voice.wav", MediaKind::Audio, "Audio 1", TrackKind::Audio),
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
            size: vec2(900.0, 300.0),
            time: 0.0,
            seen: Vec::new(),
            texts: Vec::new(),
            shaded: Vec::new(),
            refused: Vec::new(),
            said: Vec::new(),
        };
        bench.frame(Vec::new());
        (bench, [ids[0], ids[1]])
    }

    /// The bench with `voice.wav` selected, so that its line is taken hold
    /// of.
    fn selected() -> (Self, ClipId) {
        let (mut bench, [_, voice]) = Self::new();
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

    fn volume_keys(&mut self, clip: ClipId, keys: Vec<Keyframe>) {
        self.edit(Command::SetKeyframes {
            clip_id: clip,
            target: None,
            key: ParamKey::Gain,
            keys,
        });
    }

    fn clip(&self, id: ClipId) -> &Clip {
        self.editor.project().clip(id).expect("the clip").1
    }

    fn gain(&self, id: ClipId) -> f32 {
        self.clip(id).gain
    }

    /// The keys of the clip's volume, as seconds from its start and values.
    fn keyed(&self, id: ClipId) -> Vec<(f64, f32)> {
        (self.clip(id).animation.iter())
            .filter(|track| track.target.is_none() && track.key == ParamKey::Gain)
            .flat_map(|track| &track.keys)
            .map(|key| (key.at.as_seconds(), key.value))
            .collect()
    }

    fn steps(&self) -> usize {
        self.editor.history().count()
    }

    fn last(&self) -> String {
        self.editor.undo_label().unwrap_or_default().to_owned()
    }

    /// One frame of the application with these events in it.
    fn frame(&mut self, events: Vec<Event>) {
        fn painted(shape: &egui::Shape, bench: &mut Bench) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| painted(shape, bench)),
                egui::Shape::Text(text) => bench.texts.push((
                    text.galley.text().to_owned(),
                    Rect::from_min_size(text.pos, text.galley.size()),
                )),
                egui::Shape::Mesh(mesh) if mesh.texture_id == egui::TextureId::default() => {
                    bench.shaded.push(mesh.calc_bounds());
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
        self.texts.clear();
        self.shaded.clear();
        for clipped in &output.shapes {
            painted(&clipped.shape, self);
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

    /// Where a time of the project is in the lanes.
    fn x(&self, at: f64) -> f32 {
        HEAD_WIDTH + at as f32 * self.timeline.zoom - self.timeline.scroll.x
    }

    /// The tile of the clip, as the lanes lay it out while no keyframe lane
    /// is open above it.
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

    /// The point of the clip's volume line at this time of the project.
    fn line(&self, clip: ClipId, at: f64) -> Pos2 {
        let tile = self.tile(clip);
        let x = self.x(at);
        let y = volume_y(
            self.clip(clip),
            tile,
            sound_area(tile),
            self.timeline.zoom,
            x,
        );
        pos2(x, y)
    }

    fn button(pos: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        }
    }

    fn move_to(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
    }

    fn press(&mut self, pos: Pos2) {
        self.move_to(pos);
        let none = Modifiers::NONE;
        self.frame(vec![Self::button(pos, PointerButton::Primary, true, none)]);
    }

    fn release(&mut self, pos: Pos2) {
        let none = Modifiers::NONE;
        self.frame(vec![Self::button(pos, PointerButton::Primary, false, none)]);
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

    /// Takes hold of the clip's line three seconds into the project, lifts
    /// it by `rise` points and lets it go.
    fn lift(&mut self, clip: ClipId, rise: f32) {
        let from = self.line(clip, 3.0);
        let to = from - vec2(0.0, rise);
        self.carry(from, to);
        self.release(to);
    }

    /// A click with Alt held.
    fn alt_click(&mut self, pos: Pos2) {
        let alt = Modifiers::ALT;
        self.move_to(pos);
        self.frame(vec![
            Event::ModifiersChanged(alt),
            Self::button(pos, PointerButton::Primary, true, alt),
        ]);
        self.frame(vec![Self::button(pos, PointerButton::Primary, false, alt)]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    fn click(&mut self, name: &str) {
        self.frame(Vec::new());
        let pos = match self.find(name) {
            Some(seen) => seen.rect.center(),
            None => {
                let names: Vec<&str> = self.seen.iter().map(|seen| seen.name.as_str()).collect();
                panic!("FIXTURE: no control is named \"{name}\" among {names:?}")
            }
        };
        self.press(pos);
        self.release(pos);
    }

    fn secondary_click_at(&mut self, pos: Pos2) {
        let none = Modifiers::NONE;
        self.move_to(pos);
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

// ---- Where the line stands -------------------------------------------------

#[test]
fn the_whole_volume_stands_at_two_thirds_and_twice_it_at_the_top() {
    assert_eq!(volume_level(0.0), 0.0);
    assert!((volume_level(0.5) - 1.0 / 3.0).abs() < 1e-6);
    assert!((volume_level(1.0) - 2.0 / 3.0).abs() < 1e-6);
    assert!((volume_level(1.5) - 5.0 / 6.0).abs() < 1e-6);
    assert_eq!(volume_level(2.0), 1.0);
    // More than the line shows is drawn at the top, and less than nothing
    // at the bottom.
    assert_eq!(volume_level(8.0), 1.0);
    assert_eq!(volume_level(-1.0), 0.0);
    for percent in 0..=200 {
        let gain = percent as f32 / 100.0;
        assert!((volume_at_level(volume_level(gain)) - gain).abs() < 1e-5);
    }
    assert_eq!(volume_bounds(), (0.0, 8.0));
    // A drag lands on a whole percent and never leaves what a clip takes,
    // however far the pointer goes.
    assert_eq!(volume_dragged(1.0, 2.0 / 3.0, 4.0, 24.0), 1.5);
    assert_eq!(volume_dragged(1.0, 2.0 / 3.0, 1.0, 24.0), 1.13);
    for rise in [-1e9, -500.0, -24.0, 0.0, 24.0, 500.0, 1e9, f32::INFINITY] {
        let gain = volume_dragged(1.0, 2.0 / 3.0, rise, 24.0);
        assert!((0.0..=8.0).contains(&gain), "{rise}: {gain}");
    }
    assert_eq!(volume_dragged(1.0, 1.0, 1e9, 24.0), 2.0);
    assert_eq!(volume_dragged(1.0, 0.0, -1e9, 24.0), 0.0);
    // A line standing where the clip's volume is drawn keeps that volume,
    // finer or louder than the line shows: only a line that has left its
    // place gives the clip a whole percent.
    for stored in [8.0, 2.5, 2.004] {
        for rise in [0.0, 0.01, 6.0, 1e9] {
            assert_eq!(volume_dragged(stored, 1.0, rise, 24.0), stored, "{rise}");
        }
        assert_eq!(volume_dragged(stored, 1.0, -4.0, 24.0), 1.5);
    }
    let fine = 1.0 / 3.0;
    assert_eq!(volume_dragged(fine, volume_level(fine), 0.0, 24.0), fine);
    assert_eq!(volume_dragged(fine, volume_level(fine), 4.0, 24.0), 0.58);
}

#[test]
fn the_line_is_level_without_keys_and_the_tracks_own_evaluation_with_them() {
    // A hundred points to the second: a four second clip from 200 to 600.
    let asset = Asset::new("/media/voice.wav".into(), MediaKind::Audio, seconds(10.0));
    let mut clip = Clip::from_asset(&asset, seconds(1.0));
    clip.duration = seconds(4.0);
    let tile = Rect::from_min_max(pos2(200.0, 100.0), pos2(600.0, 144.0));
    let area = sound_area(tile);
    assert_eq!(area.height(), 24.0);
    let whole = (0.0, 2000.0);
    let level = |point: &Pos2| (area.bottom() - point.y) / area.height();

    // No keys: two points, at the clip's volume.
    let flat = volume_line(&clip, tile, area, 100.0, whole);
    assert_eq!(flat.len(), 2);
    assert_eq!((flat[0].x, flat[1].x), (200.0, 600.0));
    assert!(
        flat.iter()
            .all(|point| (level(point) - 2.0 / 3.0).abs() < 1e-6)
    );
    clip.gain = 0.5;
    let half = volume_line(&clip, tile, area, 100.0, whole);
    assert!(
        half.iter()
            .all(|point| (level(point) - 1.0 / 3.0).abs() < 1e-6)
    );
    // Five times the whole is drawn at the top, and the part in view only.
    clip.gain = 5.0;
    let pinned = volume_line(&clip, tile, area, 100.0, (450.0, 520.0));
    assert_eq!((pinned[0].x, pinned[1].x), (450.0, 520.0));
    assert!(pinned.iter().all(|point| point.y == area.top()));
    assert!(volume_line(&clip, tile, area, 100.0, (700.0, 900.0)).is_empty());

    // Keys: whole for a second, down to nothing two seconds on, held there,
    // then a step up to twice the whole a second before the end.
    clip.gain = 1.0;
    clip.animation.push(editor_core::AnimTrack {
        target: None,
        key: ParamKey::Gain,
        keys: vec![
            key(1.0, 1.0),
            Keyframe {
                at: seconds(2.0),
                value: 0.0,
                ease: Ease::Hold,
            },
            key(3.0, 2.0),
        ],
    });
    let bent = volume_line(&clip, tile, area, 100.0, whole);
    // A point every two of width, and each key where it is.
    assert!(bent.windows(2).all(|pair| pair[1].x - pair[0].x <= 2.0));
    assert!(bent.windows(2).all(|pair| pair[1].x >= pair[0].x));
    for at in [300.0, 400.0, 500.0] {
        assert!(bent.iter().any(|point| point.x == at), "a point at {at}");
    }
    for point in &bent {
        let local = tile_time(tile, 100.0, point.x);
        let now = volume_level(gain_at(&clip, clip.start + local));
        // The point that ends a held stretch is drawn at the held value.
        let before = volume_level(gain_at(&clip, clip.start + local - Time(1)));
        let drawn = level(point);
        assert!(
            (drawn - now).abs() < 1e-4 || (drawn - before).abs() < 1e-4,
            "{point:?}: {drawn} is neither {now} nor {before}"
        );
    }
    let at = |x: f32| -> Vec<f32> {
        (bent.iter().filter(|point| point.x == x))
            .map(level)
            .collect()
    };
    assert!((at(250.0)[0] - 2.0 / 3.0).abs() < 1e-6, "held before a key");
    assert!((at(350.0)[0] - 1.0 / 3.0).abs() < 1e-4, "half way down");
    assert!(at(450.0)[0].abs() < 1e-6, "held at nothing");
    // The step: nothing and twice the whole at one place.
    let step = at(500.0);
    assert!(step.iter().any(|level| level.abs() < 1e-6), "{step:?}");
    assert!(step.iter().any(|level| (level - 1.0).abs() < 1e-6));
    assert!((at(560.0)[0] - 1.0).abs() < 1e-6, "held after the last");
    // Only the part in view is worked out.
    let part = volume_line(&clip, tile, area, 100.0, (320.0, 380.0));
    assert_eq!((part[0].x, part[part.len() - 1].x), (320.0, 380.0));
    assert!(part.len() <= 33, "{}", part.len());
}

// ---- The drag --------------------------------------------------------------

#[test]
fn a_dragged_line_previews_the_volume_and_is_one_step_on_release() {
    let (mut bench, voice) = Bench::selected();
    let steps = bench.steps();
    let tile = bench.tile(voice);
    let area = sound_area(tile);
    let from = bench.line(voice, 3.0);
    assert!((from.y - (area.bottom() - area.height() * 2.0 / 3.0)).abs() < 0.01);
    // Four points up of twenty-four: half way from the whole to twice it.
    let to = from - vec2(0.0, 4.0);
    bench.carry(from, to);
    bench.frame(Vec::new());
    // Held, nothing is edited yet.
    assert!(matches!(
        bench.timeline.drag,
        Some(Drag::Volume { gain, moved: true, .. }) if gain == 1.5
    ));
    assert_eq!(bench.steps(), steps);
    assert_eq!(bench.gain(voice), 1.0);

    bench.release(to);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Change clip volume");
    assert_eq!(bench.gain(voice), 1.5);
    assert!(bench.clip(voice).animation.is_empty(), "no key is written");
    // The line is now where it was let go.
    assert!((bench.line(voice, 3.0).y - to.y).abs() < 0.01);

    // Down by half the sound's height: from five sixths to a third.
    bench.lift(voice, -12.0);
    assert_eq!(bench.steps(), steps + 2, "{:?}", bench.refused);
    assert_eq!(bench.gain(voice), 0.5);

    // Each drag undoes by itself.
    assert!(bench.editor.undo());
    assert_eq!(bench.gain(voice), 1.5);
    assert!(bench.editor.undo());
    assert_eq!(bench.gain(voice), 1.0);
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}

#[test]
fn the_held_percent_is_said_beside_the_point_taken_and_clear_of_the_fades() {
    let (mut bench, voice) = Bench::selected();
    // A second and a half of fade in and a second of fade out on a clip of
    // four: the line is taken over each fade and between them.
    bench.edit(Command::SetFades {
        clip_id: voice,
        fade_in: seconds(1.5),
        fade_out: seconds(1.0),
    });
    let steps = bench.steps();
    let tile = bench.tile(voice);
    let area = sound_area(tile);
    // Holds the line at `at` four points up, half way from the whole to
    // twice it, and says where its percent is drawn; then gives it up.
    let held = |bench: &mut Bench, at: f64| -> (Rect, Vec<(String, Rect)>) {
        let from = bench.line(voice, at);
        bench.carry(from, from - vec2(0.0, 4.0));
        bench.frame(Vec::new());
        assert!(
            matches!(bench.timeline.drag, Some(Drag::Volume { gain, .. }) if gain == 1.5),
            "{at}: the line is held at 150%, not {:?}",
            bench.timeline.drag
        );
        let said: Vec<Rect> = (bench.texts.iter())
            .filter(|(text, _)| text == "150%")
            .map(|(_, rect)| *rect)
            .collect();
        let [readout] = said.as_slice() else {
            panic!("{at}: the percent is said once: {:?}", bench.texts);
        };
        let readout = *readout;
        // Inside the tile, clear of both fades' curves and handles.
        assert!(tile.shrink(1.0).contains_rect(readout), "{at}: {readout:?}");
        assert_eq!(bench.shaded.len(), 2, "{at}: both fades are drawn");
        for curve in &bench.shaded {
            assert!(!curve.intersects(readout), "{at}: {readout:?} on {curve:?}");
        }
        for handle in ["Fade in of voice.wav", "Fade out of voice.wav"] {
            let handle = bench.find(handle).expect("the handles are shown").rect;
            assert!(
                !handle.intersects(readout),
                "{at}: {readout:?} on {handle:?}"
            );
        }
        let texts = bench.texts.clone();
        bench.escape();
        bench.release(from);
        (readout, texts)
    };

    // Between the fades: just after the point taken, in the tile's sound.
    let (between, _) = held(&mut bench, 3.0);
    assert!((between.left() - (bench.x(3.0) + READOUT_GAP)).abs() < 0.5);
    assert!(area.contains_rect(between), "{between:?}");
    // Over the fade in: just past it, not at the clip's left edge.
    let (over_in, _) = held(&mut bench, 1.4);
    let fade_in_ends = bench.x(2.5);
    assert!(over_in.left() > fade_in_ends, "{over_in:?}");
    assert!(
        over_in.left() < fade_in_ends + 3.0 * READOUT_GAP,
        "{over_in:?}"
    );
    // Over the fade out: just before it.
    let (over_out, _) = held(&mut bench, 4.6);
    let fade_out_starts = bench.x(4.0);
    assert!(over_out.right() < fade_out_starts, "{over_out:?}");
    assert!(over_out.right() > fade_out_starts - 3.0 * READOUT_GAP);

    // Fades that leave no room between them: the name's row says it, and
    // the name gives way.
    bench.edit(Command::SetFades {
        clip_id: voice,
        fade_in: seconds(2.0),
        fade_out: seconds(2.0),
    });
    // Taken away from where the fades meet, which is where their handles
    // are.
    let (named, texts) = held(&mut bench, 2.2);
    assert!(
        named.bottom() <= area.top(),
        "{named:?} is in the name's row"
    );
    let name = (texts.iter())
        .find(|(text, _)| text.starts_with('v'))
        .map(|(_, rect)| *rect)
        .expect("the name is drawn");
    assert!(name.right() < named.left(), "{name:?} runs under {named:?}");

    // Nothing was edited by holding the line and giving it up.
    assert_eq!(bench.steps(), steps + 1);
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}

#[test]
fn on_a_tile_too_narrow_for_its_name_beside_it_the_held_percent_is_said_alone() {
    let (mut bench, voice) = Bench::selected();
    let mut said_in_row = 0;
    // From the narrowest tile whose line is taken hold of to one whose name
    // has room for a few letters beside the percent.
    for width in [FADE_TILE, 44.0, 48.0, 56.0, 72.0] {
        bench.timeline.zoom = width / 4.0;
        bench.frame(Vec::new());
        let tile = bench.tile(voice);
        let from = bench.line(voice, 3.0);
        bench.carry(from, from - vec2(0.0, 4.0));
        bench.frame(Vec::new());
        assert!(
            matches!(bench.timeline.drag, Some(Drag::Volume { gain, .. }) if gain == 1.5),
            "{width}: the line is held at 150%, not {:?}",
            bench.timeline.drag
        );
        let inner = tile.shrink(1.0);
        let on_tile: Vec<&(String, Rect)> = (bench.texts.iter())
            .filter(|(_, rect)| inner.intersects(*rect))
            .collect();
        if let Some((_, said)) = on_tile.iter().find(|(text, _)| text == "150%") {
            if said.bottom() <= sound_area(tile).top() {
                said_in_row += 1;
            }
            // Not a glyph of the name runs under the percent or its pill.
            let pill = said.expand2(vec2(4.0, 1.0));
            for (text, rect) in &on_tile {
                if text != "150%" {
                    assert!(
                        !rect.intersects(pill),
                        "{width}: \"{text}\" at {rect:?} runs under the percent at {said:?}"
                    );
                }
            }
        }
        bench.escape();
        bench.release(from);
    }
    assert!(
        said_in_row >= 2,
        "the name's row said it {said_in_row} times"
    );
}

#[test]
fn a_dragged_volume_stops_at_nothing_and_at_the_top_of_the_line() {
    let (mut bench, voice) = Bench::selected();
    let steps = bench.steps();
    // Far under the tile: nothing, never less.
    bench.lift(voice, -400.0);
    assert_eq!(bench.gain(voice), 0.0);
    // Far over it: twice the whole, the top of the line.
    bench.lift(voice, 400.0);
    assert_eq!(bench.gain(voice), 2.0);
    assert_eq!(bench.steps(), steps + 2, "{:?}", bench.refused);
    // Already at the top, a drag further up changes nothing and is no step.
    bench.lift(voice, 400.0);
    assert_eq!(bench.steps(), steps + 2);

    // A volume the line cannot show is drawn at the top and comes down
    // from there: by four points of twenty-four to one and a half.
    bench.edit(Command::SetGain {
        clip_id: voice,
        gain: 8.0,
    });
    let before = bench.steps();
    assert_eq!(
        bench.line(voice, 3.0).y,
        sound_area(bench.tile(voice)).top()
    );
    bench.lift(voice, -4.0);
    assert_eq!(bench.gain(voice), 1.5);
    assert_eq!(bench.steps(), before + 1);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_line_left_where_it_was_taken_keeps_a_volume_it_cannot_show() {
    let (mut bench, voice) = Bench::selected();
    // Louder than the top of the line, and finer than a whole percent: both
    // set where a line does not reach, as the inspector and an agent do.
    for gain in [8.0, 1.0 / 3.0] {
        bench.edit(Command::SetGain {
            clip_id: voice,
            gain,
        });
        let steps = bench.steps();
        let from = bench.line(voice, 3.0);
        // Carried down and back to where it was pressed.
        bench.carry(from, from + vec2(0.0, 10.0));
        assert_ne!(held_gain(&bench), gain);
        bench.move_to(from);
        assert_eq!(held_gain(&bench), gain);
        bench.release(from);
        assert_eq!(bench.gain(voice), gain);
        assert_eq!(bench.steps(), steps, "{gain}: {:?}", bench.refused);
    }
    // Lifting a line already drawn at the top does not turn the clip down.
    bench.edit(Command::SetGain {
        clip_id: voice,
        gain: 8.0,
    });
    let steps = bench.steps();
    bench.lift(voice, 12.0);
    assert_eq!(bench.gain(voice), 8.0);
    assert_eq!(bench.steps(), steps, "{:?}", bench.refused);
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}

#[test]
fn a_press_on_the_line_that_does_not_travel_changes_nothing() {
    let (mut bench, voice) = Bench::selected();
    let (steps, before) = (bench.steps(), bench.editor.project().clone());
    let on = bench.line(voice, 3.0);
    bench.press(on);
    assert!(matches!(
        bench.timeline.drag,
        Some(Drag::Volume { moved: false, .. })
    ));
    // Two points are not a drag.
    bench.move_to(on - vec2(0.0, 2.0));
    bench.release(on - vec2(0.0, 2.0));
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
    assert_eq!(bench.selection.clips, [voice]);
}

#[test]
fn escape_gives_up_a_held_line_and_leaves_the_project_unchanged() {
    let (mut bench, voice) = Bench::selected();
    bench.lift(voice, -4.0);
    let (steps, before) = (bench.steps(), bench.editor.project().clone());
    let from = bench.line(voice, 3.0);
    let to = from - vec2(0.0, 10.0);
    bench.carry(from, to);
    assert!(matches!(bench.timeline.drag, Some(Drag::Volume { .. })));
    bench.escape();
    assert!(bench.timeline.drag.is_none());
    // The pointer is still down and still moving: nothing follows it.
    bench.move_to(to - vec2(0.0, 6.0));
    bench.release(to - vec2(0.0, 6.0));
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
    // The Escape let go of the line, not of the clip.
    assert_eq!(bench.selection.clips, [voice]);
    // The next drag is its own, and is kept.
    bench.lift(voice, 4.0);
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
}

#[test]
fn the_line_of_a_keyed_clip_is_not_dragged_as_a_whole() {
    let (mut bench, voice) = Bench::selected();
    bench.volume_keys(voice, vec![key(0.0, 1.0), key(4.0, 0.5)]);
    let (steps, before) = (bench.steps(), bench.editor.project().clone());
    // Half way along the line stands at three quarters of the whole.
    let from = bench.line(voice, 3.0);
    let area = sound_area(bench.tile(voice));
    assert!((from.y - (area.bottom() - area.height() * 0.5)).abs() < 0.01);
    let to = from - vec2(0.0, 5.0);
    bench.carry(from, to);
    // The press was the clip's: it is held to be moved, and it has not
    // moved along the timeline.
    assert!(matches!(bench.timeline.drag, Some(Drag::Move { .. })));
    bench.release(to);
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn the_line_is_taken_hold_of_only_on_a_selected_sound_tile_that_takes_an_edit() {
    let (mut bench, [shot, voice]) = Bench::new();
    let before = bench.editor.project().clone();
    let steps = bench.steps();

    // Not selected: the press selects the clip and holds it to be moved.
    let from = bench.line(voice, 3.0);
    bench.carry(from, from - vec2(0.0, 5.0));
    assert!(matches!(bench.timeline.drag, Some(Drag::Move { .. })));
    bench.release(from - vec2(0.0, 5.0));
    assert_eq!(bench.selection.clips, [voice]);
    assert_eq!(*bench.editor.project(), before);

    // In the grip that trims the clip, the press trims it.
    let edge = pos2(bench.tile(voice).left() + 2.0, from.y);
    bench.press(edge);
    assert!(matches!(bench.timeline.drag, Some(Drag::Trim { .. })));
    bench.release(edge);
    assert_eq!(*bench.editor.project(), before);

    // A picture tile has no line, of a video with sound either.
    bench.selection.only(shot);
    bench.frame(Vec::new());
    let tile = bench.tile(shot);
    let area = sound_area(tile);
    let on = pos2(bench.x(3.0), area.bottom() - area.height() * 2.0 / 3.0);
    bench.carry(on, on - vec2(0.0, 5.0));
    assert!(matches!(bench.timeline.drag, Some(Drag::Move { .. })));
    bench.release(on - vec2(0.0, 5.0));
    assert_eq!(bench.gain(shot), 1.0);

    // With more clips selected and a modifier held the press is the
    // selection's.
    bench.selection.clips = vec![shot, voice];
    bench.frame(Vec::new());
    bench.move_to(from);
    let shift = Modifiers::SHIFT;
    bench.frame(vec![
        Event::ModifiersChanged(shift),
        Bench::button(from, PointerButton::Primary, true, shift),
    ]);
    assert!(!matches!(bench.timeline.drag, Some(Drag::Volume { .. })));
    bench.frame(vec![
        Bench::button(from, PointerButton::Primary, false, shift),
        Event::ModifiersChanged(Modifiers::NONE),
    ]);

    // A locked track: nothing is taken hold of, and nothing changes.
    bench.selection.only(voice);
    let track_id = bench.editor.project().clip(voice).expect("the clip").0.id;
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    });
    let locked = bench.steps();
    assert_eq!(locked, steps + 1);
    bench.carry(from, from - vec2(0.0, 5.0));
    assert!(bench.timeline.drag.is_none());
    bench.release(from - vec2(0.0, 5.0));
    bench.alt_click(from);
    assert_eq!(bench.steps(), locked);
    assert_eq!(bench.gain(voice), 1.0);
    assert!(bench.keyed(voice).is_empty());
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_track_locked_while_the_line_is_held_has_the_release_refused() {
    let (mut bench, voice) = Bench::selected();
    let from = bench.line(voice, 3.0);
    let to = from - vec2(0.0, 4.0);
    bench.carry(from, to);
    let track_id = bench.editor.project().clip(voice).expect("the clip").0.id;
    bench.edit(Command::SetTrackState {
        track_id,
        locked: Some(true),
        hidden: None,
        muted: None,
        name: None,
        gain: None,
        solo: None,
    });
    let (steps, before) = (bench.steps(), bench.editor.project().clone());
    bench.release(to);
    assert_eq!(bench.refused.len(), 1, "the project refuses it");
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
}

#[test]
fn a_line_keyed_while_it_is_held_is_not_written_as_a_volume() {
    let (mut bench, voice) = Bench::selected();
    let from = bench.line(voice, 3.0);
    let to = from - vec2(0.0, 4.0);
    bench.carry(from, to);
    // An agent, or an undo, keys the volume under the held line.
    bench.volume_keys(voice, vec![key(0.0, 1.0), key(4.0, 0.5)]);
    let (steps, before) = (bench.steps(), bench.editor.project().clone());
    bench.release(to);
    assert_eq!(bench.steps(), steps);
    assert_eq!(*bench.editor.project(), before);
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

// ---- Alt-click -------------------------------------------------------------

#[test]
fn alt_click_keys_the_volume_at_the_frame_under_the_pointer_and_opens_the_lane() {
    let (mut bench, [_, voice]) = Bench::new();
    bench.edit(Command::SetGain {
        clip_id: voice,
        gain: 0.5,
    });
    let steps = bench.steps();
    let rate = bench.editor.project().settings.frame_rate;
    // Between two frames: the nearer one. The clip was not selected.
    let loose = 2.02;
    let frame = rules::quantize(seconds(loose), rate);
    assert_ne!(frame, seconds(loose));
    bench.alt_click(bench.line(voice, loose));
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Key volume");
    // One key, at that frame from the clip's start, at the volume it had.
    let local = (frame - seconds(1.0)).as_seconds();
    assert_eq!(bench.keyed(voice), [(local, 0.5)]);
    assert_eq!(bench.gain(voice), 0.5);
    assert_eq!(bench.selection.clips, [voice]);
    // The lane the key is in is open, with its row and the key in it.
    assert_eq!(bench.timeline.keys.lane_clip(), Some(voice));
    bench.frame(Vec::new());
    assert!(bench.has("Select Volume keyframes"));
    assert!(
        bench
            .seen
            .iter()
            .any(|seen| seen.name.starts_with("Volume keyframe at"))
    );
    assert!(bench.timeline.drag.is_none());

    // A second one, on the line the first one left level: one more key and
    // nothing heard changes.
    bench.alt_click(bench.line(voice, 4.0));
    assert_eq!(bench.steps(), steps + 2, "{:?}", bench.refused);
    assert_eq!(bench.keyed(voice), [(local, 0.5), (3.0, 0.5)]);
    // Where a key already is, none is written.
    bench.alt_click(bench.line(voice, 4.0));
    assert_eq!(bench.steps(), steps + 2);

    // On a line the keys bend, the key has the value the line has there.
    bench.volume_keys(voice, vec![key(0.0, 0.0), key(4.0, 2.0)]);
    let before = bench.steps();
    bench.alt_click(bench.line(voice, 3.0));
    assert_eq!(bench.steps(), before + 1, "{:?}", bench.refused);
    assert_eq!(bench.keyed(voice), [(0.0, 0.0), (2.0, 1.0), (4.0, 2.0)]);

    // Each undoes by itself.
    assert!(bench.editor.undo());
    assert_eq!(bench.keyed(voice), [(0.0, 0.0), (4.0, 2.0)]);

    // A key in a stretch that is held, or eased, leaves what is heard as it
    // was on both sides of it: the stretch is divided, not straightened.
    let ease_in = Ease::Bezier {
        x1: 0.42,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };
    let back = Ease::Bezier {
        x1: 0.34,
        y1: 1.56,
        x2: 0.64,
        y2: 1.0,
    };
    for ease in [Ease::Hold, ease_in, back] {
        let first = Keyframe {
            ease,
            ..key(1.0, 0.25)
        };
        bench.volume_keys(voice, vec![first, key(3.0, 1.75)]);
        let (was, before) = (bench.clip(voice).clone(), bench.steps());
        bench.alt_click(bench.line(voice, 3.5));
        assert_eq!(bench.steps(), before + 1, "{ease:?}: {:?}", bench.refused);
        assert_eq!(bench.last(), "Key volume");
        let keyed = bench.keyed(voice);
        let there = gain_at(&was, seconds(3.5));
        assert!(keyed.contains(&(2.5, there)), "{ease:?}: {keyed:?}");
        for step in 0..=400 {
            let at = seconds(1.0) + Time(step * 10_000);
            let (old, new) = (gain_at(&was, at), gain_at(bench.clip(voice), at));
            assert!(
                (old - new).abs() <= 2e-4,
                "{ease:?} {at:?}: {old} became {new}"
            );
        }
        // One step back and the stretch is whole again.
        assert!(bench.editor.undo());
        assert_eq!(*bench.clip(voice), was);
    }
    // Away from the line, Alt does nothing of its own.
    let tile = bench.tile(voice);
    let steps = bench.steps();
    bench.alt_click(pos2(bench.x(3.0), tile.top() + 6.0));
    assert_eq!(bench.steps(), steps);
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}

// ---- The clip menu ---------------------------------------------------------

// §5.7.3: the rows of a clip's sound are in its Audio menu.
#[test]
fn the_clip_menu_keys_the_volume_at_the_playhead_and_opens_the_lane() {
    let (mut bench, [shot, voice]) = Bench::new();
    let steps = bench.steps();
    bench.playhead = seconds(2.5);
    bench.audio_menu_at(bench.tile(voice).center());
    bench.click("Key volume");
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.last(), "Key volume");
    assert_eq!(bench.keyed(voice), [(1.5, 1.0)]);
    assert_eq!(bench.timeline.keys.lane_clip(), Some(voice));
    bench.frame(Vec::new());
    assert!(bench.has("Select Volume keyframes"));
    // Asked for again there, it is keyed already: shown, not to be chosen.
    bench.timeline.keys.set_lane(None);
    bench.frame(Vec::new());
    bench.audio_menu_at(bench.tile(voice).center());
    assert!(bench.find("Key volume").is_some_and(|row| row.disabled));
    bench.close_menus();
    // Nor with the playhead off the clip.
    bench.playhead = seconds(7.0);
    bench.audio_menu_at(bench.tile(voice).center());
    assert!(bench.find("Key volume").is_some_and(|row| row.disabled));
    bench.close_menus();
    assert!(bench.editor.undo());
    assert!(bench.keyed(voice).is_empty());

    // A picture clip with sound has its volume keyed too.
    bench.playhead = seconds(1.0);
    bench.audio_menu_at(bench.tile(shot).center());
    bench.click("Key volume");
    assert_eq!(bench.keyed(shot), [(0.0, 1.0)]);
    assert_eq!(bench.timeline.keys.lane_clip(), Some(shot));
    assert!(bench.editor.undo());
    bench.timeline.keys.set_lane(None);

    // One whose sound is on its linked audio clip: shown, not to be chosen.
    let detach = rules::detach_audio(bench.editor.project(), shot).unwrap();
    bench.edit(detach);
    let steps = bench.steps();
    bench.selection.clear();
    bench.audio_menu_at(bench.tile(shot).center());
    assert!(bench.find("Key volume").is_some_and(|row| row.disabled));
    bench.click("Key volume");
    assert_eq!(bench.steps(), steps);
    bench.close_menus();

    // A title has no sound: the row is not there.
    let mut text = Track::new("Text 1", TrackKind::Text);
    let title = Clip::text("Title", seconds(1.0), seconds(4.0));
    let title_id = title.id;
    text.clips.push(title);
    bench.edit(Command::AddTrack { track: text });
    bench.audio_menu_at(bench.tile(title_id).center());
    assert!(bench.has("Split at playhead"), "the menu is open");
    assert!(!bench.has("Key volume"));
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}
