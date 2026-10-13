//! Execute the real private transport against a separately built native helper.
//! The explicit fixture path is mandatory; unsupported ownership probes stay
//! target-scoped while protocol/cancellation cases run on every native target.
use super::*;
use serde_json::json;
use std::path::PathBuf;

const DROP_BOUND: Duration = Duration::from_secs(2);
const OUTPUT_BUDGET_ERROR: &str = "AI provider exceeded its output budget";

pub(super) fn fixture_command(mode: &str) -> Command {
    let path = PathBuf::from(std::env::var_os("ASTER_PROVIDER_TEST_FIXTURE").expect(
        "ASTER_PROVIDER_TEST_FIXTURE must name the absolute built provider-fault-fixture example; run scripts/test-workspace.py or build the example first",
    ));
    assert!(
        path.is_absolute() && path.is_file(),
        "ASTER_PROVIDER_TEST_FIXTURE must be an existing absolute native executable: {}",
        path.display()
    );
    let mut command = Command::new(path);
    // A fallback ten seconds later cannot masquerade as the asserted two-second
    // adapter teardown, cancellation or one-second request deadline.
    command.args([mode, "--ttl-ms", "10000"]);
    command
}

fn fixture(mode: &str, deadline_seconds: u64) -> Process {
    Process::spawn(
        fixture_command(mode),
        Arc::new(AtomicBool::new(false)),
        deadline_seconds,
    )
    .unwrap()
}

fn drop_within_bound(process: Process) {
    let started = Instant::now();
    drop(process);
    assert!(
        started.elapsed() < DROP_BOUND,
        "provider teardown exceeded the bound"
    );
}

#[test]
fn fragmented_ndjson_decodes_and_clean_eof_is_not_a_proposal() {
    let mut process = fixture("split-line", 5);
    assert_eq!(process.next().unwrap(), json!({"value":7}));
    assert!(matches!(process.next(), Err(AiError::Provider(message))
            if message == "AI provider exited before completing the request"));
    drop_within_bound(process);
}

#[test]
fn invalid_protocol_line_has_its_own_error_and_bounded_teardown() {
    let mut process = fixture("invalid-line", 5);
    assert!(matches!(process.next(), Err(AiError::Provider(message))
            if message == "AI provider returned an invalid protocol message"));
    drop_within_bound(process);
}

#[test]
fn oversized_line_reports_output_limit_and_bounded_teardown() {
    let mut process = fixture("oversized-line", 5);
    assert!(matches!(process.next(), Err(AiError::Provider(message))
            if message == OUTPUT_BUDGET_ERROR));
    assert_eq!(process.reason.load(Ordering::Acquire), OUTPUT_LIMIT);
    drop_within_bound(process);
}

#[test]
fn stderr_flood_is_drained_and_reports_only_the_output_budget_error() {
    let mut process = fixture("stderr-flood-hold", 5);
    assert!(matches!(process.next(), Err(AiError::Provider(message))
            if message == OUTPUT_BUDGET_ERROR));
    assert_eq!(process.reason.load(Ordering::Acquire), OUTPUT_LIMIT);
    drop_within_bound(process);
}

#[test]
fn cancellation_unblocks_an_owned_provider_that_never_reads_stdin() {
    let mut process = fixture("never-read-stdin", 5);
    let identity = process.next().unwrap();
    assert_eq!(identity["event"], "never_read_stdin");
    assert_eq!(identity["ttl_ms"], 10000);
    let cancel = process.cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        cancel.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let outcome = process.send_text(&"x".repeat(MAX_OUTPUT_BYTES));
    // Join the only auxiliary test thread before asserting, including on failure.
    canceller.join().unwrap();
    let cancelled = matches!(outcome, Err(AiError::Cancelled));
    drop_within_bound(process);
    assert!(cancelled, "blocked input did not report cancellation");
    assert!(started.elapsed() < DROP_BOUND);
}

