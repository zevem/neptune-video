//! The picture of an export: each output frame drawn by the compositor from
//! the plan, at the size the output asks for.
//!
//! [`FrameRenderer`] owns a [`Gpu`] of its own on the device it is given and
//! sizes its canvases from the job alone: there is no budget to step down to
//! and no canvas count. Every layer of a frame, plain, composited and title,
//! is drawn in linear light and stacked onto one canvas as
//! [`assemble::stack`] says, so a run over a title blends only with the media
//! between them, as the viewer shows it. The canvas is read back without
//! blocking the device, which the interface may share, and placed in the
//! output picture: opaque, with black bars when letterboxed and the overflow
//! cut evenly when filled.
//!
//! The pictures come from a [`Pictures`] the job owns, asked for the frame
//! `render::source_frame` chooses, as preview asks, at as many pixels as the
//! clip covers of the drawn canvas at its largest. Titles are drawn here, on
//! the calling thread, with a font book of the export's own. Everything this
//! allocates on the device or submits to it runs inside [`Gpu::checked`] on
//! the calling thread, so what the device refuses fails the frame and never
//! reaches the handler that ends the program.
use crate::{
    compose::{
        self, Draw, MAX_PASSES,
        assemble::{self, SteadyParams},
        gpu::Gpu,
        lut::Lut,
    },
    textures::Input,
};
use editor_core::{
    Asset, AssetId, Clip, ClipId, ClipOverride, EffectKind, Layer, LayerSource, MediaKind,
    ParamKey, Project, Time, TrackKind, export::Geometry, render, resolve,
};
use editor_media::{
    Cube, Frame, MAX_CUBE_BYTES, Motion, Sampling, Steadied, SteadyCache,
    text::{self, FontBook, TitlePicture},
};
use eframe::wgpu;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

/// The decoded pictures an export draws from.
pub trait Pictures {
    /// The frame of `asset` at the source time `time`, fitted inside `size`
    /// pixels in the asset's shape: straight alpha, tightly packed RGBA.
    /// `lane` is the clip that asks; a clip's clean plate is asked for in
    /// its lane too.
    fn picture(
        &mut self,
        lane: ClipId,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<Frame, String>;
}

/// How long one frame may take the device to hand back before the export
/// gives up on it.
pub const READ_DEADLINE: Duration = Duration::from_secs(60);
/// The longest side a picture is decoded at for an export.
pub const MAX_DECODE_SIDE: u32 = 8192;
/// The shortest side a picture is decoded at, however small it is drawn.
const MIN_DECODE_SIDE: u32 = 16;
/// Samples taken between two keyframes when looking for the largest value
/// an animation reaches, which an eased curve may overshoot.
const KEY_SAMPLES: usize = 16;

/// Draws the output pictures of one export.
pub struct FrameRenderer {
    gpu: Gpu,
    geometry: Geometry,
    /// The project's canvas the plans are of.
    canvas: [u32; 2],
    fonts: FontBook,
    /// The project font files the book was last given.
    font_files: Vec<PathBuf>,
    steady: Option<SteadyCache>,
    /// The camera path of each stabilized asset, once looked for.
    motions: HashMap<AssetId, Option<Arc<Motion>>>,
    /// The table of each lookup asset with its serial, once read.
    luts: HashMap<AssetId, Option<(u64, Arc<Cube>)>>,
    /// The size each clip decodes each of its assets at.
    sizes: HashMap<(ClipId, AssetId), [u32; 2]>,
    /// The picture each clip last drew of each of its assets.
    held: HashMap<(ClipId, AssetId), Held>,
    titles: HashMap<ClipId, Title>,
    /// The last name a picture or table was given; none is given twice.
    serial: u64,
    /// Counts frames rendered, to let go what a frame did not use.
    frame: u64,
    output: Vec<u8>,
    warnings: Vec<String>,
    stop: Arc<AtomicBool>,
    deadline: Duration,
}

/// A decoded picture kept while its clip draws it.
struct Held {
    time: Time,
    size: [u32; 2],
    serial: u64,
    frame: Frame,
    used: u64,
}

/// A title's picture, made straight, kept while it is what the title shows.
struct Title {
    source: LayerSource,
    scale: f32,
    serial: u64,
    frame: Frame,
    /// The picture's left top from the anchor and its size, in canvas
    /// pixels at the layer's scale 1.
    origin: [f32; 2],
    size: [f32; 2],
    used: u64,
}

/// What a layer of a frame is drawn from.
enum Prepared<'p> {
    Media {
        asset: &'p Asset,
        serial: u64,
        frame: Frame,
        time: Time,
        plates: Vec<(Uuid, u64, Frame)>,
        luts: Vec<(Uuid, u64, Arc<Cube>)>,
        steady: Option<Sampling>,
    },
    Title {
        serial: u64,
        frame: Frame,
        origin: [f32; 2],
        size: [f32; 2],
    },
}

