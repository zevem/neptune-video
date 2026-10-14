//! The export sheet's poster: the output frame at the playhead, as the
//! export will make it.
//!
//! It is drawn by the export's own [`FrameRenderer`] from the original
//! media, never a proxy, at the output's shape scaled down to fit
//! [`POSTER_SIDE`]: the same fit, bars and crop as the file. The drawing
//! happens on the `aster-poster` thread, which owns its renderer and its
//! [`SourceReader`]; the interface only asks and uploads what comes back.
//!
//! A request is named by a [`PosterKey`]: the editing context, the project
//! revision, the poster's geometry, what of the settings changes the
//! picture, and the output frame's time. Only the latest request waits, an
//! older one not yet taken is dropped, and a picture is shown only when it
//! answers the latest request sent: one that comes back after a newer one
//! was sent is stale and thrown away. Requests are sent once what they ask
//! for has stayed the same for [`SETTLE`], so typing a size or dragging
//! through choices does not draw every step. Nothing repaints while
//! nothing is waited for, and the thread stops when the sheet closes.
use super::frame::{FrameRenderer, Pictures, headless_device};
use editor_core::{
    Asset, ClipId, FrameRate, Project, Time,
    export::{ExportSettings, FrameSpan, Geometry},
};
use editor_media::{Frame, export::SourceReader};
use eframe::{egui, wgpu};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// The longest side the poster is drawn at, in pixels.
pub const POSTER_SIDE: u32 = 640;
/// How long a request stays the same before it is sent.
pub const SETTLE: Duration = Duration::from_millis(150);
/// How long the device may take to hand one poster back.
const READ_DEADLINE: Duration = Duration::from_secs(10);

/// What a poster is of. Two requests with the same key draw the same
/// picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PosterKey {
    /// The editing context (`Editor::instance_id`).
    pub context: Uuid,
    pub revision: u64,
    /// The output's geometry, scaled down to the poster.
    pub geometry: Geometry,
    /// The rate the render project runs at.
    pub rate: FrameRate,
    /// Captions burnt into the picture.
    pub burn_in: bool,
    /// The timeline time of the output frame shown.
    pub at: Time,
}

/// The poster the export of `project` with `settings` would show at
/// `playhead`: the output frame the playhead is in, or the range's first or
/// last frame when it is outside the range, at the output's shape fitting
/// [`POSTER_SIDE`]. `None` without a picture to show: no video, or no
/// frame in the range.
pub fn poster_key(
    context: Uuid,
    revision: u64,
    project: &Project,
    settings: &ExportSettings,
    playhead: Time,
) -> Option<PosterKey> {
    let video = settings.video.as_ref()?;
    let canvas = [project.settings.width, project.settings.height];
    if canvas.contains(&0) {
        return None;
    }
    let output = video.resolution.size(canvas);
    let longest = output[0].max(output[1]).max(1);
    let scale = (f64::from(POSTER_SIDE) / f64::from(longest)).min(1.0);
    let small = output.map(|side| ((f64::from(side) * scale).round() as u32).max(1));
    let geometry = Geometry::new(canvas, small, video.fit);
    let (start, end) = settings.span(project).ok()?;
    let rate = video.frame_rate.unwrap_or(project.settings.frame_rate);
    let frames = FrameSpan::covering(start, end, rate);
    if frames.count == 0 {
        return None;
    }
    let index =
        (rate.frame_at(playhead.max(Time::ZERO)) - frames.first).clamp(0, frames.count as i64 - 1);
    Some(PosterKey {
        context,
        revision,
        geometry,
        rate,
        burn_in: settings.subtitles.burn_in,
        at: frames.time(index as u64),
    })
}

/// Where the poster's pictures are decoded: the original media in the
/// application, memory in tests. Made on the poster's thread, and kept
/// there.
pub trait PosterSource {
    /// As [`Pictures::picture`].
    fn picture(
        &mut self,
        lane: ClipId,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<Frame, String>;
    /// Lets go of what lanes no longer `live` hold open.
    fn retain(&mut self, live: &dyn Fn(ClipId) -> bool);
    /// Stops the source from any thread.
    fn canceller(&self) -> Box<dyn Fn() + Send + Sync>;
}

impl PosterSource for SourceReader {
    fn picture(
        &mut self,
        lane: ClipId,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<Frame, String> {
        SourceReader::picture(self, lane, asset, time, size)
            .map_err(|error| format!("{} could not be read: {error}", asset.name))
    }

    fn retain(&mut self, live: &dyn Fn(ClipId) -> bool) {
        SourceReader::retain(self, live);
    }

    fn canceller(&self) -> Box<dyn Fn() + Send + Sync> {
        Box::new(SourceReader::canceller(self))
    }
}

struct Sources<'a>(&'a mut dyn PosterSource);

impl Pictures for Sources<'_> {
    fn picture(
        &mut self,
        lane: ClipId,
        asset: &Asset,
        time: Time,
        size: [u32; 2],
    ) -> Result<Frame, String> {
        self.0.picture(lane, asset, time, size)
    }
}

