//! The export's pictures, read back from a real device: what the frame
//! renderer draws of a project beside what the viewer composites of it, and
//! where the canvas lands in the output.
//!
//! Pictures come from a fake source of solid and quartered frames. Expected
//! colours are worked out in plain arithmetic: sRGB bytes to linear light,
//! the operation, and back, with two code values of slack for a device's
//! rounding. With no adapter a test prints a line beginning `SKIP:` and
//! passes.
use super::{FrameRenderer, MAX_DECODE_SIDE, Pictures, decode_size, export_title_scale, place_row};
use crate::{
    compose::{
        ComposeBudget, Draw, Job,
        assemble::{self},
        gpu_tests::{composer_beside_device, device},
    },
    textures::Input,
};
use editor_core::{AnimTrack, Ease};
use editor_core::{
    Asset, AssetId, BlendMode, Clip, ClipId, Effect, EffectKind, Element, ElementKind, FrameRate,
    Keyframe, Mask, MaskOp, MaskShape, MediaKind, ParamKey, Project, RenderPlan, Time, Track,
    TrackId, TrackKind,
    export::{Fit, Geometry},
    resolve,
};
use editor_media::Frame;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use uuid::Uuid;

const NIGHT: [u8; 4] = [10, 20, 30, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [220, 40, 40, 255];
const BLUE: [u8; 4] = [40, 80, 220, 255];
const GREEN: [u8; 4] = [40, 190, 90, 255];
const AMBER: [u8; 4] = [230, 170, 30, 255];
const CREAM: [u8; 4] = [200, 200, 120, 255];
const SIDE: u32 = 64;

fn linear(byte: u8) -> f64 {
    let c = f64::from(byte) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn encoded(light: f64) -> u8 {
    let c = light.clamp(0.0, 1.0);
    let c = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round() as u8
}

/// `over` multiplied into `under`, both opaque, in linear light.
fn multiplied(over: [u8; 4], under: [u8; 4]) -> [u8; 4] {
    let channel = |at: usize| encoded(linear(over[at]) * linear(under[at]));
    [channel(0), channel(1), channel(2), 255]
}

fn solid(colour: [u8; 4], [width, height]: [u32; 2]) -> Frame {
    Frame {
        width,
        height,
        rgba: colour.repeat((width * height) as usize).into(),
    }
}

/// Four colours, one a quarter: left top, right top, left bottom, right
/// bottom.
fn quartered(colours: [[u8; 4]; 4]) -> Frame {
    let mut rgba = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16 {
        for x in 0..16 {
            rgba.extend_from_slice(&colours[usize::from(y >= 8) * 2 + usize::from(x >= 8)]);
        }
    }
    Frame {
        width: 16,
        height: 16,
        rgba: rgba.into(),
    }
}

/// A project built clip by clip, with the frame each asset decodes to.
struct Film {
    project: Project,
    frames: HashMap<AssetId, Frame>,
}

impl Film {
    fn new(canvas: [u32; 2]) -> Self {
        let mut project = Project::new("Export");
        project.settings.width = canvas[0];
        project.settings.height = canvas[1];
        project.settings.background = NIGHT;
        Self {
            project,
            frames: HashMap::new(),
        }
    }

    /// A video of `size` that decodes to `frame` whatever is asked.
    fn video(&mut self, size: [u32; 2], frame: Frame) -> Asset {
        let mut asset = Asset::new(
            PathBuf::from(format!("/nowhere/{}.mp4", Uuid::new_v4())),
            MediaKind::Video,
            Time::from_seconds(10.0),
        );
        asset.width = size[0];
        asset.height = size[1];
        asset.frame_rate = Some(FrameRate::default());
        asset.file_size = 1000;
        self.frames.insert(asset.id, frame);
        Arc::make_mut(&mut self.project.assets).insert(asset.id, asset.clone());
        asset
    }

    fn track(&self, kind: TrackKind) -> TrackId {
        (self.project.tracks.iter())
            .find(|track| track.kind == kind)
            .map(|track| track.id)
            .expect("a new project has a track of each kind")
    }

    /// A video track over every other.
    fn top_track(&mut self) -> TrackId {
        let track = Track::new("Video over", TrackKind::Video);
        let id = track.id;
        self.project.tracks.push(Arc::new(track));
        id
    }

    fn add(&mut self, track: TrackId, clip: Clip) -> ClipId {
        let id = clip.id;
        let track = (self.project.tracks.iter_mut())
            .find(|held| held.id == track)
            .expect("the track is the project's");
        let track = Arc::make_mut(track);
        track.clips.push(clip);
        track.clips.sort_by_key(|clip| clip.start);
        id
    }

    /// A clip of `asset` from nought for four seconds.
    fn clip(&mut self, track: TrackId, asset: &Asset, shape: impl FnOnce(&mut Clip)) -> ClipId {
        let mut clip = Clip::from_asset(asset, Time::ZERO);
        clip.duration = Time::from_seconds(4.0);
        shape(&mut clip);
        self.add(track, clip)
    }

    /// The project, checked as the editor would check it.
    fn done(&self) -> &Project {
        if let Err(error) = self.project.validate() {
            panic!("FIXTURE: the project is not valid: {error}");
        }
        &self.project
    }

    fn pictures(&self) -> Fake {
        Fake {
            frames: self.frames.clone(),
            asked: Vec::new(),
        }
    }
}

/// Pictures from memory, each asking remembered.
struct Fake {
    frames: HashMap<AssetId, Frame>,
    asked: Vec<(ClipId, AssetId, Time, [u32; 2])>,
}

impl Pictures for Fake {
    fn picture(
        &mut self,
        lane: ClipId,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<Frame, String> {
        self.asked.push((lane, asset.id, time, size));
        (self.frames.get(&asset.id).cloned()).ok_or_else(|| format!("no picture of {}", asset.name))
    }
}

/// The bytes of a drawn picture.
struct Image {
    width: u32,
    bytes: Vec<u8>,
}

impl Image {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let from = ((y * self.width + x) * 4) as usize;
        std::array::from_fn(|channel| self.bytes[from + channel])
    }

    #[track_caller]
    fn is(&self, x: u32, y: u32, expected: [u8; 4], what: &str) {
        let actual = self.at(x, y);
        let near = (actual.iter().zip(&expected)).all(|(a, e)| a.abs_diff(*e) <= 2);
        assert!(
            near,
            "{what}: pixel ({x}, {y}) is {actual:?}, expected {expected:?}"
        );
    }
}

/// Draws `project` at `at` through an export's renderer at `output`.
fn exported(
    device: &(eframe::wgpu::Device, eframe::wgpu::Queue),
    project: &Project,
    output: [u32; 2],
    fit: Fit,
    at: Time,
    pictures: &mut Fake,
) -> Image {
    let canvas = [project.settings.width, project.settings.height];
    let geometry = Geometry::new(canvas, output, fit);
    let mut renderer =
        FrameRenderer::new(device.0.clone(), device.1.clone(), geometry, project, None)
            .expect("the renderer is made");
    let bytes = renderer
        .render(project, at, pictures)
        .expect("the frame is drawn")
        .to_vec();
    assert_eq!(bytes.len(), (output[0] * output[1] * 4) as usize);
    Image {
        width: output[0],
        bytes,
    }
}

/// The viewer's jobs for the composited runs of `plan`, made as the viewer
/// makes them, from the frames `pictures` holds under one serial an asset.
fn viewer_jobs<'a>(
    project: &'a Project,
    plan: &'a RenderPlan,
    pictures: &'a [(AssetId, u64, Frame)],
) -> Vec<Job<'a>> {
    let canvas = [
        project.settings.width as f32,
        project.settings.height as f32,
    ];
    let mut jobs = Vec::new();
    for run in crate::compose::runs(plan) {
        let crate::compose::Run::Composite(range) = run else {
            continue;
        };
        let draws = plan.layers[range.clone()]
            .iter()
            .filter_map(|layer| {
                let editor_core::LayerSource::Media { asset_id, .. } = &layer.source else {
                    return None;
                };
                let asset = project.assets.get(asset_id)?;
                let (_, serial, frame) = pictures.iter().find(|(id, ..)| id == asset_id)?;
                let source =
                    assemble::source_size(asset, Some([frame.width, frame.height]), canvas);
                Some(Draw {
                    layer,
                    place: assemble::layer_box(&layer.transform, source, canvas)
                        .placement(canvas, layer.transform.rotation),
                    input: Input {
                        serial: *serial,
                        frame,
                        time: Time::ZERO,
                    },
                    plates: Vec::new(),
                    luts: Vec::new(),
                    steady: None,
                })
            })
            .collect();
        jobs.push(Job {
            canvas: plan.canvas,
            background: assemble::background(plan, &range),
            draws,
        });
    }
    jobs
}

