//! Decoded frames as GPU textures.
//!
//! The media service decodes off the UI thread and reports frames; this keeps
//! the ones in use as textures: one poster per asset for the library and the
//! timeline, and one texture per layer the viewer is compositing. A layer
//! keeps its last frame until the next one arrives, so scrubbing never flashes
//! empty.
//!
//! A layer belongs to what it is drawn for, a clip or the source monitor, and
//! not to the asset or the size it shows. Each has a [`PreviewConsumer`] of
//! its own for as long as it is kept: its frames are asked for and taken under
//! that, so two clips of one asset at one size never share a picture, and a
//! clip keeps its own whatever else comes onto the screen or leaves it.
//!
//! The layers are kept within a [`ViewerBudget`], whatever is drawn. Each is
//! charged the larger of the picture it has and the box its next frame is
//! asked at, and a frame is asked for only at a box that was charged first, so
//! one that arrives always has room. When the layers drawn together want more
//! than the budget, [`Textures::compose`] holds them all to a smaller box
//! instead of leaving any out; a layer keeps the picture it has until the
//! smaller one arrives.
//!
//! A frame is asked for once. One that could not be decoded is not asked for
//! again until what is asked changes, so a file that is gone costs one decode
//! and one notice, not one each frame. The media service keeps a bounded
//! number of consumers asked for; a layer it had no room for asks ahead of
//! the others from then on, longest refused first, so that many layers drawn
//! together are all served in turn.
//!
//! While the clock plays, a layer asks for one frame at a time: the next is
//! asked for when that one has been decoded, and not at every time the clock
//! passes meanwhile, each of which would put off the one before it. When
//! something other than the clock changes what a layer shows, a seek, an
//! edit, another box or another clip of a cut, what it asks next takes the
//! place of whatever is still being decoded for it, as everything it asks
//! does while the clock stands still. From that moment the layer takes
//! nothing that was asked for before it, published already or not, whether or
//! not the service has room yet for what is asked in its place: it keeps the
//! picture it has until the frame it now shows arrives.
//!
//! Posters are kept within a [`PosterBudget`]. One that arrives past it takes
//! the place of the one shown longest ago, and a poster is asked for only
//! while those on screen leave it room, so a view of more media than the
//! budget holds settles with icons for the rest instead of decoding in turns.
//!
//! A layer of a video reads its frames from the video's proxy instead, when
//! proxies are read, one is ready for the video as it is now, and it has at
//! least the pixels of the box the layer is drawn at; otherwise from the
//! original. Only the pictures of layers are read so: a proxy is named in no
//! project and reaches nothing else. A layer that turns from one to the
//! other asks for its frames afresh, under a consumer of its own, so nothing
//! decoded from what it read before is taken; it keeps its picture until the
//! first frame arrives. A proxy that cannot be decoded is not read again in
//! this session, and the original is read in its place.
//!
//! The preview's quality divides the boxes the viewer asks at: see
//! [`Textures::detail`]. At `Auto` they are those of `Full` until frames are
//! measured late during playback, and then half, until playback stops.
use crate::settings::PreviewQuality;
use editor_core::{Asset, AssetId, ClipId, MediaKind, Time};
use editor_media::{Frame, MediaError, MediaService, PreviewConsumer, Proxy, ProxyStatus};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
};

/// The size asked for a poster. Frames keep their aspect inside it.
pub const POSTER: [u32; 2] = [320, 180];
/// Viewer layers that were not drawn for this many frames are released.
pub(crate) const LAYER_IDLE_FRAMES: u64 = 120;
/// The sides the boxes of the layers drawn together are held to when they do
/// not fit the viewer budget as wanted: the largest of these that fits is the
/// most any of them is asked at. They go down to one texel, so that any
/// number of layers the budget has a texel for has a step.
const LAYER_STEPS: [u32; 17] = [
    1, 2, 4, 8, 16, 32, 64, 96, 128, 192, 256, 320, 480, 640, 960, 1280, 1920,
];
/// The RGBA bytes of one texel.
const TEXEL: usize = 4;
/// How many of the last advances of the clock across the layers drawn are
/// looked at for frames that were late: see [`Preview::advance`].
const LATE_WINDOW: u32 = 16;
/// How many of those late steps `Auto` quality down for the rest of the
/// playback. One late frame, as when a decoder starts, is not enough.
const LATE_STEP: u32 = 4;
// The window is the bits of [`Preview::late`].
const _: () = assert!(LATE_WINDOW == u16::BITS);

/// The RGBA bytes of a picture that fills `size`. A box too large to count
/// comes to more than any budget.
fn box_bytes([width, height]: [u32; 2]) -> usize {
    let texels = u64::from(width) * u64::from(height);
    usize::try_from(texels)
        .unwrap_or(usize::MAX)
        .saturating_mul(TEXEL)
}

/// `size` with neither side over `cap`, nor under one texel.
fn held_to(size: [u32; 2], cap: u32) -> [u32; 2] {
    size.map(|side| side.clamp(1, cap))
}

/// The longer side of `size`.
fn long_side([width, height]: [u32; 2]) -> u32 {
    width.max(height)
}

/// Whether two descriptions of an asset decode to the same pictures and
/// sound. A name or a folder is not part of that.
pub fn same_source(a: &Asset, b: &Asset) -> bool {
    a.path == b.path
        && a.kind == b.kind
        && a.duration == b.duration
        && a.width == b.width
        && a.height == b.height
        && a.frame_rate == b.frame_rate
        && a.has_audio == b.has_audio
        && a.file_size == b.file_size
}

/// What a change of the library made out of date.
#[derive(Debug, Default, PartialEq)]
pub struct Stale {
    /// Assets that are still there but now decode from something else.
    pub changed: Vec<AssetId>,
    pub removed: Vec<AssetId>,
}

pub fn stale(old: &BTreeMap<AssetId, Asset>, new: &BTreeMap<AssetId, Asset>) -> Stale {
    let mut stale = Stale::default();
    for (id, asset) in old {
        match new.get(id) {
            Some(now) if same_source(asset, now) => {}
            Some(_) => stale.changed.push(*id),
            None => stale.removed.push(*id),
        }
    }
    stale
}

/// What a viewer layer is drawn for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Viewing {
    Clip(ClipId),
    /// The source monitor. It shows one asset at a time.
    Source,
    /// The clean plate of one of a clip's difference mattes, by the effect's
    /// ID. Only a compositor reads it: it is kept as the decoded frame and
    /// never as a texture.
    Plate(ClipId, uuid::Uuid),
}

struct Layer {
    /// What its frames are of, and the box they are asked at now. A frame of
    /// anything else is not this layer's. A layer that waits for room in the
    /// viewer budget has no box, and asks for nothing.
    asset: AssetId,
    granted: Option<[u32; 2]>,
    texture: Option<TextureHandle>,
    /// The frame this layer shows and the count it was taken at, kept for
    /// the compositor to upload from: see [`Textures::keep_frames`]. They are
    /// the bytes the worker decoded, shared and not copied.
    frame: Option<(u64, Frame)>,
    /// The source time `frame` was decoded for.
    frame_at: Time,
    /// A canvas showed this layer in the frame last painted, drawn from
    /// `frame`: nothing samples a texture of its own, so it has none, and a
    /// frame it takes is uploaded once, by the compositor. See
    /// [`Textures::settle`].
    on_canvas: bool,
    /// It is a clean plate ([`Viewing::Plate`]): no quad is ever drawn of
    /// it, so it is never given a texture.
    plate: bool,
    /// The texels of the picture it shows, as counted against the viewer
    /// budget: those of `texture`, or of `frame` while that stands for it.
    kept: usize,
    /// The frame in which a picture was last handed over to be uploaded to
    /// `texture`. A second one in the same frame takes the place of the
    /// first instead of waiting behind it: see [`Textures::accept`].
    written: Option<u64>,
    /// The source time most recently asked for, so that a still playhead
    /// asks once.
    requested: Option<Time>,
    /// The source time of the request the media service last took for this
    /// layer: what it is decoding for it, or decoded last. A request the
    /// service had no room for leaves it as it was, and so does a failure,
    /// which names no time. Nothing once something other than the clock
    /// changed what the layer shows, until the service takes a request for
    /// that: see [`Layer::jump`].
    admitted: Option<Time>,
    /// The source time the service took a request at before `admitted`, when
    /// that one was asked as the clock's next frame. The service takes such
    /// a request only once the one before it is decoded, so this frame was
    /// not put off: it may be published and not yet taken, and is then still
    /// this layer's, until one is taken. Nothing when `admitted` took the
    /// place of what was asked before, as a seek does, nor once the layer
    /// jumped or was given another box: it is only ever a frame of the clock
    /// carrying on.
    before: Option<Time>,
    /// What `admitted` was when the layer last jumped, until the service
    /// takes a request for it or a frame at that time is turned away. A
    /// layer left showing what it showed takes that request up again
    /// instead of asking the same of the service twice: see
    /// [`Layer::jump`].
    left: Option<Time>,
    /// The source time that was asked for when a frame of this layer could
    /// not be decoded. It is not asked for again at this box.
    failed: Option<Time>,
    /// When the media service first had no room for what this layer asks,
    /// as a reading of the count of such refusals, until it takes it.
    waiting: Option<u64>,
    /// What it shows was changed by something other than the clock since it
    /// last asked: what it asks next takes the place of what is still being
    /// decoded for it, playing or not.
    jumped: bool,
    used: u64,
    /// The reading of the layer clock when it was last drawn. No two layers
    /// share one, so the order they were drawn in is never a tie.
    drawn: u64,
    /// What its frames are read from in the place of its asset: the asset
    /// with the path and the size of its proxy, and nothing else changed.
    /// Nothing while the original is read.
    proxy: Option<Asset>,
    /// The source time it was last drawn at, which the clock moves on.
    wanted_at: Option<Time>,
    /// The service took a request for it whose frame has not come.
    pending: bool,
}

impl Layer {
    /// What it holds of the viewer budget: the picture it has or the box its
    /// next is asked at, whichever is more. A frame is taken only if it fits
    /// that box, so taking one never raises this.
    fn charge(&self) -> usize {
        self.kept.max(self.granted.map_or(0, box_bytes))
    }

    /// Asks at `size` from now on. What was asked at another box is no
    /// longer taken, so the frame is asked for again, whether or not it
    /// could be decoded at the other box, and in the place of what is still
    /// being decoded at it.
    fn grant(&mut self, size: Option<[u32; 2]>) {
        if self.granted != size {
            self.granted = size;
            self.requested = None;
            self.before = None;
            self.left = None;
            self.failed = None;
            self.jumped = true;
        }
    }

    /// Something other than the clock changed what this layer shows. What it
    /// asks next takes the place of what is still being decoded for it, and
    /// nothing asked for before now is taken any more: not what is published
    /// and waiting, and not while the service has no room for what is asked
    /// next. The picture it has stays until that arrives.
    ///
    /// It may be left showing what it showed: sought to where it was, or
    /// under an edit of something else. What the service had taken for it
    /// is remembered for that. When the layer is next asked for the same
    /// frame, [`Textures::ask`] takes that request up again as it stands,
    /// decoded already or still to be; one whose frame was turned away in
    /// the meantime is gone, and is asked for again.
    fn jump(&mut self) {
        self.jumped = true;
        self.before = None;
        // What it waited for is not what it shows now: not waiting for that
        // is not lateness.
        self.pending = false;
        // One jump after another leaves what the first left.
        if let Some(admitted) = self.admitted.take() {
            self.left = Some(admitted);
        }
    }

    fn forget_picture(&mut self) {
        self.texture = None;
        self.frame = None;
        self.kept = 0;
    }

    /// Gives up a picture larger than the box now asked at, when the frame
    /// that was to take its place did not come: it cannot be replaced within
    /// the budget, and the room it holds is what other layers wait for.
    /// Returns whether there was one. A picture that fits its box stays.
    fn forget_oversized(&mut self) -> bool {
        let oversized = self
            .granted
            .is_some_and(|granted| self.kept > box_bytes(granted));
        if oversized {
            self.forget_picture();
        }
        oversized
    }
}

/// A kept poster and when it was last shown.
struct Poster {
    texture: TextureHandle,
    /// Its RGBA texels, as counted against the budget.
    bytes: usize,
    /// The reading of the clock when it was last shown, asked for or decoded.
    shown: Cell<u64>,
}

/// What the retained posters may add up to: how many, and how many bytes of
/// RGBA texels. That is the pictures kept, not what uploading them or the
/// graphics driver costs on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PosterBudget {
    posters: usize,
    bytes: usize,
}

impl PosterBudget {
    /// Whichever is reached first. A full-size poster is 230,400 bytes, so
    /// 256 of them come to 56.25 MiB and the count is what binds.
    const PRODUCTION: Self = Self {
        posters: 256,
        bytes: 64 * 1024 * 1024,
    };
}

impl Default for PosterBudget {
    fn default() -> Self {
        Self::PRODUCTION
    }
}

/// What the retained posters add up to, in the terms of a [`PosterBudget`].
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PosterUsage {
    posters: usize,
    bytes: usize,
}

/// What the viewer's layers may add up to: bytes of RGBA texels, and how many
/// layers that are not drawn may be kept in the room the drawn ones leave.
/// As with posters, that is the pictures kept and not what uploading them or
/// the graphics driver costs on top.
///
/// The bytes are those the layers are charged: for each, the larger of the
/// picture it has and the box its next frame is asked at. The viewer asks at
/// a box of the shape of the picture, which a frame fitted inside it fills;
/// one held to a smaller side here may leave some of its box empty.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewerBudget {
    pub(crate) bytes: usize,
    pub(crate) spare_layers: usize,
}

impl ViewerBudget {
    /// Eight pictures of 1920 by 1080, the most the viewer asks of such an
    /// asset, come to 63.28 MiB. Of boxes 1920 a side, four come to 56.25 MiB
    /// and a fifth does not fit beside them.
    pub(crate) const PRODUCTION: Self = Self {
        bytes: 64 * 1024 * 1024,
        spare_layers: 8,
    };
}

impl Default for ViewerBudget {
    fn default() -> Self {
        Self::PRODUCTION
    }
}

