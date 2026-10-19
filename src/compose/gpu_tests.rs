//! The compositor's pixels, read back from a real device.
//!
//! Each test draws a 64×64 canvas from solid or quartered pictures and reads
//! the bytes the interface would sample. What is expected is worked out here
//! in plain arithmetic from the definitions: sRGB bytes to linear light, the
//! operation, and back. Two code values of slack cover a device's rounding.
//!
//! With no adapter a test prints a line beginning `SKIP:` and passes. Set
//! `NEPTUNE_VIDEO_TEST_FALLBACK_ADAPTER` to run on the software adapter alone.
use super::{
    ComposeBudget, Composer, Draw, Job, MAX_CANVASES, Placement, gpu::Gpu, lut::Lut, target_size,
};
use crate::textures::{Input, LAYER_IDLE_FRAMES};
use editor_core::{BlendMode, Layer, LayerSource, Mask, MaskOp, MaskShape, Time, Transform};
use editor_media::{Cube, Frame};
use eframe::{
    egui::{self, mutex::RwLock, vec2},
    egui_wgpu::{self, RenderState},
    wgpu,
};
use std::{
    ops::{Deref, DerefMut},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use uuid::Uuid;

const SIDE: u32 = 64;
pub(super) const CANVAS: [u32; 2] = [SIDE, SIDE];
pub(super) const NIGHT: [u8; 4] = [10, 20, 30, 255];
pub(super) const BLACK: [u8; 4] = [0, 0, 0, 255];
pub(super) const WHITE: [u8; 4] = [255, 255, 255, 255];
pub(super) const RED: [u8; 4] = [220, 40, 40, 255];
pub(super) const BLUE: [u8; 4] = [40, 80, 220, 255];
pub(super) const GREEN: [u8; 4] = [40, 190, 90, 255];
pub(super) const AMBER: [u8; 4] = [230, 170, 30, 255];

struct Rig {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// A device to draw on: the adapter the host prefers, else its software one.
fn open() -> Option<Rig> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let software_only = std::env::var_os("NEPTUNE_VIDEO_TEST_FALLBACK_ADAPTER").is_some();
    let mut why = String::from("no adapter was asked for");
    for force_fallback_adapter in [false, true] {
        if software_only && !force_fallback_adapter {
            continue;
        }
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter,
            ..Default::default()
        }));
        let adapter = match adapter {
            Ok(adapter) => adapter,
            Err(error) => {
                why = error.to_string();
                continue;
            }
        };
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) {
            Ok((device, queue)) => {
                let info = adapter.get_info();
                println!(
                    "ADAPTER: {} ({:?}, {:?})",
                    info.name, info.backend, info.device_type
                );
                return Some(Rig {
                    instance,
                    adapter,
                    device,
                    queue,
                });
            }
            Err(error) => why = error.to_string(),
        }
    }
    println!("SKIP: no wgpu device to read pixels back from: {why}");
    None
}

/// Something made on the tests' device, and the turn to use it.
pub(crate) struct Turn<T> {
    made: T,
    _turn: MutexGuard<'static, ()>,
}

impl<T> Deref for Turn<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.made
    }
}

impl<T> DerefMut for Turn<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.made
    }
}

/// The one device these tests share, a test at a time: it is opened once and
/// never closed, because drivers have been seen to crash when several
/// devices are opened and closed at once by the threads of a test run.
fn rig() -> Option<Turn<&'static Rig>> {
    static RIG: OnceLock<Option<Rig>> = OnceLock::new();
    static TURN: Mutex<()> = Mutex::new(());
    let turn = TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let rig = RIG.get_or_init(open).as_ref();
    if rig.is_none() {
        println!("SKIP: no wgpu device to read pixels back from");
    }
    Some(Turn {
        made: rig?,
        _turn: turn,
    })
}

/// The tests' device and queue, for what makes a compositor of its own: an
/// export's renderer.
pub(crate) fn device() -> Option<Turn<(wgpu::Device, wgpu::Queue)>> {
    let Turn { made: rig, _turn } = rig()?;
    Some(Turn {
        made: (rig.device.clone(), rig.queue.clone()),
        _turn,
    })
}

pub(super) fn gpu() -> Option<Turn<Gpu>> {
    let Turn { made: rig, _turn } = rig()?;
    match Gpu::new(rig.device.clone(), rig.queue.clone()) {
        Ok(made) => Some(Turn { made, _turn }),
        Err(error) => panic!("the compositor's pipelines were refused: {error}"),
    }
}

