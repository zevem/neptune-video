//! Issue 16: what the media worker is asked for, and which layers get to ask.
//!
//! Layers are composed with [`Textures::compose`] in a real egui context
//! against a real [`MediaService`]: its own preview worker decodes still
//! pictures from files in a directory of the test's own, publishes, and calls
//! the wake the service was started with. Results are polled and handed over
//! the way the application does, to [`Textures::accept`] and
//! [`Textures::failed`]. Stills need no external decoder, and a still whose
//! file is not there fails each time it is asked for.
//!
//! The wake is the one thing here that is the test's: it holds the worker
//! where it has just published a result until the test lets it on, so there is
//! one result to take at a time and every frame is composed while the worker
//! stands still. What the worker was asked for is read from what it publishes.
//! It takes what consumers ask for in the order they first asked, so a picture
//! of the test's own, asked for last under a consumer no layer has, comes
//! after everything asked before it: that nothing was asked is known when it
//! comes first, with no waiting for nothing to happen. The only clocks are
//! bounds on waiting for something that should come.
//!
//! A failure says what kind it is. `ISSUE16:` is the behavior the issue asks
//! for. `CONTROL:` holds before the correction and must hold after it.
//! `FIXTURE:` is a precondition of the test itself and proves nothing about
//! the viewer.
//!
//! Nothing is painted and nothing is timed. A round is one result published
//! and one frame composed, not a frame of a display; a wake reaching a painted
//! frame is the windowing system's and is not shown here, nor is anything
//! about video, which needs a decoder these tests do not start.
use super::*;
use crate::textures::{LayerUsage, ViewerBudget, Wanted, layer_textures};
use editor_core::AssetId;
use editor_media::{MediaEvent, MediaService, PreviewConsumer};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

/// The side of the box layers are drawn at, and of the stills drawn at it: a
/// still as large as its box is decoded as it is, every texel its colour.
const BOX: u32 = 32;
/// The frame of the project a paused scene stands at.
const STILL: i64 = 12;
/// Layers drawn together: more than the 32 the worker keeps asked for.
const COHORT: u8 = 40;
/// Results published one at a time in which each of [`COHORT`] playing layers
/// is to have had a picture: twice what one each takes.
const ROUNDS: i64 = 80;
/// Paused frames composed after a failure with nothing changed.
const UNCHANGED: usize = 4;
/// How long the test waits for a result the worker should publish.
const PATIENCE: Duration = Duration::from_secs(20);
/// How long the worker stays held if the test never lets it on.
const HELD: Duration = Duration::from_secs(120);

fn side(side: u32) -> [usize; 2] {
    [side as usize; 2]
}

/// The RGBA bytes of a picture that fills a square box.
fn texels(side: u32) -> usize {
    side as usize * side as usize * 4
}

/// A still picture `side` pixels square at `path`, as an import describes it.
fn still(path: PathBuf, side: u32) -> Asset {
    let mut asset = Asset::new(path, MediaKind::Image, Time::from_seconds(5.0));
    asset.width = side;
    asset.height = side;
    asset.file_size = 1000;
    asset
}

/// The one colour every texel of `frame` has, when it is whole and has one.
fn one_colour(frame: &Frame) -> Option<[u8; 4]> {
    let whole = frame.rgba.len() == frame.width as usize * frame.height as usize * 4;
    let mut each = frame.rgba.chunks_exact(4);
    let first: [u8; 4] = each.next()?.try_into().ok()?;
    (whole && each.all(|texel| texel == first.as_slice())).then_some(first)
}

// ---- The worker, held at each result ----------------------------------------

/// What the worker and the test have each done.
#[derive(Default)]
struct Turns {
    /// Results the worker has published and woken the application for.
    published: u64,
    /// How many of those the test has waited for and seen.
    seen: u64,
    /// How many of those the test has let the worker on from.
    passed: u64,
    /// The test is over, and the worker is held no longer.
    open: bool,
    /// The worker was held for [`HELD`] and went on by itself.
    gave_up: bool,
}

/// The wake of the application, as the test gives it to the service.
#[derive(Default)]
struct Gate {
    turns: Mutex<Turns>,
    turned: Condvar,
}

