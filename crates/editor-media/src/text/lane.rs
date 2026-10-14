//! The title lane: one thread draws the newest request of each title in
//! turn, so typing into a title or playing an animation never waits on a
//! drawing and never queues up stale ones.
use super::{FontBook, TitlePicture, draw_title};
use crate::{MediaError, Wake, io};
use editor_core::{ClipId, LayerSource};
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};

/// Titles with a drawing waiting; past it the oldest is let go.
pub const TEXT_QUEUE: usize = 64;
/// Bytes of drawn pictures waiting to be taken; the lane draws no more
/// until the caller takes some.
pub const TITLE_BUDGET: usize = 64 * 1024 * 1024;

/// One title at one moment and scale.
#[derive(Clone, Debug)]
pub struct TitleRequest {
    pub clip: ClipId,
    /// Rises with each request the caller makes: of two answers for a
    /// title, the one with the higher serial is the newer.
    pub serial: u64,
    pub source: LayerSource,
    /// Picture pixels per canvas pixel; see [`super::quantize_scale`].
    pub scale: f32,
    /// The project's font files.
    pub fonts: Arc<[PathBuf]>,
}

#[derive(Clone, Debug)]
pub struct TitleDrawn {
    pub clip: ClipId,
    pub serial: u64,
    /// The request's own source and scale, to tell it from a newer one.
    pub source: LayerSource,
    pub scale: f32,
    pub picture: Arc<TitlePicture>,
}

#[derive(Clone, Debug)]
pub enum TextEvent {
    Drawn(Box<TitleDrawn>),
    /// Every family a title may name, sorted.
    Families(Arc<[String]>),
}

#[derive(Default)]
struct Lane {
    waiting: VecDeque<TitleRequest>,
    events: VecDeque<TextEvent>,
    held: usize,
    families: bool,
    stopped: bool,
}

#[derive(Default)]
struct Shared {
    lane: Mutex<Lane>,
    ready: Condvar,
}

/// Nonblocking caller interface of the title lane.
pub struct TextService {
    shared: Arc<Shared>,
}

impl TextService {
    /// A lane drawing with `bundled` font files besides the project's and
    /// the system's.
    pub fn new(
        bundled: Vec<&'static [u8]>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, MediaError> {
        Self::with_book(move || FontBook::new(&bundled), Arc::new(wake))
    }

    /// A lane with its own font book, made on its thread.
    pub fn with_book(
        book: impl Fn() -> FontBook + Send + 'static,
        wake: Wake,
    ) -> Result<Self, MediaError> {
        let shared = Arc::new(Shared::default());
        let state = Arc::clone(&shared);
        thread::Builder::new()
            .name("aster-titles".into())
            .spawn(move || worker(state, book, wake))
            .map_err(|error| io("Cannot start the title worker", error))?;
        Ok(Self { shared })
    }

    /// Asks for a drawing. One that waits for the same title is replaced in
    /// its place in line.
    pub fn request(&self, request: TitleRequest) {
        let Ok(mut lane) = self.shared.lane.lock() else {
            return;
        };
        if let Some(waiting) =
            (lane.waiting.iter_mut()).find(|waiting| waiting.clip == request.clip)
        {
            *waiting = request;
        } else {
            if lane.waiting.len() >= TEXT_QUEUE {
                lane.waiting.pop_front();
            }
            lane.waiting.push_back(request);
        }
        drop(lane);
        self.shared.ready.notify_all();
    }

    /// Asks for the families titles may name; they come as an event.
    pub fn want_families(&self) {
        if let Ok(mut lane) = self.shared.lane.lock() {
            lane.families = true;
        }
        self.shared.ready.notify_all();
    }

    pub fn poll(&self) -> Option<TextEvent> {
        let mut lane = self.shared.lane.lock().ok()?;
        let event = lane.events.pop_front()?;
        if let TextEvent::Drawn(drawn) = &event {
            lane.held = lane.held.saturating_sub(drawn.picture.byte_len());
        }
        drop(lane);
        self.shared.ready.notify_all();
        Some(event)
    }

    /// Whether a drawing waits or is under way.
    pub fn busy(&self) -> bool {
        (self.shared.lane.lock()).is_ok_and(|lane| !lane.waiting.is_empty() || lane.families)
    }
}

impl Drop for TextService {
    fn drop(&mut self) {
        if let Ok(mut lane) = self.shared.lane.lock() {
            lane.stopped = true;
            lane.waiting.clear();
        }
        self.shared.ready.notify_all();
    }
}

enum Job {
    Draw(Box<TitleRequest>),
    Families,
}

fn worker(shared: Arc<Shared>, make_book: impl Fn() -> FontBook, wake: Wake) {
    let mut book = make_book();
    loop {
        let job = {
            let Ok(mut lane) = shared.lane.lock() else {
                return;
            };
            loop {
                if lane.stopped {
                    return;
                }
                if lane.families {
                    lane.families = false;
                    break Job::Families;
                }
                // A full outbox waits for the caller to take from it.
                if lane.held < TITLE_BUDGET
                    && let Some(next) = lane.waiting.pop_front()
                {
                    break Job::Draw(Box::new(next));
                }
                let Ok(next) = shared.ready.wait(lane) else {
                    return;
                };
                lane = next;
            }
        };
        let event = match job {
            Job::Families => Some(TextEvent::Families(book.families().into())),
            Job::Draw(request) => {
                book.set_project_fonts(&request.fonts);
                let drawn = catch_unwind(AssertUnwindSafe(|| {
                    draw_title(&mut book, &request.source, request.scale)
                }));
                match drawn {
                    Ok(Some(picture)) => Some(TextEvent::Drawn(Box::new(TitleDrawn {
                        clip: request.clip,
                        serial: request.serial,
                        source: request.source,
                        scale: request.scale,
                        picture: Arc::new(picture),
                    }))),
                    Ok(None) => None,
                    Err(_) => {
                        // A face that broke the drawing may have left the
                        // book half changed: start a new one.
                        book = make_book();
                        None
                    }
                }
            }
        };
        let Some(event) = event else {
            continue;
        };
        let Ok(mut lane) = shared.lane.lock() else {
            return;
        };
        if lane.stopped {
            return;
        }
        if let TextEvent::Drawn(drawn) = &event {
            lane.held += drawn.picture.byte_len();
        }
        lane.events.push_back(event);
        drop(lane);
        wake();
    }
}