/// What the preview reads in the place of large media, and how much of the
/// pictures it asks for.
#[derive(Default)]
struct Preview {
    quality: PreviewQuality,
    /// Proxies are read where one serves.
    proxies_on: bool,
    /// What there is of a proxy of each asset, as the proxy service last said.
    proxies: HashMap<AssetId, ProxyStatus>,
    /// The proxy file of each asset that could not be decoded in this
    /// session. That file is not read again; another, or the same path
    /// written afresh after the proxy stopped being ready, is.
    unreadable: HashMap<AssetId, PathBuf>,
    /// The last advances of the clock across the layers drawn while playing,
    /// newest in the lowest bit: one for an advance made before the frame
    /// asked for at the last had come.
    late: u16,
    /// Frames were measured late in this playback: `Auto` asks at half.
    stepped: bool,
}

impl Preview {
    /// The proxy a layer of `asset` drawn at `size` reads, when one serves:
    /// proxies are read, the asset is a video whose proxy is ready, of the
    /// file as the asset says it is now and not the file found unreadable,
    /// and with as many pixels on its long side as the box.
    fn serving(&self, asset: &Asset, size: [u32; 2]) -> Option<&Proxy> {
        if !self.proxies_on || asset.kind != MediaKind::Video {
            return None;
        }
        let proxy = self.ready_readable(asset.id)?;
        // `SourceIdentity::of(asset)`, field by field and without a copy.
        let source = &proxy.source;
        let current = source.path == asset.path
            && source.file_size == asset.file_size
            && source.duration == asset.duration
            && source.width == asset.width
            && source.height == asset.height
            && source.frame_rate == asset.frame_rate;
        (current && long_side(size) <= long_side([proxy.width, proxy.height])).then_some(proxy)
    }

    /// The ready proxy of `asset`, unless its file is the one that could
    /// not be decoded.
    fn ready_readable(&self, asset: AssetId) -> Option<&Proxy> {
        let ProxyStatus::Ready(proxy) = self.proxies.get(&asset)? else {
            return None;
        };
        (self.unreadable.get(&asset) != Some(&proxy.path)).then_some(proxy)
    }

    /// The clock moved a layer on while playing; `late` says the frame it
    /// asked for before had not come. Enough of the last advances late is
    /// late playback.
    fn advance(&mut self, late: bool) {
        self.late = (self.late << 1) | u16::from(late);
        if self.late.count_ones() >= LATE_STEP {
            self.stepped = true;
        }
    }

    /// Playback stopped: what was measured of it is over.
    fn stopped(&mut self) {
        self.late = 0;
        self.stepped = false;
    }
}

/// `asset` as its proxy describes it: the same identity, length and frame
/// rate, so that its frames are asked for and placed as the original's, but
/// read from the proxy's file at the proxy's size.
fn proxied(asset: &Asset, proxy: &Proxy) -> Asset {
    Asset {
        path: proxy.path.clone(),
        width: proxy.width,
        height: proxy.height,
        ..asset.clone()
    }
}

/// One picture the viewer draws this frame.
pub struct Wanted<'a> {
    pub viewing: Viewing,
    pub asset: &'a Asset,
    /// The box that covers the screen pixels it is drawn at.
    pub size: [u32; 2],
    pub time: Time,
    /// The clips whose layer this one carries on from, nearest first: the run
    /// of a cut. Empty for a clip that has a layer. The source monitor
    /// carries on from nothing.
    pub carries_on: Vec<ClipId>,
}

/// What the pictures of a frame came to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Composed {
    /// Pictures that could not be kept at even one texel each: more are
    /// drawn together than the viewer budget has texels. They are the last
    /// painted, and show nothing.
    pub unplaced: usize,
}

/// The pictures of the viewer's layers and the texels they come to.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayerUsage {
    pub(crate) pictures: usize,
    pub(crate) bytes: usize,
}

/// The layer textures `ctx` holds, whoever keeps their handles: read from its
/// texture manager, and not from what a [`Textures`] counts itself as keeping.
#[cfg(test)]
pub(crate) fn layer_textures(ctx: &egui::Context) -> LayerUsage {
    let manager = ctx.tex_manager();
    let manager = manager.read();
    let mut usage = LayerUsage {
        pictures: 0,
        bytes: 0,
    };
    for (_, meta) in manager.allocated() {
        if meta.name.starts_with("layer-") {
            usage.pictures += 1;
            usage.bytes += meta.bytes_used();
        }
    }
    usage
}

#[derive(Default)]
pub struct Textures {
    posters: HashMap<AssetId, Poster>,
    poster_budget: PosterBudget,
    /// The texels of the kept posters, kept in step with `posters`.
    poster_bytes: usize,
    /// The largest poster kept so far: the room set aside for one that has
    /// been asked for and not yet decoded.
    poster_reserve: usize,
    /// Posters asked for that have not arrived, with the proxy file each was
    /// asked of when it was not the original. Never more than the budget has
    /// posters.
    poster_requested: HashMap<AssetId, Option<PathBuf>>,
    /// Assets whose poster could not be decoded or kept; they keep their icon.
    poster_failed: HashSet<AssetId>,
    /// A poster was turned away this frame for want of room, and nothing has
    /// made room since.
    poster_refused: bool,
    /// Counts every showing of a poster, so the order they were shown in is
    /// never a tie.
    clock: Cell<u64>,
    /// The clock when this frame began, and when the one before it did. A
    /// poster shown since the earlier of the two is on screen.
    shown_this_frame: u64,
    shown_last_frame: u64,
    viewer_budget: ViewerBudget,
    /// The consumer of each thing a layer is kept for.
    viewing: HashMap<Viewing, PreviewConsumer>,
    layers: HashMap<PreviewConsumer, Layer>,
    /// Consumers whose layers were given up, until the media service is told.
    retired: Vec<PreviewConsumer>,
    frame: u64,
    /// Counts every drawing of a layer.
    layer_clock: u64,
    /// What the frame last composed came to, and how many pictures were last
    /// said to be unplaced.
    composed: Composed,
    unplaced_said: usize,
    /// Counts every request the media service had no room for.
    refusals: u64,
    /// The consumers whose decoders the media service is asked to keep open:
    /// see [`Self::hint_retention`]. Each keeps its place for as long as it
    /// is one of them.
    retention: [Option<PreviewConsumer>; 3],
    /// What the media service was last told of them and still holds.
    retention_told: [Option<PreviewConsumer>; 3],
    /// Frames are now taken at the end of a frame: see [`Self::take_late`].
    late: bool,
    /// Whether a layer keeps the frame its picture was uploaded from: see
    /// [`Self::keep_frames`].
    keeps_frames: bool,
    /// Counts every frame a layer took, so each has a name of its own.
    taken: u64,
    preview: Preview,
}

/// The decoded frame a layer shows, for a compositor that uploads its own
/// copy. `serial` names this taking of it: the same for as long as the layer
/// shows this picture, whoever the layer is handed to, and never used again.
#[derive(Clone, Copy)]
pub struct Input<'a> {
    pub serial: u64,
    pub frame: &'a Frame,
    /// The source time the frame was decoded for: what of its asset it
    /// shows, which is not the playhead's while the next frame is awaited.
    pub time: Time,
}

/// The picture of a decoded frame as the renderer wants it. A frame is
/// straight alpha whatever it was decoded from. Most video carries no
/// transparency, and then its bytes are already what the renderer wants and
/// need no per-pixel conversion: `video` says the frame is worth looking
/// through for that. One with a texel that is not opaque is multiplied by
/// its alpha like a still, as the compositor reads it.
fn image_of(frame: &Frame, video: bool) -> ColorImage {
    let size = [frame.width as usize, frame.height as usize];
    if video
        && frame
            .rgba
            .chunks_exact(TEXEL)
            .all(|texel| texel[3] == u8::MAX)
    {
        ColorImage::from_rgba_premultiplied(size, &frame.rgba)
    } else {
        ColorImage::from_rgba_unmultiplied(size, &frame.rgba)
    }
}

impl Textures {
    /// Forgets everything; used when another project replaces this one. The
    /// budgets are not the project's and stay.
    pub fn clear(&mut self) {
        let mut retired = std::mem::take(&mut self.retired);
        retired.extend(self.layers.keys());
        *self = Self {
            poster_budget: self.poster_budget,
            viewer_budget: self.viewer_budget,
            keeps_frames: self.keeps_frames,
            // A frame of the project that was is never named again.
            taken: self.taken,
            retired,
            // The person's choices and the proxies known are not the
            // project's; assets of another project have other IDs.
            preview: Preview {
                late: 0,
                stepped: false,
                ..std::mem::take(&mut self.preview)
            },
            ..Self::default()
        };
    }

    /// The quality the preview is drawn at.
    pub fn set_quality(&mut self, quality: PreviewQuality) {
        self.preview.quality = quality;
    }

    /// What the long side of every box the viewer asks at, and of every
    /// canvas it composites, is divided by: 1, 2 or 4. At `Auto`, 2 once
    /// frames were measured late in this playback, and 1 otherwise: never
    /// another merely because playback started.
    pub fn detail(&self) -> u32 {
        self.preview.quality.divisor(self.preview.stepped)
    }

    /// Whether frames were measured late in this playback.
    #[cfg(test)]
    pub(crate) fn playing_late(&self) -> bool {
        self.preview.stepped
    }

    /// Whether proxies are read where one serves. Turned off, every layer
    /// reading one turns back to its original when it is next composed.
    pub fn read_proxies(&mut self, on: bool) {
        self.preview.proxies_on = on;
    }

    /// Whether proxies are read where one serves: Preview › Proxies is not
    /// Off.
    pub fn reads_proxies(&self) -> bool {
        self.preview.proxies_on
    }

    /// What the proxy service says there is of a proxy of `asset`. Anything
    /// but the very proxy file found unreadable being ready forgets that it
    /// was: a proxy removed, made again or replaced by another is tried.
    pub fn set_proxy(&mut self, asset: AssetId, status: ProxyStatus) {
        let same_file = matches!(
            (&status, self.preview.unreadable.get(&asset)),
            (ProxyStatus::Ready(proxy), Some(failed)) if proxy.path == *failed
        );
        if !same_file {
            self.preview.unreadable.remove(&asset);
        }
        if status == ProxyStatus::None {
            self.preview.proxies.remove(&asset);
        } else {
            self.preview.proxies.insert(asset, status);
        }
    }

    /// Forgets every proxy status: the project is another, or the cache was
    /// emptied.
    pub fn forget_proxies(&mut self) {
        self.preview.proxies.clear();
    }

    /// What the proxy service last said there is of a proxy of `asset`.
    pub fn proxy_status(&self, asset: AssetId) -> &ProxyStatus {
        self.preview
            .proxies
            .get(&asset)
            .unwrap_or(&ProxyStatus::None)
    }

    /// Whether the proxy of `asset` that is ready could not be decoded in
    /// this session, so its original is read.
    pub fn proxy_unreadable(&self, asset: AssetId) -> bool {
        let ready = matches!(
            self.preview.proxies.get(&asset),
            Some(ProxyStatus::Ready(_))
        );
        ready && self.preview.ready_readable(asset).is_none()
    }

    /// Forgets that the proxy of `asset` could not be decoded: the person
    /// asked for it to be made again, or removed it.
    pub fn forget_unreadable(&mut self, asset: AssetId) {
        self.preview.unreadable.remove(&asset);
    }

    /// The proxy files layers read now, each once and in order: those
    /// decoders may have open.
    pub fn proxies_open(&self) -> Vec<PathBuf> {
        let mut open: Vec<PathBuf> = (self.layers.values())
            .filter_map(|layer| Some(layer.proxy.as_ref()?.path.clone()))
            .collect();
        open.sort();
        open.dedup();
        open
    }

    /// The source time the layer kept for `viewing` last asked for, while
    /// that is what it waits for or shows.
    #[cfg(test)]
    pub(crate) fn requested(&self, viewing: Viewing) -> Option<Time> {
        self.layers.get(self.viewing.get(&viewing)?)?.requested
    }

    /// The file the layer kept for `viewing` reads its frames from: its
    /// proxy's, or nothing while it reads the original.
    #[cfg(test)]
    pub(crate) fn reads(&self, viewing: Viewing) -> Option<&std::path::Path> {
        let layer = self.layers.get(self.viewing.get(&viewing)?)?;
        Some(layer.proxy.as_ref()?.path.as_path())
    }

    /// The layer `consumer` has reads its frames from `reads` from now on:
    /// a proxy, or the original for nothing. It is asked again as a layer
    /// whose box changed is, under a consumer of its own, so whatever was
    /// asked, decoded or published for it from the other file is not taken,
    /// and the service lets go of that decoder. Its picture, its box and
    /// its place among the decoders kept open stay. Returns the consumer it
    /// has now.
    fn read_from(&mut self, consumer: PreviewConsumer, reads: Option<Asset>) -> PreviewConsumer {
        let Some(mut layer) = self.layers.remove(&consumer) else {
            return consumer;
        };
        layer.proxy = reads;
        layer.requested = None;
        layer.admitted = None;
        layer.before = None;
        layer.left = None;
        layer.failed = None;
        layer.waiting = None;
        layer.pending = false;
        layer.jumped = true;
        let fresh = PreviewConsumer::new();
        self.layers.insert(fresh, layer);
        for kept in self.viewing.values_mut() {
            if *kept == consumer {
                *kept = fresh;
            }
        }
        for place in &mut self.retention {
            if *place == Some(consumer) {
                *place = Some(fresh);
            }
        }
        self.retired.push(consumer);
        fresh
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        self.release_layers(|layer| frame.saturating_sub(layer.used) > LAYER_IDLE_FRAMES);
        self.shown_last_frame = self.shown_this_frame;
        self.shown_this_frame = self.clock.get();
        self.poster_refused = false;
    }

    /// The poster of an asset, when it is kept. Reading it is what says the
    /// poster is on screen.
    pub fn poster(&self, asset: AssetId) -> Option<&TextureHandle> {
        let poster = self.posters.get(&asset)?;
        poster.shown.set(self.tick());
        Some(&poster.texture)
    }

    /// Asks once for the poster of an asset that can have one. It is read
    /// from the asset's proxy when one serves: at a poster's size the picture
    /// is the same, and a 4K original's frame two seconds in can take a
    /// second to decode where the proxy's takes a few milliseconds.
    pub fn want_poster(&mut self, media: &MediaService, asset: &Asset) {
        let Some(time) = self.claim_poster(asset) else {
            return;
        };
        let proxy = self.preview.serving(asset, POSTER);
        let source = proxy.map(|proxy| proxied(asset, proxy));
        if media
            .request_poster(source.as_ref().unwrap_or(asset), time, POSTER)
            .is_err()
        {
            // The worker was busy; ask again on a later frame.
            self.unclaim_poster(asset.id);
        } else if let Some(source) = source {
            self.poster_requested.insert(asset.id, Some(source.path));
        }
    }

