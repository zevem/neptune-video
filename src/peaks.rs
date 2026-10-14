//! The peaks the editor holds for the sound it draws.
//!
//! A tile on an audio lane shows its media's sound as a waveform. The peaks
//! behind it are read by the sound lane, kept on disk in a bounded cache,
//! and held here while a tile in view wants them. Nothing here reads a file
//! or waits: peaks that are wanted and not held are put on a list the
//! application hands to the lane, first to ask what the cache keeps and,
//! when it keeps none, to read the sound. Until the answer comes the tile is
//! drawn without them.
//!
//! An answer is taken only when it is for the request this asked, for the
//! project that is open and for the media as it is now; any other is
//! dropped. The peaks held are bounded in bytes. Those least lately wanted
//! make room, and never those of a tile on screen: media that has no room
//! beside what is on screen is not asked about until it has. Sound being
//! read for a tile that is no longer drawn is given up, and read when the
//! tile is drawn again.
//!
//! Work that reads peaks rather than draws them, as the ducking gate does,
//! wants them by [`HeldPeaks::want_work`]: through the same requests, so a
//! tile and the work never ask the lane twice, but not as a tile on screen.
//! What work wants is not given up when no tile draws it, and is let go by
//! the work when it is done ([`HeldPeaks::let_go`]). It takes its room
//! when it comes, from the least lately wanted, and from tiles on screen
//! when nothing else is left; those tiles have no room until it is let go.
//!
//! The beats of a music file are held here too, in [`HeldBeats`]: a grid the
//! lane finds in the peaks it keeps, for the files whose beats a person asked
//! to see. Whether a file's beats are shown is the person's for the session;
//! the grids are derived, found again in another session and never stored.
//! [`beats_through`] puts a grid on the timeline through a clip, and
//! [`beat_markers`] makes markers of it, which are the only beats a project
//! keeps.
use editor_core::{Asset, AssetId, Clip, ClipId, Command, Marker, Project, Time, TrackKind, sound};
use editor_media::{
    BeatGrid, KeptPeaks, MAX_PEAK_BUCKETS, MediaError, PEAKS_PER_SECOND, PeakCache, PeakJob, Peaks,
    SoundEvent, SoundService, SourceIdentity,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use uuid::Uuid;

/// The bytes of peaks held at once. One file at the lane's bound is 8 MiB,
/// so as few as four are held together.
pub const MAX_HELD_PEAK_BYTES: usize = 32 << 20;

/// Where peaks are kept between sessions: the user's cache directory, or
/// the system's temporary one on a system without a home.
pub fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("app", "Aster", "Aster").map_or_else(
        || std::env::temp_dir().join("aster-peaks"),
        |dirs| dirs.cache_dir().join("peaks"),
    )
}

/// What the lane is to be asked about an asset's peaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// What the cache keeps of them.
    Kept,
    /// To read the sound: the cache keeps none of the media as it is.
    Sound,
}

/// What the lane answered.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Kept(KeptPeaks),
    Read(Result<Arc<Peaks>, String>),
}

#[derive(Clone, Debug, PartialEq)]
enum Known {
    /// Wanted, and not yet asked of the lane.
    Wanted(Ask),
    /// Asked: the request that answers, and what was asked.
    Asked(u64, Ask),
    Ready(Arc<Peaks>),
    /// Not held for want of room: asked about again when there is some.
    NoRoom,
    /// The sound could not be read. It is not asked about again.
    Failed,
}

struct Entry {
    /// The media the answer is about, as it was when it was wanted.
    source: SourceIdentity,
    known: Known,
    /// The bytes its peaks take: what its length says until they are read.
    bytes: usize,
    /// The frame a tile last wanted it in; none has when only work has.
    used: Option<u64>,
    /// Work wants it, and lets it go when done.
    work: bool,
}

impl Entry {
    /// The bytes it holds or is about to.
    fn claimed(&self) -> usize {
        match self.known {
            Known::Wanted(_) | Known::Asked(..) | Known::Ready(_) => self.bytes,
            Known::NoRoom | Known::Failed => 0,
        }
    }

    fn held(&self) -> usize {
        match &self.known {
            Known::Ready(peaks) => peaks.byte_len(),
            _ => 0,
        }
    }
}

/// What became of an answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Taken {
    /// The peaks are held now: the tiles that wanted them are drawn again.
    Kept,
    /// The cache keeps none of this media: the sound is read when a tile
    /// next wants it.
    Unkept,
    /// The sound could not be read: why.
    Failed(String),
    /// The peaks are more than there is room for beside those on screen.
    NoRoom,
    /// It is not for anything this waits for.
    Dropped,
}

pub struct HeldPeaks {
    entries: HashMap<AssetId, Entry>,
    wants: Vec<AssetId>,
    frame: u64,
    /// The least bytes of peaks turned away in this frame for want of room.
    refused: Option<usize>,
    /// The bytes of peaks held at most: [`MAX_HELD_PEAK_BYTES`].
    bound: usize,
    /// The beats found in the peaks the lane keeps, and whose are shown.
    pub beats: HeldBeats,
}

impl Default for HeldPeaks {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            wants: Vec::new(),
            frame: 0,
            refused: None,
            bound: MAX_HELD_PEAK_BYTES,
            beats: HeldBeats::default(),
        }
    }
}

// What the peaks of media of this length take, before they are read.
fn expected_bytes(asset: &Asset) -> usize {
    let micros = u64::try_from(asset.duration.0).unwrap_or(0);
    let buckets = (micros.saturating_mul(u64::from(PEAKS_PER_SECOND))).div_ceil(1_000_000);
    (buckets.min(MAX_PEAK_BUCKETS as u64) as usize) * 4
}

// Whether `source` is what `asset` is now: what `SourceIdentity::of(asset)
// == *source` says, without the path that builds. Every tile drawn asks.
fn is_of(source: &SourceIdentity, asset: &Asset) -> bool {
    let SourceIdentity {
        path,
        file_size,
        duration,
        width,
        height,
        frame_rate,
    } = source;
    *path == asset.path
        && *file_size == asset.file_size
        && *duration == asset.duration
        && *width == asset.width
        && *height == asset.height
        && *frame_rate == asset.frame_rate
}