#[test]
fn a_composited_plan_is_drawn_as_the_viewer_composites_it() {
    let Some(mut rig) = composer_beside_device(ComposeBudget::PRODUCTION) else {
        return;
    };
    let (composer, device, queue) = &mut *rig;
    let mut film = Film::new([SIDE, SIDE]);
    let under = film.video([16, 16], quartered([RED, BLUE, GREEN, AMBER]));
    let over = film.video([16, 16], solid(CREAM, [16, 16]));
    let video = film.track(TrackKind::Video);
    film.clip(video, &under, |_| {});
    let top = film.top_track();
    film.clip(top, &over, |clip| {
        clip.blend = BlendMode::Multiply;
        clip.transform.opacity = 0.75;
        clip.transform.scale = [0.5, 0.5];
        clip.transform.rotation = 30.0;
        clip.transform.position = [6.0, -4.0];
        clip.masks = vec![Mask {
            id: Uuid::new_v4(),
            shape: MaskShape::Ellipse,
            op: MaskOp::Add,
            invert: false,
            center: [0.5, 0.5],
            size: [0.45, 0.45],
            rotation: 0.0,
            feather: 0.1,
            roundness: 0.0,
        }];
    });
    let project = film.done();
    let at = Time::from_seconds(1.0);
    let plan = resolve(project, at, &[]);
    let pictures = [
        (under.id, 1, film.frames[&under.id].clone()),
        (over.id, 2, film.frames[&over.id].clone()),
    ];
    let jobs = viewer_jobs(project, &plan, &pictures);
    assert_eq!(jobs.len(), 1, "one composited run");
    composer.frame([SIDE, SIDE], &jobs);
    let (size, viewed) = composer.read(0).expect("the viewer composited the run");
    assert_eq!(size, [SIDE, SIDE]);

    let mut fake = film.pictures();
    let image = exported(
        &(device.clone(), queue.clone()),
        project,
        [SIDE, SIDE],
        Fit::Letterbox,
        at,
        &mut fake,
    );
    let viewed = Image {
        width: SIDE,
        bytes: viewed,
    };
    for y in 0..SIDE {
        for x in 0..SIDE {
            image.is(x, y, viewed.at(x, y), "the export beside the viewer");
        }
    }
    // Something of each was drawn: the parity is not of two blank canvases.
    image.is(8, 8, RED, "the lower layer, outside the upper one");
    assert_ne!(image.at(32, 30), RED, "the upper layer multiplies in");
}