    /// Whether a poster was turned away this frame for want of room that the
    /// next frame may have: something kept was on screen in the last frame
    /// and not in this one. Nothing else would bring that frame about.
    pub fn poster_room_is_due(&self) -> bool {
        self.poster_refused
            && self.posters.values().any(|poster| {
                let shown = poster.shown.get();
                shown > self.shown_last_frame && shown <= self.shown_this_frame
            })
    }

    /// Marks the poster of `asset` as asked for and returns the source time
    /// to ask at, or nothing when it is kept, was asked for, has failed or
    /// has no room among the posters on screen.
    fn claim_poster(&mut self, asset: &Asset) -> Option<Time> {
        // Sound, a lookup table and a font have no picture: they keep their
        // icon.
        if !matches!(
            asset.kind,
            MediaKind::Video | MediaKind::Image | MediaKind::Gif
        ) {
            return None;
        }
        if let Some(poster) = self.posters.get(&asset.id) {
            // Wanted is as good as shown.
            poster.shown.set(self.tick());
            return None;
        }
        if self.poster_failed.contains(&asset.id) || self.poster_requested.contains_key(&asset.id) {
            return None;
        }
        // Room only shrinks while a frame is drawn, so one refusal answers
        // for the rest of it.
        if self.poster_refused || !self.has_poster_room() {
            self.poster_refused = true;
            return None;
        }
        self.poster_requested.insert(asset.id, None);
        // A little way in: the very first frame of a video is often black.
        Some(match asset.kind {
            MediaKind::Video => Time(asset.duration.0 / 10).min(Time::from_seconds(2.0)),
            _ => Time::ZERO,
        })
    }

    /// Takes back a claim whose request was not taken.
    fn unclaim_poster(&mut self, asset: AssetId) {
        self.poster_requested.remove(&asset);
        self.poster_refused = false;
    }

    /// Whether one more poster can be asked for without giving up one that
    /// is on screen: those on screen and those on their way leave room for
    /// it, by count and by bytes.
    fn has_poster_room(&self) -> bool {
        let (mut on_screen, mut bytes) = (0, 0);
        for poster in self.posters.values() {
            if poster.shown.get() > self.shown_last_frame {
                on_screen += 1;
                bytes += poster.bytes;
            }
        }
        let asked = self.poster_requested.len();
        on_screen + asked < self.poster_budget.posters
            && bytes + (asked + 1) * self.poster_reserve <= self.poster_budget.bytes
    }

    fn tick(&self) -> u64 {
        let now = self.clock.get() + 1;
        self.clock.set(now);
        now
    }

    fn drop_poster(&mut self, asset: AssetId) {
        if let Some(poster) = self.posters.remove(&asset) {
            self.poster_bytes -= poster.bytes;
        }
    }

    /// Keeps a decoded poster, giving up those shown longest ago until the
    /// budget holds. However it came to arrive, it is held to the budget.
    fn keep_poster(&mut self, ctx: &egui::Context, asset: AssetId, image: ColorImage) {
        self.poster_requested.remove(&asset);
        self.poster_refused = false;
        let budget = self.poster_budget;
        let bytes = image.size[0] * image.size[1] * 4;
        if budget.posters == 0 || bytes > budget.bytes {
            // No room could be made for it, so it is not asked for again.
            self.drop_poster(asset);
            self.poster_failed.insert(asset);
            return;
        }
        self.poster_failed.remove(&asset);
        let shown = Cell::new(self.tick());
        match self.posters.get_mut(&asset) {
            Some(poster) => {
                poster.texture.set(image, TextureOptions::LINEAR);
                self.poster_bytes = self.poster_bytes - poster.bytes + bytes;
                poster.bytes = bytes;
                poster.shown = shown;
            }
            None => {
                let texture =
                    ctx.load_texture(format!("poster-{asset}"), image, TextureOptions::LINEAR);
                self.posters.insert(
                    asset,
                    Poster {
                        texture,
                        bytes,
                        shown,
                    },
                );
                self.poster_bytes += bytes;
            }
        }
        self.poster_reserve = self.poster_reserve.max(bytes);
        while self.posters.len() > budget.posters || self.poster_bytes > budget.bytes {
            let oldest = self
                .posters
                .iter()
                .filter(|(id, _)| **id != asset)
                .min_by_key(|(_, poster)| poster.shown.get())
                .map(|(id, _)| *id);
            let Some(oldest) = oldest else { break };
            self.drop_poster(oldest);
        }
    }

    #[cfg(test)]
    fn with_poster_budget(poster_budget: PosterBudget) -> Self {
        Self {
            poster_budget,
            ..Self::default()
        }
    }

    /// Measured from the textures themselves, as they are kept now.
    #[cfg(test)]
    fn poster_usage(&self) -> PosterUsage {
        PosterUsage {
            posters: self.posters.len(),
            bytes: self
                .posters
                .values()
                .map(|poster| {
                    let [width, height] = poster.texture.size();
                    width * height * 4
                })
                .sum(),
        }
    }

    /// One layer outside any frame's plan, as the tests of a single layer
    /// keep it: drawn for `viewing`, asked at `size` when the viewer budget
    /// has room for that box, and asked for the frame at `time` when it is
    /// not the one already asked for. Returns its texture. The layer is its
    /// own from the first call until it has gone undrawn for a while, at any
    /// size; one that turns to another asset starts over with nothing to show.
    #[cfg(test)]
    pub fn layer(
        &mut self,
        media: Option<&MediaService>,
        viewing: Viewing,
        asset: &Asset,
        size: [u32; 2],
        time: Time,
    ) -> Option<TextureHandle> {
        self.retire(media);
        let consumer = self.draw(viewing, asset.id);
        self.retire(media);
        let charged = self.charged();
        let budget = self.viewer_budget.bytes;
        let layer = self.layers.get_mut(&consumer)?;
        let more = layer
            .kept
            .max(box_bytes(size))
            .saturating_sub(layer.charge());
        if charged.saturating_add(more) <= budget {
            layer.grant(Some(size));
        }
        self.ask(media, consumer, asset, time, false);
        self.layers.get(&consumer)?.texture.clone()
    }

