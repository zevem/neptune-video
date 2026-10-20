//! The viewer: the canvas composited from decoded frames, with the selected
//! clip's handles over it and the transport under it.
//!
//! What is shown is the plan `editor_core::resolve` makes of the project at
//! the playhead: the layers in track order, each with its transform as its
//! keyframes and tracker leave it. A plain layer is a textured quad; a layer
//! with a blend mode, masks, effects or a transition is drawn by the
//! compositor, when the window has a device for one. Decoding happens
//! elsewhere; a layer shows its last frame until the next one arrives.
mod tracking;

use super::{
    Action, ClipDraft, ColourPick, Selection, TrackRegion, Transport, icon_of_media, inspector,
    shortcut, tone_of_media, widgets,
};
use crate::{
    compose::{
        self, Composer, Draw, Job, Run,
        assemble::{self, SteadyParams, fade_weight, reading_of, steadied},
        lut::Luts,
    },
    icons::{self, Icon},
    record::{self, RecordView},
    steady::Steadies,
    textures::{Textures, Viewing, Wanted},
    theme::{self, Palette, metrics},
    timefmt,
    titles::Titles,
};
use editor_core::{
    AnimTarget, Asset, AssetId, Clip, ClipId, ClipOverride, FrameRate, Layer, LayerSource, Mask,
    MaskShape, MediaKind, ParamKey, Project, ProjectQuery, Reading, RenderPlan, Time, Track,
    TrackKind, Transform, look, render, resolve,
};
// The tests below build their clips from it.
#[cfg(test)]
use editor_core::ClipContent;
use editor_media::{MediaService, Sampling, text::TitlePicture};
use eframe::{
    egui::{
        self, Align2, Color32, CornerRadius, CursorIcon, Id, Mesh, Pos2, Rect, Sense, Shape,
        Stroke, StrokeKind, Ui, Vec2, epaint::RectShape, pos2, vec2,
    },
    egui_wgpu::RenderState,
};
use std::time::{Duration, Instant};

const TRANSPORT: f32 = 48.0;
const SOURCE_HEADER: f32 = 40.0;
const SCRUB: f32 = 22.0;
const HANDLE: f32 = 9.0;
const ROTATE_STEM: f32 = 26.0;
/// Decode sizes a layer may ask for. A few fixed steps, so that resizing the
/// window does not ask the decoder for a new size every frame.
const DECODE_STEPS: [u32; 6] = [320, 480, 640, 960, 1280, 1920];

#[derive(Clone, Copy, Debug)]
enum Hold {
    Move {
        from: Pos2,
        position: [f32; 2],
        /// How far from its stored position the layer is shown, in canvas
        /// pixels: nought unless it follows a tracker.
        shown: Vec2,
    },
    Scale {
        distance: f32,
        scale: [f32; 2],
    },
    Rotate {
        angle: f32,
        rotation: f32,
    },
}

/// How long the compositor's canvases, the tables and the decoded frames
/// kept for it outlast the last frame that had something to composite. Long
/// enough to look away at another view and back, and short beside what they
/// hold: an editor left on a plain stretch, or asleep, gives them up.
pub const COMPOSE_IDLE: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct ViewerState {
    hold: Option<(ClipId, Hold)>,
    /// Where the playhead was in the held clip when it was taken hold of: a
    /// drag of a value keyframes drive writes its keyframe there.
    hold_at: Time,
    composer: Composer,
    /// The lookup tables the layers on screen are drawn with.
    luts: Luts,
    /// The viewer was drawn in this frame of the application with something
    /// for the compositor to draw.
    composited: bool,
    /// When it first was not, while something was still held for it.
    idle_since: Option<Instant>,
    /// The mask of the selected clip that is outlined on the canvas.
    mask: Option<uuid::Uuid>,
    /// The clean plates the frame last drawn asked for, each with the taking
    /// of it a canvas was drawn from, when it had come.
    plates: Vec<(Viewing, Option<u64>)>,
    /// The key colour the next click on the canvas picks, and whether one
    /// was picked since that was last asked.
    pick: Option<ColourPick>,
    picked: bool,
    /// The animation the timeline's keyframe editor holds in a drag.
    keys: Option<ClipOverride>,
    /// The box being chosen for a tracker, how the pointer holds it, and
    /// whether a drag changed it since that was last asked.
    region: Option<TrackRegion>,
    region_hold: Option<tracking::RegionHold>,
    region_changed: bool,
    /// Whether the paths of the selected clip's trackers are drawn.
    paths: bool,
    /// The camera paths the stabilized layers are steadied with.
    pub steady: Steadies,
    /// How each steadied layer of the last frame read its picture.
    readings: Vec<ClipOverride>,
    /// The pictures titles and graphics are drawn from.
    pub titles: Titles,
}

impl ViewerState {
    /// A viewer that composites on the window's own device, when it has one.
    pub fn with_device(state: Option<RenderState>) -> Self {
        Self {
            composer: Composer::new(state),
            ..Self::default()
        }
    }

    /// A viewer that draws its titles with `titles`.
    pub fn with_titles(self, titles: Titles) -> Self {
        Self { titles, ..self }
    }

    pub fn cancel(&mut self) -> bool {
        self.hold.take().is_some()
    }

    /// Outlines this mask of the selected clip on the canvas, or none.
    pub fn outline_mask(&mut self, mask: Option<uuid::Uuid>) {
        self.mask = mask;
    }

    /// Draws a clip with the animation a drag of its keyframes holds, or as
    /// it is stored.
    pub fn preview_keys(&mut self, held: Option<ClipOverride>) {
        self.keys = held;
    }

    /// Says which effect's key colour a click on the canvas picks, or none.
    pub fn pick_colour(&mut self, pick: Option<ColourPick>) {
        self.pick = pick;
    }

    /// Says which region is being chosen on the canvas for a tracker, or
    /// that none is. A drag draws or moves it.
    pub fn choose_region(&mut self, region: Option<TrackRegion>) {
        if region.is_none() {
            self.region_hold = None;
        }
        self.region = region;
    }

    /// How the steadied layers of the frame drawn last read their pictures,
    /// as overrides that hold nothing else: a point tracked on one of them
    /// is on the canvas where its reading shows it.
    pub fn readings(&self) -> &[ClipOverride] {
        &self.readings
    }

    /// The region as a drag left it, when one changed it since this was
    /// last asked.
    pub fn take_region(&mut self) -> Option<TrackRegion> {
        std::mem::take(&mut self.region_changed)
            .then_some(self.region)
            .flatten()
    }

    /// Draws the paths of the trackers the selected clip has to do with.
    pub fn show_paths(&mut self, paths: bool) {
        self.paths = paths;
    }

    /// Whether a click picked a colour since this was last asked: the pick
    /// is over.
    pub fn take_picked(&mut self) -> bool {
        std::mem::take(&mut self.picked)
    }

    /// Begins a frame of the application, whether or not the viewer is drawn
    /// in it: canvases nothing has asked for in a while are given up.
    pub fn begin_frame(&mut self) {
        self.composer.begin_frame();
        self.luts.begin_frame();
        self.steady.begin_frame();
        self.titles.begin_frame();
        self.composited = false;
        // The plates are those of the frame drawn last: a frame the viewer
        // is not drawn in composites nothing that could be stale.
        self.plates.clear();
    }

    /// Ends a frame of the application at `now`. What is held for a
    /// compositor that had nothing to draw in it is given up once that has
    /// lasted [`COMPOSE_IDLE`]: the canvases and the pictures they were
    /// drawn from, the tables, and the decoded frames `textures` keeps
    /// beside its layers. They are counted in frames as well, as an undrawn
    /// viewer layer is, but an editor that has stopped painting counts none.
    ///
    /// Returns how long from now this is to be called again, when something
    /// is held that long and nothing else would paint a frame by then: one
    /// wake to give it up in, and none once it is given up.
    pub fn rest(&mut self, textures: &mut Textures, now: Instant) -> Option<Duration> {
        let holds = self.composer.holds() || self.luts.holds() || textures.keeps_frames();
        if self.composited || !holds {
            self.idle_since = None;
            return None;
        }
        let since = *self.idle_since.get_or_insert(now);
        let left = COMPOSE_IDLE.saturating_sub(now.saturating_duration_since(since));
        if left.is_zero() {
            self.idle_since = None;
            self.release_canvases();
            textures.keep_frames(false);
            return None;
        }
        Some(left)
    }

    /// Gives up the canvases and the pictures they were drawn from: the
    /// project is another.
    pub fn release_canvases(&mut self) {
        self.composer.release();
        self.luts.clear();
    }

    /// Gives up the lookup table of an asset that has left the library or
    /// is now of another file.
    pub fn forget_lut(&mut self, asset: AssetId) {
        self.luts.forget(asset);
    }

    /// Takes a lookup table the media worker read: see [`Luts::take`].
    pub fn take_lut(
        &mut self,
        now: Option<&Asset>,
        asset_id: AssetId,
        generation: u64,
        path: &std::path::Path,
        file_size: u64,
        table: Result<std::sync::Arc<editor_media::Cube>, String>,
    ) -> compose::lut::Taken {
        self.luts
            .take(now, asset_id, generation, path, file_size, table)
    }

    /// Whether a canvas composited in this frame was drawn from a picture
    /// its layer has since replaced. A frame taken at the end of a frame is
    /// in its layer's texture when that frame is painted, but a canvas was
    /// composited before it came: it wants one more frame.
    pub fn canvas_is_stale(&self, textures: &Textures) -> bool {
        self.composer.drawn().any(|(clip, serial)| {
            textures
                .input(Viewing::Clip(clip))
                .is_some_and(|input| input.serial != serial)
        }) || (self.plates.iter()).any(|(plate, drawn)| {
            // A clean plate that came, or changed, after its matte was drawn.
            // One whose frame has been given up is not waited for: nothing
            // is drawn from it until the viewer asks for it again.
            (textures.input(*plate)).is_some_and(|input| Some(input.serial) != *drawn)
        })
    }
}

pub struct ViewerInput<'a> {
    pub p: &'a Palette,
    pub project: &'a Project,
    pub query: Option<&'a ProjectQuery>,
    pub playhead: Time,
    pub playing: bool,
    pub speed: f64,
    pub looping: bool,
    pub media: Option<&'a MediaService>,
    /// The voiceover take, as the record control shows it.
    pub record: RecordView,
}

impl ViewerInput<'_> {
    /// Bounds belong to an exact immutable root. A same-ID replacement must
    /// never be evaluated through the snapshot it replaced.
    fn indexed(&self) -> Option<&ProjectQuery> {
        self.query
            .filter(|query| std::ptr::eq(query.project(), self.project))
    }

    fn resolve(&self, overrides: &[ClipOverride]) -> RenderPlan {
        match self.indexed() {
            Some(query) => query.resolve(self.playhead, overrides),
            None => resolve(self.project, self.playhead, overrides),
        }
    }

    fn active_clips(&self) -> impl Iterator<Item = (&Track, &Clip)> {
        let indexed = self.indexed();
        let fallback = indexed.is_none().then_some(self.project);
        indexed
            .into_iter()
            .flat_map(|query| query.active_clips(self.playhead))
            .chain(
                fallback
                    .into_iter()
                    .flat_map(|project| project.active_clips(self.playhead)),
            )
            .filter(|(track, _)| track.kind != TrackKind::Audio)
    }
}

/// A layer placed on screen.
#[derive(Clone, Copy)]
struct Placed {
    clip: ClipId,
    center: Pos2,
    half: Vec2,
    /// Radians, clockwise.
    angle: f32,
    locked: bool,
}

impl Placed {
    fn corner(&self, x: f32, y: f32) -> Pos2 {
        let (sin, cos) = self.angle.sin_cos();
        let local = vec2(x * self.half.x, y * self.half.y);
        self.center + vec2(local.x * cos - local.y * sin, local.x * sin + local.y * cos)
    }
    fn contains(&self, pos: Pos2) -> bool {
        let (sin, cos) = (-self.angle).sin_cos();
        let d = pos - self.center;
        let local = vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
        local.x.abs() <= self.half.x && local.y.abs() <= self.half.y
    }
    fn rotate_handle(&self) -> Pos2 {
        let top = self.corner(0.0, -1.0);
        let (sin, cos) = self.angle.sin_cos();
        top + vec2(sin, -cos) * ROTATE_STEM
    }
}