impl FrameRenderer {
    /// A renderer on `device` for pictures of `geometry`, of `project`, which
    /// is the export's render project: the output's rate and the caption
    /// tracks it draws. `steady` is the directory camera paths are kept in;
    /// without one every stabilized clip is drawn as it is, with a warning.
    ///
    /// Fails when the device cannot build the compositor's pipelines, or
    /// cannot make a texture the size the canvas is drawn at.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        geometry: Geometry,
        project: &Project,
        steady: Option<PathBuf>,
    ) -> Result<Self, String> {
        let canvas = [project.settings.width, project.settings.height];
        if canvas.contains(&0) || geometry.drawn.contains(&0) || geometry.output.contains(&0) {
            return Err("The export has no picture to draw".into());
        }
        let gpu = Gpu::new(device, queue)
            .map_err(|why| format!("This GPU cannot draw the export: {why}"))?;
        let longest = gpu.longest_side();
        if geometry.drawn[0].max(geometry.drawn[1]) > longest {
            return Err(format!(
                "The picture would be drawn at {} × {}, larger than this GPU draws ({longest} \
                 a side)",
                geometry.drawn[0], geometry.drawn[1]
            ));
        }
        let pixels = geometry.output[0] as usize * geometry.output[1] as usize;
        let mut renderer = Self {
            gpu,
            geometry,
            canvas,
            fonts: FontBook::new(&crate::titles::bundled_fonts()),
            font_files: Vec::new(),
            steady: steady.map(SteadyCache::at),
            motions: HashMap::new(),
            luts: HashMap::new(),
            sizes: HashMap::new(),
            held: HashMap::new(),
            titles: HashMap::new(),
            serial: 0,
            frame: 0,
            output: [0, 0, 0, u8::MAX].repeat(pixels),
            warnings: Vec::new(),
            stop: Arc::new(AtomicBool::new(false)),
            deadline: READ_DEADLINE,
        };
        renderer.set_fonts(project);
        Ok(renderer)
    }

    /// A flag that, once set, stops a frame waiting on the device: the
    /// frame then fails.
    pub fn set_stop(&mut self, stop: Arc<AtomicBool>) {
        self.stop = stop;
    }

    /// How long one frame may take the device to hand back.
    pub fn set_deadline(&mut self, deadline: Duration) {
        self.deadline = deadline;
    }

    /// What was drawn other than the viewer would show it, each said once:
    /// a stabilized clip without its camera's path, a lookup table that
    /// could not be read.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The output picture at timeline time `at`: opaque RGBA rows of the
    /// output's width, with black bars where it is letterboxed. `project`
    /// is the one given to [`Self::new`].
    pub fn render(
        &mut self,
        project: &Project,
        at: Time,
        pictures: &mut dyn Pictures,
    ) -> Result<&[u8], String> {
        if [project.settings.width, project.settings.height] != self.canvas {
            return Err("The project's canvas changed during the export".into());
        }
        self.frame += 1;
        self.set_fonts(project);
        let first = resolve(project, at, &[]);
        let mut prepared = Vec::with_capacity(first.layers.len());
        for layer in &first.layers {
            prepared.push(self.prepare(project, layer, at, pictures)?);
        }
        // A point tracked on a steadied picture is where its reading shows
        // it: what follows one is placed again with the readings known.
        let mut readings: Vec<ClipOverride> = Vec::new();
        for (layer, prepared) in first.layers.iter().zip(&prepared) {
            if let Some(Prepared::Media {
                steady: Some(sampling),
                ..
            }) = prepared
                && readings.iter().all(|held| held.clip != layer.clip)
            {
                readings.push(ClipOverride {
                    clip: layer.clip,
                    reading: Some(assemble::reading_of(sampling)),
                    ..ClipOverride::default()
                });
            }
        }
        let first_clips: Vec<ClipId> = first.layers.iter().map(|layer| layer.clip).collect();
        let follows = project
            .active_clips(at)
            .any(|(track, clip)| track.kind != TrackKind::Audio && clip.follow.is_some());
        let plan = if !readings.is_empty() && follows {
            resolve(project, at, &readings)
        } else {
            first
        };
        // The layers of the plan placed again are those of the first, and
        // are matched to what they are drawn from by their clips.
        let prepared: Vec<Option<Prepared<'_>>> = {
            let mut by_clip: HashMap<ClipId, Prepared<'_>> = HashMap::new();
            for (layer, made) in first_clips.iter().zip(prepared) {
                if let Some(made) = made {
                    by_clip.entry(*layer).or_insert(made);
                }
            }
            (plan.layers.iter())
                .map(|layer| by_clip.remove(&layer.clip))
                .collect()
        };

        let canvas = self.canvas.map(|side| side as f32);
        let titles: Vec<Option<Layer>> = (plan.layers.iter().zip(&prepared))
            .map(|(layer, prepared)| match prepared {
                Some(Prepared::Title { .. }) => Some(assemble::title_layer(layer)),
                _ => None,
            })
            .collect();
        let mut live = HashSet::new();
        let mut live_luts = HashSet::new();
        let draws: Vec<Option<Draw<'_>>> = (plan.layers.iter().enumerate())
            .map(|(index, layer)| match prepared[index].as_ref()? {
                Prepared::Media {
                    asset,
                    serial,
                    frame,
                    time,
                    plates,
                    luts,
                    steady,
                } => {
                    live.insert(*serial);
                    let source =
                        assemble::source_size(asset, Some([frame.width, frame.height]), canvas);
                    let place = assemble::layer_box(&layer.transform, source, canvas)
                        .placement(canvas, layer.transform.rotation);
                    Some(Draw {
                        layer,
                        place,
                        input: Input {
                            serial: *serial,
                            frame,
                            time: *time,
                        },
                        plates: (plates.iter())
                            .map(|(effect, serial, frame)| {
                                live.insert(*serial);
                                let input = Input {
                                    serial: *serial,
                                    frame,
                                    time: Time::ZERO,
                                };
                                (*effect, input)
                            })
                            .collect(),
                        luts: (luts.iter())
                            .map(|(effect, serial, table)| {
                                live_luts.insert(*serial);
                                let table = Lut {
                                    serial: *serial,
                                    table,
                                };
                                (*effect, table)
                            })
                            .collect(),
                        steady: *steady,
                    })
                }
                Prepared::Title {
                    serial,
                    frame,
                    origin,
                    size,
                } => {
                    live.insert(*serial);
                    Some(Draw {
                        layer: titles[index].as_ref()?,
                        place: assemble::title_placement(canvas, &layer.transform, *origin, *size),
                        input: Input {
                            serial: *serial,
                            frame,
                            time: Time::ZERO,
                        },
                        plates: Vec::new(),
                        luts: Vec::new(),
                        steady: None,
                    })
                }
            })
            .collect();
        let jobs = assemble::stack(&plan, draws);
        let passes = compose::stack_passes(&jobs);
        if passes > MAX_PASSES {
            return Err(format!(
                "The frame at {} needs {passes} compositor passes, more than the {MAX_PASSES} \
                 one frame may take",
                timecode(at)
            ));
        }
        let targets = compose::stack_targets(&jobs);
        let drawn = self.geometry.drawn;
        let asked = self.gpu.checked(|gpu| {
            gpu.hold(drawn, targets, 1);
            gpu.render_stack(0, &jobs);
            gpu.begin_read(0)
        });
        drop(jobs);
        match asked {
            Ok(Ok(())) => {}
            Ok(Err(why)) | Err(why) => {
                self.gpu.release();
                return Err(format!("The GPU could not draw the export: {why}"));
            }
        }
        let stop = Arc::clone(&self.stop);
        let geometry = self.geometry;
        let output = &mut self.output;
        let read = self.gpu.finish_read(
            &|| stop.load(Ordering::Relaxed),
            Instant::now() + self.deadline,
            &mut |row, texels| place_row(output, &geometry, row, texels),
        );
        // What this frame did not draw from is let go, here and on the
        // device.
        let frame = self.frame;
        self.held.retain(|_, held| held.used == frame);
        self.titles.retain(|_, title| title.used == frame);
        self.gpu.keep_pictures(|serial| live.contains(&serial));
        self.gpu.keep_luts(|serial| live_luts.contains(&serial));
        read.map_err(|why| why.to_string())?;
        Ok(&self.output)
    }

    /// The project's font files, for titles that name their families.
    fn set_fonts(&mut self, project: &Project) {
        let mut files: Vec<PathBuf> = (project.assets.values())
            .filter(|asset| asset.kind == MediaKind::Font)
            .map(|asset| asset.path.clone())
            .collect();
        files.sort();
        if files != self.font_files {
            self.fonts.set_project_fonts(&files);
            self.font_files = files;
            self.titles.clear();
        }
    }

    /// What `layer` of the plan at `at` is drawn from: nothing for a layer
    /// the viewer draws nothing of either.
    fn prepare<'p>(
        &mut self,
        project: &'p Project,
        layer: &Layer,
        at: Time,
        pictures: &mut dyn Pictures,
    ) -> Result<Option<Prepared<'p>>, String> {
        let asset_id = match &layer.source {
            LayerSource::Media { asset_id, .. } => *asset_id,
            LayerSource::Text { .. } | LayerSource::Graphic { .. } => {
                return self.title(layer);
            }
        };
        let Some(asset) = project.assets.get(&asset_id) else {
            return Ok(None);
        };
        let Some(clip) = clip_of(project, layer) else {
            return Ok(None);
        };
        if !is_picture(asset) {
            return Ok(None);
        }
        let time = render::source_frame(project, asset, clip, at);
        let size = self.decode_size(clip, asset);
        let (serial, frame) = self.picture(pictures, clip.id, asset, time, size)?;
        // A plate is laid over the box of its layer, and is asked for as
        // large: a still, or the first frame of anything else.
        let mut plates = Vec::new();
        for (effect, plate) in assemble::plates(project, layer) {
            if !is_picture(plate) {
                continue;
            }
            let size = fitted_box(plate, size[0].max(size[1]));
            let (serial, frame) = self.picture(pictures, clip.id, plate, Time::ZERO, size)?;
            plates.push((effect, serial, frame));
        }
        let mut luts = Vec::new();
        for (effect, table) in assemble::tables(project, layer) {
            if let Some((serial, cube)) = self.table(table) {
                luts.push((effect, serial, cube));
            }
        }
        let steady = match assemble::steadied(layer) {
            Some(effect) => (self.motion(asset)).map(|motion| {
                assemble::steady_sampling(&motion, asset, time, SteadyParams::of(effect))
            }),
            None => None,
        };
        Ok(Some(Prepared::Media {
            asset,
            serial,
            frame,
            time,
            plates,
            luts,
            steady,
        }))
    }

    /// The picture `lane` draws of `asset` at `time`: the one it drew last
    /// when that is it, otherwise a new one from `pictures` under a new
    /// serial.
    fn picture(
        &mut self,
        pictures: &mut dyn Pictures,
        lane: ClipId,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<(u64, Frame), String> {
        let frame = self.frame;
        if let Some(held) = self.held.get_mut(&(lane, asset.id))
            && held.time == time
            && held.size == size
        {
            held.used = frame;
            return Ok((held.serial, held.frame.clone()));
        }
        let picture = pictures
            .picture(lane, asset, time, size)
            .map_err(|why| format!("A frame of {} could not be read: {why}", asset.name))?;
        let whole = picture.width > 0
            && picture.height > 0
            && picture.rgba.len() == picture.width as usize * picture.height as usize * 4;
        if !whole {
            return Err(format!(
                "A frame of {} came back {} × {} with {} bytes",
                asset.name,
                picture.width,
                picture.height,
                picture.rgba.len()
            ));
        }
        self.serial += 1;
        self.held.insert(
            (lane, asset.id),
            Held {
                time,
                size,
                serial: self.serial,
                frame: picture.clone(),
                used: frame,
            },
        );
        Ok((self.serial, picture))
    }

    /// The size `clip` decodes `asset` at, worked out once: see
    /// [`decode_size`].
    fn decode_size(&mut self, clip: &Clip, asset: &Asset) -> [u32; 2] {
        let (canvas, drawn) = (self.canvas, self.geometry.drawn);
        *(self.sizes)
            .entry((clip.id, asset.id))
            .or_insert_with(|| decode_size(asset, clip, canvas, drawn))
    }

    /// The picture of a title or graphic layer, drawn at the export's scale
    /// and made straight for the compositor.
    fn title(&mut self, layer: &Layer) -> Result<Option<Prepared<'static>>, String> {
        let scale = export_title_scale(
            assemble::title_scale(&layer.transform),
            self.canvas,
            self.geometry.drawn,
        );
        let frame = self.frame;
        if let Some(title) = self.titles.get_mut(&layer.clip)
            && title.scale == scale
            && title.source == layer.source
        {
            title.used = frame;
            return Ok(Some(Prepared::Title {
                serial: title.serial,
                frame: title.frame.clone(),
                origin: title.origin,
                size: title.size,
            }));
        }
        let fonts = &mut self.fonts;
        let drawn = catch_unwind(AssertUnwindSafe(|| {
            text::draw_title(fonts, &layer.source, scale)
        }));
        let picture = match drawn {
            Ok(Some(picture)) => picture,
            Ok(None) => return Ok(None),
            Err(_) => {
                // Whatever the book was left holding is not trusted again.
                self.fonts = FontBook::new(&crate::titles::bundled_fonts());
                self.fonts.set_project_fonts(&self.font_files);
                return Err("A title could not be drawn".into());
            }
        };
        self.serial += 1;
        let title = Title {
            source: layer.source.clone(),
            scale,
            serial: self.serial,
            frame: straight(&picture),
            origin: picture.origin,
            size: picture.size(),
            used: frame,
        };
        let prepared = Prepared::Title {
            serial: title.serial,
            frame: title.frame.clone(),
            origin: title.origin,
            size: title.size,
        };
        self.titles.insert(layer.clip, title);
        Ok(Some(prepared))
    }

    /// The table of a lookup asset with its serial, read once. One that is
    /// not a table or cannot be read changes nothing, as in the viewer, and
    /// is said.
    fn table(&mut self, asset: &Asset) -> Option<(u64, Arc<Cube>)> {
        if asset.kind != MediaKind::Lut {
            return None;
        }
        if let Some(known) = self.luts.get(&asset.id) {
            return known.clone();
        }
        let known = match read_cube(&asset.path) {
            Ok(cube) => {
                self.serial += 1;
                Some((self.serial, Arc::new(cube)))
            }
            Err(why) => {
                self.warn(format!(
                    "The LUT {} could not be read ({why}); it is left out of the export.",
                    asset.name
                ));
                None
            }
        };
        self.luts.insert(asset.id, known.clone());
        known
    }

    /// The camera's path through a stabilized video, looked for once. A
    /// video whose path is not kept is drawn as it is, as in the viewer,
    /// and that is said.
    fn motion(&mut self, asset: &Asset) -> Option<Arc<Motion>> {
        if let Some(known) = self.motions.get(&asset.id) {
            return known.clone();
        }
        let found = match &self.steady {
            Some(cache) => cache.load(asset),
            None => Steadied::Missing,
        };
        let known = match found {
            Steadied::Ready(motion) => Some(motion),
            Steadied::Stale => {
                self.warn(format!(
                    "{} is exported without stabilization: the file changed since its camera \
                     was measured.",
                    asset.name
                ));
                None
            }
            Steadied::Missing => {
                self.warn(format!(
                    "{} is exported without stabilization: its camera has not been measured.",
                    asset.name
                ));
                None
            }
        };
        self.motions.insert(asset.id, known.clone());
        known
    }

    fn warn(&mut self, warning: String) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }
}

