//! The canvas compositor: what the mesh path cannot draw.
//!
//! A [`RenderPlan`] whose layers are all plain is drawn as before, one quad a
//! layer. Layers with a blend mode, masks, effects or a transition are drawn
//! here instead: each run of them is composited off screen, in linear light,
//! into one canvas texture the interface then draws as a single quad.
//!
//! [`runs`] splits a plan, [`Ledger`] decides what a frame costs and whether
//! anything has changed since the canvas was last drawn, and [`gpu::Gpu`]
//! draws. [`Composer`] joins the two to the interface's renderer. Only the
//! last needs a device; with none, or with pipelines the device refuses,
//! every layer keeps the mesh path and the viewer says effects are not shown.
//!
//! Composition happens while the interface is drawn and never elsewhere. A
//! canvas whose plan, pictures and size are what they were is not drawn
//! again, and nothing here asks for a repaint.
pub mod gpu;
pub mod lut;
pub(crate) mod pass;

pub use pass::paired;

use crate::textures::{Input, LAYER_IDLE_FRAMES};
use editor_core::{ClipId, Layer, LayerSource, RenderPlan};
use editor_media::Sampling;
use eframe::{egui, egui_wgpu::RenderState, wgpu};
use std::{collections::HashMap, ops::Range};
use uuid::Uuid;

/// The longer sides a canvas steps down to when it does not fit the budget
/// at the size the screen asks for.
pub const CANVAS_STEPS: [u32; 7] = [160, 320, 480, 640, 960, 1280, 1920];
/// The canvases one frame may composite: the runs of layers that text clips
/// part. Runs past these keep the mesh path.
pub const MAX_CANVASES: usize = 4;
/// The passes one canvas may take.
pub const MAX_PASSES: usize = 1024;
const TEXEL: usize = 4;

/// Where a layer's box is: its centre and half size in pixels from the left
/// top of the canvas it is placed on, and its angle in radians, clockwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub angle: f32,
}

/// One layer of a canvas, with the picture it shows.
pub struct Draw<'a> {
    pub layer: &'a Layer,
    /// In pixels of the project's canvas.
    pub place: Placement,
    pub input: Input<'a>,
    /// The clean plate of each difference matte among the layer's effects,
    /// by the effect's ID. A matte without one leaves the layer as it is.
    pub plates: Vec<(Uuid, Input<'a>)>,
    /// The table of each lookup among the layer's effects, by the effect's
    /// ID. A lookup without one leaves the layer as it is.
    pub luts: Vec<(Uuid, lut::Lut<'a>)>,
    /// How the picture is read to steady it, when the layer is stabilized
    /// and its camera's path is known: the layer's box shows the picture
    /// through this, moved against the camera's shake and cropped. A layer
    /// without one is read as it is.
    pub steady: Option<Sampling>,
}

/// One canvas to composite: the layers of a run that have a picture, bottom
/// first.
pub struct Job<'a> {
    /// The project's canvas, which the placements are in.
    pub canvas: [u32; 2],
    /// What is under the lowest layer: the canvas's colour for the lowest
    /// run, nothing for one drawn over text.
    pub background: Option<[u8; 4]>,
    pub draws: Vec<Draw<'a>>,
}

impl<'a> Job<'a> {
    /// The most passes that draw it: place and blend a layer, its chain
    /// between them and the pass of its transition, and the encoding of the
    /// canvas. The two layers of a transition are blended as one, so a pair
    /// takes one pass fewer than is counted.
    pub fn passes(&self) -> usize {
        let layers: usize = (self.draws.iter())
            .map(|draw| {
                2 + pass::chain_len(draw.layer) + usize::from(draw.layer.transition.is_some())
            })
            .sum();
        layers + 1
    }

    /// The draws as they are blended onto the canvas, bottom first: one at
    /// a time, but for the two layers of a transition, which are made one
    /// layer first. A clip of a transition whose partner has no picture yet
    /// is drawn by itself, against nothing.
    fn groups(&self) -> Vec<&[Draw<'a>]> {
        let mut groups = Vec::with_capacity(self.draws.len());
        let mut from = 0;
        while from < self.draws.len() {
            let pair = (self.draws.get(from + 1))
                .is_some_and(|above| pass::paired(self.draws[from].layer, above.layer));
            let to = from + 1 + usize::from(pair);
            groups.push(&self.draws[from..to]);
            from = to;
        }
        groups
    }

