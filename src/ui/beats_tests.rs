//! The beats of music on its tiles, and edits that land on them.
//!
//! [`Bench`] paints the timeline in a real egui pass, with the peaks and
//! beats it holds kept from frame to frame as the application keeps them,
//! and hands it pointer events. A grid is held as the lane would have it
//! held: shown, wanted by its tile, asked for and answered. The bench ends a
//! frame as the application does: an [`Action::Edit`] is dispatched, and
//! every edit below is one `Editor::dispatch` accepted.
use super::*;
use crate::theme::{Accent, Appearance};
use editor_core::{Editor, MediaKind};
use editor_media::SourceIdentity;
use eframe::egui::{Event, Modifiers, PointerButton, accesskit::Role};
use std::sync::Arc;

fn seconds(value: f64) -> Time {
    Time::from_seconds(value)
}

/// A beat every half second of source, from a quarter of a second.
fn grid(count: u32) -> BeatGrid {
    BeatGrid {
        bpm: 120.0,
        beats: (0..count).map(|index| 250 + index * 500).collect(),
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
    peaks: HeldPeaks,
    playhead: Time,
    size: Vec2,
    time: f64,
    seen: Vec<Seen>,
    /// The ticks each mesh of bare colour the last frame drew would be, at
    /// four corners a tick: no peaks are held and no clip fades, so every
    /// such mesh is a tile's beats.
    ticks: Vec<usize>,
    refused: Vec<String>,
    said: Vec<String>,
}

impl Bench {
    /// `song.wav` from half a second for six seconds on "Music", its beats
    /// at 0.75 s and every half second after on the timeline, and
    /// `voice.wav` from seven seconds for one on "Voice". Snapping is on,
    /// nothing is selected and the playhead is at the start. The song's
    /// beats are neither shown nor held.
    fn new() -> (Self, [ClipId; 2]) {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        ctx.enable_accesskit();
        let mut project = Project::new("Beats");
        project.tracks.clear();
        let mut ids = Vec::new();
        for (name, track, start, length) in [
            ("song.wav", "Music", 0.5, 6.0),
            ("voice.wav", "Voice", 7.0, 1.0),
        ] {
            let mut asset = Asset::new(
                format!("/media/{name}").into(),
                MediaKind::Audio,
                seconds(10.0),
            );
            asset.name = name.to_owned();
            let mut clip = Clip::from_asset(&asset, seconds(start));
            clip.duration = seconds(length);
            ids.push(clip.id);
            let mut track = Track::new(track, TrackKind::Audio);
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
            peaks: HeldPeaks::default(),
            playhead: Time::ZERO,
            size: vec2(900.0, 300.0),
            time: 0.0,
            seen: Vec::new(),
            ticks: Vec::new(),
            refused: Vec::new(),
            said: Vec::new(),
        };
        bench.frame(Vec::new());
        (bench, [ids[0], ids[1]])
    }

    /// The bench with the song's beats shown and held.
    fn beating() -> (Self, [ClipId; 2]) {
        let (mut bench, ids) = Self::new();
        bench.hold("song.wav", grid(20));
        (bench, ids)
    }

    fn asset(&self, name: &str) -> Asset {
        (self.editor.project().assets.values())
            .find(|asset| asset.name == name)
            .cloned()
            .expect("FIXTURE: the media is in the library")
    }

    /// Shows the beats of the media with this name, and holds `grid` as
    /// theirs as the lane would answer the tile that wants them.
    fn hold(&mut self, name: &str, grid: BeatGrid) {
        let asset = self.asset(name);
        let beats = &mut self.peaks.beats;
        beats.show(asset.id, true);
        beats.want(&asset);
        let project = self.editor.project();
        beats.ask_with(project, |_| true, |_| Ok(1));
        let source = SourceIdentity::of(&asset);
        let instance = uuid::Uuid::nil();
        let taken = beats.take(
            instance,
            Some(&asset),
            1,
            instance,
            asset.id,
            &source,
            Ok(Some(grid)),
        );
        assert_eq!(taken, None);
        assert!(beats.grid(&asset).is_some(), "FIXTURE: the grid is held");
        self.frame(Vec::new());
    }

    fn edit(&mut self, command: Command) {
        let label = command.label().to_owned();
        if let Err(error) = self.editor.dispatch(command) {
            panic!("FIXTURE: \"{label}\" was refused: {error}");
        }
        self.frame(Vec::new());
    }

    fn clip(&self, id: ClipId) -> Clip {
        self.editor.project().clip(id).expect("the clip").1.clone()
    }

    fn steps(&self) -> usize {
        self.editor.history().count()
    }

    /// One frame of the application with these events in it.
    fn frame(&mut self, events: Vec<Event>) {
        fn meshes(shape: &egui::Shape, found: &mut Vec<usize>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| meshes(shape, found)),
                egui::Shape::Mesh(mesh) if mesh.texture_id == egui::TextureId::default() => {
                    found.push(mesh.vertices.len() / 4);
                }
                _ => {}
            }
        }
        self.time += 0.05;
        self.peaks.begin_frame();
        let ctx = self.ctx.clone();
        let project = self.editor.snapshot();
        let mut actions: Vec<Action> = Vec::new();
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
                &mut self.peaks,
                &TimelineInput {
                    p: &self.palette,
                    project: &project,
                    playhead: self.playhead,
                    snapping: true,
                    media: None,
                    can_paste: false,
                    freeze_hold: seconds(2.0),
                    separator: crate::stems::Tool::Missing,
                },
                &mut actions,
            );
        });
        output.textures_delta.clear();
        self.ticks.clear();
        for clipped in &output.shapes {
            meshes(&clipped.shape, &mut self.ticks);
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
                Action::ShowBeats { assets, shown } => {
                    for asset in assets {
                        self.peaks.beats.show(asset, shown);
                    }
                }
                other => panic!("FIXTURE: the timeline asked for {other:?}"),
            }
        }
    }

    fn find(&self, name: &str) -> Option<&Seen> {
        (self.seen.iter())
            .find(|seen| seen.name == name && !matches!(seen.role, Role::Label | Role::TextRun))
    }

    /// Where a time of the project is in the lanes.
    fn x(&self, at: f64) -> f32 {
        HEAD_WIDTH + at as f32 * self.timeline.zoom - self.timeline.scroll.x
    }

    /// The tile of a clip, as the lanes lay it out.
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

    /// Takes hold at `from` and carries to `to`, without letting go.
    fn carry(&mut self, from: Pos2, to: Pos2) {
        self.move_to(from);
        self.frame(vec![Self::button(from, PointerButton::Primary, true)]);
        for step in 1..=5 {
            self.move_to(from + (to - from) * (step as f32 / 5.0));
        }
    }

    fn release(&mut self, pos: Pos2) {
        self.frame(vec![Self::button(pos, PointerButton::Primary, false)]);
        self.frame(Vec::new());
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.carry(from, to);
        self.release(to);
    }

    /// Takes the end of a clip's tile at its name's row and lets it go at
    /// this time of the project.
    fn trim_end(&mut self, clip: ClipId, to: f64) {
        let tile = self.tile(clip);
        let from = pos2(tile.right() - 2.0, tile.top() + 8.0);
        self.drag(from, pos2(self.x(to) - 2.0, from.y));
    }

    /// Takes a clip by its name's row and moves it by `by` seconds.
    fn move_by(&mut self, clip: ClipId, by: f64) {
        let tile = self.tile(clip);
        let from = pos2(tile.center().x, tile.top() + 8.0);
        let to = from + vec2(by as f32 * self.timeline.zoom, 0.0);
        self.drag(from, to);
    }

    fn secondary_click_at(&mut self, pos: Pos2) {
        self.move_to(pos);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, true)]);
        self.frame(vec![Self::button(pos, PointerButton::Secondary, false)]);
        self.frame(Vec::new());
    }

    /// Presses the menu row with this name, and lets it go there.
    fn choose(&mut self, name: &str) {
        let rect = self.menu_row(name);
        let pos = rect.center();
        self.move_to(pos);
        self.frame(vec![Self::button(pos, PointerButton::Primary, true)]);
        self.frame(vec![Self::button(pos, PointerButton::Primary, false)]);
        self.frame(Vec::new());
    }

    fn menu_row(&self, name: &str) -> Rect {
        match self.find(name) {
            Some(seen) => seen.rect,
            None => {
                let names: Vec<&str> = self.seen.iter().map(|seen| seen.name.as_str()).collect();
                panic!("FIXTURE: no row is named \"{name}\" among {names:?}")
            }
        }
    }
}