#[test]
fn a_plain_plan_is_drawn_whole_on_the_canvas_colour() {
    let Some(device) = device() else { return };
    let mut film = Film::new([SIDE, SIDE]);
    let back = film.video([16, 16], solid(RED, [16, 16]));
    let front = film.video([16, 16], solid(GREEN, [16, 16]));
    let video = film.track(TrackKind::Video);
    film.clip(video, &back, |clip| clip.transform.scale = [0.5, 1.0]);
    let top = film.top_track();
    film.clip(top, &front, |clip| {
        clip.transform.scale = [0.25, 0.25];
        clip.transform.opacity = 0.5;
    });
    let project = film.done();
    assert!(
        resolve(project, Time::ZERO, &[])
            .layers
            .iter()
            .all(|layer| layer.is_plain()),
        "every layer is plain"
    );
    let mut fake = film.pictures();
    let image = exported(
        &device,
        project,
        [SIDE, SIDE],
        Fit::Letterbox,
        Time::ZERO,
        &mut fake,
    );
    image.is(2, 32, NIGHT, "the canvas's colour beside the lower layer");
    image.is(20, 5, RED, "the lower layer");
    // Half of green over red, in linear light: the compositor's, not the
    // mesh path's gamma-space mix.
    let half = std::array::from_fn(|at| {
        if at == 3 {
            255
        } else {
            encoded(0.5 * linear(GREEN[at]) + 0.5 * linear(RED[at]))
        }
    });
    image.is(32, 32, half, "half the upper layer over the lower");
}

