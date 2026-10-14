//! Task-owned subprocesses. Blocking reads have a sleeping deadline monitor;
//! cancellation kills only registered children, and the UI never joins workers.
use crate::{MediaError, io};
use std::{
    io::{Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, Weak,
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

pub(crate) struct Process {
    pub child: Arc<Mutex<Child>>,
    pub stdout: ChildStdout,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// Set only by `spawn_writer`: the child's input pipe until closed.
    stdin: Option<ChildStdin>,
    stderr_reader: Option<thread::JoinHandle<()>>,
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
        let mut instance = Self {
            child,
            stdout,
            stderr: Arc::new(Mutex::new(Vec::new())),
            stdin: None,
            stderr_reader: None,
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
            let reader = thread::Builder::new()
                .name("aster-media-stderr".into())
                .spawn(move || {
                    let mut bytes = [0; 1024];
                    while let Ok(count) = pipe.read(&mut bytes) {
                        if count == 0 {
                            break;
                        }
                        if let Ok(mut tail) = tail.lock() {
                            tail.extend_from_slice(&bytes[..count]);
                            let excess = tail.len().saturating_sub(8192);
                            if excess != 0 {
                                tail.drain(..excess);
                            }
                        }
                    }
                })
                .map_err(|error| io("Cannot start media error reader", error))?;
            instance.stderr_reader = Some(reader);
        }
        let monitor_child = Arc::clone(&instance.child);
        let monitor = Arc::clone(&instance.monitor);
        thread::Builder::new()
            .name("aster-media-deadline".into())
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
    /// 8 KiB, once that output has closed or `until` has passed.
    pub fn stderr_until(&mut self, until: Instant) -> String {
        if let Some(reader) = self.stderr_reader.take() {
            while !reader.is_finished() && Instant::now() < until {
                thread::sleep(Duration::from_millis(2));
            }
            if !reader.is_finished() {
                self.stderr_reader = Some(reader);
            }
        }
        self.stderr
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    }

    pub fn failure(&self, detail: &str) -> MediaError {
        let tail = self
            .stderr
            .lock()
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .unwrap_or_default();
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
    let variable = if tool == "ffmpeg" {
        "ASTER_FFMPEG"
    } else {
        "ASTER_FFPROBE"
    };
    Command::new(std::env::var_os(variable).unwrap_or_else(|| tool.into()))
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

    #[test]
    fn cancellation_wakes_and_reaps_a_child_blocked_on_an_unread_pipe() {
        if !tool_command("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
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
        if !tool_command("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
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