pub(super) fn linear(byte: u8) -> f64 {
    let c = f64::from(byte) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub(super) fn encoded(light: f64) -> u8 {
    let c = light.clamp(0.0, 1.0);
    let c = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round() as u8
}

/// `over` at `cover` of a pixel, the rest `under`, both opaque.
fn covered(over: [u8; 4], under: [u8; 4], cover: f64) -> [u8; 4] {
    let mix = |channel: usize| {
        encoded(linear(over[channel]) * cover + linear(under[channel]) * (1.0 - cover))
    };
    [mix(0), mix(1), mix(2), 255]
}

/// A picture with the layer it is drawn as.
pub(super) struct Pic {
    pub(super) layer: Layer,
    pub(super) place: Placement,
    pub(super) frame: Frame,
    /// The name its frame is uploaded under: no two pictures share one.
    pub(super) serial: u64,
    /// The clean plates of its difference mattes: effect, serial and frame.
    pub(super) plates: Vec<(Uuid, u64, Frame)>,
    /// The tables of its lookups: effect, serial and table.
    pub(super) luts: Vec<(Uuid, u64, Arc<Cube>)>,
}

static SERIAL: AtomicU64 = AtomicU64::new(1000);

pub(super) fn solid(colour: [u8; 4], [width, height]: [u32; 2]) -> Frame {
    Frame {
        width,
        height,
        rgba: colour.repeat((width * height) as usize).into(),
    }
}

/// Four colours, one a quarter: left top, right top, left bottom, right
/// bottom. Each quarter is 8 texels square.
pub(super) fn quartered(colours: [[u8; 4]; 4]) -> Frame {
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

impl Pic {
    /// A picture that fills the canvas.
    pub(super) fn of(frame: Frame) -> Self {
        Self {
            layer: Layer {
                clip: Uuid::new_v4(),
                track: Uuid::new_v4(),
                source: LayerSource::Media {
                    asset_id: Uuid::new_v4(),
                    source_time: Time::ZERO,
                    speed: 1.0,
                },
                transform: Transform::default(),
                blend: BlendMode::Normal,
                effects: Vec::new(),
                masks: Vec::new(),
                transition: None,
                seed: 7,
                time: Time::ZERO,
            },
            place: Placement {
                center: [32.0, 32.0],
                half: [32.0, 32.0],
                angle: 0.0,
            },
            frame,
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
            plates: Vec::new(),
            luts: Vec::new(),
        }
    }

    pub(super) fn full(colour: [u8; 4]) -> Self {
        Self::of(solid(colour, [16, 16]))
    }

    pub(super) fn at(mut self, center: [f32; 2], half: [f32; 2]) -> Self {
        self.place.center = center;
        self.place.half = half;
        self
    }

    fn turned(mut self, degrees: f32) -> Self {
        self.place.angle = degrees.to_radians();
        self
    }

    fn opacity(mut self, opacity: f32) -> Self {
        self.layer.transform.opacity = opacity;
        self
    }

    pub(super) fn blend(mut self, mode: BlendMode) -> Self {
        self.layer.blend = mode;
        self
    }

    pub(super) fn masked(mut self, mask: Mask) -> Self {
        self.layer.masks.push(mask);
        self
    }
}

pub(super) fn mask(shape: MaskShape, center: [f32; 2], size: [f32; 2]) -> Mask {
    Mask {
        id: Uuid::new_v4(),
        shape,
        op: MaskOp::Add,
        invert: false,
        center,
        size,
        rotation: 0.0,
        feather: 0.0,
        roundness: 0.0,
    }
}

pub(super) fn job<'a>(background: Option<[u8; 4]>, pics: &'a [Pic]) -> Job<'a> {
    Job {
        canvas: CANVAS,
        background,
        draws: pics
            .iter()
            .map(|pic| Draw {
                layer: &pic.layer,
                place: pic.place,
                input: Input {
                    serial: pic.serial,
                    frame: &pic.frame,
                    time: Time::ZERO,
                },
                steady: None,
                plates: (pic.plates.iter())
                    .map(|(effect, serial, frame)| {
                        (
                            *effect,
                            Input {
                                serial: *serial,
                                frame,
                                time: Time::ZERO,
                            },
                        )
                    })
                    .collect(),
                luts: (pic.luts.iter())
                    .map(|(effect, serial, table)| {
                        (
                            *effect,
                            Lut {
                                serial: *serial,
                                table,
                            },
                        )
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// The bytes of a drawn canvas.
pub(super) struct Image(pub(super) Vec<u8>);

impl Image {
    pub(super) fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let from = ((y * SIDE + x) * 4) as usize;
        [
            self.0[from],
            self.0[from + 1],
            self.0[from + 2],
            self.0[from + 3],
        ]
    }

    #[track_caller]
    pub(super) fn is(&self, x: u32, y: u32, expected: [u8; 4], what: &str) {
        let actual = self.at(x, y);
        let near = (actual.iter().zip(&expected)).all(|(a, e)| a.abs_diff(*e) <= 2);
        assert!(
            near,
            "{what}: pixel ({x}, {y}) is {actual:?}, expected {expected:?}"
        );
    }
}

pub(super) fn draw(gpu: &mut Gpu, background: Option<[u8; 4]>, pics: &[Pic]) -> Image {
    let job = job(background, pics);
    gpu.hold(CANVAS, 2 + job.slots(), 1);
    gpu.render(0, &job);
    Image(gpu.read(0))
}

#[test]
fn an_opaque_layer_is_drawn_where_it_is_placed() {
    let Some(mut gpu) = gpu() else { return };
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &[Pic::full(RED).at([32.0, 32.0], [16.0, 8.0])],
    );
    image.is(32, 32, RED, "the layer's centre");
    image.is(16, 24, RED, "its first pixel");
    image.is(47, 39, RED, "its last pixel");
    image.is(15, 32, NIGHT, "left of it");
    image.is(48, 32, NIGHT, "right of it");
    image.is(32, 23, NIGHT, "over it");
    image.is(32, 40, NIGHT, "under it");
    image.is(0, 0, NIGHT, "the canvas's corner");
}

#[test]
fn a_layer_is_turned_about_its_centre() {
    let Some(mut gpu) = gpu() else { return };
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &[Pic::full(RED).at([32.0, 32.0], [24.0, 8.0]).turned(90.0)],
    );
    image.is(32, 10, RED, "the top of the turned layer");
    image.is(32, 54, RED, "the bottom of the turned layer");
    image.is(12, 32, NIGHT, "where it was before it was turned");
    image.is(52, 32, NIGHT, "where it was before it was turned");
}

#[test]
fn a_picture_is_drawn_the_way_up_it_is_and_flipped_and_cropped_as_asked() {
    let Some(mut gpu) = gpu() else { return };
    let quarters = [RED, BLUE, GREEN, AMBER];
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::of(quartered(quarters))]);
    image.is(16, 16, RED, "left top");
    image.is(48, 16, BLUE, "right top");
    image.is(16, 48, GREEN, "left bottom");
    image.is(48, 48, AMBER, "right bottom");

    let mut flipped = Pic::of(quartered(quarters));
    flipped.layer.transform.flip_x = true;
    let image = draw(&mut gpu, Some(NIGHT), &[flipped]);
    image.is(16, 16, BLUE, "left top, flipped across");
    image.is(48, 48, GREEN, "right bottom, flipped across");

    let mut flipped = Pic::of(quartered(quarters));
    flipped.layer.transform.flip_y = true;
    let image = draw(&mut gpu, Some(NIGHT), &[flipped]);
    image.is(16, 16, GREEN, "left top, flipped down");
    image.is(48, 48, BLUE, "right bottom, flipped down");

    // The left half cropped away: the right half fills the box it is given.
    let mut cropped = Pic::of(quartered(quarters));
    cropped.layer.transform.crop = [0.5, 0.0, 0.0, 0.0];
    let image = draw(&mut gpu, Some(NIGHT), &[cropped]);
    image.is(8, 16, BLUE, "left top of the cropped picture");
    image.is(56, 16, BLUE, "right top of the cropped picture");
    image.is(8, 48, AMBER, "left bottom of the cropped picture");
}

#[test]
fn opacity_mixes_a_layer_with_what_is_under_it_in_linear_light() {
    let Some(mut gpu) = gpu() else { return };
    let image = draw(&mut gpu, Some(BLACK), &[Pic::full(WHITE).opacity(0.5)]);
    // Half the light of white is code 188, not 128.
    image.is(32, 32, [188, 188, 188, 255], "half white over black");
    image.is(32, 32, covered(WHITE, BLACK, 0.5), "the same, worked out");

    let image = draw(&mut gpu, Some(BLUE), &[Pic::full(RED).opacity(0.25)]);
    image.is(32, 32, covered(RED, BLUE, 0.25), "a quarter red over blue");

    let image = draw(&mut gpu, Some(BLUE), &[Pic::full(RED).opacity(0.0)]);
    image.is(32, 32, BLUE, "a layer that is not there");
}

#[test]
fn a_canvas_with_nothing_under_it_keeps_its_alpha() {
    let Some(mut gpu) = gpu() else { return };
    let image = draw(
        &mut gpu,
        None,
        &[Pic::full(WHITE).at([16.0, 32.0], [16.0, 32.0]).opacity(0.5)],
    );
    // Gamma-encoded colour multiplied by alpha, as the interface blends it.
    image.is(8, 32, [128, 128, 128, 128], "half-clear white");
    image.is(48, 32, [0, 0, 0, 0], "nothing");
}

/// The W3C blend functions, on one channel of linear light.
fn blended(mode: BlendMode, b: f64, s: f64) -> f64 {
    match mode {
        BlendMode::Normal => s,
        BlendMode::Multiply => b * s,
        BlendMode::Screen => b + s - b * s,
        BlendMode::Overlay => {
            if b <= 0.5 {
                2.0 * b * s
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
            }
        }
        BlendMode::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    b.sqrt()
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        BlendMode::Add => (b + s).min(1.0),
        BlendMode::Darken => b.min(s),
        BlendMode::Lighten => b.max(s),
        BlendMode::Difference => (b - s).abs(),
    }
}

const MODES: [BlendMode; 9] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::Add,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
];

