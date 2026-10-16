//! Writing an export with FFmpeg: what this host can encode, the encoder
//! process fed raw pictures, the timeline's sound rendered to a file, source
//! pictures decoded in order at full quality, and the check of the result.
//!
//! Every call here blocks and belongs on a worker thread the desktop owns.
//! Each value owns the processes it starts; dropping it kills and reaps them,
//! and its canceller does the same from any thread. Nothing here composes a
//! picture (ADR 0006): FFmpeg receives finished RGBA frames and converts them
//! to Y'CbCr only. Nothing here reads a proxy: sources are `asset.path`.

mod args;
mod audio;
mod caps;
mod encoder;
mod loudness;
mod reader;
mod verify;

pub use audio::{AudioRender, render_audio};
pub use caps::probe_capabilities;
pub use encoder::Encoder;
pub use reader::{MAX_READ_BYTES, MAX_READ_SIDE, SourceReader};
pub use verify::{Expectation, verify_output};

use editor_core::FrameRate;
use std::path::Path;

/// What an encoder reads besides its settings.
#[derive(Debug, Clone, Copy)]
pub struct EncodeInputs<'a> {
    /// Raw RGBA pictures written to the encoder's input pipe.
    pub video: Option<VideoInput>,
    /// The sound, already rendered to a raw file.
    pub audio: Option<AudioInput<'a>>,
    /// A subtitle file carried as a stream in the output.
    pub subtitles: Option<SubtitleInput<'a>>,
    /// The output's title metadata; empty writes none.
    pub title: &'a str,
}

/// Tightly packed RGBA pictures of `size` at `rate`, one per output frame,
/// from the encoder's input pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoInput {
    pub size: [u32; 2],
    pub rate: FrameRate,
}

/// Interleaved little-endian `f32` samples in a file, as `render_audio`
/// writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioInput<'a> {
    pub path: &'a Path,
    pub rate: u32,
    pub channels: u8,
}

/// An SRT or WebVTT file and the caption track's language tag (BCP 47, such
/// as `en` or `pt-BR`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubtitleInput<'a> {
    pub path: &'a Path,
    pub language: Option<&'a str>,
}

#[cfg(test)]
mod args_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod reader_tests;