/// The edge of a mask of the layer placed at `place`, before its feather: a
/// mask's place and size are fractions of the layer's box, and it turns
/// about its own centre within that box.
fn mask_outline(place: &Placed, mask: &Mask, color: Color32) -> Shape {
    let centre = vec2(
        (mask.center[0] - 0.5) * 2.0 * place.half.x,
        (mask.center[1] - 0.5) * 2.0 * place.half.y,
    );
    let half = vec2(mask.size[0] * place.half.x, mask.size[1] * place.half.y);
    let turned = |v: Vec2, angle: f32| {
        let (sin, cos) = angle.sin_cos();
        vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
    };
    let on_screen = |local: Vec2| {
        place.center
            + turned(
                centre + turned(local, mask.rotation.to_radians()),
                place.angle,
            )
    };
    let points: Vec<Pos2> = match mask.shape {
        MaskShape::Rectangle => [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .into_iter()
            .map(|(x, y)| on_screen(vec2(x * half.x, y * half.y)))
            .collect(),
        MaskShape::Ellipse => (0..MASK_OUTLINE_POINTS)
            .map(|step| {
                let angle = step as f32 * std::f32::consts::TAU / MASK_OUTLINE_POINTS as f32;
                on_screen(vec2(angle.cos() * half.x, angle.sin() * half.y))
            })
            .collect(),
    };
    Shape::closed_line(points, Stroke::new(1.5, color))
}

/// The straight pieces an ellipse mask's outline is drawn in.
const MASK_OUTLINE_POINTS: usize = 64;

/// The size a media layer takes on the canvas, in canvas pixels, before the
/// clip's own scale: its cropped picture fitted inside the canvas.
pub(crate) fn fitted_size(source: Vec2, crop: [f32; 4], canvas: Vec2) -> Vec2 {
    render::fitted(source.into(), crop, canvas.into()).into()
}

/// The box a layer asks its frames to fit, for the screen pixels it needs.
/// It says nothing of whose the frames are: layers that need as much ask at
/// the same box, and each still gets its own.
fn decode_size(needed: f32) -> [u32; 2] {
    let side = DECODE_STEPS
        .into_iter()
        .find(|step| *step as f32 >= needed)
        .unwrap_or(DECODE_STEPS[DECODE_STEPS.len() - 1]);
    [side, side]
}

/// The square a layer asks at for the screen pixels it needs, at a preview
/// quality that divides by `divisor`: the step [`decode_size`] picks, held to
/// the largest step within a half or a quarter of it, and never under the
/// smallest. A divisor of one is full quality, the step itself.
fn detailed_size(needed: f32, divisor: u32) -> [u32; 2] {
    let [full, _] = decode_size(needed);
    if divisor <= 1 {
        return [full, full];
    }
    let cap = full / divisor;
    let side = (DECODE_STEPS.into_iter().rev())
        .find(|step| *step <= cap)
        .unwrap_or(DECODE_STEPS[0]);
    [side, side]
}

/// [`decode_box_at`] at full quality.
#[cfg(test)]
fn decode_box(needed: f32, asset: &Asset) -> [u32; 2] {
    decode_box_at(needed, asset, 1)
}

/// The box a layer of `asset` asks its frames to fit, for the screen pixels
/// it needs at a preview quality that divides by `divisor`: the square of
/// [`detailed_size`] drawn in to the shape of the asset's picture, and to no
/// more than the picture has. A frame is fitted inside the box it is asked
/// at, so what a square leaves empty around a picture of another shape is
/// room of the viewer budget that nothing fills.
///
/// The picture is held to the lesser of what the square has of its width and
/// of its height, and never enlarged. Its sides are rounded up, so a frame
/// fitted inside the square and rounded to whole texels still fits the box.
/// An asset whose size is not known asks at the square. It is always the
/// asset's own size that counts, whatever its frames are read from.
fn decode_box_at(needed: f32, asset: &Asset, divisor: u32) -> [u32; 2] {
    let [across, down] = detailed_size(needed, divisor);
    let (width, height) = (u64::from(asset.width), u64::from(asset.height));
    if width == 0 || height == 0 {
        return [across, down];
    }
    let (across, down) = (u64::from(across), u64::from(down));
    // The ratio the picture is held to, as two whole numbers: one, what the
    // square has of its width, or what it has of its height.
    let (over, under) = if width <= across && height <= down {
        (1, 1)
    } else if across * height <= down * width {
        (across, width)
    } else {
        (down, height)
    };
    let fit = |side: u64| u32::try_from((side * over).div_ceil(under).max(1)).unwrap_or(u32::MAX);
    [fit(width), fit(height)]
}

/// Whether `after` carries on exactly where `before` stops: the two halves of
/// a cut. They are of one asset and play it at one rate, `after` starts on the
/// timeline where `before` ends, and in the source at the time `before` ends
/// on. That is what a split leaves, and what the two would be as one clip.
/// The halves of a time map meet within the rounding of its subdivision.
fn continues(before: &Clip, after: &Clip) -> bool {
    let mapped = before.retime.is_some() || after.retime.is_some();
    let (ending, starting) = (before.speed_at(before.end()), after.speed_at(after.start));
    let gap = before.source_time(before.end()).0 - after.source_time(after.start).0;
    before.asset_id().is_some()
        && before.asset_id() == after.asset_id()
        && before.end() == after.start
        && (ending == starting
            || (mapped && (ending - starting).abs() <= 0.05 * ending.abs().max(starting.abs())))
        && gap.abs() <= if mapped { 2 } else { 0 }
}

/// The clips `clip`, which has no layer yet, may take the layer of: those it
/// is cut from, in the order their layers are looked for.
///
/// Played or stepped across a cut, the incoming clip is another clip showing
/// the same pictures: it takes the outgoing one's layer, so it is painted at
/// once with the frame that one held and decodes on from where that one
/// stopped. The layer is looked for along the run of clips on this track that
/// [`continues`] joins to `clip`, earlier ones first and through any number of
/// cuts, so a seek from one part of a cut clip to another holds a frame just
/// as it would had the clip not been cut.
///
/// A clip in that run ends before `clip` starts or starts after it ends, so
/// it is on screen with it only while a transition holds it past its end.
/// Then the two are two pictures: the clips in `shown`, those of this frame's
/// plan, are left out of the run, so the clip that is over keeps its layer
/// and its decoder and the incoming one starts on its own. Without a
/// transition the cut is handed over as ever. [`Textures::hand_over`] takes
/// nothing from a layer drawn this frame either way: a clip that carries on
/// keeps its own. A clip that joins nothing, or whose neighbour differs in
/// asset, rate or source time, starts with no picture.
fn cut_run(track: &Track, clip: &Clip, shown: &[Layer]) -> Vec<ClipId> {
    let mut run = Vec::new();
    for earlier in [true, false] {
        let mut at = clip;
        // A run has no more clips than the track.
        for _ in 0..track.clips.len() {
            let next = track.clips.iter().find(|other| {
                if earlier {
                    continues(other, at)
                } else {
                    continues(at, other)
                }
            });
            let Some(next) = next else { break };
            if !shown.iter().any(|layer| layer.clip == next.id) {
                run.push(next.id);
            }
            at = next;
        }
    }
    run
}

/// The longer side a media layer needs a picture for, in screen pixels: that
/// of the canvas as drawn, by the clip's scale where that is under one.
///
/// The scale is the one `stored` with the clip, or held in its place while
/// it is dragged or typed, and not the one its keyframes or its tracker
/// leave it at the playhead: a layer asks at one box for as long as it is
/// shown, where a box that followed a zoom would start its decoder over at
/// every step of it. A clip whose scale is keyed or follows a tracker asks
/// for the canvas whole.
fn needed_pixels(canvas: Rect, pixels: f32, clip: &Clip, stored: &Transform) -> f32 {
    let driven = clip.follow.as_ref().is_some_and(|follow| follow.scale)
        || keyed(clip, &[ParamKey::ScaleX, ParamKey::ScaleY]);
    let scale = if driven {
        1.0
    } else {
        stored.scale[0].max(stored.scale[1]).min(1.0)
    };
    canvas.width().max(canvas.height()) * pixels * scale
}

/// The transform stored with a clip, or the one held in its place while it
/// is dragged or typed.
fn stored<'a>(draft: &'a Option<ClipDraft>, clip: &'a Clip) -> &'a Transform {
    (draft.as_ref())
        .filter(|draft| draft.clip == clip.id)
        .map_or(&clip.transform, |draft| &draft.transform)
}

/// Whether keyframes of the clip itself drive one of `keys`: the value
/// stored with the clip is then not the one shown.
fn keyed(clip: &Clip, keys: &[ParamKey]) -> bool {
    (clip.animation.iter())
        .any(|track| track.target.is_none() && !track.keys.is_empty() && keys.contains(&track.key))
}

/// The value a drag of one of the clip's own parameters starts from. Where
/// keyframes drive it that is the one they give it `local` into the clip, or
/// the keyframe already held there; `plain` otherwise, which is the stored
/// value or the one held in its place.
fn drawn(draft: &Option<ClipDraft>, clip: &Clip, key: ParamKey, local: Time, plain: f32) -> f32 {
    let Some(track) = (clip.animation.iter())
        .find(|track| track.target.is_none() && track.key == key && !track.keys.is_empty())
    else {
        return plain;
    };
    (draft.as_ref())
        .filter(|draft| draft.clip == clip.id)
        .and_then(|draft| draft.key(None, key))
        .map_or_else(|| track.sample(local), |held| held.value)
}

/// Holds `value` for one of the clip's own parameters while it is dragged:
/// as the keyframe `at` into the clip where keyframes drive it, since the
/// stored value is then not the one shown, and in the transform otherwise.
fn hold_value(next: &mut ClipDraft, clip: &Clip, key: ParamKey, at: Time, value: f32) {
    if keyed(clip, &[key]) {
        let value = look::param_spec(AnimTarget::Clip, key)
            .map_or(value, |spec| value.clamp(spec.min, spec.max));
        next.set_key(None, key, at, value);
        return;
    }
    let transform = &mut next.transform;
    match key {
        ParamKey::PositionX => transform.position[0] = value,
        ParamKey::PositionY => transform.position[1] = value,
        ParamKey::ScaleX => transform.scale[0] = value,
        ParamKey::ScaleY => transform.scale[1] = value,
        ParamKey::Rotation => transform.rotation = value,
        _ => {}
    }
}

/// A place on one axis held to the canvas's centre when the layer is shown
/// within `reach` of it. `stored` is the value a drag writes and `shown` how
/// far from it the layer is drawn, which is not nought for one that follows
/// a tracker: it is where the layer is seen that snaps, and the value that
/// puts it on the centre is returned.
fn snapped(stored: f32, shown: f32, reach: f32) -> Option<f32> {
    ((stored + shown).abs() <= reach).then_some(-shown)
}

pub(crate) use editor_core::render::{backwards_until, source_frame};

#[expect(clippy::too_many_arguments, reason = "one call site in the frame")]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &mut ViewerState,
    selection: &mut Selection,
    draft: &mut Option<ClipDraft>,
    textures: &mut Textures,
    input: &ViewerInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    widgets::panel(ui, rect, p);
    let project = input.project;
    let settings = &project.settings;
    let stage = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - TRANSPORT));
    let transport = Rect::from_min_max(pos2(rect.left(), stage.bottom()), rect.max);

    // The canvas, fitted in the stage with room around it for handles.
    let canvas_size = vec2(settings.width as f32, settings.height as f32);
    let room = stage.shrink(18.0);
    let view = (room.width() / canvas_size.x)
        .min(room.height() / canvas_size.y)
        .max(0.01);
    let canvas = Rect::from_center_size(room.center(), canvas_size * view);
    let [r, g, b, _] = settings.background;
    let painter = ui.painter_at(stage);
    painter.rect_filled(canvas, CornerRadius::same(4), Color32::from_rgb(r, g, b));

    // ---- Layers, bottom track first ---------------------------------------
    let layer_painter = painter.with_clip_rect(canvas.intersect(stage));
    state.titles.set_fonts(project.assets.values());
    let pixels = ui.pixels_per_point();
    // What the canvas shows now, with a value being dragged or typed in the
    // place of the stored one.
    let mut overrides: Vec<ClipOverride> = draft
        .iter()
        .map(|draft| draft.overriding(project))
        .collect();
    // Keyframes held in the timeline show too; a value held here comes first.
    if let Some(held) = &state.keys
        && overrides.iter().all(|other| other.clip != held.clip)
    {
        overrides.push(held.clone());
    }
    let mut plan = input.resolve(&overrides);
    let active: Vec<(&Track, &Clip)> = input.active_clips().collect();
    let staged: Vec<Option<Staged<'_>>> = plan
        .layers
        .iter()
        .map(|layer| Staged::of(project, &active, layer, input.playhead))
        .collect();
    // How much of the pictures the preview's quality asks for.
    let detail = textures.detail();
    let composing = state.composer.available();
    // The clean plate of each difference matte of a layer. Only a compositor
    // reads one, so without one none is asked for.
    let plates = |layer: &'_ Layer| {
        if composing {
            assemble::plates(project, layer)
        } else {
            Vec::new()
        }
    };
    // The pictures this frame draws are asked for together, in the order they
    // are painted, before any is.
    let mut wanted: Vec<Wanted<'_>> = Vec::new();
    for (layer, staged) in plan.layers.iter().zip(&staged) {
        let Some((staged, asset)) = staged
            .as_ref()
            .and_then(|staged| Some((staged, staged.asset?)))
        else {
            continue;
        };
        // The layer is the clip's own for as long as it is kept, whatever
        // else of the asset is on screen.
        let viewing = Viewing::Clip(layer.clip);
        let needed = needed_pixels(canvas, pixels, staged.clip, stored(draft, staged.clip));
        // A plate is laid over the box of its layer, and is asked for as
        // large: a still, or the first frame of anything else.
        for (effect, plate) in plates(layer) {
            wanted.push(Wanted {
                viewing: Viewing::Plate(layer.clip, effect),
                asset: plate,
                size: decode_box_at(needed, plate, detail),
                time: Time::ZERO,
                carries_on: Vec::new(),
            });
        }
        wanted.push(Wanted {
            viewing,
            asset,
            size: decode_box_at(needed, asset, detail),
            time: source_frame(project, asset, staged.clip, input.playhead),
            carries_on: if textures.has_layer(viewing) {
                Vec::new()
            } else {
                cut_run(staged.track, staged.clip, &plan.layers)
            },
        });
    }
    // Only a compositor reads the decoded frames behind the pictures, and
    // only of a plan that has something for it to draw. They are kept from
    // before the frames are asked for, so a plan that stops being plain
    // while paused asks for those it shows again, and let go once every
    // layer has a texture again: see below.
    // The runs the compositor draws, each on a canvas of its own.
    let runs = compose::runs(&plan);
    // A colour is picked from the decoded frame too, whatever draws it.
    let keeps = (composing && runs.iter().any(|run| matches!(run, Run::Composite(_))))
        || state.pick.is_some();
    state.composited = keeps;
    if keeps {
        textures.keep_frames(true);
    }
    textures.compose(input.media, input.playing, &wanted);

    // The camera's path through the video of each stabilized layer is
    // wanted before any is drawn with. Until it is held, and while it is of
    // what the media no longer is, the layer is drawn as it is: no frame
    // waits for a path.
    if composing {
        for (layer, staged) in plan.layers.iter().zip(&staged) {
            if steadied(layer).is_some()
                && let Some(asset) = staged.as_ref().and_then(|staged| staged.asset)
            {
                state.steady.want(asset);
            }
        }
    }
    let steady = &state.steady;
    // How each stabilized layer whose path is held reads its picture: the
    // correction of the frame the layer shows, which while the next is
    // awaited is not the playhead's.
    let samplings: Vec<Option<Sampling>> = (plan.layers.iter().zip(&staged))
        .map(|(layer, staged)| {
            let effect = steadied(layer).filter(|_| composing)?;
            let asset = staged.as_ref()?.asset?;
            let shown = textures.input(Viewing::Clip(layer.clip))?;
            let SteadyParams {
                smoothness,
                zoom,
                mix,
            } = SteadyParams::of(effect);
            steady.sampling(asset, shown.time, smoothness, zoom, mix)
        })
        .collect();
    // A point tracked on a steadied picture is where that reading shows it:
    // what follows one is placed again with the readings known. They are
    // kept for whoever works out a follower's place besides this frame.
    state.readings.clear();
    for (layer, sampling) in plan.layers.iter().zip(&samplings) {
        if let Some(sampling) = sampling
            && state.readings.iter().all(|held| held.clip != layer.clip)
        {
            state.readings.push(ClipOverride {
                clip: layer.clip,
                reading: Some(reading_of(sampling)),
                ..ClipOverride::default()
            });
        }
    }
    if !state.readings.is_empty() && active.iter().any(|(_, clip)| clip.follow.is_some()) {
        for held in &state.readings {
            match overrides.iter_mut().find(|other| other.clip == held.clip) {
                Some(other) => other.reading = held.reading,
                None => overrides.push(held.clone()),
            }
        }
        plan = input.resolve(&overrides);
    }
    let plan = plan;

    // Where a media layer's box is, in canvas pixels about the canvas centre.
    let boxes: Vec<Option<assemble::LayerBox>> = (plan.layers.iter().zip(&staged))
        .map(|(layer, staged)| {
            let asset = staged.as_ref()?.asset?;
            let picture = textures.picture_size(Viewing::Clip(layer.clip));
            let source = assemble::source_size(asset, picture, canvas_size.into());
            Some(assemble::layer_box(
                &layer.transform,
                source,
                canvas_size.into(),
            ))
        })
        .collect();
    let boxed = |index: usize| boxes[index];

    // The lookup tables this frame looks colours up in are asked for before
    // any is drawn with. One that has not come, cannot be read or has no
    // room changes nothing in its layer; the last two are said.
    let tables = |layer: &'_ Layer| assemble::tables(project, layer);
    let mut lut_problem: Option<String> = None;
    if composing {
        for (_, asset) in plan.layers.iter().flat_map(&tables) {
            state.luts.want(input.media, asset);
            if lut_problem.is_none() {
                lut_problem = state.luts.problem(asset);
            }
        }
    }
    let luts = &state.luts;

    let mut jobs: Vec<Job<'_>> = Vec::new();
    let mut job_of: Vec<Option<usize>> = Vec::with_capacity(runs.len());
    for run in &runs {
        let Run::Composite(range) = run else {
            job_of.push(None);
            continue;
        };
        // A layer that has a picture and not yet the frame it is of was
        // decoded for a plain plan, and has asked for the frame again. Until
        // it comes the run is drawn as it was, as quads, and not without
        // that layer.
        let ready = range.clone().all(|index| {
            let viewing = Viewing::Clip(plan.layers[index].clip);
            textures.input(viewing).is_some() || textures.picture(viewing).is_none()
        });
        let draws: Vec<Draw<'_>> = range
            .clone()
            .filter(|_| composing && ready)
            .filter_map(|index| {
                let layer = &plan.layers[index];
                let input = textures.input(Viewing::Clip(layer.clip))?;
                let place = boxed(index)?.placement(canvas_size.into(), layer.transform.rotation);
                Some(Draw {
                    layer,
                    place,
                    input,
                    // A matte whose plate has not come leaves its layer
                    // whole until it has.
                    plates: (plates(layer).into_iter())
                        .filter_map(|(effect, _)| {
                            let plate = Viewing::Plate(layer.clip, effect);
                            Some((effect, textures.input(plate)?))
                        })
                        .collect(),
                    luts: (tables(layer).into_iter())
                        .filter_map(|(effect, asset)| Some((effect, luts.get(asset)?)))
                        .collect(),
                    steady: samplings[index],
                })
            })
            .collect();
        if draws.is_empty() {
            job_of.push(None);
            continue;
        }
        job_of.push(Some(jobs.len()));
        jobs.push(Job {
            canvas: plan.canvas,
            // Only the lowest layers have the canvas's colour under them.
            background: assemble::background(&plan, range),
            draws,
        });
    }
    let canvases = state.composer.frame(
        compose::detailed(
            compose::target_size(plan.canvas, canvas.size(), pixels),
            detail,
        ),
        &jobs,
    );
    // What each plate asked for was when the canvases were drawn.
    state.plates.clear();
    for layer in &plan.layers {
        for (effect, _) in plates(layer) {
            let plate = Viewing::Plate(layer.clip, effect);
            let drawn = textures.input(plate).map(|input| input.serial);
            state.plates.push((plate, drawn));
        }
    }
    // Whether the compositor was given each run, whatever it made of it.
    let given: Vec<bool> = job_of.iter().map(Option::is_some).collect();
    let canvas_of: Vec<Option<egui::TextureId>> = (job_of.iter())
        .map(|job| job.and_then(|job| canvases[job]))
        .collect();
    drop(jobs);
    // The reading of each layer a canvas shows steadied: what is drawn over
    // its picture goes through the same.
    let steadied_on: Vec<(ClipId, Reading)> = (runs.iter().zip(&canvas_of))
        .filter_map(|(run, canvas)| match run {
            Run::Composite(range) if canvas.is_some() => Some(range.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|index| Some((plan.layers[index].clip, reading_of(&samplings[index]?))))
        .collect();
    // A layer a canvas shows needs no texture of its own, and every other
    // layer needs one: a run left to the mesh path is drawn from them.
    let on_canvas = |clip: ClipId| {
        (runs.iter().zip(&canvas_of)).any(|(run, canvas)| match run {
            Run::Composite(range) if canvas.is_some() => plan.layers[range.clone()]
                .iter()
                .any(|layer| layer.clip == clip),
            _ => false,
        })
    };
    textures.settle(ui.ctx(), on_canvas);
    if !keeps {
        textures.keep_frames(false);
    }
    let textures = &*textures;

    // The opacity of the partner a transition draws over the layer at
    // `index`, when that is a quad on the very same box.
    let covered_by = |index: usize| -> Option<f32> {
        let (below, above) = (&plan.layers[index], plan.layers.get(index + 1)?);
        let same_box = compose::paired(below, above)
            && boxed(index) == boxed(index + 1)
            && below.transform.rotation == above.transform.rotation
            && below.transform.crop == above.transform.crop;
        (same_box && textures.picture(Viewing::Clip(above.clip)).is_some())
            .then_some(above.transform.opacity)
    };

    let mut placed: Vec<Placed> = Vec::new();
    // A media layer on screen, and the handles it is given unless it is a
    // clip shown past its end for a transition.
    let place = |index: usize, placed: &mut Vec<Placed>| -> Option<Placed> {
        let layer = &plan.layers[index];
        let staged = staged[index].as_ref()?;
        let layer_box = boxed(index)?;
        let (position, size) = (Vec2::from(layer_box.position), Vec2::from(layer_box.size));
        let on_screen = Placed {
            clip: layer.clip,
            center: canvas.center() + position * view,
            half: size * view * 0.5,
            angle: layer.transform.rotation.to_radians(),
            locked: staged.track.locked,
        };
        if !staged.held {
            placed.push(on_screen);
        }
        Some(on_screen)
    };
    // Whether a layer that is not plain was drawn as if it were.
    let mut effects_hidden = false;
    for ((run, given), canvas_texture) in runs.iter().zip(given).zip(canvas_of) {
        match run {
            Run::Title(index) => {
                let layer = &plan.layers[*index];
                let Some(staged) = staged[*index].as_ref() else {
                    continue;
                };
                let transform = &layer.transform;
                // Drawn as many picture pixels as it covers on screen.
                let scale = assemble::title_scale(transform) * view * pixels;
                (state.titles).want(layer.clip, &layer.source, scale);
                let Some((picture, texture)) = state.titles.picture(layer.clip) else {
                    continue;
                };
                let anchor =
                    canvas.center() + vec2(transform.position[0], transform.position[1]) * view;
                // Titles are drawn by no compositor: a transition is a fade,
                // whatever its kind.
                let opacity = transform.opacity.clamp(0.0, 1.0) * fade_weight(layer, None);
                layer_painter.add(title_mesh(
                    anchor, view, transform, picture, texture, opacity,
                ));
                if !staged.held {
                    placed.push(title_place(
                        layer.clip,
                        anchor,
                        view,
                        transform,
                        picture.bounds,
                        staged.track.locked,
                    ));
                }
            }
            Run::Plain(range) | Run::Composite(range) => {
                if let Some(texture) = canvas_texture {
                    layer_painter.add(
                        RectShape::filled(canvas, CornerRadius::same(4), Color32::WHITE)
                            .with_texture(
                                texture,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            ),
                    );
                }
                // Nothing composites, or the compositor refused the run:
                // what of it is drawn below is drawn without its effects. A
                // run still waiting for its pictures was refused nothing.
                let left_out = canvas_texture.is_none() && (!composing || given);
                for index in range.clone() {
                    let Some(on_screen) = place(index, &mut placed) else {
                        continue;
                    };
                    let layer = &plan.layers[index];
                    if canvas_texture.is_none()
                        && let Some(texture) = textures.picture(Viewing::Clip(layer.clip))
                    {
                        // A transition the compositor does not draw is a
                        // cross-fade of the two quads.
                        let mut transform = layer.transform.clone();
                        transform.opacity = transform.opacity.clamp(0.0, 1.0)
                            * fade_weight(layer, covered_by(index));
                        layer_painter.add(layer_mesh(&on_screen, texture.id(), &transform));
                        effects_hidden |= left_out && !layer.is_plain();
                    }
                }
            }
        }
    }
    // The canvas edge, over the layers: a hairline in pure white or black.
    painter.rect_stroke(
        canvas,
        CornerRadius::same(4),
        Stroke::new(
            1.0,
            if p.dark {
                Color32::from_white_alpha(26)
            } else {
                Color32::from_black_alpha(26)
            },
        ),
        StrokeKind::Outside,
    );
    if project.duration() == Time::ZERO {
        painter.text(
            canvas.center(),
            Align2::CENTER_CENTER,
            "Add media to the timeline to see it here",
            theme::regular(12.5),
            Color32::from_white_alpha(120),
        );
    }

    let drawn = tracking::Drawn {
        plan: &plan,
        placed: &placed,
        steadied: &steadied_on,
    };
    if let Some(pick) = state.pick {
        // While a colour is being picked a press on the canvas picks, and
        // moves or selects nothing.
        let layer = plan
            .layers
            .iter()
            .rev()
            .find(|layer| layer.clip == pick.clip);
        let place = placed.iter().find(|place| place.clip == pick.clip);
        let frame = textures.input(Viewing::Clip(pick.clip));
        let sample = |pos: Pos2| sampled(frame?.frame, place?, &layer?.transform, pos);
        // The stack the click writes: the one held, when one is.
        let stack = (draft.as_ref())
            .filter(|draft| draft.clip == pick.clip)
            .and_then(|draft| draft.effects.as_deref());
        if let Some(rgb) = pick_colour(ui, stage, place, p, &sample) {
            let command = inspector::key_colour_command(project, pick, stack, rgb, input.playhead);
            actions.extend(command.map(Action::Edit));
            state.picked = true;
        }
    } else if let Some(region) = state.region {
        // While a region is chosen a drag on the canvas draws or moves it.
        let picture = tracking::Picture::of(&drawn, region.clip);
        tracking::choose_region(ui, stage, picture, p, state);
    } else {
        interact(
            ui, stage, canvas, view, &placed, state, selection, draft, input, actions,
        );
    }
    if state.paths {
        tracking::paths(&painter, p, project, &drawn, selection, input.playhead);
    }
    // The mask whose rows are open in the inspector, where the plan puts it.
    if let Some(id) = state.mask
        && let Some(clip) = selection.single()
        && let Some(place) = placed.iter().find(|place| place.clip == clip)
        && let Some(mask) = (plan.layers.iter())
            .filter(|layer| layer.clip == clip)
            .find_map(|layer| layer.masks.iter().find(|mask| mask.id == id))
    {
        painter.add(mask_outline(place, mask, p.yellow));
    }
    if effects_hidden {
        effects_not_shown(ui, stage, p, None, state.composer.failure());
    } else if let Some(problem) = &lut_problem {
        effects_not_shown(ui, stage, p, Some("A LUT is"), Some(problem));
    }
    draw_transport(
        ui,
        transport,
        &Deck {
            p,
            playhead: input.playhead,
            duration: project.duration(),
            rate: settings.frame_rate,
            playing: input.playing,
            speed: input.speed,
            looping: input.looping,
            detail: format!(
                "{}×{} · {} fps",
                settings.width,
                settings.height,
                trim_number(settings.frame_rate.fps())
            ),
            emit: Transport::timeline,
            record: Some(input.record),
        },
        actions,
    );
}

/// The source monitor's view of one library asset.
pub struct SourceInput<'a> {
    pub p: &'a Palette,
    pub asset: &'a Asset,
    /// The frame grid the source is stepped on.
    pub rate: FrameRate,
    pub background: [u8; 4],
    pub playhead: Time,
    pub playing: bool,
    pub speed: f64,
    pub looping: bool,
    pub media: Option<&'a MediaService>,
}

