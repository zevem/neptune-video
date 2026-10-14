//! Versioned project storage, atomic replacement and structurally shared recovery.
mod document;
mod lease;
mod media;
mod recovery;
mod worker;
pub use document::{DocumentSession, load_project_with_session, save_project_with_session};
pub use lease::RecoveryLease;
pub use media::{media_folder, new_take};
pub use worker::*;

use editor_core::{Editor, Project};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Written by every save; older builds refuse it instead of stripping data.
/// Version 3 added styled, animated and timed titles, graphics, caption
/// languages and font assets. Version 4 added clip fades, track volume and
/// solo, and audio effects.
pub const SCHEMA_VERSION: u32 = 4;
/// Oldest version read; versions 1 to 3 are strict subsets through serde
/// defaults.
const MIN_SCHEMA_VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

struct BudgetWriter<W> {
    inner: W,
    remaining: u64,
}
impl<W: Write> Write for BudgetWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(std::io::Error::other(
                "Project exceeds the 256 MiB storage limit",
            ));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod byte_budget_tests {
    use super::BudgetWriter;
    use std::io::Write;

    #[test]
    fn a_serialization_budget_refuses_excess_before_touching_the_destination() {
        let mut writer = BudgetWriter {
            inner: Vec::new(),
            remaining: 5,
        };
        writer.write_all(b"abc").unwrap();
        assert!(writer.write_all(b"def").is_err());
        assert_eq!(writer.inner, b"abc");
        writer.write_all(b"de").unwrap();
        assert!(writer.write_all(b"f").is_err());
        assert_eq!(writer.inner, b"abcde");
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Storage error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid project file: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Project schema {0} is newer than this editor supports")]
    FutureSchema(u32),
    #[error("Invalid project: {0}")]
    Edit(#[from] editor_core::EditError),
    #[error("{0}")]
    Invalid(String),
    #[error("Save worker is unavailable")]
    WorkerStopped,
    #[error("Too many save destinations are pending")]
    QueueFull,
    #[error("Save flush exceeded its deadline")]
    FlushTimeout,
    #[error("The project changed on disk; save your edits to another file or reopen it: {}", .0.display())]
    DocumentConflict(PathBuf),
    #[error("Another writer is saving this project; try again: {}", .0.display())]
    DocumentBusy(PathBuf),
    #[error("Recovery is active in another editor: {}", .0.display())]
    RecoveryInUse(PathBuf),
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    schema_version: u32,
    project: Project,
    #[serde(default)]
    checkpoint_label: Option<String>,
}

fn checked_reader(path: &Path) -> Result<BufReader<fs::File>, StoreError> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_FILE_BYTES {
        return Err(StoreError::Invalid(
            "Project exceeds the 256 MiB storage limit".into(),
        ));
    }
    Ok(BufReader::new(file))
}
fn protect_future(path: &Path) -> Result<(), StoreError> {
    if path.exists() {
        let value = read_versioned(path)?;
        if value.get("project").is_some() {
            let existing: ProjectFile = serde_json::from_value(value)?;
            existing.project.validate()?;
        } else if value.get("projects").is_some() {
            let _ = recovery::decode(value, None)?;
        } else {
            return Err(StoreError::Invalid("Existing file is not a valid project or recovery archive; refusing to overwrite it".into()));
        }
    }
    Ok(())
}
fn validate_version(value: &serde_json::Value) -> Result<(), StoreError> {
    let version = value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| StoreError::Invalid("File has no schema version".into()))?;
    if version > u64::from(SCHEMA_VERSION) {
        return Err(StoreError::FutureSchema(version.min(u32::MAX as u64) as u32));
    }
    if version < u64::from(MIN_SCHEMA_VERSION) {
        return Err(StoreError::Invalid("Unsupported project schema".into()));
    }
    Ok(())
}
fn read_versioned(path: &Path) -> Result<serde_json::Value, StoreError> {
    let value: serde_json::Value = serde_json::from_reader(checked_reader(path)?)?;
    validate_version(&value)?;
    Ok(value)
}

/// Serialize and sync before holding the destination's short write lock.
fn prepare_json(
    path: &Path,
    value: &impl Serialize,
) -> Result<tempfile::NamedTempFile, StoreError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut writer = BufWriter::new(BudgetWriter {
            inner: temp.as_file_mut(),
            remaining: MAX_FILE_BYTES,
        });
        serde_json::to_writer(&mut writer, value)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    if temp.as_file().metadata()?.len() > MAX_FILE_BYTES {
        return Err(StoreError::Invalid(
            "Project exceeds the 256 MiB storage limit".into(),
        ));
    }
    temp.as_file().sync_all()?;
    Ok(temp)
}

fn replace_json(path: &Path, temp: tempfile::NamedTempFile) -> Result<(), StoreError> {
    temp.persist(path).map_err(|e| StoreError::Io(e.error))?;
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), StoreError> {
    protect_future(path)?;
    replace_json(path, prepare_json(path, value)?)
}

