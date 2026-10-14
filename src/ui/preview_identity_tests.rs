//! Issue 12: which picture a clip that stays on screen is painted with.
//!
//! Each test builds its timeline with the commands the editor uses, paints
//! it with [`show`] inside a real egui pass, hands decoded frames over the way
//! the application does, and reads the textures of the painted quads. Nothing
//! here decodes: the viewer is given no media service, so a layer is kept once
//! it was painted and holds what [`Textures::accept`] put in it.
//!
//! A failure says what kind it is. `FIXTURE:` is a precondition of the test
//! itself and proves nothing about the viewer. `ISSUE12:` is the behavior the
//! issue describes. `CONTROL:` is behavior that holds today and must still
//! hold after the correction.
use super::{SourceInput, ViewerInput, ViewerState, show, show_source};
use crate::{
    textures::{Textures, Viewing},
    theme::{self, Accent, Appearance, Palette},
    ui::{Action, ClipDraft, Selection},
};
use editor_core::{
    Asset, Clip, ClipId, Command, Editor, FrameRate, MediaKind, Playback, Project, Time, Track,
    TrackId, TrackKind, Transform,
};
use editor_media::Frame;
use eframe::egui::{self, Color32, ImageData, Pos2, Rect, Shape, TextureId, Vec2, vec2};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const RED: [u8; 4] = [220, 40, 40, 255];
const BLUE: [u8; 4] = [40, 80, 220, 255];
const AMBER: [u8; 4] = [230, 170, 30, 255];
const GREEN: [u8; 4] = [40, 190, 90, 255];
/// One picture for each clip of an asset that is on screen at once.
const SETTLED: [[u8; 4]; 2] = [RED, BLUE];

/// Canvas pixels between the columns clips are placed in. With the scale
/// below, layers in different columns never overlap.
const COLUMN: f32 = 640.0;
const SCALE: f32 = 0.25;

fn window() -> Rect {
    Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))
}

fn fixture(holds: bool, what: &str) {
    assert!(holds, "FIXTURE: {what}");
}

/// The texel a solid frame of this colour is uploaded as.
fn shown(colour: [u8; 4]) -> Color32 {
    let [r, g, b, a] = colour;
    Color32::from_rgba_premultiplied(r, g, b, a)
}

fn solid(colour: [u8; 4]) -> Frame {
    Frame {
        width: 4,
        height: 4,
        rgba: colour.repeat(16).into(),
    }
}

fn video(path: &str) -> Asset {
    let mut asset = Asset::new(
        PathBuf::from(path),
        MediaKind::Video,
        Time::from_seconds(10.0),
    );
    asset.width = 1920;
    asset.height = 1080;
    asset.frame_rate = Some(FrameRate::default());
    asset.file_size = 1000;
    asset
}

/// The time of a frame of the project, which is also the assets' grid here.
fn frame(index: i64) -> Time {
    FrameRate::default().frame_time(index)
}

/// Left, middle or right of a centre.
fn column_of(offset: f32, reach: f32) -> i8 {
    if offset < -reach {
        -1
    } else if offset > reach {
        1
    } else {
        0
    }
}

/// Where a clip goes: its track, its column on the canvas and its span, in
/// seconds.
struct Place {
    track: TrackId,
    column: i8,
    start: f64,
    source_in: f64,
    duration: f64,
}

/// A project edited only through commands.
struct Scene {
    editor: Editor,
    asset: Asset,
    /// Painted first.
    lower: TrackId,
    /// Painted after `lower`.
    upper: TrackId,
}

impl Scene {
    fn new() -> Self {
        let editor = Editor::new(Project::new("Preview identity"));
        let Ok(editor) = editor else {
            panic!("FIXTURE: a new project is not valid");
        };
        let lower = editor
            .project()
            .tracks
            .iter()
            .find(|track| track.kind == TrackKind::Video)
            .map(|track| track.id);
        let Some(lower) = lower else {
            panic!("FIXTURE: a new project has no video track");
        };
        let asset = video("/media/identity.mp4");
        let upper = Track::new("Video 2", TrackKind::Video);
        let mut scene = Self {
            editor,
            asset: asset.clone(),
            lower,
            upper: upper.id,
        };
        scene.edit(Command::AddAssets {
            assets: vec![asset],
        });
        // Added after the tracks a new project has, so it is painted last.
        scene.edit(Command::AddTrack { track: upper });
        scene
    }

    fn edit(&mut self, command: Command) {
        let label = command.label().to_owned();
        if let Err(error) = self.editor.dispatch(command) {
            panic!("FIXTURE: \"{label}\" was refused: {error}");
        }
    }

    fn project(&self) -> Arc<Project> {
        self.editor.snapshot()
    }

    /// Adds a clip of the scene's own asset.
    fn clip(&mut self, place: Place) -> ClipId {
        let asset = self.asset.clone();
        self.clip_of(&asset, place)
    }

    fn clip_of(&mut self, asset: &Asset, place: Place) -> ClipId {
        let mut clip = Clip::from_asset(asset, Time::from_seconds(place.start));
        clip.source_in = Time::from_seconds(place.source_in);
        clip.duration = Time::from_seconds(place.duration);
        clip.transform = Transform {
            position: [f32::from(place.column) * COLUMN, 0.0],
            scale: [SCALE, SCALE],
            ..Transform::default()
        };
        let id = clip.id;
        self.edit(Command::AddClip {
            track_id: place.track,
            clip,
        });
        id
    }

