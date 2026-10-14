//! Preferences that belong to the person rather than to a project: how the
//! window looks, how the panels are arranged and which projects were open.
//!
//! The file is read once before the window exists and written on a worker
//! when something changed, so a frame never waits for the disk.
use crate::{
    ai::AiSettings,
    speech::SpeechSettings,
    theme::{Accent, Appearance},
    ui::library::LibrarySort,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RECENT_LIMIT: usize = 8;

/// How far the whole interface can be scaled. Below half its size text is
/// not read; above three times the editor's least window has no room left
/// for its panels.
pub const INTERFACE_SCALE: std::ops::RangeInclusive<f32> = 0.5..=3.0;

/// The least room, in points at 100%, the editor is laid out for: the
/// window's own least size.
pub const LEAST_WINDOW: [f32; 2] = [720.0, 520.0];

/// The largest scale, in tenths, at which a window `size` across at 100%
/// still gives the editor its least room. A window no larger than that
/// holds 100%; one not measured yet holds any scale.
pub fn largest_scale(size: [f32; 2]) -> f32 {
    if !(size[0] > 0.0 && size[1] > 0.0) {
        return *INTERFACE_SCALE.end();
    }
    let room = (size[0] / LEAST_WINDOW[0]).min(size[1] / LEAST_WINDOW[1]);
    // A hair over, so that a window of exactly the room's size is not
    // rounded down below it.
    ((room * 10.0 + 1e-3).floor() / 10.0).clamp(1.0, *INTERFACE_SCALE.end())
}

/// A step of the interface's scale, as Cmd or Ctrl with plus, minus and 0
/// take it in a browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Larger,
    Smaller,
    Actual,
}

impl Scale {
    /// The scale one step on from `scale`, in tenths and within
    /// [`INTERFACE_SCALE`].
    pub fn applied_to(self, scale: f32) -> f32 {
        let stepped = match self {
            Scale::Larger => scale + 0.1,
            Scale::Smaller => scale - 0.1,
            Scale::Actual => 1.0,
        };
        // Tenths, so that steps out and back come home to 100% exactly.
        ((stepped * 10.0).round() / 10.0).clamp(*INTERFACE_SCALE.start(), *INTERFACE_SCALE.end())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub accent: Accent,
    pub snapping: bool,
    pub autosave: bool,
    pub library_grid: bool,
    pub library_sort: LibrarySort,
    pub show_library: bool,
    pub show_inspector: bool,
    /// The size of the whole interface: 1 is as designed.
    pub interface_scale: f32,
    /// The trailing panel shows the agent rather than the inspector.
    pub agent_panel: bool,
    /// The agent panel's field asks the model rather than running a command.
    pub agent_ask: bool,
    /// Which model answers. No credential is kept here or anywhere else.
    pub ai: AiSettings,
    pub library_width: f32,
    pub inspector_width: f32,
    /// Share of the content height the timeline takes.
    pub timeline_share: f32,
    pub recent: Vec<PathBuf>,
    /// The transitions kept as the defaults of their kinds: what a new one
    /// of that kind starts as.
    pub transitions: Vec<editor_core::Transition>,
    /// What captions from speech are made with.
    pub speech: SpeechSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            accent: Accent::default(),
            snapping: true,
            autosave: true,
            library_grid: true,
            library_sort: LibrarySort::default(),
            show_library: true,
            show_inspector: true,
            interface_scale: 1.0,
            agent_panel: false,
            agent_ask: true,
            ai: AiSettings::default(),
            library_width: 300.0,
            inspector_width: 300.0,
            timeline_share: 0.42,
            recent: Vec::new(),
            transitions: Vec::new(),
            speech: SpeechSettings::default(),
        }
    }
}

