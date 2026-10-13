//! Files dropped on the window from another application.
//!
//! The window toolkit reports drops on X11, macOS and Windows. On Wayland it
//! reports none, so there the window's own display connection is listened to
//! for them. Whatever arrives either way is merged here, each file once.
//!
//! The Wayland listener is one thread that never waits on anything but a
//! `poll` that includes its stop signal, and at most one more thread that
//! reads the dropped list under a size budget and a deadline. Neither is
//! waited on by a frame. [`FileDrop::stop`] ends and joins both; the
//! application calls it from `on_exit`, while the window still exists.

use eframe::egui;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

/// What a file drag has done by this frame.
#[derive(Debug, Default, PartialEq)]
pub struct Dropped {
    /// Files are held over the window.
    pub hovering: bool,
    /// Files released over the window since the last frame.
    pub paths: Vec<PathBuf>,
    /// Why a drop, or part of one, was not taken.
    pub problem: Option<&'static str>,
}

impl Dropped {
    /// What the toolkit and the display reported, merged. More files than
    /// are taken at once are refused whole, with the reason.
    fn of<'a>(
        hovering: bool,
        toolkit: impl IntoIterator<Item = &'a Path>,
        display: &'a [PathBuf],
        problem: Option<&'static str>,
    ) -> Self {
        match merge(toolkit, display) {
            Ok(paths) => Self {
                hovering,
                paths,
                problem,
            },
            Err(TooMany) => Self {
                hovering,
                paths: Vec::new(),
                problem: Some(TOO_MANY),
            },
        }
    }
}

#[derive(Default)]
pub struct FileDrop {
    #[cfg(target_os = "linux")]
    wayland: Option<wayland::Listener>,
    /// The window was asked about once; a listener is not started twice.
    #[cfg(target_os = "linux")]
    attached: bool,
}

impl FileDrop {
    /// Starts listening on the window's display, once the window exists. Only
    /// a Wayland window needs it.
    pub fn attach(&mut self, frame: &eframe::Frame, ctx: &egui::Context) {
        #[cfg(target_os = "linux")]
        if !self.attached {
            use eframe::wgpu::rwh::{
                HasDisplayHandle as _, HasWindowHandle as _, RawDisplayHandle, RawWindowHandle,
            };
            let (Ok(display), Ok(window)) = (frame.display_handle(), frame.window_handle()) else {
                return;
            };
            self.attached = true;
            if let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) =
                (display.as_raw(), window.as_raw())
            {
                let ctx = ctx.clone();
                // SAFETY: both pointers are the live window's, and this is
                // its thread. `stop` joins the listener from `on_exit`, which
                // runs before the window and its connection are closed.
                self.wayland = unsafe {
                    wayland::Listener::start(
                        display.display.as_ptr(),
                        window.surface.as_ptr(),
                        move || ctx.request_repaint(),
                    )
                };
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (frame, ctx);
    }

    /// What the toolkit and the display reported since the last frame. The
    /// display's listener belongs to the root window's surface.
    pub fn poll(&mut self, ctx: &egui::Context) -> Dropped {
        #[allow(unused_mut)]
        let (mut hovering, mut display, mut problem) = (false, Vec::<PathBuf>::new(), None);
        #[cfg(target_os = "linux")]
        if ctx.viewport_id() == egui::ViewportId::ROOT
            && let Some(listener) = &self.wayland
        {
            let taken = listener.take();
            hovering = taken.hovering;
            problem = taken.problem.map(Refusal::text);
            display = taken.paths;
        }
        // The toolkit's list is read where it is: only a file that is taken
        // is copied.
        ctx.input(|input| {
            Dropped::of(
                hovering || !input.raw.hovered_files.is_empty(),
                input.raw.dropped_files.iter().map(|file| file.path()),
                &display,
                problem,
            )
        })
    }

    /// Ends the listener and waits until its threads are gone. Nothing of
    /// the window's display is touched after this returns.
    pub fn stop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(listener) = self.wayland.take() {
            listener.stop();
        }
    }
}

/// The most files taken from one drop, or kept for a frame yet to be drawn.
const MAX_PATHS: usize = 4096;
const TOO_MANY: &str = "Too many files were dropped at once. Import their folder instead.";

/// More than [`MAX_PATHS`] files were named.
#[derive(Debug, PartialEq)]
struct TooMany;

/// Every named file once, in the order it was first reported: the toolkit's,
/// then the display's. A file named again is not one more file. Naming more
/// than [`MAX_PATHS`] is refused where it happens, without reading on.
fn merge<'a>(
    toolkit: impl IntoIterator<Item = &'a Path>,
    display: &'a [PathBuf],
) -> Result<Vec<PathBuf>, TooMany> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for path in toolkit
        .into_iter()
        .chain(display.iter().map(PathBuf::as_path))
    {
        if path.as_os_str().is_empty() || seen.contains(path) {
            continue;
        }
        if paths.len() == MAX_PATHS {
            return Err(TooMany);
        }
        seen.insert(path);
        paths.push(path.to_path_buf());
    }
    Ok(paths)
}