/// The clip a layer of the plan is of: one on the timeline at the layer's
/// track, including one a transition holds past its end.
fn clip_of<'p>(project: &'p Project, layer: &Layer) -> Option<&'p Clip> {
    let track = project.track(layer.track)?;
    track.clips.iter().find(|clip| clip.id == layer.clip)
}

/// Whether an asset has a picture to draw.
fn is_picture(asset: &Asset) -> bool {
    matches!(
        asset.kind,
        MediaKind::Video | MediaKind::Image | MediaKind::Gif
    )
}

/// The size `clip` decodes the pictures of `asset` at, for a canvas of
/// `canvas` pixels drawn at `drawn`: as many pixels as the clip shows of
/// the picture where it is largest, in the asset's shape, and never more
/// than the asset has or [`MAX_DECODE_SIDE`] a side.
///
/// The largest is found over the clip's whole animation, once, so the clip
/// asks at one size for as long as it is drawn: its scale, crop and the crop
/// of a stabilization zoom it in. A clip whose scale follows a tracker is
/// decoded whole. An asset whose size is not known asks for a square as
/// large as the canvas is drawn.
pub fn decode_size(asset: &Asset, clip: &Clip, canvas: [u32; 2], drawn: [u32; 2]) -> [u32; 2] {
    let transform = &clip.transform;
    let canvas = canvas.map(|side| f64::from(side.max(1)));
    let drawn_long = f64::from(drawn[0].max(drawn[1]).max(1));
    let per_canvas = drawn_long / canvas[0].max(canvas[1]);
    let scale = if clip.follow.as_ref().is_some_and(|follow| follow.scale) {
        None
    } else {
        Some(
            largest(clip, None, ParamKey::ScaleX, transform.scale[0]).max(largest(
                clip,
                None,
                ParamKey::ScaleY,
                transform.scale[1],
            )),
        )
    };
    let zoom = (clip.effects.iter())
        .filter(|effect| effect.enabled && effect.kind == EffectKind::Stabilize)
        .map(|effect| {
            let plain = effect.param(ParamKey::CropZoom).unwrap_or(1.0);
            largest(clip, Some(effect.id), ParamKey::CropZoom, plain)
        })
        .fold(1.0_f64, |most, zoom| most.max(zoom))
        .clamp(1.0, 4.0);
    if asset.width == 0 || asset.height == 0 {
        let side = drawn_long * scale.unwrap_or(1.0) * zoom;
        let side = covering(side).clamp(MIN_DECODE_SIDE, MAX_DECODE_SIDE);
        return [side, side];
    }
    let keys = [
        ParamKey::CropLeft,
        ParamKey::CropTop,
        ParamKey::CropRight,
        ParamKey::CropBottom,
    ];
    let crop: [f64; 4] =
        std::array::from_fn(|side| largest(clip, None, keys[side], transform.crop[side]));
    let native = [f64::from(asset.width), f64::from(asset.height)];
    let cropped = [
        native[0] * (1.0 - crop[0] - crop[2]).max(0.001),
        native[1] * (1.0 - crop[1] - crop[3]).max(0.001),
    ];
    // Canvas pixels to a picture pixel, where the cropped picture is fitted
    // inside the canvas at scale 1.
    let fit = (canvas[0] / cropped[0]).min(canvas[1] / cropped[1]);
    let long = native[0].max(native[1]);
    let shown = scale.map_or(1.0, |scale| fit * scale * per_canvas * zoom);
    let most = f64::from(MAX_DECODE_SIDE) / long;
    let least = f64::from(MIN_DECODE_SIDE) / long;
    let k = shown.min(1.0).min(most).max(least.min(1.0));
    native.map(|side| covering(side * k).min(MAX_DECODE_SIDE))
}

