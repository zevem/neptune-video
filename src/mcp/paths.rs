//! The folders this server may read and write. A path is resolved before any
//! file is touched: relative paths are taken from the first root, `..` is
//! refused, the deepest part that exists is canonicalized (so links are
//! followed to where they lead) and the result must be under a root. A
//! refusal does not say whether a path outside the roots exists.
//!
//! Roots guard this server's tools. They are not a sandbox for a client
//! that also runs commands of its own.
use super::errors::{ToolError, codes};
use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};

/// The most a roots file may hold.
const MOST_ROOTS_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone)]
pub struct Roots {
    roots: Vec<PathBuf>,
    /// Folders named in a file that another program rewrites while this
    /// server runs (`--roots-file`), read again whenever it changed.
    added: Option<Arc<Added>>,
}

#[derive(Debug)]
struct Added {
    file: PathBuf,
    /// The file as last read: its identity, and the usable folders it named.
    read: Mutex<(Option<Seen>, Vec<PathBuf>)>,
}

/// What tells one version of the roots file from another: it is replaced
/// whole, so a new version is a new file.
type Seen = (Option<SystemTime>, u64, u64);

impl Added {
    /// The folders the file names now. A file that is missing, too large or
    /// not understood names none; a folder that is gone, too wide or inside
    /// another is left out.
    fn current(&self) -> Vec<PathBuf> {
        let seen = std::fs::metadata(&self.file).ok().map(|metadata| {
            #[cfg(unix)]
            let identity = std::os::unix::fs::MetadataExt::ino(&metadata);
            #[cfg(not(unix))]
            let identity = 0;
            (metadata.modified().ok(), metadata.len(), identity)
        });
        let mut read = self.read.lock().unwrap_or_else(|e| e.into_inner());
        if read.0 == seen && seen.is_some() {
            return read.1.clone();
        }
        let folders = match seen {
            Some((_, len, _)) if len <= MOST_ROOTS_BYTES => std::fs::read(&self.file)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<RootsFile>(&bytes).ok())
                .map(|file| super::usable_roots(file.roots))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        *read = (seen, folders.clone());
        folders
    }
}

/// A roots file: `{"roots": ["/path", …]}`.
#[derive(serde::Deserialize)]
struct RootsFile {
    roots: Vec<PathBuf>,
}

impl Roots {
    /// The roots named, else the working directory unless it is the whole
    /// disk, the home folder or a system folder. With a roots file, the
    /// folders it names are roots too, and the working directory is not.
    pub fn new(named: Vec<PathBuf>, cwd: &Path, file: Option<PathBuf>) -> Result<Self, String> {
        let implicit = named.is_empty() && file.is_none();
        let named = if implicit {
            vec![cwd.to_path_buf()]
        } else {
            named
        };
        let mut roots = Vec::new();
        for root in named {
            let root = std::path::absolute(&root).unwrap_or(root);
            let canonical = std::fs::canonicalize(&root)
                .map_err(|error| format!("The root {} cannot be used: {error}", root.display()))?;
            if !canonical.is_dir() {
                return Err(format!("The root {} is not a folder", root.display()));
            }
            if too_wide(&canonical) {
                return Err(format!(
                    "{} is too wide to work in{}. Name the folder the agent may use with \
                     --root DIR or NEPTUNE_VIDEO_MCP_ROOTS.",
                    canonical.display(),
                    if implicit {
                        " (it is the working directory)"
                    } else {
                        ""
                    }
                ));
            }
            roots.push(canonical);
        }
        Ok(Self {
            roots,
            added: file.map(|file| {
                Arc::new(Added {
                    file,
                    read: Mutex::new((None, Vec::new())),
                })
            }),
        })
    }

    /// The roots now: those named, then those the roots file names.
    pub fn list(&self) -> Vec<PathBuf> {
        let mut all = self.roots.clone();
        if let Some(added) = &self.added {
            for root in added.current() {
                if !all.contains(&root) {
                    all.push(root);
                }
            }
        }
        all
    }

    /// Where relative paths are taken from; none while a roots file names
    /// no folder.
    pub fn first(&self) -> Option<PathBuf> {
        self.list().into_iter().next()
    }