/// Why a drop, or part of one, was not taken.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    /// More than the budget for one drop.
    TooMuch,
    /// The other application did not hand the list over in time.
    Late,
    /// Nothing in the drop is a file on this computer.
    NotLocal,
    /// Some of it is not.
    PartlyNotLocal,
    Failed,
}

#[cfg(target_os = "linux")]
impl Refusal {
    fn text(self) -> &'static str {
        match self {
            Self::TooMuch => TOO_MANY,
            Self::Late => "The dropped files were not handed over in time.",
            Self::NotLocal => "Only files on this computer can be dropped here.",
            Self::PartlyNotLocal => "Some dropped items are not files on this computer.",
            Self::Failed => "The dropped files could not be read.",
        }
    }
}

/// The longest list of dropped files that is read.
#[cfg(target_os = "linux")]
const MAX_TRANSFER: usize = 1024 * 1024;
/// `PATH_MAX`, which counts the terminating NUL.
#[cfg(target_os = "linux")]
const MAX_PATH_BYTES: usize = 4095;

#[cfg(target_os = "linux")]
#[derive(Debug, Default, PartialEq)]
struct UriList {
    paths: Vec<PathBuf>,
    /// Entries that do not name a file on this computer.
    refused: usize,
}

/// The local files named by a `text/uri-list` transfer, each once. `host` is
/// this computer's name, which a `file` URI may carry. A list that names more
/// than [`MAX_PATHS`] files is refused where it does, without reading on.
#[cfg(target_os = "linux")]
fn uri_list(list: &[u8], host: &[u8]) -> Result<UriList, Refusal> {
    let mut found = UriList::default();
    let mut seen = HashSet::new();
    for line in list.split(|byte| matches!(byte, b'\r' | b'\n')) {
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        match local_path(line, host) {
            Some(path) if seen.contains(&path) => {}
            Some(_) if found.paths.len() == MAX_PATHS => return Err(Refusal::TooMuch),
            Some(path) => {
                seen.insert(path.clone());
                found.paths.push(path);
            }
            None => found.refused += 1,
        }
    }
    Ok(found)
}