/// Whole pixels that cover `pixels`, a hair under a whole number being that
/// number: what an exact fit comes to after rounding.
fn covering(pixels: f64) -> u32 {
    ((pixels - 1e-6).ceil() as u32).max(1)
}

/// The largest magnitude the clip's `key` reaches, of its own (`target`
/// none) or of one of its effects: `plain` where nothing animates it.
fn largest(clip: &Clip, target: Option<Uuid>, key: ParamKey, plain: f32) -> f64 {
    let Some(track) = (clip.animation.iter())
        .find(|track| track.target == target && track.key == key && !track.keys.is_empty())
    else {
        return f64::from(plain.abs());
    };
    let mut most = 0.0_f32;
    for pair in track.keys.windows(2) {
        let (from, to) = (pair[0].at, pair[1].at);
        for step in 0..=KEY_SAMPLES {
            let at = from + Time((to.0 - from.0) * step as i64 / KEY_SAMPLES as i64);
            most = most.max(track.sample(at).abs());
        }
    }
    for key in &track.keys {
        most = most.max(key.value.abs());
    }
    f64::from(most)
}

/// A box of `long` pixels on its longer side in the shape of `asset`'s
/// picture, never larger than the picture: what a clean plate is decoded at.
fn fitted_box(asset: &Asset, long: u32) -> [u32; 2] {
    let long = long.clamp(1, MAX_DECODE_SIDE);
    if asset.width == 0 || asset.height == 0 {
        return [long, long];
    }
    let (width, height) = (f64::from(asset.width), f64::from(asset.height));
    let k = (f64::from(long) / width.max(height)).min(1.0);
    [width, height].map(|side| covering(side * k))
}

