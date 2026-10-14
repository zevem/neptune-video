//! The person's own presets: the titles' looks and the graphics they kept,
//! and their brand colours. They belong to the person rather than to a
//! project, and live in one file beside the settings.
//!
//! Giving a preset copies its values into a clip, so a project never
//! depends on this file. The file is read once before the window exists and
//! written on a worker when something changed. One this build cannot read,
//! damaged or made by a newer Aster, is never written over: it is left as
//! it is until a preset is kept, and then moved aside first.
use editor_core::{ClipContent, Time, title};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The version this build writes, and the newest it reads.
pub const VERSION: u32 = 1;
/// The most looks and colours kept: a long list is no longer a palette.
pub const MOST_LOOKS: usize = 200;
pub const MOST_COLORS: usize = 24;
pub const NAME_LENGTH: usize = 64;

/// A title's look or a graphic, kept under a name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedLook {
    pub id: Uuid,
    pub name: String,
    /// A title (with the text it was kept with, and no word times) or a
    /// graphic.
    pub content: ClipContent,
    /// Where on the canvas it was, from the centre, when it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presets {
    pub looks: Vec<SavedLook>,
    /// Colours offered in every colour row of the inspector, in order.
    pub colors: Vec<[u8; 4]>,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    looks: Vec<SavedLook>,
    #[serde(default)]
    colors: Vec<[u8; 4]>,
}

/// What reading the file found.
#[derive(Debug, PartialEq)]
pub enum Read {
    /// There is no file yet.
    Missing,
    Presets(Presets),
    /// The file is there but this build cannot keep what it holds; it is
    /// left as it is. Says why.
    Unreadable(String),
}

/// The longest an animation may be on any clip, and so on a kept one.
const LONGEST_CLIP: Time = Time(3_600_000_000);

/// Whether `content` is something a preset may hold: a title or graphic
/// that is within bounds on a clip long enough for its animations.
pub fn keepable(content: &ClipContent) -> bool {
    let frame = Time(1);
    match content {
        ClipContent::Media { .. } => false,
        ClipContent::Text {
            text,
            font_size,
            style,
            enter,
            exit,
            words,
            ..
        } => {
            !text.trim().is_empty()
                && text.len() <= 4096
                && words.is_empty()
                && font_size.is_finite()
                && (1.0..=4096.0).contains(font_size)
                && style.validate()
                && [enter, exit]
                    .into_iter()
                    .flatten()
                    .all(|animation| animation.validate(frame, LONGEST_CLIP))
        }
        ClipContent::Graphic { size, elements } => {
            title::graphic_valid(*size, elements, frame, LONGEST_CLIP)
        }
    }
}

/// A name a preset may be kept under, or `None`.
pub fn name_of(name: &str) -> Option<String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).collect();
    (!name.is_empty()).then(|| name.chars().take(NAME_LENGTH).collect())
}

impl Presets {
    /// Where the file lives: beside the settings.
    pub fn path_beside(settings: &Path) -> Option<PathBuf> {
        settings.parent().map(|dir| dir.join("presets.json"))
    }