/// The absolute path a `file` URI names on this computer. Its escapes are
/// bytes, not text: a file name need not be UTF-8.
#[cfg(target_os = "linux")]
fn local_path(uri: &[u8], host: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;
    let (scheme, rest) = uri.split_at_checked(5)?;
    if !scheme.eq_ignore_ascii_case(b"file:") {
        return None;
    }
    let path = match rest.strip_prefix(b"//") {
        Some(rest) => {
            let slash = rest.iter().position(|byte| *byte == b'/')?;
            let (authority, path) = rest.split_at(slash);
            let here = authority.is_empty()
                || authority.eq_ignore_ascii_case(b"localhost")
                || (!host.is_empty() && authority.eq_ignore_ascii_case(host));
            if !here {
                return None;
            }
            path
        }
        // `file:/path` has no authority at all.
        None if rest.first() == Some(&b'/') => rest,
        None => return None,
    };
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.iter();
    while let Some(&byte) = rest.next() {
        let byte = match byte {
            b'%' => {
                let mut digit = || (*rest.next()? as char).to_digit(16);
                (digit()? * 16 + digit()?) as u8
            }
            // Control bytes are always escaped in a URI, and so are the
            // marks that would begin a query or a fragment.
            0..=0x1f | 0x7f | b'?' | b'#' => return None,
            byte => byte,
        };
        if byte == 0 || bytes.len() == MAX_PATH_BYTES {
            return None;
        }
        bytes.push(byte);
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

#[cfg(target_os = "linux")]
mod wayland {
    use super::{MAX_PATHS, MAX_TRANSFER, PathBuf, Refusal, uri_list};
    use rustix::{
        event::{EventfdFlags, PollFd, PollFlags, Timespec, eventfd, poll},
        fs::{OFlags, fcntl_setfl},
        io::Errno,
    };
    use std::{
        ffi::c_void,
        os::fd::{AsFd, BorrowedFd, OwnedFd},
        sync::{Arc, Mutex, mpsc},
        thread::JoinHandle,
        time::{Duration, Instant},
    };
    use wayland_backend::client::{Backend, ObjectId, WaylandError};
    use wayland_client::{
        Connection, Dispatch, Proxy as _, QueueHandle, WEnum, delegate_noop, event_created_child,
        protocol::{
            wl_data_device::{self, WlDataDevice},
            wl_data_device_manager::{DndAction, WlDataDeviceManager},
            wl_data_offer::{self, WlDataOffer},
            wl_registry::{self, WlRegistry},
            wl_seat::WlSeat,
            wl_surface::WlSurface,
        },
    };

    const URI_LIST: &str = "text/uri-list";
    /// How long the other application has to hand its list over.
    const TRANSFER_DEADLINE: Duration = Duration::from_secs(5);
    /// Offers the compositor introduced that nothing has claimed yet. A drag
    /// and a clipboard selection are two.
    const MAX_OFFERS: usize = 8;

    /// What the listener has for the next frame.
    #[derive(Debug, Default, PartialEq)]
    pub struct Inbox {
        pub hovering: bool,
        pub paths: Vec<PathBuf>,
        pub problem: Option<Refusal>,
    }

    impl Inbox {
        /// Adds a finished drop. What a window that is not drawing has yet
        /// to take stays bounded.
        fn deliver(&mut self, result: Result<Vec<PathBuf>, Refusal>, refused: usize) {
            match result {
                Ok(paths) if self.paths.len() + paths.len() > MAX_PATHS => {
                    self.problem = Some(Refusal::TooMuch);
                }
                Ok(paths) => {
                    if refused > 0 {
                        self.problem = Some(if paths.is_empty() {
                            Refusal::NotLocal
                        } else {
                            Refusal::PartlyNotLocal
                        });
                    }
                    self.paths.extend(paths);
                }
                Err(refusal) => self.problem = Some(refusal),
            }
        }
    }

    /// A signal that stays raised once sent, for any number of `poll`s.
    fn signal() -> Option<Arc<OwnedFd>> {
        eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)
            .ok()
            .map(Arc::new)
    }

    fn raise(signal: &OwnedFd) {
        // A counter that is already raised cannot be raised further; either
        // way it reads as ready.
        let _ = rustix::io::write(signal, &1u64.to_ne_bytes());
    }

    pub struct Listener {
        inbox: Arc<Mutex<Inbox>>,
        stop: Arc<OwnedFd>,
        thread: Option<JoinHandle<()>>,
    }

    impl Listener {
        /// Listens for drops on `surface` through a queue of its own on the
        /// window's connection.
        ///
        /// # Safety
        ///
        /// `display` is a live `wl_display` and `surface` a live `wl_surface`
        /// of it. The display stays connected until [`Listener::stop`] has
        /// returned: the listener's thread owns every handle made from it and
        /// drops them before it ends, and `stop` joins that thread. The
        /// surface is only read here, before this returns; afterwards its
        /// identity is compared, never followed.
        pub unsafe fn start(
            display: *mut c_void,
            surface: *mut c_void,
            wake: impl Fn() + Send + 'static,
        ) -> Option<Self> {
            // SAFETY: the caller's guarantee. The foreign backend adds an
            // event queue to the display and never closes the display.
            let (backend, surface) = unsafe {
                (
                    Backend::from_foreign_display(display.cast()),
                    ObjectId::from_ptr(WlSurface::interface(), surface.cast()).ok()?,
                )
            };
            let connection = Connection::from_backend(backend);
            let inbox = Arc::new(Mutex::new(Inbox::default()));
            let (stop, done) = (signal()?, signal()?);
            let thread = std::thread::Builder::new()
                .name("aster-file-drop".into())
                .spawn({
                    let (inbox, stop) = (inbox.clone(), stop.clone());
                    // Had the thread not started, this closure and the
                    // connection in it are dropped here, on the window's thread.
                    move || run(connection, surface, inbox, stop, done, Box::new(wake))
                });
            Some(Self {
                inbox,
                stop,
                thread: Some(thread.ok()?),
            })
        }

        /// What arrived since the last call. The hover stays until it ends.
        pub fn take(&self) -> Inbox {
            let mut inbox = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
            Inbox {
                hovering: inbox.hovering,
                paths: std::mem::take(&mut inbox.paths),
                problem: inbox.problem.take(),
            }
        }

        /// Ends the listener. Its thread waits only in a `poll` that this
        /// signal ends, and its reader likewise, so the join is prompt.
        pub fn stop(mut self) {
            self.end();
        }

        fn end(&mut self) {
            raise(&self.stop);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// A listener that was never stopped is ended with what owns it, so its
    /// thread does not outlive the application either way.
    impl Drop for Listener {
        fn drop(&mut self) {
            self.end();
        }
    }

    /// A drop whose list is being read.
    struct Transfer {
        offer: WlDataOffer,
        /// The other application agreed to a copy, so the offer may be
        /// finished.
        agreed: bool,
        result: mpsc::Receiver<(Result<Vec<PathBuf>, Refusal>, usize)>,
        reader: JoinHandle<()>,
    }

    struct Offer {
        offer: WlDataOffer,
        /// It carries a list of files.
        files: bool,
        /// The compositor reported an action both sides allow.
        agreed: bool,
    }

    struct State {
        inbox: Arc<Mutex<Inbox>>,
        wake: Box<dyn Fn() + Send>,
        surface: ObjectId,
        stop: Arc<OwnedFd>,
        /// Raised by the reader when its result is ready.
        done: Arc<OwnedFd>,
        /// This computer's name.
        host: Arc<[u8]>,
        /// The seat and the name of its global.
        seat: Option<(u32, WlSeat)>,
        manager: Option<WlDataDeviceManager>,
        device: Option<WlDataDevice>,
        offers: Vec<Offer>,
        /// The offer held over the window, and whether it was accepted.
        drag: Option<(WlDataOffer, bool)>,
        transfer: Option<Transfer>,
    }

    impl State {
        fn publish(&self, change: impl FnOnce(&mut Inbox)) {
            change(&mut self.inbox.lock().unwrap_or_else(|e| e.into_inner()));
            (self.wake)();
        }

        /// The data device exists once the seat and its manager are known.
        fn connect(&mut self, handle: &QueueHandle<Self>) {
            if self.device.is_none()
                && let (Some((_, seat)), Some(manager)) = (&self.seat, &self.manager)
            {
                self.device = Some(manager.get_data_device(seat, handle, ()));
            }
        }

        /// Gives up the seat's device and every offer made through it.
        fn disconnect(&mut self) {
            if let Some((offer, _)) = self.drag.take() {
                self.release(&offer);
            }
            for known in self.offers.drain(..) {
                // One being read is destroyed when its transfer ends.
                if self
                    .transfer
                    .as_ref()
                    .is_none_or(|t| t.offer != known.offer)
                {
                    known.offer.destroy();
                }
            }
            if let Some(device) = self.device.take()
                && device.version() >= 2
            {
                device.release();
            }
            if let Some((_, seat)) = self.seat.take()
                && seat.version() >= 5
            {
                seat.release();
            }
        }

        fn release(&mut self, offer: &WlDataOffer) {
            self.offers.retain(|known| known.offer != *offer);
            offer.destroy();
        }

        fn known(&mut self, offer: &WlDataOffer) -> Option<&mut Offer> {
            self.offers.iter_mut().find(|known| known.offer == *offer)
        }

        /// Asks for the dropped list and reads it on a thread of its own.
        fn receive(&mut self, offer: WlDataOffer, connection: &Connection) {
            let agreed = self.known(&offer).is_some_and(|known| known.agreed);
            // One list at a time: a second drop this soon is not taken.
            let pipe = match &self.transfer {
                None => std::io::pipe().ok(),
                Some(_) => None,
            };
            let Some((reader, writer)) = pipe else {
                self.release(&offer);
                self.publish(|inbox| inbox.deliver(Err(Refusal::Failed), 0));
                return;
            };
            offer.receive(URI_LIST.to_owned(), writer.as_fd());
            let _ = connection.flush();
            // The other application's end is the only one left open, so its
            // closing is the end of the list.
            drop(writer);
            let (sender, result) = mpsc::sync_channel(1);
            let (stop, done, host) = (self.stop.clone(), self.done.clone(), self.host.clone());
            let spawned = std::thread::Builder::new()
                .name("aster-file-drop-read".into())
                .spawn(move || {
                    let deadline = Instant::now() + TRANSFER_DEADLINE;
                    let found = read_list(&reader, &stop, deadline, MAX_TRANSFER)
                        .and_then(|list| uri_list(&list, &host));
                    let answer = match found {
                        Ok(found) => (Ok(found.paths), found.refused),
                        Err(refusal) => (Err(refusal), 0),
                    };
                    let _ = sender.send(answer);
                    raise(&done);
                });
            match spawned {
                Ok(reader) => {
                    self.transfer = Some(Transfer {
                        offer,
                        agreed,
                        result,
                        reader,
                    });
                }
                Err(_) => {
                    self.release(&offer);
                    self.publish(|inbox| inbox.deliver(Err(Refusal::Failed), 0));
                }
            }
        }

        /// Takes the reader's result when it has one, and lets the offer go.
        fn settle(&mut self) {
            let Some(transfer) = &self.transfer else {
                return;
            };
            let (result, refused) = match transfer.result.try_recv() {
                Ok(answer) => answer,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => (Err(Refusal::Failed), 0),
            };
            let Some(transfer) = self.transfer.take() else {
                return;
            };
            let _ = transfer.reader.join();
            // Finishing an offer no action was agreed for is a protocol
            // error, and that would close the window's connection.
            if result.is_ok() && transfer.agreed && transfer.offer.version() >= 3 {
                transfer.offer.finish();
            }
            self.release(&transfer.offer);
            // A reader that was told to stop has nothing to report.
            if result != Err(Refusal::Failed) || !ready(&self.stop) {
                self.publish(|inbox| inbox.deliver(result, refused));
            }
        }

        /// Ends the reader and gives every object back to the compositor.
        fn close(&mut self) {
            raise(&self.stop);
            if let Some(transfer) = self.transfer.take() {
                let _ = transfer.reader.join();
                self.release(&transfer.offer);
            }
            self.disconnect();
            self.inbox
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .hovering = false;
        }
    }

    fn ready(signal: &OwnedFd) -> bool {
        let mut fds = [PollFd::new(signal, PollFlags::IN)];
        let now = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        poll(&mut fds, Some(&now)).is_ok_and(|ready| ready > 0)
    }

    /// The listener's thread. It owns the connection handle and everything
    /// made from it, and all of that is dropped when this returns.
    fn run(
        connection: Connection,
        surface: ObjectId,
        inbox: Arc<Mutex<Inbox>>,
        stop: Arc<OwnedFd>,
        done: Arc<OwnedFd>,
        wake: Box<dyn Fn() + Send>,
    ) {
        let mut queue = connection.new_event_queue::<State>();
        let handle = queue.handle();
        // The globals arrive as events like any other: nothing waits for a
        // round trip.
        let _registry = connection.display().get_registry(&handle, ());
        let host = std::fs::read("/proc/sys/kernel/hostname").unwrap_or_default();
        let mut state = State {
            inbox,
            wake,
            surface,
            stop: stop.clone(),
            done: done.clone(),
            host: host.trim_ascii().into(),
            seat: None,
            manager: None,
            device: None,
            offers: Vec::new(),
            drag: None,
            transfer: None,
        };
        loop {
            if queue.dispatch_pending(&mut state).is_err() {
                break;
            }
            state.settle();
            match connection.flush() {
                Ok(()) => {}
                // The socket is full; what is left goes out with a later flush.
                Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
            // Refused while events are queued: they are dispatched first.
            let Some(guard) = queue.prepare_read() else {
                continue;
            };
            let (readable, closed, stopped, finished) = {
                let mut fds = [
                    PollFd::from_borrowed_fd(guard.connection_fd(), PollFlags::IN),
                    PollFd::new(&*stop, PollFlags::IN),
                    PollFd::new(&*done, PollFlags::IN),
                ];
                match poll(&mut fds, None) {
                    Ok(_) => {}
                    Err(Errno::INTR) => continue,
                    Err(_) => break,
                }
                let [display, stop, done] = fds.map(|fd| fd.revents());
                (
                    display.contains(PollFlags::IN),
                    display.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL),
                    !stop.is_empty(),
                    !done.is_empty(),
                )
            };
            // Dropping the guard withdraws the intent to read, which is what
            // lets the window's own thread read without waiting for this one.
            if stopped {
                break;
            }
            if readable {
                match guard.read() {
                    Ok(_) => {}
                    Err(WaylandError::Io(error))
                        if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            } else if closed {
                break;
            }
            if finished {
                let _ = rustix::io::read(&*done, &mut [0u8; 8][..]);
            }
        }
        state.close();
        let _ = connection.flush();
    }

    /// Reads a dropped list to its end. Stops at `limit` bytes, at
    /// `deadline`, and when `stop` is raised; it waits nowhere else.
    pub fn read_list(
        pipe: &impl AsFd,
        stop: &OwnedFd,
        deadline: Instant,
        limit: usize,
    ) -> Result<Vec<u8>, Refusal> {
        let pipe: BorrowedFd<'_> = pipe.as_fd();
        fcntl_setfl(pipe, OFlags::NONBLOCK).map_err(|_| Refusal::Failed)?;
        let mut list = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
                .ok_or(Refusal::Late)?;
            let left = Timespec {
                tv_sec: left.as_secs() as _,
                tv_nsec: left.subsec_nanos() as _,
            };
            let mut fds = [
                PollFd::from_borrowed_fd(pipe, PollFlags::IN),
                PollFd::new(stop, PollFlags::IN),
            ];
            match poll(&mut fds, Some(&left)) {
                Ok(_) => {}
                Err(Errno::INTR) => continue,
                Err(_) => return Err(Refusal::Failed),
            }
            if !fds[1].revents().is_empty() {
                return Err(Refusal::Failed);
            }
            if fds[0].revents().is_empty() {
                continue;
            }
            match rustix::io::read(pipe, &mut chunk[..]) {
                Ok(0) => return Ok(list),
                Ok(read) if list.len() + read > limit => return Err(Refusal::TooMuch),
                Ok(read) => list.extend_from_slice(&chunk[..read]),
                Err(Errno::INTR | Errno::AGAIN) => {}
                Err(_) => return Err(Refusal::Failed),
            }
        }
    }

    impl Dispatch<WlRegistry, ()> for State {
        fn event(
            state: &mut Self,
            registry: &WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            handle: &QueueHandle<Self>,
        ) {
            match event {
                wl_registry::Event::Global {
                    name,
                    interface,
                    version,
                } => {
                    // One seat is listened to: the first there is.
                    if interface == WlSeat::interface().name && state.seat.is_none() {
                        state.seat = Some((name, registry.bind(name, version.min(5), handle, ())));
                    } else if interface == WlDataDeviceManager::interface().name
                        && state.manager.is_none()
                    {
                        state.manager = Some(registry.bind(name, version.min(3), handle, ()));
                    }
                    state.connect(handle);
                }
                wl_registry::Event::GlobalRemove { name }
                    if state.seat.as_ref().is_some_and(|(seat, _)| *seat == name) =>
                {
                    state.disconnect();
                    state.publish(|inbox| inbox.hovering = false);
                }
                _ => {}
            }
        }
    }
    delegate_noop!(State: ignore WlSeat);
    delegate_noop!(State: WlDataDeviceManager);

    impl Dispatch<WlDataOffer, ()> for State {
        fn event(
            state: &mut Self,
            offer: &WlDataOffer,
            event: wl_data_offer::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            let Some(known) = state.known(offer) else {
                return;
            };
            match event {
                wl_data_offer::Event::Offer { mime_type } if mime_type == URI_LIST => {
                    known.files = true;
                }
                wl_data_offer::Event::Action { dnd_action } => {
                    known.agreed = matches!(dnd_action, WEnum::Value(action) if !action.is_empty());
                }
                _ => {}
            }
        }
    }

    impl Dispatch<WlDataDevice, ()> for State {
        fn event(
            state: &mut Self,
            _: &WlDataDevice,
            event: wl_data_device::Event,
            _: &(),
            connection: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use wl_data_device::Event;
            match event {
                Event::DataOffer { id } => {
                    // The compositor claims each offer with an enter or a
                    // selection; one left over is the oldest.
                    if state.offers.len() >= MAX_OFFERS {
                        let oldest = state.offers.remove(0).offer;
                        if state.drag.as_ref().is_some_and(|(held, _)| *held == oldest) {
                            state.drag = None;
                        }
                        if state.transfer.as_ref().is_none_or(|t| t.offer != oldest) {
                            oldest.destroy();
                        }
                    }
                    state.offers.push(Offer {
                        offer: id,
                        files: false,
                        agreed: false,
                    });
                }
                // The clipboard is read through the toolkit.
                Event::Selection { id: Some(offer) } => state.release(&offer),
                Event::Enter {
                    serial,
                    surface,
                    id,
                    ..
                } => {
                    if let Some((stale, _)) = state.drag.take() {
                        state.release(&stale);
                    }
                    let Some(offer) = id else {
                        return;
                    };
                    // The window's decorations are surfaces of their own.
                    let accepted = surface.id() == state.surface
                        && state.known(&offer).is_some_and(|known| known.files);
                    offer.accept(serial, accepted.then(|| URI_LIST.to_owned()));
                    if accepted && offer.version() >= 3 {
                        offer.set_actions(DndAction::Copy, DndAction::Copy);
                    }
                    state.drag = Some((offer, accepted));
                    if accepted {
                        state.publish(|inbox| inbox.hovering = true);
                    }
                }
                Event::Leave => {
                    if let Some((offer, _)) = state.drag.take() {
                        state.release(&offer);
                    }
                    state.publish(|inbox| inbox.hovering = false);
                }
                Event::Drop => {
                    let Some((offer, accepted)) = state.drag.take() else {
                        return;
                    };
                    if accepted {
                        state.receive(offer, connection);
                    } else {
                        state.release(&offer);
                    }
                    state.publish(|inbox| inbox.hovering = false);
                }
                _ => {}
            }
        }

        event_created_child!(State, WlDataDevice, [
            wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
        ]);
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Write as _;

        fn soon(millis: u64) -> Instant {
            Instant::now() + Duration::from_millis(millis)
        }

        #[test]
        fn a_list_is_read_to_its_end() {
            let (reader, mut writer) = std::io::pipe().unwrap();
            let stop = signal().unwrap();
            let sent = std::thread::spawn(move || {
                writer.write_all(b"file:///a.mp4\r\n").unwrap();
                std::thread::sleep(Duration::from_millis(30));
                writer.write_all(b"file:///b.mp4\r\n").unwrap();
            });
            let list = read_list(&reader, &stop, soon(5000), 1024).unwrap();
            sent.join().unwrap();
            assert_eq!(list, b"file:///a.mp4\r\nfile:///b.mp4\r\n");
        }

        #[test]
        fn a_list_over_the_budget_is_refused_whole() {
            let (reader, mut writer) = std::io::pipe().unwrap();
            let stop = signal().unwrap();
            writer.write_all(&[b'a'; 1025]).unwrap();
            drop(writer);
            assert_eq!(
                read_list(&reader, &stop, soon(5000), 1024),
                Err(Refusal::TooMuch)
            );
            // Exactly the budget is taken.
            let (reader, mut writer) = std::io::pipe().unwrap();
            writer.write_all(&[b'a'; 1024]).unwrap();
            drop(writer);
            assert_eq!(
                read_list(&reader, &stop, soon(5000), 1024).unwrap().len(),
                1024
            );
        }

        #[test]
        fn a_source_that_never_finishes_is_given_up_at_the_deadline() {
            // The writing end stays open and silent.
            let (reader, mut writer) = std::io::pipe().unwrap();
            let stop = signal().unwrap();
            writer.write_all(b"file:///a").unwrap();
            let began = Instant::now();
            assert_eq!(
                read_list(&reader, &stop, soon(80), 1024),
                Err(Refusal::Late)
            );
            assert!(began.elapsed() < Duration::from_secs(2));
            drop(writer);
        }

        #[test]
        fn stopping_ends_a_read_that_is_waiting() {
            let (reader, writer) = std::io::pipe().unwrap();
            let stop = signal().unwrap();
            let raised = stop.clone();
            let began = Instant::now();
            let stopper = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(40));
                raise(&raised);
            });
            assert_eq!(
                read_list(&reader, &stop, soon(30_000), 1024),
                Err(Refusal::Failed)
            );
            assert!(began.elapsed() < Duration::from_secs(5));
            stopper.join().unwrap();
            // The signal stays raised for whoever looks next.
            assert!(ready(&stop));
            drop(writer);
        }

        #[test]
        fn what_waits_for_a_frame_is_bounded() {
            let file = |n: usize| PathBuf::from(format!("/media/{n}.mp4"));
            let mut inbox = Inbox::default();
            inbox.deliver(Ok((0..MAX_PATHS).map(file).collect()), 0);
            assert_eq!(inbox.paths.len(), MAX_PATHS);
            assert_eq!(inbox.problem, None);
            inbox.deliver(Ok(vec![file(MAX_PATHS)]), 0);
            assert_eq!(inbox.paths.len(), MAX_PATHS);
            assert_eq!(inbox.problem, Some(Refusal::TooMuch));

            let mut inbox = Inbox::default();
            inbox.deliver(Ok(vec![file(1)]), 2);
            assert_eq!(inbox.problem, Some(Refusal::PartlyNotLocal));
            inbox.deliver(Ok(Vec::new()), 1);
            assert_eq!(inbox.problem, Some(Refusal::NotLocal));
            inbox.deliver(Err(Refusal::Late), 0);
            assert_eq!((inbox.paths.len(), inbox.problem), (1, Some(Refusal::Late)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(numbers: std::ops::Range<usize>) -> Vec<PathBuf> {
        numbers
            .map(|n| PathBuf::from(format!("/media/{n}.mp4")))
            .collect()
    }

    fn each(paths: &[PathBuf]) -> impl Iterator<Item = &Path> {
        paths.iter().map(PathBuf::as_path)
    }

    #[test]
    fn a_file_reported_twice_is_dropped_once() {
        let path = |name: &str| PathBuf::from(name);
        let toolkit = [path("/a.mp4"), path(""), path("/b.mp4")];
        let display = [path("/b.mp4"), path("/c.mp4"), path("/a.mp4")];
        assert_eq!(
            merge(each(&toolkit), &display).unwrap(),
            [path("/a.mp4"), path("/b.mp4"), path("/c.mp4")]
        );
    }

    #[test]
    fn files_from_both_sources_keep_the_order_they_were_first_reported() {
        let path = |name: &str| PathBuf::from(name);
        let toolkit = [path("/z.mp4"), path("/a.mp4"), path("/z.mp4")];
        let display = [path("/m.mp4"), path("/a.mp4"), path("/b.mp4"), path("")];
        let dropped = Dropped::of(true, each(&toolkit), &display, Some("partly"));
        assert_eq!(
            dropped,
            Dropped {
                hovering: true,
                paths: vec![
                    path("/z.mp4"),
                    path("/a.mp4"),
                    path("/m.mp4"),
                    path("/b.mp4")
                ],
                // What the display had to say about its own drop is kept.
                problem: Some("partly"),
            }
        );
        assert_eq!(merge(each(&[]), &[]), Ok(Vec::new()));
        assert_eq!(Dropped::of(false, each(&[]), &[], None), Dropped::default());
    }

    #[test]
    fn as_many_files_as_are_taken_may_be_named_by_both_sources() {
        let most = files(0..MAX_PATHS);
        // Each source alone, and the two halves of one drop.
        assert_eq!(merge(each(&most), &[]).unwrap(), most);
        assert_eq!(merge(each(&[]), &most).unwrap(), most);
        let (first, second) = most.split_at(MAX_PATHS / 2);
        assert_eq!(merge(each(first), second).unwrap(), most);

        // The same files from both, in another order and over again, are
        // not one file more.
        let again: Vec<PathBuf> = most.iter().rev().chain(&most).cloned().collect();
        let dropped = Dropped::of(false, each(&most), &again, None);
        assert_eq!(dropped.paths, most);
        assert_eq!(dropped.problem, None);
    }

    #[test]
    fn one_file_too_many_refuses_the_whole_drop_and_says_so() {
        let refused = Dropped {
            hovering: true,
            paths: Vec::new(),
            problem: Some(TOO_MANY),
        };
        let over = files(0..MAX_PATHS + 1);
        // From the toolkit alone, from the display alone, and only once the
        // two are counted together.
        assert_eq!(Dropped::of(true, each(&over), &[], None), refused);
        assert_eq!(Dropped::of(true, each(&[]), &over, None), refused);
        let (first, second) = over.split_at(MAX_PATHS);
        assert_eq!(merge(each(first), &[]).unwrap().len(), MAX_PATHS);
        assert_eq!(merge(each(first), second), Err(TooMany));
        // Nothing is taken, and this reason replaces a lesser one.
        assert_eq!(
            Dropped::of(true, each(first), second, Some("partly")),
            refused
        );
        // A file past the limit is one too many however often the others
        // are named before it.
        let mut repeated = files(0..MAX_PATHS);
        repeated.extend(files(0..MAX_PATHS));
        repeated.extend(files(MAX_PATHS..MAX_PATHS + 1));
        assert_eq!(merge(each(&repeated), &[]), Err(TooMany));
        #[cfg(target_os = "linux")]
        assert_eq!(Refusal::TooMuch.text(), TOO_MANY);
    }

    #[test]
    fn a_long_report_of_few_files_is_merged_in_good_time() {
        // Far more names than are taken, nearly all of them repeats.
        let few = files(0..MAX_PATHS);
        let many: Vec<PathBuf> = few.iter().cycle().take(MAX_PATHS * 16).cloned().collect();
        let began = std::time::Instant::now();
        assert_eq!(merge(each(&many), &many).unwrap(), few);
        assert!(began.elapsed() < std::time::Duration::from_secs(1));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_uri_list_names_local_files_and_decodes_their_bytes() {
        use std::os::unix::ffi::OsStrExt as _;
        let list = b"# from a file manager\r\n\
            file:///home/me/Screenshot%20from%202026.png\r\n\
            file://localhost/tmp/caf%C3%A9.mov\r\n\
            FILE://Studio/tmp/a.mp4\r\n\
            file:/tmp/b%23%3F.wav\r\n\
            file:///tmp/%ff%FEraw.gif\r\n\
            \r\n\
            file:///home/me/Screenshot%20from%202026.png\r\n";
        let found = uri_list(list, b"studio").unwrap();
        assert_eq!(found.refused, 0);
        assert_eq!(
            found.paths[..4],
            [
                PathBuf::from("/home/me/Screenshot from 2026.png"),
                PathBuf::from("/tmp/café.mov"),
                PathBuf::from("/tmp/a.mp4"),
                PathBuf::from("/tmp/b#?.wav"),
            ]
        );
        // Escapes are bytes: a name that is not UTF-8 survives.
        assert_eq!(
            found.paths[4].as_os_str().as_bytes(),
            b"/tmp/\xff\xferaw.gif"
        );
        assert_eq!(found.paths.len(), 5);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn what_is_not_a_file_on_this_computer_is_refused() {
        let refused: &[&[u8]] = &[
            b"https://example.com/a.png",
            b"file://example.com/tmp/a.png",
            b"file://user@localhost/tmp/a.png",
            b"file://localhost:80/tmp/a.png",
            b"file://localhost",
            b"file:relative/a.png",
            b"file:",
            b"/tmp/a.png",
            // Malformed escapes, and a NUL however it is written.
            b"file:///tmp/100%",
            b"file:///tmp/%2",
            b"file:///tmp/%zz.png",
            b"file:///tmp/a%00.png",
            b"file:///tmp/a\0.png",
            b"file:///tmp/a\tb.png",
            // A query or a fragment is not part of a file's name.
            b"file:///tmp/a.mp4#t=10",
            b"file:///tmp/a.mp4?download",
        ];
        for uri in refused {
            assert_eq!(
                local_path(uri, b"studio"),
                None,
                "{}",
                String::from_utf8_lossy(uri)
            );
        }
        // Without a known name for this computer, only the empty host and
        // `localhost` are here.
        assert_eq!(local_path(b"file://studio/tmp/a.png", b""), None);
        assert!(local_path(b"file:///tmp/a.png", b"").is_some());

        let long = [b"file:///".as_slice(), &[b'a'; MAX_PATH_BYTES]].concat();
        assert_eq!(local_path(&long, b""), None);
        assert!(local_path(&long[..long.len() - 1], b"").is_some());

        let found = uri_list(b"file:///a.mp4\nhttps://example.com/b.mp4\n", b"").unwrap();
        assert_eq!((found.paths.len(), found.refused), (1, 1));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_list_of_too_many_files_is_refused_where_it_overflows() {
        let list = |files: usize| -> Vec<u8> {
            (0..files)
                .flat_map(|n| format!("file:///m/{n}\r\n").into_bytes())
                .collect()
        };
        assert_eq!(
            uri_list(&list(MAX_PATHS), b"").unwrap().paths.len(),
            MAX_PATHS
        );
        assert_eq!(uri_list(&list(MAX_PATHS + 1), b""), Err(Refusal::TooMuch));
        // A file named again is not one more file.
        let mut repeated = list(MAX_PATHS);
        repeated.extend_from_slice(b"file:///m/0\r\n");
        assert!(uri_list(&repeated, b"").is_ok());

        // The largest list that is read at all, of the shortest entries
        // there are, is parsed in good time.
        let mut most = Vec::with_capacity(MAX_TRANSFER);
        let mut n = 0u32;
        while most.len() + 16 < MAX_TRANSFER {
            most.extend_from_slice(format!("file:///{n:x}\n").as_bytes());
            n += 1;
        }
        let began = std::time::Instant::now();
        assert_eq!(uri_list(&most, b""), Err(Refusal::TooMuch));
        assert!(began.elapsed() < std::time::Duration::from_secs(1));
    }
}