    /// The targets its layers are placed on and their chains go between:
    /// as many as the layer, or the pair of a transition, that needs the
    /// most.
    fn slots(&self) -> usize {
        (self.groups().into_iter())
            .map(|group| match group {
                [below, above] => pass::pair_slots(below.layer, above.layer),
                _ => group.iter().map(|draw| pass::slots(draw.layer)).sum(),
            })
            .max()
            .unwrap_or(1)
    }
}

/// How a stretch of a plan's layers is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    /// A text layer: interface geometry, as ever.
    Text(usize),
    /// Media layers drawn as quads.
    Plain(Range<usize>),
    /// Media layers composited into one canvas.
    Composite(Range<usize>),
}

/// The plan's layers as the stretches they are drawn in, bottom first.
///
/// Text is never composited, so it parts the media layers into runs, and a
/// run is composited from its lowest layer through its highest one that is
/// not plain: what is under a blended layer is what it blends with. Plain
/// layers above that are quads over the canvas.
///
/// A run's canvas holds that run alone. A layer above a text layer therefore
/// blends with the media layers between the two and with nothing lower: not
/// the text, not what is under the text, and not the canvas's colour.
pub fn runs(plan: &RenderPlan) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut from = 0;
    let layers = &plan.layers;
    let media = |range: Range<usize>, runs: &mut Vec<Run>| {
        let fancy = layers[range.clone()]
            .iter()
            .rposition(|layer| !layer.is_plain());
        let split = fancy.map_or(range.start, |last| range.start + last + 1);
        if split > range.start {
            runs.push(Run::Composite(range.start..split));
        }
        if split < range.end {
            runs.push(Run::Plain(split..range.end));
        }
    };
    for (index, layer) in layers.iter().enumerate() {
        if matches!(layer.source, LayerSource::Text { .. }) {
            media(from..index, &mut runs);
            runs.push(Run::Text(index));
            from = index + 1;
        }
    }
    media(from..layers.len(), &mut runs);
    runs
}

/// The size a canvas of `canvas` pixels is best composited at to cover
/// `points` on screen: the screen pixels of its longer side, in the canvas's
/// shape. A canvas of that size is drawn a texel to a pixel, so a layer on
/// it is resampled once, from its decoded picture, as a quad of the mesh
/// path is, and is no softer for being composited. The price is a canvas
/// made and drawn again for every size a window is dragged through.
///
/// What is asked for here is not always given: [`Ledger::turn`] steps a
/// canvas down to fit the budget, and [`Composer::frame`] holds it to what
/// the device can make.
pub fn target_size(canvas: [u32; 2], points: egui::Vec2, pixels_per_point: f32) -> [u32; 2] {
    let needed = (points.x.max(points.y) * pixels_per_point).round();
    sized(canvas, needed.clamp(1.0, MAX_SIDE as f32) as u32)
}

/// The longest side asked for of any canvas, before the device and the
/// budget have their say.
const MAX_SIDE: u32 = 16384;

/// `canvas` drawn in to `long` pixels on its longer side.
fn sized([width, height]: [u32; 2], long: u32) -> [u32; 2] {
    let (width, height) = (u64::from(width.max(1)), u64::from(height.max(1)));
    let longer = width.max(height);
    let side = |side: u64| ((side * u64::from(long) + longer / 2) / longer).max(1) as u32;
    [side(width), side(height)]
}

/// What the compositor may hold, in logical bytes: every canvas it draws on
/// or publishes and every picture it uploaded, at four bytes a texel, its
/// uniform records, and each lookup table it draws with: the table's colours
/// as the viewer keeps them and its texture, at eight bytes a texel. It is a
/// count of what was asked of the device, and says nothing of what a driver
/// allocates for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComposeBudget {
    pub bytes: usize,
}

impl ComposeBudget {
    pub const PRODUCTION: Self = Self {
        bytes: 96 * 1024 * 1024,
    };
}

impl Default for ComposeBudget {
    fn default() -> Self {
        Self::PRODUCTION
    }
}