/// What the poster is drawn of.
pub struct PosterRequest {
    pub key: PosterKey,
    pub project: Arc<Project>,
    pub settings: ExportSettings,
}

/// A poster drawn: opaque RGBA rows of `size`.
#[derive(Debug)]
pub struct Picture {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}

/// What the thread answers a request with.
#[derive(Debug)]
pub struct Published {
    pub key: PosterKey,
    pub picture: Result<Picture, String>,
}

#[derive(Default)]
struct Inbox {
    /// The latest request not yet taken.
    asked: Option<PosterRequest>,
    /// The latest answer not yet read.
    done: Option<Published>,
    stopped: bool,
}

struct Shared {
    inbox: Mutex<Inbox>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inbox> {
        self.inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Stops a source from another thread.
type Canceller = Box<dyn Fn() + Send + Sync>;

/// Makes the source a poster thread decodes with, on that thread.
pub type SourceMaker = Arc<dyn Fn() -> Box<dyn PosterSource> + Send + Sync>;

/// The `aster-poster` thread: a renderer and a source of its own.
pub struct PosterWorker {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    /// What stops the thread's source, once it has made it.
    cancel: Arc<Mutex<Option<Canceller>>>,
    thread: Option<JoinHandle<()>>,
}

impl PosterWorker {
    /// Starts the thread, drawing on `gpu` when given (the window's
    /// device), else on a device of its own opened when first needed.
    /// `steady` is where camera paths are kept. `repaint` is called when a
    /// poster is ready.
    pub fn start(
        gpu: Option<(wgpu::Device, wgpu::Queue)>,
        steady: Option<PathBuf>,
        source: SourceMaker,
        repaint: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            inbox: Mutex::new(Inbox::default()),
            wake: Condvar::new(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let cancel: Arc<Mutex<Option<Canceller>>> = Arc::default();
        let thread = thread::Builder::new()
            .name("aster-poster".to_owned())
            .spawn({
                let shared = Arc::clone(&shared);
                let stop = Arc::clone(&stop);
                let cancel = Arc::clone(&cancel);
                move || {
                    let source = source();
                    {
                        let mut slot = cancel.lock().unwrap_or_else(|p| p.into_inner());
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        *slot = Some(source.canceller());
                    }
                    let mut drawer = Drawer {
                        gpu,
                        device_failed: None,
                        steady,
                        source,
                        stop,
                        renderer: None,
                    };
                    drawer.run(&shared, &repaint);
                }
            })
            .map_err(|error| format!("The poster could not be drawn: {error}"))?;
        Ok(Self {
            shared,
            stop,
            cancel,
            thread: Some(thread),
        })
    }

    /// Asks for a poster, in the place of one asked for and not yet taken.
    pub fn ask(&self, request: PosterRequest) {
        self.shared.lock().asked = Some(request);
        self.shared.wake.notify_all();
    }

    /// The latest poster drawn, once.
    pub fn take(&self) -> Option<Published> {
        self.shared.lock().done.take()
    }

    /// Stops the thread without waiting for it: its decoders are killed
    /// and a frame it waits on gives up.
    pub fn stop(mut self) {
        self.halt();
        self.thread.take();
    }

    fn halt(&self) {
        self.shared.lock().stopped = true;
        {
            // Under the lock the thread takes to hand its canceller over:
            // a source made after this sees the stop and is not used.
            let cancel = self.cancel.lock().unwrap_or_else(|p| p.into_inner());
            self.stop.store(true, Ordering::Relaxed);
            if let Some(cancel) = cancel.as_ref() {
                cancel();
            }
        }
        self.shared.wake.notify_all();
    }

    /// Stops the thread and waits for it to end.
    #[cfg(test)]
    pub fn join(mut self) {
        self.halt();
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the poster thread ends cleanly");
        }
    }
}

impl Drop for PosterWorker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.halt();
        }
    }
}

