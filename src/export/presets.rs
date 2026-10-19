//! The person's saved export presets, kept in `export-presets.json` beside
//! the settings (rule 18).
//!
//! The file is `{ "version": 1, "presets": [ … ] }`. Each preset is read on
//! its own: one this build cannot read (made by a newer Neptune Video, or damaged by
//! hand) is kept as it was read and written back unchanged, so saving a
//! preset never loses another. A file that is not such an object at all, or
//! that names a newer version, is never written over: it is moved aside
//! before the first write, as the looks presets are. A saved preset carries
//! neither a range nor a caption track.
use editor_core::export::{Destination, ExportPreset, ExportRange, ExportSettings};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The version this build writes, and the newest it reads.
pub const VERSION: u32 = 1;
/// The most presets kept, counting those this build cannot read.
pub const MOST_PRESETS: usize = 100;
/// The longest name a preset is kept under, in characters.
pub const NAME_LENGTH: usize = 64;
/// The file's name beside the settings.
pub const FILE_NAME: &str = "export-presets.json";

/// One entry of the file.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Preset(ExportPreset),
    /// An entry this build cannot read, exactly as it was read.
    Kept(serde_json::Value),
}

/// The person's export presets, in the order they are offered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportPresets {
    entries: Vec<Entry>,
}

/// What reading the file found.
#[derive(Debug, PartialEq)]
pub enum Read {
    /// There is no file yet.
    Missing,
    Presets(ExportPresets),
    /// The file is there but is not one this build may write over. Says why.
    Unreadable(String),
}

#[derive(Serialize)]
struct FileOut<'a> {
    version: u32,
    presets: Vec<&'a serde_json::Value>,
}

/// A name a preset may be kept under, or `None`: trimmed, without control
/// characters and at most [`NAME_LENGTH`] characters.
pub fn name_of(name: &str) -> Option<String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).collect();
    let name: String = name.chars().take(NAME_LENGTH).collect();
    let name = name.trim_end().to_owned();
    (!name.is_empty()).then_some(name)
}

/// Settings as a preset keeps them: no range, no caption track.
pub fn presettable(settings: &ExportSettings) -> ExportSettings {
    let mut settings = settings.clone();
    settings.range = ExportRange::Timeline;
    settings.subtitles.track = None;
    settings
}

/// One entry as this build reads it: a preset of the person's with a
/// usable name and settings stripped of range and track, or `None`.
fn preset_of(value: &serde_json::Value) -> Option<ExportPreset> {
    let mut preset = ExportPreset::deserialize(value).ok()?;
    preset.name = name_of(&preset.name)?;
    if preset.id.trim().is_empty() {
        return None;
    }
    preset.destination = Destination::Custom;
    preset.settings = presettable(&preset.settings);
    Some(preset)
}

impl ExportPresets {
    /// Where the file lives: beside the settings.
    pub fn path_beside(settings: &Path) -> Option<PathBuf> {
        settings.parent().map(|dir| dir.join(FILE_NAME))
    }