pub use editor_core::render::source_frame_time;

fn media_kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Video => "Video",
        MediaKind::Audio => "Audio",
        MediaKind::Image => "Image",
        MediaKind::Gif => "GIF",
        MediaKind::Lut => "LUT",
        MediaKind::Font => "Font",
    }
}

/// The source monitor: one library asset played from its file, in the
/// viewer's place. Nothing here edits the project; its transport drives the
/// monitor's own clock.
pub fn show_source(
    ui: &mut Ui,
    rect: Rect,
    textures: &mut Textures,
    input: &SourceInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let asset = input.asset;
    widgets::panel(ui, rect, p);
    let timed = asset.kind != MediaKind::Image;
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), SOURCE_HEADER));
    let transport = Rect::from_min_max(pos2(rect.left(), rect.bottom() - TRANSPORT), rect.max);
    let scrub = Rect::from_min_max(
        pos2(
            rect.left(),
            transport.top() - if timed { SCRUB } else { 0.0 },
        ),
        transport.right_top(),
    );
    let stage = Rect::from_min_max(header.left_bottom(), scrub.right_top());
    let tone = p.tone(tone_of_media(asset.kind));

    // ---- Header: what this is, and the ways out of it ----------------------
    ui.painter()
        .hline(rect.x_range(), header.bottom(), p.hairline());
    let mut trailing = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header.shrink2(vec2(8.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    trailing.spacing_mut().item_spacing.x = 4.0;
    if widgets::icon_button(&mut trailing, p, Icon::Close, "Close preview (Esc)").clicked() {
        actions.push(Action::CloseSource);
    }
    let compact = header.width() < 420.0;
    let add = if compact {
        widgets::icon_button(&mut trailing, p, Icon::Plus, "Add to timeline")
    } else {
        widgets::button(
            &mut trailing,
            p,
            Some(Icon::Plus),
            "Add to timeline",
            widgets::Kind::Plain,
        )
    };
    if add.clicked() {
        actions.push(Action::InsertAsset {
            asset: asset.id,
            place: None,
        });
    }
    let free = trailing.min_rect().left() - 8.0;
    let painter = ui.painter_at(header);
    let chip = painter.layout_no_wrap("Source".to_owned(), theme::medium(11.0), tone);
    let pill = Rect::from_min_size(
        pos2(
            header.left() + 12.0,
            header.center().y - chip.size().y * 0.5 - 3.0,
        ),
        chip.size() + vec2(30.0, 6.0),
    );
    painter.rect_filled(pill, CornerRadius::same(7), theme::tint(tone, 0.16));
    icons::paint(
        &painter,
        Rect::from_center_size(pos2(pill.left() + 12.0, pill.center().y), Vec2::splat(12.0)),
        icon_of_media(asset.kind),
        tone,
    );
    painter.galley(pos2(pill.left() + 22.0, pill.top() + 3.0), chip, tone);
    let name = widgets::truncated(
        ui,
        &asset.name,
        theme::medium(13.0),
        p.fg,
        (free - pill.right() - 10.0).max(0.0),
    );
    painter.galley(
        pos2(pill.right() + 10.0, header.center().y - name.size().y * 0.5),
        name,
        p.fg,
    );

    // ---- Stage ------------------------------------------------------------
    let painter = ui.painter_at(stage);
    let room = stage.shrink(18.0);
    if asset.kind == MediaKind::Audio {
        // Sound has no picture, and none is made up for it. Nothing is drawn,
        // which the layers kept for the timeline are still held to.
        textures.compose(input.media, input.playing, &[]);
        let side = 96.0f32.min(room.height() - 56.0).max(40.0);
        let disc = Rect::from_center_size(room.center() - vec2(0.0, 26.0), Vec2::splat(side));
        painter.rect_filled(
            disc,
            CornerRadius::same((side * 0.28) as u8),
            theme::tint(tone, 0.14),
        );
        icons::paint(
            &painter,
            Rect::from_center_size(disc.center(), Vec2::splat(side * 0.42)),
            Icon::Music,
            tone,
        );
        let name = widgets::truncated(ui, &asset.name, theme::medium(14.0), p.fg, room.width());
        painter.galley(
            pos2(room.center().x - name.size().x * 0.5, disc.bottom() + 14.0),
            name,
            p.fg,
        );
        let mut detail = format!("Audio · {}", timefmt::short(asset.duration));
        if asset.file_size > 0 {
            detail.push_str(&format!(" · {}", timefmt::file_size(asset.file_size)));
        }
        painter.text(
            pos2(room.center().x, disc.bottom() + 36.0),
            Align2::CENTER_TOP,
            detail,
            theme::regular(12.0),
            p.muted,
        );
    } else {
        let needed = room.width().max(room.height()) * ui.pixels_per_point();
        // The monitor's layer is its own, and no timeline clip's.
        let wanted = Wanted {
            viewing: Viewing::Source,
            asset,
            size: decode_box_at(needed, asset, textures.detail()),
            time: source_frame_time(asset, input.rate, input.playhead),
            carries_on: Vec::new(),
        };
        textures.compose(input.media, input.playing, &[wanted]);
        // Until the first frame is decoded the poster stands in.
        let texture = textures
            .picture(Viewing::Source)
            .or_else(|| textures.poster(asset.id));
        let source = if asset.width > 0 && asset.height > 0 {
            vec2(asset.width as f32, asset.height as f32)
        } else if let Some(texture) = texture {
            texture.size_vec2()
        } else {
            vec2(16.0, 9.0)
        };
        let scale = (room.width() / source.x)
            .min(room.height() / source.y)
            .max(0.001);
        let picture = Rect::from_center_size(room.center(), source * scale);
        let [r, g, b, _] = input.background;
        painter.rect_filled(picture, CornerRadius::same(4), Color32::from_rgb(r, g, b));
        match texture {
            Some(texture) => {
                painter.add(
                    RectShape::filled(picture, CornerRadius::same(4), Color32::WHITE).with_texture(
                        texture.id(),
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    ),
                );
            }
            None => icons::paint(
                &painter,
                Rect::from_center_size(picture.center(), Vec2::splat(30.0)),
                icon_of_media(asset.kind),
                tone,
            ),
        }
        painter.rect_stroke(
            picture,
            CornerRadius::same(4),
            Stroke::new(
                1.0,
                if p.dark {
                    Color32::from_white_alpha(26)
                } else {
                    Color32::from_black_alpha(26)
                },
            ),
            StrokeKind::Outside,
        );
    }
    let response = ui.interact(stage, Id::new("source-stage"), Sense::click());
    if timed && response.double_clicked() {
        actions.push(Action::Source(Transport::TogglePlay));
    }

    if !timed {
        // A still has nothing to play: its facts take the transport's place.
        ui.painter()
            .hline(rect.x_range(), transport.top(), p.hairline());
        let mut detail = "Still image".to_owned();
        if asset.width > 0 && asset.height > 0 {
            detail.push_str(&format!(" · {}×{}", asset.width, asset.height));
        }
        if asset.file_size > 0 {
            detail.push_str(&format!(" · {}", timefmt::file_size(asset.file_size)));
        }
        ui.painter().text(
            transport.center(),
            Align2::CENTER_CENTER,
            detail,
            theme::regular(12.0),
            p.muted,
        );
        return;
    }

    // ---- Scrub bar: the source has no timeline of its own -------------------
    let duration = asset.duration.max(Time(1));
    let lane = Rect::from_min_max(
        pos2(scrub.left() + 16.0, scrub.top()),
        pos2(scrub.right() - 16.0, scrub.bottom()),
    );
    let scrubbed = ui
        .interact(
            lane.expand2(vec2(8.0, 0.0)),
            Id::new("source-scrub"),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(CursorIcon::PointingHand);
    scrubbed.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Source position")
    });
    // A click may begin and end inside one frame, so it counts as well as
    // a press that is still held.
    if (scrubbed.clicked() || scrubbed.dragged() || scrubbed.is_pointer_button_down_on())
        && let Some(pos) = scrubbed.interact_pointer_pos()
    {
        let share = ((pos.x - lane.left()) / lane.width().max(1.0)).clamp(0.0, 1.0);
        let time = source_frame_time(
            asset,
            input.rate,
            Time((duration.0 as f64 * f64::from(share)).round() as i64),
        );
        if time != input.playhead {
            actions.push(Action::Source(Transport::Seek(time)));
        }
    }
    let share = (input.playhead.0 as f32 / duration.0 as f32).clamp(0.0, 1.0);
    let track = Rect::from_center_size(lane.center(), vec2(lane.width(), 4.0));
    let at = track.left() + track.width() * share;
    let painter = ui.painter_at(scrub.expand(2.0));
    painter.rect_filled(track, CornerRadius::same(2), p.control);
    painter.rect_filled(
        Rect::from_min_max(track.min, pos2(at, track.bottom())),
        CornerRadius::same(2),
        p.accent,
    );
    let grown = ui.ctx().animate_bool_with_time(
        scrubbed.id.with("knob"),
        scrubbed.hovered() || scrubbed.dragged(),
        0.12,
    );
    painter.circle_filled(pos2(at, track.center().y), 5.0 + grown, Color32::WHITE);
    painter.circle_stroke(
        pos2(at, track.center().y),
        5.0 + grown,
        Stroke::new(1.5, p.accent),
    );

    let mut detail = media_kind_name(asset.kind).to_owned();
    if asset.width > 0 && asset.height > 0 {
        detail.push_str(&format!(" · {}×{}", asset.width, asset.height));
    }
    if let Some(rate) = asset.frame_rate {
        detail.push_str(&format!(" · {} fps", trim_number(rate.fps())));
    }
    draw_transport(
        ui,
        transport,
        &Deck {
            p,
            playhead: input.playhead,
            duration: asset.duration,
            rate: input.rate,
            playing: input.playing,
            speed: input.speed,
            looping: input.looping,
            detail,
            emit: |transport| Action::Source(transport),
            record: None,
        },
        actions,
    );
}