impl HeldPeaks {
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.refused = None;
        self.beats.begin_frame();
    }

    /// Gives up everything: the project is another. What the lane still
    /// answers is then for nothing held here, and no media's beats are
    /// shown.
    pub fn clear(&mut self) {
        *self = Self {
            frame: self.frame,
            bound: self.bound,
            ..Self::default()
        };
    }

    /// Holds at most `bound` bytes of peaks, for a test to fill.
    #[cfg(test)]
    pub fn bounded(bound: usize) -> Self {
        Self {
            bound,
            ..Self::default()
        }
    }

    /// The bytes of peaks held.
    pub fn bytes(&self) -> usize {
        self.entries.values().map(Entry::held).sum()
    }

    // Whether it was wanted in this frame or the last: its tile is on
    // screen, as far as anything drawn so far says.
    fn on_screen(&self, entry: &Entry) -> bool {
        entry.used.is_some_and(|used| used + 1 >= self.frame)
    }

    // Whether peaks of `bytes` fit beside those on screen, those work
    // wants, and those on their way to either.
    fn has_room(&self, bytes: usize, but: AssetId) -> bool {
        let claimed: usize = (self.entries.iter())
            .filter(|(id, entry)| **id != but && (self.on_screen(entry) || entry.work))
            .map(|(_, entry)| entry.claimed())
            .sum();
        claimed + bytes <= self.bound
    }

    /// Says a tile in view shows the sound of `asset` this frame. Peaks
    /// that are not held, or are held of what the media no longer is, are
    /// put on the list to ask the lane about, once.
    pub fn want(&mut self, asset: &Asset) {
        if !asset.has_audio {
            return;
        }
        let frame = self.frame;
        let known = (self.entries.get_mut(&asset.id))
            .filter(|entry| is_of(&entry.source, asset))
            .map(|entry| {
                entry.used = Some(frame);
                (entry.known.clone(), entry.bytes)
            });
        let bytes = match known {
            // Still to be asked: a question the lane had no room for is put
            // again by the tile that wants the answer.
            Some((Known::Wanted(_), _)) => {
                if !self.wants.contains(&asset.id) {
                    self.wants.push(asset.id);
                }
                return;
            }
            Some((Known::NoRoom, bytes)) => bytes,
            Some(_) => return,
            None => expected_bytes(asset),
        };
        // Room only shrinks while a frame is drawn, so one refusal answers
        // for the rest of it for peaks no smaller. Smaller ones may fit yet.
        let room = self.refused.is_none_or(|least| bytes < least) && self.has_room(bytes, asset.id);
        if !room {
            self.refused = Some(self.refused.map_or(bytes, |least| least.min(bytes)));
        }
        self.entries.insert(
            asset.id,
            Entry {
                source: SourceIdentity::of(asset),
                known: if room {
                    Known::Wanted(Ask::Kept)
                } else {
                    Known::NoRoom
                },
                bytes,
                used: Some(frame),
                work: false,
            },
        );
        self.wants.retain(|id| *id != asset.id);
        if room {
            self.wants.push(asset.id);
        }
    }

    /// Says work wants the peaks of `asset`, which it reads through
    /// [`Self::held`] once they are there and lets go of by
    /// [`Self::let_go`]. Peaks that are not held are put on the list to ask
    /// the lane about, once, whatever the room: they take theirs when they
    /// come. Wanted again while the lane is asked, nothing more is asked.
    pub fn want_work(&mut self, asset: &Asset) {
        let current = (self.entries.get_mut(&asset.id)).filter(|entry| is_of(&entry.source, asset));
        let Some(entry) = current else {
            self.entries.insert(
                asset.id,
                Entry {
                    source: SourceIdentity::of(asset),
                    known: Known::Wanted(Ask::Kept),
                    bytes: expected_bytes(asset),
                    used: None,
                    work: true,
                },
            );
            self.wants.retain(|id| *id != asset.id);
            self.wants.push(asset.id);
            return;
        };
        entry.work = true;
        // Work does not wait for room on screen.
        if entry.known == Known::NoRoom {
            entry.known = Known::Wanted(Ask::Kept);
        }
        if matches!(entry.known, Known::Wanted(_)) && !self.wants.contains(&asset.id) {
            self.wants.push(asset.id);
        }
    }

    /// Whether the peaks of `asset` as it is now could not be read: work
    /// that wants them goes without.
    pub fn failed(&self, asset: &Asset) -> bool {
        (self.entries.get(&asset.id))
            .is_some_and(|entry| entry.known == Known::Failed && is_of(&entry.source, asset))
    }

    /// Says work is done with the peaks of `asset`. Unless a tile on screen
    /// wants them they are let go, and a read still under way is handed
    /// back for the lane to be told. A failure is kept: it holds no room,
    /// and the sound is not read again, nor said again, for a tile.
    pub fn let_go(&mut self, asset: AssetId) -> Option<u64> {
        let shown = (self.entries.get(&asset)).is_some_and(|entry| self.on_screen(entry));
        let entry = self.entries.get_mut(&asset)?;
        entry.work = false;
        if shown || entry.known == Known::Failed {
            return None;
        }
        self.forget_asset(asset)
    }

    /// Whether peaks, or a beat grid, were turned away this frame for want
    /// of room that the next frame may have: a tile was on screen in the
    /// last frame and not in this one. Nothing else would bring that frame
    /// about.
    pub fn room_is_due(&self) -> bool {
        let peaks = self.refused.is_some()
            && (self.entries.values()).any(|entry| {
                entry.used.is_some_and(|used| used + 1 == self.frame) && entry.claimed() > 0
            });
        peaks || self.beats.room_is_due()
    }

    /// The assets to ask the lane about, each once, and what to ask.
    pub fn take_wants(&mut self) -> Vec<(AssetId, Ask)> {
        (std::mem::take(&mut self.wants).into_iter())
            .filter_map(|id| match self.entries.get(&id)?.known {
                Known::Wanted(ask) => Some((id, ask)),
                _ => None,
            })
            .collect()
    }

    /// Whether anything waits to be asked of the lane: peaks, or beats.
    pub fn wanting(&self) -> bool {
        !self.wants.is_empty() || self.beats.wanting()
    }

    /// The sound being read for tiles that are no longer drawn: the
    /// requests, for the lane to be told. Each is read when its tile is
    /// drawn again, and until then holds up no tile that is. What work
    /// wants is read on.
    pub fn take_unwanted(&mut self) -> Vec<u64> {
        let frame = self.frame;
        (self.entries.values_mut())
            .filter(|entry| !entry.work && entry.used.is_none_or(|used| used + 1 < frame))
            .filter_map(|entry| match entry.known {
                Known::Asked(request, Ask::Sound) => {
                    entry.known = Known::Wanted(Ask::Sound);
                    Some(request)
                }
                _ => None,
            })
            .collect()
    }

    /// Has the lane give up the sound it reads for tiles no longer drawn:
    /// see [`Self::take_unwanted`].
    pub fn give_up_unwanted(&mut self, sound: &SoundService) {
        for request in self.take_unwanted() {
            sound.cancel(request);
        }
    }

    /// The project is another: the lane gives up everything asked of it,
    /// and nothing is held.
    pub fn project_replaced(&mut self, sound: Option<&SoundService>) {
        if let Some(sound) = sound {
            sound.cancel_all();
        }
        self.clear();
    }

    /// Media left the library or is now of another file: what is known of
    /// it is forgotten, its beats too, and the lane gives up what it was
    /// asked about it. Whether its beats are shown is kept.
    pub fn media_replaced(&mut self, asset: AssetId, sound: Option<&SoundService>) {
        let asked = [self.forget_asset(asset), self.beats.forget_asset(asset)];
        if let Some(sound) = sound {
            for request in asked.into_iter().flatten() {
                sound.cancel(request);
            }
        }
    }

    /// Asks the lane about the peaks the tiles of this frame wanted and did
    /// not have: what `cache` keeps of them and, for those it keeps none
    /// of, to read the sound. Asking reads no file here and returns at once.
    /// Hands back, in words, the media the lane will not read at all.
    pub fn ask(
        &mut self,
        sound: &SoundService,
        project: &Project,
        instance: Uuid,
        cache: &PeakCache,
    ) -> Vec<String> {
        // First, so that what is in view has the lane's room.
        self.give_up_unwanted(sound);
        let mut refused = Vec::new();
        for (id, ask) in self.take_wants() {
            let Some(asset) = project.assets.get(&id) else {
                continue;
            };
            let job = PeakJob {
                project: instance,
                asset: asset.clone(),
                cache: cache.clone(),
            };
            let asked = match ask {
                Ask::Kept => sound.load_peaks(job),
                Ask::Sound => sound.peaks(job),
            };
            match asked {
                Ok(request) => self.asked(id, request),
                // The lane is full: asked again by the tile that next wants
                // it, in the frame an answer of the lane brings about.
                Err(MediaError::Busy) => {}
                // Media the lane will not read, as too long.
                Err(error) => {
                    self.refuse(id);
                    refused.push(format!("{} has no waveform: {error}", asset.name));
                }
            }
        }
        // The beats of media whose peaks the cache keeps, found in them.
        let entries = &self.entries;
        let kept = |asset: &Asset| ready(entries, asset).is_some();
        refused.extend(self.beats.ask_with(project, kept, |asset| {
            sound.beats(PeakJob {
                project: instance,
                asset: asset.clone(),
                cache: cache.clone(),
            })
        }));
        refused
    }

    /// Takes what the lane said, with the project that is open and its
    /// identity: see [`Self::take`]. Hands back, in words, sound that could
    /// not be read.
    pub fn answered(
        &mut self,
        instance: Uuid,
        project: &Project,
        event: SoundEvent,
    ) -> Option<String> {
        let (request, asked_by, asset_id, source, answer) = match event {
            // A waveform shows no progress: the tile keeps its level line
            // until the sound is read. A measurement, a written file and
            // stems are not a waveform's.
            SoundEvent::Progress { .. }
            | SoundEvent::Measured { .. }
            | SoundEvent::Extracted { .. }
            | SoundEvent::Separated { .. } => {
                return None;
            }
            SoundEvent::Beats {
                request,
                project: asked_by,
                asset_id,
                source,
                result,
            } => {
                let now = project.assets.get(&asset_id);
                return (self.beats)
                    .take(instance, now, request, asked_by, asset_id, &source, result);
            }
            SoundEvent::Loaded {
                request,
                project,
                asset_id,
                source,
                kept,
            } => (request, project, asset_id, source, Answer::Kept(kept)),
            // Peaks that could not be kept for another session are drawn
            // all the same, and read again in the next.
            SoundEvent::Peaked {
                request,
                project,
                asset_id,
                source,
                result,
                unkept: _,
            } => (request, project, asset_id, source, Answer::Read(result)),
        };
        let now = project.assets.get(&asset_id);
        let taken = self.take(instance, now, request, asked_by, asset_id, &source, answer);
        match taken {
            Taken::Failed(why) => {
                let name = now.map_or("The media", |asset| asset.name.as_str());
                // The first line of what FFmpeg said: the rest repeats it.
                let why = why.lines().next().unwrap_or_default();
                Some(format!("{name} has no waveform: {why}"))
            }
            _ => None,
        }
    }

    /// Says the lane took the question about `asset` as `request`.
    pub fn asked(&mut self, asset: AssetId, request: u64) {
        if let Some(entry) = self.entries.get_mut(&asset)
            && let Known::Wanted(ask) = entry.known
        {
            entry.known = Known::Asked(request, ask);
        }
    }

    /// Says the sound of `asset` cannot be asked about at all. It is not
    /// asked about again while the media is what it is.
    pub fn refuse(&mut self, asset: AssetId) {
        if let Some(entry) = self.entries.get_mut(&asset) {
            entry.known = Known::Failed;
        }
    }

    /// Forgets what is known of an asset that left the library or is now of
    /// another file, and hands back the request still waited for, for the
    /// lane to be told.
    pub fn forget_asset(&mut self, asset: AssetId) -> Option<u64> {
        self.wants.retain(|id| *id != asset);
        match self.entries.remove(&asset)?.known {
            Known::Asked(request, _) => Some(request),
            _ => None,
        }
    }

    /// Takes an answer of the lane. `instance` is the project that is open
    /// and `now` the asset as the project has it, if it still does.
    ///
    /// It is taken when it answers the request last put for its asset, by
    /// this project, of media that is still what was asked about. Anything
    /// else changes nothing.
    #[expect(
        clippy::too_many_arguments,
        reason = "what one answer carries, handed over as it is"
    )]
    pub fn take(
        &mut self,
        instance: Uuid,
        now: Option<&Asset>,
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        source: &SourceIdentity,
        answer: Answer,
    ) -> Taken {
        let ask = match &answer {
            Answer::Kept(_) => Ask::Kept,
            Answer::Read(_) => Ask::Sound,
        };
        let asked = (self.entries.get(&asset_id)).is_some_and(|entry| {
            entry.known == Known::Asked(request, ask) && entry.source == *source
        });
        if !asked || project != instance {
            return Taken::Dropped;
        }
        if !now.is_some_and(|asset| is_of(source, asset)) {
            // What it was asked about is no more: asked again when wanted.
            self.entries.remove(&asset_id);
            return Taken::Dropped;
        }
        let (known, taken) = match answer {
            Answer::Kept(KeptPeaks::Held(peaks)) | Answer::Read(Ok(peaks)) => {
                let bytes = peaks.byte_len();
                if self.make_room(bytes, asset_id) {
                    (Known::Ready(peaks), Taken::Kept)
                } else if self.entries.get(&asset_id).is_some_and(|entry| entry.work) {
                    // Work makes room of everything else: these are more
                    // than the bound itself.
                    let why = format!("its peaks are {bytes} bytes, more than are held at once");
                    (Known::Failed, Taken::Failed(why))
                } else {
                    (Known::NoRoom, Taken::NoRoom)
                }
            }
            Answer::Kept(KeptPeaks::Stale | KeptPeaks::Missing) => {
                // No tile may be drawn to ask for what work waits for.
                if self.entries.get(&asset_id).is_some_and(|entry| entry.work)
                    && !self.wants.contains(&asset_id)
                {
                    self.wants.push(asset_id);
                }
                (Known::Wanted(Ask::Sound), Taken::Unkept)
            }
            Answer::Read(Err(why)) => (Known::Failed, Taken::Failed(why)),
        };
        if let Some(entry) = self.entries.get_mut(&asset_id) {
            entry.known = known;
        }
        taken
    }

    // Makes room for `bytes` of peaks of `keep`: those least lately wanted
    // go first, and none whose tile is on screen, unless work wants `keep`.
    // Peaks work wants are never taken. Says whether there is room.
    fn make_room(&mut self, bytes: usize, keep: AssetId) -> bool {
        // What is read may be more than the length said: remembered, so the
        // media is asked about again only when that much is free.
        let work = match self.entries.get_mut(&keep) {
            Some(entry) => {
                entry.bytes = bytes;
                entry.work
            }
            None => false,
        };
        while self.bytes() + bytes > self.bound {
            let oldest = (self.entries.iter())
                .filter(|(id, entry)| {
                    **id != keep
                        && entry.held() > 0
                        && !entry.work
                        && (work || !self.on_screen(entry))
                })
                .min_by_key(|(_, entry)| (self.on_screen(entry), entry.used))
                .map(|(id, _)| *id);
            let Some(oldest) = oldest else { return false };
            self.entries.remove(&oldest);
        }
        true
    }

    /// The peaks of `asset` as it is now, when they are held.
    pub fn held(&self, asset: &Asset) -> Option<&Arc<Peaks>> {
        ready(&self.entries, asset)
    }
}

// The peaks of `asset` as it is now among `entries`, when they are held.
fn ready<'a>(entries: &'a HashMap<AssetId, Entry>, asset: &Asset) -> Option<&'a Arc<Peaks>> {
    let entry = entries.get(&asset.id)?;
    match &entry.known {
        Known::Ready(peaks) if is_of(&entry.source, asset) => Some(peaks),
        _ => None,
    }
}

// ---- Beats -----------------------------------------------------------------

/// The media files whose beat grids are held at once. One at the bound of
/// [`editor_media::MAX_BEATS`] is 64 KiB.
pub const MAX_HELD_GRIDS: usize = 8;

/// The most markers [`beat_markers`] writes for one clip.
pub const MAX_BEAT_MARKERS: usize = 256;

/// The label of a marker made from a beat, and of the step that makes them.
pub const BEAT_LABEL: &str = "Beat";
pub const BEAT_MARKERS: &str = "Markers from beats";