/// What the compositor has done, for tests and measurement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Canvases drawn.
    pub composed: u64,
    /// Canvases left as they were, because nothing they show had changed.
    pub skipped: u64,
    /// Canvases not drawn for want of room, and left to the mesh path.
    pub refused: u64,
    /// Times the canvas size went under what the screen asked for.
    pub stepped_down: u64,
    /// Times everything was given up: idle, or another project.
    pub released: u64,
}

/// What one canvas was last drawn from. Equal means nothing to draw.
struct Shown {
    size: [u32; 2],
    canvas: [u32; 2],
    background: Option<[u8; 4]>,
    draws: Vec<(Layer, Placement, u64, Option<Sampling>)>,
    /// The clean plates the draws were given, in order.
    plates: Vec<(Uuid, u64)>,
    /// And the lookup tables.
    luts: Vec<(Uuid, u64)>,
}

fn luts<'a>(job: &'a Job<'_>) -> impl Iterator<Item = (Uuid, u64)> + 'a {
    (job.draws.iter())
        .flat_map(|draw| draw.luts.iter())
        .map(|(effect, table)| (*effect, table.serial))
}

fn plates<'a>(job: &'a Job<'_>) -> impl Iterator<Item = (Uuid, u64)> + 'a {
    (job.draws.iter())
        .flat_map(|draw| draw.plates.iter())
        .map(|(effect, plate)| (*effect, plate.serial))
}

impl Shown {
    fn of(size: [u32; 2], job: &Job<'_>) -> Self {
        Self {
            size,
            canvas: job.canvas,
            background: job.background,
            draws: (job.draws.iter())
                .map(|draw| {
                    (
                        draw.layer.clone(),
                        draw.place,
                        draw.input.serial,
                        draw.steady,
                    )
                })
                .collect(),
            plates: plates(job).collect(),
            luts: luts(job).collect(),
        }
    }

    fn is(&self, size: [u32; 2], job: &Job<'_>) -> bool {
        self.size == size
            && self.canvas == job.canvas
            && self.background == job.background
            && self.draws.len() == job.draws.len()
            && plates(job).eq(self.plates.iter().copied())
            && luts(job).eq(self.luts.iter().copied())
            && (self.draws.iter().zip(&job.draws)).all(|((layer, place, serial, steady), draw)| {
                *serial == draw.input.serial
                    && *place == draw.place
                    && *steady == draw.steady
                    && layer == draw.layer
            })
    }
}

/// What to do about one job of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Render,
    /// Its canvas already shows it.
    Skip,
    /// It is not composited: the mesh path draws its layers.
    Refuse,
}

/// What a frame's jobs came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    /// The size every canvas of the frame is drawn at.
    pub size: [u32; 2],
    /// Intermediate canvases the frame needs.
    pub targets: usize,
    /// One for each job, in order. The slot of a job is its index.
    pub verdicts: Vec<Verdict>,
}

/// The compositor's books: what each canvas shows, what is held, and when
/// it was last used. It decides and counts, and touches no device.
pub struct Ledger {
    budget: ComposeBudget,
    /// Held whenever a canvas is: see [`gpu::Gpu::fixed_bytes`].
    fixed: usize,
    frame: u64,
    /// The frame a job was last given in.
    used: u64,
    shown: Vec<Option<Shown>>,
    size: [u32; 2],
    targets: usize,
    /// The pictures the canvases shown were drawn from, and their bytes.
    inputs: HashMap<u64, usize>,
    /// The lookup tables they were drawn with, and theirs.
    luts: HashMap<u64, usize>,
    counters: Counters,
}

impl Ledger {
    pub fn new(budget: ComposeBudget, fixed: usize) -> Self {
        Self {
            budget,
            fixed,
            frame: 0,
            used: 0,
            shown: Vec::new(),
            size: [0, 0],
            targets: 0,
            inputs: HashMap::new(),
            luts: HashMap::new(),
            counters: Counters::default(),
        }
    }

    #[cfg(test)]
    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// The logical bytes held for the canvases shown.
    pub fn bytes(&self) -> usize {
        if self.shown.iter().all(Option::is_none) {
            return 0;
        }
        let canvases = self.targets + self.shown.iter().flatten().count();
        self.fixed
            + self.inputs.values().sum::<usize>()
            + self.luts.values().sum::<usize>()
            + canvases * bytes_of(self.size)
    }

    /// Whether a canvas shown was drawn from the picture `serial` names.
    pub fn live(&self, serial: u64) -> bool {
        self.inputs.contains_key(&serial)
    }

