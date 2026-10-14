//! Source pictures for an export, in order and at full quality. A video is
//! read through one FFmpeg pipe per clip, asset and size, with the preview's
//! seek and frame grid so both show the same source frame, and a better
//! scaler. Stills and GIFs are decoded here as the preview decodes them.
use crate::{
    Frame, MediaError,
    import::{gif_delay_us, image_reader, limits},
    io,
    process::{Process, Processes, tool_command},
};
use editor_core::{Asset, AssetId, FrameRate, MediaKind, Time};
use image::{
    AnimationDecoder, DynamicImage, Frames, ImageDecoder, codecs::gif::GifDecoder,
    imageops::FilterType,
};
use std::{
    fs::File,
    io::{BufReader, ErrorKind, Read},
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

/// The longest side a source is decoded at for an export.
pub const MAX_READ_SIDE: u32 = 8192;
/// The most bytes one decoded export picture may have.
pub const MAX_READ_BYTES: usize = 256 * 1024 * 1024;
/// The longest one picture may take to arrive, seeking included.
const READ_DEADLINE: Duration = Duration::from_secs(60);
/// Pipes kept open at once; the least recently read closes first.
const MAX_PIPES: usize = 16;
/// GIF decoders kept at once.
const MAX_GIFS: usize = 8;
/// Decoded stills kept at once, in bytes; the one just used always stays.
const STILL_BUDGET: usize = 512 * 1024 * 1024;
/// How far ahead a pipe reads on rather than seeking: at least this many
/// frames, or a second of the source.
const MIN_GAP: i64 = 12;
const MAX_GIF_FRAMES: i64 = 10_000;
/// Decoder threads a pipe; up to `MAX_PIPES` are open. FFmpeg's own count
/// (up to 16 frame threads, each holding pictures) held 1,099-1,147 MiB in
/// a 4K pipe read at 16 fps, and four held 779-790 MiB. Four decoded as
/// fast as automatic, the pipe's scaler setting the pace; two lost about
/// 40 % on a high-rate 4K H.264 source and at 1080p.
const DECODE_THREADS: &str = "4";

/// The size a source is decoded at: `size` as a box the source's aspect is
/// fitted into, never above the source's own size, at most
/// [`MAX_READ_SIDE`] a side and [`MAX_READ_BYTES`] in all.
pub(crate) fn read_dimensions(source: [u32; 2], size: [u32; 2]) -> Result<[u32; 2], MediaError> {
    if size.contains(&0) || size.iter().any(|side| *side > MAX_READ_SIDE) {
        return Err(MediaError::Invalid(format!(
            "Export pictures must be between 1 and {MAX_READ_SIDE} pixels a side"
        )));
    }
    if source.contains(&0) {
        return Err(MediaError::Invalid("Media has no video dimensions".into()));
    }
    let ratio = (f64::from(size[0]) / f64::from(source[0]))
        .min(f64::from(size[1]) / f64::from(source[1]))
        .min(1.0);
    let fitted = source.map(|side| (f64::from(side) * ratio).round().max(1.0) as u32);
    let bytes = fitted[0] as usize * fitted[1] as usize * 4;
    if bytes > MAX_READ_BYTES {
        return Err(MediaError::Invalid(
            "An export picture exceeds its 256 MiB budget".into(),
        ));
    }
    Ok(fitted)
}

/// What a source was when it was opened: a pipe, still or GIF of another
/// file, length or size is not reused.
#[derive(Clone, PartialEq, Eq)]
struct Origin {
    path: PathBuf,
    kind: MediaKind,
    duration: Time,
    width: u32,
    height: u32,
    frame_rate: Option<FrameRate>,
    file_size: u64,
}

impl From<&Asset> for Origin {
    fn from(asset: &Asset) -> Self {
        Self {
            path: asset.path.clone(),
            kind: asset.kind,
            duration: asset.duration,
            width: asset.width,
            height: asset.height,
            frame_rate: asset.frame_rate,
            file_size: asset.file_size,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    lane: Uuid,
    asset: AssetId,
    size: [u32; 2],
}

struct Pipe {
    key: Key,
    origin: Origin,
    /// `None` once the stream has ended.
    process: Option<Process>,
    /// The index of the frame the pipe gives next.
    next: i64,
    /// The last frame read and its index: what a repeat or an end holds.
    last: Option<(i64, Frame)>,
    used: u64,
}

struct Still {
    key: Key,
    origin: Origin,
    frame: Frame,
    used: u64,
}

struct Gif {
    key: Key,
    origin: Origin,
    frames: Frames<'static>,
    next_index: i64,
    next_time: Time,
    last: Option<(Frame, (Time, Time))>,
    used: u64,
}

/// Decodes the source pictures an export draws. Owns its processes; dropping
/// it kills and reaps them. Blocking: use it on the render thread.
pub struct SourceReader {
    processes: Processes,
    cancelled: Arc<AtomicBool>,
    pipes: Vec<Pipe>,
    stills: Vec<Still>,
    gifs: Vec<Gif>,
    tick: u64,
    #[cfg(test)]
    pub(crate) starts: usize,
}

impl Default for SourceReader {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceReader {
    pub fn new() -> Self {
        Self {
            processes: Processes::default(),
            cancelled: Arc::new(AtomicBool::new(false)),
            pipes: Vec::new(),
            stills: Vec::new(),
            gifs: Vec::new(),
            tick: 0,
            #[cfg(test)]
            starts: 0,
        }
    }

    /// The picture of `asset` at source time `time` (a `source_frame`
    /// result), fitted into `size` without upscaling (see
    /// [`MAX_READ_SIDE`] and [`MAX_READ_BYTES`]). `lane` is the clip asking:
    /// pipes are kept per lane, asset and size, so a clip's matte plate does
    /// not restart its picture's pipe. A later frame is read on from where
    /// the pipe is; an earlier or far one starts the pipe again; the same
    /// frame is given again; past the stream's real end the last frame
    /// holds.
    pub fn picture(
        &mut self,
        lane: Uuid,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<Frame, MediaError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(MediaError::Cancelled);
        }
        let size = read_dimensions([asset.width, asset.height], size)?;
        let key = Key {
            lane,
            asset: asset.id,
            size,
        };
        // As the preview: never before the start or on the end itself.
        let time = Time(time.0.min(asset.duration.0.saturating_sub(1)).max(0));
        self.tick += 1;
        match asset.kind {
            MediaKind::Video => self.video(key, asset, time),
            MediaKind::Image => self.still(key, asset),
            MediaKind::Gif => self.gif(key, asset, time),
            MediaKind::Audio => Err(MediaError::Invalid("Audio has no video frame".into())),
            MediaKind::Lut => Err(MediaError::Invalid("A LUT has no video frame".into())),
            MediaKind::Font => Err(MediaError::Invalid("A font has no video frame".into())),
        }
    }

    /// Closes the pipes and decoders of lanes that are not `live`.
    pub fn retain(&mut self, live: &dyn Fn(Uuid) -> bool) {
        self.pipes.retain(|pipe| live(pipe.key.lane));
        self.stills.retain(|still| live(still.key.lane));
        self.gifs.retain(|gif| live(gif.key.lane));
    }

    /// Stops the reader from any thread: its processes are killed and every
    /// later `picture` is `Cancelled`.
    pub fn canceller(&self) -> impl Fn() + Send + Sync + 'static {
        let processes = self.processes.clone();
        let cancelled = Arc::clone(&self.cancelled);
        move || {
            cancelled.store(true, Ordering::Release);
            processes.kill_all();
        }
    }

    #[cfg(test)]
    pub(crate) fn processes(&self) -> &Processes {
        &self.processes
    }

    /// Open video pipes.
    pub fn pipes(&self) -> usize {
        self.pipes
            .iter()
            .filter(|pipe| pipe.process.is_some())
            .count()
    }

    fn video(&mut self, key: Key, asset: &Asset, time: Time) -> Result<Frame, MediaError> {
        let rate = asset.frame_rate.unwrap_or_default();
        let target = rate.frame_at(time);
        let origin = Origin::from(asset);
        let gap = MIN_GAP.max(rate.fps().ceil() as i64);
        // A pipe of what the asset no longer is goes.
        self.pipes
            .retain(|pipe| pipe.key != key || pipe.origin == origin);
        let existing = (self.pipes.iter())
            .position(|pipe| pipe.key == key)
            .map(|index| self.pipes.swap_remove(index));
        let mut pipe = match existing {
            Some(pipe) => pipe,
            None => self.open(key, origin.clone(), asset, rate, target)?,
        };
        pipe.used = self.tick;
        if let Some((index, frame)) = &pipe.last
            && (*index == target || (pipe.process.is_none() && target >= pipe.next))
        {
            let frame = frame.clone();
            self.keep(pipe);
            return Ok(frame);
        }
        if target < pipe.next || target > pipe.next + gap {
            pipe = self.open(key, origin.clone(), asset, rate, target)?;
        }
        let mut frame = self.read_to(&mut pipe, target, asset)?;
        // A stream shorter than its container says, sought past its real
        // end, gives nothing: the pipe starts further back, then from the
        // start, and reads on to its last frame, which then holds.
        let fps = rate.fps().ceil() as i64;
        let mut from = pipe.next;
        for seconds in [1, 10, i64::MAX] {
            if frame.is_some() {
                break;
            }
            let back = target.saturating_sub(fps.saturating_mul(seconds)).max(0);
            if back >= from {
                continue;
            }
            pipe = self.open(key, origin.clone(), asset, rate, back)?;
            from = back;
            frame = self.read_to(&mut pipe, target, asset)?;
        }
        match frame {
            Some(frame) => {
                self.keep(pipe);
                Ok(frame)
            }
            None => Err(MediaError::Decode(format!(
                "{} has no picture at {:.3}s",
                asset.name,
                time.as_seconds()
            ))),
        }
    }

    /// Reads `pipe` up to `target`: that frame, or the last one when the
    /// stream ends first, or `None` when it ended before giving any.
    fn read_to(
        &self,
        pipe: &mut Pipe,
        target: i64,
        asset: &Asset,
    ) -> Result<Option<Frame>, MediaError> {
        let [width, height] = pipe.key.size;
        let bytes = width as usize * height as usize * 4;
        while pipe.next <= target {
            let Some(process) = pipe.process.as_mut() else {
                break;
            };
            let mut rgba: Arc<[u8]> = std::iter::repeat_n(0, bytes).collect();
            let Some(buffer) = Arc::get_mut(&mut rgba) else {
                return Err(MediaError::Closed);
            };
            match self.read_frame(process, buffer, asset)? {
                true => {
                    let frame = Frame {
                        width,
                        height,
                        rgba,
                    };
                    pipe.last = Some((pipe.next, frame));
                    pipe.next += 1;
                }
                false => pipe.process = None,
            }
        }
        Ok(pipe.last.as_ref().map(|(_, frame)| frame.clone()))
    }

    /// Fills `buffer` with the next picture: `false` at the stream's end.
    fn read_frame(
        &self,
        process: &mut Process,
        buffer: &mut [u8],
        asset: &Asset,
    ) -> Result<bool, MediaError> {
        let cancelled = || self.cancelled.load(Ordering::Acquire);
        if !process.begin_read_until(Instant::now() + READ_DEADLINE) {
            return Err(if cancelled() {
                MediaError::Cancelled
            } else {
                process.failure("Could not read on")
            });
        }
        let mut filled = 0;
        let result = loop {
            match process.stdout.read(&mut buffer[filled..]) {
                Ok(0) => break Ok(false),
                Ok(count) => {
                    filled += count;
                    if filled == buffer.len() {
                        break Ok(true);
                    }
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => break Err(error),
            }
        };
        let completed = process.end_read_checked();
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        if !completed {
            return Err(process.failure(&format!(
                "Decoding {} took longer than {} seconds",
                asset.name,
                READ_DEADLINE.as_secs()
            )));
        }
        match result {
            Ok(true) => Ok(true),
            Ok(false) => {
                // The end of the stream, or FFmpeg failing: only a clean exit
                // is an end.
                let status = process.wait_until(Instant::now() + Duration::from_secs(5))?;
                if cancelled() {
                    return Err(MediaError::Cancelled);
                }
                match status {
                    Some(status) if status.success() => Ok(false),
                    _ => {
                        let _ = process.stderr_until(Instant::now() + Duration::from_secs(1));
                        Err(process.failure(&format!("Could not decode {}", asset.name)))
                    }
                }
            }
            Err(error) => {
                Err(process.failure(&format!("Could not decode {} ({error})", asset.name)))
            }
        }
    }

    fn open(
        &mut self,
        key: Key,
        origin: Origin,
        asset: &Asset,
        rate: FrameRate,
        target: i64,
    ) -> Result<Pipe, MediaError> {
        while self.pipes.len() >= MAX_PIPES {
            let oldest = (0..self.pipes.len())
                .min_by_key(|index| self.pipes[*index].used)
                .unwrap_or(0);
            self.pipes.swap_remove(oldest);
        }
        let mut command = read_command(asset, rate, target, key.size);
        let process = Process::spawn("ffmpeg", &mut command, &self.processes)?;
        #[cfg(test)]
        {
            self.starts += 1;
        }
        Ok(Pipe {
            key,
            origin,
            process: Some(process),
            next: target,
            last: None,
            used: self.tick,
        })
    }

    fn keep(&mut self, pipe: Pipe) {
        self.pipes.push(pipe);
    }

    fn still(&mut self, key: Key, asset: &Asset) -> Result<Frame, MediaError> {
        let origin = Origin::from(asset);
        self.stills
            .retain(|still| still.key != key || still.origin == origin);
        if let Some(still) = self.stills.iter_mut().find(|still| still.key == key) {
            still.used = self.tick;
            return Ok(still.frame.clone());
        }
        let image = image_reader(&asset.path)?
            .decode()
            .map_err(|error| MediaError::Decode(error.to_string()))?;
        let frame = scaled(image, key.size)?;
        let bytes = frame.byte_len();
        let mut held: usize = self.stills.iter().map(|still| still.frame.byte_len()).sum();
        while !self.stills.is_empty() && held + bytes > STILL_BUDGET {
            let oldest = (0..self.stills.len())
                .min_by_key(|index| self.stills[*index].used)
                .unwrap_or(0);
            held -= self.stills.swap_remove(oldest).frame.byte_len();
        }
        self.stills.push(Still {
            key,
            origin,
            frame: frame.clone(),
            used: self.tick,
        });
        Ok(frame)
    }

    fn gif(&mut self, key: Key, asset: &Asset, time: Time) -> Result<Frame, MediaError> {
        let origin = Origin::from(asset);
        let index = self.gifs.iter().position(|gif| gif.key == key);
        let mut gif = index.map(|index| self.gifs.swap_remove(index));
        if gif.as_ref().is_some_and(|gif| {
            gif.origin != origin
                || (time < gif.next_time
                    && !gif
                        .last
                        .as_ref()
                        .is_some_and(|(_, (start, end))| *start <= time && time < *end))
        }) {
            gif = None;
        }
        let mut gif = match gif {
            Some(gif) => gif,
            None => {
                let file = File::open(&asset.path).map_err(|error| io("Cannot open GIF", error))?;
                let mut native = GifDecoder::new(BufReader::new(file))
                    .map_err(|error| MediaError::Decode(error.to_string()))?;
                native
                    .set_limits(limits())
                    .map_err(|error| MediaError::Decode(error.to_string()))?;
                Gif {
                    key,
                    origin,
                    frames: native.into_frames(),
                    next_index: 0,
                    next_time: Time::ZERO,
                    last: None,
                    used: self.tick,
                }
            }
        };
        gif.used = self.tick;
        let result = loop {
            if let Some((frame, (start, end))) = &gif.last
                && *start <= time
                && time < *end
            {
                break Ok(frame.clone());
            }
            if gif.next_index >= MAX_GIF_FRAMES {
                break Err(MediaError::Invalid("GIF exceeds 10,000 frames".into()));
            }
            let image = match gif.frames.next() {
                Some(Ok(frame)) => frame,
                Some(Err(error)) => break Err(MediaError::Decode(error.to_string())),
                // Past the last frame: it holds.
                None => {
                    break gif
                        .last
                        .as_ref()
                        .map(|(frame, _)| frame.clone())
                        .ok_or_else(|| MediaError::Decode("GIF has no frames".into()));
                }
            };
            let start = gif.next_time;
            let end = start + Time(gif_delay_us(image.delay()));
            gif.next_index += 1;
            gif.next_time = end;
            if time < end {
                match scaled(DynamicImage::ImageRgba8(image.into_buffer()), key.size) {
                    Ok(frame) => gif.last = Some((frame, (start, end))),
                    Err(error) => break Err(error),
                }
            }
        };
        if result.is_ok() {
            while self.gifs.len() >= MAX_GIFS {
                let oldest = (0..self.gifs.len())
                    .min_by_key(|index| self.gifs[*index].used)
                    .unwrap_or(0);
                self.gifs.swap_remove(oldest);
            }
            self.gifs.push(gif);
        }
        result
    }
}

/// The preview's decode (`-ss` on the input, `fps` on the asset's grid,
/// colour matrix defaults) with an accurate bicubic scaler.
pub(crate) fn read_command(asset: &Asset, rate: FrameRate, target: i64, size: [u32; 2]) -> Command {
    let mut command = tool_command("ffmpeg");
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(["-threads", DECODE_THREADS, "-ss"])
        .arg(format!("{:.9}", target as f64 / rate.fps()))
        .arg("-i")
        .arg(&asset.path)
        .args([
            "-map",
            "0:V:0",
            "-an",
            "-sn",
            "-dn",
            "-filter_threads",
            "2",
            "-vf",
        ])
        .arg(format!(
            "scale={}:{}:flags=bicubic+accurate_rnd+full_chroma_int,fps={}/{}",
            size[0], size[1], rate.numerator, rate.denominator
        ))
        // Few whole pictures waiting in FFmpeg per pipe.
        .args([
            "-pix_fmt",
            "rgba",
            "-threads:v",
            "1",
            "-thread_queue_size",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ]);
    command
}

/// A decoded still or GIF frame at `size`, scaled bicubically.
fn scaled(image: DynamicImage, size: [u32; 2]) -> Result<Frame, MediaError> {
    let rgba = if image.width() == size[0] && image.height() == size[1] {
        image.into_rgba8()
    } else {
        image
            .resize_exact(size[0], size[1], FilterType::CatmullRom)
            .into_rgba8()
    };
    let rgba = rgba.into_raw();
    if rgba.len() != size[0] as usize * size[1] as usize * 4 {
        return Err(MediaError::Invalid(
            "Decoded picture has the wrong byte count".into(),
        ));
    }
    Ok(Frame {
        width: size[0],
        height: size[1],
        rgba: rgba.into(),
    })
}
