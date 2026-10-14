//! A media file's sound as peaks, a hundred to the second, and the cache they
//! are kept in between sessions.
//!
//! The sound is read once, mixed to one channel at 8 kHz, and every 80
//! samples become four bytes: the least and the greatest sample, the level,
//! and the level of what is under about 150 Hz. That is what a waveform
//! column, the ducking gate and the beat grid are drawn from. Bucket `i` is
//! container time `i / 100` s. Nothing here is project data.
use editor_core::{Asset, Time};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

/// The version of the reduction: peaks made by another are not read.
pub const PEAK_ALGORITHM: u32 = 1;
/// The buckets a second of sound becomes.
pub const PEAKS_PER_SECOND: u32 = 100;
/// The buckets one file may have, 5.8 hours. A longer file has no peaks.
pub const MAX_PEAK_BUCKETS: usize = 1 << 21;
/// What the cache directory may hold, and in how many files.
pub const PEAK_CACHE_BYTES: u64 = 64 * 1024 * 1024;
pub const PEAK_CACHE_ENTRIES: usize = 512;

/// The rate the sound is read at, and the samples of it in a bucket.
pub(super) const SAMPLE_RATE: u32 = 8000;
const BUCKET_SAMPLES: usize = (SAMPLE_RATE / PEAKS_PER_SECOND) as usize;
// A bucket's length on the container's clock, in microseconds.
const BUCKET_TIME: i64 = 1_000_000 / PEAKS_PER_SECOND as i64;
// Where the one-pole low-pass behind `low` turns over, in Hz.
const LOW_BELOW: f64 = 150.0;
// The level a `rms` or `low` of nought stands for, in dBFS.
const FLOOR_DB: f64 = -96.0;

/// One bucket: a hundredth of a second of sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peak {
    /// The least and the greatest sample, in 128ths of full scale.
    pub min: i8,
    pub max: i8,
    /// The level, from -96 dBFS (0) to 0 dBFS (255).
    pub rms: u8,
    /// The level of what is under about 150 Hz, on the same scale.
    pub low: u8,
}

impl Peak {
    fn of(bucket: [u8; 4]) -> Self {
        Self {
            min: bucket[0] as i8,
            max: bucket[1] as i8,
            rms: bucket[2],
            low: bucket[3],
        }
    }

    /// What a `rms` or `low` says, in dBFS.
    pub fn decibels(level: u8) -> f32 {
        (FLOOR_DB + f64::from(level) * -FLOOR_DB / 255.0) as f32
    }
}

/// A file's sound, a bucket for each hundredth of a second of container
/// time from 0: `[min: i8, max: i8, rms: u8, low: u8]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Peaks {
    pub buckets: Vec<[u8; 4]>,
}

impl Peaks {
    /// The bucket a source time falls in. None before the file's start and
    /// after the last sound read.
    pub fn at(&self, source: Time) -> Option<Peak> {
        let index = usize::try_from(source.0.div_euclid(BUCKET_TIME)).ok()?;
        self.buckets.get(index).copied().map(Peak::of)
    }

    /// The least and the greatest sample of the buckets a stretch of source
    /// time touches, either end first. None when it touches none.
    pub fn range(&self, from: Time, to: Time) -> Option<(i8, i8)> {
        let (from, to) = (from.min(to), from.max(to));
        // A stretch that ends on a bucket's edge does not reach into it,
        // but a stretch of no length still touches the bucket it is in.
        let end = to.0 - i64::from(to > from);
        if end < 0 {
            return None;
        }
        let first = usize::try_from(from.0.max(0) / BUCKET_TIME).ok()?;
        let last = usize::try_from(end / BUCKET_TIME).ok()?;
        let touched = (self.buckets).get(first..=last.min(self.buckets.len().checked_sub(1)?))?;
        touched.iter().fold(None, |held, bucket| {
            let (min, max) = (bucket[0] as i8, bucket[1] as i8);
            Some(held.map_or((min, max), |(low, high): (i8, i8)| {
                (low.min(min), high.max(max))
            }))
        })
    }