impl Gate {
    /// A worker that panicked is no reason for the test to say nothing.
    fn turns(&self) -> MutexGuard<'_, Turns> {
        self.turns.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Called by the worker when it has published a result, holding no lock
    /// of the service. Stays there until the test lets it on.
    fn woken(&self) {
        let mut turns = self.turns();
        turns.published += 1;
        self.turned.notify_all();
        let until = Instant::now() + HELD;
        while !turns.open && turns.passed < turns.published {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                turns.gave_up = true;
                return;
            }
            turns = self
                .turned
                .wait_timeout(turns, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Lets the worker on from the results the test has seen and waits for
    /// the next, which may be there already. Returns whether one came. The
    /// worker is held at each result until the call after the one that saw
    /// it, so it is never more than one result ahead of the test.
    fn pass(&self) -> bool {
        let mut turns = self.turns();
        turns.passed = turns.seen;
        self.turned.notify_all();
        let until = Instant::now() + PATIENCE;
        while turns.published == turns.seen {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            turns = self
                .turned
                .wait_timeout(turns, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        turns.seen += 1;
        true
    }

    fn gave_up(&self) -> bool {
        self.turns().gave_up
    }

    fn open(&self) {
        self.turns().open = true;
        self.turned.notify_all();
    }
}

/// The service and the gate its worker is held at. The gate is opened before
/// the service is dropped, however the test ends, so no worker is left held.
struct Held {
    gate: Arc<Gate>,
    service: MediaService,
}

impl Drop for Held {
    fn drop(&mut self) {
        self.gate.open();
    }
}

/// A directory of the test's own, removed with everything in it when the test
/// ends, however it ends.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("aster-test-{}", uuid::Uuid::new_v4()));
        fixture(
            std::fs::create_dir_all(&dir).is_ok(),
            "the test's own directory was not made",
        );
        Self { dir }
    }

    /// A path in the directory that nothing was written at.
    fn unwritten(&self) -> PathBuf {
        self.dir.join(format!("{}.png", uuid::Uuid::new_v4()))
    }

    /// A still of one colour, `side` pixels square, written to a file.
    fn painted(&self, side: u32, colour: [u8; 4]) -> PathBuf {
        let path = self.unwritten();
        let saved = image::RgbaImage::from_pixel(side, side, image::Rgba(colour)).save(&path);
        fixture(saved.is_ok(), "a still was not written to its file");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---- What is drawn and what comes of it -------------------------------------

/// One picture of a frame: what it is drawn for, of which asset, in a square
/// box of this side, at this source time.
#[derive(Clone, Copy)]
struct Draw {
    viewing: Viewing,
    asset: AssetId,
    side: u32,
    time: Time,
}

fn at(viewing: Viewing, asset: AssetId, side: u32, time: Time) -> Draw {
    Draw {
        viewing,
        asset,
        side,
        time,
    }
}

/// A result the worker published: who asked, for what, at which box.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Outcome {
    consumer: PreviewConsumer,
    asset: AssetId,
    size: [u32; 2],
    result: Published,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Published {
    /// A decoded frame: the time it was asked at, its texels, the one colour
    /// they all have if they do, and whether a layer kept it.
    Picture {
        time: Time,
        texels: [u32; 2],
        colour: Option<[u8; 4]>,
        taken: bool,
    },
    /// A failure to decode, and whether it was of something a layer still
    /// asks for: what the application tells the person of.
    Failure { told: bool },
}

/// A failure of what `consumer` asks for now.
fn failure(consumer: PreviewConsumer, asset: AssetId, side: u32) -> Outcome {
    Outcome {
        consumer,
        asset,
        size: [side, side],
        result: Published::Failure { told: true },
    }
}

/// Whether one of `outcomes` is a frame that fills a box of `side`, published
/// for `consumer` and kept by its layer.
fn kept(outcomes: &[Outcome], consumer: PreviewConsumer, side: u32) -> bool {
    outcomes.iter().any(|outcome| {
        outcome.consumer == consumer
            && outcome.size == [side, side]
            && matches!(
                outcome.result,
                Published::Picture { texels, taken: true, .. } if texels == [side, side]
            )
    })
}

/// One of the layers drawn together, and the colour of its own still.
struct Member {
    viewing: Viewing,
    asset: AssetId,
    colour: [u8; 4],
}

/// One frame of all of `cohort`, in paint order, at `time`.
fn all_at(cohort: &[Member], time: Time) -> Vec<Draw> {
    cohort
        .iter()
        .map(|member| at(member.viewing, member.asset, BOX, time))
        .collect()
}

/// Layers composed in a real context against a real media service.
struct Rig {
    ctx: egui::Context,
    textures: Textures,
    /// The assets of the project, as the application looks them up.
    library: HashMap<AssetId, Asset>,
    /// The still the test asks for itself, under consumers no layer has.
    marker: Asset,
    /// Dropped before the directory its worker reads from.
    media: Held,
    scratch: Scratch,
}

impl Rig {
    fn new() -> Self {
        Self::with_budget(ViewerBudget::PRODUCTION)
    }

    /// Starts the service and holds its worker at a first result, so that
    /// nothing a layer asks for is taken up before the test lets it be.
    fn with_budget(budget: ViewerBudget) -> Self {
        let scratch = Scratch::new();
        let marker = still(scratch.painted(1, GREEN), 1);
        let gate = Arc::new(Gate::default());
        let wake = Arc::clone(&gate);
        let Ok(service) = MediaService::new(move || wake.woken()) else {
            panic!("FIXTURE: the media service did not start");
        };
        let mut rig = Self {
            ctx: egui::Context::default(),
            textures: Textures::with_viewer_budget(budget),
            library: HashMap::new(),
            marker,
            media: Held { gate, service },
            scratch,
        };
        fixture(
            rig.asked_for().is_empty(),
            "the worker published something before any layer asked",
        );
        rig
    }

    fn add(&mut self, asset: Asset) -> AssetId {
        let id = asset.id;
        self.library.insert(id, asset);
        id
    }

    /// An asset whose still is a file of one colour, `side` pixels square.
    fn painted(&mut self, side: u32, colour: [u8; 4]) -> AssetId {
        let asset = still(self.scratch.painted(side, colour), side);
        self.add(asset)
    }

    /// An asset whose file is not there: moved or deleted since its import.
    fn gone(&mut self) -> AssetId {
        let asset = still(self.scratch.unwritten(), BOX);
        self.add(asset)
    }

    /// [`COHORT`] clips of an asset each, every still its own colour.
    fn cohort(&mut self) -> Vec<Member> {
        (0..COHORT)
            .map(|index| {
                let colour = [40 + 5 * index, 245 - 5 * index, 90, 255];
                Member {
                    viewing: Viewing::Clip(ClipId::new_v4()),
                    asset: self.painted(BOX, colour),
                    colour,
                }
            })
            .collect()
    }

    /// Deletes the file of `asset`. What the worker has decoded of it at a
    /// size stays decoded; at any other size it now fails.
    fn lose(&self, asset: AssetId) {
        let lost = self
            .library
            .get(&asset)
            .is_some_and(|asset| std::fs::remove_file(&asset.path).is_ok());
        fixture(lost, "the file of an asset was not deleted");
    }

    /// The library now has `asset` at a file that is there, and the
    /// application brings its textures and the worker in line with that.
    fn relink(&mut self, asset: AssetId, colour: [u8; 4]) {
        let mut found = still(self.scratch.painted(BOX, colour), BOX);
        found.id = asset;
        self.library.insert(asset, found);
        self.textures.forget_asset(asset);
        self.restart();
    }

    /// The application drops what the worker has queued and asks again.
    fn restart(&mut self) {
        self.media.service.cancel();
        self.textures.ask_again();
    }

    /// Another project takes this one's place, with the same library.
    fn replace_project(&mut self) {
        self.media.service.cancel();
        self.textures.clear();
    }

    /// One frame that draws `drawn`, in that order.
    fn frame(&mut self, playing: bool, drawn: &[Draw]) {
        self.textures.begin_frame();
        let mut wanted = Vec::with_capacity(drawn.len());
        for draw in drawn {
            let Some(asset) = self.library.get(&draw.asset) else {
                panic!("FIXTURE: a picture is drawn of an asset the library does not have");
            };
            wanted.push(Wanted {
                viewing: draw.viewing,
                asset,
                size: [draw.side, draw.side],
                time: draw.time,
                carries_on: Vec::new(),
            });
        }
        let composed = self
            .textures
            .compose(Some(&self.media.service), playing, &wanted);
        fixture(
            composed.unplaced == 0,
            "a picture went unplaced in a budget with room for all of them",
        );
    }

    /// The consumer of the layer kept for `viewing`, which asks at a box.
    fn consumer(&self, viewing: Viewing) -> PreviewConsumer {
        let Some((consumer, _)) = self.textures.asked(viewing) else {
            panic!("FIXTURE: a layer that was drawn asks at no box");
        };
        consumer
    }

    /// The size of the picture the layer kept for `viewing` has.
    fn picture(&self, viewing: Viewing) -> Option<[usize; 2]> {
        self.textures.picture(viewing).map(|picture| picture.size())
    }

    /// The layer textures the context holds, which must be those the layers
    /// keep, within the viewer budget.
    fn held(&self) -> LayerUsage {
        let held = layer_textures(&self.ctx);
        assert_eq!(
            self.textures.layer_usage(),
            held,
            "CONTROL: the layers' own handles are the left; the layer textures the \
             context holds are the right"
        );
        let budget = self.textures.viewer_budget().bytes;
        assert!(
            held.bytes <= budget,
            "CONTROL: the layer textures hold {} bytes in {} pictures where the viewer \
             budget is {budget} bytes",
            held.bytes,
            held.pictures
        );
        held
    }

    /// Lets the worker on to its next result and waits for it. Returns
    /// whether it published one; it is then held there.
    fn publication(&self) -> bool {
        let came = self.media.gate.pass();
        fixture(
            !self.media.gate.gave_up(),
            "the worker was held longer than it waits to be let on",
        );
        came
    }

    /// Takes the one result the worker published and hands it over as the
    /// application does: only of this generation, a frame only of an asset
    /// the library has.
    fn take(&mut self) -> Outcome {
        let generation = self.media.service.generation();
        let Some(event) = self.media.service.poll() else {
            panic!("FIXTURE: the worker woke the application with nothing to take");
        };
        let outcome = match event {
            MediaEvent::FrameReady {
                generation: from,
                asset_id,
                time,
                size,
                consumer,
                frame,
            } => {
                let Some(consumer) = consumer else {
                    panic!("FIXTURE: a frame was published for no consumer");
                };
                let mut taken = false;
                if from == generation
                    && let Some(asset) = self.library.get(&asset_id)
                {
                    taken = self.textures.accept(
                        &self.ctx,
                        Some(asset),
                        Some(consumer),
                        asset_id,
                        time,
                        size,
                        &frame,
                    );
                }
                Outcome {
                    consumer,
                    asset: asset_id,
                    size,
                    result: Published::Picture {
                        time,
                        texels: [frame.width, frame.height],
                        colour: one_colour(&frame),
                        taken,
                    },
                }
            }
            MediaEvent::Error {
                generation: from,
                asset_id,
                consumer,
                size,
                ..
            } => {
                let (Some(asset), Some(consumer), Some(size)) = (asset_id, consumer, size) else {
                    panic!("FIXTURE: a failure names no asset, no consumer or no box");
                };
                let told =
                    from == generation && self.textures.failed(Some(consumer), asset, Some(size));
                Outcome {
                    consumer,
                    asset,
                    size,
                    result: Published::Failure { told },
                }
            }
            MediaEvent::Imported { .. } => panic!("FIXTURE: nothing was imported"),
            MediaEvent::LutLoaded { .. } => panic!("FIXTURE: no table was asked for"),
        };
        fixture(
            self.media.service.poll().is_none(),
            "the worker published more than one result while it was held",
        );
        self.held();
        outcome
    }

    /// The next result the worker publishes, handed over. Nothing when it
    /// publishes none for as long as the test waits.
    fn result(&mut self) -> Option<Outcome> {
        if !self.publication() {
            return None;
        }
        Some(self.take())
    }

    /// Everything the worker has been asked for and has not published, handed
    /// over in the order it is published. The test's own still is asked for
    /// last; the worker takes consumers in the order they first asked, so
    /// what is published before that still is all there was. Empty when
    /// nothing was asked for.
    fn asked_for(&mut self) -> Vec<Outcome> {
        let marker = PreviewConsumer::new();
        let asked = self
            .media
            .service
            .request_preview(marker, &self.marker, Time::ZERO, [1, 1]);
        fixture(
            asked.is_ok(),
            "the worker did not take the test's own request",
        );
        let mut before = Vec::new();
        loop {
            let Some(outcome) = self.result() else {
                panic!(
                    "FIXTURE: the test's own still was not published; before it came \
                     {before:?}"
                );
            };
            if outcome.consumer == marker {
                return before;
            }
            fixture(
                before.len() < 64,
                "the worker went on publishing ahead of the test's own still",
            );
            before.push(outcome);
        }
    }

    /// One paused frame of `failing` alone, whose file is gone: it is asked
    /// for once and fails once. Returns the consumer of its layer.
    fn fails_once(&mut self, failing: Draw) -> PreviewConsumer {
        self.frame(false, &[failing]);
        let consumer = self.consumer(failing.viewing);
        assert_eq!(
            self.asked_for(),
            [failure(consumer, failing.asset, failing.side)],
            "CONTROL: a picture whose file is gone was not asked for once, at the box \
             its layer asks at, and failed once as its layer's"
        );
        consumer
    }

    /// The member of `cohort` that `outcome` is the own picture of: published
    /// under the consumer its layer has and at the box it asks at, of its
    /// asset, a frame that fills the box with every texel the colour of its
    /// still, and kept by its layer. A box asked at is not a picture.
    fn own_picture(&self, cohort: &[Member], outcome: Outcome) -> Option<usize> {
        cohort.iter().position(|member| {
            self.textures.asked(member.viewing) == Some((outcome.consumer, [BOX, BOX]))
                && outcome.asset == member.asset
                && outcome.size == [BOX, BOX]
                && matches!(
                    outcome.result,
                    Published::Picture { texels, colour, taken: true, .. }
                        if texels == [BOX, BOX] && colour == Some(member.colour)
                )
                && self.picture(member.viewing) == Some(side(BOX))
        })
    }
}

// ---- A request that fails ---------------------------------------------------

/// A paused picture whose file is gone, drawn for `viewing` and nothing else,
/// frame after frame with nothing changed.
fn settles_after_one_failure(viewing: Viewing, what: &str) {
    let mut rig = Rig::new();
    let gone = rig.gone();
    let scene = [at(viewing, gone, BOX, frame(STILL))];
    let consumer = rig.fails_once(scene[0]);

    let (mut frames, mut asked) = (0, 0);
    for _ in 0..UNCHANGED {
        rig.frame(false, &scene);
        let again = rig.asked_for();
        frames += usize::from(!again.is_empty());
        asked += again.len();
    }
    assert!(
        asked == 0,
        "ISSUE16: {what} whose file is gone was asked for again in {frames} of \
         {UNCHANGED} paused frames in which nothing changed, {asked} times in all: each \
         is a decode, a failure handed over and a wake of the application, which brings \
         the next frame about"
    );
    assert_eq!(
        rig.textures.asked(viewing),
        Some((consumer, [BOX, BOX])),
        "CONTROL: a layer whose picture failed is no longer kept under its consumer at \
         its box"
    );
    assert_eq!(
        rig.held(),
        LayerUsage {
            pictures: 0,
            bytes: 0
        },
        "CONTROL: a layer whose every frame failed has a picture"
    );
}

#[test]
fn a_clip_whose_file_is_gone_is_asked_for_once_while_nothing_changes() {
    settles_after_one_failure(Viewing::Clip(ClipId::new_v4()), "a clip");
}

#[test]
fn the_source_monitor_asks_once_for_a_file_that_is_gone_while_nothing_changes() {
    settles_after_one_failure(Viewing::Source, "the source monitor's picture");
}

#[test]
fn a_picture_that_failed_is_asked_for_again_at_another_time() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let gone = rig.gone();
    let consumer = rig.fails_once(at(clip, gone, BOX, frame(STILL)));

    rig.frame(false, &[at(clip, gone, BOX, frame(STILL + 1))]);
    assert_eq!(
        rig.asked_for(),
        [failure(consumer, gone, BOX)],
        "CONTROL: a picture that failed at one time was not asked for once, under the \
         same consumer, when its layer was drawn at another"
    );
}

#[test]
fn a_picture_that_failed_is_asked_for_again_at_another_box() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let gone = rig.gone();
    let consumer = rig.fails_once(at(clip, gone, BOX, frame(STILL)));

    rig.frame(false, &[at(clip, gone, BOX / 2, frame(STILL))]);
    assert_eq!(
        rig.asked_for(),
        [failure(consumer, gone, BOX / 2)],
        "CONTROL: a picture that failed at one box was not asked for once, under the \
         same consumer, when its layer was drawn at a smaller one"
    );
}

#[test]
fn a_picture_that_failed_is_asked_for_again_when_the_worker_starts_over() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let gone = rig.gone();
    let scene = at(clip, gone, BOX, frame(STILL));
    let consumer = rig.fails_once(scene);

    rig.restart();
    rig.frame(false, &[scene]);
    assert_eq!(
        rig.asked_for(),
        [failure(consumer, gone, BOX)],
        "CONTROL: a picture that failed was not asked for once more, under the same \
         consumer, after the worker dropped what it had and everything was to be asked \
         again"
    );
}

#[test]
fn a_picture_that_failed_is_asked_for_again_when_its_asset_is_forgotten() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let gone = rig.gone();
    let scene = at(clip, gone, BOX, frame(STILL));
    let old = rig.fails_once(scene);