    pub fn read(path: &Path) -> Read {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Read::Missing,
            Err(error) => return Read::Unreadable(error.to_string()),
        };
        // The version first: a newer file may hold what this build's
        // types cannot.
        #[derive(Deserialize)]
        struct Version {
            version: u32,
        }
        match serde_json::from_slice::<Version>(&bytes) {
            Ok(Version { version }) if version > VERSION => {
                return Read::Unreadable(format!(
                    "they were saved by a newer Aster (version {version})"
                ));
            }
            Ok(_) => {}
            Err(error) => return Read::Unreadable(format!("the file is damaged ({error})")),
        }
        match serde_json::from_slice::<File>(&bytes) {
            Ok(file) => Read::Presets(Self::sanitized(file.looks, file.colors)),
            Err(error) => Read::Unreadable(format!("the file is damaged ({error})")),
        }
    }

    /// What may be kept of what was read: a look out of bounds, a second
    /// look with the same ID and what passes the limits are dropped.
    fn sanitized(looks: Vec<SavedLook>, colors: Vec<[u8; 4]>) -> Self {
        let mut ids = std::collections::HashSet::new();
        let looks = (looks.into_iter())
            .filter_map(|mut look| {
                look.name = name_of(&look.name)?;
                if look
                    .position
                    .is_some_and(|at| at.iter().any(|v| !v.is_finite() || v.abs() > 1.0e6))
                {
                    look.position = None;
                }
                (keepable(&look.content) && ids.insert(look.id)).then_some(look)
            })
            .take(MOST_LOOKS)
            .collect();
        let mut kept: Vec<[u8; 4]> = Vec::new();
        for color in colors {
            if !kept.contains(&color) && kept.len() < MOST_COLORS {
                kept.push(color);
            }
        }
        Self {
            looks,
            colors: kept,
        }
    }

    /// Keeps `content` under `name`, first in the list, placed at
    /// `position` when it is given. Hands back its ID, or why it cannot be
    /// kept.
    pub fn keep(
        &mut self,
        name: &str,
        content: ClipContent,
        position: Option<[f32; 2]>,
    ) -> Result<Uuid, String> {
        let name = name_of(name).ok_or("A preset needs a name.")?;
        let content = match content {
            ClipContent::Text {
                text,
                font_size,
                color,
                style,
                enter,
                exit,
                ..
            } => ClipContent::Text {
                text,
                font_size,
                color,
                style,
                enter,
                exit,
                words: Vec::new(),
            },
            other => other,
        };
        if !keepable(&content) {
            return Err("Only a title with text, or a graphic, can be kept.".into());
        }
        if self.looks.len() >= MOST_LOOKS {
            return Err(format!(
                "There are already {MOST_LOOKS} presets: remove one to keep another."
            ));
        }
        let id = Uuid::new_v4();
        let position = position.filter(|at| at.iter().all(|v| v.is_finite() && v.abs() <= 1.0e6));
        self.looks.insert(
            0,
            SavedLook {
                id,
                name,
                content,
                position,
            },
        );
        Ok(id)
    }

    pub fn rename(&mut self, id: Uuid, name: &str) -> bool {
        match (
            self.looks.iter_mut().find(|look| look.id == id),
            name_of(name),
        ) {
            (Some(look), Some(name)) => {
                look.name = name;
                true
            }
            _ => false,
        }
    }

    pub fn remove(&mut self, id: Uuid) -> Option<SavedLook> {
        let index = self.looks.iter().position(|look| look.id == id)?;
        Some(self.looks.remove(index))
    }

    /// Adds a brand colour at the end; one already there is not repeated.
    pub fn add_color(&mut self, color: [u8; 4]) -> bool {
        if self.colors.contains(&color) || self.colors.len() >= MOST_COLORS {
            return false;
        }
        self.colors.push(color);
        true
    }

    /// Writes beside the destination and renames over it, so a crash never
    /// leaves half a file. When `aside` is set the file there is one this
    /// build could not read: it is moved to a name of its own first, and
    /// that name is handed back.
    pub fn save(&self, path: &Path, aside: bool) -> std::io::Result<Option<PathBuf>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let kept = if aside && path.exists() {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs());
            let mut target = path.with_file_name(format!("presets-unread-{stamp}.json"));
            let mut n = 1;
            while target.exists() {
                n += 1;
                target = path.with_file_name(format!("presets-unread-{stamp}-{n}.json"));
            }
            std::fs::rename(path, &target)?;
            Some(target)
        } else {
            None
        };
        let file = File {
            version: VERSION,
            looks: self.looks.clone(),
            colors: self.colors.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, path)?;
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Element, ElementKind, TextStyle};

    fn dir() -> PathBuf {
        std::env::temp_dir().join(format!("aster-presets-{}", Uuid::new_v4()))
    }

    fn title(text: &str) -> ClipContent {
        ClipContent::Text {
            text: text.into(),
            font_size: 96.0,
            color: [255, 220, 0, 255],
            style: TextStyle::default(),
            enter: None,
            exit: None,
            words: vec![editor_core::WordTime {
                at: Time::ZERO,
                end: Time(1),
            }],
        }
    }

    #[test]
    fn a_kept_look_survives_a_round_trip_without_its_word_times() {
        let dir = dir();
        let path = dir.join("presets.json");
        assert_eq!(Presets::read(&path), Read::Missing);
        let mut presets = Presets::default();
        let id = presets
            .keep("  Hook ", title("Watch"), Some([0.0, 300.0]))
            .unwrap();
        let bar = Element::new(
            "Bar",
            ElementKind::Rect {
                color: [255, 0, 0, 255],
                radius: 0.0,
                stroke: None,
            },
            [0.0, 0.0, 10.0, 10.0],
        );
        presets
            .keep(
                "Bar",
                ClipContent::Graphic {
                    size: [10.0, 10.0],
                    elements: vec![bar],
                },
                None,
            )
            .unwrap();
        assert!(presets.add_color([1, 2, 3, 255]));
        assert!(!presets.add_color([1, 2, 3, 255]), "a colour is kept once");
        assert_eq!(presets.save(&path, false).unwrap(), None);
        let Read::Presets(read) = Presets::read(&path) else {
            panic!("the file is read back");
        };
        assert_eq!(read, presets);
        assert_eq!(read.looks[1].name, "Hook");
        assert_eq!(read.looks[1].id, id);
        assert!(matches!(
            &read.looks[1].content,
            ClipContent::Text { words, .. } if words.is_empty()
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_newer_or_damaged_file_is_left_alone_and_moved_aside_before_a_save() {
        let dir = dir();
        let path = dir.join("presets.json");
        std::fs::create_dir_all(&dir).unwrap();
        let newer = r#"{"version": 9, "looks": [], "kits": {"logo": "x"}}"#;
        std::fs::write(&path, newer).unwrap();
        let Read::Unreadable(why) = Presets::read(&path) else {
            panic!("a newer file is not read");
        };
        assert!(why.contains("newer"), "{why}");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(Presets::read(&path), Read::Unreadable(_)));

        std::fs::write(&path, newer).unwrap();
        let mut presets = Presets::default();
        presets.keep("Hook", title("Watch"), None).unwrap();
        let aside = presets.save(&path, true).unwrap().expect("set aside");
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), newer);
        assert!(matches!(Presets::read(&path), Read::Presets(read) if read == presets));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_a_preset_cannot_hold_is_refused_or_dropped_on_reading() {
        let mut presets = Presets::default();
        assert!(presets.keep("", title("Watch"), None).is_err());
        assert!(presets.keep("Empty", title("   "), None).is_err());
        let media = ClipContent::Media {
            asset_id: Uuid::new_v4(),
        };
        assert!(presets.keep("Media", media, None).is_err());
        let mut broken = title("Watch");
        if let ClipContent::Text { font_size, .. } = &mut broken {
            *font_size = f32::NAN;
        }
        let good = SavedLook {
            id: Uuid::new_v4(),
            name: "Good".into(),
            content: ClipContent::Text {
                text: "Watch".into(),
                font_size: 96.0,
                color: [255; 4],
                style: TextStyle::default(),
                enter: None,
                exit: None,
                words: Vec::new(),
            },
            position: None,
        };
        let read = Presets::sanitized(
            vec![
                good.clone(),
                SavedLook {
                    name: "Broken".into(),
                    content: broken,
                    ..good.clone()
                },
                SavedLook {
                    id: Uuid::new_v4(),
                    name: "\n".into(),
                    ..good.clone()
                },
                // The same ID twice: the first is kept.
                SavedLook {
                    name: "Again".into(),
                    ..good.clone()
                },
            ],
            vec![[0; 4], [0; 4], [9; 4]],
        );
        assert_eq!(read.looks, [good]);
        assert_eq!(read.colors, [[0; 4], [9; 4]]);
    }
}