/// One transport: the timeline's, or the source monitor's. Each sends its
/// requests to its own clock.
struct Deck<'a> {
    p: &'a Palette,
    playhead: Time,
    duration: Time,
    rate: FrameRate,
    playing: bool,
    speed: f64,
    looping: bool,
    /// Facts about what is playing, at the trailing edge.
    detail: String,
    emit: fn(Transport) -> Action,
    /// The record control: the timeline's alone has one.
    record: Option<RecordView>,
}

/// A layer of the plan with the clip it is of.
struct Staged<'a> {
    track: &'a Track,
    clip: &'a Clip,
    /// The asset of a media layer, when the library has it. A layer of an
    /// asset it has not is not drawn.
    asset: Option<&'a Asset>,
    /// The clip is over, and shown on for the transition into the next.
    held: bool,
}

impl<'a> Staged<'a> {
    /// The clip `layer` is of: one of those on screen, or the one a
    /// transition holds past its end, which ended where the playhead is or
    /// before it.
    fn of(
        project: &'a Project,
        active: &[(&'a Track, &'a Clip)],
        layer: &Layer,
        playhead: Time,
    ) -> Option<Self> {
        let (track, clip) = active
            .iter()
            .copied()
            .find(|(_, clip)| clip.id == layer.clip)
            .or_else(|| {
                let track = project.track(layer.track)?;
                let started = track.clips.partition_point(|clip| clip.start <= playhead);
                let clip =
                    (track.clips[..started].iter().rev()).find(|clip| clip.id == layer.clip)?;
                Some((track, clip))
            })?;
        let asset = match &layer.source {
            LayerSource::Media { asset_id, .. } => project.assets.get(asset_id),
            LayerSource::Text { .. } | LayerSource::Graphic { .. } => None,
        };
        Some(Self {
            track,
            clip,
            asset,
            held: (layer.transition.as_ref())
                .is_some_and(|transition| transition.outgoing && transition.partner.is_some()),
        })
    }
}

/// Says, quietly and for as long as it is so, that layers are drawn without
/// their blend modes, masks, effects and transitions: in the margin over the
/// canvas, where it covers nothing and a narrow window still has room for it.
/// `what` narrows that to the one thing left out, when the rest is drawn.
fn effects_not_shown(ui: &mut Ui, stage: Rect, p: &Palette, what: Option<&str>, why: Option<&str>) {
    const NAME: &str = "Effects not shown";
    let galley = ui
        .painter()
        .layout_no_wrap(NAME.to_owned(), theme::regular(11.0), p.muted);
    let rect = Rect::from_min_size(stage.left_top() + vec2(12.0, 2.0), galley.size());
    ui.painter_at(stage).galley(rect.min, galley, p.muted);
    let response = ui.interact(rect, Id::new("viewer-effects-not-shown"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, NAME));
    response.on_hover_text(format!(
        "{} left out of this preview. {}",
        what.unwrap_or("Blend modes, masks, effects and transitions are"),
        why.unwrap_or("This frame asks more of the compositor than its limits allow.")
    ));
}