    fn hide(&mut self, track: TrackId) {
        self.edit(Command::SetTrackState {
            track_id: track,
            locked: None,
            hidden: Some(true),
            muted: None,
            name: None,
            gain: None,
            solo: None,
        });
    }
}

/// The clips on screen at `playhead`, in the order they are painted.
fn active(project: &Project, playhead: Time) -> Vec<ClipId> {
    project
        .active_clips(playhead)
        .map(|(_, clip)| clip.id)
        .collect()
}

/// The timeline clock after playing from `from` for `millis`, ticked the way
/// the application ticks it.
fn played(project: &Project, from: Time, millis: u64) -> Playback {
    let mut playback = Playback::default();
    playback.seek(from);
    playback.play();
    let began = Instant::now();
    playback.tick(began, project.duration());
    playback.tick(began + Duration::from_millis(millis), project.duration());
    playback
}

/// A textured quad that was painted.
struct Quad {
    texture: TextureId,
    centre: Pos2,
}

fn collect(shape: &Shape, quads: &mut Vec<Quad>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, quads);
            }
        }
        // The font texture is what an untextured shape names.
        Shape::Mesh(mesh) if mesh.texture_id != TextureId::default() => {
            let sum = mesh
                .vertices
                .iter()
                .fold(Vec2::ZERO, |sum, vertex| sum + vertex.pos.to_vec2());
            let count = mesh.vertices.len().max(1) as f32;
            quads.push(Quad {
                texture: mesh.texture_id,
                centre: (sum / count).to_pos2(),
            });
        }
        Shape::Rect(rect) if rect.fill_texture_id() != TextureId::default() => {
            quads.push(Quad {
                texture: rect.fill_texture_id(),
                centre: rect.rect.center(),
            });
        }
        _ => {}
    }
}

/// What one pass painted with a picture.
struct Painted {
    quads: Vec<Quad>,
}

impl Painted {
    /// The texture of the quad painted where `clip` is placed.
    fn texture(&self, project: &Project, clip: ClipId) -> Option<TextureId> {
        let Some((_, clip)) = project.clip(clip) else {
            panic!("FIXTURE: the clip is not in the project");
        };
        let column = column_of(clip.transform.position[0], COLUMN * 0.5);
        let centre = window().center().x;
        let mut found = self
            .quads
            .iter()
            .filter(|quad| column_of(quad.centre.x - centre, 100.0) == column);
        let first = found.next()?;
        fixture(
            found.next().is_none(),
            "two layers were painted in one column",
        );
        Some(first.texture)
    }
}

/// What a clip's quad was painted with: which texture, and the picture in it.
/// Neither when no quad was painted for the clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    texture: Option<TextureId>,
    picture: Option<Color32>,
}

impl Seen {
    const NOTHING: Self = Self {
        texture: None,
        picture: None,
    };
}

/// Clips that each show a picture of their own.
struct Settled {
    seen: HashMap<ClipId, Seen>,
}

impl Settled {
    fn seen(&self, clip: ClipId) -> Seen {
        self.seen[&clip]
    }
}

/// A window the viewer is painted in, and the textures it draws from.
struct Stage {
    ctx: egui::Context,
    textures: Textures,
    viewer: ViewerState,
    selection: Selection,
    draft: Option<ClipDraft>,
    palette: Palette,
    /// The first texel last uploaded to each texture.
    pictures: HashMap<TextureId, Color32>,
}

impl Stage {
    fn new() -> Self {
        let ctx = egui::Context::default();
        let palette = Palette::new(Appearance::Dark, Accent::default());
        theme::install_fonts(&ctx);
        theme::apply(&ctx, &palette);
        Self {
            ctx,
            textures: Textures::default(),
            viewer: ViewerState::default(),
            selection: Selection::default(),
            draft: None,
            palette,
            pictures: HashMap::new(),
        }
    }