impl Settings {
    /// Where the settings file lives, or `None` on a system without a home.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("app", "Aster", "Aster")
            .map(|dirs| dirs.config_dir().join("settings.json"))
    }

    /// A missing or unreadable file gives the defaults: preferences are never
    /// a reason not to start.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .map(Settings::sanitized)
            .unwrap_or_default()
    }

    fn sanitized(mut self) -> Self {
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        self.library_width = finite(self.library_width, 300.0).clamp(220.0, 520.0);
        self.inspector_width = finite(self.inspector_width, 300.0).clamp(240.0, 520.0);
        self.timeline_share = finite(self.timeline_share, 0.42).clamp(0.2, 0.7);
        self.interface_scale = finite(self.interface_scale, 1.0)
            .clamp(*INTERFACE_SCALE.start(), *INTERFACE_SCALE.end());
        self.recent.truncate(RECENT_LIMIT);
        self.ai = self.ai.sanitized();
        self.transitions = crate::ui::transitions::sanitized(self.transitions);
        self
    }

    pub fn remember(&mut self, path: &Path) {
        self.recent.retain(|recent| recent != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(RECENT_LIMIT);
    }

    pub fn forget(&mut self, path: &Path) {
        self.recent.retain(|recent| recent != path);
    }

    /// Writes beside the destination and renames over it, so a crash never
    /// leaves half a file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_projects_keep_the_newest_first_without_duplicates() {
        let mut settings = Settings::default();
        settings.remember(Path::new("/a.aster"));
        settings.remember(Path::new("/b.aster"));
        settings.remember(Path::new("/a.aster"));
        assert_eq!(
            settings.recent,
            [PathBuf::from("/a.aster"), PathBuf::from("/b.aster")]
        );
    }

    #[test]
    fn unusable_panel_sizes_are_brought_back_into_range() {
        let settings = Settings {
            library_width: f32::NAN,
            inspector_width: 9000.0,
            timeline_share: 0.0,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(settings.library_width, 300.0);
        assert_eq!(settings.inspector_width, 520.0);
        assert_eq!(settings.timeline_share, 0.2);
    }

    #[test]
    fn the_interface_scale_steps_by_tenths_within_its_range() {
        let mut scale = 1.0;
        for _ in 0..3 {
            scale = Scale::Larger.applied_to(scale);
        }
        assert_eq!(scale, 1.3);
        for _ in 0..3 {
            scale = Scale::Smaller.applied_to(scale);
        }
        // Back exactly where it started, with no drift from the steps.
        assert_eq!(scale, 1.0);
        assert_eq!(Scale::Larger.applied_to(*INTERFACE_SCALE.end()), 3.0);
        assert_eq!(Scale::Smaller.applied_to(*INTERFACE_SCALE.start()), 0.5);
        assert_eq!(Scale::Actual.applied_to(2.4), 1.0);
    }

    #[test]
    fn a_window_holds_the_scales_that_leave_the_editor_its_least_room() {
        assert_eq!(largest_scale([1280.0, 800.0]), 1.5);
        assert_eq!(largest_scale([1920.0, 1080.0]), 2.0);
        assert_eq!(largest_scale([1440.0, 1040.0]), 2.0);
        assert_eq!(largest_scale([5120.0, 2880.0]), 3.0);
        // The least window, or one a system made smaller, holds 100%.
        assert_eq!(largest_scale(LEAST_WINDOW), 1.0);
        assert_eq!(largest_scale([400.0, 300.0]), 1.0);
        // Before the window is measured nothing is held back.
        assert_eq!(largest_scale([0.0, 0.0]), 3.0);
        assert_eq!(largest_scale([f32::NAN, 800.0]), 3.0);
    }

    #[test]
    fn an_unusable_interface_scale_is_brought_back_into_range() {
        for (kept, read) in [
            (f32::NAN, 1.0),
            (f32::INFINITY, 1.0),
            (0.1, 0.5),
            (9.0, 3.0),
            (1.4, 1.4),
        ] {
            let settings = Settings {
                interface_scale: kept,
                ..Settings::default()
            }
            .sanitized();
            assert_eq!(settings.interface_scale, read, "{kept}");
        }
        // A file from before the scale was kept opens at the designed size.
        let settings: Settings = serde_json::from_str(r#"{"snapping":false}"#).unwrap();
        assert_eq!(settings.interface_scale, 1.0);
    }

    #[test]
    fn the_interface_scale_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("aster-settings-{}", uuid::Uuid::new_v4()));
        let path = dir.join("settings.json");
        let settings = Settings {
            interface_scale: 1.3,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).interface_scale, 1.3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_or_missing_fields_fall_back_to_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"snapping":false}"#).unwrap();
        assert!(!settings.snapping);
        assert!(settings.show_library);
        assert!(settings.agent_ask);
        assert_eq!(settings.ai, AiSettings::default());
    }

    #[test]
    fn a_kept_transition_default_survives_a_round_trip_and_a_broken_one_does_not() {
        use editor_core::{Ease, Time, Transition, TransitionKind};
        let dir = std::env::temp_dir().join(format!("aster-settings-{}", uuid::Uuid::new_v4()));
        let path = dir.join("settings.json");
        let wipe = Transition {
            kind: TransitionKind::WipeLeft,
            duration: Time::from_seconds(1.5),
            ease: Ease::Linear,
            params: vec![editor_core::ParamValue {
                key: editor_core::ParamKey::Softness,
                value: 0.25,
            }],
        };
        let mut broken = wipe.clone();
        broken.kind = TransitionKind::Fade;
        let settings = Settings {
            transitions: vec![wipe.clone(), broken],
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        // A fade has no softness: that default is not one a clip would take.
        assert_eq!(Settings::load(&path).transitions, [wipe]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_agent_choices_survive_a_round_trip() {
        let mut settings = Settings::default();
        settings.ai.provider = crate::ai::Provider::Codex;
        settings.ai.effort = crate::ai::Effort::Low;
        settings.ai.set_model("gpt-5");
        settings.agent_ask = false;
        let text = serde_json::to_string(&settings).unwrap();
        let read: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(read, settings);
    }
}