/// The colour of the picture a layer shows under `pos`, gamma-encoded, from
/// the decoded frame and not from what is drawn of it: the mean of the
/// texels about the place, through the layer's turn, crop and flips. Nothing
/// outside the layer's box.
fn sampled(
    frame: &editor_media::Frame,
    place: &Placed,
    transform: &Transform,
    pos: Pos2,
) -> Option<[f32; 3]> {
    if !place.contains(pos) || frame.width == 0 || frame.height == 0 {
        return None;
    }
    let (sin, cos) = (-place.angle).sin_cos();
    let d = pos - place.center;
    let local = vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
    let across = (local.x / place.half.x.max(f32::EPSILON) + 1.0) * 0.5;
    let down = (local.y / place.half.y.max(f32::EPSILON) + 1.0) * 0.5;
    let [u0, v0, u1, v1] = compose::pass::corners(transform);
    let u = u0 + (u1 - u0) * across;
    let v = v0 + (v1 - v0) * down;
    let (width, height) = (frame.width as i64, frame.height as i64);
    let x = ((u * width as f32) as i64).clamp(0, width - 1);
    let y = ((v * height as f32) as i64).clamp(0, height - 1);
    let mut sum = [0u32; 3];
    let mut count = 0u32;
    for (dx, dy) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))) {
        let (x, y) = (x + dx, y + dy);
        if x < 0 || y < 0 || x >= width || y >= height {
            continue;
        }
        let at = ((y * width + x) * 4) as usize;
        let texel = frame.rgba.get(at..at + 3)?;
        for (sum, byte) in sum.iter_mut().zip(texel) {
            *sum += u32::from(*byte);
        }
        count += 1;
    }
    Some(sum.map(|sum| sum as f32 / (count.max(1) as f32 * 255.0)))
}

/// The stage while a key colour is picked off it: the colour under the
/// pointer is shown beside it, and a click hands it back. `sample` reads the
/// colour of the clip being keyed at a place on screen.
fn pick_colour(
    ui: &mut Ui,
    stage: Rect,
    place: Option<&Placed>,
    p: &Palette,
    sample: &dyn Fn(Pos2) -> Option<[f32; 3]>,
) -> Option<[f32; 3]> {
    const NAME: &str = "Click the picture to pick its key colour";
    let response = ui.interact(stage, Id::new("viewer-stage"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, NAME));
    let painter = ui.painter_at(stage);
    painter.text(
        stage.center_top() + vec2(0.0, 3.0),
        Align2::CENTER_TOP,
        if place.is_some() {
            NAME
        } else {
            "The clip is not on screen at the playhead"
        },
        theme::regular(11.0),
        p.muted,
    );
    if let Some(place) = place {
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        painter.add(Shape::closed_line(
            corners.map(|(x, y)| place.corner(x, y)).to_vec(),
            Stroke::new(1.5, p.accent),
        ));
    }
    let pos = response.hover_pos()?;
    let Some(rgb) = sample(pos) else {
        ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
        return None;
    };
    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    let [r, g, b] = rgb.map(|channel| (channel * 255.0).round() as u8);
    let swatch = pos + vec2(18.0, 18.0);
    painter.circle_filled(swatch, 9.0, Color32::from_rgb(r, g, b));
    painter.circle_stroke(swatch, 9.0, Stroke::new(1.5, Color32::WHITE));
    painter.circle_stroke(
        swatch,
        10.5,
        Stroke::new(1.0, Color32::from_black_alpha(120)),
    );
    response.clicked().then_some(rgb)
}

/// A textured quad with the clip's rotation, crop, flips and opacity.
/// Where a point of a title is on screen: `x` and `y` canvas pixels from its
/// anchor at the layer's scale 1, which is at `anchor`.
fn title_point(anchor: Pos2, view: f32, transform: &Transform, x: f32, y: f32) -> Pos2 {
    assemble::title_point(anchor.into(), view, transform, x, y).into()
}

/// A title's picture as one quad through the layer's transform.
fn title_mesh(
    anchor: Pos2,
    view: f32,
    transform: &Transform,
    picture: &TitlePicture,
    texture: egui::TextureId,
    opacity: f32,
) -> Shape {
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let tint = Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha);
    let [left, top] = picture.origin;
    let [width, height] = picture.size();
    let mut mesh = Mesh::with_texture(texture);
    for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: title_point(anchor, view, transform, left + u * width, top + v * height),
            uv: pos2(u, v),
            color: tint,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Shape::mesh(mesh)
}

/// A title on screen by its text or design box, `bounds` from its anchor:
/// what is selected and dragged, whatever its shadow or outline reach.
fn title_place(
    clip: ClipId,
    anchor: Pos2,
    view: f32,
    transform: &Transform,
    bounds: [f32; 4],
    locked: bool,
) -> Placed {
    let [left, top, right, bottom] = bounds;
    let center = (left + right) * 0.5;
    let middle = (top + bottom) * 0.5;
    let half = vec2(
        (right - left) * 0.5 * transform.scale[0].abs(),
        (bottom - top) * 0.5 * transform.scale[1].abs(),
    ) * view;
    Placed {
        clip,
        center: title_point(anchor, view, transform, center, middle),
        // A title with nothing in it can still be taken hold of.
        half: half.max(vec2(4.0, 4.0)),
        angle: transform.rotation.to_radians(),
        locked,
    }
}

fn layer_mesh(layer: &Placed, texture: egui::TextureId, transform: &Transform) -> Shape {
    let [left, top, right, bottom] = transform.crop;
    let (mut u0, mut u1) = (left, 1.0 - right);
    let (mut v0, mut v1) = (top, 1.0 - bottom);
    if transform.flip_x {
        std::mem::swap(&mut u0, &mut u1);
    }
    if transform.flip_y {
        std::mem::swap(&mut v0, &mut v1);
    }
    let alpha = (transform.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let tint = Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha);
    let mut mesh = Mesh::with_texture(texture);
    for (x, y, u, v) in [
        (-1.0, -1.0, u0, v0),
        (1.0, -1.0, u1, v0),
        (1.0, 1.0, u1, v1),
        (-1.0, 1.0, u0, v1),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: layer.corner(x, y),
            uv: pos2(u, v),
            color: tint,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Shape::mesh(mesh)
}

#[expect(clippy::too_many_arguments, reason = "one call site in the frame")]
fn interact(
    ui: &mut Ui,
    stage: Rect,
    canvas: Rect,
    view: f32,
    placed: &[Placed],
    state: &mut ViewerState,
    selection: &mut Selection,
    draft: &mut Option<ClipDraft>,
    input: &ViewerInput,
    actions: &mut Vec<Action>,
) {
    let p = input.p;
    let project = input.project;
    let response = ui.interact(stage, Id::new("viewer-stage"), Sense::click_and_drag());
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let painter = ui.painter_at(stage);
    let selected = selection
        .single()
        .and_then(|id| placed.iter().find(|layer| layer.clip == id))
        .copied();

    let near = |a: Pos2, b: Pos2| a.distance(b) <= HANDLE + 3.0;
    let corners = |layer: &Placed| {
        [
            layer.corner(-1.0, -1.0),
            layer.corner(1.0, -1.0),
            layer.corner(1.0, 1.0),
            layer.corner(-1.0, 1.0),
        ]
    };

    // Press: a handle of the selected layer first, then the topmost layer.
    if ui.input(|i| i.pointer.primary_pressed())
        && response.contains_pointer()
        && let Some(pos) = pointer.filter(|pos| stage.contains(*pos))
    {
        state.hold = None;
        // A value keyframes drive is not the stored one: a drag starts from
        // the one shown at the playhead and writes its keyframe there.
        let local = |clip: &Clip| (input.playhead - clip.start).clamp(Time::ZERO, clip.duration);
        let handle = selected.filter(|layer| !layer.locked).and_then(|layer| {
            let clip = project.clip(layer.clip)?.1;
            let transform = stored(draft, clip);
            let from = |key, plain| drawn(draft, clip, key, local(clip), plain);
            if near(layer.rotate_handle(), pos) {
                let d = pos - layer.center;
                Some((
                    layer.clip,
                    Hold::Rotate {
                        angle: d.y.atan2(d.x),
                        rotation: from(ParamKey::Rotation, transform.rotation),
                    },
                ))
            } else if corners(&layer).iter().any(|corner| near(*corner, pos)) {
                Some((
                    layer.clip,
                    Hold::Scale {
                        distance: pos.distance(layer.center).max(1.0),
                        scale: [
                            from(ParamKey::ScaleX, transform.scale[0]),
                            from(ParamKey::ScaleY, transform.scale[1]),
                        ],
                    },
                ))
            } else {
                None
            }
        });
        let hold = handle.or_else(|| {
            let layer = placed.iter().rev().find(|layer| layer.contains(pos))?;
            let clip = project.clip(layer.clip)?.1;
            selection.only(layer.clip);
            // Where it is drawn is from the value held in the stored one's
            // place, when one is.
            let [x, y] = stored(draft, clip).position;
            let x = drawn(draft, clip, ParamKey::PositionX, local(clip), x);
            let y = drawn(draft, clip, ParamKey::PositionY, local(clip), y);
            (!layer.locked).then_some((
                layer.clip,
                Hold::Move {
                    from: pos,
                    position: [x, y],
                    shown: (layer.center - canvas.center()) / view - vec2(x, y),
                },
            ))
        });
        state.hold = hold;
        if let Some((_, clip)) = hold.and_then(|(id, _)| project.clip(id)) {
            state.hold_at = local(clip);
        }
        if state.hold.is_none() && !placed.iter().any(|layer| layer.contains(pos)) {
            selection.clear();
        }
    }

    // Drag: write the change into the draft; it is committed on release.
    let mut guides: Vec<[Pos2; 2]> = Vec::new();
    if let (Some((id, hold)), Some(pos), true) = (state.hold, pointer, response.dragged())
        && let Some((_, clip)) = project.clip(id)
    {
        let entry = draft
            .take()
            .filter(|draft| draft.clip == id)
            .unwrap_or_else(|| ClipDraft::of(clip));
        let mut next = entry;
        let at = state.hold_at;
        let precise = ui.input(|i| i.modifiers.shift);
        // The layer turns and grows about where it is shown, which is not
        // where its stored position puts it once that is animated or follows
        // a tracker.
        let centre_of = |transform: &Transform| {
            placed.iter().find(|layer| layer.clip == id).map_or_else(
                || canvas.center() + vec2(transform.position[0], transform.position[1]) * view,
                |layer| layer.center,
            )
        };
        match hold {
            Hold::Move {
                from,
                position,
                shown,
            } => {
                let moved = (pos - from) / view;
                let mut x = position[0] + moved.x;
                let mut y = position[1] + moved.y;
                // The canvas centre holds a layer that is shown close to it.
                if !precise {
                    let reach = 6.0 / view;
                    if let Some(held) = snapped(x, shown.x, reach) {
                        x = held;
                        guides.push([canvas.center_top(), canvas.center_bottom()]);
                    }
                    if let Some(held) = snapped(y, shown.y, reach) {
                        y = held;
                        guides.push([canvas.left_center(), canvas.right_center()]);
                    }
                }
                hold_value(&mut next, clip, ParamKey::PositionX, at, x.round());
                hold_value(&mut next, clip, ParamKey::PositionY, at, y.round());
            }
            Hold::Scale { distance, scale } => {
                let center = centre_of(&next.transform);
                let factor = pos.distance(center).max(1.0) / distance;
                let grown = |scale: f32| (scale * factor).clamp(0.01, 100.0);
                hold_value(&mut next, clip, ParamKey::ScaleX, at, grown(scale[0]));
                hold_value(&mut next, clip, ParamKey::ScaleY, at, grown(scale[1]));
            }
            Hold::Rotate { angle, rotation } => {
                let center = centre_of(&next.transform);
                let d = pos - center;
                let mut degrees = rotation + (d.y.atan2(d.x) - angle).to_degrees();
                degrees = (degrees + 180.0).rem_euclid(360.0) - 180.0;
                if !precise {
                    // Quarter turns hold within three degrees.
                    let quarter = (degrees / 90.0).round() * 90.0;
                    if (degrees - quarter).abs() <= 3.0 {
                        degrees = quarter;
                    }
                }
                hold_value(&mut next, clip, ParamKey::Rotation, at, degrees);
            }
        }
        *draft = Some(next);
    }
    if ui.input(|i| i.pointer.primary_released()) {
        state.hold = None;
    }

    // Guides and the selected layer's frame.
    for [a, b] in guides {
        painter.line_segment([a, b], Stroke::new(1.0, p.yellow));
    }
    if let Some(layer) = selected {
        let points = corners(&layer);
        painter.add(Shape::closed_line(
            points.to_vec(),
            Stroke::new(1.5, p.accent),
        ));
        if !layer.locked {
            let top = layer.corner(0.0, -1.0);
            let handle = layer.rotate_handle();
            painter.line_segment([top, handle], Stroke::new(1.5, p.accent));
            painter.circle_filled(handle, 5.5, Color32::WHITE);
            painter.circle_stroke(handle, 5.5, Stroke::new(1.5, p.accent));
            for corner in points {
                let square = Rect::from_center_size(corner, Vec2::splat(HANDLE));
                painter.rect_filled(square, CornerRadius::same(2), Color32::WHITE);
                painter.rect_stroke(
                    square,
                    CornerRadius::same(2),
                    Stroke::new(1.5, p.accent),
                    StrokeKind::Inside,
                );
            }
            if let Some(pos) = pointer.filter(|_| response.contains_pointer()) {
                if near(handle, pos) {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                } else if points.iter().any(|corner| near(*corner, pos)) {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe);
                } else if layer.contains(pos) {
                    ui.ctx().set_cursor_icon(CursorIcon::Move);
                }
            }
        }
    }
    if response.double_clicked() && selected.is_none() {
        actions.push(Action::TogglePlay);
    }
}