    fn input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(window()),
            ..Default::default()
        }
    }

    /// One frame of the timeline viewer, as `App::window_ui` paints it.
    fn paint(&mut self, project: &Project, playhead: Time, playing: bool) -> Painted {
        let query = editor_core::ProjectQuery::new(std::sync::Arc::new(project.clone()));
        self.textures.begin_frame();
        let ctx = self.ctx.clone();
        let mut actions: Vec<Action> = Vec::new();
        let output = ctx.run_ui(Self::input(), |ui| {
            let rect = ui.max_rect();
            show(
                ui,
                rect,
                &mut self.viewer,
                &mut self.selection,
                &mut self.draft,
                &mut self.textures,
                &ViewerInput {
                    p: &self.palette,
                    project: query.project(),
                    query: Some(&query),
                    playhead,
                    playing,
                    speed: 1.0,
                    looping: false,
                    media: None,
                    record: Default::default(),
                },
                &mut actions,
            );
        });
        self.painted(output)
    }

    /// One frame of the source monitor showing `asset`.
    fn paint_source(&mut self, asset: &Asset) -> Painted {
        self.textures.begin_frame();
        let ctx = self.ctx.clone();
        let mut actions: Vec<Action> = Vec::new();
        let output = ctx.run_ui(Self::input(), |ui| {
            let rect = ui.max_rect();
            show_source(
                ui,
                rect,
                &mut self.textures,
                &SourceInput {
                    p: &self.palette,
                    asset,
                    rate: FrameRate::default(),
                    background: [0, 0, 0, 255],
                    playhead: Time::ZERO,
                    playing: false,
                    speed: 1.0,
                    looping: false,
                    media: None,
                },
                &mut actions,
            );
        });
        self.painted(output)
    }

    /// Applies the pass's texture changes to `pictures`, as a renderer would
    /// to the GPU, and keeps the textured quads.
    fn painted(&mut self, mut output: egui::FullOutput) -> Painted {
        for (id, deltas) in std::mem::take(&mut output.textures_delta.set) {
            for delta in deltas {
                let ImageData::Color(image) = &delta.image;
                // A full-origin partial upload changes the first texel just
                // as a complete replacement does. Preserve the picture oracle
                // when native textures are updated in place.
                if delta.pos.is_none_or(|position| position == [0, 0])
                    && let Some(texel) = image.pixels.first()
                {
                    self.pictures.insert(id, *texel);
                }
            }
        }
        for id in std::mem::take(&mut output.textures_delta.free) {
            self.pictures.remove(&id);
        }
        let mut quads = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut quads);
        }
        Painted { quads }
    }

    fn seen(&self, painted: &Painted, project: &Project, clip: ClipId) -> Seen {
        let texture = painted.texture(project, clip);
        Seen {
            texture,
            picture: texture.and_then(|texture| self.pictures.get(&texture).copied()),
        }
    }

    /// A decoded frame arriving the way `App::take_media` hands it over:
    /// addressed by the consumer that asked for it, its asset and the size
    /// asked at, and by nothing else.
    ///
    /// This is the one place that knows how a layer is addressed.
    fn arrive(&mut self, viewing: Viewing, asset: &Asset, colour: [u8; 4]) -> bool {
        let Some((consumer, size)) = self.textures.asked(viewing) else {
            return false;
        };
        let frame = solid(colour);
        let time = self.textures.admitted(consumer);
        self.textures.accept(
            &self.ctx,
            Some(asset),
            Some(consumer),
            asset.id,
            time,
            size,
            &frame,
        )
    }

    /// A decoded frame for `clip`, which was painted and so asked for one.
    fn deliver(&mut self, asset: &Asset, clip: ClipId, colour: [u8; 4]) {
        let taken = self.arrive(Viewing::Clip(clip), asset, colour);
        fixture(taken, "no painted layer of the clip took the frame");
    }

    /// A decoded frame for the source monitor.
    fn deliver_source(&mut self, asset: &Asset, colour: [u8; 4]) {
        let taken = self.arrive(Viewing::Source, asset, colour);
        fixture(taken, "the source monitor's layer did not take the frame");
    }

    /// Paints `clips`, all of `asset` and all on screen at `playhead`, until
    /// each shows a picture of its own: a paint that asks, one decoded frame
    /// per clip, and a paint that shows them.
    fn settle(
        &mut self,
        project: &Project,
        playhead: Time,
        asset: &Asset,
        clips: &[ClipId],
        colours: &[[u8; 4]],
    ) -> Settled {
        fixture(clips.len() <= colours.len(), "a picture for every clip");
        self.paint(project, playhead, false);
        for (clip, colour) in clips.iter().zip(colours) {
            self.deliver(asset, *clip, *colour);
        }
        let painted = self.paint(project, playhead, false);
        let mut settled = Settled {
            seen: HashMap::new(),
        };
        for clip in clips {
            let seen = self.seen(&painted, project, *clip);
            if !colours
                .iter()
                .any(|colour| Some(shown(*colour)) == seen.picture)
            {
                panic!("FIXTURE: a clip did not settle on a delivered picture: {seen:?}");
            }
            fixture(
                settled.seen.values().all(|other| *other != seen),
                "two clips settled on one texture and picture",
            );
            settled.seen.insert(*clip, seen);
        }
        settled
    }
}

/// Two clips of one asset on screen together at frame 59, at different source
/// times and equal scale: A below for two seconds from source 0 s, B above
/// for four from source 1 s.
struct Pair {
    scene: Scene,
    a: ClipId,
    b: ClipId,
}

fn overlapping() -> Pair {
    let mut scene = Scene::new();
    let a = scene.clip(Place {
        track: scene.lower,
        column: -1,
        start: 0.0,
        source_in: 0.0,
        duration: 2.0,
    });
    let b = scene.clip(Place {
        track: scene.upper,
        column: 1,
        start: 0.0,
        source_in: 1.0,
        duration: 4.0,
    });
    fixture(
        active(&scene.project(), frame(59)) == [a, b],
        "A then B are painted at frame 59",
    );
    Pair { scene, a, b }
}

// ---- Issue 12: transitions that re-bind a continuing clip -----------------

