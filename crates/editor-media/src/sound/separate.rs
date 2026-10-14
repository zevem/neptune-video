//! Stems: a media file's sound split into its voice and the rest by a stem
//! separator the person installs, found and run as FFmpeg is.
//!
//! The editor ships no separator, no model and no runtime, and fetches none:
//! the tool is Demucs's command line, `demucs`, or the executable that
//! `ASTER_SEPARATOR` names, and whatever it fetches to run is its own doing.
//! It is probed once a session by `--help`, which must end well and name
//! `--two-stems`; a job is made only with a separator so found.
//!
//! The tool writes into a folder this job makes fresh, `<media> (stems).part`
//! in the folder the stems go to, and which it removes with whatever is in
//! it, whether the job ends well, fails or is given up. That folder and the
//! folder the stems go to are the only ones it makes; nothing else is ever
//! deleted. Once the tool has ended well the job claims its last step, moves
//! each stem to `<media> (<Stem>).wav`, never over a file that is there (a
//! number is added to the name), and probes it with the import's own
//! ffprobe step.
//!
//! The whole source file is separated, not a clip's span, so stems line up
//! with their source and survive a later trim. The tool reads the first
//! sound stream itself: one that starts after the container's time 0 is
//! early by the gap, as a saved file is.
use super::Control;
use crate::{
    MediaError, Wake, io,
    process::{Process, Processes, tool_program},
};
use editor_core::{Asset, MediaKind, Time};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    process::{ChildStdout, Command, ExitStatus},
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

/// The variable that names a separator executable, in place of `demucs` on
/// `PATH`.
pub const SEPARATOR_VARIABLE: &str = "ASTER_SEPARATOR";
/// The separator looked for on `PATH`.
pub const SEPARATOR: &str = "demucs";
/// The stems asked of the tool, as it names its files, and as the editor
/// names them: the voice, and everything that is not the voice.
pub const STEMS: [(&str, &str); 2] = [("vocals", "Vocals"), ("no_vocals", "Instruments")];
/// The model asked for by name, Demucs's default since its fourth version,
/// so the folder the tool writes into is known.
const MODEL: &str = "htdemucs";
/// How long `--help` may take: the tool loads its own runtime first.
const PROBE_DEADLINE: Duration = Duration::from_secs(60);
/// The most of what `--help` prints that is read.
const HELP_BYTES: usize = 256 * 1024;
/// How long a separation may run at most: this, and [`DEADLINE_PER_SECOND`]
/// times the length of the source. The tool says how far it is only on its
/// error pipe, so a run is held to this whole, not to a read.
const DEADLINE_BASE: Duration = Duration::from_secs(15 * 60);
const DEADLINE_PER_SECOND: u32 = 4;
/// How often a running separation is looked at.
const POLL: Duration = Duration::from_millis(100);
/// How often its progress is read from what it said last.
const PROGRESS_EVERY: Duration = Duration::from_millis(500);
/// Names tried for one stem before giving up: `Song (Vocals) 2.wav`…
const MAX_NAMES: u32 = 999;

/// A stem separator that answered its probe. Only [`Separator::probe`]
/// makes one, so a job is never sent to a tool that is not there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Separator {
    program: OsString,
}

impl Separator {
    /// Runs `program --help` and keeps it when it ends well, within a
    /// minute, and names `--two-stems`. A program that is not there is an
    /// error in words, and so is one that answers otherwise.
    pub fn probe(program: impl Into<OsString>) -> Result<Self, MediaError> {
        let program = program.into();
        let name = tool_name(&program);
        let processes = Processes::default();
        let mut command = Command::new(&program);
        command.arg("--help");
        let mut process = match Process::spawn(&name, &mut command, &processes) {
            Err(MediaError::MissingTool { .. }) => return Err(missing(&program)),
            started => started?,
        };
        let until = Instant::now() + PROBE_DEADLINE;
        process.begin_read_until(until);
        let mut help = Vec::new();
        let read = (process.stdout.by_ref())
            .take(HELP_BYTES as u64)
            .read_to_end(&mut help);
        if !process.end_read_checked() {
            return Err(process.failure("did not answer --help within a minute"));
        }
        read.map_err(|error| io(format!("Cannot read {name}"), error))?;
        let status = exited(&process, until)?;
        if !status.success() {
            // The last line: a tool that cannot start often ends on a whole
            // traceback, of which that is the reason.
            let last = last_line(&process.stderr_tail()).unwrap_or_default();
            return Err(MediaError::Decode(format!(
                "{name} could not show its options ({status}): {last}"
            )));
        }
        // Some tools print their usage on the error pipe.
        let mut said = String::from_utf8_lossy(&help).into_owned();
        said.push_str(&process.stderr_tail());
        if !said.contains("--two-stems") {
            return Err(MediaError::Invalid(format!(
                "{name} is no stem separator this editor can run: its options do not name --two-stems"
            )));
        }
        Ok(Self { program })
    }