    rig.textures.forget_asset(gone);
    rig.frame(false, &[scene]);
    let new = rig.consumer(clip);
    assert_ne!(
        new, old,
        "CONTROL: a layer of a forgotten asset was kept under its consumer"
    );
    assert_eq!(
        rig.asked_for(),
        [failure(new, gone, BOX)],
        "CONTROL: the layer started for a clip after its asset was forgotten did not \
         ask once for its picture"
    );
}

#[test]
fn an_asset_found_again_is_asked_for_and_shows_its_picture() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let gone = rig.gone();
    let scene = at(clip, gone, BOX, frame(STILL));
    rig.fails_once(scene);

    rig.relink(gone, AMBER);
    rig.frame(false, &[scene]);
    let consumer = rig.consumer(clip);
    assert_eq!(
        rig.asked_for(),
        [Outcome {
            consumer,
            asset: gone,
            size: [BOX, BOX],
            result: Published::Picture {
                time: frame(STILL),
                texels: [BOX, BOX],
                colour: Some(AMBER),
                taken: true,
            },
        }],
        "CONTROL: a clip whose asset now has a file that is there was not asked for \
         once and given that file's picture"
    );
    assert_eq!(
        rig.picture(clip),
        Some(side(BOX)),
        "CONTROL: a clip whose asset was found again shows no picture of it"
    );
}