#[test]
fn a_clip_keeps_its_picture_when_playback_passes_the_end_of_a_clip_under_it() {
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);

    let playback = played(&project, frame(59), 50);
    fixture(
        playback.playing && active(&project, playback.position) == [b],
        "playback is past the end of A and inside B",
    );
    let painted = stage.paint(&project, playback.position, playback.playing);
    assert_eq!(
        stage.seen(&painted, &project, b),
        before.seen(b),
        "ISSUE12: playback passed the end of A and no frame has arrived since. B is \
         painted with the left; its own frame is the right. A's was {:?}",
        before.seen(a)
    );
}

#[test]
fn a_clip_keeps_its_picture_when_a_track_under_it_is_hidden() {
    let Pair { mut scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);

    scene.hide(scene.lower);
    let project = scene.project();
    fixture(active(&project, frame(59)) == [b], "only B is painted");
    let painted = stage.paint(&project, frame(59), false);
    assert_eq!(
        stage.seen(&painted, &project, b),
        before.seen(b),
        "ISSUE12: A's track was hidden and no frame has arrived since. B is painted \
         with the left; its own frame is the right. A's was {:?}",
        before.seen(a)
    );
}

#[test]
fn clips_keep_their_pictures_when_their_tracks_are_reordered() {
    let Pair { mut scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);

    scene.edit(Command::ReorderTrack {
        track_id: scene.upper,
        index: 0,
    });
    let project = scene.project();
    fixture(
        active(&project, frame(59)) == [b, a],
        "B is painted before A",
    );
    let painted = stage.paint(&project, frame(59), false);
    assert_eq!(
        (
            stage.seen(&painted, &project, a),
            stage.seen(&painted, &project, b)
        ),
        (before.seen(a), before.seen(b)),
        "ISSUE12: the tracks were reordered and no frame has arrived since. A and B \
         are painted with the left; their own frames are the right"
    );
}

/// B alone on screen and settled, then playback reaches the start of A on the
/// track under it. Returns what B showed before, and what A and B are painted
/// with in the first paint after.
fn a_clip_appears_under_another() -> (Seen, Seen, Seen) {
    let mut scene = Scene::new();
    let a = scene.clip(Place {
        track: scene.lower,
        column: -1,
        start: 2.0,
        source_in: 0.0,
        duration: 2.0,
    });
    let b = scene.clip(Place {
        track: scene.upper,
        column: 1,
        start: 0.0,
        source_in: 1.0,
        duration: 4.0,
    });
    let project = scene.project();
    fixture(
        active(&project, frame(59)) == [b],
        "only B is painted at frame 59",
    );
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(59), &scene.asset, &[b], &SETTLED);

    let playback = played(&project, frame(59), 50);
    fixture(
        playback.playing && active(&project, playback.position) == [a, b],
        "playback is inside A and B",
    );
    let painted = stage.paint(&project, playback.position, playback.playing);
    (
        before.seen(b),
        stage.seen(&painted, &project, a),
        stage.seen(&painted, &project, b),
    )
}

#[test]
fn a_clip_keeps_its_picture_when_a_clip_appears_under_it() {
    let (own, _, b) = a_clip_appears_under_another();
    assert_eq!(
        b, own,
        "ISSUE12: playback reached the start of A under B and no frame has arrived \
         since. B is painted with the left; its own frame is the right"
    );
}

#[test]
fn a_clip_that_appears_is_not_painted_with_the_picture_of_one_that_continues() {
    let (own, a, _) = a_clip_appears_under_another();
    assert_ne!(
        a, own,
        "ISSUE12: playback reached the start of A under B and no frame of A has \
         arrived. A is painted with B's frame"
    );
}

#[test]
fn a_clip_that_returns_by_undo_is_not_painted_with_the_picture_of_one_that_stayed() {
    let Pair { mut scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);

    scene.hide(scene.lower);
    let project = scene.project();
    stage.paint(&project, frame(59), false);
    // B is the asset's only clip on screen, and the frame it asks for now
    // comes back addressed to B.
    stage.deliver(&scene.asset, b, GREEN);
    let painted = stage.paint(&project, frame(59), false);
    fixture(
        stage.seen(&painted, &project, b).picture == Some(shown(GREEN)),
        "B shows the frame decoded for it while A was hidden",
    );

    fixture(scene.editor.undo(), "the hide is undone");
    let project = scene.project();
    fixture(active(&project, frame(59)) == [a, b], "A is painted again");
    let painted = stage.paint(&project, frame(59), false);
    assert_ne!(
        stage.seen(&painted, &project, a).picture,
        Some(shown(GREEN)),
        "ISSUE12: A came back by undo and no frame of A has arrived. A is painted \
         with the frame decoded for B while A was hidden"
    );
}

// ---- Controls: hold today, and must hold after the correction --------------

#[test]
fn settled_clips_of_one_asset_show_separate_pictures() {
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    let settled = stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);
    assert_ne!(
        settled.seen(a).texture,
        settled.seen(b).texture,
        "CONTROL: A and B share a texture"
    );
    assert_ne!(
        settled.seen(a).picture,
        settled.seen(b).picture,
        "CONTROL: A and B show one picture"
    );
    // Nothing changes while nothing arrives.
    for _ in 0..3 {
        let painted = stage.paint(&project, frame(59), false);
        assert_eq!(painted.quads.len(), 2, "CONTROL: two layers are painted");
        assert_eq!(
            (
                stage.seen(&painted, &project, a),
                stage.seen(&painted, &project, b)
            ),
            (settled.seen(a), settled.seen(b)),
            "CONTROL: a settled clip changed with nothing decoded"
        );
    }
}

