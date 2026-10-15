//! Optimistic document ownership. A session remembers the bytes it opened, and
//! a separate advisory lock makes checking and replacement indivisible for
//! cooperating writers. Recovery ownership is deliberately independent.

use crate::{MAX_FILE_BYTES, ProjectFile, SCHEMA_VERSION, StoreError};
use editor_core::Project;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, BufReader, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileVersion {
    bytes: u64,
    sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Baseline {
    Missing,
    Present(FileVersion),
}

/// An immutable ordinary-document baseline paired with a recovery snapshot.
/// The version is independent of the project format; absent bindings denote
/// legacy archives or recovery owned by an unsaved/private window.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RecoveryDocumentBinding {
    schema_version: u32,
    path: PathBuf,
    baseline: Baseline,
}

impl RecoveryDocumentBinding {
    pub(super) fn validate(&self) -> Result<(), StoreError> {
        if self.schema_version != 1
            || matches!(self.baseline, Baseline::Present(version) if version.bytes > MAX_FILE_BYTES)
        {
            return Err(StoreError::Invalid(
                "Invalid recovery document binding".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn restore(
        self,
        destination: &Path,
    ) -> Result<Option<(Arc<DocumentSession>, bool)>, StoreError> {
        self.validate()?;
        // Recovery copied from another platform can retain all editing history
        // while its native destination is treated as unsaved on this host.
        if !self.path.is_absolute()
            || self.path.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
        {
            return Ok(None);
        }
        // A copied/private archive must not acquire an ordinary destination
        // merely because its JSON happens to contain another file's binding.
        let expected = match destination.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(parent) = destination.parent() else {
                    return Ok(None);
                };
                let Some(name) = destination.file_name() else {
                    return Ok(None);
                };
                let Ok(parent) = parent.canonicalize() else {
                    return Ok(None);
                };
                parent.join(name)
            }
            Err(_) => return Ok(None),
        };
        if expected != self.path {
            return Ok(None);
        }
        // Validate a present document before restoring save authority. Archive
        // JSON is untrusted: a forged fingerprint must not permit replacing a
        // damaged/future document. Its recovery still opens safely as unsaved.
        // The same handle provides validation and the observed fingerprint.
        let observed = match read_document(&self.path) {
            Ok((value, version)) => {
                if validate_destination(value).is_err() {
                    return Ok(None);
                }
                Baseline::Present(version)
            }
            Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                Baseline::Missing
            }
            Err(_) => return Ok(None),
        };
        // Observe conflicts for presentation, but never adopt current bytes as
        // the baseline. Every later protected save rechecks the archived one.
        let changed = observed != self.baseline;
        Ok(Some((
            DocumentSession::with_baseline(self.path, self.baseline),
            changed,
        )))
    }
}

/// The disk baseline owned by one editing context and its queued saves.
///
/// Clone the `Arc`, not the session, so an earlier successful save advances the
/// baseline that later queued snapshots use. Independently opened windows must
/// get independent sessions. All methods except [`Self::path`] do filesystem
/// work and belong on a storage/startup worker, never the GUI thread. Recovery
/// submission additionally captures its baseline in memory, without disk I/O.
///
/// Changes observed before replacement are conflicts, including deletion and
/// byte-preserving-length changes. Cooperating Aster writers additionally share
/// an OS advisory lock across the check and replacement. External applications
/// that ignore that lock can still race the final check and rename.
#[derive(Debug)]
pub struct DocumentSession {
    path: PathBuf,
    saving: Mutex<()>,
    baseline: Mutex<Baseline>,
}

impl DocumentSession {
    /// Records a valid destination or its absence, for a deliberate Save As.
    ///
    /// Preparing an existing destination does not authorize refreshing the
    /// baseline of an open editor after a conflict. Save the edits elsewhere or
    /// explicitly reopen instead. Missing parent directories are created here.
    pub fn prepare(path: &Path) -> Result<Arc<Self>, StoreError> {
        let path = canonical_destination(path)?;
        let baseline = match read_document(&path) {
            Ok((value, version)) => {
                validate_destination(value)?;
                Baseline::Present(version)
            }
            Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                Baseline::Missing
            }
            Err(error) => return Err(error),
        };
        Ok(Self::with_baseline(path, baseline))
    }