    /// Keeps a layer for each picture drawn this frame, in paint order, and
    /// asks for the frames they need, within the viewer budget. What each
    /// then shows is read with [`Self::picture`].
    ///
    /// Every picture drawn is kept a layer; none is left out to make room.
    /// When they do not all fit at the boxes they are wanted at, all are held
    /// to the largest of [`LAYER_STEPS`] at which they do. In order:
    ///
    /// 1. A layer kept for something drawn is marked as drawn. A clip that
    ///    has none then takes the one kept for the nearest clip it carries on
    ///    from, where [`Self::hand_over`] allows, or starts a new one.
    /// 2. A layer asked at more than it is now held to asks at the smaller
    ///    box. It keeps the picture it has, and what that holds of the
    ///    budget, until the smaller frame arrives.
    /// 3. Only then are layers that were not drawn given up, down to the
    ///    spare ones the budget allows: after any of them changed hands.
    /// 4. A layer with no box, or asked at less than it is now held to, is
    ///    given the box when the budget has room, giving up spare layers where
    ///    that makes the room. Otherwise it waits with what it has, asking
    ///    for nothing, until a smaller frame of another layer arrives; that
    ///    arrival is what brings the next frame about. While `playing`, a
    ///    layer that has a box is not asked for more; one that has none is.
    ///
    /// Beyond one texel for each picture there is nothing to hold them to.
    /// The first painted then keep a texel each and the rest are counted in
    /// [`Composed::unplaced`], with no box and no picture.
    ///
    /// The frames are asked for last, after the media service is told which
    /// decoders to keep open ([`Self::hint_retention`]). Layers it had no
    /// room for are asked first, longest refused first, and then the rest in
    /// paint order: a layer that was served asks again behind those still
    /// waiting, so every layer with a box is asked for in turn however many
    /// are drawn. The order changes nothing else; which layer is whose, what
    /// box each has and what is painted over what are settled before it.
    ///
    /// While `playing`, a layer asks as the clock's next frame, which the
    /// media service takes only once the one before it is decoded; a layer
    /// still waiting for its frame waits its turn like one there was no room
    /// for. A layer that [`Self::jumped`], or whose box or clip changed,
    /// asks as it does when paused, in the place of what it waits for.
    pub fn compose(
        &mut self,
        media: Option<&MediaService>,
        playing: bool,
        wanted: &[Wanted<'_>],
    ) -> Composed {
        self.retire(media);
        let frame = self.frame;
        let budget = self.viewer_budget;
        // Something drawn twice is one picture.
        let mut seen = HashSet::with_capacity(wanted.len());
        let wanted: Vec<&Wanted<'_>> = wanted
            .iter()
            .filter(|want| seen.insert(want.viewing))
            .collect();

        // Layers that are kept are marked first, so that none is taken from
        // a clip painted later in this frame.
        let mut drawn: Vec<Option<PreviewConsumer>> = wanted
            .iter()
            .map(|want| self.draw_kept(want.viewing, want.asset.id))
            .collect();
        for (want, consumer) in wanted.iter().zip(&mut drawn) {
            if consumer.is_some() {
                continue;
            }
            if matches!(want.viewing, Viewing::Clip(_)) {
                for donor in &want.carries_on {
                    let donor = Viewing::Clip(*donor);
                    // A layer of another asset would only be started over.
                    if self.layer_asset(donor) == Some(want.asset.id)
                        && self.hand_over(donor, want.viewing)
                    {
                        break;
                    }
                }
            }
            *consumer = Some(self.draw(want.viewing, want.asset.id));
        }
        let mut drawn: Vec<PreviewConsumer> = drawn.into_iter().flatten().collect();

        // The most any side is asked at: the same for every layer, and a
        // matter of what is drawn alone, not of what happens to be kept.
        let fits = |cap: u32| {
            wanted
                .iter()
                .try_fold(0usize, |sum, want| {
                    let sum = sum.checked_add(box_bytes(held_to(want.size, cap)))?;
                    (sum <= budget.bytes).then_some(sum)
                })
                .is_some()
        };
        let cap = LAYER_STEPS
            .partition_point(|cap| fits(*cap))
            .checked_sub(1)
            .map(|step| LAYER_STEPS[step]);
        let held = |index: usize, size: [u32; 2]| match cap {
            Some(cap) => Some(held_to(size, cap)),
            None => (index < budget.bytes / TEXEL).then_some([1, 1]),
        };

        // Each reads from its proxy where one serves the box it is asked at,
        // and from its original otherwise. That box is the one it is held to,
        // and while playing no larger than the one it was granted, which
        // playback does not grow: a box asked for and never granted would
        // turn to the original for nothing. A layer that turns from one to
        // the other is asked for afresh under another consumer.
        for (index, (want, consumer)) in wanted.iter().zip(&mut drawn).enumerate() {
            let Some(layer) = self.layers.get(consumer) else {
                continue;
            };
            let target = held(index, want.size).unwrap_or(want.size);
            let asked = match layer.granted {
                Some(granted) if playing && long_side(granted) < long_side(target) => granted,
                _ => target,
            };
            let serving = self.preview.serving(want.asset, asked);
            let same = match (&layer.proxy, serving) {
                (None, None) => true,
                (Some(read), Some(proxy)) => {
                    read.path == proxy.path
                        && read.width == proxy.width
                        && read.height == proxy.height
                }
                _ => false,
            };
            if !same {
                let reads = serving.map(|proxy| proxied(want.asset, proxy));
                *consumer = self.read_from(*consumer, reads);
            }
        }
        let drawn = drawn;

        // Whether the frames of this playback come late: each time the clock
        // moves a video's layer on, whether the frame it asked for at the
        // time before had come. Paused, nothing is late.
        for (want, consumer) in wanted.iter().zip(&drawn) {
            let Some(layer) = self.layers.get_mut(consumer) else {
                continue;
            };
            if layer.wanted_at == Some(want.time) {
                continue;
            }
            let moved_on = layer.wanted_at.replace(want.time).is_some();
            if playing && moved_on && want.asset.kind == MediaKind::Video && !layer.plate {
                self.preview.advance(layer.pending);
            }
        }
        if !playing {
            self.preview.stopped();
        }

        let mut unplaced = 0;
        for (index, (want, consumer)) in wanted.iter().zip(&drawn).enumerate() {
            let Some(layer) = self.layers.get_mut(consumer) else {
                continue;
            };
            match held(index, want.size) {
                Some(target) => {
                    let smaller = layer
                        .granted
                        .is_some_and(|granted| target[0] <= granted[0] && target[1] <= granted[1]);
                    if smaller {
                        layer.grant(Some(target));
                    }
                }
                None => {
                    layer.grant(None);
                    layer.forget_picture();
                    unplaced += 1;
                }
            }
        }

        self.keep_spares(budget.spare_layers);

        // The spare layers, least recently drawn last, and the room they hold.
        let mut spares: Vec<(u64, PreviewConsumer, usize)> = self
            .layers
            .iter()
            .filter(|(_, layer)| layer.used != frame)
            .map(|(consumer, layer)| (layer.drawn, *consumer, layer.charge()))
            .collect();
        spares.sort_unstable_by_key(|(drawn, ..)| std::cmp::Reverse(*drawn));
        let mut spare_room = spares
            .iter()
            .fold(0usize, |sum, (.., room)| sum.saturating_add(*room));
        let mut charged = self.charged();
        for (index, (want, consumer)) in wanted.iter().zip(&drawn).enumerate() {
            let Some(target) = held(index, want.size) else {
                continue;
            };
            let Some(layer) = self.layers.get(consumer) else {
                continue;
            };
            if layer.granted == Some(target) || (playing && layer.granted.is_some()) {
                continue;
            }
            let more = layer
                .kept
                .max(box_bytes(target))
                .saturating_sub(layer.charge());
            // No spare layer is given up for a box that would still not fit.
            if charged.saturating_sub(spare_room).saturating_add(more) > budget.bytes {
                continue;
            }
            while charged.saturating_add(more) > budget.bytes {
                let Some((_, spare, room)) = spares.pop() else {
                    break;
                };
                self.release(spare);
                charged = charged.saturating_sub(room);
                spare_room = spare_room.saturating_sub(room);
            }
            if let Some(layer) = self.layers.get_mut(consumer) {
                layer.grant(Some(target));
                charged = charged.saturating_add(more);
            }
        }

        // Those given up are told of first: what was queued for them makes
        // way for what is asked now.
        self.retire(media);
        self.hint_retention(media, &wanted, &drawn);
        let mut asks: Vec<(u64, &Wanted<'_>, PreviewConsumer)> = wanted
            .iter()
            .zip(&drawn)
            .map(|(want, consumer)| {
                let since = self
                    .layers
                    .get(consumer)
                    .and_then(|layer| layer.waiting)
                    .unwrap_or(u64::MAX);
                (since, *want, *consumer)
            })
            .collect();
        // Stable: layers that do not wait keep their paint order.
        asks.sort_by_key(|(since, ..)| *since);
        for (_, want, consumer) in asks {
            self.ask(media, consumer, want.asset, want.time, playing);
        }
        self.composed = Composed { unplaced };
        self.composed
    }

    /// The frame last composed, when pictures went unplaced in it and that
    /// many were not said to be before: once for as long as it stays so, and
    /// again when it has changed or has stopped in between.
    pub fn newly_unplaced(&mut self) -> Option<Composed> {
        let unplaced = self.composed.unplaced;
        if unplaced == self.unplaced_said {
            return None;
        }
        self.unplaced_said = unplaced;
        (unplaced > 0).then_some(self.composed)
    }

    /// Chooses the consumers whose decoders the media service is asked to
    /// keep open, and tells it when they are not those it holds.
    ///
    /// They are at most three of the layers drawn this frame that are of a
    /// video and have a box: only a video has a decoder to keep, and a layer
    /// that waits for room asks for nothing. One chosen before stays chosen,
    /// in its place, for as long as it is such a layer, so a clip that takes
    /// over the layer of a cut keeps it and a change of paint order moves
    /// none; the places left are filled in paint order. Nothing drawn that
    /// qualifies leaves none chosen, as when the source monitor shows a still
    /// or a sound.
    ///
    /// It is a hint about decoders alone. It asks for no frame, and no
    /// layer's picture, box or turn depends on it; a service that does not
    /// take it is told again in a later frame and nothing else follows.
    fn hint_retention(
        &mut self,
        media: Option<&MediaService>,
        wanted: &[&Wanted<'_>],
        drawn: &[PreviewConsumer],
    ) {
        let videos: Vec<PreviewConsumer> = wanted
            .iter()
            .zip(drawn)
            .filter(|(want, consumer)| {
                want.asset.kind == MediaKind::Video
                    // A plate is one frame of its file, asked for once.
                    && !matches!(want.viewing, Viewing::Plate(..))
                    && self
                        .layers
                        .get(*consumer)
                        .is_some_and(|layer| layer.granted.is_some())
            })
            .map(|(_, consumer)| *consumer)
            .collect();
        let mut retention = self
            .retention
            .map(|kept| kept.filter(|consumer| videos.contains(consumer)));
        for video in videos {
            if retention.contains(&Some(video)) {
                continue;
            }
            let Some(free) = retention.iter_mut().find(|kept| kept.is_none()) else {
                break;
            };
            *free = Some(video);
        }
        self.retention = retention;
        if let Some(media) = media
            && self.retention_told != retention
            && media.set_preview_retention(retention).is_ok()
        {
            self.retention_told = retention;
        }
    }

    /// The consumer of the layer kept for `viewing`, marked as drawn this
    /// frame, when it is of `asset`. One of another asset is given up.
    fn draw_kept(&mut self, viewing: Viewing, asset: AssetId) -> Option<PreviewConsumer> {
        let consumer = *self.viewing.get(&viewing)?;
        self.layer_clock += 1;
        let (frame, now) = (self.frame, self.layer_clock);
        if let Some(layer) = self.layers.get_mut(&consumer)
            && layer.asset == asset
        {
            layer.used = frame;
            layer.drawn = now;
            return Some(consumer);
        }
        // Frames of the other asset may be on their way under the old
        // consumer; under a new one they have nowhere to arrive.
        self.viewing.remove(&viewing);
        if self.layers.remove(&consumer).is_some() {
            self.retired.push(consumer);
        }
        None
    }

    /// The consumer of the layer drawn for `viewing` this frame: the one
    /// kept for it, or a new one with no box and nothing to show.
    fn draw(&mut self, viewing: Viewing, asset: AssetId) -> PreviewConsumer {
        if let Some(consumer) = self.draw_kept(viewing, asset) {
            return consumer;
        }
        self.layer_clock += 1;
        let consumer = *self.viewing.entry(viewing).or_default();
        self.layers.insert(
            consumer,
            Layer {
                asset,
                granted: None,
                texture: None,
                frame: None,
                frame_at: Time::ZERO,
                on_canvas: false,
                plate: matches!(viewing, Viewing::Plate(..)),
                kept: 0,
                written: None,
                requested: None,
                admitted: None,
                before: None,
                left: None,
                failed: None,
                waiting: None,
                jumped: false,
                used: self.frame,
                drawn: self.layer_clock,
                proxy: None,
                wanted_at: None,
                pending: false,
            },
        );
        consumer
    }

    /// The asset the layer kept for `viewing` is of.
    fn layer_asset(&self, viewing: Viewing) -> Option<AssetId> {
        let consumer = self.viewing.get(&viewing)?;
        Some(self.layers.get(consumer)?.asset)
    }

    /// What the layers hold of the viewer budget between them.
    fn charged(&self) -> usize {
        self.layers
            .values()
            .fold(0, |sum: usize, layer| sum.saturating_add(layer.charge()))
    }

    /// Asks for the frame of `asset` at `time` under `consumer`, when its
    /// layer has a box and that is not the frame already asked for, nor one
    /// that could not be decoded at this box.
    ///
    /// While `playing` it is asked as the next frame of the clock, which the
    /// media service takes only when it has decoded the one this layer asked
    /// before: the clock moving on does not put that one off. Paused, or when
    /// something other than the clock changed what the layer shows, it is
    /// asked in the place of whatever is still being decoded for it.
    fn ask(
        &mut self,
        media: Option<&MediaService>,
        consumer: PreviewConsumer,
        asset: &Asset,
        time: Time,
        playing: bool,
    ) {
        let (Some(media), Some(layer)) = (media, self.layers.get_mut(&consumer)) else {
            return;
        };
        let Some(size) = layer.granted else {
            return;
        };
        if layer.requested == Some(time) {
            // What it waits for is what it shows now, however it came to:
            // the request it jumped away from is the one it now asks.
            if layer.admitted.is_none() {
                layer.admitted = layer.left.take();
            }
            layer.jumped = false;
            return;
        }
        if layer.failed == Some(time) {
            return;
        }
        let clocked = playing && !layer.jumped;
        // Only a picture request is ever given the proxy.
        let read = layer.proxy.as_ref().unwrap_or(asset);
        let asked = if clocked {
            media.request_playback_preview(consumer, read, time, size)
        } else {
            media.request_preview(consumer, read, time, size)
        };
        match asked {
            Ok(_) => {
                layer.pending = true;
                layer.requested = Some(time);
                layer.before = layer.admitted.filter(|_| clocked);
                // The service asks at no time before zero, and says so.
                layer.admitted = Some(time.max(Time::ZERO));
                layer.left = None;
                layer.failed = None;
                layer.waiting = None;
                layer.jumped = false;
            }
            // The worker has as much queued as it takes, or is still on the
            // frame this layer asked before, and reports as it works through
            // it: the frame that brings about asks again, this layer ahead of
            // those refused after it. The box stays charged, so the room is
            // there when it is asked.
            Err(MediaError::Busy) => {
                if layer.waiting.is_none() {
                    self.refusals += 1;
                    layer.waiting = Some(self.refusals);
                }
            }
            // A worker that has stopped decodes nothing for any layer, and
            // nothing waits its turn with it.
            Err(MediaError::Closed) => {}
            // It will not decode this at all, and says so at once: nothing
            // is published and nothing woken, so it is asked again. A picture
            // that was to be replaced by a smaller one is not kept waiting
            // for it.
            Err(_) => {
                layer.forget_oversized();
            }
        }
    }

    /// Gives up the layers that were not drawn this frame, but for the
    /// `spare` drawn most recently.
    fn keep_spares(&mut self, spare: usize) {
        let frame = self.frame;
        let mut undrawn: Vec<u64> = self
            .layers
            .values()
            .filter(|layer| layer.used != frame)
            .map(|layer| layer.drawn)
            .collect();
        if undrawn.len() <= spare {
            return;
        }
        undrawn.sort_unstable();
        let last_given_up = undrawn[undrawn.len() - spare - 1];
        self.release_layers(|layer| layer.used != frame && layer.drawn <= last_given_up);
    }

    /// Gives up the layer `consumer` has, and what it was kept for.
    fn release(&mut self, consumer: PreviewConsumer) {
        if self.layers.remove(&consumer).is_some() {
            self.retired.push(consumer);
        }
        self.viewing.retain(|_, kept| *kept != consumer);
    }

    /// The picture the layer kept for `viewing` shows, when it has one.
    pub fn picture(&self, viewing: Viewing) -> Option<&TextureHandle> {
        let consumer = self.viewing.get(&viewing)?;
        self.layers.get(consumer)?.texture.as_ref()
    }

    /// The decoded frame behind [`Self::picture`], when frames are kept.
    pub fn input(&self, viewing: Viewing) -> Option<Input<'_>> {
        let consumer = self.viewing.get(&viewing)?;
        let layer = self.layers.get(consumer)?;
        let (serial, frame) = layer.frame.as_ref()?;
        Some(Input {
            serial: *serial,
            frame,
            time: layer.frame_at,
        })
    }

    /// Whether decoded frames are kept: see [`Self::keep_frames`].
    pub fn keeps_frames(&self) -> bool {
        self.keeps_frames
    }

    /// The size of the picture the layer kept for `viewing` shows, whether
    /// it is kept as a texture or as the frame a canvas is drawn from.
    pub fn picture_size(&self, viewing: Viewing) -> Option<[u32; 2]> {
        let layer = self.layers.get(self.viewing.get(&viewing)?)?;
        match (&layer.texture, &layer.frame) {
            (Some(texture), _) => Some(texture.size().map(|side| side as u32)),
            (None, Some((_, frame))) => Some([frame.width, frame.height]),
            (None, None) => None,
        }
    }

    /// Says whether the layers of clips keep the decoded frame their picture
    /// is of, which only a compositor reads: the viewer asks for it while
    /// the plan it draws has something to composite, and not for a plan of
    /// plain layers. The bytes stay alive for as long as the picture does,
    /// in main memory and outside the viewer budget, which counts texels
    /// once. The source monitor's layer keeps none.
    ///
    /// Frames taken before this was said were not kept, so a layer that has
    /// a picture and no frame asks for its frame again, once. Turned off,
    /// those kept are let go; a layer that had its frame alone, because a
    /// canvas showed it, then has no picture and asks again too.
    /// [`Self::settle`] gives such a layer a texture first, so the viewer
    /// turns this off after it.
    pub fn keep_frames(&mut self, keep: bool) {
        if self.keeps_frames == keep {
            return;
        }
        self.keeps_frames = keep;
        let source = self.viewing.get(&Viewing::Source).copied();
        for (consumer, layer) in &mut self.layers {
            if !keep {
                layer.frame = None;
                layer.on_canvas = false;
                if layer.texture.is_none() && layer.kept > 0 {
                    layer.kept = 0;
                    layer.requested = None;
                }
            } else if (layer.texture.is_some() || layer.plate) && Some(*consumer) != source {
                layer.requested = None;
            }
        }
    }

    /// Says which of the layers kept for clips a canvas showed in the frame
    /// being painted: `on_canvas` is asked of each. Such a layer is drawn
    /// from its decoded frame, which the compositor uploads, so the texture
    /// of its own is let go and the frames it takes from now on are not
    /// uploaded to one. Every other layer that has a frame and no texture is
    /// given one, made from that frame: a layer whose run is plain again or
    /// was left to the mesh path, one a cut handed to a clip that is drawn
    /// as a quad, one no longer drawn. Nothing is decoded for that.
    ///
    /// It is said once the canvases of a frame are composited and before
    /// its quads are painted. Returns whether a texture was made.
    pub fn settle(&mut self, ctx: &egui::Context, on_canvas: impl Fn(ClipId) -> bool) -> bool {
        let mut shown: HashMap<PreviewConsumer, bool> = HashMap::new();
        for (viewing, consumer) in &self.viewing {
            if let Viewing::Clip(clip) = viewing {
                shown.insert(*consumer, on_canvas(*clip));
            }
        }
        let frame = self.frame;
        let mut made = false;
        for (consumer, layer) in &mut self.layers {
            let on_canvas = layer.used == frame
                && layer.frame.is_some()
                && shown.get(consumer).copied().unwrap_or(false);
            layer.on_canvas = on_canvas;
            if on_canvas || layer.plate {
                layer.texture = None;
                layer.written = None;
            } else if layer.texture.is_none()
                && let Some((_, kept)) = &layer.frame
            {
                // Looked through for transparency whatever it is of: the
                // picture is the same either way.
                layer.texture = Some(ctx.load_texture(
                    format!("layer-{}", layer.asset),
                    image_of(kept, true),
                    TextureOptions::LINEAR,
                ));
                made = true;
            }
        }
        made
    }

    /// Whether a layer is kept for `viewing`.
    pub fn has_layer(&self, viewing: Viewing) -> bool {
        self.viewing.contains_key(&viewing)
    }

    /// Whether a frame is on its way to the layer kept for `viewing`: the
    /// service took what it asks and has not answered, or has had no room
    /// for it yet. Not once that frame could not be decoded, nor for a layer
    /// whose worker stopped or refused it outright, nor for one that waits
    /// for room in the viewer budget and so asks for nothing.
    pub(crate) fn awaits(&self, viewing: Viewing) -> bool {
        self.layer_of(viewing).is_some_and(|layer| {
            layer.granted.is_some()
                && layer.failed.is_none()
                && (layer.pending || layer.waiting.is_some())
        })
    }

    /// Whether the frame the layer kept for `viewing` asks for could not be
    /// decoded. It stays so until the layer asks for another frame, at
    /// another time or box, or the worker starts over.
    pub(crate) fn undecodable(&self, viewing: Viewing) -> bool {
        self.layer_of(viewing)
            .is_some_and(|layer| layer.failed.is_some())
    }

    fn layer_of(&self, viewing: Viewing) -> Option<&Layer> {
        self.layers.get(self.viewing.get(&viewing)?)
    }

    /// Gives the layer kept for `from`, with its picture and its consumer, to
    /// `to`, which carries on from it. Nothing is given to something that has
    /// a layer of its own, and nothing is taken from a layer drawn this frame.
    /// Returns whether the layer changed hands.
    pub fn hand_over(&mut self, from: Viewing, to: Viewing) -> bool {
        if self.viewing.contains_key(&to) {
            return false;
        }
        let Some(consumer) = self.viewing.get(&from).copied() else {
            return false;
        };
        if self
            .layers
            .get(&consumer)
            .is_none_or(|layer| layer.used == self.frame)
        {
            return false;
        }
        self.viewing.remove(&from);
        self.viewing.insert(to, consumer);
        // What is still being decoded for it, or published and not yet
        // taken, is of the clip it came from.
        if let Some(layer) = self.layers.get_mut(&consumer) {
            layer.jump();
        }
        true
    }

    /// Something other than a clock moved a playhead, or changed what is
    /// shown at it: a seek, a step, a jump, a loop starting over, an edit.
    /// What each layer asks next takes the place of what is still being
    /// decoded for it, instead of waiting behind it as the next frame of
    /// playback does. Only the application knows the one from the other; a
    /// clock that advances says nothing here.
    ///
    /// It takes effect at once, and not when the layers are next composed:
    /// the application takes what the worker published before it composes,
    /// and a frame asked for before the playhead moved is not taken after.
    /// Each layer keeps its picture meanwhile. See [`Layer::jump`].
    pub fn jumped(&mut self) {
        for layer in self.layers.values_mut() {
            layer.jump();
        }
    }

    /// What `viewing` is drawn for starts over at another time, by something
    /// that is no seek of a playhead: the source monitor opened again on the
    /// asset it shows. The layer kept for it jumps as in [`Self::jumped`],
    /// at once and alone: it keeps its picture, and no other layer's playhead
    /// moved. A layer of another asset is given up when it is next composed,
    /// and takes nothing more until then. Nothing kept for `viewing` is
    /// nothing to tell.
    pub fn started_over(&mut self, viewing: Viewing) {
        if let Some(layer) = self
            .viewing
            .get(&viewing)
            .and_then(|consumer| self.layers.get_mut(consumer))
        {
            layer.jump();
        }
    }

    /// Says whether the frames taken from now on are taken at the end of a
    /// frame, after it was composed and before it is painted, and not at the
    /// start of the next. Those are painted with what was taken before the
    /// frame was composed: for [`Self::accept`] the two are one painted frame,
    /// in which a layer is left one upload however many frames it took.
    pub fn take_late(&mut self, late: bool) {
        self.late = late;
    }

    /// Whether a layer drawn in the frame last composed asked for a frame the
    /// media service had no room for, and still waits to ask again. The
    /// service reports what it decodes and not when it has room, so while a
    /// clock plays the application looks in again by itself.
    pub fn refused(&self) -> bool {
        self.layers
            .values()
            .any(|layer| layer.used == self.frame && layer.waiting.is_some())
    }

    /// Tells the media service which consumers are no longer drawn for, so it
    /// keeps nothing queued, decoded or open for them. Without a service
    /// nothing was asked under them. It no longer holds one of them among the
    /// decoders to keep open either.
    pub fn retire(&mut self, media: Option<&MediaService>) {
        for consumer in self.retired.drain(..) {
            if let Some(media) = media {
                media.retire_preview(consumer);
            }
            for told in &mut self.retention_told {
                if *told == Some(consumer) {
                    *told = None;
                }
            }
        }
    }

    /// Gives up the layers `gone` holds of, and what they were kept for.
    fn release_layers(&mut self, gone: impl Fn(&Layer) -> bool) {
        let retired = &mut self.retired;
        self.layers.retain(|consumer, layer| {
            let gone = gone(layer);
            if gone {
                retired.push(*consumer);
            }
            !gone
        });
        let layers = &self.layers;
        self.viewing
            .retain(|_, consumer| layers.contains_key(consumer));
    }

    #[cfg(test)]
    pub(crate) fn with_viewer_budget(viewer_budget: ViewerBudget) -> Self {
        Self {
            viewer_budget,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn viewer_budget(&self) -> ViewerBudget {
        self.viewer_budget
    }

    /// How many layers are kept, with or without a picture, or how many
    /// things one is kept for, whichever is more.
    #[cfg(test)]
    pub(crate) fn layers_kept(&self) -> usize {
        self.layers.len().max(self.viewing.len())
    }

    /// Measured from the handles of the layers kept now, as
    /// [`layer_textures`] measures from the context.
    #[cfg(test)]
    pub(crate) fn layer_usage(&self) -> LayerUsage {
        let pictures = || {
            self.layers
                .values()
                .filter_map(|layer| layer.texture.as_ref())
        };
        LayerUsage {
            pictures: pictures().count(),
            bytes: pictures()
                .map(|texture| {
                    let [width, height] = texture.size();
                    width * height * 4
                })
                .sum(),
        }
    }

    /// The consumers whose decoders the frame last composed chose to have
    /// kept open, each in its place.
    #[cfg(test)]
    pub(crate) fn preview_retention(&self) -> [Option<PreviewConsumer>; 3] {
        self.retention
    }

    /// The consumer and the box the layer kept for `viewing` asks at. Nothing
    /// for a layer that waits for room, which asks at none.
    #[cfg(test)]
    pub(crate) fn asked(&self, viewing: Viewing) -> Option<(PreviewConsumer, [u32; 2])> {
        let consumer = *self.viewing.get(&viewing)?;
        Some((consumer, self.layers.get(&consumer)?.granted?))
    }

    /// The source time a frame published for `consumer` is taken at: the one
    /// the media service last took a request at for its layer. A layer
    /// composed without a service was asked of none, and is admitted here at
    /// zero as if by one. Zero too for a consumer no layer has, whose frames
    /// are taken at no time.
    #[cfg(test)]
    pub(crate) fn admitted(&mut self, consumer: PreviewConsumer) -> Time {
        self.layers.get_mut(&consumer).map_or(Time::ZERO, |layer| {
            *layer.admitted.get_or_insert(Time::ZERO)
        })
    }

    /// Takes a decoded frame. Returns whether anything on screen changed.
    ///
    /// A frame asked for by no consumer is a poster. One asked for by a
    /// consumer is taken only by the layer that consumer still has, and only
    /// while it is of the asset and at the size that layer asks for now:
    /// whatever was decoded for a layer since given up, for another asset or
    /// for a size since changed is dropped. So is one of another source time
    /// than the request the media service last took for the layer: it was
    /// asked for before a seek, and published before the request that took
    /// its place. A layer that jumped takes none at all until the service
    /// takes what it asks next, however long it has no room for that, or
    /// until it is asked for the very frame it had asked for before. The
    /// one exception is the frame the clock asked for just before, which
    /// nothing took the place of: published while its next was asked for, it
    /// is taken once, and not after that next, nor after a jump. And so is
    /// one with more texels than the box it was asked at, which is all the
    /// layer was given room for.
    ///
    /// A layer a canvas shows keeps the frame itself and no texture: see
    /// [`Self::settle`]. Otherwise a frame with the texels of the picture
    /// it replaces is written into the texture the layer has, which is then neither allocated nor bound
    /// again; one of another size takes the place of the whole texture. What
    /// is written into a texture waits behind what was to be uploaded to it
    /// before, where a whole texture takes the place of that. So only the
    /// first frame a layer takes after a frame was painted is written into
    /// its texture: while frames arrive and none is painted, as when the
    /// window is hidden, each takes the place of the one before and one
    /// upload waits, not one for every frame. A frame taken at the end of a
    /// frame, once it was composed, is painted with those taken before it
    /// was: see [`Self::take_late`].
    #[expect(
        clippy::too_many_arguments,
        reason = "what one published frame carries, handed over as it is"
    )]
    pub fn accept(
        &mut self,
        ctx: &egui::Context,
        asset: Option<&Asset>,
        consumer: Option<PreviewConsumer>,
        asset_id: AssetId,
        time: Time,
        size: [u32; 2],
        frame: &Frame,
    ) -> bool {
        let opaque = asset.is_some_and(|asset| asset.kind == MediaKind::Video);
        let Some(consumer) = consumer else {
            if size != POSTER {
                return false;
            }
            self.keep_poster(ctx, asset_id, image_of(frame, opaque));
            return true;
        };
        // The painted frame this one is uploaded for, as the count of frames
        // begun when what is taken before it is composed is taken: one less
        // by the end of it.
        let now = self.frame.saturating_sub(u64::from(self.late));
        let Some(layer) = self.layers.get_mut(&consumer) else {
            return false;
        };
        if layer.asset != asset_id || layer.granted != Some(size) {
            return false;
        }
        if layer.admitted != Some(time) && layer.before != Some(time) {
            // The frame of the request a layer jumped away from: it is not
            // coming again, so a layer still to show it asks once more.
            if layer.left == Some(time) {
                layer.left = None;
                layer.requested = None;
            }
            return false;
        }
        // Whichever of the two this is, the earlier one is not taken after it.
        layer.before = None;
        if layer.admitted == Some(time) {
            layer.pending = false;
        }
        let bytes = box_bytes([frame.width, frame.height]);
        if bytes > box_bytes(size) {
            // The frame this layer waited for did not come in a form it can
            // keep. The picture it has stays unless that was to shrink.
            return layer.forget_oversized();
        }
        layer.kept = bytes;
        self.taken += 1;
        // Only the layers of clips are composited.
        let keeps = self.keeps_frames && self.viewing.get(&Viewing::Source) != Some(&consumer);
        layer.frame = keeps.then(|| (self.taken, frame.clone()));
        layer.frame_at = time;
        if layer.plate {
            // Read by a compositor or by nothing. One that came after the
            // frames stopped being kept is asked for again when they are.
            if layer.frame.is_none() {
                layer.kept = 0;
                layer.requested = None;
            }
            return true;
        }
        if layer.on_canvas && layer.frame.is_some() {
            // A canvas shows this layer, and is drawn from the frame: it is
            // uploaded once, by the compositor, and to no texture here.
            layer.texture = None;
            layer.written = None;
            return true;
        }
        let image = image_of(frame, opaque);
        let first = layer.written.replace(now) != Some(now);
        match &mut layer.texture {
            Some(texture) if first && texture.size() == image.size => {
                texture.set_partial([0, 0], image, TextureOptions::LINEAR);
            }
            Some(texture) => texture.set(image, TextureOptions::LINEAR),
            None => {
                layer.texture = Some(ctx.load_texture(
                    format!("layer-{asset_id}"),
                    image,
                    TextureOptions::LINEAR,
                ));
            }
        }
        true
    }

    /// Takes a failure to decode. Returns whether it is of something still
    /// wanted, and so worth telling the person about.
    ///
    /// One that names no consumer is a poster's: it keeps its icon and is not
    /// asked for again, unless it was asked of a proxy. That proxy is not read
    /// again while it is the proxy, and the original is asked for instead, with
    /// nothing to tell. One that names a consumer is of the layer that
    /// consumer still has, while it is of the asset and at the size that layer
    /// asks for now. The time it had asked for is not asked for again at that
    /// box: it asks again at another time or another box, when the worker
    /// starts over, or as a new layer once its asset or project is another.
    /// It keeps the picture it has, unless that is larger than the box now
    /// asked at: the smaller frame was to take its place and free room other
    /// layers wait for, so that picture alone is given up. A failure for a
    /// layer since given up, or for what it no longer asks, changes nothing:
    /// no poster and no other layer of the asset failed.
    pub fn failed(
        &mut self,
        consumer: Option<PreviewConsumer>,
        asset: AssetId,
        size: Option<[u32; 2]>,
    ) -> bool {
        let Some(consumer) = consumer else {
            match self.poster_requested.remove(&asset) {
                Some(Some(proxy)) => {
                    self.preview.unreadable.insert(asset, proxy);
                    self.poster_refused = false;
                    return false;
                }
                Some(None) => {
                    self.poster_failed.insert(asset);
                    self.poster_refused = false;
                }
                None => {}
            }
            return true;
        };
        let Some(layer) = self.layers.get_mut(&consumer) else {
            return false;
        };
        if layer.asset != asset || size.is_some_and(|size| layer.granted != Some(size)) {
            return false;
        }
        if let Some(proxy) = &layer.proxy {
            // Its proxy could not be read: the original is, for as long as
            // that file is the proxy, and there is nothing to tell while it
            // can be.
            self.preview.unreadable.insert(asset, proxy.path.clone());
            self.read_from(consumer, None);
            return false;
        }
        layer.failed = layer.requested.take();
        layer.forget_oversized();
        true
    }

    /// The media worker dropped what it had queued: whatever was asked for
    /// and has not arrived is asked for again, and so is what could not be
    /// decoded. It waits on no layer's turn any more, and holds no decoders
    /// to keep open until it is told of them again.
    pub fn ask_again(&mut self) {
        self.poster_requested.clear();
        self.poster_refused = false;
        for layer in self.layers.values_mut() {
            layer.requested = None;
            layer.admitted = None;
            layer.before = None;
            layer.left = None;
            layer.failed = None;
            layer.waiting = None;
            layer.pending = false;
            layer.jumped = true;
        }
        self.retention_told = [None; 3];
    }

    pub fn forget_asset(&mut self, asset: AssetId) {
        self.drop_poster(asset);
        self.poster_requested.remove(&asset);
        self.poster_failed.remove(&asset);
        self.poster_refused = false;
        self.release_layers(|layer| layer.asset == asset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn asset(path: &str) -> Asset {
        let mut asset = Asset::new(
            PathBuf::from(path),
            MediaKind::Video,
            Time::from_seconds(6.0),
        );
        asset.width = 1920;
        asset.height = 1080;
        asset.file_size = 1000;
        asset
    }

    /// A small decoded picture, shaped like a poster.
    const SMALL: [u32; 2] = [16, 9];
    const SMALL_BYTES: usize = 16 * 9 * 4;

    fn decoded([width, height]: [u32; 2]) -> Frame {
        Frame {
            width,
            height,
            rgba: vec![255; (width * height * 4) as usize].into(),
        }
    }

    fn assets(count: usize) -> Vec<Asset> {
        (0..count)
            .map(|index| asset(&format!("/media/shot-{index}.mp4")))
            .collect()
    }

    /// A poster arriving the way the application hands it over.
    fn arrive(textures: &mut Textures, ctx: &egui::Context, asset: &Asset) {
        arrive_sized(textures, ctx, asset, SMALL);
    }

    fn arrive_sized(textures: &mut Textures, ctx: &egui::Context, asset: &Asset, size: [u32; 2]) {
        let frame = decoded(size);
        let changed = textures.accept(ctx, Some(asset), None, asset.id, Time::ZERO, POSTER, &frame);
        assert!(changed);
        // The running total is the texels of the textures actually kept.
        assert_eq!(textures.poster_bytes, textures.poster_usage().bytes);
    }

    /// One frame of a view of `assets`: each is asked for and then drawn, as
    /// the library and the timeline do. Returns those newly asked for.
    fn show<'a>(textures: &mut Textures, assets: &'a [Asset]) -> Vec<&'a Asset> {
        textures.begin_frame();
        assets
            .iter()
            .filter(|asset| {
                let asked = textures.claim_poster(asset).is_some();
                textures.poster(asset.id);
                asked
            })
            .collect()
    }

    fn kept(textures: &Textures, assets: &[Asset]) -> Vec<bool> {
        assets
            .iter()
            .map(|asset| textures.posters.contains_key(&asset.id))
            .collect()
    }

    /// Textures the context holds, whoever keeps their handles.
    fn allocated(ctx: &egui::Context) -> usize {
        ctx.tex_manager().read().num_allocated()
    }

    #[test]
    fn a_video_frame_with_alpha_is_uploaded_as_the_straight_alpha_it_is() {
        // What a decoder hands over is straight alpha, of a video as of a
        // still: half-clear red, beside an opaque texel.
        let frame = Frame {
            width: 2,
            height: 1,
            rgba: vec![220, 40, 40, 128, 220, 40, 40, 255].into(),
        };
        let image = image_of(&frame, true);
        assert_eq!(
            image.pixels[0],
            egui::Color32::from_rgba_unmultiplied(220, 40, 40, 128),
            "multiplied by its alpha, as the compositor reads it and the renderer wants it"
        );
        assert_eq!(image.pixels[1], egui::Color32::from_rgb(220, 40, 40));
        assert_eq!(image.pixels, image_of(&frame, false).pixels);

        // An opaque frame is the bytes it came as, whichever way it is read.
        let opaque = decoded([4, 2]);
        assert_eq!(
            image_of(&opaque, true).pixels,
            image_of(&opaque, false).pixels
        );
    }

    #[test]
    fn retained_posters_stay_within_the_production_count() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let mut textures = Textures::default();
        let budget = textures.poster_budget;
        assert_eq!(budget, PosterBudget::PRODUCTION);
        for asset in &assets(budget.posters + 1) {
            arrive(&mut textures, &ctx, asset);
        }
        let usage = textures.poster_usage();
        assert!(
            usage.posters <= budget.posters,
            "{} posters are kept where {} are allowed",
            usage.posters,
            budget.posters
        );
        assert!(
            allocated(&ctx) - before <= budget.posters,
            "{} poster textures stay allocated where {} are allowed",
            allocated(&ctx) - before,
            budget.posters
        );
    }

    #[test]
    fn retained_poster_bytes_stay_within_a_small_budget() {
        let ctx = egui::Context::default();
        // Room for many posters by count, for three of these by size.
        let budget = PosterBudget {
            posters: 64,
            bytes: 3 * SMALL_BYTES,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(4);
        for asset in &assets {
            arrive(&mut textures, &ctx, asset);
        }
        let usage = textures.poster_usage();
        assert!(
            usage.bytes <= budget.bytes,
            "{} bytes of posters are kept where {} are allowed",
            usage.bytes,
            budget.bytes
        );
        // The one that arrived last is the one being looked at.
        assert!(textures.poster(assets[3].id).is_some());
    }

    #[test]
    fn a_full_budget_gives_up_the_poster_shown_longest_ago() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let budget = PosterBudget {
            posters: 2,
            bytes: PosterBudget::PRODUCTION.bytes,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(3);
        arrive(&mut textures, &ctx, &assets[0]);
        textures.begin_frame();
        arrive(&mut textures, &ctx, &assets[1]);
        textures.begin_frame();
        // The first is on screen again; the second is not.
        assert!(textures.poster(assets[0].id).is_some());
        arrive(&mut textures, &ctx, &assets[2]);

        assert!(textures.poster(assets[0].id).is_some());
        assert!(textures.poster(assets[2].id).is_some());
        assert!(
            textures.poster(assets[1].id).is_none(),
            "the poster shown longest ago is still kept past the budget"
        );
        assert_eq!(allocated(&ctx) - before, budget.posters);
        // Only what was given up is in view now. The others were drawn a
        // frame ago and hold their room for this one, with a frame asked
        // for; in the next it is asked for, and kept when it arrives.
        let revisited = std::slice::from_ref(&assets[1]);
        assert!(show(&mut textures, revisited).is_empty());
        assert!(textures.poster_room_is_due());
        assert_eq!(show(&mut textures, revisited).len(), 1);
        arrive(&mut textures, &ctx, &assets[1]);
        assert!(textures.poster(assets[1].id).is_some());
        // Of the two it left behind, the one shown first made way.
        assert_eq!(kept(&textures, &assets), [false, true, true]);
        assert_eq!(allocated(&ctx) - before, budget.posters);
    }

    #[test]
    fn the_budget_outlasts_another_project() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let budget = PosterBudget {
            posters: 2,
            bytes: PosterBudget::PRODUCTION.bytes,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(5);
        for asset in &assets[..2] {
            arrive(&mut textures, &ctx, asset);
        }
        textures.clear();
        assert_eq!(textures.poster_budget, budget);
        assert_eq!(allocated(&ctx), before);
        for asset in &assets[2..] {
            arrive(&mut textures, &ctx, asset);
        }
        assert_eq!(textures.poster_usage().posters, 2);
        assert_eq!(allocated(&ctx) - before, 2);
    }

    #[test]
    fn a_newer_picture_of_a_kept_asset_is_held_to_the_budget() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        // Room for three small posters, or for one small and one large.
        let budget = PosterBudget {
            posters: 64,
            bytes: 3 * SMALL_BYTES,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(3);
        for asset in &assets {
            arrive(&mut textures, &ctx, asset);
        }
        // The first now decodes to twice the texels.
        arrive_sized(&mut textures, &ctx, &assets[0], [32, 9]);
        let usage = textures.poster_usage();
        assert!(
            usage.bytes <= budget.bytes,
            "{} bytes of posters are kept where {} are allowed",
            usage.bytes,
            budget.bytes
        );
        let poster = textures.poster(assets[0].id).expect("the newer picture");
        assert_eq!(poster.size(), [32, 9]);
        // The one shown longest ago made way, not the one after it.
        assert_eq!(kept(&textures, &assets), [true, false, true]);
        assert_eq!(allocated(&ctx) - before, 2);
    }

    #[test]
    fn sound_a_lookup_table_and_a_font_keep_their_icon_and_ask_for_no_poster() {
        let mut textures = Textures::default();
        for kind in [MediaKind::Audio, MediaKind::Lut, MediaKind::Font] {
            let mut asset = assets(1).remove(0);
            asset.kind = kind;
            assert_eq!(textures.claim_poster(&asset), None, "{kind:?}");
        }
        for kind in [MediaKind::Video, MediaKind::Image, MediaKind::Gif] {
            let mut asset = assets(1).remove(0);
            asset.kind = kind;
            assert!(textures.claim_poster(&asset).is_some(), "{kind:?}");
        }
    }

    #[test]
    fn a_poster_no_budget_could_hold_is_not_kept_or_asked_for_again() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let budget = PosterBudget {
            posters: 4,
            bytes: 2 * SMALL_BYTES,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(3);
        arrive(&mut textures, &ctx, &assets[0]);
        assert!(textures.claim_poster(&assets[1]).is_some());
        arrive_sized(&mut textures, &ctx, &assets[1], [32, 18]);
        assert!(textures.poster(assets[1].id).is_none());
        // Nothing was given up for a picture that could not stay.
        assert!(textures.poster(assets[0].id).is_some());
        assert_eq!(allocated(&ctx) - before, 1);
        assert_eq!(textures.claim_poster(&assets[1]), None);
        textures.ask_again();
        assert_eq!(textures.claim_poster(&assets[1]), None);

        // A kept poster whose newer picture is too large goes the same way.
        arrive_sized(&mut textures, &ctx, &assets[0], [32, 18]);
        assert_eq!(textures.poster_usage().posters, 0);
        assert_eq!(allocated(&ctx), before);
        assert_eq!(textures.claim_poster(&assets[0]), None);
        // Others are kept as before, and a changed source starts over.
        arrive(&mut textures, &ctx, &assets[2]);
        assert!(textures.poster(assets[2].id).is_some());
        textures.forget_asset(assets[1].id);
        assert!(textures.claim_poster(&assets[1]).is_some());
    }

    #[test]
    fn a_poster_that_arrives_late_is_held_to_the_budget() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let budget = PosterBudget {
            posters: 1,
            bytes: PosterBudget::PRODUCTION.bytes,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(2);
        assert!(textures.claim_poster(&assets[0]).is_some());
        // The worker drops its queue, and something else is wanted since.
        textures.ask_again();
        assert!(textures.claim_poster(&assets[1]).is_some());
        // The dropped request was already decoding and arrives all the same.
        arrive(&mut textures, &ctx, &assets[0]);
        arrive(&mut textures, &ctx, &assets[1]);
        assert_eq!(kept(&textures, &assets), [false, true]);
        assert_eq!(allocated(&ctx) - before, 1);
    }

    #[test]
    fn a_claim_the_worker_refused_is_made_again() {
        let budget = PosterBudget {
            posters: 1,
            bytes: PosterBudget::PRODUCTION.bytes,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(2);
        textures.begin_frame();
        assert!(textures.claim_poster(&assets[0]).is_some());
        // The one claim there is room for is out.
        assert_eq!(textures.claim_poster(&assets[1]), None);
        textures.unclaim_poster(assets[0].id);
        assert!(textures.claim_poster(&assets[1]).is_some());
        // The refused one waits its turn, and gets it on a later frame.
        assert_eq!(textures.claim_poster(&assets[0]), None);
        textures.unclaim_poster(assets[1].id);
        textures.begin_frame();
        assert!(textures.claim_poster(&assets[0]).is_some());
    }

    #[test]
    fn a_view_of_more_posters_than_the_budget_settles() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let budget = PosterBudget {
            posters: 3,
            bytes: PosterBudget::PRODUCTION.bytes,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(5);
        let asked = show(&mut textures, &assets);
        assert_eq!(asked.len(), budget.posters);
        assert!(!textures.poster_room_is_due());
        for asset in asked {
            arrive(&mut textures, &ctx, asset);
        }
        let settled = kept(&textures, &assets);
        assert_eq!(settled, [true, true, true, false, false]);
        // The same view, frame after frame: nothing more is decoded, nothing
        // is given up and no further frame is asked for.
        for _ in 0..8 {
            assert!(show(&mut textures, &assets).is_empty());
            assert!(!textures.poster_room_is_due());
            assert_eq!(kept(&textures, &assets), settled);
        }
        assert_eq!(allocated(&ctx) - before, budget.posters);
    }

    #[test]
    fn a_view_of_more_bytes_than_the_budget_settles() {
        let ctx = egui::Context::default();
        // Room for all of them by count, for three by size.
        let budget = PosterBudget {
            posters: 64,
            bytes: 3 * SMALL_BYTES,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(5);
        // Nothing has been decoded yet, so their size is not known.
        for asset in show(&mut textures, &assets) {
            arrive(&mut textures, &ctx, asset);
        }
        assert!(textures.poster_usage().bytes <= budget.bytes);
        let settled = kept(&textures, &assets);
        assert_eq!(settled.iter().filter(|kept| **kept).count(), 3);
        for _ in 0..8 {
            assert!(show(&mut textures, &assets).is_empty());
            assert!(!textures.poster_room_is_due());
            assert_eq!(kept(&textures, &assets), settled);
        }
    }

    #[test]
    fn room_comes_back_a_frame_after_posters_leave_the_screen() {
        let ctx = egui::Context::default();
        let budget = PosterBudget {
            posters: 2,
            bytes: PosterBudget::PRODUCTION.bytes,
        };
        let mut textures = Textures::with_poster_budget(budget);
        let assets = assets(4);
        let (first, second) = assets.split_at(2);
        for asset in show(&mut textures, first) {
            arrive(&mut textures, &ctx, asset);
        }
        assert!(show(&mut textures, first).is_empty());

        // The view moves on. What it left was drawn a frame ago, so one more
        // frame is asked for, and that frame has the room.
        assert!(show(&mut textures, second).is_empty());
        assert!(textures.poster_room_is_due());
        let asked = show(&mut textures, second);
        assert_eq!(asked.len(), 2);
        assert!(!textures.poster_room_is_due());
        for asset in asked {
            arrive(&mut textures, &ctx, asset);
        }
        assert_eq!(kept(&textures, &assets), [false, false, true, true]);

        // And back again: what was given up is asked for and kept anew.
        assert!(show(&mut textures, second).is_empty());
        assert!(show(&mut textures, first).is_empty());
        assert!(textures.poster_room_is_due());
        for asset in show(&mut textures, first) {
            arrive(&mut textures, &ctx, asset);
        }
        assert_eq!(kept(&textures, &assets), [true, true, false, false]);
        assert!(show(&mut textures, first).is_empty());
        assert!(!textures.poster_room_is_due());
    }

    #[test]
    fn a_poster_within_the_budget_is_kept_and_measured() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let mut textures = Textures::default();
        let assets = assets(3);
        for asset in &assets {
            arrive(&mut textures, &ctx, asset);
        }
        // A newer picture of the same asset takes the place of the old one.
        arrive(&mut textures, &ctx, &assets[0]);
        assert_eq!(
            textures.poster_usage(),
            PosterUsage {
                posters: 3,
                bytes: 3 * SMALL_BYTES,
            }
        );
        assert_eq!(allocated(&ctx) - before, 3);
        for asset in &assets {
            let poster = textures.poster(asset.id).expect("kept");
            assert_eq!(poster.size(), [16, 9]);
        }
    }

    #[test]
    fn a_poster_is_asked_for_once_until_it_arrives() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        assert!(textures.claim_poster(&asset).is_some());
        assert_eq!(textures.claim_poster(&asset), None);
        arrive(&mut textures, &ctx, &asset);
        assert_eq!(textures.claim_poster(&asset), None);

        let path = PathBuf::from("/media/voice.wav");
        let sound = Asset::new(path, MediaKind::Audio, Time::from_seconds(6.0));
        assert_eq!(textures.claim_poster(&sound), None);
    }

    #[test]
    fn a_failed_poster_keeps_its_icon_and_is_not_asked_for_again() {
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        assert!(textures.claim_poster(&asset).is_some());
        assert!(textures.failed(None, asset.id, Some(POSTER)));
        assert!(textures.poster(asset.id).is_none());
        assert_eq!(textures.claim_poster(&asset), None);
        // Not even when the worker drops its queue.
        textures.ask_again();
        assert_eq!(textures.claim_poster(&asset), None);
    }

    #[test]
    fn a_dropped_request_is_asked_for_again_and_a_kept_poster_is_not() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let assets = assets(2);
        assert!(textures.claim_poster(&assets[0]).is_some());
        assert!(textures.claim_poster(&assets[1]).is_some());
        arrive(&mut textures, &ctx, &assets[1]);
        textures.ask_again();
        assert!(textures.claim_poster(&assets[0]).is_some());
        assert_eq!(textures.claim_poster(&assets[1]), None);
    }

    #[test]
    fn a_forgotten_asset_releases_its_poster_and_is_asked_for_again() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let mut textures = Textures::default();
        let assets = assets(2);
        for asset in &assets {
            assert!(textures.claim_poster(asset).is_some());
            arrive(&mut textures, &ctx, asset);
        }
        // One decoded and one failed, then both sources are replaced.
        let failing = asset("/media/broken.mp4");
        assert!(textures.claim_poster(&failing).is_some());
        textures.failed(None, failing.id, Some(POSTER));

        textures.forget_asset(assets[0].id);
        textures.forget_asset(failing.id);
        assert!(textures.poster(assets[0].id).is_none());
        assert_eq!(textures.poster_usage().posters, 1);
        assert_eq!(allocated(&ctx) - before, 1);
        assert!(textures.claim_poster(&assets[0]).is_some());
        assert!(textures.claim_poster(&failing).is_some());
        assert_eq!(textures.claim_poster(&assets[1]), None);

        // Another project: nothing of this one is kept.
        textures.clear();
        assert_eq!(textures.poster_usage().posters, 0);
        assert_eq!(allocated(&ctx), before);
        assert!(textures.claim_poster(&assets[1]).is_some());
    }