    /// The memory the buckets take.
    pub fn byte_len(&self) -> usize {
        self.buckets.len() * 4
    }
}

/// How many buckets a length of sound is, the last counted when it is part
/// of one.
pub(super) fn buckets_in(duration: Time) -> u64 {
    (duration.0.max(0) as u64).div_ceil(BUCKET_TIME as u64)
}

// A mean square of 16-bit samples as a level byte:
// `round((dBFS + 96) * 255 / 96)`, held to the byte.
fn level(mean_square: f64) -> u8 {
    if mean_square <= 0.0 {
        return 0;
    }
    let decibels = 10.0 * (mean_square / (32768.0 * 32768.0)).log10();
    ((decibels - FLOOR_DB) * 255.0 / -FLOOR_DB)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Turns 16-bit little-endian samples at [`SAMPLE_RATE`] into buckets as
/// they are read, in pieces of any length.
pub(super) struct Reducer {
    buckets: Vec<[u8; 4]>,
    /// The first byte of a sample whose second has not been read.
    half: Option<u8>,
    count: usize,
    min: i16,
    max: i16,
    squares: f64,
    low_squares: f64,
    /// The low-pass's one value of state, carried from bucket to bucket.
    low: f64,
    pole: f64,
}

impl Default for Reducer {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            half: None,
            count: 0,
            min: 0,
            max: 0,
            squares: 0.0,
            low_squares: 0.0,
            low: 0.0,
            pole: 1.0 - (-std::f64::consts::TAU * LOW_BELOW / f64::from(SAMPLE_RATE)).exp(),
        }
    }
}

impl Reducer {
    /// The whole buckets so far.
    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    pub fn push(&mut self, mut bytes: &[u8]) {
        if let Some(first) = self.half
            && let Some((second, rest)) = bytes.split_first()
        {
            self.half = None;
            self.sample(i16::from_le_bytes([first, *second]));
            bytes = rest;
        }
        let pairs = bytes.chunks_exact(2);
        if let [first] = pairs.remainder() {
            self.half = Some(*first);
        }
        for pair in pairs {
            self.sample(i16::from_le_bytes([pair[0], pair[1]]));
        }
    }

    fn sample(&mut self, sample: i16) {
        if self.count == 0 {
            (self.min, self.max) = (sample, sample);
        } else {
            (self.min, self.max) = (self.min.min(sample), self.max.max(sample));
        }
        let value = f64::from(sample);
        self.low += self.pole * (value - self.low);
        self.squares += value * value;
        self.low_squares += self.low * self.low;
        self.count += 1;
        if self.count == BUCKET_SAMPLES {
            self.close();
        }
    }

    fn close(&mut self) {
        let count = self.count as f64;
        // Outwards to the next 128th, so a sample is never outside its
        // bucket's two.
        let min = self.min.div_euclid(256) as i8;
        let max = (i32::from(self.max) + 255).div_euclid(256).min(127) as i8;
        self.buckets.push([
            min as u8,
            max as u8,
            level(self.squares / count),
            level(self.low_squares / count),
        ]);
        (self.count, self.squares, self.low_squares) = (0, 0.0, 0.0);
    }

    /// The peaks, with what was read of a last bucket as one.
    pub fn finish(mut self) -> Peaks {
        if self.count != 0 {
            self.close();
        }
        Peaks {
            buckets: self.buckets,
        }
    }
}

// ---- The cache -----------------------------------------------------------------

/// Where peaks are kept between sessions: one file for a media file's
/// sound, named for the path of the media, in a directory held to a size and
/// a count. The least recently read are removed to make room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeakCache {
    pub dir: PathBuf,
    pub bytes: u64,
    pub entries: usize,
}