    /// Prepares a new destination and refuses any existing file.
    ///
    /// This is the create/duplicate policy. A different writer creating the
    /// destination after this returns is still caught by the protected save.
    pub fn create(path: &Path) -> Result<Arc<Self>, StoreError> {
        let path = canonical_destination(path)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err(StoreError::DocumentConflict(path)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(Self::with_baseline(path, Baseline::Missing))
    }

    /// The canonical destination. Symlink aliases share one lock and identity.
    /// Keep this path in save completion/destination matching.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The version of the file this session last read or wrote, as
    /// `bytes:sha256` in hex; `None` while the file does not exist yet.
    /// Two sessions with the same version hold the same file contents.
    pub fn version(&self) -> Option<String> {
        match *self.baseline.lock().unwrap_or_else(|e| e.into_inner()) {
            Baseline::Missing => None,
            Baseline::Present(version) => Some(format!(
                "{}:{}",
                version.bytes,
                version
                    .sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )),
        }
    }

    fn with_baseline(path: PathBuf, baseline: Baseline) -> Arc<Self> {
        Arc::new(Self {
            path,
            saving: Mutex::new(()),
            baseline: Mutex::new(baseline),
        })
    }

    pub(super) fn recovery_binding(&self) -> RecoveryDocumentBinding {
        // The publication lock never spans serialization, sync or replacement;
        // a GUI-thread queue submission cannot wait behind storage work.
        let baseline = *self.baseline.lock().unwrap_or_else(|e| e.into_inner());
        RecoveryDocumentBinding {
            schema_version: 1,
            path: self.path.clone(),
            baseline,
        }
    }
}

/// Reads the project and its exact byte baseline from the same open file.
/// Atomic replacement during the read therefore cannot pair an old project
/// with a newer baseline. An ordinary open starts fresh editing history.
pub fn load_project_with_session(
    path: &Path,
) -> Result<(Project, Arc<DocumentSession>), StoreError> {
    // Opening must never create a missing parent or a sidecar, and a readable
    // document in a read-only directory must remain openable.
    let path = path.canonicalize()?;
    let (value, version) = read_document(&path)?;
    let project = crate::decode_project(value, &path)?;
    Ok((
        project,
        DocumentSession::with_baseline(path, Baseline::Present(version)),
    ))
}

/// Replaces the document only while the session's disk baseline still matches.
///
/// The temp file is serialized and synced before the short nonblocking write
/// lock. On conflict/busy/validation/serialization failure, disk bytes and the
/// session baseline stay unchanged. A successful save advances the baseline
/// under the session mutex before another queued save can use it. Directory
/// sync failure can occur after replacement; such a failure is returned and
/// does not advance the baseline or acknowledge a durable save.
pub fn save_project_with_session(
    session: &DocumentSession,
    project: &Project,
) -> Result<(), StoreError> {
    project.validate()?;
    let _saving = session.saving.lock().unwrap_or_else(|e| e.into_inner());
    let baseline = *session.baseline.lock().unwrap_or_else(|e| e.into_inner());
    let temp = crate::prepare_json(
        &session.path,
        &ProjectFile {
            schema_version: SCHEMA_VERSION,
            project: project.clone(),
            checkpoint_label: None,
        },
    )?;
    let next = fingerprint(temp.reopen()?)?;
    let _write = WriteLease::acquire(&session.path)?;
    if !matches_baseline(&session.path, baseline)? {
        return Err(StoreError::DocumentConflict(session.path.clone()));
    }
    crate::replace_json(&session.path, temp)?;
    *session.baseline.lock().unwrap_or_else(|e| e.into_inner()) = Baseline::Present(next);
    Ok(())
}

/// A separate lock, rather than the recovery lease a live editor already holds.
/// The lock file must remain after release to avoid locking different inodes.
pub(super) struct WriteLease {
    _file: File,
}

impl WriteLease {
    pub(super) fn acquire(path: &Path) -> Result<Self, StoreError> {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(".write-lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(sidecar))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(StoreError::DocumentBusy(path.to_path_buf())),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    }
}

pub(super) fn canonical_destination(path: &Path) -> Result<PathBuf, StoreError> {
    let name = path
        .file_name()
        .ok_or_else(|| StoreError::Invalid("A project destination requires a file path".into()))?;
    match path.canonicalize() {
        Ok(path) => return Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Do not silently replace a dangling link instead of its target.
            match fs::symlink_metadata(path) {
                Ok(_) => return Err(error.into()),
                Err(missing) if missing.kind() == io::ErrorKind::NotFound => {}
                Err(other) => return Err(other.into()),
            }
        }
        Err(error) => return Err(error.into()),
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    Ok(parent.canonicalize()?.join(name))
}

fn validate_destination(value: serde_json::Value) -> Result<(), StoreError> {
    if value.get("project").is_some() {
        let file: ProjectFile = serde_json::from_value(value)?;
        file.project.validate()?;
    } else if value.get("projects").is_some() {
        let _ = crate::recovery::decode(value, None)?;
    } else {
        return Err(StoreError::Invalid(
            "Existing file is not a valid project or recovery archive; refusing to overwrite it"
                .into(),
        ));
    }
    Ok(())
}

fn read_document(path: &Path) -> Result<(serde_json::Value, FileVersion), StoreError> {
    read_document_file(File::open(path)?)
}

fn read_document_file(file: File) -> Result<(serde_json::Value, FileVersion), StoreError> {
    // Hash below buffering: the JSON reader otherwise calls Read one byte at
    // a time, which would turn a content baseline into one hash update per byte.
    let mut reader = BufReader::new(VersionReader::new(file)?);
    let value = serde_json::from_reader(&mut reader)?;
    crate::validate_version(&value)?;
    Ok((value, reader.into_inner().version()))
}

fn matches_baseline(path: &Path, expected: Baseline) -> Result<bool, StoreError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(expected == Baseline::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    let Baseline::Present(version) = expected else {
        // Even a newly created corrupt or oversized document is somebody
        // else's work, rather than an absent destination we may replace.
        return Ok(false);
    };
    if file.metadata()?.len() != version.bytes {
        return Ok(false);
    }
    Ok(fingerprint(file)? == version)
}

fn fingerprint(file: File) -> Result<FileVersion, StoreError> {
    let mut reader = VersionReader::new(file)?;
    let mut chunk = [0u8; 64 * 1024];
    while reader.read(&mut chunk)? != 0 {}
    Ok(reader.version())
}

struct VersionReader {
    file: File,
    digest: Sha256,
    bytes: u64,
}

impl VersionReader {
    fn new(file: File) -> Result<Self, StoreError> {
        if file.metadata()?.len() > MAX_FILE_BYTES {
            return Err(StoreError::Invalid(
                "Project exceeds the 256 MiB storage limit".into(),
            ));
        }
        Ok(Self {
            file,
            digest: Sha256::new(),
            bytes: 0,
        })
    }

    fn version(self) -> FileVersion {
        FileVersion {
            bytes: self.bytes,
            sha256: self.digest.finalize().into(),
        }
    }
}

impl Read for VersionReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        // One byte beyond the budget lets us distinguish exact-budget EOF from
        // a file that grows after metadata inspection, without allocating it.
        let allowed = (MAX_FILE_BYTES - self.bytes + 1).min(bytes.len() as u64) as usize;
        let read = self.file.read(&mut bytes[..allowed])?;
        if self.bytes + read as u64 > MAX_FILE_BYTES {
            return Err(io::Error::other(
                "Project exceeds the 256 MiB storage limit",
            ));
        }
        self.digest.update(&bytes[..read]);
        self.bytes += read as u64;
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capturing_recovery_never_waits_for_the_lock_held_across_storage_work() {
        let dir = tempfile::tempdir().unwrap();
        let session = DocumentSession::create(&dir.path().join("film.aster")).unwrap();
        let storage_lock = session.saving.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let snapshot_session = session.clone();
        let snapshot = std::thread::spawn(move || {
            tx.send(snapshot_session.recovery_binding()).unwrap();
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(2));
        // Release even if the assertion fails, so a regression cannot leak the
        // owned helper waiting for the deliberately held storage lock.
        drop(storage_lock);
        snapshot.join().unwrap();
        let binding = result.unwrap();
        assert_eq!(binding.path, session.path());
        assert_eq!(binding.baseline, Baseline::Missing);
    }

    #[test]
    fn atomic_replacement_during_open_cannot_pair_old_contents_with_a_new_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("film.aster");
        let before = Project::new("Opened file handle");
        crate::save_project(&path, &before).unwrap();
        // The loader owns the old handle before another process atomically
        // replaces its pathname; both its parse and baseline must stay old.
        let opened = File::open(&path).unwrap();
        let after = Project::new("Intervening replacement");
        crate::save_project(&path, &after).unwrap();
        let (value, version) = read_document_file(opened).unwrap();
        let project = crate::decode_project(value, &path).unwrap();
        assert_eq!(project, before);
        let stale = DocumentSession::with_baseline(path.clone(), Baseline::Present(version));
        assert!(matches!(
            save_project_with_session(&stale, &project),
            Err(StoreError::DocumentConflict(_))
        ));
        assert_eq!(crate::load_project(&path).unwrap(), after);
    }
}