/// The picture pixels a title is drawn at for a canvas pixel of the export:
/// its scale on the canvas by the drawn canvas's pixels to one of the
/// project's, on the steps titles are drawn at and never under what it
/// covers.
fn export_title_scale(scale: f32, canvas: [u32; 2], drawn: [u32; 2]) -> f32 {
    let per_canvas = drawn[0].max(drawn[1]) as f32 / canvas[0].max(canvas[1]).max(1) as f32;
    let wanted = scale * per_canvas;
    let step = text::quantize_scale(wanted);
    if step < wanted {
        (step * 2_f32.powf(1.0 / 8.0)).min(text::MAX_TITLE_SCALE)
    } else {
        step
    }
}

/// A title's premultiplied picture as the compositor reads a decoded frame:
/// straight alpha.
fn straight(picture: &TitlePicture) -> Frame {
    let mut rgba = picture.rgba.to_vec();
    for texel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(texel[3]);
        if alpha == 0 {
            texel[..3].fill(0);
        } else if alpha < 255 {
            for channel in &mut texel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    Frame {
        width: picture.width,
        height: picture.height,
        rgba: rgba.into(),
    }
}

/// Reads a `.cube` file, held to the size the media worker reads.
fn read_cube(path: &Path) -> Result<Cube, String> {
    let file = std::fs::File::open(path).map_err(|why| why.to_string())?;
    let length = file.metadata().map_err(|why| why.to_string())?.len();
    if length > MAX_CUBE_BYTES {
        return Err("the file is larger than a LUT is read".into());
    }
    let mut text = String::with_capacity(length as usize);
    file.take(MAX_CUBE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|why| why.to_string())?;
    if text.len() as u64 > MAX_CUBE_BYTES {
        return Err("the file is larger than a LUT is read".into());
    }
    Cube::parse(&text).map_err(|why| why.to_string())
}

/// Puts row `row` of the drawn canvas into the output where the geometry
/// places it, cut to the output, and opaque.
fn place_row(output: &mut [u8], geometry: &Geometry, row: usize, texels: &[u8]) {
    let [width, height] = geometry.output.map(i64::from);
    let [left, top] = geometry.offset.map(i64::from);
    let y = row as i64 + top;
    if y < 0 || y >= height {
        return;
    }
    let drawn = (texels.len() / 4) as i64;
    let (from, to) = (left.max(0), (left + drawn).min(width));
    if to <= from {
        return;
    }
    let source = &texels[((from - left) * 4) as usize..((to - left) * 4) as usize];
    let start = ((y * width + from) * 4) as usize;
    let into = &mut output[start..start + source.len()];
    into.copy_from_slice(source);
    for texel in into.chunks_exact_mut(4) {
        texel[3] = u8::MAX;
    }
}

/// A timeline time as a person reads it in a message.
fn timecode(at: Time) -> String {
    let seconds = at.as_seconds();
    format!("{}:{:06.3}", (seconds / 60.0).floor(), seconds % 60.0)
}

/// A device of the export's own, for a host whose window has none: the
/// adapter the host prefers, else its software one.
pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// Opens a device to export on without a window. Blocks while the driver
/// opens it, so it belongs on the render thread.
pub fn headless_device() -> Result<Headless, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut why = String::from("no adapter was found");
    for force_fallback_adapter in [false, true] {
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter,
                ..Default::default()
            })) {
                Ok(adapter) => adapter,
                Err(error) => {
                    why = error.to_string();
                    continue;
                }
            };
        // Every desktop device makes textures of 8192 a side; one that
        // cannot is asked for what it has.
        let limits = [
            wgpu::Limits::default(),
            wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ];
        for required_limits in limits {
            let descriptor = wgpu::DeviceDescriptor {
                label: Some("aster export"),
                required_limits,
                ..Default::default()
            };
            match pollster::block_on(adapter.request_device(&descriptor)) {
                Ok((device, queue)) => {
                    return Ok(Headless { device, queue });
                }
                Err(error) => why = error.to_string(),
            }
        }
    }
    Err(format!("No GPU could be opened to export with: {why}"))
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod frame_tests;