#[test]
fn another_clip_of_an_asset_that_failed_asks_for_its_own_picture() {
    let mut rig = Rig::new();
    let [first, second] = [0, 1].map(|_| Viewing::Clip(ClipId::new_v4()));
    let gone = rig.gone();
    rig.fails_once(at(first, gone, BOX, frame(STILL)));

    rig.frame(
        false,
        &[
            at(first, gone, BOX, frame(STILL)),
            at(second, gone, BOX, frame(STILL)),
        ],
    );
    let consumer = rig.consumer(second);
    let asked = rig.asked_for();
    let its_own = asked
        .iter()
        .filter(|outcome| outcome.consumer == consumer)
        .count();
    assert!(
        its_own == 1 && asked.contains(&failure(consumer, gone, BOX)),
        "CONTROL: a clip that came on screen was not asked for once under its own \
         consumer because another clip of its asset had failed; the worker published \
         {asked:?}"
    );
}

#[test]
fn a_picture_that_failed_is_asked_for_again_in_another_project() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let gone = rig.gone();
    let scene = at(clip, gone, BOX, frame(STILL));
    let old = rig.fails_once(scene);

    rig.replace_project();
    rig.frame(false, &[scene]);
    let new = rig.consumer(clip);
    assert_ne!(
        new, old,
        "CONTROL: a layer was kept under its consumer when the project was replaced"
    );
    assert_eq!(
        rig.asked_for(),
        [failure(new, gone, BOX)],
        "CONTROL: a layer of the project that took the place of another did not ask \
         once for its picture"
    );
}

