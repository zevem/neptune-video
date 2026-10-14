//! A span of a media file's sound written to a WAV file the person chose:
//! the lane's one job, and the editor's first FFmpeg job, that writes a file.
//!
//! FFmpeg writes beside the target, to the target's name with `.part`, a
//! file this job makes before it starts FFmpeg and the only file it ever
//! removes. Once FFmpeg has ended well the part is renamed over the target;
//! on a failure or a cancellation it is removed. A target that was there is
//! replaced by a whole file or not at all, and nothing else is deleted.
//!
//! Before the rename the job claims its last step: a cancellation after that
//! is too late, and the job answers with the file it wrote.
//!
//! What is written is the first sound stream as it is, 16-bit PCM at its own
//! rate and channels, from its first sample: no clip volume, fade, effect or
//! speed, and none of the container's time before the stream starts.
use super::{
    Control,
    loudness::{MAX_LINE, out_time},
};
use crate::{
    MediaError, io,
    process::{Process, Processes, tool_command},
};
use editor_core::{Asset, MediaKind, Time};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// A span of one media file's sound to be written to a file.
#[derive(Clone, Debug)]
pub struct ExtractJob {
    /// The project that asks, handed back with the answer and never read.
    pub project: Uuid,
    pub asset: Asset,
    /// Where the span starts, in source time.
    pub from: Time,
    /// How long it is, in source time. A span from the start to the media's
    /// length or past it is the whole stream, read without a seek or a limit.
    pub span: Time,
    /// The file to write. One that is there is replaced once the sound is
    /// written whole.
    pub to: PathBuf,
    /// Files `to` must not be, by any name it reaches them: the project's
    /// media. The source is never written over either way.
    pub keep: Vec<PathBuf>,
}

impl ExtractJob {
    /// The whole of the media's sound, to `to`.
    pub fn whole(project: Uuid, asset: Asset, to: PathBuf) -> Self {
        Self {
            project,
            from: Time::ZERO,
            span: asset.duration,
            asset,
            to,
            keep: Vec::new(),
        }
    }

    fn is_whole(&self) -> bool {
        self.from == Time::ZERO && self.span >= self.asset.duration
    }
}

/// The file the sound is written to until it is whole: the target's name
/// with `.part`, in the target's folder.
pub fn part_path(to: &Path) -> PathBuf {
    let mut name = to.file_name().map(OsString::from).unwrap_or_default();
    name.push(".part");
    to.with_file_name(name)
}

// Refused here, before any process or file.
pub(super) fn check(job: &ExtractJob) -> Result<(), MediaError> {
    let invalid = |why: &str| Err(MediaError::Invalid(why.into()));
    let asset = &job.asset;
    if !asset.has_audio && asset.kind != MediaKind::Audio {
        return invalid("Media has no sound to save");
    }
    if !asset.duration.is_valid() || asset.duration <= Time::ZERO {
        return invalid("Media has no length");
    }
    if !job.from.is_valid() || !job.span.is_valid() || job.span <= Time::ZERO {
        return invalid("The span to save is empty");
    }
    if job.from >= asset.duration {
        return invalid("The span to save starts after the media ends");
    }
    if job.to.file_name().is_none() {
        return invalid("Choose a file to save the sound to");
    }
    if job.to == asset.path {
        return invalid(OVER_SOURCE);
    }
    if let Some(media) = job.keep.iter().find(|media| **media == job.to) {
        return Err(over_media(media));
    }
    Ok(())
}

const OVER_SOURCE: &str = "The sound is not saved over the media it is read from";

fn over_media(media: &Path) -> MediaError {
    let name = media.file_name().unwrap_or(media.as_os_str());
    MediaError::Invalid(format!(
        "{} is media of the project: its sound is not saved over it",
        name.display()
    ))
}

/// The part file this job made, removed when it is dropped unless it was
/// put in the target's place.
pub(super) struct Part {
    path: PathBuf,
    placed: bool,
}

impl Part {
    /// Makes the part file, which must not be there yet: a file of that name
    /// is someone's, and is left as it is.
    pub(super) fn make(path: PathBuf) -> Result<Self, MediaError> {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            // Closed at once: FFmpeg opens it again to write it.
            Ok(_) => Ok(Self {
                path,
                placed: false,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(MediaError::Invalid(format!(
                    "{} is in the way: a file of that name is already there, and it was left as it is",
                    path.display()
                )))
            }
            Err(error) => Err(io(format!("Cannot write {}", path.display()), error)),
        }
    }

