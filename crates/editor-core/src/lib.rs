//! Deterministic editing model shared by the desktop and agent transports.
//!
//! No filesystem, decoder, device or GUI dependencies belong in this crate.
//! [`PreparedEdit`] retains the exact validated result of a command for guarded
//! off-thread review and a later commit through the same history boundary.
pub mod animate;
mod command;
mod editor;
pub mod look;
mod model;
mod playback;
mod query;
pub mod render;
pub mod retime;

pub use animate::{AnimTrack, Ease, Keyframe, ParamKey};
pub use command::*;
pub use editor::*;
pub use look::{
    AnimTarget, BlendMode, Effect, EffectKind, Follow, LookPreset, Mask, MaskOp, MaskShape,
    ParamSpec, ParamValue, TrackKey, Tracker, Transition, TransitionEdge, TransitionKind,
};
pub use model::*;
pub use playback::*;
pub use query::*;
pub use render::{
    ClipOverride, Layer, LayerSource, Reading, RenderPlan, ResolvedEffect, ResolvedTransition,
    gain_at, resolve,
};
pub use retime::{Retime, TimeKey};