#[test]
fn a_clip_is_asked_for_its_source_frame_at_the_size_it_is_drawn() {
    let Some(device) = device() else { return };
    let mut film = Film::new([SIDE, SIDE]);
    let asset = film.video([1920, 1080], solid(RED, [16, 9]));
    let video = film.track(TrackKind::Video);
    let clip = film.clip(video, &asset, |clip| {
        clip.source_in = Time::from_seconds(2.0);
        clip.transform.scale = [0.5, 0.5];
    });
    let project = film.done();
    let mut fake = film.pictures();
    let at = Time::from_seconds(1.0);
    exported(
        &device,
        project,
        [SIDE * 2, SIDE * 2],
        Fit::Letterbox,
        at,
        &mut fake,
    );
    let held = project.clip(clip).expect("the clip").1;
    // Half the canvas, drawn at twice the canvas: the picture's own 128
    // pixels across, cut to the asset's shape.
    assert_eq!(
        fake.asked,
        vec![(
            clip,
            asset.id,
            editor_core::render::source_frame(project, &asset, held, at),
            [64, 36]
        )]
    );
}

#[test]
fn a_title_is_drawn_where_the_viewer_puts_it_over_the_media() {
    let Some(device) = device() else { return };
    let mut film = Film::new([SIDE, SIDE]);
    let media = film.video([16, 16], solid(GREEN, [16, 16]));
    let video = film.track(TrackKind::Video);
    film.clip(video, &media, |_| {});
    let text = film.track(TrackKind::Text);
    // Ten across in red, then ten in blue: which way it is flipped shows.
    let element = |colour: [u8; 4], left: f32| {
        Element::new(
            "Half",
            ElementKind::Rect {
                color: colour,
                radius: 0.0,
                stroke: None,
            },
            [left, 0.0, 10.0, 10.0],
        )
    };
    let graphic = |transform: editor_core::Transform| {
        let mut clip = Clip::graphic(
            "Badge",
            [20.0, 10.0],
            vec![element(RED, 0.0), element(BLUE, 10.0)],
            Time::ZERO,
            Time::from_seconds(4.0),
        );
        clip.transform = transform;
        clip
    };
    let placed = editor_core::Transform {
        position: [10.0, -8.0],
        ..editor_core::Transform::default()
    };
    film.add(text, graphic(placed.clone()));
    let project = film.done();
    let mut fake = film.pictures();
    let image = exported(
        &device,
        project,
        [SIDE, SIDE],
        Fit::Letterbox,
        Time::ZERO,
        &mut fake,
    );
    // The anchor is the canvas's centre moved by the title's place; its
    // design box is twenty by ten about it.
    let anchor = [32.0 + 10.0, 32.0 - 8.0];
    let point = |transform: &editor_core::Transform, x: f32, y: f32| {
        let [x, y] = assemble::title_point(anchor, 1.0, transform, x, y);
        (x.floor() as u32, y.floor() as u32)
    };
    let (x, y) = point(&placed, -5.0, 0.0);
    image.is(x, y, RED, "the left half of the title");
    let (x, y) = point(&placed, 5.0, 0.0);
    image.is(x, y, BLUE, "the right half of the title");
    image.is(30, 24, GREEN, "left of the title");
    image.is(54, 24, GREEN, "right of the title");
    image.is(42, 17, GREEN, "over the title");
    image.is(42, 31, GREEN, "under the title");
    assert_eq!(fake.asked.len(), 1, "a title is no decoded picture");

    // Mirrored, enlarged and turned: where the viewer's quad puts each
    // half.
    let turned = editor_core::Transform {
        position: [10.0, -8.0],
        scale: [1.5, 1.0],
        flip_x: true,
        rotation: 90.0,
        ..editor_core::Transform::default()
    };
    let track = (film.project.tracks.iter_mut())
        .find(|track| track.id == text)
        .expect("the text track");
    Arc::make_mut(track).clips[0].transform = turned.clone();
    let project = film.done();
    let image = exported(
        &device,
        project,
        [SIDE, SIDE],
        Fit::Letterbox,
        Time::ZERO,
        &mut fake,
    );
    let (x, y) = point(&turned, -5.0, 0.0);
    image.is(x, y, RED, "the left half, mirrored and turned");
    let (x, y) = point(&turned, 5.0, 0.0);
    image.is(x, y, BLUE, "the right half, mirrored and turned");
    image.is(42, 2, GREEN, "past the turned title's end");
}