#[test]
fn a_clip_holds_its_own_frame_until_its_next_one_arrives() {
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    let settled = stage.settle(&project, frame(30), &scene.asset, &[a, b], &SETTLED);

    // A paused step: both clips stay on screen and both ask for new frames.
    let mut playback = Playback::default();
    playback.seek(frame(30));
    playback.step(15, project.settings.frame_rate);
    fixture(
        active(&project, playback.position) == [a, b],
        "A and B are both painted after the step",
    );
    let painted = stage.paint(&project, playback.position, playback.playing);
    assert_eq!(
        (
            stage.seen(&painted, &project, a),
            stage.seen(&painted, &project, b)
        ),
        (settled.seen(a), settled.seen(b)),
        "CONTROL: a clip did not hold its own frame while its next was decoding"
    );

    // B's frame arrives first: B changes, in the texture it had, and A holds.
    stage.deliver(&scene.asset, b, GREEN);
    let painted = stage.paint(&project, playback.position, playback.playing);
    assert_eq!(
        stage.seen(&painted, &project, b),
        Seen {
            texture: settled.seen(b).texture,
            picture: Some(shown(GREEN)),
        },
        "CONTROL: B does not show its new frame in its own texture"
    );
    assert_eq!(
        stage.seen(&painted, &project, a),
        settled.seen(a),
        "CONTROL: A changed when B's frame arrived"
    );
}

#[test]
fn the_source_monitor_shows_no_timeline_clips_picture_and_changes_none() {
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    let settled = stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);

    // The same asset in the source monitor: no poster and no frame yet.
    let painted = stage.paint_source(&scene.asset);
    assert!(
        painted.quads.is_empty(),
        "CONTROL: the source monitor painted a picture before one was decoded for it"
    );
    stage.deliver_source(&scene.asset, GREEN);
    let painted = stage.paint_source(&scene.asset);
    let [source] = painted.quads.as_slice() else {
        panic!("CONTROL: the source monitor did not paint exactly one picture");
    };
    assert!(
        Some(source.texture) != settled.seen(a).texture
            && Some(source.texture) != settled.seen(b).texture,
        "CONTROL: the source monitor is painted with a timeline clip's texture"
    );
    assert_eq!(
        stage.pictures.get(&source.texture),
        Some(&shown(GREEN)),
        "CONTROL: the source monitor does not show the frame decoded for it"
    );

    // Back on the timeline, each clip still has its own.
    let painted = stage.paint(&project, frame(59), false);
    assert_eq!(
        (
            stage.seen(&painted, &project, a),
            stage.seen(&painted, &project, b)
        ),
        (settled.seen(a), settled.seen(b)),
        "CONTROL: the source monitor's frame changed a timeline clip"
    );
}

/// A and B of one asset on two tracks for four seconds, one of them split at
/// two. Playback crosses the cut: the incoming clip must be painted at once
/// with the picture the outgoing one had, and the other clip must hold its
/// own.
fn a_cut_is_played_through(cut_the_lower: bool) {
    let mut scene = Scene::new();
    let a = scene.clip(Place {
        track: scene.lower,
        column: -1,
        start: 0.0,
        source_in: 0.0,
        duration: 4.0,
    });
    let b = scene.clip(Place {
        track: scene.upper,
        column: 1,
        start: 0.0,
        source_in: 1.0,
        duration: 4.0,
    });
    let (outgoing, other) = if cut_the_lower { (a, b) } else { (b, a) };
    let at = Time::from_seconds(2.0);
    scene.edit(Command::SplitClip {
        clip_id: outgoing,
        at,
    });
    let project = scene.project();
    let incoming = project
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.start == at)
        .map(|clip| clip.id);
    let Some(incoming) = incoming else {
        panic!("FIXTURE: the split made no clip starting at the cut");
    };
    fixture(
        incoming != a && incoming != b,
        "the incoming clip has an id of its own",
    );
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);

    let playback = played(&project, frame(59), 50);
    let now = active(&project, playback.position);
    fixture(
        playback.playing && now.contains(&incoming) && now.contains(&other) && now.len() == 2,
        "playback is past the cut, inside the incoming clip and the other one",
    );
    let painted = stage.paint(&project, playback.position, playback.playing);
    assert_eq!(
        stage.seen(&painted, &project, incoming),
        before.seen(outgoing),
        "CONTROL: across a cut the incoming clip is painted with the left; the \
         outgoing clip's frame is the right"
    );
    assert_eq!(
        stage.seen(&painted, &project, other),
        before.seen(other),
        "CONTROL: the clip that was not cut changed across the cut"
    );
}

#[test]
fn a_cut_on_the_lower_track_hands_its_picture_to_the_incoming_clip() {
    a_cut_is_played_through(true);
}

#[test]
fn a_cut_on_the_upper_track_hands_its_picture_to_the_incoming_clip() {
    a_cut_is_played_through(false);
}