    /// The executable that is run.
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    fn name(&self) -> String {
        tool_name(&self.program)
    }
}

/// What the notices call a tool: its file's name without an extension.
fn tool_name(program: &OsStr) -> String {
    Path::new(program).file_stem().map_or_else(
        || SEPARATOR.to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

/// Whether a probe's answer says no separator is there at all, as for most
/// people, rather than that one is there and cannot be run.
pub fn separator_absent(why: &str) -> bool {
    why.starts_with(ABSENT)
}

const ABSENT: &str = "No stem separator was found";

fn missing(program: &OsStr) -> MediaError {
    MediaError::Invalid(format!(
        "{ABSENT}: {} could not be run. Install Demucs so that demucs is on PATH, or set {SEPARATOR_VARIABLE} to a separator's executable path.",
        Path::new(program).display()
    ))
}

/// The separator this session looks for: what `ASTER_SEPARATOR` names, or
/// `demucs` on `PATH`.
pub fn separator_program() -> OsString {
    tool_program(SEPARATOR)
}

static SESSION: OnceLock<Result<Separator, String>> = OnceLock::new();
static LOOKING: OnceLock<()> = OnceLock::new();

/// Starts this session's probe for a separator on a thread of its own, the
/// first time it is called, and wakes `wake` when it has answered. It never
/// waits.
pub fn look_for_separator(wake: impl Fn() + Send + Sync + 'static) {
    let wake: Wake = Arc::new(wake);
    LOOKING.get_or_init(|| {
        let started = thread::Builder::new()
            .name("aster-separator-probe".into())
            .spawn(move || {
                let found =
                    Separator::probe(separator_program()).map_err(|error| error.to_string());
                let _ = SESSION.set(found);
                wake();
            });
        if let Err(error) = started {
            let _ = SESSION.set(Err(format!("Cannot look for a stem separator: {error}")));
        }
    });
}

/// This session's separator, or why there is none. `None` until the probe
/// started by [`look_for_separator`] has answered; this never waits.
pub fn stem_separator() -> Option<Result<Separator, String>> {
    SESSION.get().cloned()
}

/// One media file's sound to be separated into stems.
#[derive(Clone, Debug)]
pub struct SeparateJob {
    /// The project that asks, handed back with the answer and never read.
    pub project: Uuid,
    pub asset: Asset,
    /// The folder the stems are put in, made when it is not there.
    pub to: PathBuf,
    pub separator: Separator,
}

impl SeparateJob {
    /// What the stems' files are named after: the media's name without its
    /// extension.
    fn stem(&self) -> String {
        let name = Path::new(&self.asset.name).file_stem().map_or_else(
            || self.asset.name.clone(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        let name = name.replace(['/', '\\'], "-");
        match name.trim() {
            "" => "Audio".to_owned(),
            name => name.to_owned(),
        }
    }

    /// The folder the tool writes into until its stems are put in place.
    pub fn part_folder(&self) -> PathBuf {
        self.to.join(format!("{} (stems).part", self.stem()))
    }

    /// Where the stem `label` goes when nothing is there yet: `Song
    /// (Vocals).wav`, and with ` 2`, ` 3`… for the `n`th name.
    pub fn stem_path(&self, label: &str, n: u32) -> PathBuf {
        let stem = self.stem();
        match n {
            0 | 1 => self.to.join(format!("{stem} ({label}).wav")),
            n => self.to.join(format!("{stem} ({label}) {n}.wav")),
        }
    }

    /// How long the tool may run on this media.
    fn deadline(&self) -> Duration {
        let seconds = self.asset.duration.as_seconds().clamp(0.0, 24.0 * 3600.0);
        DEADLINE_BASE + Duration::from_secs_f64(seconds * f64::from(DEADLINE_PER_SECOND))
    }
}

// Refused here, before any process or folder.
pub(super) fn check(job: &SeparateJob) -> Result<(), MediaError> {
    let invalid = |why: &str| Err(MediaError::Invalid(why.into()));
    let asset = &job.asset;
    if !asset.has_audio && asset.kind != MediaKind::Audio {
        return invalid("Media has no sound to separate");
    }
    if !asset.duration.is_valid() || asset.duration <= Time::ZERO {
        return invalid("Media has no length");
    }
    if job.to.file_name().is_none() {
        return invalid("Choose a folder to put the stems in");
    }
    Ok(())
}

/// The folder this job made for the tool, removed with what is in it when
/// it is dropped: everything in it is the tool's work for this job.
struct PartFolder {
    path: PathBuf,
}

impl PartFolder {
    /// Makes the folder, which must not be there yet: one of that name is
    /// someone's, and is left as it is.
    fn make(path: PathBuf) -> Result<Self, MediaError> {
        match fs::create_dir(&path) {
            Ok(()) => Ok(Self { path }),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                Err(MediaError::Invalid(format!(
                    "{} is in the way: something of that name is already there, and it was left as it is",
                    path.display()
                )))
            }
            Err(error) => Err(io(format!("Cannot make {}", path.display()), error)),
        }
    }
}

impl Drop for PartFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// How a process ended, waited for until `until`. The child's lock is never
/// held while waiting: a watchdog or a cancellation needs it.
fn exited(process: &Process, until: Instant) -> Result<ExitStatus, MediaError> {
    loop {
        let ended = (process.child.lock().map_err(|_| MediaError::Closed)?)
            .try_wait()
            .map_err(|error| io("Cannot wait for the stem separator", error))?;
        if let Some(status) = ended {
            return Ok(status);
        }
        if Instant::now() >= until {
            return Err(process.failure("did not end"));
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// Reads a process's output to its end, and lets it go, on a thread of its
/// own through a second handle to the pipe. The thread ends when the last
/// writer closes the pipe, and holds nothing else.
fn drain(stdout: &ChildStdout) -> std::io::Result<()> {
    #[cfg(unix)]
    let pipe = {
        use std::os::fd::AsFd;
        stdout.as_fd().try_clone_to_owned()?
    };
    #[cfg(windows)]
    let pipe = {
        use std::os::windows::io::AsHandle;
        stdout.as_handle().try_clone_to_owned()?
    };
    let mut pipe = fs::File::from(pipe);
    thread::Builder::new()
        .name("aster-separator-output".into())
        .spawn(move || {
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
        })?;
    Ok(())
}

/// How far the tool says it is: the last `NN%|` of its progress bar in what
/// it wrote to its error pipe, `0..=1`.
pub(crate) fn said_progress(tail: &str) -> Option<f32> {
    let bar = tail.rfind("%|")?;
    let before = tail[..bar].trim_end();
    let digits = before.len()
        - before
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .count();
    let percent: u32 = before[digits..].parse().ok()?;
    (percent <= 100).then(|| percent as f32 / 100.0)
}

/// The last line the tool wrote, for a failure: what a tool ends on is often
/// why it ended. Progress bars are rewritten with carriage returns, which
/// end a line here too.
pub(crate) fn last_line(tail: &str) -> Option<String> {
    let line = tail
        .split(['\n', '\r'])
        .map(str::trim)
        .rfind(|line| !line.is_empty())?;
    let mut line = line.to_owned();
    if line.len() > 400 {
        let mut cut = 400;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        line.truncate(cut);
        line.push('…');
    }
    Some(line)
}

/// Separates the job's media through one run of the tool, puts its stems in
/// place and probes them; the assets are in [`STEMS`]' order.
pub(super) fn separate(
    job: &SeparateJob,
    processes: &Processes,
    control: &Control,
) -> Result<Vec<Asset>, MediaError> {
    if control.cancelled() {
        return Err(MediaError::Cancelled);
    }
    fs::create_dir_all(&job.to)
        .map_err(|error| io(format!("Cannot make {}", job.to.display()), error))?;
    // Made before the process, so dropped after it: the process is waited
    // for, and its files closed, before the folder is removed.
    let part = PartFolder::make(job.part_folder())?;
    let name = job.separator.name();
    let mut command = Command::new(job.separator.program());
    command
        .args(["--two-stems", STEMS[0].0, "-n", MODEL, "-o"])
        .arg(&part.path)
        .args(["--filename", "{stem}.{ext}"])
        .arg(&job.asset.path);
    let process = Process::spawn(&name, &mut command, processes).map_err(|error| match error {
        MediaError::MissingTool { .. } => missing(job.separator.program()),
        error => error,
    })?;
    // What it prints is read and let go on a thread of its own, so a tool
    // that says much is never held up by a full pipe; how far it is comes
    // from its error pipe.
    drain(&process.stdout).map_err(|error| io(format!("Cannot read what {name} prints"), error))?;
    // Held to the whole run: the watchdog stops the tool at the deadline.
    let until = Instant::now() + job.deadline();
    process.begin_read_until(until);
    let mut said = 0.0;
    let mut looked = Instant::now();
    let status = loop {
        if control.cancelled() {
            return Err(MediaError::Cancelled);
        }
        let ended = (process.child.lock().map_err(|_| MediaError::Closed)?)
            .try_wait()
            .map_err(|error| io(format!("Cannot wait for {name}"), error))?;
        if let Some(status) = ended {
            break status;
        }
        if looked.elapsed() >= PROGRESS_EVERY {
            looked = Instant::now();
            if let Some(done) = said_progress(&process.stderr_tail())
                && done - said >= 0.01
            {
                said = done;
                control.progress(done);
            }
        }
        thread::sleep(POLL);
    };
    if control.cancelled() {
        return Err(MediaError::Cancelled);
    }
    if !process.end_read_checked() {
        let minutes = job.deadline().as_secs().div_ceil(60);
        return Err(MediaError::Decode(format!(
            "{name} was stopped after {minutes} minutes, the most a separation of this media may take"
        )));
    }
    if !status.success() {
        let last = last_line(&process.stderr_tail());
        return Err(MediaError::Decode(match last {
            Some(last) => format!("{name} could not separate the sound ({status}): {last}"),
            None => format!("{name} could not separate the sound ({status})"),
        }));
    }
    drop(process);
    let written: Vec<PathBuf> = (STEMS.iter())
        .map(|(file, _)| {
            find(&part.path, &format!("{file}.wav"), 3).ok_or_else(|| {
                MediaError::Decode(format!("{name} ended without writing {file}.wav"))
            })
        })
        .collect::<Result<_, _>>()?;
    // From here the job is no longer given up: the stems are put in place
    // and answered for, or the step fails and says so.
    if !control.claim() {
        return Err(MediaError::Cancelled);
    }
    let mut placed = Vec::with_capacity(STEMS.len());
    for (from, (_, label)) in written.iter().zip(STEMS) {
        match put(job, from, label) {
            Ok(to) => placed.push(to),
            Err(error) => return Err(kept(error, &placed)),
        }
    }
    let mut assets = Vec::with_capacity(placed.len());
    for path in &placed {
        let probed = crate::import::probe(path, processes, &|| false)
            .map_err(|error| kept(error, &placed))?;
        assets.push(probed);
    }
    Ok(assets)
}

/// An error after some stems were put in place, which names where they are:
/// they are the person's and stay.
fn kept(error: MediaError, placed: &[PathBuf]) -> MediaError {
    if placed.is_empty() {
        return error;
    }
    let names: Vec<String> = (placed.iter())
        .map(|path| path.display().to_string())
        .collect();
    MediaError::Decode(format!("{error}. Kept: {}", names.join(", ")))
}

/// The file named `name` in `folder` or a folder under it, at most `depth`
/// deep, without following links.
fn find(folder: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    let direct = folder.join(name);
    if fs::symlink_metadata(&direct).is_ok_and(|meta| meta.is_file()) {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    let mut folders: Vec<PathBuf> = (fs::read_dir(folder).ok()?.flatten())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    folders
        .iter()
        .find_map(|inner| find(inner, name, depth - 1))
}

/// Moves one stem to the first free name for `label`, never over what is
/// there: a hard link refuses a name that is taken, and the stem's own name
/// in the part folder goes with the folder.
fn put(job: &SeparateJob, from: &Path, label: &str) -> Result<PathBuf, MediaError> {
    for n in 1..=MAX_NAMES {
        let to = job.stem_path(label, n);
        match fs::symlink_metadata(&to) {
            Ok(_) => continue,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(io(format!("Cannot look at {}", to.display()), error)),
        }
        match fs::hard_link(from, &to) {
            Ok(()) => return Ok(to),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            // A folder that takes no links: a rename, the name having been
            // seen free just now.
            Err(_) => {
                return fs::rename(from, &to)
                    .map(|()| to.clone())
                    .map_err(|error| io(format!("Cannot write {}", to.display()), error));
            }
        }
    }
    Err(MediaError::Invalid(format!(
        "Every name for the {label} stem of {} is taken in {}",
        job.stem(),
        job.to.display()
    )))
}

#[cfg(test)]
#[path = "separate_tests.rs"]
mod tests;