#[test]
fn every_blend_mode_is_its_formula_in_linear_light() {
    let Some(mut gpu) = gpu() else { return };
    // Channels on both sides of every branch: dark and light backdrops under
    // dark and light sources.
    let pairs = [
        ([200, 100, 50, 255], [100, 150, 220, 255]),
        ([60, 230, 140, 255], [240, 40, 200, 255]),
        ([120, 250, 20, 255], [30, 90, 250, 255]),
    ];
    for mode in MODES {
        for (under, over) in pairs {
            let image = draw(
                &mut gpu,
                Some(BLACK),
                &[Pic::full(under), Pic::full(over).blend(mode)],
            );
            let channel = |c: usize| encoded(blended(mode, linear(under[c]), linear(over[c])));
            image.is(
                32,
                32,
                [channel(0), channel(1), channel(2), 255],
                &format!("{mode:?} of {over:?} over {under:?}"),
            );
        }
    }
    // One worked by hand: code 188 is half the light of white, and half of
    // a half is a quarter, which is code 137.
    let grey = [188, 188, 188, 255];
    let image = draw(
        &mut gpu,
        Some(BLACK),
        &[Pic::full(grey), Pic::full(grey).blend(BlendMode::Multiply)],
    );
    image.is(32, 32, [137, 137, 137, 255], "half multiplied by half");
}