#[test]
fn a_run_over_a_title_blends_with_the_media_between_and_nothing_lower() {
    let Some(device) = device() else { return };
    let mut film = Film::new([SIDE, SIDE]);
    let white = film.video([16, 16], solid(WHITE, [16, 16]));
    let green = film.video([16, 16], solid(GREEN, [16, 16]));
    let red = film.video([16, 16], solid(RED, [16, 16]));
    let video = film.track(TrackKind::Video);
    film.clip(video, &white, |_| {});
    let fill = Element::new(
        "Fill",
        ElementKind::Rect {
            color: BLUE,
            radius: 0.0,
            stroke: None,
        },
        [0.0, 0.0, 64.0, 64.0],
    );
    let text = film.track(TrackKind::Text);
    film.add(
        text,
        Clip::graphic(
            "Fill",
            [64.0, 64.0],
            vec![fill],
            Time::ZERO,
            Time::from_seconds(4.0),
        ),
    );
    // Over the title: green across the left half, then red multiplied over
    // the whole canvas.
    let between = film.top_track();
    film.clip(between, &green, |clip| {
        clip.transform.scale = [0.5, 1.0];
        clip.transform.position = [-16.0, 0.0];
    });
    let top = film.top_track();
    film.clip(top, &red, |clip| clip.blend = BlendMode::Multiply);
    let project = film.done();
    let plan = resolve(project, Time::ZERO, &[]);
    assert_eq!(
        crate::compose::runs(&plan),
        vec![
            crate::compose::Run::Plain(0..1),
            crate::compose::Run::Title(1),
            crate::compose::Run::Composite(2..4),
        ]
    );
    let mut fake = film.pictures();
    let image = exported(
        &device,
        project,
        [SIDE, SIDE],
        Fit::Letterbox,
        Time::ZERO,
        &mut fake,
    );
    image.is(
        16,
        32,
        multiplied(RED, GREEN),
        "red multiplied into the green",
    );
    // Not red multiplied into the title's blue, nor into the white under it.
    image.is(
        48,
        32,
        RED,
        "red over nothing of its run, laid over the title",
    );
}