    /// Whether a canvas shown was drawn with the lookup table `serial`
    /// names.
    pub fn live_lut(&self, serial: u64) -> bool {
        self.luts.contains_key(&serial)
    }

    /// The clips drawn on the canvases of this frame, each with the picture
    /// it was drawn from. Nothing when no canvas was asked for this frame.
    pub fn drawn(&self) -> impl Iterator<Item = (ClipId, u64)> + '_ {
        (self.shown.iter().flatten())
            .filter(|_| self.used == self.frame)
            .flat_map(|shown| shown.draws.iter())
            .map(|(layer, _, serial, _)| (layer.clip, *serial))
    }

    /// Begins a frame. Returns whether everything is to be given up: no
    /// canvas was asked for in as many frames as an undrawn viewer layer is
    /// kept for.
    pub fn begin_frame(&mut self) -> bool {
        self.frame += 1;
        let idle =
            !self.shown.is_empty() && self.frame.saturating_sub(self.used) > LAYER_IDLE_FRAMES;
        if idle {
            self.clear();
        }
        idle
    }

    /// Forgets every canvas: idle, or the project is another.
    pub fn clear(&mut self) {
        if !self.shown.is_empty() {
            self.counters.released += 1;
        }
        self.shown.clear();
        self.inputs.clear();
        self.luts.clear();
        self.size = [0, 0];
        self.targets = 0;
    }

    /// Takes the jobs of a frame, which would best be drawn at `want`.
    ///
    /// A job past [`MAX_CANVASES`], or of more passes than [`MAX_PASSES`], is
    /// refused, and so are all of a frame that names more lookup tables than
    /// [`lut::MAX_LUTS`]. The others share one size: `want`, or the largest step under
    /// it at which the pictures they are drawn from, the canvases between
    /// their passes and their outputs fit the budget together. If not even
    /// the smallest does, all are refused.
    ///
    /// A job whose canvas was drawn at that size from the same layers,
    /// placements and pictures is skipped.
    pub fn turn(&mut self, want: [u32; 2], jobs: &[Job<'_>]) -> Turn {
        self.used = self.frame;
        let taken = |index: usize, job: &Job<'_>| {
            index < MAX_CANVASES && !job.draws.is_empty() && job.passes() <= MAX_PASSES
        };
        let mut inputs = HashMap::new();
        let mut luts = HashMap::new();
        let mut outputs = 0;
        let mut slots = 1;
        for (_, job) in (jobs.iter().enumerate()).filter(|(index, job)| taken(*index, job)) {
            outputs += 1;
            slots = slots.max(job.slots());
            for draw in &job.draws {
                let plates = draw.plates.iter().map(|(_, plate)| plate);
                for Input { serial, frame, .. } in std::iter::once(&draw.input).chain(plates) {
                    inputs.insert(*serial, bytes_of([frame.width.max(1), frame.height.max(1)]));
                }
                for (_, table) in &draw.luts {
                    luts.insert(table.serial, lut::bytes(table.table));
                }
            }
        }
        // Two canvases to blend between, and those a layer is placed on and
        // its chain goes between.
        let targets = 2 + slots;
        let held = self.fixed + inputs.values().sum::<usize>() + luts.values().sum::<usize>();
        let long = want[0].max(want[1]);
        let size = std::iter::once(want)
            .chain(
                (CANVAS_STEPS.into_iter().rev())
                    .filter(|step| *step < long)
                    .map(|step| sized(want, step)),
            )
            .find(|size| {
                (targets + outputs)
                    .checked_mul(bytes_of(*size))
                    .and_then(|canvases| canvases.checked_add(held))
                    .is_some_and(|bytes| bytes <= self.budget.bytes)
            })
            .filter(|_| outputs > 0 && luts.len() <= lut::MAX_LUTS);
        let Some(size) = size else {
            self.counters.refused += jobs.len() as u64;
            self.shown.clear();
            self.inputs.clear();
            self.luts.clear();
            self.size = [0, 0];
            self.targets = 0;
            return Turn {
                size: [0, 0],
                targets: 0,
                verdicts: vec![Verdict::Refuse; jobs.len()],
            };
        };
        if size != want && size != self.size {
            self.counters.stepped_down += 1;
        }
        self.shown
            .resize_with(jobs.len().min(MAX_CANVASES), || None);
        let mut verdicts = Vec::with_capacity(jobs.len());
        for (index, job) in jobs.iter().enumerate() {
            let verdict = if !taken(index, job) {
                if let Some(shown) = self.shown.get_mut(index) {
                    *shown = None;
                }
                self.counters.refused += 1;
                Verdict::Refuse
            } else if (self.shown[index].as_ref()).is_some_and(|shown| shown.is(size, job)) {
                self.counters.skipped += 1;
                Verdict::Skip
            } else {
                self.shown[index] = Some(Shown::of(size, job));
                self.counters.composed += 1;
                Verdict::Render
            };
            verdicts.push(verdict);
        }
        self.inputs = inputs;
        self.luts = luts;
        self.size = size;
        self.targets = targets;
        Turn {
            size,
            targets,
            verdicts,
        }
    }
}

