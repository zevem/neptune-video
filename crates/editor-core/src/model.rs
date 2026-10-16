use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ops::{Add, Sub},
    path::PathBuf,
    sync::Arc,
};
use uuid::Uuid;

use crate::{
    AnimTarget, AnimTrack, BlendMode, Effect, Follow, Mask, ParamKey, Retime, Tracker, Transition,
    animate, look,
    title::{self, Element, TextAnimation, TextStyle, WordTime},
};

pub type AssetId = Uuid;
pub type TrackId = Uuid;
pub type ClipId = Uuid;
pub type FolderId = Uuid;
pub type MarkerId = Uuid;

/// Timeline time, in integer microseconds. Source and timeline clocks are distinct.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Time(pub i64);

impl Time {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(i64::MAX / 16);
    pub fn from_seconds(seconds: f64) -> Self {
        Self((seconds * 1_000_000.0).round() as i64)
    }
    pub fn as_seconds(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }
    pub fn scaled(self, rate: f64) -> Self {
        Self((self.0 as f64 * rate).round() as i64)
    }
    pub fn is_valid(self) -> bool {
        self >= Self::ZERO && self <= Self::MAX
    }
    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }
}
impl Add for Time {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }
}
impl Sub for Time {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}

/// A rational rate avoids drift at 24000/1001 and 30000/1001 fps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}
impl Default for FrameRate {
    fn default() -> Self {
        Self {
            numerator: 30,
            denominator: 1,
        }
    }
}
impl FrameRate {
    pub fn fps(self) -> f64 {
        self.numerator as f64 / self.denominator.max(1) as f64
    }
    pub fn frame_time(self, frame: i64) -> Time {
        if self.numerator == 0 {
            return Time::ZERO;
        }
        let value = frame as i128 * 1_000_000 * self.denominator as i128 / self.numerator as i128;
        Time(value.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
    }
    pub fn frame_at(self, time: Time) -> i64 {
        // frame_time floors the rational boundary. Adding one microsecond before
        // inversion makes those representable boundaries round-trip exactly.
        let value = ((time.0 as i128 + 1) * self.numerator as i128 - 1)
            / (1_000_000 * self.denominator.max(1) as i128);
        value.clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }
    pub fn validate(self) -> bool {
        self.numerator > 0 && self.denominator > 0 && (1.0..=240.0).contains(&self.fps())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CanvasSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub background: [u8; 4],
    pub audio_sample_rate: u32,
}
impl Default for CanvasSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::default(),
            background: [14, 15, 18, 255],
            audio_sample_rate: 48_000,
        }
    }
}
impl CanvasSettings {
    pub fn preset(aspect: &str) -> Self {
        let (width, height) = match aspect {
            "9:16" => (1080, 1920),
            "1:1" => (1080, 1080),
            _ => (1920, 1080),
        };
        Self {
            width,
            height,
            ..Self::default()
        }
    }
    pub fn validate(&self) -> bool {
        (16..=16384).contains(&self.width)
            && (16..=16384).contains(&self.height)
            && u64::from(self.width) * u64::from(self.height) <= 134_217_728
            && self.frame_rate.validate()
            && (8_000..=192_000).contains(&self.audio_sample_rate)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
    Gif,
    /// A colour lookup table. It is the asset of a `Lut` effect and never a
    /// clip: it has no picture, no sound and no time. Its `duration` is one
    /// microsecond, `width` is the points along each edge of its cube and
    /// `height` is zero.
    Lut,
    /// A font file whose family titles may name. Like a lookup table it has
    /// no picture, sound or time and is never a clip; its name is its
    /// family.
    Font,
}
impl MediaKind {
    /// A kind that is only ever read by something else, never placed.
    pub fn is_resource(self) -> bool {
        matches!(self, Self::Lut | Self::Font)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Asset {
    pub id: AssetId,
    pub name: String,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub duration: Time,
    pub width: u32,
    pub height: u32,
    pub frame_rate: Option<FrameRate>,
    pub has_audio: bool,
    pub folder_id: Option<FolderId>,
    pub file_size: u64,
}
impl Asset {
    pub fn new(path: PathBuf, kind: MediaKind, duration: Time) -> Self {
        let name = path
            .file_name()
            .map_or_else(|| "Media".into(), |n| n.to_string_lossy().into_owned());
        Self {
            id: Uuid::new_v4(),
            name,
            path,
            kind,
            duration,
            width: 0,
            height: 0,
            frame_rate: None,
            has_audio: kind == MediaKind::Audio,
            folder_id: None,
            file_size: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MediaFolder {
    pub id: FolderId,
    pub name: String,
    pub parent_id: Option<FolderId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
    Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    pub locked: bool,
    pub hidden: bool,
    pub muted: bool,
    // Linear volume of everything on the track, within `0.0..=8.0`. Plain
    // comments here and below: the planner text says it, the schema need not.
    #[serde(default = "unity", skip_serializing_if = "is_unity")]
    pub gain: f32,
    // While any video or audio track is soloed, only soloed tracks sound.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub solo: bool,
    pub clips: Vec<Clip>,
    /// The language of a caption track: a text track whose titles are
    /// cues. A BCP 47 style tag such as `en` or `pt-BR`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}
fn unity() -> f32 {
    1.0
}
fn is_unity(gain: &f32) -> bool {
    *gain == 1.0
}
impl Track {
    pub fn new(name: impl Into<String>, kind: TrackKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            locked: false,
            hidden: false,
            muted: false,
            gain: 1.0,
            solo: false,
            clips: Vec::new(),
            language: None,
        }
    }
    pub fn active_clips(&self, time: Time) -> impl Iterator<Item = &Clip> {
        let end = self.clips.partition_point(|c| c.start <= time);
        self.clips[..end].iter().filter(move |c| time < c.end())
    }

    /// Borrow clips intersecting the half-open range `[start, end)` in track order.
    /// Empty or reversed ranges yield no clips; touching an edge is not overlap.
    ///
    /// Requires clips sorted by nondecreasing start, as enforced by
    /// [`Project::validate`]. Uses no allocation and takes O(log n + k) to consume,
    /// where n is the track's clip count and k counts clips starting before `end`.
    /// Clip ends may be nonmonotonic when clips overlap or nest.
    pub fn clips_overlapping(&self, start: Time, end: Time) -> impl Iterator<Item = &Clip> {
        let prefix = if start < end {
            self.clips.partition_point(|c| c.start < end)
        } else {
            0
        };
        self.clips[..prefix].iter().filter(move |c| c.end() > start)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipContent {
    Media {
        asset_id: AssetId,
    },
    /// A title: text drawn on the canvas. With word times it is a caption
    /// cue whose words can be highlighted as they are said.
    Text {
        text: String,
        font_size: f32,
        color: [u8; 4],
        #[serde(default, skip_serializing_if = "TextStyle::is_plain")]
        style: TextStyle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enter: Option<TextAnimation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit: Option<TextAnimation>,
        /// Clip-relative times of the words of `text`, or none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        words: Vec<WordTime>,
    },
    /// Named shapes and texts in a design box: a lower third, a title card,
    /// a callout or a sticker. The clip's transform places the box's centre.
    Graphic {
        size: [f32; 2],
        elements: Vec<Element>,
    },
}
impl ClipContent {
    /// Generated picture, drawn by the text renderer: a title or a graphic.
    pub fn is_generated(&self) -> bool {
        !matches!(self, Self::Media { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Transform {
    pub position: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: f32,
    pub opacity: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Normalized amounts removed from the left, top, right, bottom.
    pub crop: [f32; 4],
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0],
            scale: [1.0, 1.0],
            rotation: 0.0,
            opacity: 1.0,
            flip_x: false,
            flip_y: false,
            crop: [0.0; 4],
        }
    }
}
impl Transform {
    pub fn validate(&self) -> bool {
        self.position
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1_000_000.0)
            && self
                .scale
                .iter()
                .all(|v| v.is_finite() && (0.001..=100.0).contains(v))
            && self.rotation.is_finite()
            && self.rotation.abs() <= 36_000.0
            && self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
            && self
                .crop
                .iter()
                .all(|v| v.is_finite() && (0.0..1.0).contains(v))
            && self.crop[0] + self.crop[2] < 1.0
            && self.crop[1] + self.crop[3] < 1.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Clip {
    pub id: ClipId,
    pub name: String,
    pub content: ClipContent,
    pub start: Time,
    pub source_in: Time,
    pub duration: Time,
    pub playback_rate: f64,
    pub transform: Transform,
    pub linked_id: Option<Uuid>,
    pub group_id: Option<Uuid>,
    pub gain: f32,
    #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
    pub blend: BlendMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub masks: Vec<Mask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub animation: Vec<AnimTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retime: Option<Retime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<Transition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_out: Option<Transition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<Follow>,
    // Timeline length the sound rises over from the clip's start.
    #[serde(default, skip_serializing_if = "Time::is_zero")]
    pub fade_in: Time,
    // Timeline length the sound falls over to the clip's end.
    #[serde(default, skip_serializing_if = "Time::is_zero")]
    pub fade_out: Time,
}
impl Clip {
    pub fn from_asset(asset: &Asset, start: Time) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: asset.name.clone(),
            content: ClipContent::Media { asset_id: asset.id },
            start,
            source_in: Time::ZERO,
            duration: asset.duration,
            playback_rate: 1.0,
            transform: Transform::default(),
            linked_id: None,
            group_id: None,
            gain: 1.0,
            blend: BlendMode::Normal,
            effects: Vec::new(),
            masks: Vec::new(),
            animation: Vec::new(),
            retime: None,
            transition_in: None,
            transition_out: None,
            follow: None,
            fade_in: Time::ZERO,
            fade_out: Time::ZERO,
        }
    }
    pub fn text(text: impl Into<String>, start: Time, duration: Time) -> Self {
        let text = text.into();
        Self {
            id: Uuid::new_v4(),
            name: text.clone(),
            content: ClipContent::Text {
                text,
                font_size: 64.0,
                color: [255; 4],
                style: TextStyle::default(),
                enter: None,
                exit: None,
                words: Vec::new(),
            },
            start,
            source_in: Time::ZERO,
            duration,
            playback_rate: 1.0,
            transform: Transform::default(),
            linked_id: None,
            group_id: None,
            gain: 1.0,
            blend: BlendMode::Normal,
            effects: Vec::new(),
            masks: Vec::new(),
            animation: Vec::new(),
            retime: None,
            transition_in: None,
            transition_out: None,
            follow: None,
            fade_in: Time::ZERO,
            fade_out: Time::ZERO,
        }
    }
    /// A graphic clip of `elements` in a box of `size` design pixels.
    pub fn graphic(
        name: impl Into<String>,
        size: [f32; 2],
        elements: Vec<Element>,
        start: Time,
        duration: Time,
    ) -> Self {
        let mut clip = Self::text("", start, duration);
        clip.name = name.into();
        clip.content = ClipContent::Graphic { size, elements };
        clip
    }
    pub fn end(&self) -> Time {
        self.start + self.duration
    }
    /// Source position shown at timeline `playhead`, through the time map when one exists.
    pub fn source_time(&self, playhead: Time) -> Time {
        let local = playhead - self.start;
        self.source_in
            + match &self.retime {
                Some(retime) => retime.source_at(local),
                None => local.scaled(self.playback_rate),
            }
    }
    /// Earliest and latest source positions the clip uses.
    pub fn source_span(&self) -> (Time, Time) {
        match &self.retime {
            Some(retime) => {
                let (min, max) = retime.span();
                (self.source_in + min, self.source_in + max)
            }
            None => (
                self.source_in,
                self.source_in + self.duration.scaled(self.playback_rate),
            ),
        }
    }
    /// Signed source seconds per timeline second at `playhead`; 0 on a freeze.
    pub fn speed_at(&self, playhead: Time) -> f64 {
        match &self.retime {
            Some(retime) => retime.speed_at(playhead - self.start),
            None => self.playback_rate,
        }
    }
    pub fn asset_id(&self) -> Option<AssetId> {
        match self.content {
            ClipContent::Media { asset_id } => Some(asset_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Marker {
    pub id: MarkerId,
    pub time: Time,
    pub label: String,
    pub color: [u8; 4],
}

/// Unchanged tracks and library maps are shared with history and worker snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub settings: CanvasSettings,
    pub assets: Arc<BTreeMap<AssetId, Asset>>,
    pub folders: Arc<BTreeMap<FolderId, MediaFolder>>,
    pub tracks: Vec<Arc<Track>>,
    pub markers: Arc<Vec<Marker>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub trackers: Arc<BTreeMap<Uuid, Arc<Tracker>>>,
}
impl Default for Project {
    fn default() -> Self {
        Self::new("Untitled")
    }
}
impl Project {
    /// A transient source monitor document. It shares no editing history and
    /// never inserts clips into the person's timeline.
    pub fn asset_preview(asset: &Asset) -> Result<Self, crate::EditError> {
        // A lookup table or a font has nothing to monitor.
        if asset.kind.is_resource() {
            return Err(crate::EditError::IncompatibleTrack);
        }
        let mut project = Self::new(asset.name.clone());
        let kind = if asset.kind == MediaKind::Audio {
            TrackKind::Audio
        } else {
            TrackKind::Video
        };
        let mut track = Track::new("Source", kind);
        track.clips.push(Clip::from_asset(asset, Time::ZERO));
        project.tracks = vec![Arc::new(track)];
        let mut preview_asset = asset.clone();
        // Library organization belongs to the editing document. A source
        // monitor has no folder tree, while retaining the original asset ID.
        preview_asset.folder_id = None;
        Arc::make_mut(&mut project.assets).insert(asset.id, preview_asset);
        if let Some(rate) = asset.frame_rate {
            project.settings.frame_rate = rate;
        }
        if asset.width >= 16 && asset.height >= 16 {
            let settings = CanvasSettings {
                width: asset.width,
                height: asset.height,
                ..project.settings.clone()
            };
            if settings.validate() {
                project.settings = settings;
            }
        }
        project.validate()?;
        Ok(project)
    }
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            settings: CanvasSettings::default(),
            assets: Arc::new(BTreeMap::new()),
            folders: Arc::new(BTreeMap::new()),
            tracks: vec![
                Arc::new(Track::new("Video 1", TrackKind::Video)),
                Arc::new(Track::new("Audio 1", TrackKind::Audio)),
                Arc::new(Track::new("Text 1", TrackKind::Text)),
            ],
            markers: Arc::new(Vec::new()),
            trackers: Arc::new(BTreeMap::new()),
        }
    }
    pub fn duration(&self) -> Time {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(Clip::end)
            .max()
            .unwrap_or(Time::ZERO)
    }
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id).map(AsRef::as_ref)
    }
    pub fn clip(&self, id: ClipId) -> Option<(&Track, &Clip)> {
        self.tracks
            .iter()
            .find_map(|t| t.clips.iter().find(|c| c.id == id).map(|c| (t.as_ref(), c)))
    }
    pub fn active_clips(&self, time: Time) -> impl Iterator<Item = (&Track, &Clip)> {
        self.tracks
            .iter()
            .filter(|t| !t.hidden)
            .flat_map(move |t| t.active_clips(time).map(move |c| (t.as_ref(), c)))
    }
    pub fn validate(&self) -> Result<(), crate::EditError> {
        use crate::EditError;
        if self.name.trim().is_empty() || !self.settings.validate() {
            return Err(EditError::Invalid("project settings".into()));
        }
        // Identity validation needs membership only. Document traversal still
        // determines the first error; sorting every UUID adds no information.
        let identity_count = 1
            + self.folders.len()
            + self.assets.len()
            + self.tracks.len()
            + self.markers.len()
            + self.trackers.len()
            + self
                .tracks
                .iter()
                .map(|track| track.clips.len())
                .sum::<usize>();
        let mut ids = HashSet::with_capacity(identity_count);
        ids.insert(self.id);
        for (id, folder) in self.folders.iter() {
            if *id != folder.id || !ids.insert(*id) || folder.name.trim().is_empty() {
                return Err(EditError::Invalid("folder identity".into()));
            }
            let mut seen = BTreeSet::from([*id]);
            let mut parent = folder.parent_id;
            while let Some(parent_id) = parent {
                if !seen.insert(parent_id) {
                    return Err(EditError::Invalid("folder cycle".into()));
                }
                parent = self
                    .folders
                    .get(&parent_id)
                    .ok_or(EditError::Missing(parent_id))?
                    .parent_id;
            }
        }
        for (id, asset) in self.assets.iter() {
            if *id != asset.id
                || !ids.insert(*id)
                || asset.path.as_os_str().is_empty()
                || asset.duration <= Time::ZERO
                || !asset.duration.is_valid()
                || asset.frame_rate.is_some_and(|f| !f.validate())
                || (asset.kind.is_resource() && (asset.has_audio || asset.frame_rate.is_some()))
            {
                return Err(EditError::Invalid("asset metadata".into()));
            }
            if let Some(folder) = asset.folder_id
                && !self.folders.contains_key(&folder)
            {
                return Err(EditError::Missing(folder));
            }
        }
        let frame = self.settings.frame_rate.frame_time(1);
        for track in &self.tracks {
            if !ids.insert(track.id) || track.name.trim().is_empty() {
                return Err(EditError::Invalid("track identity".into()));
            }
            if !track.gain.is_finite() || !(0.0..=8.0).contains(&track.gain) {
                return Err(EditError::Invalid("track volume".into()));
            }
            if let Some(language) = &track.language
                && (track.kind != TrackKind::Text || !title::language_valid(language))
            {
                return Err(EditError::Invalid("track language".into()));
            }
            let mut previous = Time::ZERO;
            for clip in &track.clips {
                let invalid = |what: &str| {
                    EditError::Invalid(format!("{what} (clip {}, track {})", clip.id, track.id))
                };
                if !ids.insert(clip.id)
                    || !clip.start.is_valid()
                    || clip.start < previous
                    || clip.duration <= Time::ZERO
                    || !clip.duration.is_valid()
                    || !clip.end().is_valid()
                    || !clip.source_in.is_valid()
                    || !clip.playback_rate.is_finite()
                    || !(0.05..=16.0).contains(&clip.playback_rate)
                    || !clip.transform.validate()
                    || !clip.gain.is_finite()
                    || !(0.0..=8.0).contains(&clip.gain)
                {
                    return Err(invalid("clip bounds or transform"));
                }
                previous = clip.start;
                // A fade is held to its edge, and the two never meet.
                if clip.fade_in < Time::ZERO
                    || clip.fade_out < Time::ZERO
                    || clip.fade_in + clip.fade_out > clip.duration
                {
                    return Err(invalid("fades"));
                }
                let faded = !clip.fade_in.is_zero() || !clip.fade_out.is_zero();
                match &clip.content {
                    ClipContent::Media { asset_id } => {
                        let asset = self
                            .assets
                            .get(asset_id)
                            .ok_or(EditError::Missing(*asset_id))?;
                        // A lookup table or a font is on no track.
                        let allowed = !asset.kind.is_resource()
                            && match track.kind {
                                TrackKind::Video => asset.kind != MediaKind::Audio,
                                TrackKind::Audio => asset.has_audio,
                                TrackKind::Text => false,
                            };
                        if !allowed {
                            return Err(EditError::IncompatibleTrack);
                        }
                        if asset.kind != MediaKind::Image
                            && clip.source_span().1 > asset.duration + Time(2)
                        {
                            return Err(invalid("clip extends beyond source"));
                        }
                        // Only sound fades.
                        if faded && !asset.has_audio && asset.kind != MediaKind::Audio {
                            return Err(invalid("fades"));
                        }
                    }
                    ClipContent::Text {
                        text,
                        font_size,
                        style,
                        enter,
                        exit,
                        words,
                        ..
                    } => {
                        if track.kind != TrackKind::Text {
                            return Err(EditError::IncompatibleTrack);
                        }
                        if text.len() > 1_048_576
                            || !font_size.is_finite()
                            || !(1.0..=4096.0).contains(font_size)
                            || !style.validate()
                        {
                            return Err(invalid("text style"));
                        }
                        for (edge, animation) in [("enter", enter), ("exit", exit)] {
                            if let Some(animation) = animation
                                && !animation.validate(frame, clip.duration)
                            {
                                return Err(EditError::Invalid(format!(
                                    "text animation (clip {}, content.{edge}): duration {} us, delay {} us, clip duration {} us, minimum duration {} us",
                                    clip.id,
                                    animation.duration.0,
                                    animation.delay.0,
                                    clip.duration.0,
                                    frame.0
                                )));
                            }
                        }
                        if !title::animations_fit(enter.as_ref(), exit.as_ref(), clip.duration) {
                            return Err(EditError::Invalid(format!(
                                "text animation (clip {}): content.enter and content.exit overlap within {} us",
                                clip.id, clip.duration.0
                            )));
                        }
                        if faded {
                            return Err(invalid("fades"));
                        }
                        if !title::words_valid(words, text, clip.duration) {
                            let detail = words.iter().enumerate().find(|(index, word)| {
                                word.at < Time::ZERO
                                    || word.end < word.at
                                    || word.end > clip.duration
                                    || (*index > 0 && words[*index - 1].at > word.at)
                            }).map_or_else(
                                || format!("{} times for {} words (maximum {})", words.len(), title::word_count(text), title::MAX_WORDS),
                                |(index, word)| format!("content.words[{index}]: {}..{} us, clip duration {} us; times must be ordered and inside the clip", word.at.0, word.end.0, clip.duration.0),
                            );
                            return Err(EditError::Invalid(format!(
                                "word times (clip {}): {detail}",
                                clip.id
                            )));
                        }
                        Self::validate_generated(clip)?;
                    }
                    ClipContent::Graphic { size, elements } => {
                        if track.kind != TrackKind::Text {
                            return Err(EditError::IncompatibleTrack);
                        }
                        if faded {
                            return Err(invalid("fades"));
                        }
                        if !title::graphic_valid(*size, elements, frame, clip.duration) {
                            return Err(invalid("graphic"));
                        }
                        Self::validate_generated(clip)?;
                    }
                }
                // Nothing draws an audio clip, so it carries sound only.
                if track.kind == TrackKind::Audio
                    && (clip.effects.iter().any(|effect| !effect.kind.is_audio())
                        || !clip.masks.is_empty()
                        || !clip.blend.is_normal()
                        || clip.transition_in.is_some()
                        || clip.transition_out.is_some()
                        || clip.follow.is_some()
                        || clip
                            .animation
                            .iter()
                            .any(|a| a.target.is_some() || a.key != ParamKey::Gain))
                {
                    return Err(invalid("audio clip look"));
                }
                // The decoder of a clip on an audio track is the only one filtered.
                if track.kind != TrackKind::Audio
                    && clip.effects.iter().any(|effect| effect.kind.is_audio())
                {
                    return Err(invalid("audio effect on a picture clip"));
                }
                self.validate_look(clip, frame)?;
            }
        }
        if self.trackers.len() > look::MAX_TRACKERS {
            return Err(EditError::Invalid("tracker count".into()));
        }
        for (id, tracker) in self.trackers.iter() {
            let asset = self
                .assets
                .get(&tracker.asset_id)
                .ok_or(EditError::Missing(tracker.asset_id))?;
            if *id != tracker.id
                || !ids.insert(*id)
                || !tracker.validate()
                || asset.kind == MediaKind::Audio
                || asset.kind.is_resource()
                || (asset.kind != MediaKind::Image
                    && tracker
                        .keys
                        .last()
                        .is_some_and(|key| key.source > asset.duration + Time(2)))
            {
                return Err(EditError::Invalid("tracker".into()));
            }
        }
        for marker in self.markers.iter() {
            if !ids.insert(marker.id) || !marker.time.is_valid() {
                return Err(EditError::Invalid("marker".into()));
            }
        }
        Ok(())
    }

    // What a title or a graphic cannot have: the renderer draws neither
    // through the compositor and neither has a source to retime.
    fn validate_generated(clip: &Clip) -> Result<(), crate::EditError> {
        if !clip.effects.is_empty()
            || !clip.masks.is_empty()
            || !clip.blend.is_normal()
            || clip.retime.is_some()
        {
            return Err(crate::EditError::Invalid(format!(
                "text clip look (clip {})",
                clip.id
            )));
        }
        Ok(())
    }

    // Bounds of the look, animation, time map, transitions and follow on one clip.
    fn validate_look(&self, clip: &Clip, frame: Time) -> Result<(), crate::EditError> {
        use crate::EditError;
        let invalid = |what: &str| Err(EditError::Invalid(format!("{what} (clip {})", clip.id)));
        if clip.effects.len() > look::MAX_EFFECTS || clip.masks.len() > look::MAX_MASKS {
            return invalid("effect or mask count");
        }
        let mut targets = HashSet::with_capacity(clip.effects.len() + clip.masks.len());
        for (index, effect) in clip.effects.iter().enumerate() {
            if !targets.insert(effect.id) || !effect.validate() {
                return Err(EditError::Invalid(format!(
                    "effect (clip {}, effects[{index}] {})",
                    clip.id, effect.id
                )));
            }
            if let Some(asset_id) = effect.asset_id {
                let asset = self
                    .assets
                    .get(&asset_id)
                    .ok_or(EditError::Missing(asset_id))?;
                // A lookup is done in a table, and a clean plate is a still.
                let wanted = match effect.kind {
                    look::EffectKind::Lut => MediaKind::Lut,
                    _ => MediaKind::Image,
                };
                if asset.kind != wanted {
                    return invalid("effect");
                }
            }
        }
        for (index, mask) in clip.masks.iter().enumerate() {
            if !targets.insert(mask.id) || !mask.validate() {
                return Err(EditError::Invalid(format!(
                    "mask (clip {}, masks[{index}] {})",
                    clip.id, mask.id
                )));
            }
        }
        if clip.animation.len() > animate::MAX_ANIM_TRACKS {
            return invalid("animation track count");
        }
        for (index, track) in clip.animation.iter().enumerate() {
            let target = match track.target {
                None => AnimTarget::Clip,
                Some(id) => clip
                    .effects
                    .iter()
                    .find(|effect| effect.id == id)
                    .map(|effect| AnimTarget::Effect(effect.kind))
                    .or_else(|| {
                        clip.masks
                            .iter()
                            .any(|mask| mask.id == id)
                            .then_some(AnimTarget::Mask)
                    })
                    .ok_or(EditError::Missing(id))?,
            };
            let Some(spec) = look::param_spec(target, track.key) else {
                return invalid("animation parameter");
            };
            if clip.animation[..index]
                .iter()
                .any(|earlier| earlier.target == track.target && earlier.key == track.key)
                || !track.validate(clip.duration, spec)
            {
                let detail = if clip.animation[..index]
                    .iter()
                    .any(|earlier| earlier.target == track.target && earlier.key == track.key)
                {
                    "duplicate animation target and parameter".to_owned()
                } else if !(1..=animate::MAX_KEYFRAMES).contains(&track.keys.len()) {
                    format!(
                        "expected 1..={} keys, got {}",
                        animate::MAX_KEYFRAMES,
                        track.keys.len()
                    )
                } else {
                    let bad = track.keys.iter().enumerate().find(|(key_index, key)| {
                        key.at < Time::ZERO
                            || key.at > clip.duration
                            || !spec.contains(key.value)
                            || !key.ease.validate()
                            || (*key_index > 0 && track.keys[*key_index - 1].at >= key.at)
                    });
                    bad.map_or_else(
                        || "keys must be strictly ordered and valid".into(),
                        |(key_index, key)| format!("keys[{key_index}]: at {} us (expected 0..={} us), value {} (expected {}..={}), valid easing and strictly increasing times required", key.at.0, clip.duration.0, key.value, spec.min, spec.max),
                    )
                };
                return Err(EditError::Invalid(format!(
                    "animation keys (clip {}, animation[{index}], parameter {:?}, target {:?}): {detail}",
                    clip.id, track.key, track.target
                )));
            }
        }
        if let Some(retime) = &clip.retime
            && (clip.playback_rate != 1.0 || !retime.validate(clip.duration))
        {
            return invalid("time map");
        }
        if [&clip.transition_in, &clip.transition_out]
            .into_iter()
            .flatten()
            .any(|transition| !transition.validate(frame, clip.duration))
        {
            return invalid("transition");
        }
        if let Some(follow) = &clip.follow {
            if !self.trackers.contains_key(&follow.tracker_id) {
                return Err(EditError::Missing(follow.tracker_id));
            }
            if !follow.validate() {
                return invalid("follow offset");
            }
        }
        Ok(())
    }
}

/// Deterministic nearest-edge snapping; ties favor the earlier boundary.
pub fn snap_time(project: &Project, time: Time, tolerance: Time, exclude: &[ClipId]) -> Time {
    if tolerance.0 < 0 {
        return time;
    }
    let mut best = time;
    let mut distance = tolerance.0.saturating_add(1);
    let edges = project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter(|c| !exclude.contains(&c.id))
        .flat_map(|c| [c.start, c.end()])
        .chain(project.markers.iter().map(|m| m.time))
        .chain([Time::ZERO]);
    for edge in edges {
        let delta = edge.0.abs_diff(time.0);
        if delta <= tolerance.0 as u64
            && (delta < distance as u64 || (delta == distance as u64 && edge < best))
        {
            best = edge;
            distance = delta as i64;
        }
    }
    best
}
