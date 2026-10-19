//! Neptune Video's desktop, shared by the `neptune-video` window, the `neptune-video-agent` command
//! line and the `neptune-video-mcp` server. The domain lives in the workspace crates;
//! this library composes them.

mod agent;
mod ai;
pub mod app;
mod audio;
mod clipboard;
mod compose;
mod duck;
mod export;
mod extract;
mod file_drop;
pub mod headless;
mod icons;
mod languages;
pub mod mcp;
mod normalize;
mod pacing;
mod peaks;
mod presets;
mod proxies;
mod record;
mod rules;
pub mod settings;
mod speech;
mod speed;
mod steady;
mod stems;
mod textures;
mod theme;
mod timefmt;
mod titles;
mod ui;