    /// Whether a canonical path is under a root.
    pub fn contains(&self, canonical: &Path) -> bool {
        self.list().iter().any(|root| inside(canonical, root))
    }

    /// Whether an absolute path, existing or not, leads under a root once
    /// its links are followed. A path that cannot be resolved is outside.
    pub fn contains_path(&self, path: &Path) -> bool {
        path.is_absolute()
            && !path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
            && std::fs::canonicalize(path)
                .ok()
                .or_else(|| resolve_new(path))
                .is_some_and(|place| self.contains(&place))
    }

    fn outside(&self, given: &Path) -> ToolError {
        ToolError::new(
            codes::OUTSIDE_ROOTS,
            format!(
                "{} is not inside the folders this server may use ({}); the person can add one \
                 with --root, or for Neptune Video's own agent in Settings › Agent › Folders",
                given.display(),
                self.list()
                    .iter()
                    .map(|root| root.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    }

    fn absolute(&self, given: &Path) -> Result<PathBuf, ToolError> {
        if given.as_os_str().is_empty() {
            return Err(ToolError::invalid("The path is empty"));
        }
        if given
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(ToolError::invalid(format!(
                "{} contains “..”; give the path without it",
                given.display()
            )));
        }
        #[cfg(windows)]
        if odd_windows_name(given) {
            return Err(self.outside(given));
        }
        if given.is_absolute() {
            return Ok(given.to_path_buf());
        }
        match self.first() {
            Some(first) => Ok(first.join(given)),
            None => Err(self.outside(given)),
        }
    }

    /// An existing file inside the roots, to read. Refused when it is a
    /// folder, a device or a pipe.
    pub fn file(&self, given: &Path) -> Result<PathBuf, ToolError> {
        let absolute = self.absolute(given)?;
        let Ok(canonical) = std::fs::canonicalize(&absolute) else {
            // Whether a path outside the roots exists is not said.
            return Err(match resolve_new(&absolute) {
                Some(place) if self.contains(&place) => {
                    ToolError::not_found(format!("{} does not exist", given.display()))
                }
                _ => self.outside(given),
            });
        };
        if !self.contains(&canonical) {
            return Err(self.outside(given));
        }
        if !canonical.is_file() {
            return Err(ToolError::invalid(format!(
                "{} is not a regular file",
                given.display()
            )));
        }
        Ok(canonical)
    }

    /// An existing file or folder inside the roots, for media import.
    pub fn entry(&self, given: &Path) -> Result<PathBuf, ToolError> {
        let absolute = self.absolute(given)?;
        let Ok(canonical) = std::fs::canonicalize(&absolute) else {
            return Err(match resolve_new(&absolute) {
                Some(place) if self.contains(&place) => {
                    ToolError::not_found(format!("{} does not exist", given.display()))
                }
                _ => self.outside(given),
            });
        };
        if !self.contains(&canonical) {
            return Err(self.outside(given));
        }
        if !(canonical.is_file() || canonical.is_dir()) {
            return Err(ToolError::invalid(format!(
                "{} is neither a file nor a folder",
                given.display()
            )));
        }
        Ok(canonical)
    }

    /// A place to write inside the roots. Its folder need not exist yet; it
    /// is not created here. Refused when something is there, unless
    /// `overwrite` and it is a file.
    pub fn destination(&self, given: &Path, overwrite: bool) -> Result<PathBuf, ToolError> {
        let absolute = self.absolute(given)?;
        let place = resolve_new(&absolute).ok_or_else(|| self.outside(given))?;
        if !self.contains(&place) {
            return Err(self.outside(given));
        }
        match std::fs::symlink_metadata(&place) {
            Ok(metadata) if metadata.is_dir() => Err(ToolError::invalid(format!(
                "{} is a folder",
                given.display()
            ))),
            // A link that leads nowhere yet would be followed by the write.
            Ok(metadata) if metadata.file_type().is_symlink() => Err(self.outside(given)),
            Ok(_) if !overwrite => Err(ToolError::new(
                codes::EXISTS,
                format!(
                    "{} exists; pass overwrite: true to replace it",
                    given.display()
                ),
            )),
            _ => Ok(place),
        }
    }

    /// Creates a checked destination's missing parent folders. Resolve
    /// again before and after creation, so an existing link cannot move a
    /// new folder outside the allowed roots.
    pub fn create_parent(&self, place: &Path) -> Result<(), ToolError> {
        let checked = self.destination(place, true)?;
        let parent = checked
            .parent()
            .ok_or_else(|| ToolError::invalid("The destination has no parent folder"))?;
        std::fs::create_dir_all(parent).map_err(|error| {
            ToolError::new(
                codes::STORAGE,
                format!("The output folder could not be created: {error}"),
            )
        })?;
        let canonical = std::fs::canonicalize(parent).map_err(|error| {
            ToolError::new(
                codes::STORAGE,
                format!("The output folder could not be read: {error}"),
            )
        })?;
        if !self.contains(&canonical) {
            return Err(self.outside(place));
        }
        Ok(())
    }

    /// Whether media at `path`, as a project stores it, may be read.
    pub fn readable(&self, path: &Path) -> bool {
        path.is_absolute()
            && std::fs::canonicalize(path)
                .is_ok_and(|canonical| self.contains(&canonical) && canonical.is_file())
    }
}

/// Writes `bytes` to `place` (a file `Roots::destination` gave) through a
/// new hidden file beside it, renamed over `place` when complete: a link
/// put at `place` meanwhile is replaced, never followed, and a write that
/// fails leaves what was there.
pub fn write_new(place: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let folder = place.parent().unwrap_or(Path::new("."));
    let name = place
        .file_name()
        .map_or_else(|| "file".into(), |name| name.to_string_lossy().into_owned());
    let part = folder.join(format!(".{name}.{}.part", uuid::Uuid::new_v4().simple()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&part, place)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

/// The canonical form of a path that may not exist yet: its deepest
/// existing ancestor canonicalized, with the rest appended.
fn resolve_new(absolute: &Path) -> Option<PathBuf> {
    let mut rest: Vec<OsString> = Vec::new();
    let mut base = absolute.to_path_buf();
    loop {
        match std::fs::canonicalize(&base) {
            Ok(canonical) => {
                let mut place = canonical;
                for part in rest.iter().rev() {
                    place.push(part);
                }
                return Some(place);
            }
            Err(_) => {
                rest.push(base.file_name()?.to_os_string());
                base = base.parent()?.to_path_buf();
            }
        }
    }
}

/// `path` is `root` or under it. Case is ignored where the usual volumes
/// ignore it.
fn inside(path: &Path, root: &Path) -> bool {
    if cfg!(any(target_os = "macos", windows)) {
        let lower = |path: &Path| path.to_string_lossy().to_lowercase();
        let (path, root) = (lower(path), lower(root));
        let root = root.trim_end_matches(std::path::MAIN_SEPARATOR);
        path == root
            || path
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with(std::path::MAIN_SEPARATOR))
    } else {
        path.starts_with(root)
    }
}

/// The whole disk, the home folder or a system folder.
pub(super) fn too_wide(canonical: &Path) -> bool {
    if canonical.parent().is_none() {
        return true;
    }
    let home =
        directories::BaseDirs::new().and_then(|dirs| std::fs::canonicalize(dirs.home_dir()).ok());
    if home.as_deref() == Some(canonical) {
        return true;
    }
    let system: &[&str] = if cfg!(windows) {
        &[r"C:\Windows", r"C:\Program Files", r"C:\Users"]
    } else {
        &[
            "/bin",
            "/boot",
            "/dev",
            "/etc",
            "/lib",
            "/proc",
            "/sbin",
            "/sys",
            "/usr",
            "/var",
            "/System",
            "/Library",
            "/Users",
            "/home",
            "/private",
            "/private/var",
            "/private/etc",
        ]
    };
    system
        .iter()
        .any(|folder| std::fs::canonicalize(folder).is_ok_and(|folder| folder == canonical))
}

/// UNC shares, device paths and alternate data streams.
#[cfg(windows)]
fn odd_windows_name(path: &Path) -> bool {
    use std::path::Prefix;
    let text = path.to_string_lossy();
    let prefix = path
        .components()
        .next()
        .and_then(|component| match component {
            Component::Prefix(prefix) => Some(prefix.kind()),
            _ => None,
        });
    matches!(
        prefix,
        Some(Prefix::UNC(..) | Prefix::VerbatimUNC(..) | Prefix::DeviceNS(..))
    ) || text.get(2..).is_some_and(|rest| rest.contains(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &Path) -> Roots {
        Roots::new(vec![dir.to_path_buf()], dir, None).unwrap()
    }

    #[test]
    fn a_file_inside_the_root_is_read_and_one_outside_is_refused_without_saying_it_exists() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.mp4"), b"x").unwrap();
        std::fs::write(other.path().join("b.mp4"), b"x").unwrap();
        let roots = roots(root.path());
        assert!(roots.file(Path::new("a.mp4")).is_ok());
        let outside = roots.file(&other.path().join("b.mp4")).unwrap_err();
        let missing = roots.file(&other.path().join("none.mp4")).unwrap_err();
        assert_eq!(outside.code, codes::OUTSIDE_ROOTS);
        assert_eq!(missing.code, codes::OUTSIDE_ROOTS);
        assert_eq!(
            roots.file(Path::new("none.mp4")).unwrap_err().code,
            codes::NOT_FOUND
        );
        assert_eq!(
            roots.file(Path::new("x/../a.mp4")).unwrap_err().code,
            codes::INVALID
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_the_root_to_a_file_outside_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("secret.mp4"), b"x").unwrap();
        std::os::unix::fs::symlink(other.path().join("secret.mp4"), root.path().join("l.mp4"))
            .unwrap();
        std::os::unix::fs::symlink(other.path(), root.path().join("dir")).unwrap();
        let roots = roots(root.path());
        assert_eq!(
            roots.file(Path::new("l.mp4")).unwrap_err().code,
            codes::OUTSIDE_ROOTS
        );
        assert_eq!(
            roots
                .destination(Path::new("dir/new.nvp"), false)
                .unwrap_err()
                .code,
            codes::OUTSIDE_ROOTS
        );
        assert!(!roots.readable(&root.path().join("l.mp4")));
    }

    #[test]
    fn a_destination_may_be_in_a_new_folder_but_never_replaces_without_overwrite() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("taken.mp4"), b"x").unwrap();
        let roots = roots(root.path());
        let new = roots
            .destination(Path::new("out/new/x.mp4"), false)
            .unwrap();
        assert!(new.ends_with("out/new/x.mp4"));
        assert!(!root.path().join("out").exists());
        assert_eq!(
            roots
                .destination(Path::new("taken.mp4"), false)
                .unwrap_err()
                .code,
            codes::EXISTS
        );
        assert!(roots.destination(Path::new("taken.mp4"), true).is_ok());
    }

    #[test]
    fn missing_output_folders_are_created_inside_the_roots() {
        let root = tempfile::tempdir().unwrap();
        let roots = roots(root.path());
        let place = roots
            .destination(Path::new("out/new/frame.png"), false)
            .unwrap();
        roots.create_parent(&place).unwrap();
        write_new(&place, b"picture").unwrap();
        assert_eq!(std::fs::read(&place).unwrap(), b"picture");
    }

    #[cfg(unix)]
    #[test]
    fn creating_output_folders_refuses_parent_links_outside_the_roots() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let roots = roots(root.path());
        let place = roots
            .destination(Path::new("output/new/frame.png"), false)
            .unwrap();
        std::os::unix::fs::symlink(other.path(), root.path().join("output")).unwrap();
        assert_eq!(
            roots.create_parent(&place).unwrap_err().code,
            codes::OUTSIDE_ROOTS
        );
        assert!(!other.path().join("new").exists());
    }

    #[test]
    fn the_whole_disk_and_the_home_folder_are_refused_as_roots() {
        let disk = if cfg!(windows) { r"C:\" } else { "/" };
        assert!(Roots::new(Vec::new(), Path::new(disk), None).is_err());
        if let Some(home) = directories::BaseDirs::new() {
            assert!(
                Roots::new(vec![home.home_dir().to_path_buf()], Path::new(disk), None).is_err()
            );
        }
    }
}