    /// Puts the whole file in the target's place, on disk first: through a
    /// handle that may write it, which Windows asks of a flush, and that
    /// does not cut it short.
    pub(super) fn place(mut self, to: &Path) -> Result<(), MediaError> {
        OpenOptions::new()
            .write(true)
            .open(&self.path)
            .and_then(|file| file.sync_all())
            .map_err(|error| io(format!("Cannot finish {}", self.path.display()), error))?;
        fs::rename(&self.path, to)
            .map_err(|error| io(format!("Cannot write {}", to.display()), error))?;
        self.placed = true;
        Ok(())
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        if !self.placed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The file on disk a path reaches, however it is named: through a link,
/// `..`, or letters of another case where the file system ignores case. The
/// file itself, not a resolved path, which keeps the letters it is given on
/// some systems. `None` when nothing is there.
#[cfg(unix)]
fn identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|meta| (meta.dev(), meta.ino()))
}

/// Windows resolves a path to the name the file has on disk, in its case.
#[cfg(not(unix))]
fn identity(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

/// Writes the job's span through one FFmpeg process, and hands back how much
/// sound was written.
pub(super) fn write(
    job: &ExtractJob,
    processes: &Processes,
    control: &Control,
) -> Result<Time, MediaError> {
    if control.cancelled() {
        return Err(MediaError::Cancelled);
    }
    let asset = &job.asset;
    // The media, or the project's, under another name, which `check`
    // cannot see. A target that is not there is no file of theirs.
    let target = identity(&job.to);
    let over = |media: &Path| {
        *media == job.to || target.is_some() && identity(media).as_ref() == target.as_ref()
    };
    if over(&asset.path) {
        return Err(MediaError::Invalid(OVER_SOURCE.into()));
    }
    if let Some(media) = job.keep.iter().find(|media| over(media)) {
        return Err(over_media(media));
    }
    if job.to.is_dir() {
        return Err(MediaError::Invalid(format!(
            "{} is a folder",
            job.to.display()
        )));
    }
    // Made before the process, so dropped after it: the process is waited
    // for, and its file closed, before the part is removed.
    let part = Part::make(part_path(&job.to))?;
    let whole = job.is_whole();
    // What is written is the span, or what of it the file has.
    let length = if whole {
        asset.duration
    } else {
        job.span.min(Time(asset.duration.0 - job.from.0))
    };
    let mut command = tool_command("ffmpeg");
    command
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-nostats"])
        .args(["-progress", "pipe:1", "-y", "-threads", "1"]);
    if !whole {
        command
            .arg("-ss")
            .arg(format!("{:.6}", job.from.as_seconds()))
            .arg("-t")
            .arg(format!("{:.6}", job.span.as_seconds()));
    }
    command
        .arg("-i")
        .arg(&asset.path)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-c:a", "pcm_s16le"])
        .args(["-f", "wav"])
        .arg(&part.path);
    let mut process = Process::spawn("ffmpeg", &mut command, processes)?;
    let mut bytes = [0u8; 4096];
    let mut line = Vec::with_capacity(MAX_LINE);
    let (mut said, mut written) = (0.0, 0_i64);
    loop {
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        // FFmpeg says how far it is twice a second, so a read that takes
        // the whole deadline is a process that is stuck.
        process.begin_read();
        let outcome = process.stdout.read(&mut bytes);
        let in_time = process.end_read_checked();
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        let count = match outcome {
            Ok(_) if !in_time => return Err(process.failure("the sound took too long to write")),
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(process.failure(&format!("could not read its progress ({error})")));
            }
        };
        for &byte in &bytes[..count] {
            if byte != b'\n' {
                if line.len() < MAX_LINE {
                    line.push(byte);
                }
                continue;
            }
            if let Some(micros) = out_time(&line) {
                written = written.max(micros);
                let done = (micros as f64 / length.0.max(1) as f64).clamp(0.0, 1.0) as f32;
                if done - said >= 0.01 {
                    said = done;
                    control.progress(done);
                }
            }
            line.clear();
        }
    }
    let status = super::await_exit(&process, control)?;
    if !status.success() {
        return Err(process.failure("could not write the sound"));
    }
    if written <= 0 {
        return Err(MediaError::Decode(
            "ffmpeg: no sound was read from that span".into(),
        ));
    }
    drop(process);
    // From here the job is no longer given up: the file is put in place
    // and answered for, or the step fails and says so.
    if !control.claim() {
        return Err(MediaError::Cancelled);
    }
    part.place(&job.to)?;
    Ok(Time(written))
}