    const BOX: [u32; 2] = [640, 640];

    /// A frame decoded for `viewing`, addressed the way it was asked for.
    fn arrive_for(
        textures: &mut Textures,
        ctx: &egui::Context,
        viewing: Viewing,
        asset: &Asset,
    ) -> bool {
        let (consumer, size) = textures.asked(viewing).expect("a layer is kept");
        arrive_under(textures, ctx, consumer, asset, size)
    }

    /// A frame that names `consumer`, of `asset` at `size`.
    fn arrive_under(
        textures: &mut Textures,
        ctx: &egui::Context,
        consumer: PreviewConsumer,
        asset: &Asset,
        size: [u32; 2],
    ) -> bool {
        let frame = decoded(SMALL);
        let time = textures.admitted(consumer);
        textures.accept(
            ctx,
            Some(asset),
            Some(consumer),
            asset.id,
            time,
            size,
            &frame,
        )
    }

    fn clips<const N: usize>() -> [Viewing; N] {
        [(); N].map(|()| Viewing::Clip(ClipId::new_v4()))
    }

    #[test]
    fn layers_of_one_asset_at_one_size_are_each_their_own() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let clips: [Viewing; 2] = clips();
        let all = [clips[0], clips[1], Viewing::Source];
        textures.begin_frame();
        for viewing in all {
            let time = Time::from_seconds(1.0);
            assert!(textures.layer(None, viewing, &asset, BOX, time).is_none());
        }
        let asked = all.map(|viewing| textures.asked(viewing).expect("kept"));
        assert!(asked.iter().all(|(_, size)| *size == BOX));
        let consumers: HashSet<_> = asked.iter().map(|(consumer, _)| *consumer).collect();
        assert_eq!(consumers.len(), 3);