#[derive(Clone, Debug, PartialEq)]
enum Found {
    /// Shown by a tile and not held: asked of the lane once the cache keeps
    /// the media's peaks.
    Wanted,
    /// Asked: the request that answers.
    Asked(u64),
    Ready(Arc<BeatGrid>),
    /// The sound has no clear beat. Not asked again until shown again.
    Unclear,
    /// The beats could not be found. Not asked again until shown again.
    Failed,
}

struct Grid {
    /// The media the answer is about, as it was when it was wanted.
    source: SourceIdentity,
    found: Found,
    /// The frame a tile last showed it in.
    used: u64,
}

/// Where the beats of a media file stand, for a menu to say.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BeatState<'a> {
    /// Not shown, and none held.
    Hidden,
    /// Shown, and being found or waiting for the media's peaks.
    Finding,
    Held(&'a BeatGrid),
    /// Its sound has no clear beat.
    Unclear,
    Failed,
}

/// The beat grids the window holds, and the media whose beats are shown.
///
/// A grid is wanted by a tile that shows its media's beats, asked of the
/// lane once the peaks of that media are held (the lane finds beats in the
/// peaks its cache keeps), and taken only when it answers the request this
/// asked, for the project that is open, of the media as it is now. At most
/// [`MAX_HELD_GRIDS`] are held, the least lately shown dropped; a grid is not
/// asked for while as many are on screen or on their way.
#[derive(Default)]
pub struct HeldBeats {
    shown: HashSet<AssetId>,
    grids: HashMap<AssetId, Grid>,
    frame: u64,
    /// A grid was not asked for in this frame for want of room.
    refused: bool,
}

impl HeldBeats {
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.refused = false;
    }

    /// Whether a grid was not asked for this frame for want of room that
    /// the next frame may have: a held grid was on screen in the last frame
    /// and not in this one. Nothing else would bring that frame about.
    pub fn room_is_due(&self) -> bool {
        self.refused
            && (self.grids.values())
                .any(|grid| matches!(grid.found, Found::Ready(_)) && grid.used + 1 == self.frame)
    }

    /// Shows or hides the beats of `asset` for the rest of the session. A
    /// grid that is held stays held. Shown again, beats that were unclear or
    /// could not be found are asked for anew.
    pub fn show(&mut self, asset: AssetId, shown: bool) {
        if !shown {
            self.shown.remove(&asset);
            return;
        }
        self.shown.insert(asset);
        if (self.grids.get(&asset))
            .is_some_and(|grid| matches!(grid.found, Found::Unclear | Found::Failed))
        {
            self.grids.remove(&asset);
        }
    }

    /// Whether the beats of `asset` are shown.
    pub fn shown(&self, asset: AssetId) -> bool {
        self.shown.contains(&asset)
    }

    /// Whether the beats of any media are shown.
    pub fn any_shown(&self) -> bool {
        !self.shown.is_empty()
    }

    /// Says a tile in view shows the beats of `asset` this frame. Beats that
    /// are not held, or are held of what the media no longer is, are wanted.
    /// Beats that are not shown are not.
    pub fn want(&mut self, asset: &Asset) {
        if !self.shown.contains(&asset.id) {
            return;
        }
        let frame = self.frame;
        match self.grids.get_mut(&asset.id) {
            Some(grid) if is_of(&grid.source, asset) => grid.used = frame,
            _ => {
                self.grids.insert(
                    asset.id,
                    Grid {
                        source: SourceIdentity::of(asset),
                        found: Found::Wanted,
                        used: frame,
                    },
                );
            }
        }
    }

    /// Whether beats are wanted and not yet asked of the lane.
    pub fn wanting(&self) -> bool {
        self.grids.values().any(|grid| grid.found == Found::Wanted)
    }

    // Grids on their way, and those held that a tile showed in this frame
    // or the last, but that of `but`.
    fn claimed(&self, but: AssetId) -> usize {
        (self.grids.iter())
            .filter(|(id, grid)| {
                **id != but
                    && match grid.found {
                        Found::Asked(_) => true,
                        Found::Ready(_) => grid.used + 1 >= self.frame,
                        _ => false,
                    }
            })
            .count()
    }

    /// Asks the lane, through `ask`, for the beats wanted of media whose
    /// peaks are `kept`, each once, while there is room for their grids
    /// beside those on screen and on their way. Hands back, in words, the
    /// media the lane will not find beats in.
    pub fn ask_with(
        &mut self,
        project: &Project,
        kept: impl Fn(&Asset) -> bool,
        mut ask: impl FnMut(&Asset) -> Result<u64, MediaError>,
    ) -> Vec<String> {
        let wanted: Vec<AssetId> = (self.grids.iter())
            .filter(|(_, grid)| grid.found == Found::Wanted)
            .map(|(id, _)| *id)
            .collect();
        let mut refused = Vec::new();
        for id in wanted {
            let Some(asset) = project.assets.get(&id) else {
                continue;
            };
            if !kept(asset) {
                continue;
            }
            if self.claimed(id) >= MAX_HELD_GRIDS {
                self.refused = true;
                continue;
            }
            let found = match ask(asset) {
                Ok(request) => Found::Asked(request),
                // The lane is full: asked again in a later frame.
                Err(MediaError::Busy) => continue,
                Err(error) => {
                    refused.push(format!(
                        "The beats of {} were not found: {error}",
                        asset.name
                    ));
                    Found::Failed
                }
            };
            if let Some(grid) = self.grids.get_mut(&id) {
                grid.found = found;
            }
        }
        refused
    }

    /// The lane cannot be asked at all: what is wanted goes without, until
    /// it is shown again.
    pub fn give_up_wanted(&mut self) {
        for grid in self.grids.values_mut() {
            if grid.found == Found::Wanted {
                grid.found = Found::Failed;
            }
        }
    }

    /// Takes an answer of the lane. `instance` is the project that is open
    /// and `now` the asset as the project has it, if it still does. It is
    /// taken when it answers the request last put for its asset, by this
    /// project, of media that is still what was asked about; anything else
    /// changes nothing. Hands back, in words, beats that were not found.
    #[expect(
        clippy::too_many_arguments,
        reason = "what one answer carries, handed over as it is"
    )]
    pub fn take(
        &mut self,
        instance: Uuid,
        now: Option<&Asset>,
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        source: &SourceIdentity,
        result: Result<Option<BeatGrid>, String>,
    ) -> Option<String> {
        let asked = (self.grids.get(&asset_id))
            .is_some_and(|grid| grid.found == Found::Asked(request) && grid.source == *source);
        if !asked || project != instance {
            return None;
        }
        let Some(asset) = now.filter(|asset| is_of(source, asset)) else {
            // What it was asked about is no more: wanted again when shown.
            self.grids.remove(&asset_id);
            return None;
        };
        let (found, said) = match result {
            Ok(Some(grid)) => {
                self.make_room(asset_id);
                (Found::Ready(Arc::new(grid)), None)
            }
            Ok(None) => (
                Found::Unclear,
                Some(format!("No clear beat was found in {}.", asset.name)),
            ),
            Err(why) => {
                // The first line of what was said: the rest repeats it.
                let why = why.lines().next().unwrap_or_default();
                let said = format!("The beats of {} were not found: {why}", asset.name);
                (Found::Failed, Some(said))
            }
        };
        if let Some(grid) = self.grids.get_mut(&asset_id) {
            grid.found = found;
        }
        said
    }

    // Drops the grids least lately shown until one more beside them is at
    // most the bound; never that of `keep`.
    fn make_room(&mut self, keep: AssetId) {
        loop {
            let held = || {
                (self.grids.iter())
                    .filter(|(id, grid)| **id != keep && matches!(grid.found, Found::Ready(_)))
            };
            if held().count() < MAX_HELD_GRIDS {
                return;
            }
            let Some(oldest) = held().min_by_key(|(_, grid)| grid.used).map(|(id, _)| *id) else {
                return;
            };
            self.grids.remove(&oldest);
        }
    }

    /// The beats of `asset` as it is now, when they are held.
    pub fn grid(&self, asset: &Asset) -> Option<&BeatGrid> {
        match self.grids.get(&asset.id) {
            Some(Grid {
                source,
                found: Found::Ready(grid),
                ..
            }) if is_of(source, asset) => Some(grid),
            _ => None,
        }
    }

    /// Where the beats of `asset` as it is now stand.
    pub fn state(&self, asset: &Asset) -> BeatState<'_> {
        if let Some(grid) = self.grid(asset) {
            return BeatState::Held(grid);
        }
        if !self.shown(asset.id) {
            return BeatState::Hidden;
        }
        match self.grids.get(&asset.id) {
            Some(grid) if is_of(&grid.source, asset) => match grid.found {
                Found::Unclear => BeatState::Unclear,
                Found::Failed => BeatState::Failed,
                _ => BeatState::Finding,
            },
            _ => BeatState::Finding,
        }
    }

    /// The step that puts a marker on the beats of `clip`: see
    /// [`beat_markers`]. Refused in words while its media's beats are not
    /// held.
    pub fn markers(&self, project: &Project, clip: ClipId) -> Result<Command, String> {
        let (_, held) = project.clip(clip).ok_or("The clip no longer exists.")?;
        let asset = (held.asset_id())
            .and_then(|id| project.assets.get(&id))
            .ok_or("That clip has no sound to find beats in.")?;
        let name = &asset.name;
        match self.state(asset) {
            BeatState::Held(grid) => beat_markers(project, clip, grid),
            BeatState::Hidden => Err(format!("Show the beats of {name} first.")),
            BeatState::Finding => Err(format!("The beats of {name} are still being found.")),
            BeatState::Unclear => Err(format!("No clear beat was found in {name}.")),
            BeatState::Failed => Err(format!("The beats of {name} could not be found.")),
        }
    }

    /// Forgets the beats of an asset that left the library or is now of
    /// another file, and hands back the request still waited for, for the
    /// lane to be told. Whether they are shown is kept.
    pub fn forget_asset(&mut self, asset: AssetId) -> Option<u64> {
        match self.grids.remove(&asset)?.found {
            Found::Asked(request) => Some(request),
            _ => None,
        }
    }
}

/// Where the beats of `grid` fall on the timeline through `clip`, from
/// `from` up to `to`, earliest first. Only the stretches of the clip that
/// play its source straight forward carry them, as only those are heard: a
/// beat inside a freeze, a reverse or a speed ramp is not one. A beat is
/// where the clip plays the source time the grid gives it.
pub fn beats_through(clip: &Clip, grid: &BeatGrid, from: Time, to: Time) -> Vec<Time> {
    let mut times = Vec::new();
    let mut segment = 0;
    while let Some((start, end, next)) = sound::audible_run(clip, segment) {
        segment = next;
        let (low, high) = (start.max(from), end.min(to));
        if low >= high {
            continue;
        }
        let (first, last) = (clip.source_time(start), clip.source_time(end));
        let speed = (last - first).0 as f64 / (end - start).0 as f64;
        if speed <= 0.0 {
            continue;
        }
        let source = |at: Time| first.0 as f64 + (at - start).0 as f64 * speed;
        let millis = |at: Time| (source(at) / 1000.0).ceil().max(0.0) as u64;
        let (below, above) = (millis(low), millis(high));
        let beats = &grid.beats;
        let begin = beats.partition_point(|beat| u64::from(*beat) < below);
        let end_at = beats.partition_point(|beat| u64::from(*beat) < above);
        for beat in &beats[begin..end_at] {
            let offset = (i64::from(*beat) * 1000 - first.0) as f64 / speed;
            let at = start + Time(offset.round() as i64);
            if low <= at && at < high {
                times.push(at);
            }
        }
    }
    times
}