fn bytes_of([width, height]: [u32; 2]) -> usize {
    (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(TEXEL)
}

/// The compositor as the viewer keeps it: the books, the device when there is
/// one, and the textures the interface was told of.
#[derive(Default)]
pub struct Composer {
    budget: ComposeBudget,
    state: Option<RenderState>,
    gpu: Option<gpu::Gpu>,
    /// Why the device cannot composite, once that was found out.
    failure: Option<String>,
    ledger: Option<Ledger>,
    /// The interface's name for the output of each slot.
    ids: Vec<Option<egui::TextureId>>,
}

/// Why a device that built the pipelines no longer composites, in words for
/// the person.
fn stopped(failure: &str) -> String {
    format!("This GPU could not draw the preview's effects: {failure}")
}

impl Composer {
    /// A compositor on the interface's own device, when it has one.
    pub fn new(state: Option<RenderState>) -> Self {
        Self {
            state,
            ..Self::default()
        }
    }

    /// Whether anything can be composited. Without it every layer is drawn
    /// as a quad, with its transform and opacity alone.
    pub fn available(&self) -> bool {
        self.state.is_some() && self.failure.is_none()
    }

    /// Why nothing can be composited, in words for the person.
    pub fn failure(&self) -> Option<&str> {
        match (&self.state, &self.failure) {
            (None, _) => Some("This window has no GPU device to composite with."),
            (Some(_), Some(failure)) => Some(failure),
            (Some(_), None) => None,
        }
    }

    #[cfg(test)]
    pub fn counters(&self) -> Counters {
        self.ledger
            .as_ref()
            .map(Ledger::counters)
            .unwrap_or_default()
    }

    /// The size and bytes of the canvas of `slot` as the interface samples
    /// it, when one is drawn. Waits for the GPU.
    #[cfg(test)]
    pub fn read(&self, slot: usize) -> Option<([u32; 2], Vec<u8>)> {
        let gpu = self.gpu.as_ref()?;
        gpu.output(slot)?;
        Some((gpu.size(), gpu.read(slot)))
    }

    /// Begins a frame of the application, drawn or not, and gives everything
    /// up once the viewer has gone long enough without a canvas.
    pub fn begin_frame(&mut self) {
        if self.ledger.as_mut().is_some_and(Ledger::begin_frame) {
            self.drop_canvases();
        }
    }

    /// Whether anything is held on the device now.
    pub fn holds(&self) -> bool {
        self.gpu.as_ref().is_some_and(|gpu| gpu.bytes() > 0)
    }

    /// Gives up every canvas and picture: the project is another.
    pub fn release(&mut self) {
        if let Some(ledger) = &mut self.ledger {
            ledger.clear();
        }
        self.drop_canvases();
    }

    /// The clips on this frame's canvases and the picture each was drawn
    /// from: see [`Ledger::drawn`].
    pub fn drawn(&self) -> impl Iterator<Item = (ClipId, u64)> + '_ {
        self.ledger.iter().flat_map(Ledger::drawn)
    }

    fn drop_canvases(&mut self) {
        if let Some(gpu) = &mut self.gpu {
            gpu.release();
        }
        self.forget_ids(0);
    }

    // Tells the interface's renderer to forget the outputs from `from` on.
    fn forget_ids(&mut self, from: usize) {
        if from >= self.ids.len() {
            return;
        }
        let gone: Vec<egui::TextureId> = self.ids.drain(from..).flatten().collect();
        if let (Some(state), false) = (&self.state, gone.is_empty()) {
            let mut renderer = state.renderer.write();
            for id in &gone {
                renderer.free_texture(id);
            }
        }
    }

    /// Composites the jobs of one painted frame, each best drawn at `want`
    /// pixels, and returns the texture to draw for each: nothing for a job
    /// that is left to the mesh path.
    ///
    /// The pipelines are built the first time a job is given, which is the
    /// one moment this waits on the device for more than a frame's drawing.
    ///
    /// A canvas is held to the longest side the device makes a texture of.
    /// Whatever the device then refuses, of memory or of anything else, is
    /// caught here: everything is given up, the layers of this frame and of
    /// those after it take the mesh path, and [`Self::failure`] says why.
    pub fn frame(&mut self, want: [u32; 2], jobs: &[Job<'_>]) -> Vec<Option<egui::TextureId>> {
        let refused = || vec![None; jobs.len()];
        if jobs.is_empty() || self.failure.is_some() {
            return refused();
        }
        let Some(state) = &self.state else {
            return refused();
        };
        if self.gpu.is_none() {
            match gpu::Gpu::new(state.device.clone(), state.queue.clone()) {
                Ok(gpu) => self.gpu = Some(gpu),
                Err(failure) => {
                    self.failure = Some(format!("This GPU cannot composite: {failure}"));
                    return refused();
                }
            }
        }
        let Some(gpu) = &mut self.gpu else {
            return refused();
        };
        let ledger = self
            .ledger
            .get_or_insert_with(|| Ledger::new(self.budget, gpu.fixed_bytes()));
        let limit = gpu.longest_side();
        let want = if want[0].max(want[1]) > limit {
            sized(want, limit)
        } else {
            want
        };
        let turn = ledger.turn(want, jobs);
        if turn.targets == 0 {
            self.drop_canvases();
            return refused();
        }
        let slots = jobs.len().min(MAX_CANVASES);
        // Whether each job's output is another texture than before.
        let drawn = gpu.checked(|gpu| {
            gpu.hold(turn.size, turn.targets, slots);
            (jobs.iter().zip(&turn.verdicts).enumerate())
                .map(|(slot, (job, verdict))| {
                    // A canvas that was drawn at another size is no more.
                    *verdict == Verdict::Render && gpu.render(slot, job)
                })
                .collect::<Vec<bool>>()
        });
        let drawn = match drawn {
            Ok(drawn) => drawn,
            Err(failure) => {
                ledger.clear();
                self.failure = Some(stopped(&failure));
                self.drop_canvases();
                return refused();
            }
        };
        self.ids.resize(slots.max(self.ids.len()), None);
        let mut gone = Vec::new();
        let mut textures = Vec::with_capacity(jobs.len());
        for (slot, (verdict, fresh)) in turn.verdicts.iter().zip(drawn).enumerate() {
            if *verdict == Verdict::Refuse {
                if slot < slots {
                    gpu.drop_output(slot);
                    gone.extend(self.ids[slot].take());
                }
                textures.push(None);
                continue;
            }
            if fresh && let Some(view) = gpu.output(slot) {
                let mut renderer = state.renderer.write();
                match self.ids[slot] {
                    Some(id) => renderer.update_egui_texture_from_wgpu_texture(
                        &state.device,
                        view,
                        wgpu::FilterMode::Linear,
                        id,
                    ),
                    None => {
                        self.ids[slot] = Some(renderer.register_native_texture(
                            &state.device,
                            view,
                            wgpu::FilterMode::Linear,
                        ));
                    }
                }
            }
            textures.push(self.ids[slot]);
        }
        gpu.keep_pictures(|serial| ledger.live(serial));
        gpu.keep_luts(|serial| ledger.live_lut(serial));
        // The device holds what the books say, and no more.
        debug_assert_eq!(gpu.bytes(), ledger.bytes());
        if !gone.is_empty() {
            let mut renderer = state.renderer.write();
            for id in &gone {
                renderer.free_texture(id);
            }
        }
        self.forget_ids(slots);
        textures
    }
}

#[cfg(test)]
mod effect_tests;
#[cfg(test)]
pub(crate) mod gpu_tests;
#[cfg(test)]
mod lut_tests;
#[cfg(test)]
mod steady_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transition_tests;