/// What is known of a file's peaks without reading its sound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeptPeaks {
    Held(Arc<Peaks>),
    /// Peaks are kept, made from what the file was before it changed.
    Stale,
    /// None are kept.
    Missing,
}

/// What a media file was when its sound was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Stamp {
    /// The file as the disk has it: nought for one that cannot be asked.
    length: u64,
    modified: u64,
    /// And as the project has it.
    file_size: u64,
    duration: i64,
}

impl Stamp {
    pub fn of(asset: &Asset) -> Self {
        let on_disk = fs::metadata(&asset.path).ok();
        let modified = (on_disk.as_ref())
            .and_then(|file| file.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos().min(u128::from(u64::MAX)) as u64);
        Self {
            length: on_disk.map_or(0, |file| file.len()),
            modified,
            file_size: asset.file_size,
            duration: asset.duration.0,
        }
    }

    fn write(&self, to: &mut Vec<u8>) {
        to.extend(self.length.to_le_bytes());
        to.extend(self.modified.to_le_bytes());
        to.extend(self.file_size.to_le_bytes());
        to.extend(self.duration.to_le_bytes());
    }
}

pub(super) const MAGIC: &[u8; 8] = b"ASTRPEAK";
const EXTENSION: &str = "peaks";

pub(super) fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

// What is read of a file's bytes, in order.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.0.split_at_checked(count)?;
        self.0 = rest;
        Some(taken)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