#[test]
fn a_failure_at_a_box_no_longer_asked_at_is_not_the_layers() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let gone = rig.gone();
    rig.frame(false, &[at(clip, gone, BOX, frame(STILL))]);
    let consumer = rig.consumer(clip);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );

    // The failure is published and not yet taken when the layer is drawn at
    // a smaller box.
    rig.frame(false, &[at(clip, gone, BOX / 2, frame(STILL))]);
    assert_eq!(
        rig.take(),
        Outcome {
            consumer,
            asset: gone,
            size: [BOX, BOX],
            result: Published::Failure { told: false },
        },
        "CONTROL: a failure at the box a layer asked at before was taken as a failure \
         of what it asks for now"
    );
    assert_eq!(
        rig.asked_for(),
        [failure(consumer, gone, BOX / 2)],
        "CONTROL: a layer was not asked for once at the box it is drawn at now, and \
         told of that failure, after a failure at the box it asked at before"
    );
}

// ---- A failure and the picture a layer has ----------------------------------

#[test]
fn a_larger_frame_that_fails_leaves_a_layer_the_picture_it_has() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let asset = rig.painted(2 * BOX, AMBER);
    let small = BOX / 2;
    rig.frame(false, &[at(clip, asset, small, frame(STILL))]);
    let consumer = rig.consumer(clip);
    let first = rig.asked_for();
    fixture(
        first.len() == 1 && kept(&first, consumer, small),
        "a clip was not given a picture that fills the box it asks at",
    );

    rig.lose(asset);
    rig.frame(false, &[at(clip, asset, BOX, frame(STILL))]);
    assert_eq!(
        rig.asked_for(),
        [failure(consumer, asset, BOX)],
        "CONTROL: a clip drawn larger, whose file is gone, was not asked for once at \
         the larger box and failed once"
    );
    assert_eq!(
        rig.picture(clip),
        Some(side(small)),
        "CONTROL: a layer whose larger frame failed did not keep the picture it had, \
         which fits the box it asks at"
    );
    assert_eq!(
        rig.held(),
        LayerUsage {
            pictures: 1,
            bytes: texels(small)
        },
        "CONTROL: the layer textures after a larger frame failed"
    );
}

#[test]
fn a_smaller_frame_that_fails_gives_up_that_picture_alone_and_lets_the_waiting_layer_in() {
    let mut rig = Rig::with_budget(ViewerBudget {
        bytes: 2 * texels(BOX),
        spare_layers: 8,
    });
    let clips = [0, 1, 2].map(|_| Viewing::Clip(ClipId::new_v4()));
    let assets = [AMBER, BLUE, RED].map(|colour| rig.painted(2 * BOX, colour));
    let draw = |layer: usize| at(clips[layer], assets[layer], BOX, frame(STILL));
    let small = BOX / 2;

    rig.frame(false, &[draw(0), draw(1)]);
    let consumers = [rig.consumer(clips[0]), rig.consumer(clips[1])];
    let first = rig.asked_for();
    fixture(
        first.len() == 2
            && kept(&first, consumers[0], BOX)
            && kept(&first, consumers[1], BOX)
            && rig.held().bytes == 2 * texels(BOX),
        "two clips did not fill the budget with a picture each",
    );

    // A third clip is drawn: all three are held to the smaller box, the two
    // that have pictures keep them until smaller ones arrive, and the third
    // waits for the room. The first clip's file is gone by then.
    rig.lose(assets[0]);
    let all = [draw(0), draw(1), draw(2)];
    rig.frame(false, &all);
    assert_eq!(
        (
            rig.textures.asked(clips[0]),
            rig.textures.asked(clips[1]),
            rig.textures.asked(clips[2])
        ),
        (
            Some((consumers[0], [small, small])),
            Some((consumers[1], [small, small])),
            None
        ),
        "CONTROL: beside two pictures that fill the budget, the layers that have them \
         do not ask at the smaller box, or the third does not wait for room"
    );
    let second = rig.asked_for();
    assert!(
        second.len() == 2
            && second[0] == failure(consumers[0], assets[0], small)
            && kept(&second, consumers[1], small),
        "CONTROL: the smaller frames were not asked for once each, the first failing \
         and the second kept; the worker published {second:?}"
    );
    assert_eq!(
        [rig.picture(clips[0]), rig.picture(clips[1])],
        [None, Some(side(small))],
        "CONTROL: after its smaller frame failed, the first layer's larger picture is \
         the left of these and the second layer's picture the right"
    );

    rig.frame(false, &all);
    let Some((third, _)) = rig.textures.asked(clips[2]) else {
        panic!(
            "CONTROL: the layer that waited asks at no box in the frame after the \
             failed picture was given up"
        );
    };
    let last = rig.asked_for();
    assert!(
        kept(&last, third, small),
        "CONTROL: the layer that waited was not given a picture once there was room; \
         the worker published {last:?}"
    );
    assert_eq!(
        clips.map(|clip| rig.picture(clip)),
        [None, Some(side(small)), Some(side(small))],
        "CONTROL: the pictures of the three layers once the third was let in"
    );
    assert_eq!(
        rig.held(),
        LayerUsage {
            pictures: 2,
            bytes: 2 * texels(small)
        },
        "CONTROL: the layer textures once the third layer was let in"
    );
}

// ---- More layers than the worker keeps asked for ----------------------------