// ---- Ticks ----------------------------------------------------------------

#[test]
fn a_tile_at_twice_the_speed_ticks_at_half_the_spacing_and_a_trimmed_one_only_inside() {
    let asset = Asset::new("/media/song.wav".into(), MediaKind::Audio, seconds(60.0));
    let beats = grid(100);
    let mut clip = Clip::from_asset(&asset, seconds(2.0));
    clip.duration = seconds(4.0);
    let ticks = |clip: &Clip| beat_ticks(clip, &beats, seconds(0.0), seconds(30.0), 80.0);
    let gaps =
        |ticks: &[Time]| -> Vec<Time> { ticks.windows(2).map(|pair| pair[1] - pair[0]).collect() };
    let plain = ticks(&clip);
    assert_eq!(plain.len(), 8);
    assert!(
        gaps(&plain).iter().all(|gap| *gap == seconds(0.5)),
        "{plain:?}"
    );
    clip.playback_rate = 2.0;
    let fast = ticks(&clip);
    assert_eq!(fast.len(), 16);
    assert!(
        gaps(&fast).iter().all(|gap| *gap == seconds(0.25)),
        "{fast:?}"
    );
    assert_eq!(fast[0], seconds(2.125));

    // Trimmed at both ends, only the beats it still plays, where they were.
    clip.playback_rate = 1.0;
    let trimmed = Clip {
        start: seconds(3.1),
        source_in: seconds(1.1),
        duration: seconds(1.5),
        ..clip.clone()
    };
    assert_eq!(
        ticks(&trimmed),
        [seconds(3.25), seconds(3.75), seconds(4.25)]
    );
    assert!(ticks(&trimmed).iter().all(|at| plain.contains(at)));

    // Closer than four points they are not drawn: at seven points a second
    // half a second is three and a half.
    assert!(beat_ticks(&clip, &beats, seconds(0.0), seconds(30.0), 7.0).is_empty());
    assert_eq!(
        beat_ticks(&clip, &beats, seconds(0.0), seconds(30.0), 8.0).len(),
        8
    );
}

