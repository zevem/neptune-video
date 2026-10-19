//! Task-owned subprocesses. Blocking reads have a sleeping deadline monitor;
//! cancellation kills only registered children, and the UI never joins workers.
use crate::{MediaError, io};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct Processes {
    monitors: Arc<Mutex<Vec<Weak<MonitorState>>>>,
    epoch: Arc<AtomicU64>,
    #[cfg(test)]
    starts: Arc<std::sync::atomic::AtomicUsize>,
}
impl Processes {
    #[cfg(test)]
    pub(crate) fn started_tools(&self) -> usize {
        self.starts.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn retained_monitors(&self) -> usize {
        self.monitors
            .lock()
            .unwrap()
            .iter()
            .filter(|monitor| monitor.strong_count() != 0)
            .count()
    }

    pub fn kill_all(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        // Caller threads publish cancellation only. Background monitors own
        // OS kill syscalls and unblock their worker's pipe reads.
        if let Ok(mut monitors) = self.monitors.lock() {
            monitors.retain(|monitor| {
                if let Some(monitor) = monitor.upgrade() {
                    if let Ok(mut deadline) = monitor.0.lock() {
                        deadline.cancelled = true;
                    }
                    monitor.1.notify_all();
                    true
                } else {
                    false
                }
            });
        }
    }
}

#[derive(Default)]
struct Deadline {
    at: Option<Instant>,
    stopped: bool,
    cancelled: bool,
    expired: bool,
}
impl Deadline {
    fn expire_if_due(&mut self, now: Instant) -> bool {
        if self.at.is_some_and(|at| now >= at) {
            // Expiry commits under the monitor mutex. A completed read cannot
            // disarm or reuse a child whose watchdog has decided to kill it.
            self.expired = true;
        }
        self.expired
    }

    fn arm(&mut self, at: Instant, now: Instant) -> bool {
        if self.stopped || self.cancelled || self.expire_if_due(now) || at <= now {
            return false;
        }
        self.at = Some(at);
        true
    }

    fn finish(&mut self, now: Instant) -> bool {
        if self.stopped || self.cancelled || self.expire_if_due(now) || self.at.is_none() {
            return false;
        }
        self.at = None;
        true
    }
}
type MonitorState = (Mutex<Deadline>, Condvar);
type Monitor = Arc<MonitorState>;

/// The most of a process's error pipe that is kept: its last bytes.
pub(crate) const STDERR_TAIL: usize = 8192;
/// How long a tail is waited for once its process has ended.
const TAIL_WAIT: Duration = Duration::from_millis(250);

/// The last [`STDERR_TAIL`] bytes a process wrote to its error pipe, and
/// whether the pipe has ended: its reader has read the last of them.
#[derive(Default)]
struct Tail {
    bytes: Vec<u8>,
    ended: bool,
}
type TailState = (Mutex<Tail>, Condvar);

pub(crate) struct Process {
    pub child: Arc<Mutex<Child>>,
    pub stdout: ChildStdout,
    stderr: Arc<TailState>,
    /// Set only by `spawn_writer`: the child's input pipe until closed.
    stdin: Option<ChildStdin>,
    monitor: Monitor,
    tool: String,
}
impl Process {
    pub fn spawn(
        tool: &str,
        command: &mut Command,
        processes: &Processes,
    ) -> Result<Self, MediaError> {
        Self::spawn_with(tool, command, processes, Stdio::null())
    }

    /// Like `spawn`, with the child's input a pipe this process writes with
    /// `write_until`. Each write arms the same deadline monitor reads use, so
    /// a child that stops reading is killed rather than blocking its writer.
    /// A writer that stops writing (a paused export) arms nothing.
    pub fn spawn_writer(
        tool: &str,
        command: &mut Command,
        processes: &Processes,
    ) -> Result<Self, MediaError> {
        let mut process = Self::spawn_with(tool, command, processes, Stdio::piped())?;
        process.stdin = process
            .child
            .lock()
            .map_err(|_| MediaError::Closed)?
            .stdin
            .take();
        if process.stdin.is_none() {
            return Err(MediaError::Decode(format!("{tool} has no input pipe")));
        }
        Ok(process)
    }

