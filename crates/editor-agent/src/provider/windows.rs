//! Contain a suspended provider before any of its code can spawn children.
use crate::AiError;
use std::{
    mem,
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, Command},
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, GetProcessIdOfThread, OpenThread, ResumeThread,
            THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
        },
    },
};

const MAX_SNAPSHOT_THREADS: usize = 131_072;
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(1);
const OWNER_FIELD_END: usize =
    mem::offset_of!(THREADENTRY32, th32OwnerProcessID) + mem::size_of::<u32>();

/// A sole-owned kernel handle. Its integer storage can cross the watchdog's
/// thread boundary; the underlying job is a thread-safe kernel object.
struct Handle(usize);

impl Handle {
    fn raw(&self) -> HANDLE {
        self.0 as HANDLE
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: only valid owned handles construct Handle; it is not cloned,
        // and every successful construction reaches exactly one CloseHandle.
        unsafe {
            CloseHandle(self.raw());
        }
    }
}

pub(super) struct Group {
    job: Handle,
}

impl Group {
    fn new() -> Result<Self, AiError> {
        // SAFETY: null attributes/name create an anonymous non-inheritable job.
        // The returned handle is owned before any later operation can fail.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(error("Cannot create AI provider process job"));
        }
        let job = Handle(handle as usize);
        // SAFETY: this POD structure accepts all-zero initialization; its size
        // and information class match SetInformationJobObject's buffer.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // Breakaway is deliberately absent: every later descendant stays owned.
        let configured = unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                mem::size_of_val(&limits) as u32,
            )
        };
        if configured == 0 {
            return Err(error("Cannot configure AI provider process job"));
        }
        Ok(Self { job })
    }

    pub(super) fn terminate(&self) {
        // SAFETY: the sole-owned job contains only this provider and its children.
        unsafe {
            TerminateJobObject(self.job.raw(), 1);
        }
    }
}

/// Stable Rust leaves the initial Windows thread handle private, so discover
/// that still-suspended thread through Toolhelp and verify its owner before
/// resuming. All discovery is bounded and fails closed on ambiguity.
pub(super) fn spawn(command: &mut Command) -> Result<(Child, Group), AiError> {
    let group = Group::new()?;
    command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|_| error("Cannot start native AI provider; check its executable"))?;
    if let Err(failure) = contain_and_resume(&mut child, &group) {
        // Before assignment the child is still suspended and has spawned no
        // children. After assignment the job also closes any owned descendants.
        group.terminate();
        let _ = child.kill();
        let _ = child.wait();
        return Err(failure);
    }
    Ok((child, group))
}

fn contain_and_resume(child: &mut Child, group: &Group) -> Result<(), AiError> {
    // SAFETY: both handles remain owned throughout assignment; std's process
    // handle has the quota/termination rights required by this operation.
    let assigned = unsafe { AssignProcessToJobObject(group.job.raw(), child.as_raw_handle()) };
    if assigned == 0 {
        return Err(error("Cannot contain AI provider process tree"));
    }
    let thread = primary_thread(child.id())?;
    if child
        .try_wait()
        .map_err(|_| error("Cannot inspect suspended AI provider"))?
        .is_some()
    {
        return Err(error("AI provider exited before safe startup"));
    }
    // SAFETY: the thread was verified to belong to the contained, suspended
    // child. A successful startup removes exactly our single suspend count.
    if unsafe { ResumeThread(thread.raw()) } != 1 {
        return Err(error("Cannot resume the contained AI provider thread"));
    }
    Ok(())
}

fn primary_thread(pid: u32) -> Result<Handle, AiError> {
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    // TH32CS_SNAPTHREAD covers the whole system; the PID argument is ignored.
    // SAFETY: no pointers are supplied and every valid snapshot is owned below.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw == INVALID_HANDLE_VALUE || raw.is_null() {
        return Err(error("Cannot discover suspended AI provider thread"));
    }
    let snapshot = Handle(raw as usize);
    // SAFETY: zero is valid for THREADENTRY32; dwSize initializes the ABI header.
    let mut entry: THREADENTRY32 = unsafe { mem::zeroed() };
    entry.dwSize = mem::size_of::<THREADENTRY32>() as u32;
    // SAFETY: snapshot and writable structure remain valid through enumeration.
    let mut found = unsafe { Thread32First(snapshot.raw(), &mut entry) } != 0;
    let mut candidate = None;
    let mut count = 0;
    while found {
        if count == MAX_SNAPSHOT_THREADS || Instant::now() >= deadline {
            return Err(error("AI provider thread discovery exceeded its bound"));
        }
        count += 1;
        if (entry.dwSize as usize) < OWNER_FIELD_END {
            return Err(error(
                "AI provider thread snapshot has an unsupported shape",
            ));
        }
        if entry.th32OwnerProcessID == pid && candidate.replace(entry.th32ThreadID).is_some() {
            return Err(error("AI provider startup thread is ambiguous"));
        }
        entry.dwSize = mem::size_of::<THREADENTRY32>() as u32;
        // SAFETY: the snapshot is still owned and the structure was reset above.
        found = unsafe { Thread32Next(snapshot.raw(), &mut entry) } != 0;
    }
    // SAFETY: checked immediately after the enumeration call that failed.
    if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
        return Err(error("Cannot enumerate AI provider startup threads"));
    }
    let tid = candidate.ok_or_else(|| error("Suspended AI provider thread was not found"))?;
    // SAFETY: minimal query/resume rights only; no inheritable thread handle.
    let raw = unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
            0,
            tid,
        )
    };
    if raw.is_null() {
        return Err(error("Cannot open suspended AI provider thread"));
    }
    let thread = Handle(raw as usize);
    // SAFETY: query rights were requested above. This also detects TID reuse
    // between the snapshot and OpenThread; another process is never resumed.
    if unsafe { GetProcessIdOfThread(thread.raw()) } != pid {
        return Err(error("AI provider startup thread ownership changed"));
    }
    Ok(thread)
}

fn error(message: &str) -> AiError {
    AiError::Provider(message.to_owned())
}