#[test]
fn letterbox_bars_are_black_and_fill_cuts_the_canvas_evenly() {
    let Some(device) = device() else { return };
    let mut film = Film::new([SIDE, SIDE]);
    let asset = film.video([16, 16], quartered([RED, BLUE, GREEN, AMBER]));
    let video = film.track(TrackKind::Video);
    film.clip(video, &asset, |_| {});
    let project = film.done();

    let mut fake = film.pictures();
    let wide = exported(
        &device,
        project,
        [96, 64],
        Fit::Letterbox,
        Time::ZERO,
        &mut fake,
    );
    wide.is(4, 32, BLACK, "the bar on the left");
    wide.is(91, 32, BLACK, "the bar on the right");
    wide.is(16 + 16, 16, RED, "the canvas's left top");
    wide.is(16 + 48, 16, BLUE, "its right top");
    wide.is(16 + 16, 48, GREEN, "its left bottom");
    wide.is(16 + 48, 48, AMBER, "its right bottom");

    let flat = exported(&device, project, [64, 32], Fit::Fill, Time::ZERO, &mut fake);
    flat.is(16, 0, RED, "the first row is the canvas's seventeenth");
    flat.is(48, 0, BLUE, "and across it");
    flat.is(16, 31, GREEN, "the last row is the canvas's forty-eighth");
    flat.is(48, 31, AMBER, "and across it");
}

#[test]
fn a_4k_output_is_drawn_at_its_full_size() {
    let Some(device) = device() else { return };
    let longest = device.0.limits().max_texture_dimension_2d;
    if longest < 3840 {
        println!("SKIP: this device draws {longest} a side, under 4K");
        return;
    }
    let mut film = Film::new([1920, 1080]);
    let back = film.video([1920, 1080], solid(RED, [32, 18]));
    let front = film.video([1920, 1080], solid(GREEN, [32, 18]));
    let video = film.track(TrackKind::Video);
    film.clip(video, &back, |_| {});
    let top = film.top_track();
    film.clip(top, &front, |clip| clip.transform.scale = [0.25, 0.25]);
    let project = film.done();
    let mut fake = film.pictures();
    let image = exported(
        &device,
        project,
        [3840, 2160],
        Fit::Letterbox,
        Time::ZERO,
        &mut fake,
    );
    // The upper clip is 480 by 270 of the canvas: 960 by 540 of the
    // output, about its centre, with no step down.
    image.is(1445, 1080, GREEN, "inside the left edge of the upper clip");
    image.is(1435, 1080, RED, "outside it");
    image.is(2394, 1345, GREEN, "inside its right bottom corner");
    image.is(2405, 1355, RED, "outside it");
    image.is(0, 0, RED, "the output's corner");
    let sizes: Vec<[u32; 2]> = fake.asked.iter().map(|(.., size)| *size).collect();
    assert_eq!(
        sizes,
        vec![[1920, 1080], [960, 540]],
        "each clip decodes what it covers of the output"
    );
}

#[test]
fn a_frame_stopped_while_it_is_read_fails_and_the_next_is_drawn() {
    let Some(device) = device() else { return };
    let mut film = Film::new([SIDE, SIDE]);
    let asset = film.video([16, 16], solid(AMBER, [16, 16]));
    let video = film.track(TrackKind::Video);
    film.clip(video, &asset, |_| {});
    let project = film.done();
    let geometry = Geometry::new([SIDE, SIDE], [SIDE, SIDE], Fit::Letterbox);
    let mut renderer =
        FrameRenderer::new(device.0.clone(), device.1.clone(), geometry, project, None)
            .expect("the renderer is made");
    let stop = Arc::new(AtomicBool::new(true));
    renderer.set_stop(Arc::clone(&stop));
    let mut fake = film.pictures();
    let stopped = renderer.render(project, Time::ZERO, &mut fake);
    assert!(stopped.is_err(), "a stopped frame fails");
    stop.store(false, std::sync::atomic::Ordering::Relaxed);
    let bytes = renderer
        .render(project, Time::ZERO, &mut fake)
        .expect("the next frame is drawn")
        .to_vec();
    let image = Image { width: SIDE, bytes };
    image.is(32, 32, AMBER, "the frame drawn after");
    assert_eq!(fake.asked.len(), 1, "the picture was asked for once");
}