#[test]
fn a_changed_source_file_blanks_that_assets_layers_and_no_others() {
    let Pair { mut scene, a, b } = overlapping();
    let other = video("/media/other.mp4");
    scene.edit(Command::AddAssets {
        assets: vec![other.clone()],
    });
    let third = Track::new("Video 3", TrackKind::Video);
    let track = third.id;
    scene.edit(Command::AddTrack { track: third });
    let c = scene.clip_of(
        &other,
        Place {
            track,
            column: 0,
            start: 0.0,
            source_in: 0.0,
            duration: 4.0,
        },
    );
    let project = scene.project();
    let mut stage = Stage::new();
    stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);
    let kept = stage.settle(&project, frame(59), &other, &[c], &[AMBER]);

    // What `App::media_changed` does to the textures when an asset now
    // decodes from another file.
    stage.textures.forget_asset(scene.asset.id);
    stage.textures.ask_again();
    let painted = stage.paint(&project, frame(59), false);
    assert_eq!(
        (
            stage.seen(&painted, &project, a),
            stage.seen(&painted, &project, b)
        ),
        (Seen::NOTHING, Seen::NOTHING),
        "CONTROL: a clip of the changed asset is still painted with an old picture"
    );
    assert_eq!(
        stage.seen(&painted, &project, c),
        kept.seen(c),
        "CONTROL: a clip of another asset lost its picture"
    );
}

#[test]
fn a_replaced_project_is_painted_without_the_pictures_before_it() {
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);

    // What `App::replace_editor` does to the textures.
    stage.textures.clear();
    let painted = stage.paint(&project, frame(59), false);
    assert!(
        painted.quads.is_empty(),
        "CONTROL: a layer is painted with a picture kept from before the project \
         was replaced"
    );
}

// ---- Controls added with the correction ------------------------------------

#[test]
fn clips_of_one_asset_ask_at_one_size_under_consumers_of_their_own() {
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);
    stage.paint_source(&scene.asset);
    let asked = [Viewing::Clip(a), Viewing::Clip(b), Viewing::Source]
        .map(|viewing| stage.textures.asked(viewing));
    let [Some(a), Some(b), Some(source)] = asked else {
        panic!("FIXTURE: a painted layer is not kept: {asked:?}");
    };
    assert_eq!(
        a.1, b.1,
        "CONTROL: clips of one asset at one scale ask at different sizes"
    );
    assert!(
        a.0 != b.0 && a.0 != source.0 && b.0 != source.0,
        "CONTROL: two layers ask under one consumer"
    );
}

#[test]
fn a_new_clip_is_not_painted_with_the_picture_of_one_deleted_from_its_place() {
    let Pair { mut scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);

    scene.edit(Command::DeleteClips {
        clip_ids: vec![a],
        ripple: false,
    });
    // Another clip of the asset where A was, at another source time.
    let c = scene.clip(Place {
        track: scene.lower,
        column: -1,
        start: 0.0,
        source_in: 5.0,
        duration: 2.0,
    });
    let project = scene.project();
    fixture(
        active(&project, frame(59)) == [c, b],
        "C then B are painted",
    );
    let painted = stage.paint(&project, frame(59), false);
    assert_eq!(
        stage.seen(&painted, &project, c),
        Seen::NOTHING,
        "CONTROL: a new clip is painted before a frame was decoded for it. A's \
         picture was {:?}",
        before.seen(a)
    );
    assert_eq!(
        stage.seen(&painted, &project, b),
        before.seen(b),
        "CONTROL: B changed when a clip under it was deleted and another added"
    );

    // Both edits undone: A is back with the picture it had.
    fixture(
        scene.editor.undo() && scene.editor.undo(),
        "two edits undone",
    );
    let project = scene.project();
    fixture(active(&project, frame(59)) == [a, b], "A is painted again");
    let painted = stage.paint(&project, frame(59), false);
    assert_eq!(
        (
            stage.seen(&painted, &project, a),
            stage.seen(&painted, &project, b)
        ),
        (before.seen(a), before.seen(b)),
        "CONTROL: a clip restored by undo does not hold its own last frame"
    );
}

/// The clip of `project` that starts at `at` on the track `clip` is on.
fn starting(project: &Project, clip: ClipId, at: Time) -> ClipId {
    let found = project
        .clip(clip)
        .and_then(|(track, _)| track.clips.iter().find(|clip| clip.start == at))
        .map(|clip| clip.id);
    let Some(found) = found else {
        panic!("FIXTURE: no clip starts at the cut");
    };
    found
}

/// A on the lower track for four seconds, cut at two and at three, under B.
/// Returns the three parts of A in order, and B.
fn cut_twice() -> (Scene, [ClipId; 3], ClipId) {
    let mut scene = Scene::new();
    let a = scene.clip(Place {
        track: scene.lower,
        column: -1,
        start: 0.0,
        source_in: 0.0,
        duration: 4.0,
    });
    let b = scene.clip(Place {
        track: scene.upper,
        column: 1,
        start: 0.0,
        source_in: 1.0,
        duration: 4.0,
    });
    let cuts = [Time::from_seconds(2.0), Time::from_seconds(3.0)];
    scene.edit(Command::SplitClip {
        clip_id: a,
        at: cuts[0],
    });
    let second = starting(&scene.project(), a, cuts[0]);
    scene.edit(Command::SplitClip {
        clip_id: second,
        at: cuts[1],
    });
    let third = starting(&scene.project(), a, cuts[1]);
    fixture(
        a != second && second != third && a != third,
        "each part has an id of its own",
    );
    (scene, [a, second, third], b)
}