/// How much of a transport's central group is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Controls {
    /// Play alone. The rest stays on the keyboard.
    Play,
    /// Play between the frame steps.
    Steps,
    /// And the jumps to either end around those.
    Full,
}

impl Controls {
    /// The icon buttons on each side of play.
    fn beside(self) -> f32 {
        match self {
            Self::Play => 0.0,
            Self::Steps => 1.0,
            Self::Full => 2.0,
        }
    }

    fn width(self) -> f32 {
        PLAY + 2.0 * self.beside() * (metrics::ICON_BUTTON + CONTROL_GAP)
    }
}

const PLAY: f32 = 34.0;
/// Between two controls of one group.
const CONTROL_GAP: f32 = 4.0;
/// Between the groups of a transport, at the least.
const GROUP_GAP: f32 = 8.0;
const TRANSPORT_LEADING: f32 = 16.0;
const TRANSPORT_TRAILING: f32 = 10.0;
/// The speed control: wide enough for `0.25×`, and as tall as egui makes any
/// button, which would otherwise hang below a shorter place.
const SPEED: Vec2 = vec2(48.0, metrics::legacy::CONTROL_HEIGHT);

/// The widths of what a transport shows as text.
#[derive(Clone, Copy, Debug)]
struct TransportText {
    timecode: f32,
    /// The duration after the timecode, with the space before it.
    duration: f32,
    /// The facts at the trailing edge, with the space after the controls.
    detail: f32,
    /// The record control before speed; nought on a deck that records
    /// nothing.
    record: f32,
}

/// Where the parts of a transport go in a bar of some width.
///
/// Play and speed are always there and never share a pixel, and so is the
/// record control just before speed on a deck that has one. Everything else
/// is shown only where it fits beside them: the timecode, then the frame
/// steps, then the jumps and the loop toggle, and last the duration and the
/// facts, which stay away unless the controls keep the centre of the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TransportLayout {
    timecode: bool,
    duration: bool,
    controls: Controls,
    /// The central group, exactly as wide as its controls.
    group: Rect,
    looping: Option<Rect>,
    record: Option<Rect>,
    speed: Rect,
    detail: bool,
}

impl TransportLayout {
    fn new(rect: Rect, text: TransportText) -> Self {
        let width = rect.width();
        let timecode = TRANSPORT_LEADING + text.timecode;
        let record = if text.record > 0.0 {
            text.record + CONTROL_GAP
        } else {
            0.0
        };
        // What always trails: speed, and the record control before it.
        let speed = TRANSPORT_TRAILING + record + SPEED.x;
        let looping = speed + CONTROL_GAP + metrics::ICON_BUTTON;
        let fits = |controls: Controls, trailing: f32| {
            timecode + GROUP_GAP + controls.width() + GROUP_GAP + trailing <= width
        };
        // The most that fits beside the timecode. Where not even play does,
        // the timecode goes and play and speed have the bar to themselves.
        let (controls, shows_timecode) = if fits(Controls::Full, looping) {
            (Controls::Full, true)
        } else if fits(Controls::Steps, speed) {
            (Controls::Steps, true)
        } else {
            (Controls::Play, fits(Controls::Play, speed))
        };
        let mut leading = if shows_timecode {
            timecode
        } else {
            TRANSPORT_TRAILING
        };
        let mut trailing = if controls == Controls::Full {
            looping
        } else {
            speed
        };
        // What is left on each side of controls that keep the centre.
        let side = (width - controls.width()) * 0.5 - GROUP_GAP;
        let centred = leading <= side && trailing <= side;
        let duration = centred && shows_timecode && leading + text.duration <= side;
        if duration {
            leading += text.duration;
        }
        let detail = centred && controls == Controls::Full && trailing + text.detail <= side;
        if detail {
            trailing += text.detail;
        }
        // Otherwise midway between what is at the two edges, which are not
        // the same width. A bar too narrow even for play and speed keeps
        // them apart, in that order, and lets its edge clip what is left.
        let from = rect.left() + leading + GROUP_GAP;
        let to = (rect.right() - trailing - GROUP_GAP).max(from + controls.width());
        let centre = if centred {
            rect.center().x
        } else {
            (from + to) * 0.5
        };
        let y = rect.center().y;
        let group = Rect::from_center_size(
            pos2(centre, y),
            vec2(controls.width(), metrics::ICON_BUTTON.max(PLAY)),
        );
        let trailing_left = (rect.right() - speed).max(group.right() + GROUP_GAP);
        let record_rect = (text.record > 0.0).then(|| {
            Rect::from_min_size(
                pos2(trailing_left, y - metrics::ICON_BUTTON * 0.5),
                vec2(text.record, metrics::ICON_BUTTON),
            )
        });
        let speed = Rect::from_min_size(pos2(trailing_left + record, y - SPEED.y * 0.5), SPEED);
        let looping = (controls == Controls::Full).then(|| {
            Rect::from_center_size(
                pos2(trailing_left - CONTROL_GAP - metrics::ICON_BUTTON * 0.5, y),
                Vec2::splat(metrics::ICON_BUTTON),
            )
        });
        Self {
            timecode: shows_timecode,
            duration,
            controls,
            group,
            looping,
            record: record_rect,
            speed,
            detail,
        }
    }

    /// Where the play control is within the group.
    #[cfg(test)]
    fn play(&self) -> Rect {
        Rect::from_center_size(self.group.center(), Vec2::splat(PLAY))
    }
}

fn draw_transport(ui: &mut Ui, rect: Rect, input: &Deck, actions: &mut Vec<Action>) {
    let p = input.p;
    let rate = input.rate;
    let emit = input.emit;
    ui.painter().hline(rect.x_range(), rect.top(), p.hairline());

    let painter = ui.painter_at(rect);
    let now = painter.layout_no_wrap(
        timefmt::timecode(input.playhead, rate),
        theme::mono(12.5),
        p.fg,
    );
    let total = painter.layout_no_wrap(
        format!("/ {}", timefmt::timecode(input.duration, rate)),
        theme::mono(12.5),
        p.muted,
    );
    let detail = painter.layout_no_wrap(input.detail.clone(), theme::regular(11.5), p.muted);
    let layout = TransportLayout::new(
        rect,
        TransportText {
            timecode: now.size().x,
            duration: 6.0 + total.size().x,
            detail: detail.size().x + 10.0,
            record: if input.record.is_some() {
                metrics::ICON_BUTTON
            } else {
                0.0
            },
        },
    );

    // Timecode at the leading edge.
    if layout.timecode {
        let left = rect.left() + TRANSPORT_LEADING;
        let after = left + now.size().x + 6.0;
        painter.galley(pos2(left, rect.center().y - now.size().y * 0.5), now, p.fg);
        if layout.duration {
            painter.galley(
                pos2(after, rect.center().y - total.size().y * 0.5),
                total,
                p.muted,
            );
        }
    }

    // Transport in the centre, around the one primary control. A narrow
    // viewer keeps the frame steps, then only play; the rest stays on the
    // keyboard.
    let (full, steps) = (
        layout.controls == Controls::Full,
        layout.controls >= Controls::Steps,
    );
    let mut controls = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(layout.group)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui_c = &mut controls;
    ui_c.spacing_mut().item_spacing.x = CONTROL_GAP;
    if full && widgets::icon_button(ui_c, p, Icon::SkipStart, "Go to start (Home)").clicked() {
        actions.push(emit(Transport::JumpStart));
    }
    if steps && widgets::icon_button(ui_c, p, Icon::StepBack, "Previous frame (←)").clicked() {
        actions.push(emit(Transport::Step(-1)));
    }
    let (play_rect, play) = ui_c.allocate_exact_size(Vec2::splat(PLAY), Sense::click());
    let label = if input.playing {
        "Pause (Space)"
    } else {
        "Play (Space)"
    };
    play.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let press = ui_c.ctx().animate_bool_with_time(
        play.id.with("press"),
        play.is_pointer_button_down_on(),
        0.1,
    );
    let disc = Rect::from_center_size(play_rect.center(), play_rect.size() * (1.0 - 0.04 * press));
    ui_c.painter().circle_filled(
        disc.center(),
        disc.width() * 0.5,
        if play.hovered() {
            theme::mix(p.accent, Color32::WHITE, 0.1)
        } else {
            p.accent
        },
    );
    if play.has_focus() {
        // The disc is the accent itself, so the ring stands off it.
        ui_c.painter().circle_stroke(
            disc.center(),
            disc.width() * 0.5 + 3.0,
            egui::Stroke::new(1.5, p.accent),
        );
    }
    // Play and pause trade places with a short cross-fade.
    let t = ui_c
        .ctx()
        .animate_bool_with_time(play.id.with("state"), input.playing, 0.2);
    for (icon, amount) in [(Icon::Play, 1.0 - t), (Icon::Pause, t)] {
        if amount > 0.01 {
            let side = 16.0 * (0.25 + 0.75 * amount);
            icons::paint(
                ui_c.painter(),
                Rect::from_center_size(disc.center(), Vec2::splat(side)),
                icon,
                p.on_accent.gamma_multiply(amount),
            );
        }
    }
    if play.on_hover_text(label).clicked() {
        actions.push(emit(Transport::TogglePlay));
    }
    if steps && widgets::icon_button(ui_c, p, Icon::StepForward, "Next frame (→)").clicked() {
        actions.push(emit(Transport::Step(1)));
    }
    if full && widgets::icon_button(ui_c, p, Icon::SkipEnd, "Go to end (End)").clicked() {
        actions.push(emit(Transport::JumpEnd));
    }

    // While a take runs, its stop control, elapsed time and level take the
    // place of record and speed, which is fixed at 1× until it ends.
    if let (Some(view), Some(place)) = (input.record, layout.record) {
        if let RecordView::Recording { elapsed, level } = view {
            draw_take(ui, p, place.union(layout.speed), elapsed, level, actions);
        } else {
            draw_record(ui, p, place, view, actions);
        }
    }
    let recording = matches!(input.record, Some(RecordView::Recording { .. }));

    // Loop and speed at the trailing edge, each in a place of its own.
    if !recording {
        let mut trailing = ui.new_child(egui::UiBuilder::new().max_rect(layout.speed));
        // The control is as wide as the layout made it, whatever it reads.
        trailing.spacing_mut().button_padding = vec2(4.0, 4.0);
        let speed_text = format!("{}×", trim_number(input.speed));
        let speed = trailing.put(
            layout.speed,
            egui::Button::new(
                egui::RichText::new(&speed_text)
                    .font(theme::medium(12.0))
                    .color(p.secondary),
            )
            .min_size(SPEED),
        );
        speed.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("Playback speed, {speed_text}"),
            )
        });
        egui::Popup::menu(&speed).show(|ui| {
            widgets::menu_style(ui, 120.0);
            for value in [0.25, 0.5, 1.0, 1.5, 2.0, 4.0] {
                let icon = (value == input.speed).then_some(Icon::Check);
                if widgets::menu_item(ui, p, icon, &format!("{}×", trim_number(value)), "")
                    .clicked()
                {
                    actions.push(emit(Transport::SetSpeed(value)));
                    ui.close();
                }
            }
        });
        speed.on_hover_text("Playback speed");
    }
    if let Some(rect) = layout.looping {
        let mut looping = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        if widgets::icon_toggle(
            &mut looping,
            p,
            Icon::Loop,
            input.looping,
            &format!("Loop playback ({})", shortcut("Cmd+L")),
        )
        .clicked()
        {
            actions.push(emit(Transport::ToggleLoop));
        }
    }
    if layout.detail {
        let right = layout.looping.unwrap_or(layout.speed).left() - 10.0;
        painter.galley(
            pos2(
                right - detail.size().x,
                rect.center().y - detail.size().y * 0.5,
            ),
            detail,
            p.muted,
        );
    }
}

/// The record control at rest, starting or closing a take: a red disc that
/// asks for a take, and gives up one that is starting.
fn draw_record(ui: &mut Ui, p: &Palette, place: Rect, view: RecordView, actions: &mut Vec<Action>) {
    let mut control = ui.new_child(egui::UiBuilder::new().max_rect(place));
    let (label, tint, enabled) = match view {
        RecordView::Starting => (
            format!("Starting to record; press to give up ({})", shortcut("R")),
            p.red.gamma_multiply(0.5),
            true,
        ),
        RecordView::Stopping => ("Closing the take…".to_owned(), p.muted, false),
        _ => (format!("Record voiceover ({})", shortcut("R")), p.red, true),
    };
    control.add_enabled_ui(enabled, |ui| {
        let pressed =
            widgets::icon_button_tinted(ui, p, Icon::Speech, &label, Some(tint)).clicked();
        if pressed {
            actions.push(match view {
                RecordView::Idle => Action::Record,
                _ => Action::StopRecord,
            });
        }
    });
}

