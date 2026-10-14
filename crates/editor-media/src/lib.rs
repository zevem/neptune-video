//! Background media import, bounded preview decoding, motion analysis and
//! native audio output.
//! No public operation performs file, process or device I/O on the caller thread.

mod analysis;
mod audio;
mod decode;
mod import;
mod lut;
mod process;
mod service;
pub mod speech;
pub mod text;

pub use analysis::{
    ANALYSIS_QUEUE, ANALYSIS_SIDE, AnalysisEvent, AnalysisService, Fix, LOST_BELOW,
    MAX_STEADY_FRAMES, Motion, REFRESH_BELOW, Region, STEADY_ALGORITHM, STEADY_CACHE_BYTES,
    STEADY_CACHE_ENTRIES, STEADY_LOADS, STEADY_SECONDS, STEADY_SUGGEST_FRAMES, Sampling,
    SourceIdentity, Steadied, SteadyCache, SteadyEvent, SteadyJob, TrackJob, Tracked,
};
pub use audio::AudioService;
pub use lut::{Cube, CubeError, MAX_CUBE_BYTES, MAX_CUBE_SIZE, MIN_CUBE_SIZE};
pub use service::MediaService;

use editor_core::{Asset, AssetId, MediaFolder, Time};
use std::{path::PathBuf, sync::Arc};

/// Maximum decoded preview size, independent of source resolution.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Total memory retained by a preview worker's decoded-frame cache.
pub const FRAME_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// A transient owner of an independent preview stream. Reuse it while its
/// picture continues, including a contiguous cut; retire it when that owner
/// leaves. It is not part of the durable project or the shared pixel cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreviewConsumer(uuid::Uuid);

impl PreviewConsumer {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for PreviewConsumer {
    fn default() -> Self {
        Self::new()
    }
}

/// An immutable, tightly packed RGBA8 preview frame.
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl Frame {
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }

    pub(crate) fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, MediaError> {
        if rgba.len() != frame_bytes(width, height)? {
            return Err(MediaError::Invalid(
                "Decoded frame has the wrong byte count".into(),
            ));
        }
        Ok(Self {
            width,
            height,
            rgba: rgba.into(),
        })
    }
}

/// A failed individual file does not discard successful imports in the same batch.
#[derive(Clone, Debug)]
pub struct ImportError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug)]
pub enum MediaEvent {
    Imported {
        generation: u64,
        assets: Vec<Asset>,
        folders: Vec<MediaFolder>,
        errors: Vec<ImportError>,
    },
    FrameReady {
        generation: u64,
        asset_id: AssetId,
        time: Time,
        size: [u32; 2],
        /// `None` for legacy asset/size or poster demand. Explicit consumers
        /// remain independent even at equal asset, dimensions and source time.
        consumer: Option<PreviewConsumer>,
        frame: Frame,
    },
    /// The answer to `MediaService::request_lut`. `path` and `file_size`
    /// are the asset's as it was asked about: a table read for what the
    /// asset no longer is belongs to nothing.
    LutLoaded {
        generation: u64,
        asset_id: AssetId,
        path: PathBuf,
        file_size: u64,
        table: Result<Arc<Cube>, String>,
    },
    Error {
        generation: u64,
        asset_id: Option<AssetId>,
        /// Request context for an explicit preview failure. Legacy failures
        /// have no consumer; import-level failures also have no bounds.
        consumer: Option<PreviewConsumer>,
        size: Option<[u32; 2]>,
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error(
        "{tool} was not found. Install FFmpeg and put ffmpeg and ffprobe on PATH, or set ASTER_FFMPEG and ASTER_FFPROBE to their executable paths."
    )]
    MissingTool { tool: String },
    #[error("{0}")]
    Invalid(String),
    #[error("Media worker is busy; retry after pending work finishes")]
    Busy,
    #[error("Media worker has stopped")]
    Closed,
    #[error("Media operation was cancelled")]
    Cancelled,
    #[error("{operation}: {source}")]
    Io {
        operation: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Decode(String),
}

pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

pub(crate) fn io(operation: impl Into<String>, source: std::io::Error) -> MediaError {
    MediaError::Io {
        operation: operation.into(),
        source,
    }
}

pub(crate) fn frame_bytes(width: u32, height: u32) -> Result<usize, MediaError> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(MediaError::Invalid(
            "Preview dimensions must be between 1 and 4096 pixels".into(),
        ));
    }
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    match bytes {
        Some(bytes) if bytes <= MAX_FRAME_BYTES => Ok(bytes),
        _ => Err(MediaError::Invalid(
            "Preview exceeds the 64 MiB frame budget".into(),
        )),
    }
}

pub(crate) fn fitted_dimensions(
    source: [u32; 2],
    target: [u32; 2],
) -> Result<[u32; 2], MediaError> {
    frame_bytes(target[0], target[1])?;
    if source.contains(&0) {
        return Err(MediaError::Invalid("Media has no video dimensions".into()));
    }
    let ratio = (f64::from(target[0]) / f64::from(source[0]))
        .min(f64::from(target[1]) / f64::from(source[1]))
        .min(1.0);
    Ok([
        (f64::from(source[0]) * ratio).round().max(1.0) as u32,
        (f64::from(source[1]) * ratio).round().max(1.0) as u32,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_dimensions_preserve_aspect_and_reject_unbounded_frames() {
        assert_eq!(
            fitted_dimensions([1920, 1080], [320, 320]).unwrap(),
            [320, 180]
        );
        assert!(frame_bytes(u32::MAX, u32::MAX).is_err());
        assert!(Frame::new(2, 2, vec![0; 15]).is_err());
    }
}