#[test]
fn a_blended_layer_is_mixed_by_its_opacity_and_shows_whole_over_nothing() {
    let Some(mut gpu) = gpu() else { return };
    let (under, over) = ([200, 100, 50, 255], [100, 150, 220, 255]);
    let image = draw(
        &mut gpu,
        Some(BLACK),
        &[
            Pic::full(under),
            Pic::full(over).blend(BlendMode::Multiply).opacity(0.5),
        ],
    );
    let channel = |c: usize| {
        let (b, s) = (linear(under[c]), linear(over[c]));
        encoded(0.5 * b * s + 0.5 * b)
    };
    image.is(
        32,
        32,
        [channel(0), channel(1), channel(2), 255],
        "half a multiply",
    );

    // Nothing under it to blend with: the layer as it is.
    let image = draw(
        &mut gpu,
        None,
        &[Pic::full(over).blend(BlendMode::Multiply)],
    );
    image.is(32, 32, over, "a multiply over a clear canvas");
}

#[test]
fn a_rectangle_mask_cuts_the_layer_to_its_box() {
    let Some(mut gpu) = gpu() else { return };
    let rect = mask(MaskShape::Rectangle, [0.5, 0.5], [0.5, 0.25]);
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &[Pic::full(RED).masked(rect.clone())],
    );
    image.is(32, 32, RED, "inside the mask");
    image.is(16, 24, RED, "its first pixel");
    image.is(47, 39, RED, "its last pixel");
    image.is(15, 32, NIGHT, "left of it");
    image.is(48, 32, NIGHT, "right of it");
    image.is(32, 23, NIGHT, "over it");
    image.is(32, 40, NIGHT, "under it");

    let inverted = Mask {
        invert: true,
        ..rect.clone()
    };
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED).masked(inverted)]);
    image.is(32, 32, NIGHT, "inside an inverted mask");
    image.is(4, 4, RED, "outside an inverted mask");

    let turned = Mask {
        rotation: 90.0,
        ..rect
    };
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED).masked(turned)]);
    image.is(32, 18, RED, "the top of a turned mask");
    image.is(18, 32, NIGHT, "where the mask was before it was turned");
}

#[test]
fn a_mask_turns_clockwise_about_its_own_centre() {
    let Some(mut gpu) = gpu() else { return };
    // A bar 48 pixels long and 8 across through the canvas's centre, turned
    // an eighth: clockwise, as a layer turns, it runs from the left top to
    // the right bottom.
    let bar = Mask {
        rotation: 45.0,
        ..mask(MaskShape::Rectangle, [0.5, 0.5], [0.75, 0.125])
    };
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED).masked(bar.clone())]);
    image.is(32, 32, RED, "the bar's centre");
    image.is(44, 44, RED, "down its length to the right");
    image.is(20, 20, RED, "up its length to the left");
    image.is(44, 20, NIGHT, "where it would be turned the other way");
    image.is(20, 44, NIGHT, "where it would be turned the other way");

    // And the other way when asked to.
    let back = Mask {
        rotation: -45.0,
        ..bar
    };
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED).masked(back)]);
    image.is(44, 20, RED, "up its length to the right");
    image.is(44, 44, NIGHT, "where it was turned clockwise");
}

#[test]
fn a_feathered_edge_is_half_covered_where_the_edge_is() {
    let Some(mut gpu) = gpu() else { return };
    // A band 32 pixels wide, centred on the edge at x = 16.
    let soft = Mask {
        feather: 0.5,
        ..mask(MaskShape::Rectangle, [0.5, 0.5], [0.5, 4.0])
    };
    let image = draw(&mut gpu, Some(BLACK), &[Pic::full(WHITE).masked(soft)]);
    let cover = |x: u32| {
        // The pixel's centre is this far outside the edge.
        let away = 16.0 - (f64::from(x) + 0.5);
        let t = ((away + 16.0) / 32.0).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    };
    for x in [0, 4, 12, 16, 20, 28, 32] {
        image.is(x, 32, covered(WHITE, BLACK, cover(x)), "across the band");
    }
    image.is(32, 32, WHITE, "past the band");
    image.is(0, 32, BLACK, "before the band");
    // Half the light of white at the edge itself, between its two pixels.
    let [before, ..] = image.at(15, 32);
    let [after, ..] = image.at(16, 32);
    assert!(
        before < 188 && after > 188,
        "the edge is half covered between pixels 15 and 16: {before} and {after}"
    );
}