/// A take that runs: a stop control where record was, and in speed's place
/// how long the take is with a small bar of its level. One press anywhere on
/// it ends the take.
fn draw_take(
    ui: &mut Ui,
    p: &Palette,
    place: Rect,
    elapsed: Time,
    level: f32,
    actions: &mut Vec<Action>,
) {
    let text = record::elapsed_text(elapsed);
    let label = format!("Stop recording, {text} recorded ({})", shortcut("R"));
    let response = ui.interact(place, ui.id().with("take"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let painter = ui.painter_at(place.expand(2.0));
    let radius = CornerRadius::same(metrics::CONTROL_RADIUS);
    let fill = if response.hovered() { 0.22 } else { 0.14 };
    painter.rect_filled(place, radius, theme::tint(p.red, fill));
    if response.has_focus() {
        painter.rect_stroke(
            place,
            radius,
            Stroke::new(1.5, p.accent),
            StrokeKind::Inside,
        );
    }
    let stop = Rect::from_center_size(
        pos2(place.left() + metrics::ICON_BUTTON * 0.5, place.center().y),
        Vec2::splat(metrics::ICON),
    );
    icons::paint(&painter, stop, Icon::Stop, p.red);
    // The time and the bar share what is left, centred in it.
    let rest = Rect::from_min_max(
        pos2(place.left() + metrics::ICON_BUTTON, place.top()),
        place.max,
    );
    let time = painter.layout_no_wrap(text, theme::mono(12.0), p.fg);
    painter.galley(
        pos2(
            rest.center().x - time.size().x * 0.5,
            rest.center().y - 2.5 - time.size().y * 0.5,
        ),
        time,
        p.fg,
    );
    let bar = Rect::from_center_size(
        pos2(rest.center().x, rest.center().y + 8.0),
        vec2(30.0, 3.0),
    );
    painter.rect_filled(bar, CornerRadius::same(1), theme::tint(p.fg, 0.14));
    let fill = record::level_fill(level);
    if fill > 0.0 {
        let lit = Rect::from_min_size(bar.min, vec2(bar.width() * fill, bar.height()));
        // Within 1 dB of full scale the take is clipping.
        let ink = if fill >= 59.0 / 60.0 { p.red } else { p.green };
        painter.rect_filled(lit, CornerRadius::same(1), ink);
    }
    if response.on_hover_text(label).clicked() {
        actions.push(Action::StopRecord);
    }
}

/// `1`, `1.5`, `29.97`: a number without trailing zeros.
pub fn trim_number(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Issue 12: which picture a clip that stays on screen is painted with.
#[cfg(test)]
#[path = "preview_identity_tests.rs"]
mod preview_identity_tests;

/// Issue 14: play and speed in a transport of any width.
#[cfg(test)]
#[path = "transport_tests.rs"]
mod transport_tests;

/// The viewer draws the plan: resolved transforms, transitions, fallback.
#[cfg(test)]
#[path = "plan_viewer_tests.rs"]
mod plan_viewer_tests;

#[cfg(test)]
#[path = "lut_viewer_tests.rs"]
mod lut_viewer_tests;

#[cfg(test)]
#[path = "plate_viewer_tests.rs"]
mod plate_viewer_tests;
#[cfg(test)]
#[path = "preview_quality_tests.rs"]
mod preview_quality_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn a_matching_query_resolves_the_current_picture_and_its_draft() {
        use std::sync::Arc;
        let mut project = Project::new("Current root");
        let clip = Clip::text("Current", Time::ZERO, Time(1_000_000));
        let id = clip.id;
        Arc::make_mut(&mut project.tracks[2]).clips.push(clip);
        let project = Arc::new(project);
        let query = ProjectQuery::new(Arc::clone(&project));
        let palette = Palette::new(crate::theme::Appearance::Dark, crate::theme::Accent::Blue);
        let input = ViewerInput {
            p: &palette,
            project: &project,
            query: Some(&query),
            playhead: Time(500_000),
            playing: true,
            speed: 1.0,
            looping: false,
            media: None,
            record: Default::default(),
        };
        assert!(input.indexed().is_some());
        let overrides = [ClipOverride {
            clip: id,
            transform: Some(Transform {
                position: [23.0, -17.0],
                ..Transform::default()
            }),
            ..ClipOverride::default()
        }];
        let plan = input.resolve(&overrides);
        assert_eq!(plan, resolve(&project, input.playhead, &overrides));
        assert_eq!(plan.layers[0].transform.position, [23.0, -17.0]);
    }

    #[test]
    fn mismatched_query_root_cannot_hide_a_replacement_clip() {
        use std::sync::Arc;
        let old = Arc::new(Project::new("Old root"));
        let query = ProjectQuery::new(Arc::clone(&old));
        let mut replacement = (*old).clone();
        Arc::make_mut(&mut replacement.tracks[2])
            .clips
            .push(Clip::text("New", Time::ZERO, Time(1_000_000)));
        let palette = Palette::new(crate::theme::Appearance::Dark, crate::theme::Accent::Blue);
        let input = ViewerInput {
            p: &palette,
            project: &replacement,
            query: Some(&query),
            playhead: Time::ZERO,
            playing: false,
            speed: 1.0,
            looping: false,
            media: None,
            record: Default::default(),
        };
        assert_eq!(input.active_clips().count(), 1);
        assert_eq!(input.resolve(&[]), resolve(&replacement, Time::ZERO, &[]));
        assert_eq!(input.resolve(&[]).layers.len(), 1);
    }

    use super::*;

    /// Text as wide as the transports show it, near enough.
    const TEXT: TransportText = TransportText {
        timecode: 84.0,
        duration: 104.0,
        detail: 130.0,
        record: 0.0,
    };
    /// The timeline's, with its record control.
    const RECORDING: TransportText = TransportText {
        record: metrics::ICON_BUTTON,
        ..TEXT
    };

    fn bar(width: f32) -> (Rect, TransportLayout) {
        bar_of(width, TEXT)
    }

    fn bar_of(width: f32, text: TransportText) -> (Rect, TransportLayout) {
        let rect = Rect::from_min_size(pos2(380.0, 500.0), vec2(width, TRANSPORT));
        (rect, TransportLayout::new(rect, text))
    }

    #[test]
    fn a_transport_of_any_width_keeps_its_parts_apart() {
        for text in [TEXT, RECORDING] {
            parts_apart(text);
        }
    }

    fn parts_apart(text: TransportText) {
        let slack = 0.01;
        let mut narrower = (Controls::Play, false, false, false);
        for width in 60u16..=1400 {
            let (rect, layout) = bar_of(f32::from(width), text);
            let (group, speed) = (layout.group, layout.speed);
            assert_eq!(layout.record.is_some(), text.record > 0.0);
            // What always trails starts at the record control, when there
            // is one, which keeps its gap to speed and the bar's height.
            let trailing = layout.record.map_or(speed.left(), |record| {
                assert!(
                    speed.left() - record.right() >= CONTROL_GAP - slack,
                    "{width}: record and speed touch"
                );
                assert!(
                    record.top() >= rect.top() && record.bottom() <= rect.bottom(),
                    "{width}: record is not within the bar's height"
                );
                record.left()
            });
            assert!(
                group.contains_rect(layout.play()),
                "{width}: play is outside its group"
            );
            assert!(
                group.left() >= rect.left() + TRANSPORT_TRAILING - slack,
                "{width}: the controls leave the bar at its leading edge"
            );
            let before_speed = layout.looping.map_or(trailing, |looping| {
                assert!(
                    trailing - looping.right() >= CONTROL_GAP - slack,
                    "{width}: loop and what trails it touch"
                );
                looping.left()
            });
            assert!(
                before_speed - group.right() >= GROUP_GAP - slack,
                "{width}: the controls and what trails them touch"
            );
            // Play, the gaps, record and speed need this much; anything
            // wider holds all of them.
            let record = if text.record > 0.0 {
                text.record + CONTROL_GAP
            } else {
                0.0
            };
            if f32::from(width)
                >= 2.0 * TRANSPORT_TRAILING + PLAY + 2.0 * GROUP_GAP + record + SPEED.x
            {
                assert!(
                    speed.right() <= rect.right() - TRANSPORT_TRAILING + slack,
                    "{width}: speed leaves the bar"
                );
            }
            assert!(
                speed.top() >= rect.top() && speed.bottom() <= rect.bottom(),
                "{width}: speed is not within the bar's height"
            );
            if layout.timecode {
                let mut leading = rect.left() + TRANSPORT_LEADING + text.timecode;
                if layout.duration {
                    leading += text.duration;
                }
                assert!(
                    group.left() - leading >= GROUP_GAP - slack,
                    "{width}: the timecode and the controls touch"
                );
            }
            if layout.detail {
                assert!(
                    before_speed - text.detail - group.right() >= GROUP_GAP - slack,
                    "{width}: the controls and the facts touch"
                );
            }
            assert!(!layout.duration || layout.timecode);
            assert_eq!(layout.looping.is_some(), layout.controls == Controls::Full);
            // Room never takes anything away.
            let shown = (
                layout.controls,
                layout.timecode,
                layout.duration,
                layout.detail,
            );
            assert!(
                shown.0 >= narrower.0
                    && shown.1 >= narrower.1
                    && shown.2 >= narrower.2
                    && shown.3 >= narrower.3,
                "{width}: {shown:?} is less than {narrower:?} one pixel narrower"
            );
            narrower = shown;
        }
    }

    #[test]
    fn a_transport_shows_what_its_width_has_room_for() {
        // The pane of an 800×600 window with both panels: play and speed.
        let (rect, layout) = bar(168.0);
        assert_eq!((layout.controls, layout.timecode), (Controls::Play, false));
        assert_eq!(layout.play().center().x, rect.center().x);
        assert_eq!(layout.speed.right(), rect.right() - TRANSPORT_TRAILING);
        // Then the timecode, the frame steps, and the jumps with the loop.
        let (_, layout) = bar(240.0);
        assert_eq!((layout.controls, layout.timecode), (Controls::Play, true));
        let (_, layout) = bar(300.0);
        assert_eq!((layout.controls, layout.timecode), (Controls::Steps, true));
        assert_eq!(layout.looping, None);
        let (rect, layout) = bar(498.0);
        assert_eq!((layout.controls, layout.timecode), (Controls::Full, true));
        assert_eq!(layout.group.center().x, rect.center().x);
        assert!(layout.looping.is_some() && !layout.duration && !layout.detail);
        // And a wide one everything, around controls that keep the centre.
        let (rect, layout) = bar(1000.0);
        assert!(layout.duration && layout.detail);
        assert_eq!(layout.group.center().x, rect.center().x);
    }

    #[test]
    fn the_timeline_keeps_record_just_before_speed_in_its_narrowest_pane() {
        let (rect, layout) = bar_of(168.0, RECORDING);
        let Some(record) = layout.record else {
            panic!("no record control in the narrowest pane");
        };
        assert_eq!(layout.speed.right(), rect.right() - TRANSPORT_TRAILING);
        assert_eq!(record.right() + CONTROL_GAP, layout.speed.left());
        assert!(record.left() - layout.group.right() >= GROUP_GAP);
        // A deck that records nothing lays out as it did.
        assert_eq!(bar(168.0).1.speed, layout.speed);
        assert_eq!(bar(168.0).1.record, None);
    }

    #[test]
    fn a_wide_picture_is_fitted_to_the_canvas_width() {
        let size = fitted_size(vec2(4000.0, 1000.0), [0.0; 4], vec2(1920.0, 1080.0));
        assert_eq!(size, vec2(1920.0, 480.0));
    }

    #[test]
    fn cropping_changes_the_shape_that_is_fitted() {
        // Half the width removed from a 2:1 picture leaves a square.
        let size = fitted_size(
            vec2(2000.0, 1000.0),
            [0.25, 0.0, 0.25, 0.0],
            vec2(1000.0, 1000.0),
        );
        assert_eq!(size, vec2(1000.0, 1000.0));
    }

    #[test]
    fn a_layer_asks_at_the_step_that_covers_what_it_needs() {
        assert_eq!(decode_size(600.0), [640, 640]);
        assert_eq!(decode_size(640.0), [640, 640]);
        assert_eq!(decode_size(1.0), [320, 320]);
        assert_eq!(decode_size(9000.0), [1920, 1920]);
    }

    /// An asset whose picture is `width` by `height`.
    fn pictured(width: u32, height: u32) -> Asset {
        let mut asset = Asset::new(
            "/media/shot.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(8.0),
        );
        asset.width = width;
        asset.height = height;
        asset
    }

    #[test]
    fn a_layer_asks_at_a_box_of_the_shape_of_its_picture() {
        // Wider than it is high: the step is its width.
        let wide = pictured(1920, 1080);
        assert_eq!(decode_box(1.0, &wide), [320, 180]);
        assert_eq!(decode_box(600.0, &wide), [640, 360]);
        assert_eq!(decode_box(1280.0, &wide), [1280, 720]);
        // Higher than it is wide: the step is its height.
        let tall = pictured(1080, 1920);
        assert_eq!(decode_box(600.0, &tall), [360, 640]);
        assert_eq!(decode_box(1.0, &tall), [180, 320]);
        // And square, the step is both.
        assert_eq!(decode_box(600.0, &pictured(1000, 1000)), [640, 640]);
    }

    #[test]
    fn a_layer_that_needs_all_of_a_full_picture_asks_at_the_picture() {
        let wide = pictured(1920, 1080);
        assert_eq!(decode_box(1920.0, &wide), [1920, 1080]);
        assert_eq!(decode_box(1281.0, &wide), [1920, 1080]);
        assert_eq!(decode_box(9000.0, &wide), [1920, 1080]);
        assert_eq!(decode_box(9000.0, &pictured(1080, 1920)), [1080, 1920]);
        // Five of them are 41,472,000 bytes of texels: within the 64 MiB the
        // viewer has, which four squares of 1920 a side nearly fill.
        let [width, height] = decode_box(1920.0, &wide);
        assert_eq!(5 * width as usize * height as usize * 4, 41_472_000);
        assert!(5 * u64::from(width) * u64::from(height) * 4 <= 64 * 1024 * 1024);
    }

    #[test]
    fn a_layer_never_asks_for_more_than_its_picture_has() {
        assert_eq!(decode_box(1.0, &pictured(200, 100)), [200, 100]);
        assert_eq!(decode_box(9000.0, &pictured(200, 100)), [200, 100]);
        // One side within the step and the other past it.
        assert_eq!(decode_box(1.0, &pictured(200, 640)), [100, 320]);
        assert_eq!(decode_box(1.0, &pictured(640, 200)), [320, 100]);
        assert_eq!(decode_box(1.0, &pictured(1, 1)), [1, 1]);
        // However thin, a picture is asked at a texel at least.
        assert_eq!(decode_box(1.0, &pictured(4000, 1)), [320, 1]);
        assert_eq!(decode_box(1.0, &pictured(1, 4000)), [1, 320]);
    }

    #[test]
    fn a_box_is_rounded_up_so_the_frame_fitted_inside_the_step_fits_it() {
        // 1082 lines at 320 across are 180.33: a frame rounded to 180 fits a
        // box of 181, and one of 180 would be asked for narrower than 320.
        assert_eq!(decode_box(1.0, &pictured(1920, 1082)), [320, 181]);
        assert_eq!(decode_box(1.0, &pictured(1082, 1920)), [181, 320]);
        // 1078 are 179.67, which rounds to the 180 it is also rounded up to.
        assert_eq!(decode_box(1.0, &pictured(1920, 1078)), [320, 180]);
        // Nothing is rounded where the picture divides evenly.
        assert_eq!(decode_box(960.0, &pictured(3840, 2160)), [960, 540]);
    }

    #[test]
    fn a_layer_of_a_picture_of_unknown_size_asks_at_the_square() {
        for (width, height) in [(0, 0), (1920, 0), (0, 1080)] {
            let unknown = pictured(width, height);
            assert_eq!(decode_box(600.0, &unknown), [640, 640]);
            assert_eq!(decode_box(9000.0, &unknown), [1920, 1920]);
        }
    }

    #[test]
    fn only_the_halves_of_a_cut_continue_one_another() {
        let asset = Asset::new(
            "/media/shot.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(8.0),
        );
        let mut left = Clip::from_asset(&asset, Time::from_seconds(1.0));
        left.source_in = Time::from_seconds(0.5);
        left.duration = Time::from_seconds(2.0);
        left.playback_rate = 2.0;
        let mut right = left.clone();
        right.id = uuid::Uuid::new_v4();
        right.start = left.end();
        right.source_in = left.source_time(left.end());
        assert_eq!(right.source_in, Time::from_seconds(4.5));
        assert!(continues(&left, &right));
        assert!(!continues(&right, &left));
        assert!(!continues(&left, &left));

        let changes: [fn(&mut Clip); 5] = [
            // A gap, another rate, a jump in the source, another asset, text.
            |clip: &mut Clip| clip.start.0 += 1,
            |clip: &mut Clip| clip.playback_rate = 1.0,
            |clip: &mut Clip| clip.source_in.0 += 1,
            |clip: &mut Clip| {
                clip.content = ClipContent::Media {
                    asset_id: uuid::Uuid::new_v4(),
                }
            },
            |clip: &mut Clip| *clip = Clip::text("Title", clip.start, clip.duration),
        ];
        for change in changes {
            let mut other = right.clone();
            change(&mut other);
            assert!(!continues(&left, &other));
        }
        let title = Clip::text("Title", Time::ZERO, left.start);
        let mut next = Clip::text("Title", left.start, left.duration);
        next.source_in = title.source_time(title.end());
        assert!(!continues(&title, &next));
    }

    #[test]
    fn the_halves_of_a_reversed_time_map_continue_one_another() {
        let asset = Asset::new(
            "/media/shot.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(8.0),
        );
        let mut clip = Clip::from_asset(&asset, Time::from_seconds(1.0));
        clip.duration = Time::from_seconds(4.0);
        let key = |at: f64, source: f64| editor_core::TimeKey {
            at: Time::from_seconds(at),
            source: Time::from_seconds(source),
            ease: editor_core::Ease::Linear,
        };
        clip.retime = Some(editor_core::Retime {
            keys: vec![key(0.0, 6.0), key(4.0, 0.0)],
        });
        let clip_id = clip.id;
        let mut project = editor_core::Project::new("Cut");
        std::sync::Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        std::sync::Arc::make_mut(&mut project.tracks[0])
            .clips
            .push(clip);
        let mut editor = editor_core::Editor::new(project).unwrap();
        editor
            .dispatch(editor_core::Command::SplitClip {
                clip_id,
                at: Time::from_seconds(2.5),
            })
            .unwrap();
        let clips = &editor.project().tracks[0].clips;
        let (left, right) = (&clips[0], &clips[1]);
        assert_ne!(left.source_time(left.end()), right.source_in);
        assert!(continues(left, right));
        assert!(!continues(right, left));

        let mut jumped = right.clone();
        jumped.source_in.0 += 1000;
        assert!(!continues(left, &jumped));
        let mut slower = right.clone();
        slower.retime.as_mut().unwrap().keys[0].source.0 /= 2;
        assert!(!continues(left, &slower));
    }

    /// A clip of a thirty frame a second, eight second video at one second,
    /// with this time map, and the frames it asks for at each of `frames` of
    /// the timeline from its start.
    fn frames_shown(
        duration: f64,
        keys: &[(f64, f64, editor_core::Ease)],
        frames: i64,
    ) -> Vec<i64> {
        let rate = FrameRate::default();
        let mut asset = Asset::new(
            "/media/shot.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(8.0),
        );
        asset.frame_rate = Some(rate);
        let mut clip = Clip::from_asset(&asset, Time::from_seconds(1.0));
        clip.duration = Time::from_seconds(duration);
        clip.retime = Some(editor_core::Retime {
            keys: keys
                .iter()
                .map(|&(at, source, ease)| editor_core::TimeKey {
                    at: Time::from_seconds(at),
                    source: Time::from_seconds(source),
                    ease,
                })
                .collect(),
        });
        let mut project = editor_core::Project::new("Retimed");
        std::sync::Arc::make_mut(&mut project.assets).insert(asset.id, asset.clone());
        std::sync::Arc::make_mut(&mut project.tracks[0])
            .clips
            .push(clip.clone());
        project.validate().unwrap();
        (0..frames)
            .map(|frame| {
                let playhead = clip.start + rate.frame_time(frame);
                let time = source_frame(&project, &asset, &clip, playhead);
                // On the asset's grid, so a paused playhead asks once.
                assert_eq!(rate.frame_time(rate.frame_at(time)), time);
                rate.frame_at(time)
            })
            .collect()
    }

    #[test]
    fn a_clip_at_one_rate_asks_for_the_frames_its_rate_steps_over() {
        let rate = FrameRate::default();
        let mut asset = Asset::new(
            "/media/shot.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(8.0),
        );
        asset.frame_rate = Some(rate);
        let project = editor_core::Project::new("Rate");
        let mut clip = Clip::from_asset(&asset, Time::from_seconds(1.0));
        clip.source_in = Time::from_seconds(0.5);
        clip.duration = Time::from_seconds(2.0);
        clip.playback_rate = 2.0;
        let shown: Vec<_> = (0..60)
            .map(|frame| {
                let playhead = clip.start + rate.frame_time(frame);
                rate.frame_at(source_frame(&project, &asset, &clip, playhead))
            })
            .collect();
        assert_eq!(
            shown,
            (0..60).map(|frame| 15 + 2 * frame).collect::<Vec<_>>()
        );
        // The same as a straight time map says it.
        assert_eq!(
            frames_shown(
                2.0,
                &[
                    (0.0, 0.5, editor_core::Ease::Linear),
                    (2.0, 4.5, editor_core::Ease::Linear)
                ],
                60
            ),
            shown
        );
    }

    #[test]
    fn clips_cut_on_the_frame_grid_ask_for_each_source_frame_in_turn() {
        // Frame times are rounded down to the microsecond, so a start, a
        // source time and a playhead on the grid can add up to a microsecond
        // or two before the frame they are on.
        let rate = FrameRate::default();
        let mut asset = Asset::new(
            "/media/shot.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(8.0),
        );
        asset.frame_rate = Some(rate);
        let project = editor_core::Project::new("Grid");
        for speed in [1_i64, 2, 16] {
            for (start, source) in [(0, 0), (1, 2), (2, 2), (7, 5), (5, 1)] {
                let mut clip = Clip::from_asset(&asset, rate.frame_time(start));
                clip.source_in = rate.frame_time(source);
                clip.duration = rate.frame_time(12);
                clip.playback_rate = speed as f64;
                let shown: Vec<_> = (0..12)
                    .map(|frame| {
                        let playhead = rate.frame_time(start + frame);
                        rate.frame_at(source_frame(&project, &asset, &clip, playhead))
                    })
                    .collect();
                assert_eq!(
                    shown,
                    (0..12)
                        .map(|frame| source + speed * frame)
                        .collect::<Vec<_>>(),
                    "from {source} at {start}, speed {speed}"
                );
            }
        }
    }

    #[test]
    fn a_freeze_asks_for_one_frame_however_the_playhead_moves_in_it() {
        use editor_core::Ease::{Hold, Linear};
        // One second forwards, then two held on the frame it reached.
        let shown = frames_shown(
            3.0,
            &[(0.0, 0.0, Linear), (1.0, 1.0, Linear), (3.0, 1.0, Linear)],
            90,
        );
        assert_eq!(shown[..30], (0..30).collect::<Vec<_>>());
        assert!(shown[30..].iter().all(|frame| *frame == 30));
        // A hold is a freeze on the frame it starts on.
        let shown = frames_shown(2.0, &[(0.0, 2.0, Hold), (2.0, 2.0, Linear)], 60);
        assert!(shown.iter().all(|frame| *frame == 60));
        // Held on the end of the source, it is the last frame and none after.
        let shown = frames_shown(
            2.0,
            &[(0.0, 7.0, Linear), (1.0, 8.0, Linear), (2.0, 8.0, Linear)],
            60,
        );
        assert_eq!(shown[..30], (210..240).collect::<Vec<_>>());
        assert!(shown[30..].iter().all(|frame| *frame == 239));
    }

    #[test]
    fn a_reversed_clip_asks_for_the_frames_it_shows_forwards_last_first() {
        use editor_core::Ease::Linear;
        // The whole source: its last frame first, and its first frame last.
        let forwards = frames_shown(8.0, &[(0.0, 0.0, Linear), (8.0, 8.0, Linear)], 240);
        let mut backwards = frames_shown(8.0, &[(0.0, 8.0, Linear), (8.0, 0.0, Linear)], 240);
        assert_eq!(backwards[0], 239);
        assert_eq!(backwards[239], 0);
        backwards.reverse();
        assert_eq!(backwards, forwards);
        // A span cut from inside it: nothing from past the cut.
        let forwards = frames_shown(2.0, &[(0.0, 3.0, Linear), (2.0, 5.0, Linear)], 60);
        let mut backwards = frames_shown(2.0, &[(0.0, 5.0, Linear), (2.0, 3.0, Linear)], 60);
        assert_eq!(backwards[0], 149);
        backwards.reverse();
        assert_eq!(backwards, forwards);
        // Backwards at double speed after a second forwards.
        let shown = frames_shown(
            2.0,
            &[(0.0, 1.0, Linear), (1.0, 2.0, Linear), (2.0, 0.0, Linear)],
            60,
        );
        assert_eq!(shown[..30], (30..60).collect::<Vec<_>>());
        assert_eq!(
            shown[30..],
            (0..30).map(|frame| 58 - 2 * frame).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_eased_clip_asks_for_frames_that_speed_up_and_never_step_back() {
        let ramp = editor_core::Ease::Bezier {
            x1: 0.42,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        };
        let keys = [(0.0, 0.0, ramp), (4.0, 6.0, editor_core::Ease::Linear)];
        let shown = frames_shown(4.0, &keys, 120);
        assert_eq!(shown[0], 0);
        assert!(shown.windows(2).all(|pair| pair[0] <= pair[1]));
        // Slower than the source at first, faster by the end, and inside it.
        assert!(shown[30] < 30, "{}", shown[30]);
        assert!(shown[119] - shown[89] > 30);
        assert!(shown[119] < 180);
        // The same ramp backwards never steps forwards, and ends on the first frame.
        let keys = [(0.0, 6.0, ramp), (4.0, 0.0, editor_core::Ease::Linear)];
        let shown = frames_shown(4.0, &keys, 120);
        assert_eq!(shown[0], 179);
        assert!(shown.windows(2).all(|pair| pair[0] >= pair[1]));
        assert!(shown[119] <= 2, "{}", shown[119]);
    }

    #[test]
    fn a_point_inside_a_rotated_layer_is_found() {
        let layer = Placed {
            clip: uuid::Uuid::nil(),
            center: pos2(100.0, 100.0),
            half: vec2(50.0, 10.0),
            angle: std::f32::consts::FRAC_PI_2,
            locked: false,
        };
        assert!(layer.contains(pos2(100.0, 140.0)));
        assert!(!layer.contains(pos2(140.0, 100.0)));
    }

    #[test]
    fn a_source_never_asks_for_a_frame_past_its_last() {
        let mut asset = Asset::new(
            "/media/shot.mp4".into(),
            MediaKind::Video,
            Time::from_seconds(2.0),
        );
        asset.frame_rate = Some(FrameRate::default());
        let rate = FrameRate::default();
        assert_eq!(
            source_frame_time(&asset, rate, Time::from_seconds(1.0)),
            Time::from_seconds(1.0)
        );
        assert_eq!(
            source_frame_time(&asset, rate, Time::from_seconds(2.0)),
            rate.frame_time(59)
        );
        assert_eq!(
            source_frame_time(&asset, rate, Time::from_seconds(9.0)),
            rate.frame_time(59)
        );
        asset.kind = MediaKind::Image;
        assert_eq!(
            source_frame_time(&asset, rate, Time::from_seconds(1.0)),
            Time::ZERO
        );
    }

    #[test]
    fn numbers_lose_their_trailing_zeros() {
        assert_eq!(trim_number(1.0), "1");
        assert_eq!(trim_number(29.97), "29.97");
    }
}
