//! What a layer of a plan is drawn from and where, worked out once for the
//! viewer and the export alike: the box a media layer's picture fills, the
//! clean plates and tables its effects read, how a stabilized picture is
//! read, where a title's picture goes, and the canvases a whole frame is
//! stacked from.
//!
//! Nothing here touches a device, a decoder or the interface. The viewer
//! hands in the pictures it has on screen and the export those it decoded;
//! both get the same boxes and jobs, so the two cannot drift apart.
use super::{Draw, Job, Placement, Run, runs};
use editor_core::{
    Asset, BlendMode, EffectKind, Layer, ParamKey, Project, Reading, RenderPlan, ResolvedEffect,
    Time, Transform, render,
};
use editor_media::{Motion, Sampling};
use std::ops::Range;
use uuid::Uuid;

/// The size of the picture a media layer's box is fitted from, in pixels:
/// the asset's own, so a frame decoded at any size of the same shape lands
/// in the same box; the decoded picture's when the asset's is not known;
/// and the canvas's when neither is.
pub fn source_size(asset: &Asset, picture: Option<[u32; 2]>, canvas: [f32; 2]) -> [f32; 2] {
    if asset.width > 0 && asset.height > 0 {
        [asset.width as f32, asset.height as f32]
    } else if let Some([width, height]) = picture {
        [width as f32, height as f32]
    } else {
        canvas
    }
}

/// Where a media layer's box is: its centre's place from the canvas's centre
/// and its size, both in canvas pixels. Its cropped picture is fitted inside
/// the canvas and then scaled by the clip.
pub fn layer_box(transform: &Transform, source: [f32; 2], canvas: [f32; 2]) -> LayerBox {
    let fitted = render::fitted(source, transform.crop, canvas);
    LayerBox {
        position: transform.position,
        size: [
            fitted[0] * transform.scale[0],
            fitted[1] * transform.scale[1],
        ],
    }
}

/// A media layer's box on the canvas: see [`layer_box`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerBox {
    /// The box's centre, from the canvas's centre, y down.
    pub position: [f32; 2],
    /// Its width and height, negative where the clip's scale is.
    pub size: [f32; 2],
}

impl LayerBox {
    /// The box as the compositor places it, turned by `rotation` degrees
    /// clockwise, on a canvas of `canvas` pixels.
    pub fn placement(&self, canvas: [f32; 2], rotation: f32) -> Placement {
        Placement {
            center: [
                canvas[0] * 0.5 + self.position[0],
                canvas[1] * 0.5 + self.position[1],
            ],
            half: [self.size[0] * 0.5, self.size[1] * 0.5],
            angle: rotation.to_radians(),
        }
    }
}

/// The clean plate of each difference matte among a layer's effects, by the
/// effect's ID, when the library has the plate's asset.
pub fn plates<'p>(project: &'p Project, layer: &Layer) -> Vec<(Uuid, &'p Asset)> {
    effect_assets(project, layer, EffectKind::DifferenceMatte)
}

/// The table asset of each lookup among a layer's effects, by the effect's
/// ID, when the library has it.
pub fn tables<'p>(project: &'p Project, layer: &Layer) -> Vec<(Uuid, &'p Asset)> {
    effect_assets(project, layer, EffectKind::Lut)
}

fn effect_assets<'p>(
    project: &'p Project,
    layer: &Layer,
    kind: EffectKind,
) -> Vec<(Uuid, &'p Asset)> {
    (layer.effects.iter())
        .filter(|effect| effect.kind == kind)
        .filter_map(|effect| Some((effect.id, project.assets.get(&effect.asset_id?)?)))
        .collect()
}

/// The stabilization of a layer, when it has one.
pub fn steadied(layer: &Layer) -> Option<&ResolvedEffect> {
    (layer.effects.iter()).find(|effect| effect.kind == EffectKind::Stabilize)
}

/// What a stabilization is asked for: how smooth the camera's path is made,
/// the crop that hides the borders the correction brings in, and how much
/// of the correction is taken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteadyParams {
    pub smoothness: f32,
    pub zoom: f32,
    pub mix: f32,
}

impl SteadyParams {
    pub fn of(effect: &ResolvedEffect) -> Self {
        let value = |key: ParamKey, plain: f32| effect.param(key).unwrap_or(plain);
        Self {
            smoothness: value(ParamKey::Smoothness, 0.0),
            zoom: value(ParamKey::CropZoom, 1.0),
            mix: effect.mix,
        }
    }
}

/// How the picture of `asset` decoded for the source time `at` is read to
/// steady it along `motion`: the correction of that frame, as much of it as
/// the mix says and as the crop hides the borders of, with the crop.
pub fn steady_sampling(motion: &Motion, asset: &Asset, at: Time, params: SteadyParams) -> Sampling {
    let aspect = asset.width.max(1) as f32 / asset.height.max(1) as f32;
    let zoom = params.zoom.clamp(1.0, 4.0);
    let fix = (motion.fix_at(at, params.smoothness))
        .scaled(params.mix.clamp(0.0, 1.0))
        .fitted(zoom, aspect);
    fix.sampling(zoom, aspect)
}

/// A steadied picture's reading, as the plan is told of it: a point tracked
/// on the picture is placed where the reading shows it.
pub fn reading_of(sampling: &Sampling) -> Reading {
    Reading {
        matrix: sampling.matrix,
        offset: sampling.offset,
    }
}