#[test]
fn an_ellipse_mask_and_a_round_rectangle_leave_their_corners_out() {
    let Some(mut gpu) = gpu() else { return };
    let ellipse = mask(MaskShape::Ellipse, [0.5, 0.5], [0.5, 0.5]);
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED).masked(ellipse)]);
    image.is(32, 32, RED, "the ellipse's centre");
    image.is(32, 18, RED, "just inside its top");
    image.is(45, 32, RED, "just inside its right");
    image.is(32, 14, NIGHT, "just over its top");
    image.is(18, 18, NIGHT, "the corner of the box around it");
    image.is(45, 45, NIGHT, "the opposite corner");

    // Wider than tall: radii of 24 and 8.
    let wide = mask(MaskShape::Ellipse, [0.5, 0.5], [0.75, 0.25]);
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED).masked(wide)]);
    image.is(10, 32, RED, "inside the long axis");
    image.is(32, 26, RED, "inside the short axis");
    image.is(32, 22, NIGHT, "past the short axis");
    image.is(11, 26, NIGHT, "the corner of the box around it");

    let square = mask(MaskShape::Rectangle, [0.5, 0.5], [0.5, 0.5]);
    let image = draw(
        &mut gpu,
        Some(NIGHT),
        &[Pic::full(RED).masked(square.clone())],
    );
    image.is(17, 17, RED, "the corner of a square mask");
    let round = Mask {
        roundness: 1.0,
        ..square
    };
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED).masked(round)]);
    image.is(17, 17, NIGHT, "the corner of a fully round one");
    image.is(32, 17, RED, "the middle of its top edge");
}

#[test]
fn masks_combine_as_their_operations_say() {
    let Some(mut gpu) = gpu() else { return };
    let left = mask(MaskShape::Rectangle, [0.375, 0.5], [0.5, 0.5]);
    let right = mask(MaskShape::Rectangle, [0.625, 0.5], [0.5, 0.5]);
    let with = |op: MaskOp| Mask {
        op,
        ..right.clone()
    };
    // The left covers x 8..40 and the right x 24..56, both y 16..48.
    let two = |second: Mask| Pic::full(RED).masked(left.clone()).masked(second);

    let image = draw(&mut gpu, Some(NIGHT), &[two(with(MaskOp::Add))]);
    image.is(12, 32, RED, "added: the left alone");
    image.is(32, 32, RED, "added: both");
    image.is(52, 32, RED, "added: the right alone");
    image.is(4, 32, NIGHT, "added: neither");

    let image = draw(&mut gpu, Some(NIGHT), &[two(with(MaskOp::Subtract))]);
    image.is(12, 32, RED, "subtracted: the left alone");
    image.is(32, 32, NIGHT, "subtracted: both");
    image.is(52, 32, NIGHT, "subtracted: the right alone");

    let image = draw(&mut gpu, Some(NIGHT), &[two(with(MaskOp::Intersect))]);
    image.is(12, 32, NIGHT, "intersected: the left alone");
    image.is(32, 32, RED, "intersected: both");
    image.is(52, 32, NIGHT, "intersected: the right alone");

    // A first mask that subtracts takes from the whole layer.
    let hole = Mask {
        op: MaskOp::Subtract,
        ..left.clone()
    };
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED).masked(hole)]);
    image.is(12, 32, NIGHT, "a hole");
    image.is(52, 32, RED, "around the hole");
}

#[test]
fn a_mask_moves_and_turns_with_its_layer() {
    let Some(mut gpu) = gpu() else { return };
    // The layer's box is x 32..64, y 16..48; the mask is its left half.
    let half = mask(MaskShape::Rectangle, [0.25, 0.5], [0.5, 1.0]);
    let pic = || {
        Pic::full(RED)
            .at([48.0, 32.0], [16.0, 16.0])
            .masked(half.clone())
    };
    let image = draw(&mut gpu, Some(NIGHT), &[pic()]);
    image.is(36, 32, RED, "the masked half of the layer");
    image.is(52, 32, NIGHT, "the half the mask leaves out");
    image.is(16, 32, NIGHT, "outside the layer");

    // Turned a quarter clockwise, the layer's left is up.
    let image = draw(&mut gpu, Some(NIGHT), &[pic().turned(90.0)]);
    image.is(48, 20, RED, "the masked half, turned");
    image.is(48, 44, NIGHT, "the other half, turned");
}

#[test]
fn layers_are_drawn_in_order_and_the_upper_one_covers() {
    let Some(mut gpu) = gpu() else { return };
    let left = || Pic::full(RED).at([24.0, 32.0], [16.0, 16.0]);
    let right = || Pic::full(BLUE).at([40.0, 32.0], [16.0, 16.0]);
    let image = draw(&mut gpu, Some(NIGHT), &[left(), right()]);
    image.is(12, 32, RED, "the lower layer alone");
    image.is(32, 32, BLUE, "where the upper layer covers");
    image.is(52, 32, BLUE, "the upper layer alone");

    let image = draw(&mut gpu, Some(NIGHT), &[right(), left()]);
    image.is(32, 32, RED, "the other way round");
    image.is(52, 32, BLUE, "the lower layer alone");
}

