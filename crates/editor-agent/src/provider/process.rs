//! Bounded NDJSON transport with worker-owned process trees and a separate
//! watchdog. Even a provider that never reads stdin cannot block cancellation.
use crate::{AiError, planner::MAX_OUTPUT_BYTES};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const CANCELLED: u8 = 1;
const DEADLINE: u8 = 2;
const OUTPUT_LIMIT: u8 = 3;
const STOPPED: u8 = 4;
const MAX_STREAM_BYTES: usize = 32 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 256 * 1024;

/// How much a provider may print. A planning request is one bounded answer;
/// an agent conversation goes on, and a tool's pictures travel through its
/// lines, so its lines may be larger and its stream is not counted whole.
#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub line: usize,
    pub stream: usize,
    pub stderr: usize,
}

impl Limits {
    pub(super) const PLANNING: Self = Self {
        line: MAX_OUTPUT_BYTES,
        stream: MAX_STREAM_BYTES,
        stderr: MAX_STDERR_BYTES,
    };
    /// A line holds at most one tool result, whose pictures are capped by the
    /// tool; what the agent printed before is not held.
    pub(super) const CONVERSATION: Self = Self {
        line: 256 * 1024 * 1024,
        stream: usize::MAX,
        stderr: usize::MAX,
    };
}

enum Event {
    Json(Value),
    Invalid,
    Eof,
}

struct OwnedChild {
    child: Mutex<Child>,
    group: Group,
}

