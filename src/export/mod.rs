//! Exports: the timeline drawn and encoded to a file, off the interface.
//!
//! [`queue`] is the service the application and the agent talk to: jobs
//! wait, render one at a time on the `neptune-video-render` thread and stay listed
//! when finished. [`engine`] runs one job: the sound, the pictures drawn by
//! [`frame`] at the output's size from the original media, FFmpeg fed
//! through a short pipeline, and the checked file moved into place.
//! [`presets`] keeps the person's saved export presets, [`poster`] draws the
//! export sheet's picture of the output at the playhead with the export's
//! own renderer, and [`wiring`] is what the application decides with all of
//! these.

pub mod engine;
pub mod frame;
pub mod poster;
pub mod presets;
pub mod queue;
pub mod wiring;