#[test]
fn a_paused_seek_across_cuts_holds_the_cut_clips_frame_in_either_direction() {
    let (scene, [first, _, third], b) = cut_twice();
    let project = scene.project();
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(30), &scene.asset, &[first, b], &SETTLED);

    // From the first part straight to the third, over a part never painted.
    let mut playback = Playback::default();
    playback.seek(frame(100));
    fixture(
        !playback.playing && active(&project, playback.position) == [third, b],
        "paused inside the third part and B",
    );
    let painted = stage.paint(&project, playback.position, false);
    assert_eq!(
        (
            stage.seen(&painted, &project, third),
            stage.seen(&painted, &project, b)
        ),
        (before.seen(first), before.seen(b)),
        "CONTROL: after a seek across two cuts the incoming part and B are painted \
         with the left; the frame the cut clip held and B's own are the right"
    );
    // The frame decoded for the third part arrives in that same texture.
    stage.deliver(&scene.asset, third, GREEN);
    let held = Seen {
        texture: before.seen(first).texture,
        picture: Some(shown(GREEN)),
    };
    let painted = stage.paint(&project, playback.position, false);
    assert_eq!(
        stage.seen(&painted, &project, third),
        held,
        "CONTROL: the incoming part does not show its frame in the texture handed \
         to it"
    );

    // And back to the first part.
    playback.seek(frame(30));
    fixture(
        active(&project, playback.position) == [first, b],
        "paused inside the first part and B",
    );
    let painted = stage.paint(&project, playback.position, false);
    assert_eq!(
        (
            stage.seen(&painted, &project, first),
            stage.seen(&painted, &project, b)
        ),
        (held, before.seen(b)),
        "CONTROL: after a seek back across two cuts the first part and B are \
         painted with the left; the frame the cut clip held and B's own are the \
         right"
    );
}

#[test]
fn a_clip_that_plays_on_at_another_rate_is_not_handed_the_picture_before_it() {
    let (mut scene, [first, second, _], b) = cut_twice();
    // The second part now plays the source twice as fast: no longer the
    // same pictures carried on.
    scene.edit(Command::SetPlaybackRate {
        clip_id: second,
        rate: 2.0,
    });
    let project = scene.project();
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(59), &scene.asset, &[first, b], &SETTLED);

    let playback = played(&project, frame(59), 50);
    fixture(
        playback.playing && active(&project, playback.position) == [second, b],
        "playback is inside the second part and B",
    );
    let painted = stage.paint(&project, playback.position, playback.playing);
    assert_eq!(
        stage.seen(&painted, &project, second),
        Seen::NOTHING,
        "CONTROL: a clip at another rate is painted with the frame of the clip \
         before it, which was {:?}",
        before.seen(first)
    );
    assert_eq!(
        stage.seen(&painted, &project, b),
        before.seen(b),
        "CONTROL: B changed when playback passed onto another clip under it"
    );
}

/// A four second clip with this time map, cut at two seconds. Stepped across
/// the cut while paused, the incoming part must be painted at once with the
/// picture the outgoing one had: the two are one clip showing on.
fn a_cut_time_map_is_stepped_through(keys: &[(f64, f64, editor_core::Ease)]) {
    let mut scene = Scene::new();
    let a = scene.clip(Place {
        track: scene.lower,
        column: 0,
        start: 0.0,
        source_in: 1.0,
        duration: 4.0,
    });
    scene.edit(Command::SetRetime {
        clip_id: a,
        retime: Some(editor_core::Retime {
            keys: keys
                .iter()
                .map(|&(at, source, ease)| editor_core::TimeKey {
                    at: Time::from_seconds(at),
                    source: Time::from_seconds(source),
                    ease,
                })
                .collect(),
        }),
        duration: Time::from_seconds(4.0),
    });
    let at = frame(60);
    scene.edit(Command::SplitClip { clip_id: a, at });
    let project = scene.project();
    let incoming = starting(&project, a, at);
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(59), &scene.asset, &[a], &SETTLED);

    fixture(active(&project, at) == [incoming], "only the incoming part");
    let painted = stage.paint(&project, at, false);
    assert_eq!(
        stage.seen(&painted, &project, incoming),
        before.seen(a),
        "CONTROL: across a cut in a time map the incoming part is painted with          the left; the outgoing part's frame is the right ({keys:?})"
    );
}

#[test]
fn a_cut_in_a_time_map_hands_its_picture_to_the_incoming_part() {
    use editor_core::Ease::Linear;
    let ramp = editor_core::Ease::Bezier {
        x1: 0.42,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };
    // Straight, frozen at the cut, backwards, and eased.
    a_cut_time_map_is_stepped_through(&[(0.0, 0.0, Linear), (4.0, 2.0, Linear)]);
    a_cut_time_map_is_stepped_through(&[
        (0.0, 0.0, Linear),
        (1.0, 1.0, Linear),
        (4.0, 1.0, Linear),
    ]);
    a_cut_time_map_is_stepped_through(&[(0.0, 6.0, Linear), (4.0, 2.0, Linear)]);
    a_cut_time_map_is_stepped_through(&[(0.0, 0.0, ramp), (4.0, 6.0, Linear)]);
}