/// What of a layer of a transition is shown where no compositor draws the
/// transition, whatever its kind: the incoming layer fades in and the
/// outgoing one fades out. `under` is the opacity of the partner drawn over
/// an outgoing layer on the very same box, when there is one: the outgoing
/// layer is then left as much more as its partner hides of it, so that the
/// two sum to a cross-fade and the canvas does not show through half way.
pub fn fade_weight(layer: &Layer, under: Option<f32>) -> f32 {
    let Some(transition) = &layer.transition else {
        return 1.0;
    };
    let progress = transition.progress.clamp(0.0, 1.0);
    match (transition.outgoing, under) {
        (false, _) => progress,
        (true, None) => 1.0 - progress,
        (true, Some(over)) => {
            let hidden = progress * over.clamp(0.0, 1.0);
            ((1.0 - progress) / (1.0 - hidden).max(f32::EPSILON)).min(1.0)
        }
    }
}

/// Where a point of a title is: `x` and `y` canvas pixels from its anchor
/// at the layer's scale 1, which is at `anchor`, drawn `view` pixels to a
/// canvas pixel. The layer's scale, flips and turn are about the anchor.
pub fn title_point(anchor: [f32; 2], view: f32, transform: &Transform, x: f32, y: f32) -> [f32; 2] {
    let flip = |flipped: bool| if flipped { -1.0 } else { 1.0 };
    let local = [
        x * transform.scale[0] * flip(transform.flip_x) * view,
        y * transform.scale[1] * flip(transform.flip_y) * view,
    ];
    let (sin, cos) = transform.rotation.to_radians().sin_cos();
    [
        anchor[0] + (local[0] * cos - local[1] * sin),
        anchor[1] + (local[0] * sin + local[1] * cos),
    ]
}

/// How many times larger than at scale 1 a title is drawn: by the larger of
/// its scales, so its picture is drawn with as many pixels as it covers.
pub fn title_scale(transform: &Transform) -> f32 {
    transform.scale[0].abs().max(transform.scale[1].abs())
}

/// Where the compositor places a title's picture on a canvas of `canvas`
/// pixels: the picture's box, `origin` from the anchor and `size` large in
/// canvas pixels at scale 1, through the layer's transform as the viewer
/// draws it. The box's flips are those of [`title_layer`].
pub fn title_placement(
    canvas: [f32; 2],
    transform: &Transform,
    origin: [f32; 2],
    size: [f32; 2],
) -> Placement {
    let anchor = [
        canvas[0] * 0.5 + transform.position[0],
        canvas[1] * 0.5 + transform.position[1],
    ];
    let middle = [origin[0] + size[0] * 0.5, origin[1] + size[1] * 0.5];
    Placement {
        center: title_point(anchor, 1.0, transform, middle[0], middle[1]),
        half: [
            size[0] * transform.scale[0].abs() * 0.5,
            size[1] * transform.scale[1].abs() * 0.5,
        ],
        angle: transform.rotation.to_radians(),
    }
}

/// A title's layer as the compositor draws it, the way the viewer shows a
/// title: its picture over what is under it with normal blending, at its
/// opacity and faded through a transition of any kind, with no effects or
/// masks. A negative scale mirrors the picture, so it is drawn as a flip of
/// a box of the scale's size; nothing of it is cropped.
pub fn title_layer(layer: &Layer) -> Layer {
    let mut transform = layer.transform.clone();
    transform.opacity = transform.opacity.clamp(0.0, 1.0) * fade_weight(layer, None);
    transform.crop = [0.0; 4];
    transform.flip_x = transform.flip_x != (transform.scale[0] < 0.0);
    transform.flip_y = transform.flip_y != (transform.scale[1] < 0.0);
    Layer {
        transform,
        blend: BlendMode::Normal,
        effects: Vec::new(),
        masks: Vec::new(),
        transition: None,
        ..layer.clone()
    }
}

/// What is under the lowest layer of a composited run of a plan: the
/// canvas's colour for the run that starts the plan, nothing for one drawn
/// over a title.
pub fn background(plan: &RenderPlan, range: &Range<usize>) -> Option<[u8; 4]> {
    (range.start == 0).then_some(plan.background)
}

/// The canvases a whole frame of `plan` is drawn on, bottom first, as
/// [`super::gpu::Gpu::render_stack`] draws them: every layer composited, as
/// the viewer shows them.
///
/// `draws` holds one entry for each layer of the plan, its draw or nothing
/// for a layer without a picture; a title's is drawn through
/// [`title_layer`]. The first canvas has the plan's colour under it. A
/// composited run that starts over a title starts a clear canvas of its own,
/// which is then laid over the frame: its layers blend with the media layers
/// between it and the title and with nothing lower, as [`runs`] says. Plain
/// layers and titles are laid over the canvas below them with normal
/// blending, which is the same as laying them over the frame.
pub fn stack<'a>(plan: &RenderPlan, mut draws: Vec<Option<Draw<'a>>>) -> Vec<Job<'a>> {
    draws.resize_with(plan.layers.len(), || None);
    let mut jobs = vec![Job {
        canvas: plan.canvas,
        background: Some(plan.background),
        draws: Vec::new(),
    }];
    for run in runs(plan) {
        let range = match run {
            Run::Title(index) => index..index + 1,
            Run::Plain(range) => range,
            Run::Composite(range) => {
                // The lowest run is drawn on the canvas's colour, which the
                // first canvas already has.
                if background(plan, &range).is_none() {
                    jobs.push(Job {
                        canvas: plan.canvas,
                        background: None,
                        draws: Vec::new(),
                    });
                }
                range
            }
        };
        let canvas = jobs.last_mut().expect("there is a canvas");
        canvas
            .draws
            .extend(range.filter_map(|index| draws[index].take()));
    }
    jobs
}