#[test]
fn a_stabilized_clip_without_its_camera_path_is_drawn_as_it_is_and_said() {
    let Some(device) = device() else { return };
    let mut film = Film::new([SIDE, SIDE]);
    let asset = film.video([16, 16], solid(AMBER, [16, 16]));
    let video = film.track(TrackKind::Video);
    film.clip(video, &asset, |clip| {
        clip.effects = vec![Effect::new(EffectKind::Stabilize)];
    });
    let project = film.done();
    let geometry = Geometry::new([SIDE, SIDE], [SIDE, SIDE], Fit::Letterbox);
    let folder = Scratch::new();
    let mut renderer = FrameRenderer::new(
        device.0.clone(),
        device.1.clone(),
        geometry,
        project,
        Some(folder.0.clone()),
    )
    .expect("the renderer is made");
    let mut fake = film.pictures();
    for at in [Time::ZERO, Time::from_seconds(1.0)] {
        let bytes = renderer
            .render(project, at, &mut fake)
            .expect("the frame is drawn")
            .to_vec();
        Image { width: SIDE, bytes }.is(32, 32, AMBER, "the clip, unsteadied");
    }
    assert_eq!(renderer.warnings().len(), 1, "said once");
    assert!(
        renderer.warnings()[0].contains(&asset.name),
        "{:?}",
        renderer.warnings()
    );
}

#[test]
fn a_lookup_table_is_read_from_its_file_and_a_missing_one_is_said() {
    let Some(device) = device() else { return };
    let folder = Scratch::new();
    let path = folder.0.join("invert.cube");
    let mut text = String::from("LUT_3D_SIZE 2\n");
    for blue in 0..2 {
        for green in 0..2 {
            for red in 0..2 {
                text.push_str(&format!("{} {} {}\n", 1 - red, 1 - green, 1 - blue));
            }
        }
    }
    std::fs::write(&path, text).expect("the table is written");
    let mut film = Film::new([SIDE, SIDE]);
    let asset = film.video([16, 16], solid(RED, [16, 16]));
    let table = |path: PathBuf| {
        let mut table = Asset::new(path, MediaKind::Lut, Time(1));
        table.width = 2;
        table.file_size = 10;
        table
    };
    let (found, missing) = (table(path), table(folder.0.join("missing.cube")));
    for lut in [&found, &missing] {
        Arc::make_mut(&mut film.project.assets).insert(lut.id, lut.clone());
    }
    let lookup = |lut: &Asset| {
        let mut effect = Effect::new(EffectKind::Lut);
        effect.asset_id = Some(lut.id);
        effect
    };
    let video = film.track(TrackKind::Video);
    film.clip(video, &asset, |clip| {
        clip.effects = vec![lookup(&found), lookup(&missing)];
    });
    let project = film.done();
    let mut fake = film.pictures();
    let geometry = Geometry::new([SIDE, SIDE], [SIDE, SIDE], Fit::Letterbox);
    let mut renderer =
        FrameRenderer::new(device.0.clone(), device.1.clone(), geometry, project, None)
            .expect("the renderer is made");
    let bytes = renderer
        .render(project, Time::ZERO, &mut fake)
        .expect("the frame is drawn")
        .to_vec();
    Image { width: SIDE, bytes }.is(32, 32, [35, 215, 215, 255], "red, inverted");
    assert_eq!(renderer.warnings().len(), 1, "{:?}", renderer.warnings());
    assert!(renderer.warnings()[0].contains("missing.cube"));
}