/// One-shot unconditional replacement of a valid project.
///
/// This compatibility API coordinates its replacement with protected writers,
/// but has no open-document baseline and therefore cannot detect stale edits.
/// Interactive/headless editing must use [`save_project_with_session`] instead.
pub fn save_project(path: &Path, project: &Project) -> Result<(), StoreError> {
    project.validate()?;
    let path = document::canonical_destination(path)?;
    let _write = document::WriteLease::acquire(&path)?;
    atomic_json(
        &path,
        &ProjectFile {
            schema_version: SCHEMA_VERSION,
            project: project.clone(),
            checkpoint_label: None,
        },
    )
}
pub fn load_project(path: &Path) -> Result<Project, StoreError> {
    decode_project(read_versioned(path)?, path)
}
fn decode_project(value: serde_json::Value, path: &Path) -> Result<Project, StoreError> {
    let file: ProjectFile = serde_json::from_value(value)?;
    let mut project = file.project;
    // A relative reference is resolved from the document, never process cwd.
    let base = path
        .canonicalize()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    for asset in std::sync::Arc::make_mut(&mut project.assets).values_mut() {
        if asset.path.is_relative() {
            asset.path = base.join(&asset.path);
        }
    }
    project.validate()?;
    Ok(project)
}
pub fn recovery_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".recovery");
    PathBuf::from(name)
}
pub fn save_recovery(path: &Path, editor: &Editor) -> Result<(), StoreError> {
    recovery::save(&recovery_path(path), &editor.session_state())
}
/// Writes a named recovery archive paired with this exact document baseline.
/// This synchronous API belongs on a storage worker; interactive callers use
/// [`SaveWorker::request_recovery`]. Private/unsaved archives use [`save_recovery`].
pub fn save_recovery_with_document(
    document: &DocumentSession,
    editor: &Editor,
) -> Result<(), StoreError> {
    let state = editor.session_state();
    let binding = document.recovery_binding();
    recovery::save_with_binding(&recovery_path(document.path()), &state, Some(binding))
}
/// Recovered history plus the ordinary document ownership captured at crash.
/// An absent document means unsaved work: never infer a fresh disk baseline.
pub struct RecoveredSession {
    pub editor: Editor,
    pub document: Option<std::sync::Arc<DocumentSession>>,
    /// The valid ordinary document differs from the archived baseline. Its
    /// original baseline remains protected for later saves.
    pub document_changed: bool,
}
/// Loads recovery and checks its original ordinary baseline on this thread.
/// Legacy, private, moved, inaccessible/damaged ordinary or cross-platform
/// bindings restore as dirty unsaved work. A changed valid document still
/// restores edits/history, but protected saves
/// retain the original expected bytes and refuse intervening writers.
pub fn load_recovery_with_document(path: &Path) -> Result<RecoveredSession, StoreError> {
    recovery::load_with_document(&recovery_path(path), path)
}
/// Compatibility history-only loader. It returns no ordinary save authority;
/// opening a recovery archive must never be followed by silently preparing a
/// fresh ordinary baseline. Use [`load_recovery_with_document`] when resuming.
pub fn load_recovery(path: &Path) -> Result<Editor, StoreError> {
    recovery::load(&recovery_path(path))
}
pub fn remove_recovery(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(recovery_path(path)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub fn duplicate_project(project: &Project, name: impl Into<String>) -> Project {
    let mut duplicate = project.clone();
    duplicate.id = editor_core::AssetId::new_v4();
    duplicate.name = name.into();
    duplicate
}

#[derive(Debug, Clone)]
pub struct CheckpointInfo {
    pub path: PathBuf,
    pub label: String,
}
fn checkpoint_dir(path: &Path) -> PathBuf {
    let mut dir = path.as_os_str().to_os_string();
    dir.push(".checkpoints");
    PathBuf::from(dir)
}
pub fn checkpoint(path: &Path, project: &Project, label: &str) -> Result<PathBuf, StoreError> {
    project.validate()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StoreError::Invalid("System clock precedes Unix epoch".into()))?
        .as_millis();
    let path = checkpoint_dir(path).join(format!("{now}-{}.aster", editor_core::AssetId::new_v4()));
    atomic_json(
        &path,
        &ProjectFile {
            schema_version: SCHEMA_VERSION,
            project: project.clone(),
            checkpoint_label: Some(label.into()),
        },
    )?;
    Ok(path)
}
pub fn list_checkpoints(path: &Path) -> Result<Vec<CheckpointInfo>, StoreError> {
    let dir = checkpoint_dir(path);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "aster") {
            continue;
        }
        let file: ProjectFile = serde_json::from_value(read_versioned(&path)?)?;
        entries.push(CheckpointInfo {
            path,
            label: file.checkpoint_label.unwrap_or_else(|| "Checkpoint".into()),
        });
    }
    entries.sort_by(|a, b| b.path.cmp(&a.path));
    Ok(entries)
}