#[test]
fn a_tile_whose_beats_are_shown_draws_a_tick_at_each_while_it_is_heard() {
    let (mut bench, [song, _]) = Bench::new();
    // Not shown: no ticks.
    assert!(bench.ticks.is_empty(), "{:?}", bench.ticks);
    bench.hold("song.wav", grid(20));
    // The song plays source 0 to 6 s: the beats from 0.25 to 5.75 s.
    assert_eq!(bench.ticks, [12]);
    assert_eq!(bench.timeline.beats.len(), 12);
    assert!(bench.timeline.beats.iter().all(|(id, _)| *id == song));
    // A muted track is not heard, and shows none.
    let track_id = bench.editor.project().clip(song).unwrap().0.id;
    let mute = |muted| Command::SetTrackState {
        track_id,
        locked: None,
        hidden: None,
        muted: Some(muted),
        name: None,
        gain: None,
        solo: None,
    };
    bench.edit(mute(true));
    assert!(bench.ticks.is_empty() && bench.timeline.beats.is_empty());
    bench.edit(mute(false));
    assert_eq!(bench.ticks, [12]);
    // Too far out to tell apart, none.
    bench.timeline.zoom = 7.0;
    bench.frame(Vec::new());
    assert!(bench.ticks.is_empty() && bench.timeline.beats.is_empty());
}

// ---- Snapping -------------------------------------------------------------