#[test]
fn a_clip_decodes_what_it_covers_at_its_largest_and_never_more() {
    let mut asset = Asset::new(
        PathBuf::from("/nowhere/wide.mp4"),
        MediaKind::Video,
        Time::from_seconds(10.0),
    );
    asset.width = 3840;
    asset.height = 2160;
    let mut clip = Clip::from_asset(&asset, Time::ZERO);
    clip.transform.scale = [0.5, 0.5];
    let hd = [1920, 1080];
    assert_eq!(decode_size(&asset, &clip, hd, hd), [960, 540]);
    assert_eq!(
        decode_size(&asset, &clip, hd, [3840, 2160]),
        [1920, 1080],
        "drawn twice the canvas"
    );
    // Keyed up to twice its size at some moment: decoded whole, at the
    // asset's own size and no more.
    clip.animation = vec![AnimTrack {
        target: None,
        key: ParamKey::ScaleX,
        keys: vec![
            Keyframe {
                at: Time::ZERO,
                value: 0.5,
                ease: Ease::Linear,
            },
            Keyframe {
                at: Time::from_seconds(2.0),
                value: 2.0,
                ease: Ease::Linear,
            },
        ],
    }];
    assert_eq!(decode_size(&asset, &clip, hd, hd), [3840, 2160]);

    // Cropped to its middle quarter, which fills the canvas: every pixel
    // of the picture is needed.
    let mut cropped = Clip::from_asset(&asset, Time::ZERO);
    cropped.transform.crop = [0.25, 0.25, 0.25, 0.25];
    assert_eq!(decode_size(&asset, &cropped, hd, hd), [3840, 2160]);

    // A small picture is never enlarged; an enormous one is held to the
    // longest side decoded.
    asset.width = 640;
    asset.height = 360;
    let plain = Clip::from_asset(&asset, Time::ZERO);
    assert_eq!(decode_size(&asset, &plain, hd, hd), [640, 360]);
    asset.width = 16000;
    asset.height = 1000;
    assert_eq!(
        decode_size(&asset, &plain, [16000, 1000], [16000, 1000])[0],
        MAX_DECODE_SIDE
    );
}

#[test]
fn a_title_is_drawn_with_at_least_the_pixels_it_covers() {
    // Scale one on a canvas drawn at its size is drawn at one.
    assert_eq!(export_title_scale(1.0, [1920, 1080], [1920, 1080]), 1.0);
    // Drawn at twice the canvas: twice the pixels.
    assert_eq!(export_title_scale(1.0, [1920, 1080], [3840, 2160]), 2.0);
    // Between the steps titles are drawn at: the step above.
    let between = export_title_scale(1.3, [1920, 1080], [1920, 1080]);
    assert!(between >= 1.3, "{between}");
    assert!(between < 1.3 * 1.1, "{between}");
}

#[test]
fn rows_are_placed_in_the_output_and_cut_to_it() {
    let geometry = Geometry {
        output: [4, 2],
        drawn: [2, 2],
        offset: [1, 0],
    };
    let mut output = [0, 0, 0, 255].repeat(8);
    place_row(&mut output, &geometry, 0, &[9, 9, 9, 7, 8, 8, 8, 7]);
    assert_eq!(
        output[..16],
        [0, 0, 0, 255, 9, 9, 9, 255, 8, 8, 8, 255, 0, 0, 0, 255],
        "opaque, one texel in"
    );
    let filled = Geometry {
        output: [2, 1],
        drawn: [4, 3],
        offset: [-1, -1],
    };
    let mut output = vec![0; 8];
    place_row(&mut output, &filled, 0, &[1; 16]);
    assert_eq!(output, vec![0; 8], "the row above the output");
    place_row(
        &mut output,
        &filled,
        1,
        &[1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4],
    );
    assert_eq!(
        output,
        vec![2, 2, 2, 255, 3, 3, 3, 255],
        "the middle of the row"
    );
}

/// A directory of the test's own, removed with it.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("aster-export-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("the scratch directory is made");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