#[test]
fn a_clip_placed_to_carry_on_from_another_is_handed_its_picture() {
    // No split made these two: C was added where B ends, starting in the
    // source at the time B ends on. What joins them is where they are.
    let Pair { mut scene, a, b } = overlapping();
    let c = scene.clip(Place {
        track: scene.upper,
        column: 0,
        start: 4.0,
        source_in: 5.0,
        duration: 2.0,
    });
    let project = scene.project();
    let mut stage = Stage::new();
    let before = stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);

    // Paused on the last frame of B, then stepped onto the first of C.
    fixture(active(&project, frame(119)) == [b], "only B is painted");
    let painted = stage.paint(&project, frame(119), false);
    assert_eq!(
        stage.seen(&painted, &project, b),
        before.seen(b),
        "CONTROL: B lost its picture on its last frame"
    );
    fixture(active(&project, frame(120)) == [c], "only C is painted");
    let painted = stage.paint(&project, frame(120), false);
    assert_eq!(
        stage.seen(&painted, &project, c),
        before.seen(b),
        "CONTROL: a clip placed to carry on from another is painted with the left; \
         that clip's frame is the right"
    );
}

#[test]
fn the_source_monitor_turned_to_another_asset_shows_nothing_of_the_first() {
    let Pair { mut scene, .. } = overlapping();
    let other = video("/media/other.mp4");
    scene.edit(Command::AddAssets {
        assets: vec![other.clone()],
    });
    let mut stage = Stage::new();
    stage.paint_source(&scene.asset);
    stage.deliver_source(&scene.asset, GREEN);
    let painted = stage.paint_source(&scene.asset);
    fixture(
        painted.quads.len() == 1,
        "the source monitor shows its frame",
    );
    let first = stage.textures.asked(Viewing::Source);

    let painted = stage.paint_source(&other);
    assert!(
        painted.quads.is_empty(),
        "CONTROL: the source monitor paints another asset with a picture of the \
         one it showed before"
    );
    // A frame of the first asset that was still decoding arrives now.
    let Some((consumer, size)) = first else {
        panic!("FIXTURE: the source monitor kept no layer");
    };
    let late = solid(AMBER);
    let time = stage.textures.admitted(consumer);
    let taken = stage.textures.accept(
        &stage.ctx,
        Some(&scene.asset),
        Some(consumer),
        scene.asset.id,
        time,
        size,
        &late,
    );
    assert!(
        !taken && !stage.arrive(Viewing::Source, &scene.asset, AMBER),
        "CONTROL: a frame of the asset shown before was taken for the one shown now"
    );
    let painted = stage.paint_source(&other);
    assert!(
        painted.quads.is_empty(),
        "CONTROL: a late frame of the asset shown before is painted"
    );
    stage.deliver_source(&other, BLUE);
    let painted = stage.paint_source(&other);
    let [source] = painted.quads.as_slice() else {
        panic!("CONTROL: the source monitor did not paint exactly one picture");
    };
    assert_eq!(
        stage.pictures.get(&source.texture),
        Some(&shown(BLUE)),
        "CONTROL: the source monitor does not show the frame decoded for it"
    );
}

#[test]
fn a_frame_decoded_before_the_project_was_replaced_is_not_painted_after() {
    let Pair { scene, a, b } = overlapping();
    let project = scene.project();
    let mut stage = Stage::new();
    stage.settle(&project, frame(59), &scene.asset, &[a, b], &SETTLED);
    let asked = [a, b].map(|clip| stage.textures.asked(Viewing::Clip(clip)));

    // What `App::replace_editor` does to the textures; the same clips are
    // then painted, as when the same file is opened again.
    stage.textures.clear();
    stage.paint(&project, frame(59), false);
    for old in asked {
        let Some((consumer, size)) = old else {
            panic!("FIXTURE: a settled clip kept no layer");
        };
        let late = solid(GREEN);
        let time = stage.textures.admitted(consumer);
        let taken = stage.textures.accept(
            &stage.ctx,
            Some(&scene.asset),
            Some(consumer),
            scene.asset.id,
            time,
            size,
            &late,
        );
        assert!(
            !taken,
            "CONTROL: a frame asked for before the project was replaced was taken"
        );
    }
    let painted = stage.paint(&project, frame(59), false);
    assert!(
        painted.quads.is_empty(),
        "CONTROL: a layer is painted with a frame asked for before the project was \
         replaced"
    );
}

// ---- Issue 13: what the layers add up to ------------------------------------

/// Nested here for the stage and the scenes above, which it paints with.
#[path = "working_set_tests.rs"]
mod working_set_tests;

// ---- Issue 16: what the media worker is asked for ---------------------------

/// Nested here beside the tests of what the layers add up to. It composes
/// layers against a real media service and paints nothing.
#[path = "media_consumer_tests.rs"]
mod media_consumer_tests;