#[test]
fn a_trimmed_edge_within_reach_of_a_beat_lands_on_it_and_shows_the_guide() {
    let (mut bench, [song, _]) = Bench::beating();
    bench.selection.only(song);
    let steps = bench.steps();
    // Let go five points from the beat at 5.75 s.
    let near = 5.75 + 5.0 / f64::from(bench.timeline.zoom);
    let tile = bench.tile(song);
    let from = pos2(tile.right() - 2.0, tile.top() + 8.0);
    bench.carry(from, pos2(bench.x(near) - 2.0, from.y));
    assert_eq!(bench.timeline.snap_guide, Some(seconds(5.75)));
    bench.release(pos2(bench.x(near) - 2.0, from.y));
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.clip(song).end(), seconds(5.75));
    // The playhead lands on one from the ruler too.
    let ruler = STRIP + RULER * 0.5;
    let at = pos2(bench.x(3.25 + 4.0 / f64::from(bench.timeline.zoom)), ruler);
    bench.drag(at, at);
    assert_eq!(bench.playhead, seconds(3.25));
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn a_moved_clip_lands_on_another_clips_beat_and_never_on_its_own() {
    let (mut bench, [song, voice]) = Bench::beating();
    let rate = bench.editor.project().settings.frame_rate;
    // The voice is taken to 3.27 s: the song's beat at 3.25 s takes it.
    bench.move_by(voice, 3.27 - 7.0);
    assert_eq!(
        bench.clip(voice).start,
        seconds(3.25),
        "{:?}",
        bench.refused
    );
    // The song moved by 0.27 s would start 0.02 s after where its own beat
    // was; its beats move with it, so it lands on a frame.
    bench.move_by(voice, 4.0);
    bench.move_by(song, 0.27);
    let start = bench.clip(song).start;
    assert_ne!(start, seconds(0.75));
    assert_eq!(start, rules::quantize(start, rate));
    assert!((start.as_seconds() - 0.77).abs() < 0.02, "{start:?}");
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

#[test]
fn with_its_beats_hidden_nothing_lands_on_them() {
    let (mut bench, [song, voice]) = Bench::beating();
    let rate = bench.editor.project().settings.frame_rate;
    let near = 5.75 + 5.0 / f64::from(bench.timeline.zoom);
    let hidden = |bench: &mut Bench| {
        let asset = bench.asset("song.wav");
        bench.peaks.beats.show(asset.id, false);
        bench.frame(Vec::new());
    };
    hidden(&mut bench);
    assert!(bench.ticks.is_empty() && bench.timeline.beats.is_empty());
    bench.selection.only(song);
    bench.trim_end(song, near);
    let end = bench.clip(song).end();
    assert_ne!(end, seconds(5.75));
    assert_eq!(end, rules::quantize(end, rate));
    bench.move_by(voice, 3.27 - 7.0);
    assert_ne!(bench.clip(voice).start, seconds(3.25));
    assert!(bench.refused.is_empty(), "{:?}", bench.refused);
}

// ---- The clip menu --------------------------------------------------------

#[test]
fn the_menu_of_an_audio_lane_clip_shows_and_hides_beats_and_marks_them() {
    let (mut bench, [song, _]) = Bench::new();
    let asset = bench.asset("song.wav");
    let steps = bench.steps();
    bench.secondary_click_at(bench.tile(song).center());
    assert!(bench.find("Markers from beats").is_none());
    bench.choose("Show beats");
    assert!(bench.peaks.beats.shown(asset.id));
    // Shown and still being found: the row is there, and cannot be chosen.
    bench.secondary_click_at(bench.tile(song).center());
    assert!(
        bench
            .find("Markers from beats")
            .is_some_and(|row| row.disabled)
    );
    assert!(bench.find("Hide beats").is_some());
    bench.choose("Hide beats");
    assert!(!bench.peaks.beats.shown(asset.id));

    // Held, the song's beats are marked: twelve, in one step.
    bench.hold("song.wav", grid(20));
    bench.secondary_click_at(bench.tile(song).center());
    assert!(
        bench
            .find("Markers from beats")
            .is_some_and(|row| !row.disabled)
    );
    bench.choose("Markers from beats");
    assert_eq!(bench.steps(), steps + 1, "{:?}", bench.refused);
    assert_eq!(bench.editor.undo_label(), Some(peaks::BEAT_MARKERS));
    let markers = &bench.editor.project().markers;
    assert_eq!(markers.len(), 12);
    assert!(
        markers
            .iter()
            .all(|marker| marker.label == peaks::BEAT_LABEL)
    );
    assert_eq!(markers[0].time, seconds(0.75));
    assert!(bench.refused.is_empty() && bench.said.is_empty());
}