/// One step that puts a marker labelled [`BEAT_LABEL`] on the beats of
/// `clip`: every beat, or every second, fourth and so on, the fewest left
/// out for at most [`MAX_BEAT_MARKERS`]. They are the beats its tile shows
/// (see [`beats_through`]) over the whole clip, which is heard. A beat that
/// has a marker already is passed over.
pub fn beat_markers(project: &Project, clip: ClipId, grid: &BeatGrid) -> Result<Command, String> {
    let (track, held) = project.clip(clip).ok_or("The clip no longer exists.")?;
    if track.kind != TrackKind::Audio {
        return Err("Beats are shown on clips on audio tracks.".to_owned());
    }
    if !sound::sounds(project, track, held) {
        return Err("That clip is not heard, so it shows no beats.".to_owned());
    }
    let times = beats_through(held, grid, held.start, held.end());
    if times.is_empty() {
        return Err("No beat falls inside that clip where it plays forward.".to_owned());
    }
    let mut every = 1;
    while times.len().div_ceil(every) > MAX_BEAT_MARKERS {
        every *= 2;
    }
    let taken: std::collections::BTreeSet<Time> =
        project.markers.iter().map(|marker| marker.time).collect();
    let commands: Vec<Command> = (times.into_iter().step_by(every))
        .filter(|time| !taken.contains(time))
        .map(|time| Command::AddMarker {
            marker: Marker {
                id: Uuid::new_v4(),
                time,
                label: BEAT_LABEL.to_owned(),
                color: [255, 159, 10, 255],
            },
        })
        .collect();
    if commands.is_empty() {
        return Err("Every beat of that clip has a marker already.".to_owned());
    }
    Ok(Command::Batch {
        label: BEAT_MARKERS.to_owned(),
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{MediaKind, Time};

    fn sound(seconds: f64) -> Asset {
        let mut asset = Asset::new(
            "/media/voice.wav".into(),
            MediaKind::Audio,
            Time::from_seconds(seconds),
        );
        asset.file_size = 4321;
        asset
    }

    fn sound_named(name: &str, seconds: f64) -> Asset {
        let mut asset = sound(seconds);
        asset.path = format!("/media/{name}").into();
        asset.name = name.to_owned();
        asset
    }

    fn peaks(buckets: usize) -> Arc<Peaks> {
        Arc::new(Peaks {
            buckets: vec![[0xc0, 0x40, 200, 100]; buckets],
        })
    }

    const HERE: Uuid = Uuid::from_u128(1);
    const ELSEWHERE: Uuid = Uuid::from_u128(2);

    /// Wants `asset` in a frame of its own and has the lane asked.
    fn asked(held: &mut HeldPeaks, asset: &Asset, request: u64, ask: Ask) {
        held.begin_frame();
        held.want(asset);
        assert_eq!(held.take_wants(), vec![(asset.id, ask)]);
        held.asked(asset.id, request);
    }

    fn answer(held: &mut HeldPeaks, asset: &Asset, request: u64, answer: Answer) -> Taken {
        let source = SourceIdentity::of(asset);
        held.take(HERE, Some(asset), request, HERE, asset.id, &source, answer)
    }

    /// Has `asset` wanted, asked about and answered from the cache.
    fn hold(held: &mut HeldPeaks, asset: &Asset, request: u64, buckets: usize) -> Taken {
        asked(held, asset, request, Ask::Kept);
        let kept = Answer::Kept(KeptPeaks::Held(peaks(buckets)));
        answer(held, asset, request, kept)
    }

    #[test]
    fn a_tile_wanted_every_frame_asks_once() {
        let mut held = HeldPeaks::default();
        let asset = sound(4.0);
        assert!(held.held(&asset).is_none());
        held.begin_frame();
        held.want(&asset);
        held.want(&asset);
        assert_eq!(held.take_wants(), vec![(asset.id, Ask::Kept)]);
        held.asked(asset.id, 7);
        // Wanted again while the answer is awaited, it is not asked again.
        for _ in 0..3 {
            held.begin_frame();
            held.want(&asset);
            assert!(held.take_wants().is_empty());
        }
        // The cache keeps none: the sound is read, and that is asked once.
        let missing = Answer::Kept(KeptPeaks::Missing);
        assert_eq!(answer(&mut held, &asset, 7, missing), Taken::Unkept);
        assert!(held.take_wants().is_empty());
        held.begin_frame();
        held.want(&asset);
        held.want(&asset);
        assert_eq!(held.take_wants(), vec![(asset.id, Ask::Sound)]);
        held.asked(asset.id, 8);
        held.begin_frame();
        held.want(&asset);
        assert!(held.take_wants().is_empty());
        let read = Answer::Read(Ok(peaks(400)));
        assert_eq!(answer(&mut held, &asset, 8, read), Taken::Kept);
        assert_eq!(
            held.held(&asset).map(|peaks| peaks.buckets.len()),
            Some(400)
        );
        assert_eq!(held.bytes(), 1600);
        // Held, it is asked about no more.
        held.begin_frame();
        held.want(&asset);
        assert!(held.take_wants().is_empty());

        // Stale peaks in the cache are read anew as missing ones are.
        let other = sound(2.0);
        asked(&mut held, &other, 9, Ask::Kept);
        let stale = Answer::Kept(KeptPeaks::Stale);
        assert_eq!(answer(&mut held, &other, 9, stale), Taken::Unkept);
        held.want(&other);
        assert_eq!(held.take_wants(), vec![(other.id, Ask::Sound)]);

        // Media without sound has none to draw.
        let mut silent = sound(4.0);
        silent.kind = MediaKind::Video;
        silent.has_audio = false;
        held.want(&silent);
        assert!(held.take_wants().is_empty());
    }

    #[test]
    fn a_question_the_lane_had_no_room_for_is_put_again() {
        let mut held = HeldPeaks::default();
        let asset = sound(4.0);
        held.begin_frame();
        held.want(&asset);
        // Taken off the list and not asked: the lane was busy.
        assert_eq!(held.take_wants(), vec![(asset.id, Ask::Kept)]);
        assert!(held.take_wants().is_empty());
        held.begin_frame();
        held.want(&asset);
        assert_eq!(held.take_wants(), vec![(asset.id, Ask::Kept)]);
        held.asked(asset.id, 1);
        // And so is the reading of its sound.
        let missing = Answer::Kept(KeptPeaks::Missing);
        assert_eq!(answer(&mut held, &asset, 1, missing), Taken::Unkept);
        for _ in 0..2 {
            held.begin_frame();
            held.want(&asset);
            assert_eq!(held.take_wants(), vec![(asset.id, Ask::Sound)]);
        }
    }

    #[test]
    fn a_failed_asset_is_not_asked_about_again() {
        let mut held = HeldPeaks::default();
        let asset = sound(4.0);
        asked(&mut held, &asset, 1, Ask::Kept);
        answer(&mut held, &asset, 1, Answer::Kept(KeptPeaks::Missing));
        asked(&mut held, &asset, 2, Ask::Sound);
        let failed = Answer::Read(Err("ffmpeg: no sound could be read".to_owned()));
        assert_eq!(
            answer(&mut held, &asset, 2, failed),
            Taken::Failed("ffmpeg: no sound could be read".to_owned())
        );
        for _ in 0..3 {
            held.begin_frame();
            held.want(&asset);
            assert!(held.take_wants().is_empty());
        }
        assert!(held.held(&asset).is_none());

        // Nor is one the lane refused outright, as too long.
        let long = sound(30_000.0);
        held.begin_frame();
        held.want(&long);
        assert_eq!(held.take_wants(), vec![(long.id, Ask::Kept)]);
        held.refuse(long.id);
        held.begin_frame();
        held.want(&long);
        assert!(held.take_wants().is_empty());

        // Relinked, the media is another: it is asked about anew.
        let mut relinked = asset.clone();
        relinked.path = "/media/other.wav".into();
        held.want(&relinked);
        assert_eq!(held.take_wants(), vec![(asset.id, Ask::Kept)]);
        // And so is media that was forgotten.
        assert_eq!(held.forget_asset(long.id), None);
        held.want(&long);
        assert_eq!(held.take_wants(), vec![(long.id, Ask::Kept)]);
    }

    #[test]
    fn an_answer_is_taken_only_for_this_project_this_request_and_this_media() {
        let asset = sound(4.0);
        let source = SourceIdentity::of(&asset);
        let kept = || Answer::Kept(KeptPeaks::Held(peaks(400)));
        let read = || Answer::Read(Ok(peaks(400)));
        let mut held = HeldPeaks::default();
        asked(&mut held, &asset, 3, Ask::Kept);

        // Another request, another project, and an answer of the other kind
        // under the same number.
        assert_eq!(answer(&mut held, &asset, 2, kept()), Taken::Dropped);
        let taken = held.take(HERE, Some(&asset), 3, ELSEWHERE, asset.id, &source, kept());
        assert_eq!(taken, Taken::Dropped);
        assert_eq!(answer(&mut held, &asset, 3, read()), Taken::Dropped);
        assert!(held.held(&asset).is_none());
        // None of them was the answer: that is still waited for.
        held.begin_frame();
        held.want(&asset);
        assert!(held.take_wants().is_empty());

        // The media was relinked while the cache was read: the peaks read
        // are of what it was, and the media as it is now is asked about anew.
        let mut relinked = asset.clone();
        relinked.file_size += 1;
        let taken = held.take(HERE, Some(&relinked), 3, HERE, asset.id, &source, kept());
        assert_eq!(taken, Taken::Dropped);
        assert!(held.held(&relinked).is_none());
        held.want(&relinked);
        assert_eq!(held.take_wants(), vec![(asset.id, Ask::Kept)]);
        held.asked(asset.id, 4);
        // The old answer comes late, after the new question.
        assert_eq!(answer(&mut held, &asset, 3, kept()), Taken::Dropped);
        assert_eq!(answer(&mut held, &relinked, 4, kept()), Taken::Kept);
        // Peaks held are of the media they were read of.
        assert!(held.held(&relinked).is_some());
        assert!(held.held(&asset).is_none());

        // Media that left the library takes nothing, and the request it
        // waited for is handed back to be given up.
        let gone = sound(2.0);
        asked(&mut held, &gone, 5, Ask::Kept);
        let source = SourceIdentity::of(&gone);
        let taken = held.take(HERE, None, 5, HERE, gone.id, &source, kept());
        assert_eq!(taken, Taken::Dropped);
        asked(&mut held, &gone, 6, Ask::Kept);
        assert_eq!(held.forget_asset(gone.id), Some(6));
        assert_eq!(answer(&mut held, &gone, 6, kept()), Taken::Dropped);
        assert!(held.held(&gone).is_none());

        // Another project: nothing is held, and what was asked answers
        // nothing.
        asked(&mut held, &gone, 7, Ask::Kept);
        held.clear();
        assert_eq!(answer(&mut held, &gone, 7, kept()), Taken::Dropped);
        assert!(held.held(&relinked).is_none());
        assert_eq!(held.bytes(), 0);
    }

    #[test]
    fn the_peaks_held_keep_their_byte_bound() {
        // Each of these is a quarter of the bound and a little more.
        let buckets = MAX_HELD_PEAK_BYTES / 16 + 1;
        let seconds = buckets as f64 / f64::from(PEAKS_PER_SECOND);
        let assets: Vec<Asset> = (0..6).map(|_| sound(seconds)).collect();
        let mut held = HeldPeaks::default();
        for (index, asset) in assets.iter().enumerate() {
            // Each in a frame of its own, the tiles before it scrolled away.
            assert_eq!(hold(&mut held, asset, index as u64, buckets), Taken::Kept);
            assert!(held.bytes() <= MAX_HELD_PEAK_BYTES);
            // The first is wanted in every frame.
            held.want(&assets[0]);
        }
        assert_eq!(held.bytes(), 3 * buckets * 4);
        assert!(held.held(&assets[0]).is_some());
        // The least lately wanted made room.
        assert!(held.held(&assets[1]).is_none());
        assert!(held.held(&assets[3]).is_none());
        assert!(held.held(&assets[4]).is_some());
        assert!(held.held(&assets[5]).is_some());
    }

    #[test]
    fn peaks_with_no_room_beside_those_on_screen_wait_for_it() {
        let buckets = MAX_HELD_PEAK_BYTES / 16 + 1;
        let seconds = buckets as f64 / f64::from(PEAKS_PER_SECOND);
        let assets: Vec<Asset> = (0..5).map(|_| sound(seconds)).collect();
        let mut held = HeldPeaks::default();
        // Three tiles fill the room, all on screen.
        held.begin_frame();
        for asset in &assets[..3] {
            held.want(asset);
        }
        for (request, (id, _)) in held.take_wants().into_iter().enumerate() {
            held.asked(id, request as u64);
        }
        for (request, asset) in assets[..3].iter().enumerate() {
            let kept = Answer::Kept(KeptPeaks::Held(peaks(buckets)));
            assert_eq!(answer(&mut held, asset, request as u64, kept), Taken::Kept);
        }
        // A fourth beside them is not asked about, frame after frame, and
        // nothing asks for another frame.
        for _ in 0..3 {
            held.begin_frame();
            for asset in &assets[..4] {
                held.want(asset);
            }
            assert!(held.take_wants().is_empty());
            assert!(!held.room_is_due());
            assert_eq!(held.bytes(), 3 * buckets * 4);
        }
        // The first scrolls away: the frame it is first missing from still
        // counts it, and asks for the frame that does not.
        held.begin_frame();
        for asset in &assets[1..4] {
            held.want(asset);
        }
        assert!(held.take_wants().is_empty());
        assert!(held.room_is_due());
        held.begin_frame();
        for asset in &assets[1..4] {
            held.want(asset);
        }
        assert_eq!(held.take_wants(), vec![(assets[3].id, Ask::Kept)]);
        assert!(!held.room_is_due());
        held.asked(assets[3].id, 9);
        let kept = Answer::Kept(KeptPeaks::Held(peaks(buckets)));
        assert_eq!(answer(&mut held, &assets[3], 9, kept), Taken::Kept);
        assert!(held.held(&assets[0]).is_none());
        assert_eq!(held.bytes(), 3 * buckets * 4);

        // Sound that proves longer than its length said has no room beside
        // what is on screen: it is not held, and is asked about again only
        // when that much is free.
        let short = sound(1.0);
        held.begin_frame();
        for asset in &assets[1..4] {
            held.want(asset);
        }
        held.want(&short);
        assert_eq!(held.take_wants(), vec![(short.id, Ask::Kept)]);
        held.asked(short.id, 10);
        let long = Answer::Kept(KeptPeaks::Held(peaks(buckets)));
        assert_eq!(answer(&mut held, &short, 10, long), Taken::NoRoom);
        assert_eq!(held.bytes(), 3 * buckets * 4);
        held.begin_frame();
        for asset in &assets[1..4] {
            held.want(asset);
        }
        held.want(&short);
        assert!(held.take_wants().is_empty());
        // Two frames with one tile fewer, and it is.
        for _ in 0..2 {
            held.begin_frame();
            for asset in &assets[2..4] {
                held.want(asset);
            }
            held.want(&short);
        }
        assert_eq!(held.take_wants(), vec![(short.id, Ask::Kept)]);
    }

    #[test]
    fn peaks_turned_away_for_want_of_room_turn_away_none_that_fit() {
        // Each of these is 7 MiB: four leave room for less than a fifth.
        let buckets = (7 << 20) / 4;
        let seconds = buckets as f64 / f64::from(PEAKS_PER_SECOND);
        let assets: Vec<Asset> = (0..6).map(|_| sound(seconds)).collect();
        let short = sound(1.0);
        let mut held = HeldPeaks::default();
        // Four tiles on screen; a fifth as long has no room, and a short
        // sound drawn after it in the same frame has, frame after frame.
        for _ in 0..3 {
            held.begin_frame();
            for asset in &assets[..5] {
                held.want(asset);
            }
            held.want(&short);
            // And one as long as the one turned away, drawn after both.
            held.want(&assets[5]);
            let wants: Vec<AssetId> = held.take_wants().into_iter().map(|(id, _)| id).collect();
            assert_eq!(wants.len(), 5);
            assert!(wants.contains(&short.id), "the short sound was turned away");
            assert!(!wants.contains(&assets[4].id) && !wants.contains(&assets[5].id));
            assert!(!held.room_is_due());
        }
        // The first scrolls away: the long ones turned away are still due
        // their frame, whatever the short one was given.
        held.begin_frame();
        for asset in &assets[1..5] {
            held.want(asset);
        }
        held.want(&short);
        assert!(held.room_is_due());
    }

    #[test]
    fn sound_read_for_a_tile_no_longer_drawn_is_given_up_and_read_when_it_is_again() {
        let mut held = HeldPeaks::default();
        let (away, stays) = (sound(4.0), sound(2.0));
        for (request, asset) in [(1, &away), (2, &stays)] {
            asked(&mut held, asset, request, Ask::Kept);
            let missing = Answer::Kept(KeptPeaks::Missing);
            assert_eq!(answer(&mut held, asset, request, missing), Taken::Unkept);
        }
        held.begin_frame();
        held.want(&away);
        held.want(&stays);
        for (request, (id, ask)) in held.take_wants().into_iter().enumerate() {
            assert_eq!(ask, Ask::Sound);
            held.asked(id, 3 + request as u64);
        }
        // Drawn, and in the frame it is first missing from, it is read on.
        assert!(held.take_unwanted().is_empty());
        held.begin_frame();
        held.want(&stays);
        assert!(held.take_unwanted().is_empty());
        // Two frames undrawn: its request is handed back, once, and that of
        // the tile still drawn is not.
        held.begin_frame();
        held.want(&stays);
        assert_eq!(held.take_unwanted(), vec![3]);
        assert!(held.take_unwanted().is_empty());
        assert!(!held.wanting());
        // What the request given up answers is taken by no one.
        let read = Answer::Read(Ok(peaks(400)));
        assert_eq!(answer(&mut held, &away, 3, read), Taken::Dropped);
        assert!(held.held(&away).is_none());
        for _ in 0..3 {
            held.begin_frame();
            held.want(&stays);
            assert!(held.take_unwanted().is_empty());
            assert!(held.take_wants().is_empty());
        }
        // Drawn again, its sound is asked for again, and not the cache.
        held.begin_frame();
        held.want(&away);
        held.want(&stays);
        assert_eq!(held.take_wants(), vec![(away.id, Ask::Sound)]);
        held.asked(away.id, 5);
        let read = Answer::Read(Ok(peaks(400)));
        assert_eq!(answer(&mut held, &away, 5, read), Taken::Kept);
        let read = Answer::Read(Ok(peaks(200)));
        assert_eq!(answer(&mut held, &stays, 4, read), Taken::Kept);

        // A question to the cache is answered at once and is not given up.
        let other = sound(3.0);
        asked(&mut held, &other, 6, Ask::Kept);
        held.begin_frame();
        held.begin_frame();
        assert!(held.take_unwanted().is_empty());
        assert_eq!(held.forget_asset(other.id), Some(6));
    }

    #[test]
    fn work_is_read_unseen_and_takes_its_room_from_tiles_until_it_lets_go() {
        // Twenty seconds each, 8,000 bytes: room for two.
        let bound = 2 * 8_000;
        let mut held = HeldPeaks::bounded(bound);
        let one = sound_named("one.wav", 20.0);
        let two = sound_named("two.wav", 20.0);
        let work = sound_named("work.wav", 20.0);
        for (request, asset) in [(1, &one), (2, &two)] {
            held.begin_frame();
            held.want(&one);
            held.want(&two);
            held.take_wants();
            held.asked(asset.id, request);
            let kept = Answer::Kept(KeptPeaks::Held(peaks(2_000)));
            assert_eq!(answer(&mut held, asset, request, kept), Taken::Kept);
        }
        assert_eq!(held.bytes(), bound);

        // Work wants a third: asked whatever the room, once.
        held.begin_frame();
        held.want(&one);
        held.want(&two);
        held.want_work(&work);
        held.want_work(&work);
        assert_eq!(held.take_wants(), vec![(work.id, Ask::Kept)]);
        held.asked(work.id, 3);
        // No tile draws it, and it is read on all the same.
        for _ in 0..3 {
            held.begin_frame();
            held.want(&one);
            held.want(&two);
            assert!(held.take_unwanted().is_empty());
        }
        // The cache keeps none: the sound is asked for without a tile.
        let missing = Answer::Kept(KeptPeaks::Missing);
        assert_eq!(answer(&mut held, &work, 3, missing), Taken::Unkept);
        assert_eq!(held.take_wants(), vec![(work.id, Ask::Sound)]);
        held.asked(work.id, 4);
        // It comes, and takes the room of the tile least lately wanted,
        // though that one is on screen.
        held.begin_frame();
        held.want(&one);
        held.begin_frame();
        held.want(&two);
        let read = Answer::Read(Ok(peaks(2_000)));
        assert_eq!(answer(&mut held, &work, 4, read), Taken::Kept);
        assert!(held.held(&work).is_some() && held.held(&one).is_none());
        assert_eq!(held.bytes(), bound);
        // That tile has no room while the work holds it, and asks nothing.
        held.begin_frame();
        held.want(&one);
        held.want(&two);
        assert!(held.take_wants().is_empty());
        assert!(!held.room_is_due());
        // Let go, the work's peaks go, and the tile is asked for again.
        assert_eq!(held.let_go(work.id), None);
        assert!(held.held(&work).is_none());
        assert_eq!(held.bytes(), 8_000);
        held.begin_frame();
        held.want(&one);
        held.want(&two);
        assert_eq!(held.take_wants(), vec![(one.id, Ask::Kept)]);

        // Work on what a tile shows reads the peaks held, asks nothing,
        // and leaves them to the tile when it lets go.
        held.want_work(&two);
        assert!(!held.wanting());
        assert!(held.held(&two).is_some());
        assert_eq!(held.let_go(two.id), None);
        assert!(held.held(&two).is_some());
        assert!(!held.failed(&two));
    }

    #[test]
    fn sound_work_could_not_read_stays_failed_once_it_is_let_go() {
        let mut held = HeldPeaks::default();
        let broken = sound_named("broken.wav", 4.0);
        held.begin_frame();
        held.want_work(&broken);
        assert_eq!(held.take_wants(), vec![(broken.id, Ask::Kept)]);
        held.asked(broken.id, 1);
        let missing = Answer::Kept(KeptPeaks::Missing);
        assert_eq!(answer(&mut held, &broken, 1, missing), Taken::Unkept);
        assert_eq!(held.take_wants(), vec![(broken.id, Ask::Sound)]);
        held.asked(broken.id, 2);
        let failed = Answer::Read(Err("ffmpeg: no sound could be read".to_owned()));
        assert!(matches!(
            answer(&mut held, &broken, 2, failed),
            Taken::Failed(_)
        ));
        assert!(held.failed(&broken));
        // Work leaves it out and lets go; no tile showed it.
        assert_eq!(held.let_go(broken.id), None);
        assert!(held.failed(&broken));
        // A tile that shows it later asks nothing, so nothing is said again.
        for _ in 0..2 {
            held.begin_frame();
            held.want(&broken);
            assert!(held.take_wants().is_empty());
        }
        // Nor does work that wants it again.
        held.want_work(&broken);
        assert!(!held.wanting());
        assert_eq!(held.let_go(broken.id), None);
        assert!(held.failed(&broken));
        // Relinked, the media is another and is asked about anew.
        let mut relinked = broken.clone();
        relinked.path = "/media/fixed.wav".into();
        held.want(&relinked);
        assert_eq!(held.take_wants(), vec![(broken.id, Ask::Kept)]);
    }

    #[test]
    fn media_is_told_from_what_it_was_by_all_that_a_source_is() {
        let mut asset = sound(4.0);
        asset.width = 1920;
        asset.height = 1080;
        asset.frame_rate = Some(editor_core::FrameRate::default());
        let source = SourceIdentity::of(&asset);
        assert!(is_of(&source, &asset));
        let changes: [fn(&mut Asset); 6] = [
            |asset| asset.path = "/media/other.wav".into(),
            |asset| asset.file_size += 1,
            |asset| asset.duration = Time::from_seconds(5.0),
            |asset| asset.width = 1280,
            |asset| asset.height = 720,
            |asset| asset.frame_rate = None,
        ];
        for change in changes {
            let mut other = asset.clone();
            change(&mut other);
            assert_ne!(SourceIdentity::of(&other), source);
            assert!(!is_of(&source, &other), "{other:?}");
        }
        // What is not of the source changes nothing.
        let mut named = asset.clone();
        named.name = "Another name".to_owned();
        assert_eq!(SourceIdentity::of(&named), source);
        assert!(is_of(&source, &named));
    }

    // ---- Beats -----------------------------------------------------------

    /// A grid of a beat every half second from `first` ms, `count` long.
    fn grid(first: u32, count: u32) -> BeatGrid {
        BeatGrid {
            bpm: 120.0,
            beats: (0..count).map(|index| first + index * 500).collect(),
        }
    }

    /// Shows the beats of `asset`, has a tile want them in a frame of its
    /// own, asks the lane as `request` and answers with `answer`.
    fn found(
        held: &mut HeldBeats,
        asset: &Asset,
        request: u64,
        answer: Result<Option<BeatGrid>, String>,
    ) -> Option<String> {
        held.begin_frame();
        held.show(asset.id, true);
        held.want(asset);
        let mut asked = Vec::new();
        let project = library(&[asset]);
        let refused = held.ask_with(
            &project,
            |_| true,
            |asset| {
                asked.push(asset.id);
                Ok(request)
            },
        );
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(asked, [asset.id], "the beats were not asked for");
        let source = SourceIdentity::of(asset);
        held.take(HERE, Some(asset), request, HERE, asset.id, &source, answer)
    }

    #[test]
    fn beats_shown_are_asked_for_once_their_peaks_are_kept_and_taken_only_as_asked() {
        let mut held = HeldBeats::default();
        let song = sound_named("song.wav", 30.0);
        let project = library(&[&song]);
        let source = SourceIdentity::of(&song);
        let mut asked: Vec<AssetId> = Vec::new();
        // Not shown, a tile wants nothing.
        held.begin_frame();
        held.want(&song);
        assert!(!held.wanting());
        assert_eq!(held.state(&song), BeatState::Hidden);

        // Shown, they wait for the peaks, then are asked for once however
        // many frames want them.
        held.show(song.id, true);
        held.want(&song);
        assert!(held.wanting());
        assert_eq!(held.state(&song), BeatState::Finding);
        held.ask_with(
            &project,
            |_| false,
            |asset| {
                asked.push(asset.id);
                Ok(1)
            },
        );
        assert!(asked.is_empty() && held.wanting());
        // The lane is full: asked again later.
        held.ask_with(&project, |_| true, |_| Err(MediaError::Busy));
        assert!(held.wanting());
        for _ in 0..3 {
            held.begin_frame();
            held.want(&song);
            held.ask_with(
                &project,
                |_| true,
                |asset| {
                    asked.push(asset.id);
                    Ok(2)
                },
            );
        }
        assert_eq!(asked, [song.id]);
        assert!(!held.wanting());

        // Another request, another project, other media: dropped.
        let beats = || Ok(Some(grid(250, 40)));
        assert_eq!(
            held.take(HERE, Some(&song), 1, HERE, song.id, &source, beats()),
            None
        );
        assert_eq!(
            held.take(HERE, Some(&song), 2, ELSEWHERE, song.id, &source, beats()),
            None
        );
        let mut other = song.clone();
        other.file_size += 1;
        let elsewhere = SourceIdentity::of(&other);
        assert_eq!(
            held.take(HERE, Some(&song), 2, HERE, song.id, &elsewhere, beats()),
            None
        );
        assert!(held.grid(&song).is_none());
        // The answer to the request: held, and asked about no more.
        assert_eq!(
            held.take(HERE, Some(&song), 2, HERE, song.id, &source, beats()),
            None
        );
        assert_eq!(held.grid(&song), Some(&grid(250, 40)));
        assert!(matches!(held.state(&song), BeatState::Held(_)));
        held.begin_frame();
        held.want(&song);
        assert!(!held.wanting());
        // Hidden, the grid stays for when they are shown again.
        held.show(song.id, false);
        assert!(!held.shown(song.id) && held.grid(&song).is_some());
        // The media is relinked: what was held is of what it was.
        assert!(held.grid(&other).is_none());
        held.show(song.id, true);
        held.want(&other);
        assert!(held.wanting());
        assert_eq!(held.forget_asset(song.id), None);
        assert!(!held.wanting());
    }

    #[test]
    fn beats_that_are_unclear_or_not_found_are_said_once_and_asked_again_when_shown_again() {
        let mut held = HeldBeats::default();
        let steady = sound_named("steady.wav", 30.0);
        assert_eq!(
            found(&mut held, &steady, 1, Ok(None)),
            Some("No clear beat was found in steady.wav.".to_owned())
        );
        assert_eq!(held.state(&steady), BeatState::Unclear);
        held.begin_frame();
        held.want(&steady);
        assert!(!held.wanting());
        // Shown again, they are asked anew.
        let gone = sound_named("gone.wav", 30.0);
        let why = "The media's sound has not been read\nmore".to_owned();
        assert_eq!(
            found(&mut held, &gone, 2, Err(why)),
            Some(
                "The beats of gone.wav were not found: The media's sound has not been read"
                    .to_owned()
            )
        );
        assert_eq!(held.state(&gone), BeatState::Failed);
        assert_eq!(found(&mut held, &gone, 3, Ok(Some(grid(0, 8)))), None);
        assert!(held.grid(&gone).is_some());

        // Media the lane refuses outright is said, and not asked again.
        let long = sound_named("long.wav", 30.0);
        let project = library(&[&long]);
        held.begin_frame();
        held.show(long.id, true);
        held.want(&long);
        let refused = held.ask_with(
            &project,
            |_| true,
            |_| {
                Err(MediaError::Invalid(
                    "Media is too long to find beats in".into(),
                ))
            },
        );
        assert_eq!(
            refused,
            ["The beats of long.wav were not found: Media is too long to find beats in"]
        );
        assert!(!held.wanting());
        // So is what is wanted when the lane cannot be started.
        let more = sound_named("more.wav", 30.0);
        held.show(more.id, true);
        held.want(&more);
        held.give_up_wanted();
        assert_eq!(held.state(&more), BeatState::Failed);
        assert!(!held.wanting());
    }

    #[test]
    fn a_ninth_grid_drops_the_one_least_lately_shown_and_waits_while_eight_are_on_screen() {
        let assets: Vec<Asset> = (0..10)
            .map(|index| sound_named(&format!("song{index}.wav"), 30.0))
            .collect();
        let mut held = HeldBeats::default();
        // Eight, each in a frame of its own; the first is shown in every one.
        for (index, asset) in assets[..8].iter().enumerate() {
            assert_eq!(
                found(&mut held, asset, index as u64, Ok(Some(grid(0, 8)))),
                None
            );
            held.want(&assets[0]);
        }
        assert!(assets[..8].iter().all(|asset| held.grid(asset).is_some()));
        // A ninth: the least lately shown goes, and the first stays.
        assert_eq!(found(&mut held, &assets[8], 8, Ok(Some(grid(0, 8)))), None);
        assert!(held.grid(&assets[8]).is_some() && held.grid(&assets[0]).is_some());
        assert!(held.grid(&assets[1]).is_none());
        let kept = assets
            .iter()
            .filter(|asset| held.grid(asset).is_some())
            .count();
        assert_eq!(kept, MAX_HELD_GRIDS);

        // With all eight on screen a tenth waits, frame after frame.
        let project = library(&assets.iter().collect::<Vec<_>>());
        let on_screen: Vec<&Asset> = (assets.iter())
            .filter(|asset| held.grid(asset).is_some())
            .collect();
        held.show(assets[9].id, true);
        let mut asked = 0;
        for _ in 0..3 {
            held.begin_frame();
            on_screen.iter().for_each(|asset| held.want(asset));
            held.want(&assets[9]);
            held.ask_with(
                &project,
                |_| true,
                |_| {
                    asked += 1;
                    Ok(20)
                },
            );
            // No room will come of another frame: none is asked for.
            assert!(!held.room_is_due());
        }
        assert_eq!(asked, 0);
        // Two frames with one of them away, and it is asked for. The first
        // still counts the one that left, and has the second brought about.
        for frame in 0..2 {
            held.begin_frame();
            on_screen[1..].iter().for_each(|asset| held.want(asset));
            held.want(&assets[9]);
            held.ask_with(
                &project,
                |_| true,
                |_| {
                    asked += 1;
                    Ok(20)
                },
            );
            assert_eq!(held.room_is_due(), frame == 0);
            // The window asks the peaks it holds, which hold the beats.
            let peaks = HeldPeaks {
                beats: std::mem::take(&mut held),
                ..HeldPeaks::default()
            };
            assert_eq!(peaks.room_is_due(), frame == 0);
            held = peaks.beats;
        }
        assert_eq!(asked, 1);
        let source = SourceIdentity::of(&assets[9]);
        let answer = Ok(Some(grid(0, 8)));
        assert_eq!(
            held.take(
                HERE,
                Some(&assets[9]),
                20,
                HERE,
                assets[9].id,
                &source,
                answer
            ),
            None
        );
        assert!(held.grid(&assets[9]).is_some() && held.grid(on_screen[0]).is_none());
    }

    #[test]
    fn a_project_replaced_shows_no_beats_and_replaced_media_is_asked_again() {
        let mut held = HeldPeaks::default();
        let song = sound_named("song.wav", 30.0);
        assert_eq!(found(&mut held.beats, &song, 1, Ok(Some(grid(0, 8)))), None);
        held.clear();
        assert!(!held.beats.shown(song.id) && held.beats.grid(&song).is_none());
        // Replaced media keeps its beats shown and is asked about anew; an
        // answer to what was asked of it before is for nothing.
        held.beats.show(song.id, true);
        held.begin_frame();
        held.beats.want(&song);
        held.beats.ask_with(&library(&[&song]), |_| true, |_| Ok(5));
        assert!(!held.wanting());
        held.media_replaced(song.id, None);
        assert!(held.beats.shown(song.id));
        assert_eq!(held.beats.state(&song), BeatState::Finding);
        let source = SourceIdentity::of(&song);
        let late = held.beats.take(
            HERE,
            Some(&song),
            5,
            HERE,
            song.id,
            &source,
            Ok(Some(grid(0, 8))),
        );
        assert_eq!(late, None);
        assert!(held.beats.grid(&song).is_none());
        held.begin_frame();
        held.beats.want(&song);
        assert!(held.wanting());
    }

    /// A clip of a minute of `song` from `start`, on an audio track of its
    /// own, in a project that holds that media.
    fn on_a_track(song: &Asset, start: f64) -> (Project, ClipId) {
        let mut project = library(&[song]);
        project.tracks.clear();
        let mut clip = Clip::from_asset(song, Time::from_seconds(start));
        clip.duration = song.duration;
        let id = clip.id;
        let mut track = editor_core::Track::new("Music", TrackKind::Audio);
        track.clips.push(clip);
        project.tracks.push(Arc::new(track));
        (project, id)
    }

    #[test]
    fn beats_fall_through_the_clip_where_it_plays_forward() {
        let song = sound_named("song.wav", 60.0);
        let beats = grid(250, 100);
        let mut clip = Clip::from_asset(&song, Time::from_seconds(10.0));
        clip.duration = Time::from_seconds(4.0);
        let all = |clip: &Clip| beats_through(clip, &beats, Time::ZERO, Time::from_seconds(100.0));
        let seconds =
            |times: Vec<Time>| -> Vec<f64> { times.into_iter().map(Time::as_seconds).collect() };
        assert_eq!(
            seconds(all(&clip)),
            [10.25, 10.75, 11.25, 11.75, 12.25, 12.75, 13.25, 13.75]
        );
        // Only those from `from` up to `to`.
        let some = beats_through(
            &clip,
            &beats,
            Time::from_seconds(10.75),
            Time::from_seconds(11.75),
        );
        assert_eq!(seconds(some), [10.75, 11.25]);
        // Trimmed, it shows only the beats of the source it plays.
        clip.source_in = Time::from_seconds(1.3);
        clip.duration = Time::from_seconds(2.0);
        let trimmed = seconds(all(&clip));
        let expected = [10.45, 10.95, 11.45, 11.95];
        assert!(
            trimmed.len() == 4
                && trimmed
                    .iter()
                    .zip(expected)
                    .all(|(at, want)| (at - want).abs() < 1e-6),
            "{trimmed:?}"
        );
        // At twice the speed, at half the spacing.
        clip.source_in = Time::ZERO;
        clip.duration = Time::from_seconds(4.0);
        clip.playback_rate = 2.0;
        let fast = seconds(all(&clip));
        assert_eq!(fast.len(), 16);
        assert!(
            fast.windows(2)
                .all(|pair| (pair[1] - pair[0] - 0.25).abs() < 1e-6),
            "{fast:?}"
        );
        // A freeze, a reverse and a ramp carry none.
        clip.playback_rate = 1.0;
        let key = |at: f64, source: f64, ease| editor_core::TimeKey {
            at: Time::from_seconds(at),
            source: Time::from_seconds(source),
            ease,
        };
        clip.retime = Some(editor_core::Retime {
            keys: vec![
                key(0.0, 0.0, editor_core::Ease::Linear),
                key(1.0, 1.0, editor_core::Ease::Linear),
                key(2.0, 1.0, editor_core::Ease::Linear),
                key(3.0, 0.0, editor_core::Ease::Linear),
                key(4.0, 1.0, editor_core::Ease::Linear),
            ],
        });
        assert_eq!(seconds(all(&clip)), [10.25, 10.75, 13.25, 13.75]);
        clip.retime.as_mut().unwrap().keys[3].ease = editor_core::Ease::Bezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        assert_eq!(seconds(all(&clip)), [10.25, 10.75]);
    }

    #[test]
    fn a_thousand_beats_are_marked_at_most_256_times_in_one_step() {
        let song = sound_named("song.wav", 1200.0);
        let (project, clip) = on_a_track(&song, 0.0);
        let markers = |count: u32| -> Vec<Time> {
            match beat_markers(&project, clip, &grid(250, count)) {
                Ok(Command::Batch { label, commands }) => {
                    assert_eq!(label, BEAT_MARKERS);
                    (commands.into_iter())
                        .map(|command| match command {
                            Command::AddMarker { marker } => {
                                assert_eq!(marker.label, BEAT_LABEL);
                                marker.time
                            }
                            other => panic!("{other:?}"),
                        })
                        .collect()
                }
                other => panic!("{other:?}"),
            }
        };
        // Every beat, every second, every fourth, and on.
        for (count, every, written) in [
            (256, 1, 256),
            (257, 2, 129),
            (512, 2, 256),
            (1000, 4, 250),
            (2000, 8, 250),
        ] {
            let times = markers(count);
            assert_eq!(times.len(), written, "{count}");
            assert!(times.len() <= MAX_BEAT_MARKERS);
            let step = Time::from_seconds(0.5 * f64::from(every));
            assert!(
                times.windows(2).all(|pair| pair[1] - pair[0] == step),
                "{count}"
            );
            assert_eq!(times[0], Time::from_seconds(0.25));
        }

        // Written, they are one step of the history, and Undo takes them back.
        let mut editor = editor_core::Editor::new(project.clone()).unwrap();
        let command = beat_markers(&project, clip, &grid(250, 1000)).unwrap();
        editor.dispatch(command).unwrap();
        assert_eq!(editor.history().count(), 1);
        assert_eq!(editor.undo_label(), Some(BEAT_MARKERS));
        assert_eq!(editor.project().markers.len(), 250);
        // Asked again, every beat it would mark has one: nothing is written.
        let again = beat_markers(editor.project(), clip, &grid(250, 1000)).unwrap_err();
        assert_eq!(again, "Every beat of that clip has a marker already.");
        assert!(editor.undo());
        assert!(editor.project().markers.is_empty());
    }

    #[test]
    fn beat_markers_are_saved_and_read_back_as_written() {
        let song = sound_named("song.wav", 60.0);
        let (project, clip) = on_a_track(&song, 2.0);
        let mut editor = editor_core::Editor::new(project).unwrap();
        let command = beat_markers(editor.project(), clip, &grid(250, 100)).unwrap();
        editor.dispatch(command).unwrap();
        let scratch = Scratch::new();
        let file = scratch.dir.join("Beats.aster");
        editor_store::save_project(&file, editor.project()).unwrap();
        let loaded = editor_store::load_project(&file).unwrap();
        assert_eq!(loaded.markers, editor.project().markers);
        assert_eq!(loaded.markers.len(), 100);
        assert_eq!(loaded.markers[0].time, Time::from_seconds(2.25));
        assert!(
            loaded
                .markers
                .iter()
                .all(|marker| marker.label == BEAT_LABEL)
        );
    }

    #[test]
    fn markers_from_beats_are_refused_in_words_where_no_beat_is_shown() {
        let song = sound_named("song.wav", 60.0);
        let (project, clip) = on_a_track(&song, 0.0);
        let mut held = HeldBeats::default();
        assert_eq!(
            held.markers(&project, clip).unwrap_err(),
            "Show the beats of song.wav first."
        );
        held.show(song.id, true);
        assert_eq!(
            held.markers(&project, clip).unwrap_err(),
            "The beats of song.wav are still being found."
        );
        assert_eq!(found(&mut held, &song, 1, Ok(Some(grid(250, 100)))), None);
        assert!(held.markers(&project, clip).is_ok());
        // A beat past the media's sound, or a clip that is not heard, has none.
        assert_eq!(
            beat_markers(&project, clip, &grid(70_000, 4)).unwrap_err(),
            "No beat falls inside that clip where it plays forward."
        );
        let mut muted = project.clone();
        Arc::make_mut(&mut muted.tracks[0]).muted = true;
        assert_eq!(
            held.markers(&muted, clip).unwrap_err(),
            "That clip is not heard, so it shows no beats."
        );
        assert_eq!(
            held.markers(&project, Uuid::new_v4()).unwrap_err(),
            "The clip no longer exists."
        );
    }

    // ---- With the lane ---------------------------------------------------

    /// A directory of the test's own, removed with everything in it when
    /// the test ends, however it ends.
    struct Scratch {
        dir: PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("aster-test-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).expect("FIXTURE: the test's own directory");
            Self { dir }
        }

        /// A sound file of two seconds at 8 kHz: a second of silence, then
        /// a second of a 1 kHz tone at half of full scale.
        fn tone(&self, name: &str) -> Asset {
            let samples: Vec<i16> = (0..16_000)
                .map(|index| {
                    let phase = std::f64::consts::TAU * 1000.0 * f64::from(index) / 8000.0;
                    if index < 8000 {
                        0
                    } else {
                        (0.5 * 32767.0 * phase.sin()).round() as i16
                    }
                })
                .collect();
            let data = samples.len() as u32 * 2;
            let mut bytes = Vec::with_capacity(44 + data as usize);
            bytes.extend(b"RIFF");
            bytes.extend((36 + data).to_le_bytes());
            bytes.extend(b"WAVEfmt ");
            // PCM, one channel, 8 kHz, 16 bits.
            bytes.extend(16u32.to_le_bytes());
            bytes.extend(1u16.to_le_bytes());
            bytes.extend(1u16.to_le_bytes());
            bytes.extend(8000u32.to_le_bytes());
            bytes.extend(16_000u32.to_le_bytes());
            bytes.extend(2u16.to_le_bytes());
            bytes.extend(16u16.to_le_bytes());
            bytes.extend(b"data");
            bytes.extend(data.to_le_bytes());
            bytes.extend(samples.iter().flat_map(|sample| sample.to_le_bytes()));
            let path = self.dir.join(name);
            std::fs::write(&path, &bytes).expect("FIXTURE: the sound file");
            let mut asset = Asset::new(path, MediaKind::Audio, Time::from_seconds(2.0));
            asset.file_size = bytes.len() as u64;
            asset
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn ffmpeg_available() -> bool {
        let tool = std::env::var_os("ASTER_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
        std::process::Command::new(tool)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn library(assets: &[&Asset]) -> Project {
        let mut project = Project::new("Waves");
        for asset in assets {
            Arc::make_mut(&mut project.assets).insert(asset.id, (*asset).clone());
        }
        project
    }

    /// What the frames of a window that keeps drawing a tile of `asset`
    /// come to, as the application runs them: the lane's answers are taken,
    /// the tile wants its sound, and what it wants is asked. Hands back the
    /// questions put, the answers that came and what was said, once the
    /// sound is held or is known not to be.
    fn drawn_until_settled(
        held: &mut HeldPeaks,
        sound: &SoundService,
        project: &Project,
        cache: &PeakCache,
        asset: &Asset,
    ) -> (Vec<Ask>, Vec<&'static str>, Vec<String>) {
        let (mut asks, mut answers, mut said) = (Vec::new(), Vec::new(), Vec::new());
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            while let Some(event) = sound.poll() {
                answers.push(match &event {
                    SoundEvent::Progress { .. } => "progress",
                    SoundEvent::Loaded { .. } => "loaded",
                    SoundEvent::Peaked { .. } => "peaked",
                    SoundEvent::Measured { .. } => "measured",
                    SoundEvent::Beats { .. } => "beats",
                    SoundEvent::Extracted { .. } => "extracted",
                    SoundEvent::Separated { .. } => "separated",
                });
                said.extend(held.answered(HERE, project, event));
            }
            held.begin_frame();
            held.want(asset);
            asks.extend(
                held.wants
                    .iter()
                    .filter_map(|id| match held.entries[id].known {
                        Known::Wanted(ask) => Some(ask),
                        _ => None,
                    }),
            );
            said.extend(held.ask(sound, project, HERE, cache));
            let settled = matches!(
                held.entries.get(&asset.id).map(|entry| &entry.known),
                Some(Known::Ready(_) | Known::Failed)
            );
            if settled && !sound.busy() {
                answers.retain(|answer| *answer != "progress");
                return (asks, answers, said);
            }
            assert!(
                std::time::Instant::now() < until,
                "the sound was not settled: asked {asks:?}, answered {answers:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn a_tile_in_view_has_its_sound_read_once_and_kept_for_the_next_session() {
        if !ffmpeg_available() {
            eprintln!("SKIP: FFmpeg is unavailable; no sound was read through the lane");
            return;
        }
        let scratch = Scratch::new();
        let asset = scratch.tone("tone.wav");
        let project = library(&[&asset]);
        let cache = PeakCache::at(scratch.dir.join("peaks"));
        let sound = SoundService::new(|| {}).expect("the lane starts");

        // The cache is asked first and keeps none, so the sound is read:
        // each asked once, however many frames the tile is drawn in.
        let mut held = HeldPeaks::default();
        let (asks, answers, said) =
            drawn_until_settled(&mut held, &sound, &project, &cache, &asset);
        assert_eq!(asks, [Ask::Kept, Ask::Sound]);
        assert_eq!(answers, ["loaded", "peaked"]);
        assert_eq!(said, Vec::<String>::new());
        let peaks = Arc::clone(held.held(&asset).expect("the peaks are held"));
        assert!(
            (200..=202).contains(&peaks.buckets.len()),
            "{}",
            peaks.buckets.len()
        );
        assert_eq!(held.bytes(), peaks.byte_len());
        // What a column of the silent second and one of the tone draw.
        let column =
            |from: f64, to: f64| peaks.range(Time::from_seconds(from), Time::from_seconds(to));
        assert_eq!(column(0.25, 0.75), Some((0, 0)));
        let (min, max) = column(1.25, 1.75).expect("the tone has peaks");
        assert!(
            (63..=65).contains(&max) && (-65..=-63).contains(&min),
            "{min} {max}"
        );
        assert_eq!(column(3.0, 4.0), None);
        // Held, the lane is asked nothing more and does nothing.
        for _ in 0..3 {
            held.begin_frame();
            held.want(&asset);
            assert!(!held.wanting());
        }
        assert!(!sound.busy() && sound.poll().is_none());
        assert_eq!(cache.held().1, 1);

        // Another session, or the project opened again: the cache answers
        // and the sound is not read a second time.
        held.clear();
        assert!(held.held(&asset).is_none());
        let (asks, answers, said) =
            drawn_until_settled(&mut held, &sound, &project, &cache, &asset);
        assert_eq!(asks, [Ask::Kept]);
        assert_eq!(answers, ["loaded"]);
        assert_eq!(said, Vec::<String>::new());
        assert_eq!(held.held(&asset), Some(&peaks));

        // An answer that comes after the project was replaced is for
        // nothing held: the lane was told to give it up, and what it had
        // already said is taken by no one.
        held.clear();
        held.begin_frame();
        held.want(&asset);
        assert!(held.ask(&sound, &project, HERE, &cache).is_empty());
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let late = loop {
            if let Some(event) = sound.poll() {
                break event;
            }
            assert!(
                std::time::Instant::now() < until,
                "the cache did not answer"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        held.clear();
        assert_eq!(held.answered(ELSEWHERE, &project, late), None);
        assert!(held.held(&asset).is_none());
        assert_eq!(held.bytes(), 0);
    }

    #[test]
    fn the_beats_shown_are_found_by_the_lane_in_the_peaks_it_keeps() {
        if !ffmpeg_available() {
            eprintln!("SKIP: FFmpeg is unavailable; no beats were asked of the lane");
            return;
        }
        let scratch = Scratch::new();
        let asset = scratch.tone("tone.wav");
        let project = library(&[&asset]);
        let cache = PeakCache::at(scratch.dir.join("peaks"));
        let sound = SoundService::new(|| {}).expect("the lane starts");
        let mut held = HeldPeaks::default();
        // Shown before its peaks are read, they are asked for once the
        // cache keeps the peaks, and not before; a second of tone has no
        // clear beat, which is said once.
        held.beats.show(asset.id, true);
        held.begin_frame();
        held.beats.want(&asset);
        let (asks, answers, said) =
            drawn_until_settled(&mut held, &sound, &project, &cache, &asset);
        assert_eq!(asks, [Ask::Kept, Ask::Sound]);
        assert_eq!(answers, ["loaded", "peaked", "beats"]);
        assert_eq!(said, ["No clear beat was found in tone.wav."]);
        assert_eq!(held.beats.state(&asset), BeatState::Unclear);
        held.begin_frame();
        held.beats.want(&asset);
        assert!(!held.wanting());
        idle(&sound);
        assert!(sound.poll().is_none());
    }

    /// Waits until the lane has nothing waiting or under way.
    fn idle(sound: &SoundService) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while sound.busy() {
            assert!(std::time::Instant::now() < until, "the lane did not stop");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn a_replaced_project_and_replaced_media_have_the_lane_give_up_what_was_asked() {
        let scratch = Scratch::new();
        let cache = PeakCache::at(scratch.dir.join("peaks"));
        let sound = SoundService::new(|| {}).expect("the lane starts");
        let mut one = sound_named("one.wav", 2.0);
        one.path = scratch.dir.join("one.wav");
        let mut two = sound_named("two.wav", 2.0);
        two.path = scratch.dir.join("two.wav");
        let project = library(&[&one, &two]);

        // Both are asked about, and the project is replaced before either
        // answer is taken: the lane holds no answer and nothing is known.
        let mut held = HeldPeaks::default();
        held.begin_frame();
        held.want(&one);
        held.want(&two);
        assert!(held.ask(&sound, &project, HERE, &cache).is_empty());
        assert!(
            held.entries
                .values()
                .all(|entry| matches!(entry.known, Known::Asked(..)))
        );
        held.project_replaced(Some(&sound));
        assert!(!sound.busy() && sound.poll().is_none());
        assert!(held.entries.is_empty() && !held.wanting());
        assert_eq!(held.bytes(), 0);
        // Before the lane was ever started there is none to tell.
        held.project_replaced(None);

        // Peaks are held of one and asked of the other; the first is then
        // of another file and the second leaves the library.
        assert_eq!(hold(&mut held, &one, 99, 200), Taken::Kept);
        held.begin_frame();
        held.want(&one);
        held.want(&two);
        assert!(held.ask(&sound, &project, HERE, &cache).is_empty());
        held.media_replaced(one.id, Some(&sound));
        held.media_replaced(two.id, Some(&sound));
        assert!(!sound.busy() && sound.poll().is_none());
        assert!(held.held(&one).is_none());
        assert_eq!(held.bytes(), 0);
        // The media as it is now is asked about anew.
        let mut relinked = one.clone();
        relinked.file_size += 1;
        held.begin_frame();
        held.want(&relinked);
        assert_eq!(held.take_wants(), vec![(one.id, Ask::Kept)]);
    }

    #[test]
    fn the_lane_stops_reading_sound_for_a_tile_no_longer_drawn() {
        let scratch = Scratch::new();
        let cache = PeakCache::at(scratch.dir.join("peaks"));
        let sound = SoundService::new(|| {}).expect("the lane starts");
        // A file that is not there: whatever reads it has nothing to keep.
        let mut away = sound_named("away.wav", 2.0);
        away.path = scratch.dir.join("away.wav");
        let project = library(&[&away]);
        let mut held = HeldPeaks::default();
        // Drawn until the cache has said it keeps none and the sound is
        // asked for.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !matches!(held.entries.get(&away.id), Some(entry) if matches!(entry.known, Known::Asked(_, Ask::Sound)))
        {
            while let Some(event) = sound.poll() {
                assert!(matches!(event, SoundEvent::Loaded { .. }), "{event:?}");
                assert_eq!(held.answered(HERE, &project, event), None);
            }
            held.begin_frame();
            held.want(&away);
            assert!(held.ask(&sound, &project, HERE, &cache).is_empty());
            assert!(
                std::time::Instant::now() < until,
                "the sound was not asked for"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // Two frames in which it is not drawn, as the application runs
        // them with nothing wanted: the read is given up and answers nothing.
        held.begin_frame();
        held.give_up_unwanted(&sound);
        held.begin_frame();
        assert!(held.ask(&sound, &project, HERE, &cache).is_empty());
        idle(&sound);
        assert!(sound.poll().is_none());
        assert_eq!(
            held.entries[&away.id].known,
            Known::Wanted(Ask::Sound),
            "it is read when it is drawn again"
        );
        held.begin_frame();
        held.want(&away);
        assert!(held.wanting());
    }

    #[test]
    fn sound_that_cannot_be_read_is_said_once_and_not_asked_for_again() {
        let scratch = Scratch::new();
        let cache = PeakCache::at(scratch.dir.join("peaks"));
        let sound = SoundService::new(|| {}).expect("the lane starts");
        let mut held = HeldPeaks::default();

        // Longer than a waveform may be: refused before any process, in
        // words, and then left alone.
        let long = sound_named("long.wav", 6.0 * 3600.0);
        let project = library(&[&long]);
        held.begin_frame();
        held.want(&long);
        assert_eq!(
            held.ask(&sound, &project, HERE, &cache),
            ["long.wav has no waveform: Media is too long for a waveform: the limit is 5.8 hours"]
        );
        for _ in 0..3 {
            held.begin_frame();
            held.want(&long);
            assert!(!held.wanting());
            assert!(held.ask(&sound, &project, HERE, &cache).is_empty());
        }
        assert!(!sound.busy() && sound.poll().is_none());

        // A file that is not there: read, failed, said once.
        if !ffmpeg_available() {
            eprintln!("SKIP: FFmpeg is unavailable; a missing sound file was not asked of it");
            return;
        }
        let mut gone = sound_named("gone.wav", 2.0);
        gone.path = scratch.dir.join("gone.wav");
        let project = library(&[&gone]);
        let (asks, answers, said) = drawn_until_settled(&mut held, &sound, &project, &cache, &gone);
        assert_eq!(asks, [Ask::Kept, Ask::Sound]);
        assert_eq!(answers, ["loaded", "peaked"]);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with("gone.wav has no waveform: "),
            "{said:?}"
        );
        assert_eq!(said[0].lines().count(), 1, "{said:?}");
        assert!(held.held(&gone).is_none());
        for _ in 0..3 {
            held.begin_frame();
            held.want(&gone);
            assert!(!held.wanting());
        }
        assert_eq!(cache.held().1, 0);
    }
}