#[test]
fn forty_paused_layers_each_get_their_picture_once_through_thirty_two_lanes() {
    let mut rig = Rig::new();
    let cohort = rig.cohort();
    let scene = all_at(&cohort, frame(STILL));
    rig.frame(false, &scene);

    let mut served = vec![0usize; cohort.len()];
    let mut strays = Vec::new();
    for _ in 0..COHORT {
        let Some(outcome) = rig.result() else {
            break;
        };
        let on_time = matches!(
            outcome.result,
            Published::Picture { time, .. } if time == frame(STILL)
        );
        match rig.own_picture(&cohort, outcome) {
            Some(member) if on_time => served[member] += 1,
            _ => strays.push(outcome),
        }
        // The result woke the application: the frame it brings about.
        rig.frame(false, &scene);
    }
    assert!(
        strays.is_empty(),
        "CONTROL: the worker published what is no layer's own picture at the time it \
         is drawn at: {strays:?}"
    );
    assert!(
        served.iter().all(|times| *times == 1),
        "CONTROL: of {COHORT} paused layers, each asked for when a result made room \
         among the lanes, not every one had its own picture once in {COHORT} results; \
         in paint order they had {served:?}"
    );
    let more = rig.asked_for();
    assert!(
        more.is_empty(),
        "CONTROL: paused layers that have their pictures were asked for again: {more:?}"
    );
    assert_eq!(
        rig.held(),
        LayerUsage {
            pictures: cohort.len(),
            bytes: cohort.len() * texels(BOX)
        },
        "CONTROL: the layer textures of {COHORT} layers that each have a picture"
    );
}

#[test]
fn forty_playing_layers_each_get_their_picture_through_thirty_two_lanes() {
    let mut rig = Rig::new();
    let cohort = rig.cohort();
    rig.frame(true, &all_at(&cohort, frame(0)));
    fixture(
        cohort.iter().all(|member| {
            rig.textures.asked(member.viewing).map(|(_, size)| size) == Some([BOX, BOX])
        }),
        "not every layer drawn was given the box it is drawn at",
    );

    let mut served = vec![0usize; cohort.len()];
    let mut strays = Vec::new();
    for round in 0..ROUNDS {
        if served.iter().all(|times| *times > 0) {
            break;
        }
        let Some(outcome) = rig.result() else {
            break;
        };
        match rig.own_picture(&cohort, outcome) {
            Some(member) => served[member] += 1,
            None => strays.push(outcome),
        }
        // The result woke the application: the next frame of playback, one
        // frame of the project on, draws them all again.
        rig.frame(true, &all_at(&cohort, frame(round + 1)));
    }
    assert!(
        strays.is_empty(),
        "CONTROL: the worker published what is no layer's own picture: {strays:?}"
    );
    let results = served.iter().sum::<usize>() + strays.len();
    let unserved: Vec<usize> = (0..cohort.len())
        .filter(|member| served[*member] == 0)
        .collect();
    assert!(
        unserved.is_empty(),
        "ISSUE16: of {COHORT} layers drawn together while playing, each with a box, \
         those at {unserved:?} in paint order never had a picture in {results} results \
         published one at a time, each followed by a frame that drew them all; in \
         paint order they had {served:?}"
    );
    assert_eq!(
        rig.held(),
        LayerUsage {
            pictures: cohort.len(),
            bytes: cohort.len() * texels(BOX)
        },
        "CONTROL: the layer textures of {COHORT} layers that each have a picture"
    );
}

// ---- The decoders the media service is asked to keep open -------------------
//
// Authored with the correction of issue 16, not before it: `RETENTION:` is
// which consumers a frame chooses to have their decoders kept open, read where
// the frame is composed. That the media service keeps those decoders, and
// what it saves, is the media crate's to show and is not shown here. Videos
// are drawn with no media service, so nothing is decoded and no decoder
// started.

/// Videos that are each an asset of their own.
fn videos(count: usize) -> Vec<Asset> {
    (0..count)
        .map(|index| video(&format!("/media/kept-open-{index}.mp4")))
        .collect()
}

/// A clip for each of `assets`.
fn clips_of(assets: &[Asset]) -> Vec<Viewing> {
    assets
        .iter()
        .map(|_| Viewing::Clip(ClipId::new_v4()))
        .collect()
}

/// One picture of a paused frame, in the box layers are drawn at here.
fn drawn(viewing: Viewing, asset: &Asset) -> Wanted<'_> {
    Wanted {
        viewing,
        asset,
        size: [BOX, BOX],
        time: frame(STILL),
        carries_on: Vec::new(),
    }
}

/// Those of `clips` named by `which`, in that order, each of its own asset.
fn drawn_of<'a>(clips: &[Viewing], assets: &'a [Asset], which: &[usize]) -> Vec<Wanted<'a>> {
    which
        .iter()
        .map(|index| drawn(clips[*index], &assets[*index]))
        .collect()
}

