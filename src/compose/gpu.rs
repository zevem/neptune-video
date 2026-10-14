//! The compositor's GPU half: pipelines, pooled targets, its own copies of
//! the decoded frames, and the passes of one canvas.
//!
//! It knows a device and a queue and nothing of the interface, so the same
//! code draws the viewer's canvas, an export's frames and the fixtures the
//! readback tests check. What it holds is what its owner says it may: the
//! viewer's [`super::Ledger`], or the size of an export's output. This module
//! allocates and frees, and decides nothing.
//!
//! An output is read back without waiting on the device: [`Gpu::begin_read`]
//! copies it and asks for the copy, and [`Gpu::finish_read`] polls until it
//! has come, so a device the interface shares is never held up by it.
use super::{
    Job, Placement, lut,
    pass::{self, Uniform},
};
use editor_core::BlendMode;
use editor_media::Cube;
use editor_media::Frame;
use eframe::wgpu;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

/// The targets between the passes: the shaders read and write linear light
/// and the format stores it gamma-encoded, so eight bits are spent where the
/// eye tells shades apart.
const LINEAR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// The canvas as the interface samples it: the bytes as `encode.wgsl` writes.
const OUTPUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// A lookup table: what it holds is not light and is read as it is, in
/// halves, so that a table is no coarser than its file and may hold colours
/// past white.
const TABLE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const TEXEL: usize = 4;

/// The two canvases a layer is blended from and to, in turn.
const BELOW: usize = 0;
/// The layer as placed, and after it the other targets its chain goes
/// between: `pass::SLOTS` at the most. The second layer of a transition is
/// placed after those of the first.
const LAYER: usize = 2;
/// In a stack of canvases, the two a canvas drawn over the frame is drawn
/// on in turn; its layers are then placed after them.
const RUN: usize = 2;

fn bytes_of([width, height]: [u32; 2]) -> usize {
    width as usize * height as usize * TEXEL
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Target {
    fn new(
        device: &wgpu::Device,
        label: &str,
        [width, height]: [u32; 2],
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }
}

/// A decoded frame as the compositor keeps it.
struct Picture {
    target: Target,
    bytes: usize,
}

#[derive(Clone, Copy)]
enum Pipeline {
    Place = 0,
    Effect = 1,
    Blend = 2,
    Encode = 3,
    Transition = 4,
}

#[derive(Clone, Copy)]
enum Source {
    Target(usize),
    Picture(u64),
    Lut(u64),
    Blank,
}

#[derive(Clone, Copy)]
enum Into {
    Target(usize),
    Output(usize),
}

struct Step {
    pipeline: Pipeline,
    uniform: Uniform,
    first: Source,
    second: Source,
    into: Into,
}

/// What a canvas is drawn by, in order: a pass, or a target cleared to a
/// colour before the passes that draw on it.
#[expect(
    clippy::large_enum_variant,
    reason = "nearly every entry is a pass; boxing each would allocate for nothing"
)]
enum Work {
    Pass(Step),
    Clear(usize, wgpu::Color),
}

/// Why the bytes of a canvas did not come back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Whoever waited said to stop.
    Stopped,
    /// The device took longer than it was given.
    TimedOut,
    /// The device refused or lost the copy.
    Failed(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => f.write_str("Reading the frame back was stopped"),
            Self::TimedOut => f.write_str("The GPU took too long to hand the frame back"),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

/// How the mapping of a readback stands, as its callback leaves it.
const MAP_WAITING: u8 = 0;
const MAP_DONE: u8 = 1;
const MAP_FAILED: u8 = 2;
/// How long a readback sleeps between polls of the device.
const READ_NAP: Duration = Duration::from_millis(1);

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::RenderPipeline; 5],
    sampler: wgpu::Sampler,
    blank: Target,
    /// One record a pass, `stride` apart; made when first drawn with.
    uniforms: Option<wgpu::Buffer>,
    stride: u64,
    size: [u32; 2],
    targets: Vec<Target>,
    outputs: Vec<Option<Target>>,
    pictures: HashMap<u64, Picture>,
    /// The lookup tables drawn with, by their serials.
    luts: HashMap<u64, Picture>,
    uploads: u64,
    /// The buffer an output is read back through, kept from one read to the
    /// next while the size is the same.
    readback: Option<wgpu::Buffer>,
    /// How the mapping [`Self::begin_read`] asked for stands.
    mapping: Option<Arc<AtomicU8>>,
}