        // A frame for one is shown by that one alone.
        assert!(arrive_for(&mut textures, &ctx, clips[1], &asset));
        let shown = all.map(|viewing| {
            textures
                .layer(None, viewing, &asset, BOX, Time::ZERO)
                .is_some()
        });
        assert_eq!(shown, [false, true, false]);
        // And one that names no consumer is no layer's.
        let unnamed = decoded(SMALL);
        assert!(!textures.accept(
            &ctx,
            Some(&asset),
            None,
            asset.id,
            Time::ZERO,
            BOX,
            &unnamed
        ));
    }

    #[test]
    fn a_layer_keeps_its_consumer_and_its_picture_at_another_size() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let clip = Viewing::Clip(ClipId::new_v4());
        textures.begin_frame();
        textures.layer(None, clip, &asset, BOX, Time::ZERO);
        let (consumer, _) = textures.asked(clip).expect("kept");
        assert!(arrive_for(&mut textures, &ctx, clip, &asset));

        // The window grew: the same consumer asks at the larger size, and
        // the picture it has stands until that arrives.
        let larger = [1280, 1280];
        let held = textures.layer(None, clip, &asset, larger, Time::ZERO);
        assert!(held.is_some());
        assert_eq!(textures.asked(clip), Some((consumer, larger)));
        // What was still decoding at the old size is not taken.
        assert!(!arrive_under(&mut textures, &ctx, consumer, &asset, BOX));
        assert!(arrive_under(&mut textures, &ctx, consumer, &asset, larger));
    }

    #[test]
    fn a_layer_turned_to_another_asset_starts_over() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let mut textures = Textures::default();
        let assets = assets(2);
        textures.begin_frame();
        textures.layer(None, Viewing::Source, &assets[0], BOX, Time::ZERO);
        let (first, _) = textures.asked(Viewing::Source).expect("kept");
        assert!(arrive_for(&mut textures, &ctx, Viewing::Source, &assets[0]));

        // The monitor turns to another asset: nothing of the first is shown.
        let shown = textures.layer(None, Viewing::Source, &assets[1], BOX, Time::ZERO);
        assert!(shown.is_none());
        assert_eq!(allocated(&ctx), before);
        let (second, _) = textures.asked(Viewing::Source).expect("kept");
        assert_ne!(first, second);
        // A frame of the first that was on its way has nowhere to arrive,
        // under its own consumer or under the one that took its place.
        assert!(!arrive_under(&mut textures, &ctx, first, &assets[0], BOX));
        assert!(!arrive_under(&mut textures, &ctx, second, &assets[0], BOX));
        assert!(arrive_for(&mut textures, &ctx, Viewing::Source, &assets[1]));
    }

    #[test]
    fn a_failure_is_of_the_layer_that_asked_and_of_nothing_else() {
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let [first, second] = clips();
        let time = Time::from_seconds(1.0);
        textures.begin_frame();
        assert!(textures.claim_poster(&asset).is_some());
        for clip in [first, second] {
            textures.layer(None, clip, &asset, BOX, time);
        }
        let asked = [first, second].map(|clip| textures.asked(clip).expect("kept").0);
        // As if both frames had been asked for of a service.
        for consumer in asked {
            let layer = textures.layers.get_mut(&consumer).expect("kept");
            layer.requested = Some(time);
        }
        let requested =
            |textures: &Textures| asked.map(|consumer| textures.layers[&consumer].requested);

        // The first clip's frame could not be decoded: it may ask again. The
        // other clip of the asset still waits for its own, and so does the
        // poster.
        assert!(textures.failed(Some(asked[0]), asset.id, Some(BOX)));
        assert_eq!(requested(&textures), [None, Some(time)]);
        assert!(textures.poster_requested.contains_key(&asset.id));
        assert!(!textures.poster_failed.contains(&asset.id));

        // Failures that are of nothing current: at a size no longer asked,
        // of another asset, and for a layer given up.
        let other = assets(1).remove(0);
        assert!(!textures.failed(Some(asked[1]), asset.id, Some(POSTER)));
        assert!(!textures.failed(Some(asked[1]), other.id, Some(BOX)));
        assert_eq!(requested(&textures), [None, Some(time)]);
        textures.forget_asset(asset.id);
        assert!(!textures.failed(Some(asked[1]), asset.id, Some(BOX)));
        assert!(!textures.failed(Some(PreviewConsumer::new()), asset.id, None));
    }

    #[test]
    fn a_layer_awaits_its_frame_until_it_comes_or_cannot_be_decoded() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let clip = Viewing::Clip(ClipId::new_v4());
        let time = Time::from_seconds(1.0);
        textures.begin_frame();
        // Nothing kept, and a layer asked of no service: nothing comes.
        assert!(!textures.awaits(clip) && !textures.undecodable(clip));
        textures.layer(None, clip, &asset, BOX, time);
        let (consumer, size) = textures.asked(clip).expect("kept");
        assert!(!textures.awaits(clip));
        // As if the service had taken the request.
        let took = |textures: &mut Textures| {
            let layer = textures.layers.get_mut(&consumer).expect("kept");
            layer.requested = Some(time);
            layer.admitted = Some(time);
            layer.pending = true;
        };
        took(&mut textures);
        assert!(textures.awaits(clip) && !textures.undecodable(clip));
        assert!(arrive_at(
            &mut textures,
            &ctx,
            consumer,
            &asset,
            size,
            time,
            SMALL
        ));
        assert!(!textures.awaits(clip));

        // It could not be decoded: it waits for nothing, and says so until
        // the worker starts over.
        took(&mut textures);
        assert!(textures.failed(Some(consumer), asset.id, Some(size)));
        assert!(!textures.awaits(clip) && textures.undecodable(clip));
        textures.ask_again();
        assert!(!textures.awaits(clip) && !textures.undecodable(clip));

        // One the service had no room for waits its turn; one with no box,
        // waiting for room in the budget, asks for nothing.
        textures.layers.get_mut(&consumer).expect("kept").waiting = Some(1);
        assert!(textures.awaits(clip));
        textures.layers.get_mut(&consumer).expect("kept").granted = None;
        assert!(!textures.awaits(clip));
    }

    #[test]
    fn a_layer_changes_hands_only_to_what_has_none_from_one_not_drawn() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let [outgoing, incoming, other] = clips();
        textures.begin_frame();
        textures.layer(None, outgoing, &asset, BOX, Time::ZERO);
        textures.layer(None, other, &asset, BOX, Time::ZERO);
        let (consumer, _) = textures.asked(outgoing).expect("kept");
        assert!(arrive_for(&mut textures, &ctx, outgoing, &asset));
        // Both were drawn this frame, and each has a layer.
        assert!(!textures.hand_over(outgoing, incoming));
        assert!(!textures.hand_over(outgoing, other));

        textures.begin_frame();
        textures.layer(None, other, &asset, BOX, Time::ZERO);
        // Not from one that is drawn, nor to one that has its own.
        assert!(!textures.hand_over(other, incoming));
        assert!(!textures.hand_over(outgoing, other));
        assert!(textures.hand_over(outgoing, incoming));
        assert!(!textures.has_layer(outgoing));
        assert_eq!(textures.asked(incoming), Some((consumer, BOX)));
        assert!(
            textures
                .layer(None, incoming, &asset, BOX, Time::ZERO)
                .is_some()
        );
        // There is nothing left to hand over twice.
        assert!(!textures.hand_over(outgoing, Viewing::Source));
    }

    #[test]
    fn layers_given_up_take_no_frames_and_are_retired() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let mut textures = Textures::default();
        let assets = assets(2);
        let clips: [Viewing; 2] = clips();
        textures.begin_frame();
        for (clip, asset) in clips.iter().zip(&assets) {
            textures.layer(None, *clip, asset, BOX, Time::ZERO);
            assert!(arrive_for(&mut textures, &ctx, *clip, asset));
        }
        let asked = clips.map(|clip| textures.asked(clip).expect("kept").0);

        // The first asset now decodes from another file.
        textures.forget_asset(assets[0].id);
        assert_eq!(textures.retired, [asked[0]]);
        assert!(!textures.has_layer(clips[0]));
        assert!(!arrive_under(
            &mut textures,
            &ctx,
            asked[0],
            &assets[0],
            BOX
        ));
        assert_eq!(allocated(&ctx) - before, 1);

        // The second goes undrawn until it is released.
        for _ in 0..=LAYER_IDLE_FRAMES {
            textures.begin_frame();
        }
        assert_eq!(textures.retired, asked);
        assert!(!textures.has_layer(clips[1]));
        assert!(!arrive_under(
            &mut textures,
            &ctx,
            asked[1],
            &assets[1],
            BOX
        ));
        assert_eq!(allocated(&ctx), before);
        // With no service nothing was asked under them, and the list is let go.
        textures.retire(None);
        assert!(textures.retired.is_empty());

        // Another project: its layers are retired too, and take nothing.
        textures.layer(None, clips[1], &assets[1], BOX, Time::ZERO);
        let (kept, _) = textures.asked(clips[1]).expect("kept");
        assert_ne!(kept, asked[1]);
        textures.clear();
        assert_eq!(textures.retired, [kept]);
        assert!(!arrive_under(&mut textures, &ctx, kept, &assets[1], BOX));
    }

    /// A frame of `texels` that names `consumer`, of `asset` at `size`,
    /// decoded at `time`.
    fn arrive_at(
        textures: &mut Textures,
        ctx: &egui::Context,
        consumer: PreviewConsumer,
        asset: &Asset,
        size: [u32; 2],
        time: Time,
        texels: [u32; 2],
    ) -> bool {
        let frame = decoded(texels);
        textures.accept(
            ctx,
            Some(asset),
            Some(consumer),
            asset.id,
            time,
            size,
            &frame,
        )
    }

    /// As if the media service had taken a request at `time` under
    /// `consumer`, in the place of whatever it had taken for it before.
    fn admit(textures: &mut Textures, consumer: PreviewConsumer, time: Time) {
        textures.layers.get_mut(&consumer).expect("kept").admitted = Some(time);
    }

    /// What `ctx` was handed to upload for `texture` since this was last
    /// read, in order: where each is written, nowhere for one that takes the
    /// place of the whole texture, and its texels.
    fn uploads(
        ctx: &egui::Context,
        texture: egui::TextureId,
    ) -> Vec<(Option<[usize; 2]>, [usize; 2])> {
        let mut delta = ctx.tex_manager().write().take_delta();
        let uploads: Vec<_> = delta
            .set
            .get(&texture)
            .map(|deltas| {
                deltas
                    .iter()
                    .map(|delta| (delta.pos, delta.image.size()))
                    .collect()
            })
            .unwrap_or_default();
        delta.clear();
        uploads
    }

    #[test]
    fn a_frame_of_a_time_asked_before_the_one_now_asked_is_not_taken() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let clip = Viewing::Clip(ClipId::new_v4());
        let (before, now) = (Time::from_seconds(1.0), Time::from_seconds(4.0));
        textures.begin_frame();
        textures.layer(None, clip, &asset, BOX, before);
        let (consumer, size) = textures.asked(clip).expect("kept");

        // The playhead was moved while the first frame was already decoded
        // and published: the service took the second request in its place.
        admit(&mut textures, consumer, before);
        admit(&mut textures, consumer, now);
        assert!(
            !arrive_at(&mut textures, &ctx, consumer, &asset, size, before, SMALL),
            "SEEK: a frame of the time asked before the seek was taken after it"
        );
        assert!(textures.picture(clip).is_none());

        // The frame of the time now asked is the layer's, and stays so when
        // one of the earlier time arrives after it.
        assert!(arrive_at(
            &mut textures,
            &ctx,
            consumer,
            &asset,
            size,
            now,
            SMALL
        ));
        let texture = textures.picture(clip).expect("a picture").id();
        assert_eq!(uploads(&ctx, texture), [(None, [16, 9])]);
        assert!(
            !arrive_at(&mut textures, &ctx, consumer, &asset, size, before, SMALL),
            "SEEK: a frame of the time asked before the seek took the place of the \
             frame sought"
        );
        assert!(
            uploads(&ctx, texture).is_empty(),
            "SEEK: a frame that was not taken was uploaded"
        );
    }

    #[test]
    fn a_frame_is_taken_only_at_the_time_the_service_took_a_request_at() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let clip = Viewing::Clip(ClipId::new_v4());
        let time = Time::from_seconds(2.0);
        textures.begin_frame();
        // Without a service nothing was asked for, at any time.
        textures.layer(None, clip, &asset, BOX, time);
        let (consumer, size) = textures.asked(clip).expect("kept");
        assert!(
            !arrive_at(&mut textures, &ctx, consumer, &asset, size, time, SMALL),
            "SEEK: a frame was taken for a layer the service took no request of"
        );

        admit(&mut textures, consumer, time);
        // One microsecond off is another frame.
        for other in [Time(time.0 - 1), Time(time.0 + 1), Time::ZERO] {
            assert!(
                !arrive_at(&mut textures, &ctx, consumer, &asset, size, other, SMALL),
                "SEEK: a frame at {other:?} was taken for a request at {time:?}"
            );
        }
        // The equalities it was taken by before still hold at the right time.
        let other = self::asset("/media/other.mp4");
        assert!(!arrive_at(
            &mut textures,
            &ctx,
            consumer,
            &other,
            size,
            time,
            SMALL
        ));
        let larger = [BOX[0] * 2, BOX[1] * 2];
        assert!(!arrive_at(
            &mut textures,
            &ctx,
            consumer,
            &asset,
            larger,
            time,
            SMALL
        ));
        let nobody = PreviewConsumer::new();
        assert!(!arrive_at(
            &mut textures,
            &ctx,
            nobody,
            &asset,
            size,
            time,
            SMALL
        ));
        assert!(textures.picture(clip).is_none());
        assert!(arrive_at(
            &mut textures,
            &ctx,
            consumer,
            &asset,
            size,
            time,
            SMALL
        ));

        // A failure names no time: what the service took last is still the
        // frame this layer takes, as it was before.
        assert!(textures.failed(Some(consumer), asset.id, Some(size)));
        assert!(arrive_at(
            &mut textures,
            &ctx,
            consumer,
            &asset,
            size,
            time,
            SMALL
        ));

        // The worker started over: what it had taken is no longer coming.
        textures.ask_again();
        assert!(
            !arrive_at(&mut textures, &ctx, consumer, &asset, size, time, SMALL),
            "SEEK: a frame asked for before the worker started over was taken after"
        );
    }

    #[test]
    fn the_frame_the_clock_asked_for_before_its_next_is_taken_once_and_not_after_it() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let clip = Viewing::Clip(ClipId::new_v4());
        let times = [0.0, 1.0, 2.0, 3.0].map(Time::from_seconds);
        textures.begin_frame();
        textures.layer(None, clip, &asset, BOX, times[0]);
        let (consumer, size) = textures.asked(clip).expect("kept");
        // As if the service had taken a request at `next` as the clock's
        // next frame, with the one before it decoded and published.
        let clocked = |textures: &mut Textures, next: Time| {
            let layer = textures.layers.get_mut(&consumer).expect("kept");
            layer.before = layer.admitted;
            layer.admitted = Some(next);
        };
        let arrive = |textures: &mut Textures, time: Time| {
            arrive_at(textures, &ctx, consumer, &asset, size, time, SMALL)
        };
        admit(&mut textures, consumer, times[0]);

        // Published and not yet taken when the clock asked for its next.
        clocked(&mut textures, times[1]);
        assert!(
            arrive(&mut textures, times[0]),
            "PLAYBACK: a frame the clock asked for, published while its next was \
             asked for, was not taken"
        );
        assert!(
            !arrive(&mut textures, times[0]),
            "PLAYBACK: a frame the clock asked for before its next was taken twice"
        );
        assert!(arrive(&mut textures, times[1]));

        // The next arrived first: the one before it is not shown after it.
        clocked(&mut textures, times[2]);
        assert!(arrive(&mut textures, times[2]));
        assert!(
            !arrive(&mut textures, times[1]),
            "PLAYBACK: a frame was taken after the one the clock asked for next"
        );

        // Two back is nobody's, and neither is the one before a seek.
        clocked(&mut textures, times[3]);
        assert!(!arrive(&mut textures, times[1]));
        admit(&mut textures, consumer, times[0]);
        textures.layers.get_mut(&consumer).expect("kept").before = None;
        for earlier in [times[2], times[3]] {
            assert!(
                !arrive(&mut textures, earlier),
                "SEEK: a frame the clock asked for before a seek was taken after it"
            );
        }
        assert!(arrive(&mut textures, times[0]));
    }

    #[test]
    fn a_jump_gives_up_what_was_asked_before_it_and_keeps_the_picture() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let [clip, next] = clips();
        let times = [0.0, 1.0, 2.0, 3.0].map(Time::from_seconds);
        textures.begin_frame();
        textures.layer(None, clip, &asset, BOX, times[0]);
        let (consumer, size) = textures.asked(clip).expect("kept");
        let arrive = |textures: &mut Textures, time: Time| {
            arrive_at(textures, &ctx, consumer, &asset, size, time, SMALL)
        };
        // As if the service had taken a request at `next` as the clock's
        // next frame, with the one before it decoded and published.
        let clocked = |textures: &mut Textures, next: Time| {
            let layer = textures.layers.get_mut(&consumer).expect("kept");
            layer.before = layer.admitted;
            layer.admitted = Some(next);
        };
        admit(&mut textures, consumer, times[0]);
        assert!(arrive(&mut textures, times[0]));
        let texture = textures.picture(clip).expect("a picture").id();
        assert_eq!(uploads(&ctx, texture), [(None, [16, 9])]);

        // Two frames of the clock are on their way when the playhead is
        // moved, and arrive before anything is asked at where it moved to.
        clocked(&mut textures, times[1]);
        clocked(&mut textures, times[2]);
        textures.jumped();
        for earlier in [times[1], times[2], times[0]] {
            assert!(
                !arrive(&mut textures, earlier),
                "SEEK: a frame asked for before a seek, at {earlier:?}, was taken after \
                 it and before the frame sought was asked for"
            );
        }
        assert_eq!(
            textures.picture(clip).map(|picture| picture.id()),
            Some(texture),
            "SEEK: a layer whose playhead was moved did not keep the picture it had"
        );
        assert!(
            uploads(&ctx, texture).is_empty(),
            "SEEK: a frame that was not taken was uploaded"
        );
        // The frame sought is the layer's when the service takes its request.
        admit(&mut textures, consumer, times[3]);
        assert!(arrive(&mut textures, times[3]));

        // A layer that changes hands at a cut gives up the same.
        clocked(&mut textures, times[1]);
        textures.begin_frame();
        assert!(textures.hand_over(clip, next));
        for earlier in [times[3], times[1]] {
            assert!(
                !arrive(&mut textures, earlier),
                "SEEK: a frame asked for by the clip before a cut, at {earlier:?}, was \
                 taken by the clip that carries on from it"
            );
        }
        assert_eq!(
            textures.picture(next).map(|picture| picture.id()),
            Some(texture)
        );
    }

    #[test]
    fn a_frame_the_size_of_the_picture_it_replaces_is_written_into_its_texture() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let clip = Viewing::Clip(ClipId::new_v4());
        textures.begin_frame();
        textures.layer(None, clip, &asset, BOX, Time::ZERO);
        assert!(arrive_for(&mut textures, &ctx, clip, &asset));
        let texture = textures.picture(clip).expect("a picture").id();
        assert_eq!(
            uploads(&ctx, texture),
            [(None, [16, 9])],
            "CONTROL: a layer's first frame was not uploaded once as a whole texture"
        );

        // Each later frame of playback, one to a painted frame.
        for _ in 0..3 {
            textures.begin_frame();
            assert!(arrive_for(&mut textures, &ctx, clip, &asset));
            assert_eq!(
                uploads(&ctx, texture),
                [(Some([0, 0]), [16, 9])],
                "UPLOAD: a frame the size of the picture it replaces was not written \
                 once into the texture kept, from its corner"
            );
        }
        assert_eq!(
            textures
                .picture(clip)
                .map(|picture| (picture.id(), picture.size())),
            Some((texture, [16, 9])),
            "UPLOAD: the layer does not show the texture it had, at its size"
        );
        assert_eq!(allocated(&ctx) - before, 1);
        assert_eq!(textures.layer_usage(), layer_textures(&ctx));
        assert_eq!(textures.layer_usage().bytes, SMALL_BYTES);
    }

    #[test]
    fn a_frame_of_another_size_takes_the_place_of_the_whole_texture() {
        let ctx = egui::Context::default();
        let before = allocated(&ctx);
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let clip = Viewing::Clip(ClipId::new_v4());
        textures.begin_frame();
        textures.layer(None, clip, &asset, BOX, Time::ZERO);
        let (consumer, size) = textures.asked(clip).expect("kept");
        let time = textures.admitted(consumer);
        assert!(arrive_at(
            &mut textures,
            &ctx,
            consumer,
            &asset,
            size,
            time,
            SMALL
        ));
        let texture = textures.picture(clip).expect("a picture").id();
        assert_eq!(uploads(&ctx, texture), [(None, [16, 9])]);

        // Larger, then smaller again: neither fits the texture kept.
        for texels in [[32, 18], SMALL] {
            textures.begin_frame();
            assert!(arrive_at(
                &mut textures,
                &ctx,
                consumer,
                &asset,
                size,
                time,
                texels
            ));
            let wanted = [texels[0] as usize, texels[1] as usize];
            assert_eq!(
                uploads(&ctx, texture),
                [(None, wanted)],
                "UPLOAD: a frame of another size than the picture it replaces was not \
                 uploaded once as a whole texture"
            );
            assert_eq!(
                textures
                    .picture(clip)
                    .map(|picture| (picture.id(), picture.size())),
                Some((texture, wanted))
            );
            assert_eq!(textures.layer_usage(), layer_textures(&ctx));
        }
        assert_eq!(allocated(&ctx) - before, 1);
    }

    #[test]
    fn frames_that_arrive_with_none_painted_between_leave_one_upload_waiting() {
        let ctx = egui::Context::default();
        let mut textures = Textures::default();
        let asset = asset("/media/shot.mp4");
        let clip = Viewing::Clip(ClipId::new_v4());
        textures.begin_frame();
        textures.layer(None, clip, &asset, BOX, Time::ZERO);
        assert!(arrive_for(&mut textures, &ctx, clip, &asset));
        let texture = textures.picture(clip).expect("a picture").id();
        // The window is hidden: frames go on arriving and none is painted.
        for _ in 0..8 {
            assert!(arrive_for(&mut textures, &ctx, clip, &asset));
        }
        assert_eq!(
            uploads(&ctx, texture),
            [(None, [16, 9])],
            "UPLOAD: frames taken with none painted between are all kept to be \
             uploaded, where the last takes the place of those before it"
        );

        // Painted again, and one more than a painted frame shows.
        textures.begin_frame();
        for _ in 0..3 {
            assert!(arrive_for(&mut textures, &ctx, clip, &asset));
        }
        assert_eq!(uploads(&ctx, texture), [(None, [16, 9])]);
    }

    #[test]
    fn a_name_or_a_folder_does_not_make_an_asset_stale() {
        let original = asset("/media/shot.mp4");
        let old = BTreeMap::from([(original.id, original.clone())]);
        let mut renamed = original.clone();
        renamed.name = "Take 2".to_owned();
        renamed.folder_id = Some(uuid::Uuid::new_v4());
        let new = BTreeMap::from([(renamed.id, renamed)]);
        assert_eq!(stale(&old, &new), Stale::default());
    }

    #[test]
    fn the_same_asset_at_another_file_is_stale() {
        let original = asset("/media/shot.mp4");
        let gone = asset("/media/gone.mp4");
        let added = asset("/media/new.mp4");
        let old = BTreeMap::from([(original.id, original.clone()), (gone.id, gone.clone())]);
        let changes: [fn(&mut Asset); 5] = [
            |a: &mut Asset| a.path = PathBuf::from("/media/other.mp4"),
            |a: &mut Asset| a.duration = Time::from_seconds(7.0),
            |a: &mut Asset| a.width = 1280,
            |a: &mut Asset| a.file_size = 999,
            |a: &mut Asset| a.kind = MediaKind::Gif,
        ];
        for change in changes {
            let mut moved = original.clone();
            change(&mut moved);
            let new = BTreeMap::from([(moved.id, moved), (added.id, added.clone())]);
            assert_eq!(
                stale(&old, &new),
                Stale {
                    changed: vec![original.id],
                    removed: vec![gone.id],
                }
            );
        }
    }
}

#[cfg(test)]
#[path = "textures_proxy_tests.rs"]
mod proxy_tests;
