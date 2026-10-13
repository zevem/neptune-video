//! Active recovery ownership uses an OS file lock, rather than a PID or the
//! existence/content of a sidecar. The sidecar stays in place after release so
//! another process cannot acquire a different inode while the old one is held.

use crate::StoreError;
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// An exclusive recovery sidecar lock held until this uncloneable guard drops.
///
/// These methods perform filesystem I/O and belong on a storage/startup worker.
/// Use the same identity path for acquisition and probing: a document or recovery
/// path `example.aster` has the sidecar `example.aster.lock`. No recovery suffix
/// is automatically added. The document itself is never read or modified.
#[derive(Debug)]
pub struct RecoveryLease {
    _file: File,
}

impl RecoveryLease {
    /// Creates an adjacent sidecar without truncation and takes its lock.
    /// A live owner returns [`StoreError::RecoveryInUse`] immediately rather
    /// than waiting; permission/storage errors remain separate I/O failures.
    pub fn acquire(path: &Path) -> Result<Self, StoreError> {
        let sidecar = sidecar_path(path)?;
        if let Some(parent) = sidecar
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(sidecar)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(StoreError::RecoveryInUse(path.to_path_buf())),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    }

    /// Probes an existing sidecar; missing sidecars are inactive. This never
    /// creates files/directories or parses recovery data or owner metadata.
    /// Other filesystem failures remain errors, rather than claiming inactivity.
    pub fn is_active(path: &Path) -> Result<bool, StoreError> {
        let sidecar = sidecar_path(path)?;
        let file = match OpenOptions::new().read(true).write(true).open(sidecar) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        match file.try_lock() {
            Ok(()) => Ok(false), // Closing this local handle immediately releases it.
            Err(TryLockError::WouldBlock) => Ok(true),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    }
}

fn sidecar_path(path: &Path) -> Result<PathBuf, StoreError> {
    if path.file_name().is_none() {
        return Err(StoreError::Invalid(
            "Recovery lease requires a file path".into(),
        ));
    }
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    Ok(PathBuf::from(name))
}