    pub fn read(path: &Path) -> Read {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Read::Missing,
            Err(error) => return Read::Unreadable(error.to_string()),
        };
        match Self::parse(&bytes) {
            Ok(presets) => Read::Presets(presets),
            Err(why) => Read::Unreadable(why),
        }
    }

    /// Reads the file's bytes. Only the outer shape and the version can
    /// refuse the whole file; each preset stands alone.
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("the file is damaged ({error})"))?;
        let object = value
            .as_object()
            .ok_or_else(|| "the file is damaged (it is not an object)".to_owned())?;
        match object.get("version").and_then(serde_json::Value::as_u64) {
            Some(version) if version > u64::from(VERSION) => {
                return Err(format!(
                    "they were saved by a newer Neptune Video (version {version})"
                ));
            }
            Some(_) => {}
            None => return Err("the file is damaged (it names no version)".to_owned()),
        }
        let listed = match object.get("presets") {
            None | Some(serde_json::Value::Null) => Vec::new(),
            Some(serde_json::Value::Array(listed)) => listed.clone(),
            Some(_) => return Err("the file is damaged (its presets are not a list)".to_owned()),
        };
        let mut ids = std::collections::HashSet::new();
        let entries = (listed.into_iter())
            .take(MOST_PRESETS)
            .map(|value| match preset_of(&value) {
                // A second preset with an id already read is kept as it was.
                Some(preset) if ids.insert(preset.id.clone()) => Entry::Preset(preset),
                _ => Entry::Kept(value),
            })
            .collect();
        Ok(Self { entries })
    }

    /// The presets this build can offer, every one with
    /// [`Destination::Custom`].
    pub fn presets(&self) -> impl Iterator<Item = &ExportPreset> {
        self.entries.iter().filter_map(|entry| match entry {
            Entry::Preset(preset) => Some(preset),
            Entry::Kept(_) => None,
        })
    }

    /// Keeps `settings` under `name`, without range or caption track. A
    /// preset of the same name takes the new settings in its place; a new
    /// one goes last. Hands back its id, or why it cannot be kept.
    pub fn keep(&mut self, name: &str, settings: &ExportSettings) -> Result<String, String> {
        let name = name_of(name).ok_or("A preset needs a name.")?;
        let settings = presettable(settings);
        let note = note_of(&settings);
        let same = self.entries.iter_mut().find_map(|entry| match entry {
            Entry::Preset(preset) if preset.name.eq_ignore_ascii_case(&name) => Some(preset),
            _ => None,
        });
        if let Some(preset) = same {
            preset.name = name;
            preset.settings = settings;
            preset.note = note;
            return Ok(preset.id.clone());
        }
        if self.entries.len() >= MOST_PRESETS {
            return Err(format!(
                "There are already {MOST_PRESETS} export presets: delete one to keep another."
            ));
        }
        let id = Uuid::new_v4().to_string();
        self.entries.push(Entry::Preset(ExportPreset {
            id: id.clone(),
            name,
            destination: Destination::Custom,
            note,
            settings,
        }));
        Ok(id)
    }

    /// Forgets the preset with this id; `None` when there is none.
    pub fn remove(&mut self, id: &str) -> Option<ExportPreset> {
        let index = self
            .entries
            .iter()
            .position(|entry| matches!(entry, Entry::Preset(preset) if preset.id == id))?;
        match self.entries.remove(index) {
            Entry::Preset(preset) => Some(preset),
            Entry::Kept(_) => None,
        }
    }

    /// The file's bytes.
    fn to_bytes(&self) -> std::io::Result<Vec<u8>> {
        let values: Vec<serde_json::Value> = (self.entries.iter())
            .map(|entry| match entry {
                Entry::Preset(preset) => serde_json::to_value(preset),
                Entry::Kept(value) => Ok(value.clone()),
            })
            .collect::<Result<_, _>>()
            .map_err(std::io::Error::other)?;
        let file = FileOut {
            version: VERSION,
            presets: values.iter().collect(),
        };
        serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)
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
            let mut target = path.with_file_name(format!("export-presets-unread-{stamp}.json"));
            let mut n = 1;
            while target.exists() {
                n += 1;
                target = path.with_file_name(format!("export-presets-unread-{stamp}-{n}.json"));
            }
            std::fs::rename(path, &target)?;
            Some(target)
        } else {
            None
        };
        let bytes = self.to_bytes()?;
        let temporary = path.with_file_name(format!(".{FILE_NAME}.{}.tmp", Uuid::new_v4()));
        // Synced through the handle that wrote it: Windows flushes only a
        // file opened for writing.
        let written = (|| {
            let mut file = std::fs::File::create(&temporary)?;
            std::io::Write::write_all(&mut file, &bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written.map(|()| kept)
    }
}

/// The writes of the presets file, shared by the workers that make them:
/// they land in the order they were asked for, and a file that could not be
/// read is moved aside by whichever write lands first, however the writes
/// before it fared.
#[derive(Debug, Default)]
pub struct Writes {
    /// The newest write made or tried.
    newest: u64,
    /// The file there could not be read: it is moved aside before anything
    /// is written over it. Cleared only once a write has succeeded.
    aside: bool,
}

impl Writes {
    /// `unread`: the file there could not be read.
    pub fn new(unread: bool) -> Self {
        Self {
            newest: 0,
            aside: unread,
        }
    }

    /// Writes `presets` to `path` as write number `asked`. `None` when a
    /// newer write was made or tried already: this one is not made.
    pub fn save(
        &mut self,
        presets: &ExportPresets,
        path: &Path,
        asked: u64,
    ) -> Option<std::io::Result<Option<PathBuf>>> {
        if self.newest > asked {
            return None;
        }
        self.newest = asked;
        let saved = presets.save(path, self.aside);
        if saved.is_ok() {
            self.aside = false;
        }
        Some(saved)
    }
}

/// What a saved preset's note says: size, codec and quality, or the sound.
fn note_of(settings: &ExportSettings) -> String {
    use editor_core::export::{RateControl, Resolution};
    let sound = settings.audio.map(|audio| audio.codec.label());
    match &settings.video {
        Some(video) => {
            let size = match video.resolution {
                Resolution::Project => "Project size".to_owned(),
                Resolution::Short { pixels } => format!("{pixels}p"),
                Resolution::Custom { width, height } => format!("{width} × {height}"),
            };
            let quality = match video.rate_control {
                RateControl::Quality { level } => level.label().to_owned(),
                RateControl::Bitrate { kbps } => format!("{kbps} kbit/s"),
            };
            let mut note = format!("{size} · {} · {quality}", video.codec.label());
            if let Some(sound) = sound {
                note.push_str(&format!(" · {sound}"));
            }
            note
        }
        None => format!(
            "{} · {}",
            settings.container.label(),
            sound.unwrap_or("No sound")
        ),
    }
}

#[cfg(test)]
#[path = "presets_tests.rs"]
mod tests;