#[test]
fn a_picture_with_alpha_lets_what_is_under_it_through() {
    let Some(mut gpu) = gpu() else { return };
    let glass = [220, 40, 40, 128];
    let image = draw(&mut gpu, Some(BLUE), &[Pic::full(glass)]);
    image.is(
        32,
        32,
        covered(RED, BLUE, 128.0 / 255.0),
        "half-clear red over blue",
    );

    // A video frame is no longer taken to be opaque: clear is clear.
    let image = draw(&mut gpu, Some(BLUE), &[Pic::full([220, 40, 40, 0])]);
    image.is(32, 32, BLUE, "a clear picture");

    // A clear texel beside an opaque one lends it no colour.
    let mut edge = Vec::new();
    edge.extend_from_slice(&RED);
    edge.extend_from_slice(&[0, 255, 0, 0]);
    let frame = Frame {
        width: 2,
        height: 1,
        rgba: edge.into(),
    };
    let image = draw(&mut gpu, Some(BLACK), &[Pic::of(frame)]);
    for x in [20, 32, 44] {
        let [red, green, ..] = image.at(x, 32);
        assert!(
            green <= 40 && red >= green,
            "the clear green texel shows at x = {x}: red {red}, green {green}"
        );
    }
}

#[test]
fn a_picture_is_uploaded_once_for_as_long_as_it_is_shown() {
    let Some(mut gpu) = gpu() else { return };
    let mut pic = Pic::full(RED);
    pic.serial = 5;
    draw(&mut gpu, Some(NIGHT), std::slice::from_ref(&pic));
    draw(&mut gpu, Some(NIGHT), std::slice::from_ref(&pic));
    assert_eq!(gpu.uploads(), 1, "the same picture drawn twice");

    pic.serial = 6;
    pic.frame = solid(BLUE, [16, 16]);
    let image = draw(&mut gpu, Some(NIGHT), std::slice::from_ref(&pic));
    image.is(32, 32, BLUE, "the picture that took its place");
    assert_eq!(gpu.uploads(), 2, "another picture");

    let held = gpu.bytes();
    gpu.keep_pictures(|serial| serial == 6);
    assert_eq!(
        held - gpu.bytes(),
        16 * 16 * 4,
        "the picture no longer shown is given up"
    );
    gpu.release();
    assert_eq!(gpu.bytes(), 0, "everything is given up");
}

/// The tests' device as a window would hand it over, with the interface's
/// renderer: what a viewer composites on.
pub(crate) fn render_state() -> Option<Turn<RenderState>> {
    let Turn { made: rig, _turn } = rig()?;
    let renderer = egui_wgpu::Renderer::new(
        &rig.device,
        wgpu::TextureFormat::Rgba8Unorm,
        egui_wgpu::RendererOptions::PREDICTABLE,
    );
    let made = RenderState {
        adapter: rig.adapter.clone(),
        available_adapters: Vec::new(),
        instance: rig.instance.clone(),
        device: rig.device.clone(),
        queue: rig.queue.clone(),
        target_format: wgpu::TextureFormat::Rgba8Unorm,
        renderer: Arc::new(RwLock::new(renderer)),
        surface_config: egui_wgpu::SurfaceConfig::LOW_LATENCY,
    };
    Some(Turn { made, _turn })
}

/// A composer on the tests' device, with the interface's renderer.
pub(super) fn composer(budget: ComposeBudget) -> Option<Turn<Composer>> {
    let Turn { made: state, _turn } = render_state()?;
    let mut made = Composer::new(Some(state));
    made.budget = budget;
    Some(Turn { made, _turn })
}

/// A composer on the tests' device, and the device and queue themselves:
/// what the viewer composites with beside what an export draws with, in
/// one turn.
pub(crate) fn composer_beside_device(
    budget: ComposeBudget,
) -> Option<Turn<(Composer, wgpu::Device, wgpu::Queue)>> {
    let Turn { made: state, _turn } = render_state()?;
    let (device, queue) = (state.device.clone(), state.queue.clone());
    let mut composer = Composer::new(Some(state));
    composer.budget = budget;
    Some(Turn {
        made: (composer, device, queue),
        _turn,
    })
}

fn held(composer: &Composer) -> usize {
    composer.gpu.as_ref().map_or(0, Gpu::bytes)
}