/// One paused frame of `wanted` with no media service, and the consumers it
/// chose to have their decoders kept open.
fn chosen(textures: &mut Textures, wanted: &[Wanted<'_>]) -> [Option<PreviewConsumer>; 3] {
    textures.begin_frame();
    textures.compose(None, false, wanted);
    textures.preview_retention()
}

/// The consumer of the layer kept for `viewing`, when it asks at a box.
fn consumer_of(textures: &Textures, viewing: Viewing) -> Option<PreviewConsumer> {
    textures.asked(viewing).map(|(consumer, _)| consumer)
}

/// A decoded frame that fills a square box of `side`.
fn filled(side: u32) -> Frame {
    Frame {
        width: side,
        height: side,
        rgba: vec![255; texels(side)].into(),
    }
}

#[test]
fn three_videos_of_five_are_kept_open_whatever_order_they_are_painted_in() {
    let mut textures = Textures::default();
    let assets = videos(5);
    let clips = clips_of(&assets);

    let first = chosen(&mut textures, &drawn_of(&clips, &assets, &[0, 1, 2, 3, 4]));
    let consumers: Vec<Option<PreviewConsumer>> = clips
        .iter()
        .map(|clip| consumer_of(&textures, *clip))
        .collect();
    fixture(
        consumers.iter().all(Option::is_some),
        "not every video drawn was given a box",
    );
    assert_eq!(
        first,
        [consumers[0], consumers[1], consumers[2]],
        "RETENTION: of five videos drawn together, the first three painted were not \
         the ones chosen to have their decoders kept open, each in its place"
    );

    for order in [[4, 3, 2, 1, 0], [3, 0, 4, 2, 1], [0, 1, 2, 3, 4]] {
        assert_eq!(
            chosen(&mut textures, &drawn_of(&clips, &assets, &order)),
            first,
            "RETENTION: the same five videos painted in the order {order:?} changed \
             which three are kept open, or the place of one of them"
        );
    }
    assert_eq!(
        clips
            .iter()
            .map(|clip| consumer_of(&textures, *clip))
            .collect::<Vec<_>>(),
        consumers,
        "CONTROL: a change of paint order changed the consumer of a layer"
    );
}

#[test]
fn a_video_that_leaves_gives_its_place_to_the_next_painted_and_the_others_keep_theirs() {
    let mut textures = Textures::default();
    let assets = videos(5);
    let clips = clips_of(&assets);
    let all = [0, 1, 2, 3, 4];
    let first = chosen(&mut textures, &drawn_of(&clips, &assets, &all));
    let fourth = consumer_of(&textures, clips[3]);
    fixture(
        first.iter().all(Option::is_some) && fourth.is_some() && !first.contains(&fourth),
        "three of five videos were not chosen, or the fourth painted was",
    );

    // The second painted is no longer drawn: its track is hidden, or the
    // playhead has left it. The fourth takes the place it leaves.
    assert_eq!(
        chosen(&mut textures, &drawn_of(&clips, &assets, &[0, 2, 3, 4])),
        [first[0], fourth, first[2]],
        "RETENTION: when one of the three chosen was no longer drawn, the next video \
         in paint order did not take its place while the other two kept theirs"
    );

    // Nothing is drawn: none is kept open. Drawn again, they are a new
    // choice, in paint order.
    assert_eq!(
        chosen(&mut textures, &[]),
        [None; 3],
        "RETENTION: a frame that draws nothing still chose decoders to keep open"
    );
    assert_eq!(
        chosen(&mut textures, &drawn_of(&clips, &assets, &all)),
        first,
        "RETENTION: the videos drawn again after a frame of nothing were not chosen \
         afresh in paint order"
    );

    // The asset of the first is forgotten: its layer is given up and the
    // clip starts another, under another consumer, which is not the one
    // chosen before.
    textures.forget_asset(assets[0].id);
    let again = chosen(&mut textures, &drawn_of(&clips, &assets, &all));
    let fresh = consumer_of(&textures, clips[0]);
    fixture(
        fresh.is_some() && fresh != first[0],
        "a layer of a forgotten asset was kept under its consumer",
    );
    assert!(
        !again.contains(&first[0]),
        "RETENTION: the consumer of a layer that was given up is still chosen: {again:?}"
    );
    assert_eq!(
        again,
        [fresh, first[1], first[2]],
        "RETENTION: the layer started in place of one given up did not take the place \
         left, or the other two did not keep theirs"
    );
}

#[test]
fn a_clip_that_carries_on_from_a_cut_keeps_the_place_of_the_clip_before_it() {
    let mut textures = Textures::default();
    let assets = videos(3);
    let clips = clips_of(&assets);
    let first = chosen(&mut textures, &drawn_of(&clips, &assets, &[0, 1, 2]));
    fixture(
        first.iter().all(Option::is_some),
        "three videos drawn together were not all chosen",
    );

    // The playhead crosses a cut in the second clip: the clip after the cut
    // is drawn, of the same asset, and carries on from the one before it. It
    // is painted first this frame.
    let Viewing::Clip(before) = clips[1] else {
        panic!("FIXTURE: a clip is not a clip");
    };
    let after = Viewing::Clip(ClipId::new_v4());
    let mut carried = drawn(after, &assets[1]);
    carried.carries_on = vec![before];
    let across = chosen(
        &mut textures,
        &[
            carried,
            drawn(clips[0], &assets[0]),
            drawn(clips[2], &assets[2]),
        ],
    );
    fixture(
        consumer_of(&textures, after) == first[1] && !textures.has_layer(clips[1]),
        "the clip after a cut did not take over the layer of the clip before it",
    );
    assert_eq!(
        across, first,
        "RETENTION: a clip that took over the layer of a cut did not keep the place \
         that layer's consumer had among the decoders kept open"
    );
}

#[test]
fn only_videos_are_kept_open() {
    let mut textures = Textures::default();
    let assets = videos(2);
    let picture = still(PathBuf::from("/media/kept-open.png"), BOX);
    let mut animation = still(PathBuf::from("/media/kept-open.gif"), BOX);
    animation.kind = MediaKind::Gif;
    let [first, second, third, fourth] = [0, 1, 2, 3].map(|_| Viewing::Clip(ClipId::new_v4()));

    let mixed = chosen(
        &mut textures,
        &[
            drawn(first, &picture),
            drawn(second, &assets[0]),
            drawn(third, &animation),
            drawn(fourth, &assets[1]),
        ],
    );
    fixture(
        [first, second, third, fourth]
            .iter()
            .all(|clip| consumer_of(&textures, *clip).is_some()),
        "not every picture drawn was given a box",
    );
    assert_eq!(
        mixed,
        [
            consumer_of(&textures, second),
            consumer_of(&textures, fourth),
            None
        ],
        "RETENTION: of a still, a video, an animation and a video drawn together, the \
         two videos alone were not chosen, in paint order"
    );
    assert_eq!(
        chosen(
            &mut textures,
            &[drawn(first, &picture), drawn(third, &animation)]
        ),
        [None; 3],
        "RETENTION: a frame that draws no video chose decoders to keep open"
    );
}

#[test]
fn a_video_that_waits_for_room_is_kept_open_only_once_it_has_a_box() {
    let ctx = egui::Context::default();
    let mut textures = Textures::with_viewer_budget(ViewerBudget {
        bytes: 2 * texels(BOX),
        spare_layers: 8,
    });
    let assets = videos(3);
    let clips = clips_of(&assets);
    let small = BOX / 2;

    // Two videos fill the budget with a picture each.
    let two = chosen(&mut textures, &drawn_of(&clips, &assets, &[0, 1]));
    let arrive = |textures: &mut Textures, layer: usize, side: u32| {
        let asked = textures.asked(clips[layer]);
        let taken = asked.is_some_and(|(consumer, size)| {
            let time = textures.admitted(consumer);
            size == [side, side]
                && textures.accept(
                    &ctx,
                    Some(&assets[layer]),
                    Some(consumer),
                    assets[layer].id,
                    time,
                    size,
                    &filled(side),
                )
        });
        fixture(
            taken,
            "a layer did not take a frame that fills the box it asks at",
        );
    };
    arrive(&mut textures, 0, BOX);
    arrive(&mut textures, 1, BOX);
    fixture(
        two[0].is_some() && two[1].is_some() && two[2].is_none(),
        "two videos drawn together were not the two chosen",
    );

    // A third is drawn: all are held to the smaller box, and the third waits
    // for the room the two pictures hold. It asks for nothing.
    let waiting = chosen(&mut textures, &drawn_of(&clips, &assets, &[0, 1, 2]));
    fixture(
        textures.asked(clips[2]).is_none(),
        "the third video was given a box beside two pictures that fill the budget",
    );
    assert_eq!(
        waiting, two,
        "RETENTION: a video that waits for room, and asks for nothing, was chosen to \
         have a decoder kept open, or one of the two that have a box was not"
    );

    // The smaller frames arrive, the third is let in, and it is chosen.
    arrive(&mut textures, 0, small);
    arrive(&mut textures, 1, small);
    let settled = chosen(&mut textures, &drawn_of(&clips, &assets, &[0, 1, 2]));
    let third = consumer_of(&textures, clips[2]);
    fixture(
        third.is_some(),
        "the video that waited was not given a box once there was room",
    );
    assert_eq!(
        settled,
        [two[0], two[1], third],
        "RETENTION: the video that waited was not chosen once it had a box, or one of \
         the other two lost its place"
    );
    assert!(
        layer_textures(&ctx).bytes <= textures.viewer_budget().bytes,
        "CONTROL: the layer textures are past the viewer budget"
    );
}

#[test]
fn the_viewer_keeps_open_the_clips_it_paints_and_the_source_monitor_only_its_own() {
    let mut scene = Scene::new();
    let (lower, upper) = (scene.lower, scene.upper);
    // Two halves of a cut on the lower track, and a clip above both.
    let before = scene.clip(Place {
        track: lower,
        column: -1,
        start: 0.0,
        source_in: 0.0,
        duration: 2.0,
    });
    let after = scene.clip(Place {
        track: lower,
        column: -1,
        start: 2.0,
        source_in: 2.0,
        duration: 2.0,
    });
    let above = scene.clip(Place {
        track: upper,
        column: 1,
        start: 0.0,
        source_in: 0.0,
        duration: 4.0,
    });
    let project = scene.project();
    let mut stage = Stage::new();
    let consumer = |stage: &Stage, clip: ClipId| consumer_of(&stage.textures, Viewing::Clip(clip));

    stage.paint(&project, Time::from_seconds(1.0), false);
    let timeline = [consumer(&stage, before), consumer(&stage, above), None];
    fixture(
        timeline[0].is_some() && timeline[1].is_some() && timeline[0] != timeline[1],
        "two clips painted together do not each have a layer that asks at a box",
    );
    assert_eq!(
        stage.textures.preview_retention(),
        timeline,
        "RETENTION: the two video clips the viewer painted were not the ones chosen to \
         have their decoders kept open, lower track first"
    );

    // Across the cut the clip after it takes over the layer, and its place.
    stage.paint(&project, Time::from_seconds(3.0), false);
    fixture(
        consumer(&stage, after) == timeline[0],
        "the clip after the cut did not take over the layer of the clip before it",
    );
    assert_eq!(
        stage.textures.preview_retention(),
        timeline,
        "RETENTION: playing across a cut changed which decoders are kept open"
    );

    // The source monitor shows another video: its own, and no clip's.
    let monitored = video("/media/in-the-monitor.mp4");
    stage.paint_source(&monitored);
    let source = consumer_of(&stage.textures, Viewing::Source);
    fixture(
        source.is_some() && !timeline.contains(&source),
        "the source monitor has no layer of its own that asks at a box",
    );
    assert_eq!(
        stage.textures.preview_retention(),
        [source, None, None],
        "RETENTION: with a video in the source monitor, its decoder alone was not the \
         one chosen to be kept open"
    );

    // A still in the monitor has no decoder to keep.
    stage.paint_source(&still(PathBuf::from("/media/in-the-monitor.png"), BOX));
    assert_eq!(
        stage.textures.preview_retention(),
        [None; 3],
        "RETENTION: with a still in the source monitor a decoder was chosen to be kept \
         open"
    );

    // Back at the timeline, the clips painted are chosen again.
    stage.paint(&project, Time::from_seconds(3.0), false);
    fixture(
        [consumer(&stage, after), consumer(&stage, above), None] == timeline,
        "the layers kept for the timeline were not kept while the source monitor was \
         shown",
    );
    assert_eq!(
        stage.textures.preview_retention(),
        timeline,
        "RETENTION: the clips painted after the source monitor was closed were not \
         chosen again, lower track first"
    );
}

#[test]
fn stills_asked_of_the_worker_are_none_of_them_kept_open() {
    let mut rig = Rig::new();
    let colours = [AMBER, BLUE, RED];
    let mut scene: Vec<Draw> = colours
        .iter()
        .map(|colour| {
            at(
                Viewing::Clip(ClipId::new_v4()),
                rig.painted(BOX, *colour),
                BOX,
                frame(STILL),
            )
        })
        .collect();
    scene.push(at(
        Viewing::Source,
        rig.painted(BOX, GREEN),
        BOX,
        frame(STILL),
    ));
    rig.frame(false, &scene);
    assert_eq!(
        rig.textures.preview_retention(),
        [None; 3],
        "RETENTION: stills drawn together, which have no decoder to keep, were chosen \
         to have one kept open"
    );
    let asked = rig.asked_for();
    assert!(
        asked.len() == scene.len()
            && scene
                .iter()
                .all(|draw| kept(&asked, rig.consumer(draw.viewing), BOX)),
        "CONTROL: four stills drawn together were not each asked for once and given a \
         picture that fills their box; the worker published {asked:?}"
    );
}

// ---- What a playing layer asks for ------------------------------------------

/// Nested here for the rig above, which it composes and publishes with.
#[path = "playback_admission_tests.rs"]
mod playback_admission_tests;

// ---- What a layer takes after its playhead was moved ------------------------

/// Nested here for the same rig, handed over in the application's order.
#[path = "seek_boundary_tests.rs"]
mod seek_boundary_tests;