    fn spawn_with(
        tool: &str,
        command: &mut Command,
        processes: &Processes,
        stdin: Stdio,
    ) -> Result<Self, MediaError> {
        let epoch = processes.epoch.load(Ordering::Acquire);
        command
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                MediaError::MissingTool { tool: tool.into() }
            } else {
                io(format!("Cannot start {tool}"), error)
            }
        })?;
        #[cfg(test)]
        processes.starts.fetch_add(1, Ordering::AcqRel);
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaError::Decode(format!("{tool} has no output pipe")));
        };
        let error_pipe = child.stderr.take();
        let child = Arc::new(Mutex::new(child));
        let tail = Tail {
            ended: error_pipe.is_none(),
            ..Tail::default()
        };
        let instance = Self {
            child,
            stdout,
            stderr: Arc::new((Mutex::new(tail), Condvar::new())),
            stdin: None,
            monitor: Arc::new((Mutex::new(Deadline::default()), Condvar::new())),
            tool: tool.into(),
        };
        if let Ok(mut monitors) = processes.monitors.lock() {
            monitors.retain(|monitor| monitor.strong_count() != 0);
            monitors.push(Arc::downgrade(&instance.monitor));
        }
        // Closing while spawn is in flight cannot leak a late child.
        if processes.epoch.load(Ordering::Acquire) != epoch {
            return Err(MediaError::Cancelled);
        }
        if let Some(mut pipe) = error_pipe {
            let tail = Arc::clone(&instance.stderr);
            thread::Builder::new()
                .name("neptune-video-media-stderr".into())
                .spawn(move || {
                    let (lock, read) = &*tail;
                    let mut bytes = [0; 1024];
                    while let Ok(count) = pipe.read(&mut bytes) {
                        if count == 0 {
                            break;
                        }
                        if let Ok(mut tail) = lock.lock() {
                            tail.bytes.extend_from_slice(&bytes[..count]);
                            let excess = tail.bytes.len().saturating_sub(STDERR_TAIL);
                            if excess != 0 {
                                tail.bytes.drain(..excess);
                            }
                        }
                    }
                    // Whoever waits for the last bytes is told they are in.
                    if let Ok(mut tail) = lock.lock() {
                        tail.ended = true;
                    }
                    read.notify_all();
                })
                .map_err(|error| io("Cannot start media error reader", error))?;
        }
        let monitor_child = Arc::clone(&instance.child);
        let monitor = Arc::clone(&instance.monitor);
        thread::Builder::new()
            .name("neptune-video-media-deadline".into())
            .spawn(move || {
                let (lock, ready) = &*monitor;
                let Ok(mut deadline) = lock.lock() else {
                    return;
                };
                loop {
                    if deadline.stopped {
                        return;
                    }
                    if deadline.cancelled || deadline.expire_if_due(Instant::now()) {
                        drop(deadline);
                        if let Ok(mut child) = monitor_child.lock() {
                            let _ = child.kill();
                        }
                        return;
                    }
                    if let Some(at) = deadline.at {
                        let Ok((next, _)) = ready
                            .wait_timeout(deadline, at.saturating_duration_since(Instant::now()))
                        else {
                            return;
                        };
                        deadline = next;
                    } else {
                        let Ok(next) = ready.wait(deadline) else {
                            return;
                        };
                        deadline = next;
                    }
                }
            })
            .map_err(|error| io("Cannot start media watchdog", error))?;
        Ok(instance)
    }
    pub fn begin_read(&self) {
        let _ = self.begin_read_until(Instant::now() + Duration::from_secs(10));
    }

    /// Arms the existing watchdog with an absolute read deadline. An expired
    /// or cancelled process cannot be revived while its OS kill is in flight.
    pub fn begin_read_until(&self, at: Instant) -> bool {
        let armed = self
            .monitor
            .0
            .lock()
            .is_ok_and(|mut deadline| deadline.arm(at, Instant::now()));
        self.monitor.1.notify_all();
        armed
    }

    pub fn end_read(&self) {
        let _ = self.end_read_checked();
    }

    /// Returns true only when the armed read completed before its deadline.
    /// A false result requires the caller to discard this process, even if
    /// the pipe happened to supply the complete requested bytes.
    pub fn end_read_checked(&self) -> bool {
        let completed = self
            .monitor
            .0
            .lock()
            .is_ok_and(|mut deadline| deadline.finish(Instant::now()));
        self.monitor.1.notify_all();
        completed
    }
    /// Writes all of `bytes` to the child's input, killing the child if the
    /// write has not finished by `at`. A failed or late write leaves this
    /// process unusable: the caller discards it.
    pub fn write_until(&mut self, bytes: &[u8], at: Instant) -> Result<(), MediaError> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(MediaError::Invalid(format!(
                "{} input is already closed",
                self.tool
            )));
        };
        if !self
            .monitor
            .0
            .lock()
            .is_ok_and(|mut deadline| deadline.arm(at, Instant::now()))
        {
            self.monitor.1.notify_all();
            return Err(self.failure("could not be given more input"));
        }
        self.monitor.1.notify_all();
        let written = stdin.write_all(bytes);
        let completed = self.end_read_checked();
        match written {
            Ok(()) if completed => Ok(()),
            Ok(()) => Err(self.failure("did not take its input in time")),
            Err(error) => Err(self.failure(&format!("stopped taking input ({error})"))),
        }
    }

    /// Closes the child's input so it sees the end of its stream.
    pub fn close_input(&mut self) {
        self.stdin = None;
    }

    /// Waits up to `until` for the child to exit. The child mutex is never
    /// held while sleeping: cancellation and the watchdog need it.
    pub fn wait_until(
        &self,
        until: Instant,
    ) -> Result<Option<std::process::ExitStatus>, MediaError> {
        let mut pause = Duration::from_millis(2);
        loop {
            if let Some(status) = self
                .child
                .lock()
                .map_err(|_| MediaError::Closed)?
                .try_wait()
                .map_err(|error| io(format!("Cannot wait for {}", self.tool), error))?
            {
                return Ok(Some(status));
            }
            let now = Instant::now();
            if now >= until {
                return Ok(None);
            }
            // A long encode's end is waited for without spinning.
            thread::sleep(pause.min(until - now));
            pause = (pause * 2).min(Duration::from_millis(50));
        }
    }

    /// Everything the child has written to its error output, up to its last
    /// [`STDERR_TAIL`] bytes, once that output has closed or `until` has
    /// passed.
    pub fn stderr_until(&self, until: Instant) -> String {
        let (lock, read) = &*self.stderr;
        let Ok(mut tail) = lock.lock() else {
            return String::new();
        };
        while !tail.ended {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let Ok((next, _)) = read.wait_timeout(tail, left) else {
                return String::new();
            };
            tail = next;
        }
        String::from_utf8_lossy(&tail.bytes).into_owned()
    }

    /// What the process wrote to its error pipe, the last [`STDERR_TAIL`]
    /// bytes of it. The pipe is read on a thread of its own, which can be
    /// behind when the process ends, and what a tool writes last is often
    /// what it is run for: once the process has ended this waits for the
    /// pipe's end, at most 250 ms. While it runs, this is what is in so far.
    pub fn stderr_tail(&self) -> String {
        // The child's lock is let go before the tail's is taken.
        let ended = (self.child.lock().ok())
            .and_then(|mut child| child.try_wait().ok().flatten())
            .is_some();
        let (lock, read) = &*self.stderr;
        let Ok(mut tail) = lock.lock() else {
            return String::new();
        };
        if ended {
            let until = Instant::now() + TAIL_WAIT;
            while !tail.ended {
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let Ok((next, _)) = read.wait_timeout(tail, left) else {
                    return String::new();
                };
                tail = next;
            }
        }
        String::from_utf8_lossy(&tail.bytes).into_owned()
    }

    pub fn failure(&self, detail: &str) -> MediaError {
        let tail = self.stderr_tail().trim().to_owned();
        MediaError::Decode(format!(
            "{}: {detail}{}",
            self.tool,
            if tail.is_empty() {
                String::new()
            } else {
                format!(": {tail}")
            }
        ))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if let Ok(mut deadline) = self.monitor.0.lock() {
            deadline.stopped = true;
        }
        self.monitor.1.notify_all();
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub(crate) fn tool_command(tool: &str) -> Command {
    Command::new(tool_program(tool))
}

/// The executable run for `tool`: the one its variable names; else the copy
/// shipped with Neptune Video (see `shipped_tool`); else the tool's name, looked for
/// on `PATH`. The stem separator, `demucs`, is named by `NEPTUNE_VIDEO_SEPARATOR` and
/// is never shipped.
pub(crate) fn tool_program(tool: &str) -> std::ffi::OsString {
    static SHIPPED_FFMPEG: OnceLock<Option<PathBuf>> = OnceLock::new();
    static SHIPPED_FFPROBE: OnceLock<Option<PathBuf>> = OnceLock::new();
    let (variable, shipped) = match tool {
        "ffmpeg" => ("NEPTUNE_VIDEO_FFMPEG", Some(&SHIPPED_FFMPEG)),
        "demucs" => ("NEPTUNE_VIDEO_SEPARATOR", None),
        _ => ("NEPTUNE_VIDEO_FFPROBE", Some(&SHIPPED_FFPROBE)),
    };
    if let Some(program) = std::env::var_os(variable) {
        return program;
    }
    let shipped = shipped.and_then(|found| {
        found
            .get_or_init(|| {
                let executable = std::env::current_exe().ok()?;
                // A symlink to Neptune Video (`/usr/bin/neptune-video`) leads to its install.
                let executable = executable.canonicalize().unwrap_or(executable);
                shipped_tool(&executable, tool)
            })
            .clone()
    });
    shipped.map_or_else(|| tool.into(), PathBuf::into_os_string)
}

/// The folder, below a source checkout, that `scripts/fetch-media-tools.py`
/// fills with this platform's FFmpeg.
fn fetched_tools() -> PathBuf {
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    Path::new(".tools").join("media").join(platform)
}

/// The copy of `tool` that comes with the Neptune Video at `executable`: the one in
/// the same folder, where a package puts it; else, for a build in a source
/// checkout (`<checkout>/target/…/neptune-video`), the one fetched into
/// `<checkout>/.tools/media/<os>-<arch>`.
fn shipped_tool(executable: &Path, tool: &str) -> Option<PathBuf> {
    let name = format!("{tool}{}", std::env::consts::EXE_SUFFIX);
    let folder = executable.parent()?;
    let beside = folder.join(&name);
    if beside.is_file() {
        return Some(beside);
    }
    let target = folder
        .ancestors()
        .find(|ancestor| ancestor.file_name() == Some("target".as_ref()))?;
    let fetched = target.parent()?.join(fetched_tools()).join(name);
    fetched.is_file().then_some(fetched)
}
pub(crate) fn capture(
    tool: &str,
    command: &mut Command,
    processes: &Processes,
    limit: usize,
) -> Result<Vec<u8>, MediaError> {
    let mut process = Process::spawn(tool, command, processes)?;
    process.begin_read();
    let mut bytes = Vec::new();
    process
        .stdout
        .by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io(format!("Cannot read {tool} output"), error))?;
    if bytes.len() > limit {
        return Err(process.failure("output exceeds its memory budget"));
    }
    // Never hold the child mutex while waiting: cancellation/watchdog needs it.
    let until = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = process
            .child
            .lock()
            .map_err(|_| MediaError::Closed)?
            .try_wait()
            .map_err(|error| io(format!("Cannot wait for {tool}"), error))?
        {
            break status;
        }
        if Instant::now() >= until {
            return Err(process.failure("process timed out"));
        }
        thread::sleep(Duration::from_millis(2));
    };
    process.end_read();
    if !status.success() {
        return Err(process.failure("process failed or timed out"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn shipped_tools_are_found_beside_the_executable_first() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("Neptune Video/bin/neptune-video");
        place(&executable);
        assert_eq!(shipped_tool(&executable, "ffmpeg"), None);

        let ffprobe = format!("ffprobe{}", std::env::consts::EXE_SUFFIX);
        let beside = root.path().join("Neptune Video/bin").join(&ffprobe);
        place(&beside);
        assert_eq!(shipped_tool(&executable, "ffprobe"), Some(beside));
        // Each tool is looked for by its own name.
        assert_eq!(shipped_tool(&executable, "ffmpeg"), None);
    }

    #[test]
    fn checkout_builds_find_the_fetched_tools() {
        let root = tempfile::tempdir().unwrap();
        let ffmpeg = format!("ffmpeg{}", std::env::consts::EXE_SUFFIX);
        let fetched = root.path().join(fetched_tools()).join(&ffmpeg);
        place(&fetched);
        for executable in [
            "target/release/neptune-video",
            "target/x86_64-unknown-linux-gnu/release/neptune-video",
            "target/debug/deps/editor_media-0123",
        ] {
            let executable = root.path().join(executable);
            place(&executable);
            assert_eq!(shipped_tool(&executable, "ffmpeg"), Some(fetched.clone()));
        }

        // An install outside a `target` folder never looks above itself.
        let installed = root.path().join("opt/neptune-video/bin/neptune-video");
        place(&installed);
        assert_eq!(shipped_tool(&installed, "ffmpeg"), None);

        // A copy beside the executable wins over the fetched one.
        let release = root.path().join("target/release/neptune-video");
        let beside = root.path().join("target/release").join(&ffmpeg);
        place(&beside);
        assert_eq!(shipped_tool(&release, "ffmpeg"), Some(beside));
    }

    #[test]
    fn expired_reads_cannot_be_disarmed_or_rearmed_before_the_os_kill() {
        let started = Instant::now();
        let at = started + Duration::from_millis(20);
        let later = at + Duration::from_secs(1);

        // The watchdog commits expiry before releasing the mutex to kill.
        // A completing reader must not clear that decision or arm another read.
        let mut monitor_first = Deadline::default();
        assert!(monitor_first.arm(at, started));
        assert!(monitor_first.expire_if_due(at));
        assert!(!monitor_first.finish(at));
        assert!(!monitor_first.arm(later, at));
        assert_eq!(monitor_first.at, Some(at));

        // Completion and rearming must also detect expiry themselves when the
        // sleeping watchdog has not yet reacquired the mutex.
        let mut completion_first = Deadline::default();
        assert!(completion_first.arm(at, started));
        assert!(!completion_first.finish(at));
        assert!(!completion_first.arm(later, at));
        assert!(completion_first.expire_if_due(at));

        let mut rearm_first = Deadline::default();
        assert!(rearm_first.arm(at, started));
        assert!(!rearm_first.arm(later, at));
        assert!(!rearm_first.finish(at));
        assert_eq!(rearm_first.at, Some(at));

        // Finishing a live read still permits another read. Rejecting an
        // already-past requested deadline does not expire an unarmed child.
        let mut live = Deadline::default();
        assert!(!live.arm(started, started));
        assert!(live.arm(at, started));
        assert!(live.finish(started));
        assert!(live.arm(later, at));
        assert!(live.finish(at));
        live.cancelled = true;
        assert!(!live.arm(later, at));
        assert!(!live.finish(at));

        let mut stopped = Deadline {
            stopped: true,
            ..Deadline::default()
        };
        assert!(!stopped.arm(later, at));
        assert!(!stopped.finish(at));
    }

    #[test]
    fn absolute_read_deadline_terminates_and_reaps_an_unread_owned_child() {
        let processes = Processes::default();
        let mut command = tool_command("ffmpeg");
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-f",
            "f32le",
            "pipe:1",
        ]);
        let mut process = Process::spawn("ffmpeg", &mut command, &processes)
            .expect("configured FFmpeg must start the task-owned deadline fixture");

        // Establish that this exact child is alive and producing bytes before
        // leaving its infinite output unread so the pipe fills and blocks it.
        process.begin_read();
        let mut first_bytes = [0; 16];
        process
            .stdout
            .read_exact(&mut first_bytes)
            .expect("owned audio fixture must produce its first bytes");
        assert!(process.end_read_checked());
        assert!(process.child.lock().unwrap().try_wait().unwrap().is_none());
        let child = Arc::clone(&process.child);
        assert!(process.begin_read_until(Instant::now() + Duration::from_millis(200)));

        // No cancellation or explicit kill is issued: the already-existing
        // deadline monitor alone must terminate this task-owned child.
        let until = Instant::now() + Duration::from_secs(3);
        let status = loop {
            if let Some(status) = child.lock().unwrap().try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < until,
                "deadline did not terminate owned child"
            );
            thread::sleep(Duration::from_millis(5));
        };
        assert!(!status.success());
        assert!(process.monitor.0.lock().unwrap().expired);
        assert!(!process.end_read_checked());
        assert!(!process.begin_read_until(Instant::now() + Duration::from_secs(1)));
        drop(process);
        assert!(child.lock().unwrap().try_wait().unwrap().is_some());

        // Weak registrations must not keep the finished monitor alive.
        while processes.retained_monitors() != 0 {
            assert!(Instant::now() < until, "finished watchdog was retained");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(processes.started_tools(), 1);
    }

    fn ffmpeg_available() -> bool {
        tool_command("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[test]
    fn the_tail_of_a_process_that_reports_and_ends_at_once_is_whole() {
        let _heavy = crate::heavy_test();
        if !ffmpeg_available() {
            eprintln!("SKIP: FFmpeg is unavailable; the error tail fixture was not run");
            return;
        }
        // A tenth of a second of tone, measured: FFmpeg writes its report to
        // the error pipe as it ends, and its stdout ends with it.
        let processes = Processes::default();
        for run in 0..30 {
            let mut command = tool_command("ffmpeg");
            command.args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "info",
                "-nostats",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:duration=0.1",
                "-af",
                "ebur128=peak=true:framelog=verbose",
                "-f",
                "null",
                "-",
            ]);
            let mut process = Process::spawn("ffmpeg", &mut command, &processes).unwrap();
            process.begin_read();
            let mut out = Vec::new();
            process.stdout.read_to_end(&mut out).unwrap();
            assert!(process.end_read_checked());
            let until = Instant::now() + Duration::from_secs(10);
            // Asked without a pause, so that the tail is read the moment the
            // process is seen to have ended, ahead of its reader if it can be.
            while process.child.lock().unwrap().try_wait().unwrap().is_none() {
                assert!(Instant::now() < until, "the fixture did not end");
                std::hint::spin_loop();
            }
            // The report is there to its last line, after the summary.
            let tail = process.stderr_tail();
            let summary = tail.rfind("Summary:").unwrap_or_else(|| {
                panic!("run {run}: no summary in {tail:?}");
            });
            let after = &tail[summary..];
            assert!(after.contains("Peak:"), "run {run}: {tail:?}");
            let last = tail.trim_end().lines().last().unwrap_or_default();
            assert!(last.starts_with("size="), "run {run}: {tail:?}");
            assert!(tail.ends_with('\n'), "run {run}: {tail:?}");
        }
        assert_eq!(processes.started_tools(), 30);
    }

    /// A shell that writes a line and ends at once, leaving a child of its own
    /// to write another to the same pipe `delay` seconds on.
    #[cfg(unix)]
    fn ends_before_its_pipe(delay: &str, processes: &Processes) -> Process {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!("(sleep {delay}; echo late >&2) & echo early >&2"),
        ]);
        let process = Process::spawn("sh", &mut command, processes).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while process.child.lock().unwrap().try_wait().unwrap().is_none() {
            assert!(Instant::now() < until, "the shell did not end");
            thread::sleep(Duration::from_millis(1));
        }
        process
    }

    #[cfg(unix)]
    #[test]
    fn once_the_process_has_ended_the_tail_waits_for_its_pipe_and_no_longer_than_its_bound() {
        let _heavy = crate::heavy_test();
        let processes = Processes::default();
        // The pipe ends 50 ms after the process: the tail waits for it.
        let process = ends_before_its_pipe("0.05", &processes);
        assert_eq!(process.stderr_tail(), "early\nlate\n");
        // It ends a second after: the tail is what came within the bound.
        let process = ends_before_its_pipe("1", &processes);
        let asked = Instant::now();
        assert_eq!(process.stderr_tail(), "early\n");
        let waited = asked.elapsed();
        assert!(
            waited >= TAIL_WAIT && waited < TAIL_WAIT + Duration::from_millis(200),
            "{waited:?}"
        );
        // And not again once the pipe has ended: the reader said so.
        let process = ends_before_its_pipe("0.02", &processes);
        thread::sleep(Duration::from_millis(150));
        let asked = Instant::now();
        assert_eq!(process.stderr_tail(), "early\nlate\n");
        assert!(asked.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn the_tail_keeps_the_last_bytes_and_waits_for_nothing_while_the_process_runs() {
        let _heavy = crate::heavy_test();
        if !ffmpeg_available() {
            eprintln!("SKIP: FFmpeg is unavailable; the error tail bound was not run");
            return;
        }
        // Every frame of a second of picture is reported, far more than the
        // tail keeps.
        let processes = Processes::default();
        let mut command = tool_command("ffmpeg");
        command.args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "info",
            "-nostats",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=32x32:r=100:d=1",
            "-vf",
            "showinfo",
            "-f",
            "null",
            "-",
        ]);
        let mut process = Process::spawn("ffmpeg", &mut command, &processes).unwrap();
        process.begin_read();
        process.stdout.read_to_end(&mut Vec::new()).unwrap();
        process.end_read();
        while process.child.lock().unwrap().try_wait().unwrap().is_none() {
            thread::sleep(Duration::from_millis(1));
        }
        let tail = process.stderr_tail();
        assert!(
            tail.len() <= STDERR_TAIL && tail.len() > STDERR_TAIL - 4,
            "{}",
            tail.len()
        );
        // Of picture, FFmpeg's last line counts frames.
        let last = tail.trim_end().lines().last().unwrap_or_default();
        assert!(last.starts_with("frame=  100"), "{last}");

        // A process that runs and writes nothing: its tail is empty at once.
        let mut command = tool_command("ffmpeg");
        command.args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-f",
            "f32le",
            "pipe:1",
        ]);
        let running = Process::spawn("ffmpeg", &mut command, &processes).unwrap();
        let asked = Instant::now();
        assert_eq!(running.stderr_tail(), "");
        assert!(asked.elapsed() < TAIL_WAIT);
    }

    #[test]
    fn cancellation_wakes_and_reaps_a_child_blocked_on_an_unread_pipe() {
        if !ffmpeg_available() {
            eprintln!("FFmpeg is unavailable; subprocess cancellation fixture skipped");
            return;
        }
        let processes = Processes::default();
        let mut command = tool_command("ffmpeg");
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-f",
            "f32le",
            "pipe:1",
        ]);
        let process = Process::spawn("ffmpeg", &mut command, &processes).unwrap();
        processes.kill_all();
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if process.child.lock().unwrap().try_wait().unwrap().is_some() {
                break;
            }
            assert!(Instant::now() < until, "cancelled decoder did not exit");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_write_deadline_kills_a_child_that_stops_reading_its_input() {
        if !ffmpeg_available() {
            eprintln!("FFmpeg is unavailable; subprocess write deadline fixture skipped");
            return;
        }
        let processes = Processes::default();
        let mut command = tool_command("ffmpeg");
        // This child never reads its input, so a large write fills the pipe.
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-f",
            "null",
            "-",
        ]);
        let mut process = Process::spawn_writer("ffmpeg", &mut command, &processes).unwrap();
        let started = Instant::now();
        let written = process.write_until(&vec![0; 8 << 20], started + Duration::from_millis(200));
        assert!(written.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        let status = process
            .wait_until(Instant::now() + Duration::from_secs(3))
            .unwrap();
        assert!(status.is_some_and(|status| !status.success()));
        // A dead writer takes nothing more.
        assert!(
            process
                .write_until(&[0; 4], Instant::now() + Duration::from_secs(1))
                .is_err()
        );
        // Plain children still get no input pipe.
        let mut command = tool_command("ffmpeg");
        command.arg("-version");
        let mut plain = Process::spawn("ffmpeg", &mut command, &processes).unwrap();
        assert!(
            plain
                .write_until(&[0; 4], Instant::now() + Duration::from_secs(1))
                .is_err()
        );
    }
}