impl Gpu {
    /// Builds the pipelines. An error says why the device cannot draw them:
    /// a shader it does not take, one its driver fails to compile, or no
    /// memory for them.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self, String> {
        let (layout, pipelines, sampler, blank) = checked(&device, || Self::build(&device))?;
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(1);
        Ok(Self {
            stride: (Uniform::BYTES as u64).div_ceil(align) * align,
            device,
            queue,
            layout,
            pipelines,
            sampler,
            blank,
            uniforms: None,
            size: [0, 0],
            targets: Vec::new(),
            outputs: Vec::new(),
            pictures: HashMap::new(),
            luts: HashMap::new(),
            uploads: 0,
            readback: None,
            mapping: None,
        })
    }

    fn build(
        device: &wgpu::Device,
    ) -> (
        wgpu::BindGroupLayout,
        [wgpu::RenderPipeline; 5],
        wgpu::Sampler,
        Target,
    ) {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compose pass"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(Uniform::BYTES as u64),
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compose pass"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label: &str, source: &str, format: wgpu::TextureFormat| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(
                    format!("{}\n{source}", include_str!("shaders/common.wgsl")).into(),
                ),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // In the order of `Pipeline`.
        let pipelines = [
            pipeline("compose place", include_str!("shaders/place.wgsl"), LINEAR),
            pipeline(
                "compose effect",
                include_str!("shaders/effect.wgsl"),
                LINEAR,
            ),
            pipeline("compose blend", include_str!("shaders/blend.wgsl"), LINEAR),
            pipeline(
                "compose encode",
                include_str!("shaders/encode.wgsl"),
                OUTPUT,
            ),
            pipeline(
                "compose transition",
                include_str!("shaders/transition.wgsl"),
                LINEAR,
            ),
        ];
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("compose"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blank = Target::new(
            device,
            "compose blank",
            [1, 1],
            LINEAR,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        (layout, pipelines, sampler, blank)
    }

    /// Does `work`, which allocates or draws, and says what the device
    /// refused of it: see [`checked`].
    pub fn checked<T>(&mut self, work: impl FnOnce(&mut Self) -> T) -> Result<T, String> {
        let device = self.device.clone();
        checked(&device, || work(self))
    }

    /// The longest side the device makes a texture of.
    pub fn longest_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d.max(1)
    }

    /// What is kept beside the canvases whenever anything is drawn: the
    /// uniform records and the blank texture.
    pub fn fixed_bytes(&self) -> usize {
        self.stride as usize * super::MAX_PASSES + TEXEL
    }

    /// The logical bytes of everything held now: texels at four bytes each,
    /// the uniform records and the buffer outputs are read back through.
    /// What the driver keeps for them is its own.
    pub fn bytes(&self) -> usize {
        let canvases = self.targets.len() + self.outputs.iter().flatten().count();
        let pictures: usize = (self.pictures.values().chain(self.luts.values()))
            .map(|picture| picture.bytes)
            .sum();
        let fixed = if self.uniforms.is_some() {
            self.fixed_bytes()
        } else {
            0
        };
        let readback = (self.readback.as_ref()).map_or(0, |buffer| buffer.size() as usize);
        fixed + canvases * bytes_of(self.size) + pictures + readback
    }

    /// Frames uploaded since this was made.
    #[cfg(test)]
    pub fn uploads(&self) -> u64 {
        self.uploads
    }

    pub fn output(&self, slot: usize) -> Option<&wgpu::TextureView> {
        Some(&self.outputs.get(slot)?.as_ref()?.view)
    }

    /// Holds `targets` intermediate canvases of `size` and room for `slots`
    /// outputs. Another size starts every canvas over.
    pub fn hold(&mut self, size: [u32; 2], targets: usize, slots: usize) {
        if self.size != size {
            self.targets.clear();
            self.outputs.clear();
            self.size = size;
        }
        self.targets.truncate(targets);
        while self.targets.len() < targets {
            self.targets.push(Target::new(
                &self.device,
                "compose canvas",
                size,
                LINEAR,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ));
        }
        self.outputs.resize_with(slots, || None);
    }

    /// Gives up the output of `slot`, which nothing is drawn to this frame.
    pub fn drop_output(&mut self, slot: usize) {
        if let Some(output) = self.outputs.get_mut(slot) {
            *output = None;
        }
    }

    /// Keeps only the pictures `live` names.
    pub fn keep_pictures(&mut self, live: impl Fn(u64) -> bool) {
        self.pictures.retain(|serial, _| live(*serial));
    }

    /// Keeps only the lookup tables `live` names.
    pub fn keep_luts(&mut self, live: impl Fn(u64) -> bool) {
        self.luts.retain(|serial, _| live(*serial));
    }

    /// The size the canvases are held at.
    #[cfg(test)]
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// The lookup tables held now.
    #[cfg(test)]
    pub fn luts(&self) -> usize {
        self.luts.len()
    }

    /// Gives up everything but the pipelines.
    pub fn release(&mut self) {
        self.targets.clear();
        self.outputs.clear();
        self.pictures.clear();
        self.luts.clear();
        self.uniforms = None;
        self.readback = None;
        self.mapping = None;
        self.size = [0, 0];
    }

    fn upload(&mut self, serial: u64, frame: &Frame) {
        if self.pictures.contains_key(&serial) {
            return;
        }
        let size = [frame.width.max(1), frame.height.max(1)];
        // A frame is tightly packed; one that is not all there is not drawn
        // from, and shows as clear.
        let target = Target::new(
            &self.device,
            "compose picture",
            size,
            LINEAR,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        if frame.rgba.len() == bytes_of([frame.width, frame.height]) && !frame.rgba.is_empty() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &target.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(frame.width * TEXEL as u32),
                    rows_per_image: Some(frame.height),
                },
                wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.uploads += 1;
        self.pictures.insert(
            serial,
            Picture {
                target,
                bytes: bytes_of(size),
            },
        );
    }

    fn upload_lut(&mut self, serial: u64, table: &Cube) {
        if self.luts.contains_key(&serial) {
            return;
        }
        let (_, [width, height]) = lut::layout(table.size);
        let target = Target::new(
            &self.device,
            "compose lut",
            [width, height],
            TABLE,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let texels = lut::texels(table);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(texels.len() as u32 / height),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.uploads += 1;
        self.luts.insert(
            serial,
            Picture {
                target,
                bytes: lut::bytes(table),
            },
        );
    }

    fn view(&self, source: Source) -> &wgpu::TextureView {
        match source {
            Source::Lut(serial) => {
                (self.luts.get(&serial)).map_or(&self.blank.view, |table| &table.target.view)
            }
            Source::Target(index) => &self.targets[index].view,
            Source::Picture(serial) => (self.pictures.get(&serial))
                .map_or(&self.blank.view, |picture| &picture.target.view),
            Source::Blank => &self.blank.view,
        }
    }

    /// Draws `job` to the output of `slot`, which [`Self::hold`] made room
    /// for. Returns whether that output is another texture than before.
    pub fn render(&mut self, slot: usize, job: &Job<'_>) -> bool {
        self.render_stack(slot, std::slice::from_ref(job))
    }

    /// Draws `jobs` one over another to the output of `slot`: the first as
    /// [`Self::render`] draws a job, and each after it on a clear canvas of
    /// its own that is then laid over what is under it whole, with normal
    /// blending. A job with nothing to draw adds nothing. [`Self::hold`]
    /// made room for [`super::stack_targets`] canvases. Returns whether that
    /// output is another texture than before.
    pub fn render_stack(&mut self, slot: usize, jobs: &[Job<'_>]) -> bool {
        let size = self.size;
        for draw in jobs.iter().flat_map(|job| &job.draws) {
            self.upload(draw.input.serial, draw.input.frame);
            for (_, plate) in &draw.plates {
                self.upload(plate.serial, plate.frame);
            }
            for (_, table) in &draw.luts {
                self.upload_lut(table.serial, table.table);
            }
        }
        let fresh = self.outputs[slot].is_none();
        if fresh {
            self.outputs[slot] = Some(Target::new(
                &self.device,
                "compose output",
                size,
                OUTPUT,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
            ));
        }

        // The passes, in order, and the canvases cleared between them.
        let mut work = Vec::new();
        let stacked = jobs.len() > 1;
        let layers = if stacked { RUN + 2 } else { LAYER };
        let mut below = BELOW;
        if let Some((first, rest)) = jobs.split_first() {
            work.push(Work::Clear(BELOW, under(first.background)));
            below = passes(first, size, [BELOW, BELOW + 1], layers, &mut work);
            for job in rest.iter().filter(|job| !job.draws.is_empty()) {
                work.push(Work::Clear(RUN, under(job.background)));
                let canvas = passes(job, size, [RUN, RUN + 1], layers, &mut work);
                let into = if below == BELOW { BELOW + 1 } else { BELOW };
                work.push(Work::Pass(Step {
                    pipeline: Pipeline::Blend,
                    uniform: Uniform::new(size)
                        .variant(pass::blend_mode(BlendMode::Normal))
                        .opacity(1.0),
                    first: Source::Target(below),
                    second: Source::Target(canvas),
                    into: Into::Target(into),
                }));
                below = into;
            }
        } else {
            work.push(Work::Clear(BELOW, wgpu::Color::TRANSPARENT));
        }
        work.push(Work::Pass(Step {
            pipeline: Pipeline::Encode,
            uniform: Uniform::new(size),
            first: Source::Target(below),
            second: Source::Blank,
            into: Into::Output(slot),
        }));
        // Passes past the most the records hold are not drawn.
        let mut counted = 0;
        work.retain(|work| match work {
            Work::Pass(_) => {
                counted += 1;
                counted <= super::MAX_PASSES
            }
            Work::Clear(..) => true,
        });

        let stride = self.stride;
        let uniforms = self
            .uniforms
            .get_or_insert_with(|| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("compose passes"),
                    size: stride * super::MAX_PASSES as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .clone();
        let steps = work.iter().filter_map(|work| match work {
            Work::Pass(step) => Some(step),
            Work::Clear(..) => None,
        });
        for (index, step) in steps.enumerate() {
            self.queue
                .write_buffer(&uniforms, index as u64 * stride, &step.uniform.bytes());
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("compose"),
            });
        let mut index = 0;
        for work in &work {
            let step = match work {
                Work::Clear(target, colour) => {
                    drop(begin(&mut encoder, &self.targets[*target].view, *colour));
                    continue;
                }
                Work::Pass(step) => step,
            };
            let record = index;
            index += 1;
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("compose pass"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &uniforms,
                            offset: 0,
                            size: wgpu::BufferSize::new(Uniform::BYTES as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(self.view(step.first)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(self.view(step.second)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let into = match step.into {
                Into::Target(index) => &self.targets[index].view,
                Into::Output(slot) => match &self.outputs[slot] {
                    Some(output) => &output.view,
                    None => continue,
                },
            };
            // The triangle covers the target, so nothing of what it held
            // needs to be read back first.
            let mut pass = begin(&mut encoder, into, wgpu::Color::TRANSPARENT);
            pass.set_pipeline(&self.pipelines[step.pipeline as usize]);
            pass.set_bind_group(0, &group, &[(record as u64 * stride) as u32]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        fresh
    }

    /// Copies the output of `slot` to a buffer kept for reading it back and
    /// asks for that buffer to be mapped; [`Self::finish_read`] hands the
    /// bytes over once the device has done it. It allocates and submits, so
    /// it belongs inside [`Self::checked`], and it waits on nothing.
    pub fn begin_read(&mut self, slot: usize) -> Result<(), String> {
        let Some(output) = self.outputs.get(slot).and_then(Option::as_ref) else {
            return Err("Nothing was drawn to read back".into());
        };
        let [width, height] = self.size;
        let row = padded_row(width);
        let bytes = u64::from(row) * u64::from(height);
        if self
            .readback
            .as_ref()
            .is_none_or(|kept| kept.size() != bytes)
        {
            self.readback = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("compose readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
        }
        let buffer = self.readback.as_ref().expect("a readback buffer was made");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("compose readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &output.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let mapped = Arc::new(AtomicU8::new(MAP_WAITING));
        let told = Arc::clone(&mapped);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                told.store(
                    if result.is_ok() { MAP_DONE } else { MAP_FAILED },
                    Ordering::Release,
                );
            });
        self.mapping = Some(mapped);
        Ok(())
    }

    /// Hands over the rows of the output [`Self::begin_read`] copied, top
    /// first, each `width` texels of four bytes, to `row` with its index.
    ///
    /// It never blocks on the device, which the interface may share: it
    /// polls it, sleeping a millisecond between polls, until the copy is
    /// mapped, `stop` says to give up, or `deadline` passes. A buffer given
    /// up on is let go, and the next read makes another.
    pub fn finish_read(
        &mut self,
        stop: &dyn Fn() -> bool,
        deadline: Instant,
        row: &mut dyn FnMut(usize, &[u8]),
    ) -> Result<(), ReadError> {
        let Some(mapped) = self.mapping.take() else {
            return Err(ReadError::Failed("No readback was asked for".into()));
        };
        loop {
            match mapped.load(Ordering::Acquire) {
                MAP_DONE => break,
                MAP_FAILED => {
                    self.readback = None;
                    return Err(ReadError::Failed(
                        "The GPU could not hand the frame back".into(),
                    ));
                }
                _ => {}
            }
            if stop() {
                self.readback = None;
                return Err(ReadError::Stopped);
            }
            if Instant::now() >= deadline {
                self.readback = None;
                return Err(ReadError::TimedOut);
            }
            if let Err(error) = self.device.poll(wgpu::PollType::Poll) {
                self.readback = None;
                return Err(ReadError::Failed(error.to_string()));
            }
            if mapped.load(Ordering::Acquire) == MAP_WAITING {
                std::thread::sleep(READ_NAP);
            }
        }
        let Some(buffer) = &self.readback else {
            return Err(ReadError::Failed("The readback buffer is gone".into()));
        };
        let width = self.size[0] as usize * TEXEL;
        let stride = padded_row(self.size[0]) as usize;
        {
            let view = buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|error| ReadError::Failed(error.to_string()))?;
            for (index, line) in view.chunks_exact(stride).enumerate() {
                row(index, &line[..width]);
            }
        }
        buffer.unmap();
        Ok(())
    }

    /// The bytes of the output of `slot`, row after row: what the interface
    /// samples. Waits for the GPU, so it is for tests alone.
    #[cfg(test)]
    pub fn read(&self, slot: usize) -> Vec<u8> {
        let output = self.outputs[slot].as_ref().expect("an output was drawn");
        let [width, height] = self.size;
        let row = (width * TEXEL as u32).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compose readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &output.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |result| {
            result.expect("the readback buffer maps");
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device finishes the readback");
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("the readback buffer is mapped");
        let mut bytes = Vec::with_capacity(bytes_of(self.size));
        for line in mapped.chunks_exact(row as usize) {
            bytes.extend_from_slice(&line[..width as usize * TEXEL]);
        }
        bytes
    }
}

/// The passes that draw `job` onto the canvases `pair`, of which the first
/// was cleared to what is under it, with the layers placed on the targets
/// from `layers` on. Returns the target the job ends on, one of `pair`.
fn passes(
    job: &Job<'_>,
    size: [u32; 2],
    pair: [usize; 2],
    layers: usize,
    work: &mut Vec<Work>,
) -> usize {
    let mut below = 0;
    for group in job.groups() {
        // Each layer of the group as its chain leaves it: the draw, the
        // centre of its box and the target it is in.
        let mut made = Vec::with_capacity(group.len());
        let mut base = layers;
        for draw in group {
            let place = draw.place.onto(job.canvas, size);
            let mut layer = base;
            work.push(Work::Pass(Step {
                pipeline: Pipeline::Place,
                uniform: pass::placed(size, &place, &draw.layer.transform, draw.steady.as_ref()),
                first: Source::Picture(draw.input.serial),
                second: Source::Blank,
                into: Into::Target(layer),
            }));
            let plates: Vec<_> = (draw.plates.iter())
                .map(|(effect, plate)| (*effect, plate.serial))
                .collect();
            let luts: Vec<_> = (draw.luts.iter())
                .map(|(effect, table)| (*effect, table.told()))
                .collect();
            for link in pass::chain_with(draw.layer, &place, size, &plates, &luts) {
                layer = base + link.into;
                work.push(Work::Pass(Step {
                    pipeline: Pipeline::Effect,
                    uniform: link.uniform,
                    first: Source::Target(base + link.first),
                    second: match link.second {
                        pass::Second::Blank => Source::Blank,
                        pass::Second::Slot(slot) => Source::Target(base + slot),
                        pass::Second::Plate(serial) => Source::Picture(serial),
                        pass::Second::Lut(serial) => Source::Lut(serial),
                    },
                    into: Into::Target(layer),
                }));
            }
            made.push((draw, place.center, layer));
            // The next layer of the group leaves this one where it is.
            base += pass::chain_slots(draw.layer);
        }
        // The layer the transition is on: the incoming one of a pair.
        let Some(&(top, _, mut layer)) = made.last() else {
            continue;
        };
        let mut opacity = top.layer.transform.opacity.clamp(0.0, 1.0);
        if let Some(transition) = &top.layer.transition {
            // The two sides, either of which may be missing, made one layer
            // in a target neither is in. Each is given its own opacity
            // there, so the one layer is blended whole.
            let side = |outgoing: bool| {
                (made.iter())
                    .find(|(draw, ..)| {
                        (draw.layer.transition.as_ref())
                            .is_some_and(|transition| transition.outgoing == outgoing)
                    })
                    .map(|(draw, centre, target)| {
                        let side = pass::Side {
                            opacity: draw.layer.transform.opacity,
                            centre: *centre,
                        };
                        (side, Source::Target(*target))
                    })
            };
            let (outgoing, incoming) = (side(true), side(false));
            let into = (layers..)
                .find(|target| made.iter().all(|(.., taken)| taken != target))
                .unwrap_or(layers);
            work.push(Work::Pass(Step {
                pipeline: Pipeline::Transition,
                uniform: pass::transition(
                    size,
                    transition,
                    top.layer.seed,
                    outgoing.map(|(side, _)| side),
                    incoming.map(|(side, _)| side),
                ),
                first: outgoing.map_or(Source::Blank, |(_, source)| source),
                second: incoming.map_or(Source::Blank, |(_, source)| source),
                into: Into::Target(into),
            }));
            layer = into;
            opacity = 1.0;
        }
        work.push(Work::Pass(Step {
            pipeline: Pipeline::Blend,
            uniform: Uniform::new(size)
                .variant(pass::blend_mode(top.layer.blend))
                .opacity(opacity),
            first: Source::Target(pair[below]),
            second: Source::Target(layer),
            into: Into::Target(pair[1 - below]),
        }));
        below = 1 - below;
    }
    pair[below]
}

/// What is under the lowest layer of a canvas: its colour, made linear and
/// opaque whatever its alpha says, or nothing.
fn under(background: Option<[u8; 4]>) -> wgpu::Color {
    background.map_or(wgpu::Color::TRANSPARENT, |[r, g, b, _]| {
        let linear = |byte: u8| {
            let c = f64::from(byte) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        wgpu::Color {
            r: linear(r),
            g: linear(g),
            b: linear(b),
            a: 1.0,
        }
    })
}

/// The bytes a row of `width` texels takes in a buffer a texture is copied
/// to: whole steps of what the device copies rows in.
fn padded_row(width: u32) -> u32 {
    (width * TEXEL as u32).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// Does `work` on `device` and returns what it made, or the first thing the
/// device refused while it was done: something that is not valid, a failure
/// of the driver's own such as a shader it could not compile, or memory it
/// did not have. None of those reaches the handler for errors nothing
/// caught, which ends the program. The device says so as the work is done,
/// so this waits on nothing.
fn checked<T>(device: &wgpu::Device, work: impl FnOnce() -> T) -> Result<T, String> {
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let made = work();
    // Popped in the order opposite to the one they were pushed in.
    let caught = [validation.pop(), internal.pop(), memory.pop()]
        .into_iter()
        .filter_map(pollster::block_on)
        .next();
    match caught {
        Some(error) => Err(error.to_string()),
        None => Ok(made),
    }
}

/// A pass that draws to all of `view`, which starts as `clear`.
fn begin<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    clear: wgpu::Color,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("compose pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(clear),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl Placement {
    /// From canvas pixels to those of a target of `size` drawn for a canvas
    /// of `canvas`.
    fn onto(&self, canvas: [u32; 2], size: [u32; 2]) -> Self {
        let across = size[0] as f32 / canvas[0].max(1) as f32;
        let down = size[1] as f32 / canvas[1].max(1) as f32;
        Self {
            center: [self.center[0] * across, self.center[1] * down],
            half: [self.half[0] * across, self.half[1] * down],
            angle: self.angle,
        }
    }
}