#[test]
fn a_canvas_is_registered_once_and_drawn_again_only_when_it_changes() {
    let Some(mut composer) = composer(ComposeBudget::PRODUCTION) else {
        return;
    };
    let mut pics = [Pic::full(RED).blend(BlendMode::Multiply)];
    pics[0].serial = 1;
    let first = composer.frame(CANVAS, &[job(Some(WHITE), &pics)]);
    let Some(texture) = first[0] else {
        panic!("a job with a device is given a texture");
    };
    assert!(
        matches!(texture, egui::TextureId::User(_)),
        "the canvas is a texture of the application's"
    );
    let image = Image(composer.gpu.as_ref().expect("a device").read(0));
    image.is(32, 32, RED, "red multiplied by white");

    // Paused: the same plan, pictures and size, frame after frame.
    for _ in 0..5 {
        composer.begin_frame();
        assert_eq!(
            composer.frame(CANVAS, &[job(Some(WHITE), &pics)]),
            first,
            "the canvas keeps its texture"
        );
    }
    let counters = composer.counters();
    assert_eq!(
        (counters.composed, counters.skipped),
        (1, 5),
        "a canvas that has not changed is not drawn again"
    );
    assert_eq!(composer.gpu.as_ref().map(Gpu::uploads), Some(1));

    // Another picture, another size: drawn again, under the same name.
    pics[0].serial = 2;
    pics[0].frame = solid(BLUE, [16, 16]);
    assert_eq!(composer.frame(CANVAS, &[job(Some(WHITE), &pics)]), first);
    assert_eq!(composer.counters().composed, 2, "another picture");
    assert_eq!(composer.gpu.as_ref().map(Gpu::uploads), Some(2));
    assert_eq!(
        composer.frame([32, 32], &[job(Some(WHITE), &pics)]),
        first,
        "a canvas of another size keeps its name"
    );
    assert_eq!(composer.counters().composed, 3, "another size");
    assert_eq!(
        composer.gpu.as_ref().map(Gpu::uploads),
        Some(2),
        "a picture is not uploaded again for another size"
    );
    assert_eq!(
        composer.drawn().collect::<Vec<_>>(),
        vec![(pics[0].layer.clip, 2)]
    );
}

#[test]
fn the_runs_of_a_frame_each_have_a_canvas_and_share_the_ones_between_passes() {
    let Some(mut composer) = composer(ComposeBudget::PRODUCTION) else {
        return;
    };
    let under = [Pic::full(RED).blend(BlendMode::Screen)];
    let over = [Pic::full(BLUE).at([16.0, 16.0], [16.0, 16.0]).masked(mask(
        MaskShape::Ellipse,
        [0.5, 0.5],
        [1.0, 1.0],
    ))];
    let canvas = (SIDE * SIDE * 4) as usize;
    let both = composer.frame(CANVAS, &[job(Some(NIGHT), &under), job(None, &over)]);
    let [Some(lower), Some(upper)] = both[..] else {
        panic!("each run is given a texture: {both:?}");
    };
    assert_ne!(lower, upper, "each its own");
    let read =
        |composer: &Composer, slot| Image(composer.gpu.as_ref().expect("a device").read(slot));
    let screened = |c: usize| encoded(blended(BlendMode::Screen, linear(NIGHT[c]), linear(RED[c])));
    read(&composer, 0).is(
        48,
        48,
        [screened(0), screened(1), screened(2), 255],
        "the lower run, screened over the canvas's colour",
    );
    read(&composer, 1).is(16, 16, BLUE, "the upper run");
    read(&composer, 1).is(
        48,
        48,
        [0, 0, 0, 0],
        "which is clear where it draws nothing",
    );
    let fixed = held(&composer) - 2 * 16 * 16 * 4 - 6 * canvas;
    // Two to blend between, the layer, one more for the mask, and two shown.
    assert_eq!(
        Some(fixed),
        composer.gpu.as_ref().map(Gpu::fixed_bytes),
        "six canvases, two pictures and the uniform records"
    );

    // More runs than a frame may composite: the last keep the mesh path.
    let many: Vec<Job<'_>> = (0..MAX_CANVASES + 1).map(|_| job(None, &over)).collect();
    let drawn = composer.frame(CANVAS, &many);
    assert_eq!(drawn.iter().flatten().count(), MAX_CANVASES);
    assert_eq!(drawn[MAX_CANVASES], None);
    assert_eq!(
        held(&composer),
        fixed + 16 * 16 * 4 + (4 + MAX_CANVASES) * canvas
    );

    // One run again: the canvases of the others are given up at once.
    let one = composer.frame(CANVAS, &[job(Some(NIGHT), &under)]);
    assert_eq!(one, vec![Some(lower)], "the first run keeps its texture");
    assert_eq!(held(&composer), fixed + 16 * 16 * 4 + 4 * canvas);
    assert_eq!(composer.ids.len(), 1, "the interface is told");
}

#[test]
fn canvases_are_given_up_when_idle_and_when_the_project_is_another() {
    let Some(mut composer) = composer(ComposeBudget::PRODUCTION) else {
        return;
    };
    let pics = [Pic::full(RED).blend(BlendMode::Screen)];
    composer.frame(CANVAS, &[job(Some(NIGHT), &pics)]);
    let canvases = 4 * (SIDE * SIDE * 4) as usize;
    assert!(
        held(&composer) >= canvases + 16 * 16 * 4,
        "three canvases to draw on, one to show and the picture are held"
    );
    assert!(
        held(&composer) <= ComposeBudget::PRODUCTION.bytes,
        "within the budget"
    );

    // Frames in which no canvas is asked for: a plain plan, or another view.
    for _ in 0..LAYER_IDLE_FRAMES {
        composer.begin_frame();
        composer.frame(CANVAS, &[]);
    }
    assert!(held(&composer) > 0, "kept for as long as a viewer layer is");
    composer.begin_frame();
    assert_eq!(held(&composer), 0, "given up once idle");
    assert_eq!(composer.counters().released, 1);
    assert!(composer.ids.is_empty(), "the interface is told");

    let again = composer.frame(CANVAS, &[job(Some(NIGHT), &pics)]);
    assert!(again[0].is_some(), "and drawn again when asked for");
    composer.release();
    assert_eq!(held(&composer), 0, "given up with the project");
    assert_eq!(composer.counters().released, 2);
}