impl OwnedChild {
    fn terminate(&self) {
        // Kill the entire owned group even if the immediate CLI has exited and
        // one of its descendants still holds a pipe open.
        self.group.terminate();
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(crate) struct Process {
    input: Option<ChildStdin>,
    output: Option<Receiver<Event>>,
    reason: Arc<AtomicU8>,
    cancel: Arc<AtomicBool>,
    monitor: Option<JoinHandle<()>>,
    readers: Vec<JoinHandle<()>>,
    owned: Arc<OwnedChild>,
    directory: tempfile::TempDir,
}

impl Process {
    pub(super) fn spawn(
        command: Command,
        cancel: Arc<AtomicBool>,
        timeout_seconds: u64,
    ) -> Result<Self, AiError> {
        Self::spawn_with(command, cancel, timeout_seconds, |_, _| Ok(()))
    }

    /// Configure provider-specific inputs only after its private directory
    /// exists, before any provider code/configuration can start executing.
    pub(super) fn spawn_with(
        command: Command,
        cancel: Arc<AtomicBool>,
        timeout_seconds: u64,
        configure: impl FnOnce(&mut Command, &Path) -> Result<(), AiError>,
    ) -> Result<Self, AiError> {
        Self::spawn_limited(
            command,
            cancel,
            timeout_seconds,
            Limits::PLANNING,
            configure,
        )
    }

    pub(super) fn spawn_limited(
        mut command: Command,
        cancel: Arc<AtomicBool>,
        timeout_seconds: u64,
        limits: Limits,
        configure: impl FnOnce(&mut Command, &Path) -> Result<(), AiError>,
    ) -> Result<Self, AiError> {
        if cancel.load(Ordering::Acquire) {
            return Err(AiError::Cancelled);
        }
        let mut directory_builder = tempfile::Builder::new();
        directory_builder.prefix("neptune-video-ai-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            directory_builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let directory = directory_builder
            .tempdir()
            .map_err(|_| AiError::Provider("Cannot create private AI working directory".into()))?;
        configure(&mut command, directory.path())?;
        if cancel.load(Ordering::Acquire) {
            return Err(AiError::Cancelled);
        }
        // A relative path configured by the person is relative to the app's
        // launch directory, not to the private temporary planning directory.
        let program = Path::new(command.get_program());
        if program.is_relative() && program.components().count() > 1 {
            let resolved = std::env::current_dir()
                .map_err(|_| AiError::Provider("Cannot resolve AI executable".into()))?
                .join(program);
            let mut resolved_command = Command::new(resolved);
            resolved_command.args(command.get_args());
            for (key, value) in command.get_envs() {
                if let Some(value) = value {
                    resolved_command.env(key, value);
                } else {
                    resolved_command.env_remove(key);
                }
            }
            command = resolved_command;
        }
        command
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (mut child, group) = spawn_group(&mut command)?;
        let input = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let owned = Arc::new(OwnedChild {
            child: Mutex::new(child),
            group,
        });
        let reason = Arc::new(AtomicU8::new(0));
        let (send, receive) = mpsc::sync_channel(8);
        let mut process = Self {
            input,
            output: Some(receive),
            reason: reason.clone(),
            cancel: cancel.clone(),
            monitor: None,
            readers: Vec::with_capacity(2),
            owned: owned.clone(),
            directory,
        };
        #[cfg(unix)]
        {
            // All three pipe ends are owned by this request. Nonblocking pipes
            // keep cancellation bounded even if a descendant escapes the group.
            nonblocking(process.input.as_ref().ok_or(AiError::WorkerStopped)?)?;
            nonblocking(stdout.as_ref().ok_or(AiError::WorkerStopped)?)?;
            nonblocking(stderr.as_ref().ok_or(AiError::WorkerStopped)?)?;
        }
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        process.monitor = Some(
            thread::Builder::new()
                .name("neptune-video-ai-watchdog".into())
                .spawn(move || {
                    loop {
                        if cancel.load(Ordering::Acquire) {
                            let _ = reason.compare_exchange(
                                0,
                                CANCELLED,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            );
                        } else if Instant::now() >= deadline {
                            let _ = reason.compare_exchange(
                                0,
                                DEADLINE,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            );
                        }
                        if reason.load(Ordering::Acquire) != 0 {
                            break;
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                    owned.terminate();
                })
                .map_err(|_| AiError::WorkerStopped)?,
        );
        let reason = process.reason.clone();
        let stdout = stdout.ok_or(AiError::WorkerStopped)?;
        process.readers.push(
            thread::Builder::new()
                .name("neptune-video-ai-stdout".into())
                .spawn(move || read_stdout(stdout, send, reason, limits))
                .map_err(|_| AiError::WorkerStopped)?,
        );
        let reason = process.reason.clone();
        let stderr = stderr.ok_or(AiError::WorkerStopped)?;
        process.readers.push(
            thread::Builder::new()
                .name("neptune-video-ai-stderr".into())
                .spawn(move || drain_stderr(stderr, reason, limits.stderr))
                .map_err(|_| AiError::WorkerStopped)?,
        );
        Ok(process)
    }

    pub(super) fn directory(&self) -> &Path {
        self.directory.path()
    }

    pub(crate) fn send(&mut self, value: &Value) -> Result<(), AiError> {
        let bytes = serde_json::to_vec(value)
            .map_err(|_| AiError::Invalid("Cannot encode AI protocol request".into()))?;
        self.write(&bytes)?;
        self.write(b"\n")
    }

    pub(super) fn send_text(&mut self, text: &str) -> Result<(), AiError> {
        self.write(text.as_bytes())?;
        self.write(b"\n")
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), AiError> {
        let mut bytes = bytes;
        while !bytes.is_empty() {
            self.check()?;
            match self
                .input
                .as_mut()
                .ok_or_else(|| AiError::Provider("AI input is closed".into()))?
                .write(bytes)
            {
                Ok(0) => return Err(AiError::Provider("AI provider closed its input".into())),
                Ok(count) => bytes = &bytes[count..],
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => {
                    self.check()?;
                    return Err(AiError::Provider("AI provider closed its input".into()));
                }
            }
        }
        self.check()
    }

    pub(super) fn close_input(&mut self) {
        self.input.take();
    }

    pub(super) fn next(&mut self) -> Result<Value, AiError> {
        loop {
            if let Some(value) = self.poll(Duration::from_millis(25))? {
                return Ok(value);
            }
        }
    }

    /// The next message if one comes within `wait`.
    pub(crate) fn poll(&mut self, wait: Duration) -> Result<Option<Value>, AiError> {
        self.check()?;
        let result = self
            .output
            .as_ref()
            .ok_or(AiError::WorkerStopped)?
            .recv_timeout(wait);
        self.check()?;
        match result {
            Ok(Event::Json(value)) => Ok(Some(value)),
            Ok(Event::Invalid) => Err(AiError::Provider(
                "AI provider returned an invalid protocol message".into(),
            )),
            Ok(Event::Eof) | Err(RecvTimeoutError::Disconnected) => Err(AiError::Provider(
                "AI provider exited before completing the request".into(),
            )),
            Err(RecvTimeoutError::Timeout) => Ok(None),
        }
    }

    fn check(&self) -> Result<(), AiError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(AiError::Cancelled);
        }
        match self.reason.load(Ordering::Acquire) {
            CANCELLED => Err(AiError::Cancelled),
            DEADLINE => Err(AiError::Deadline),
            OUTPUT_LIMIT => Err(AiError::Provider(
                "AI provider exceeded its output budget".into(),
            )),
            STOPPED => Err(AiError::WorkerStopped),
            _ => Ok(()),
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self
            .reason
            .compare_exchange(0, STOPPED, Ordering::AcqRel, Ordering::Acquire);
        // Releasing the receiver unblocks a full bounded stdout queue before
        // joining its reader. The watchdog also unblocks any stuck pipe I/O.
        self.output.take();
        self.input.take();
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        } else {
            self.owned.terminate();
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}

fn read_stdout(stdout: impl Read, send: SyncSender<Event>, reason: Arc<AtomicU8>, limits: Limits) {
    let mut reader = BufReader::with_capacity(8192, stdout);
    let mut line = Vec::new();
    let mut total = 0usize;
    loop {
        if reason.load(Ordering::Acquire) != 0 {
            return;
        }
        let buffer = match reader.fill_buf() {
            Ok(buffer) => buffer,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => {
                let _ = send.send(Event::Invalid);
                return;
            }
        };
        if buffer.is_empty() {
            if !line.is_empty() && !emit_line(&line, &send) {
                return;
            }
            let _ = send.send(Event::Eof);
            return;
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(buffer.len(), |at| at + 1);
        total = total.saturating_add(take);
        if total > limits.stream || take > limits.line.saturating_sub(line.len()) {
            let _ = reason.compare_exchange(0, OUTPUT_LIMIT, Ordering::AcqRel, Ordering::Acquire);
            return;
        }
        line.extend_from_slice(&buffer[..take]);
        reader.consume(take);
        if newline.is_some() {
            if !emit_line(&line, &send) {
                return;
            }
            line.clear();
        }
    }
}

fn emit_line(line: &[u8], send: &SyncSender<Event>) -> bool {
    if line.iter().all(u8::is_ascii_whitespace) {
        return true;
    }
    match serde_json::from_slice(line) {
        Ok(value) => send.send(Event::Json(value)).is_ok(),
        Err(_) => {
            let _ = send.send(Event::Invalid);
            false
        }
    }
}

fn drain_stderr(mut stderr: impl Read, reason: Arc<AtomicU8>, limit: usize) {
    let mut buffer = [0u8; 8192];
    let mut total = 0usize;
    loop {
        if reason.load(Ordering::Acquire) != 0 {
            return;
        }
        match stderr.read(&mut buffer) {
            Ok(0) => return,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return,
            Ok(count) => {
                total = total.saturating_add(count);
                if total > limit {
                    let _ = reason.compare_exchange(
                        0,
                        OUTPUT_LIMIT,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    return;
                }
            }
        }
    }
}

#[cfg(unix)]
struct Group {
    pid: i32,
}

#[cfg(unix)]
fn spawn_group(command: &mut Command) -> Result<(Child, Group), AiError> {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    let child = command.spawn().map_err(|_| {
        AiError::Provider(
            "Cannot start native AI provider; check the executable and its installation".into(),
        )
    })?;
    let group = Group {
        pid: child.id() as i32,
    };
    Ok((child, group))
}

#[cfg(unix)]
impl Group {
    fn terminate(&self) {
        // SAFETY: this PID became its own process group in the owned spawn.
        // A negative PID targets that group, never the editor's group.
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), AiError> {
    // SAFETY: these live owned pipe descriptors are valid, and preserving all
    // existing flags changes only this pipe's nonblocking status.
    unsafe {
        let fd = pipe.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags == -1 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            return Err(AiError::Provider(
                "Cannot configure bounded AI transport".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
use super::windows::{Group, spawn as spawn_group};

#[cfg(test)]
#[path = "process_tests.rs"]
mod portable_tests;

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(target_os = "linux")]
    fn shell(script: &str, timeout: u64) -> Process {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        Process::spawn(command, Arc::new(AtomicBool::new(false)), timeout).unwrap()
    }

    #[test]
    fn provider_configuration_sees_a_private_directory_and_failure_cleans_it_before_spawn() {
        use std::os::unix::fs::PermissionsExt;
        let mut captured = None;
        let result = Process::spawn_with(
            Command::new("/neptune-video-owned-provider-must-not-start"),
            Arc::new(AtomicBool::new(false)),
            5,
            |_, directory| {
                assert_eq!(
                    std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                    0o700
                );
                captured = Some(directory.to_path_buf());
                Err(AiError::Invalid("Owned configuration refused".into()))
            },
        );
        assert!(
            matches!(result, Err(AiError::Invalid(message)) if message == "Owned configuration refused")
        );
        assert!(!captured.unwrap().exists());
    }

    #[test]
    fn private_planning_directory_has_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let process = Process::spawn(
            portable_tests::fixture_command("pipe-holder"),
            Arc::new(AtomicBool::new(false)),
            5,
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(process.directory())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        drop(process);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn escaped_descendant_cannot_hold_pipe_cleanup_hostage() {
        struct Escaped(i32);
        impl Drop for Escaped {
            fn drop(&mut self) {
                // SAFETY: this PID was returned by this test's owned child.
                unsafe {
                    libc::kill(self.0, libc::SIGKILL);
                }
            }
        }
        let mut process = shell(
            "setsid sleep 60 & printf '{\"pid\":%s}\\n' \"$!\"; sleep 0.05",
            5,
        );
        let escaped = Escaped(process.next().unwrap()["pid"].as_i64().unwrap() as i32);
        thread::sleep(Duration::from_millis(80));
        let started = Instant::now();
        drop(process);
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(escaped);
    }

    #[test]
    fn relative_executable_is_resolved_before_private_cwd() {
        use std::os::unix::fs::PermissionsExt;
        let cwd = std::env::current_dir().unwrap();
        let directory = tempfile::Builder::new()
            .prefix("relative-provider-")
            .tempdir_in(&cwd)
            .unwrap();
        let path = directory.path().join("provider");
        std::fs::write(&path, "#!/bin/sh\nprintf '{\"relative\":true}\\n'\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let relative = path.strip_prefix(&cwd).unwrap();
        let mut process =
            Process::spawn(Command::new(relative), Arc::new(AtomicBool::new(false)), 5).unwrap();
        assert_eq!(process.next().unwrap(), json!({"relative":true}));
    }
}
