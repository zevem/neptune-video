//! Preferences that belong to the person rather than to a project: how the
//! window looks, how the panels are arranged and which projects were open.
//!
//! The file is read once before the window exists and written on a worker
//! when something changed, so a frame never waits for the disk.
use crate::{
    ai::AiSettings,
    theme::{Accent, Appearance},
    ui::library::LibrarySort,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RECENT_LIMIT: usize = 8;

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
            agent_panel: false,
            agent_ask: true,
            ai: AiSettings::default(),
            library_width: 300.0,
            inspector_width: 300.0,
            timeline_share: 0.42,
            recent: Vec::new(),
            transitions: Vec::new(),
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