#[test]
fn a_canvas_steps_down_to_fit_the_budget_or_is_left_to_the_mesh_path() {
    let Some(mut probe) = composer(ComposeBudget::PRODUCTION) else {
        return;
    };
    let pics = [Pic::full(RED).blend(BlendMode::Add)];
    probe.frame(CANVAS, &[job(Some(NIGHT), &pics)]);
    // The uniform records and the picture: what is held whatever the size.
    let fixed = held(&probe) - 4 * (SIDE * SIDE * 4) as usize;
    drop(probe);

    assert_eq!(
        target_size(CANVAS, vec2(320.0, 320.0), 1.0),
        [320, 320],
        "the pixels the canvas covers on screen"
    );
    // Room for four canvases of 160 × 160, and not of 320 × 320.
    let budget = ComposeBudget {
        bytes: fixed + 4 * 160 * 160 * 4,
    };
    let Some(mut composer) = composer(budget) else {
        return;
    };
    let drawn = composer.frame([320, 320], &[job(Some(NIGHT), &pics)]);
    assert!(drawn[0].is_some(), "drawn at a smaller size");
    assert_eq!(held(&composer), budget.bytes, "at the step that fits");
    assert_eq!(composer.counters().stepped_down, 1);
    composer.begin_frame();
    composer.frame([320, 320], &[job(Some(NIGHT), &pics)]);
    let counters = composer.counters();
    assert_eq!(
        (counters.stepped_down, counters.composed, counters.skipped),
        (1, 1, 1),
        "the smaller canvas is kept, not made again every frame"
    );
    drop(composer);

    let Some(mut composer) = self::composer(ComposeBudget { bytes: fixed }) else {
        return;
    };
    let drawn = composer.frame([320, 320], &[job(Some(NIGHT), &pics)]);
    assert_eq!(drawn, vec![None], "no room for any canvas");
    assert_eq!(held(&composer), 0);
    assert_eq!(composer.counters().refused, 1);
}

#[test]
fn what_the_device_refuses_is_caught_and_leaves_the_layers_to_the_mesh_path() {
    // A texture no device makes: longer than any allows.
    let Some(mut gpu) = gpu() else { return };
    let side = gpu.longest_side() + 1;
    let refused = gpu.checked(|gpu| gpu.hold([side, 1], 3, 1));
    assert!(refused.is_err(), "the device's refusal is handed back");
    gpu.release();
    assert_eq!(gpu.checked(|gpu| gpu.hold(CANVAS, 3, 1)), Ok(()));
    let image = draw(&mut gpu, Some(NIGHT), &[Pic::full(RED)]);
    image.is(32, 32, RED, "and the device draws on as before");
    drop(gpu);

    // The same through the compositor, from a picture too long to upload:
    // nothing is drawn, everything is given up, and it says why from then on.
    let Some(mut composer) = composer(ComposeBudget::PRODUCTION) else {
        return;
    };
    assert!(composer.available());
    let long = Pic::of(solid(RED, [side, 1])).blend(BlendMode::Screen);
    let drawn = composer.frame(CANVAS, &[job(Some(NIGHT), std::slice::from_ref(&long))]);
    assert_eq!(drawn, vec![None], "left to the mesh path");
    assert!(!composer.available());
    let why = composer.failure().expect("it says why");
    assert!(
        why.starts_with("This GPU could not draw the preview's effects: "),
        "{why}"
    );
    assert_eq!(held(&composer), 0, "everything is given up");
    assert!(composer.ids.is_empty(), "and the interface is told");

    // Frames after it ask nothing more of the device.
    let fine = [Pic::full(RED).blend(BlendMode::Screen)];
    composer.begin_frame();
    assert_eq!(composer.frame(CANVAS, &[job(Some(NIGHT), &fine)]), [None]);
    assert_eq!(held(&composer), 0);
}

#[test]
fn a_canvas_is_held_to_the_longest_side_the_device_makes() {
    let Some(mut composer) = composer(ComposeBudget::PRODUCTION) else {
        return;
    };
    let pics = [Pic::full(RED).blend(BlendMode::Screen)];
    // A strip far longer than any device's limit, and of almost no bytes.
    let drawn = composer.frame([1 << 20, 1], &[job(Some(NIGHT), &pics)]);
    assert!(drawn[0].is_some(), "drawn, at a size the device makes");
    let gpu = composer.gpu.as_ref().expect("a device");
    assert!(gpu.size()[0] <= gpu.longest_side(), "{:?}", gpu.size());
    assert!(composer.available());
}