impl PeakCache {
    /// A cache in `dir`, held to [`PEAK_CACHE_BYTES`] and
    /// [`PEAK_CACHE_ENTRIES`].
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            bytes: PEAK_CACHE_BYTES,
            entries: PEAK_CACHE_ENTRIES,
        }
    }

    fn name_of(path: &Path) -> Vec<u8> {
        path.as_os_str().as_encoded_bytes().to_vec()
    }

    /// The file a media file's peaks are kept in: one for its place on
    /// disk, so that a file changed in place finds what was read of it
    /// before.
    pub fn file(&self, asset: &Asset) -> PathBuf {
        self.dir.join(format!(
            "{:016x}.{EXTENSION}",
            fnv(&Self::name_of(&asset.path))
        ))
    }

    pub(super) fn encode(asset: &Asset, stamp: &Stamp, peaks: &Peaks) -> Vec<u8> {
        let name = Self::name_of(&asset.path);
        let mut bytes = Vec::with_capacity(64 + name.len() + peaks.byte_len());
        bytes.extend(MAGIC);
        bytes.extend(PEAK_ALGORITHM.to_le_bytes());
        stamp.write(&mut bytes);
        bytes.extend((name.len() as u32).to_le_bytes());
        bytes.extend(&name);
        bytes.extend((peaks.buckets.len() as u32).to_le_bytes());
        bytes.extend(peaks.buckets.as_flattened());
        let sum = fnv(&bytes);
        bytes.extend(sum.to_le_bytes());
        bytes
    }

    // The peaks a file holds, when it is whole, of this reduction and of
    // this media file's place; and whether the media is still what it was.
    fn decode(bytes: &[u8], asset: &Asset, stamp: &Stamp) -> Option<KeptPeaks> {
        let (body, sum) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
        if fnv(body) != u64::from_le_bytes(sum.try_into().ok()?) {
            return None;
        }
        let mut reader = Reader(body);
        if reader.take(8)? != MAGIC || reader.u32()? != PEAK_ALGORITHM {
            return None;
        }
        let mut held = Vec::new();
        stamp.write(&mut held);
        let same = reader.take(held.len())? == held.as_slice();
        let name = reader.u32()? as usize;
        if reader.take(name)? != Self::name_of(&asset.path).as_slice() {
            return None;
        }
        if !same {
            return Some(KeptPeaks::Stale);
        }
        let count = reader.u32()? as usize;
        if count == 0 || count > MAX_PEAK_BUCKETS {
            return None;
        }
        let values = reader.take(count.checked_mul(4)?)?;
        if !reader.0.is_empty() {
            return None;
        }
        let buckets: Vec<[u8; 4]> = (values.chunks_exact(4))
            .map(|bucket| [bucket[0], bucket[1], bucket[2], bucket[3]])
            .collect();
        // A bucket's least sample is never over its greatest.
        if (buckets.iter()).any(|bucket| bucket[0] as i8 > bucket[1] as i8) {
            return None;
        }
        Some(KeptPeaks::Held(Arc::new(Peaks { buckets })))
    }

    /// What is kept of a media file's peaks. A file that is damaged, of
    /// another reduction or of another media file is as good as none; one
    /// read whole is marked as read now, so it is the last to be removed.
    pub fn load(&self, asset: &Asset) -> KeptPeaks {
        let file = self.file(asset);
        // No peaks are larger than this: a larger file is not read at all.
        let most = 4096 + MAX_PEAK_BUCKETS as u64 * 4;
        if !fs::metadata(&file).is_ok_and(|kept| kept.is_file() && kept.len() <= most) {
            return KeptPeaks::Missing;
        }
        let Ok(bytes) = fs::read(&file) else {
            return KeptPeaks::Missing;
        };
        let found = Self::decode(&bytes, asset, &Stamp::of(asset)).unwrap_or(KeptPeaks::Missing);
        if matches!(found, KeptPeaks::Held(_))
            && let Ok(kept) = fs::File::options().write(true).open(&file)
        {
            let _ = kept.set_modified(SystemTime::now());
        }
        found
    }

    pub(super) fn store(&self, asset: &Asset, stamp: &Stamp, peaks: &Peaks) -> Result<(), String> {
        let bytes = Self::encode(asset, stamp, peaks);
        let failed = |error: std::io::Error| format!("{}: {error}", self.dir.display());
        fs::create_dir_all(&self.dir).map_err(failed)?;
        let file = self.file(asset);
        // Written whole beside its place and then moved into it, so a
        // reader never finds half of one.
        let beside = file.with_extension(format!("{EXTENSION}.{}", Uuid::new_v4().simple()));
        let written = fs::File::create(&beside)
            .and_then(|mut to| to.write_all(&bytes))
            .and_then(|()| fs::rename(&beside, &file));
        if let Err(error) = written {
            let _ = fs::remove_file(&beside);
            return Err(failed(error));
        }
        self.make_room(&file);
        Ok(())
    }

    /// The files kept, each with its size and when it was last read or
    /// written.
    fn kept(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
                    return None;
                }
                let file = entry.metadata().ok().filter(|file| file.is_file())?;
                Some((path, file.len(), file.modified().unwrap_or(UNIX_EPOCH)))
            })
            .collect()
    }

    /// The bytes and the files the directory holds.
    pub fn held(&self) -> (u64, usize) {
        let kept = self.kept();
        (kept.iter().map(|(_, bytes, _)| bytes).sum(), kept.len())
    }

    // Removes the files read longest ago until what is left is within the
    // bounds. `keep`, just written, is the last to go.
    fn make_room(&self, keep: &Path) {
        // What a write that never ended left beside its place.
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let old = (entry.metadata().ok())
                    .and_then(|file| file.modified().ok())
                    .and_then(|at| at.elapsed().ok())
                    .is_some_and(|age| age.as_secs() > 3600);
                if old && name.to_string_lossy().contains(&format!(".{EXTENSION}.")) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let mut kept = self.kept();
        kept.sort_by(|a, b| (a.0 == keep).cmp(&(b.0 == keep)).then(a.2.cmp(&b.2)));
        let mut bytes: u64 = kept.iter().map(|(_, bytes, _)| bytes).sum();
        let mut count = kept.len();
        for (path, size, _) in kept {
            if (bytes <= self.bytes && count <= self.entries) || path == keep {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                bytes -= size;
                count -= 1;
            }
        }
    }
}