/// What the thread owns.
struct Drawer {
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    device_failed: Option<String>,
    steady: Option<PathBuf>,
    source: Box<dyn PosterSource>,
    stop: Arc<AtomicBool>,
    /// The renderer, with the geometry and canvas it draws.
    renderer: Option<(Geometry, [u32; 2], FrameRenderer)>,
}

impl Drawer {
    fn run(&mut self, shared: &Shared, repaint: &dyn Fn()) {
        loop {
            let request = {
                let mut inbox = shared.lock();
                loop {
                    if inbox.stopped {
                        return;
                    }
                    if let Some(request) = inbox.asked.take() {
                        break request;
                    }
                    inbox = shared
                        .wake
                        .wait(inbox)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            };
            let picture = self.draw(&request);
            {
                let mut inbox = shared.lock();
                if inbox.stopped {
                    return;
                }
                inbox.done = Some(Published {
                    key: request.key,
                    picture,
                });
            }
            repaint();
        }
    }

    fn device(&mut self) -> Result<(wgpu::Device, wgpu::Queue), String> {
        if let Some(gpu) = &self.gpu {
            return Ok(gpu.clone());
        }
        if let Some(why) = &self.device_failed {
            return Err(why.clone());
        }
        match headless_device() {
            Ok(made) => {
                self.gpu = Some((made.device, made.queue));
                self.device()
            }
            Err(why) => {
                self.device_failed = Some(why.clone());
                Err(why)
            }
        }
    }

    fn draw(&mut self, request: &PosterRequest) -> Result<Picture, String> {
        let key = request.key;
        let render = request.settings.render_project(&request.project);
        let canvas = [render.settings.width, render.settings.height];
        let fresh = !matches!(
            &self.renderer,
            Some((geometry, made, _)) if *geometry == key.geometry && *made == canvas
        );
        if fresh {
            self.renderer = None;
            let (device, queue) = self.device()?;
            let mut renderer =
                FrameRenderer::new(device, queue, key.geometry, &render, self.steady.clone())?;
            renderer.set_stop(Arc::clone(&self.stop));
            renderer.set_deadline(READ_DEADLINE);
            self.renderer = Some((key.geometry, canvas, renderer));
        }
        let Some((_, _, renderer)) = self.renderer.as_mut() else {
            return Err("The poster has no renderer".to_owned());
        };
        let drawn = renderer
            .render(&render, key.at, &mut Sources(self.source.as_mut()))
            .map(<[u8]>::to_vec);
        // Pipes of clips not at this time close.
        let live = live_lanes(&render, key.at);
        self.source.retain(&|lane| live.contains(&lane));
        Ok(Picture {
            size: key.geometry.output,
            rgba: drawn?,
        })
    }
}

/// The clips drawn at `at`, with what their transitions hold.
fn live_lanes(project: &Project, at: Time) -> HashSet<ClipId> {
    let mut live = HashSet::new();
    for track in &project.tracks {
        for clip in &track.clips {
            let before = (clip.transition_in.as_ref()).map_or(Time::ZERO, |t| t.duration);
            let after = (clip.transition_out.as_ref()).map_or(Time::ZERO, |t| t.duration);
            if clip.start - before <= at && clip.end() + after >= at {
                live.insert(clip.id);
            }
        }
    }
    live
}

/// When a poster wanted is asked for.
#[derive(Debug, PartialEq)]
enum Due {
    /// Already asked for.
    Sent,
    Now,
    /// After this long, if it is still the one wanted.
    In(Duration),
}

/// The interface's side of the poster: what is asked for, when, and the
/// texture shown.
pub struct PosterService {
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    steady: Option<PathBuf>,
    source: SourceMaker,
    worker: Option<PosterWorker>,
    /// The thread could not be started: it is not tried again until the
    /// sheet is closed.
    unstarted: bool,
    /// The poster wanted and since when it has been.
    wanted: Option<(PosterKey, Instant)>,
    /// The latest request sent.
    sent: Option<PosterKey>,
    shown: Option<(PosterKey, egui::TextureHandle)>,
    /// Why the latest poster could not be drawn.
    problem: Option<String>,
}

impl PosterService {
    /// Posters drawn on `gpu` when given (the window's device), from the
    /// original media, with camera paths kept in `steady`.
    pub fn new(gpu: Option<(wgpu::Device, wgpu::Queue)>, steady: Option<PathBuf>) -> Self {
        Self::with_source(gpu, steady, Arc::new(|| Box::new(SourceReader::new())))
    }