#[test]
fn request_deadline_terminates_stalled_output_before_fixture_fallback() {
    let mut process = fixture("pipe-holder", 1);
    let started = Instant::now();
    assert!(matches!(process.next(), Err(AiError::Deadline)));
    assert_eq!(process.reason.load(Ordering::Acquire), DEADLINE);
    drop_within_bound(process);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn dropping_a_full_bounded_stdout_queue_releases_reader_and_owned_child() {
    let mut process = fixture("stdout-flood", 5);
    assert_eq!(process.next().unwrap(), json!({"event":1}));
    thread::sleep(Duration::from_millis(80));
    // The fixture emits many more than the queue's eight frames. Confirm eight
    // actual unread frames rather than treating a process spawn as queue proof.
    for _ in 0..8 {
        let event = process.output.as_ref().unwrap().try_recv().unwrap();
        assert!(matches!(event, Event::Json(value) if value == json!({"event":1})));
    }
    thread::sleep(Duration::from_millis(80));
    assert!(!process.readers[0].is_finished());
    drop_within_bound(process);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn exited_parent_leaves_a_retained_descendant_that_transport_drop_terminates() {
    let mut process = fixture("parent-with-child", 5);
    let report = process.next().unwrap();
    assert_eq!(report["event"], "descendant");
    assert_eq!(report["ttl_ms"], 10000);
    let pid = u32::try_from(report["pid"].as_u64().unwrap()).unwrap();
    let parent_id = process.owned.child.lock().unwrap().id();
    assert_eq!(report["parent_pid"], parent_id);
    assert_ne!(pid, parent_id);
    // Open before waiting for the parent. The fixture child is still alive and
    // the retained kernel identity, not a later reused PID, owns all cleanup.
    let descendant = owned_descendant::RetainedChild::open(pid);
    let parent_deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if let Some(status) = process.owned.child.lock().unwrap().try_wait().unwrap() {
            assert!(status.success(), "reporting parent did not exit normally");
            break;
        }
        assert!(
            Instant::now() < parent_deadline,
            "fixture did not exercise cleanup after immediate-parent exit"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!descendant.wait_timeout(Duration::ZERO));
    // The still-live descendant holds both output pipes. Readers must still be
    // active until the transport terminates its owned process group/job.
    assert!(process.readers.iter().all(|reader| !reader.is_finished()));
    let started = Instant::now();
    drop_within_bound(process);
    assert!(descendant.wait_timeout(Duration::from_secs(1)));
    assert!(started.elapsed() < DROP_BOUND);
}

#[cfg(windows)]
mod owned_descendant {
    use std::{io, time::Duration};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
            WaitForSingleObject,
        },
    };

    pub(super) struct RetainedChild(HANDLE);

    impl RetainedChild {
        pub(super) fn open(pid: u32) -> Self {
            // SAFETY: the bounded report identified this fixture's live child.
            // Minimal wait/terminate rights; the returned handle is not inherited.
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
            assert!(
                !handle.is_null(),
                "cannot retain the owned fixture descendant: {}",
                io::Error::last_os_error()
            );
            Self(handle)
        }

        pub(super) fn wait_timeout(&self, timeout: Duration) -> bool {
            let milliseconds = timeout.as_millis().min(u32::MAX as u128) as u32;
            // SAFETY: the sole-owned process handle remains valid throughout wait.
            let result = unsafe { WaitForSingleObject(self.0, milliseconds) };
            assert!(
                result == WAIT_OBJECT_0 || result == WAIT_TIMEOUT,
                "cannot wait for the retained fixture descendant: {}",
                io::Error::last_os_error()
            );
            result == WAIT_OBJECT_0
        }
    }

    impl Drop for RetainedChild {
        fn drop(&mut self) {
            // SAFETY: fallback cleanup uses the retained kernel handle only,
            // even when an assertion unwinds. No reused PID can be terminated.
            unsafe {
                if WaitForSingleObject(self.0, 0) != WAIT_OBJECT_0 {
                    let _ = TerminateProcess(self.0, 1);
                    let _ = WaitForSingleObject(self.0, 1000);
                }
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod owned_descendant {
    use std::{
        io,
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
        time::{Duration, Instant},
    };

    pub(super) struct RetainedChild(OwnedFd);

    impl RetainedChild {
        pub(super) fn open(pid: u32) -> Self {
            // SAFETY: this is a read-only identity acquisition for the reported
            // live child. Flags zero are required by pidfd_open.
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
            assert!(
                fd >= 0,
                "cannot retain the owned fixture descendant: {}",
                io::Error::last_os_error()
            );
            // SAFETY: a successful pidfd_open returned this new owned descriptor.
            Self(unsafe { OwnedFd::from_raw_fd(fd as i32) })
        }

        pub(super) fn wait_timeout(&self, timeout: Duration) -> bool {
            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let milliseconds = remaining.as_millis().min(i32::MAX as u128) as i32;
                let mut descriptor = libc::pollfd {
                    fd: self.0.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: one writable pollfd and a valid owned identity descriptor.
                let ready = unsafe { libc::poll(&mut descriptor, 1, milliseconds) };
                if ready >= 0 {
                    return ready != 0 && descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0;
                }
                let error = io::Error::last_os_error();
                assert_eq!(error.kind(), io::ErrorKind::Interrupted);
                if Instant::now() >= deadline {
                    return false;
                }
            }
        }
    }

    impl Drop for RetainedChild {
        fn drop(&mut self) {
            let mut descriptor = libc::pollfd {
                fd: self.0.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: all operations use this retained pidfd. Fallback cleanup
            // never signals an arbitrary numeric PID and waits are bounded.
            unsafe {
                if libc::poll(&mut descriptor, 1, 0) <= 0 {
                    let _ = libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        self.0.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    );
                    let _ = libc::poll(&mut descriptor, 1, 1000);
                }
            }
        }
    }
}