    fn with_source(
        gpu: Option<(wgpu::Device, wgpu::Queue)>,
        steady: Option<PathBuf>,
        source: SourceMaker,
    ) -> Self {
        Self {
            gpu,
            steady,
            source,
            worker: None,
            unstarted: false,
            wanted: None,
            sent: None,
            shown: None,
            problem: None,
        }
    }

    /// The poster of the export of `project` with `settings` at `playhead`
    /// to show this frame, when there is one: asks for it once it has been
    /// wanted for [`SETTLE`], and reads what the thread drew.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        project: &Arc<Project>,
        (context, revision): (Uuid, u64),
        settings: &ExportSettings,
        playhead: Time,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        let key = poster_key(context, revision, project, settings, playhead);
        self.show_at(ctx, key, project, settings, Instant::now())
    }

    fn show_at(
        &mut self,
        ctx: &egui::Context,
        key: Option<PosterKey>,
        project: &Arc<Project>,
        settings: &ExportSettings,
        now: Instant,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        if let Some(published) = self.worker.as_ref().and_then(PosterWorker::take) {
            self.receive(ctx, published);
        }
        let Some(key) = key else {
            self.wanted = None;
            return None;
        };
        match self.settle(key, now) {
            Due::Sent => {}
            Due::In(wait) => ctx.request_repaint_after(wait),
            Due::Now => {
                if let Some(worker) = self.worker(ctx) {
                    worker.ask(PosterRequest {
                        key,
                        project: Arc::clone(project),
                        settings: settings.clone(),
                    });
                    self.sent = Some(key);
                }
            }
        }
        // A poster of another shape is not stretched into this one.
        (self.shown.as_ref())
            .filter(|(shown, _)| shown.geometry.output == key.geometry.output)
            .map(|(_, texture)| (texture.id(), texture.size_vec2()))
    }

    /// Whether `key`, wanted at `now`, is to be asked for: the first poster
    /// at once, a changed one once it has stayed the same for [`SETTLE`].
    fn settle(&mut self, key: PosterKey, now: Instant) -> Due {
        if self.wanted.is_none_or(|(wanted, _)| wanted != key) {
            let first = self.sent.is_none() && self.shown.is_none();
            let since = if first {
                now.checked_sub(SETTLE).unwrap_or(now)
            } else {
                now
            };
            self.wanted = Some((key, since));
        }
        if self.sent == Some(key) {
            return Due::Sent;
        }
        let waited = (self.wanted).map_or(Duration::ZERO, |(_, since)| {
            now.saturating_duration_since(since)
        });
        if waited >= SETTLE {
            Due::Now
        } else {
            Due::In(SETTLE - waited)
        }
    }

    fn worker(&mut self, ctx: &egui::Context) -> Option<&PosterWorker> {
        if self.worker.is_none() && !self.unstarted {
            let ctx = ctx.clone();
            match PosterWorker::start(
                self.gpu.clone(),
                self.steady.clone(),
                Arc::clone(&self.source),
                move || ctx.request_repaint(),
            ) {
                Ok(worker) => self.worker = Some(worker),
                Err(why) => {
                    self.unstarted = true;
                    self.problem = Some(why);
                }
            }
        }
        self.worker.as_ref()
    }

    /// Takes a poster drawn: shown when it answers the latest request sent,
    /// thrown away when a newer one was sent since.
    fn receive(&mut self, ctx: &egui::Context, published: Published) {
        if self.sent != Some(published.key) {
            return;
        }
        match published.picture {
            Ok(picture) => {
                let [width, height] = picture.size.map(|side| side as usize);
                let image =
                    egui::ColorImage::from_rgba_unmultiplied([width, height], &picture.rgba);
                let texture = match self.shown.take() {
                    Some((_, mut texture)) => {
                        texture.set(image, egui::TextureOptions::LINEAR);
                        texture
                    }
                    None => ctx.load_texture("export-poster", image, egui::TextureOptions::LINEAR),
                };
                self.shown = Some((published.key, texture));
                self.problem = None;
            }
            Err(why) => self.problem = Some(why),
        }
    }

    /// Why the latest poster could not be drawn, if it could not.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Stops the thread and lets the poster go: the sheet is closed, or the
    /// window is.
    pub fn stop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.stop();
        }
        self.unstarted = false;
        self.wanted = None;
        self.sent = None;
        self.shown = None;
        self.problem = None;
    }

    /// Whether the thread is running.
    #[cfg(test)]
    pub fn running(&self) -> bool {
        self.worker.is_some()
    }
}

#[cfg(test)]
#[path = "poster_tests.rs"]
mod poster_tests;
