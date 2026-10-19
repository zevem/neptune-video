//! The camera paths the editor holds for the clips it steadies.
//!
//! A stabilized clip stores an effect with two numbers; the path its video's
//! camera took is measured by the analysis worker, kept on disk in a bounded
//! cache, and held here while a layer or the inspector wants it. Nothing
//! here reads a file or waits: a path that is wanted and not held is put on
//! a list the application hands to the worker, and until the answer comes
//! the layer is drawn as it is.
//!
//! An answer is taken only when it is for the request this asked, for the
//! project that is open and for the media as it is now; any other is
//! dropped. The paths held are bounded in number and in frames, and the one
//! least lately wanted makes room.
use editor_core::{Asset, AssetId, MediaKind, Time};
use editor_media::{Motion, Sampling, SourceIdentity, Steadied};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use uuid::Uuid;

/// The paths held at once, and the frames they may have between them: at
/// sixteen bytes a frame, 32 MiB.
pub const MAX_STEADIES: usize = 8;
pub const MAX_STEADY_HELD_FRAMES: usize = 2 << 20;

/// Where camera paths are kept between sessions: the user's cache
/// directory, or the system's temporary one on a system without a home.
pub fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("rs", "Neptune", "NeptuneVideo").map_or_else(
        || std::env::temp_dir().join("neptune-video-stabilization"),
        |dirs| dirs.cache_dir().join("stabilization"),
    )
}

#[derive(Clone, Debug, PartialEq)]
enum Known {
    /// Wanted, and not yet asked of the worker.
    Wanted,
    /// Asked: the request that answers.
    Asked(u64),
    Missing,
    Stale,
    Ready(Arc<Motion>),
    Failed(String),
}

struct Entry {
    /// The media the answer is about, as it was when it was wanted.
    source: SourceIdentity,
    known: Known,
    /// The frame it was last wanted in.
    used: u64,
}

/// The measuring of one video under way.
#[derive(Clone, Debug, PartialEq)]
pub struct Measuring {
    pub request: u64,
    pub asset: AssetId,
    source: SourceIdentity,
    pub done: f32,
}

/// What is known of a video's camera path, as the inspector says it.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Being read from the cache: nothing is known yet.
    Reading,
    NotAnalysed,
    /// Measured so far, `0..=1`.
    Analysing(f32),
    Ready {
        frames: usize,
        /// Frames whose motion could not be measured and are left as they
        /// are.
        unmeasured: u32,
    },
    /// A path is kept, of what the media was before it changed.
    Stale,
    Failed(String),
}

/// What became of an answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Taken {
    /// It is held now: the layers that wanted it are to be drawn again.
    Kept,
    /// It says there is no path, or none of this media.
    Noted,
    /// It could not be measured: why.
    Failed(String),
    /// It was measured of media that has since changed or gone.
    Changed,
    /// It is not for anything this holds.
    Dropped,
}

#[derive(Default)]
pub struct Steadies {
    entries: HashMap<AssetId, Entry>,
    wants: Vec<AssetId>,
    measuring: Option<Measuring>,
    frame: u64,
    /// The crop last suggested, and what it was suggested for.
    suggested: Option<(Suggestion, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
struct Suggestion {
    asset: AssetId,
    /// The path's own address: another path of the same asset is another.
    path: usize,
    from: Time,
    until: Time,
    smoothness: u32,
}

impl Steadies {
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Gives up everything: the project is another. What the worker still
    /// answers is then for nothing held here.
    pub fn clear(&mut self) {
        *self = Self {
            frame: self.frame,
            ..Self::default()
        };
    }

    /// How many paths are held or waited for.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Says the camera path of `asset` is wanted this frame, by a layer or
    /// by the inspector. One that is not held, or is held of what the media
    /// no longer is, is put on the list to ask the worker about.
    pub fn want(&mut self, asset: &Asset) {
        if asset.kind != MediaKind::Video {
            return;
        }
        let source = SourceIdentity::of(asset);
        if let Some(entry) = self.entries.get_mut(&asset.id)
            && entry.source == source
        {
            entry.used = self.frame;
            return;
        }
        self.entries.insert(
            asset.id,
            Entry {
                source,
                known: Known::Wanted,
                used: self.frame,
            },
        );
        if !self.wants.contains(&asset.id) {
            self.wants.push(asset.id);
        }
        self.make_room(asset.id);
    }

    // Holds the paths to their bounds: the least lately wanted go first, and
    // never the one just wanted.
    fn make_room(&mut self, keep: AssetId) {
        let frames = |entries: &HashMap<AssetId, Entry>| -> usize {
            (entries.values())
                .map(|entry| match &entry.known {
                    Known::Ready(motion) => motion.path.len(),
                    _ => 0,
                })
                .sum()
        };
        while self.entries.len() > MAX_STEADIES || frames(&self.entries) > MAX_STEADY_HELD_FRAMES {
            let oldest = (self.entries.iter())
                .filter(|(id, _)| **id != keep)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(id, _)| *id);
            let Some(oldest) = oldest else { break };
            self.entries.remove(&oldest);
            self.wants.retain(|id| *id != oldest);
        }
    }

    /// The assets to ask the worker about, each once.
    pub fn take_wants(&mut self) -> Vec<AssetId> {
        std::mem::take(&mut self.wants)
    }

    /// Says the worker took the question about `asset` as `request`.
    pub fn asked(&mut self, asset: AssetId, request: u64) {
        if let Some(entry) = self.entries.get_mut(&asset)
            && entry.known == Known::Wanted
        {
            entry.known = Known::Asked(request);
        }
    }

    /// Says the question about `asset` could not be put: it is wanted again
    /// by whoever next draws it.
    pub fn unasked(&mut self, asset: AssetId) {
        if (self.entries.get(&asset)).is_some_and(|entry| entry.known == Known::Wanted) {
            self.entries.remove(&asset);
        }
    }

    /// Says the worker is measuring `asset` as `request`.
    pub fn begin(&mut self, asset: &Asset, request: u64) {
        self.measuring = Some(Measuring {
            request,
            asset: asset.id,
            source: SourceIdentity::of(asset),
            done: 0.0,
        });
    }

    pub fn measuring(&self) -> Option<&Measuring> {
        self.measuring.as_ref()
    }

    /// Gives up the measuring under way and hands back its request, for
    /// the worker to be told.
    pub fn cancel(&mut self) -> Option<u64> {
        self.measuring.take().map(|measuring| measuring.request)
    }

    pub fn progress(&mut self, request: u64, done: f32) {
        if let Some(measuring) = (self.measuring.as_mut()).filter(|m| m.request == request) {
            measuring.done = done.clamp(0.0, 1.0);
        }
    }

    /// Takes an answer of the worker. `instance` is the project that is
    /// open and `now` the asset as the project has it, if it still does.
    ///
    /// A measuring is taken when it is the one under way, asked by this
    /// project, of media that is still what was measured. A reading is
    /// taken when it answers the request last put for its asset. Anything
    /// else changes nothing.
    #[expect(
        clippy::too_many_arguments,
        reason = "what one answer carries, handed over as it is"
    )]
    pub fn take(
        &mut self,
        instance: Uuid,
        now: Option<&Asset>,
        request: u64,
        project: Uuid,
        asset_id: AssetId,
        source: &SourceIdentity,
        measured: bool,
        result: Result<Steadied, String>,
    ) -> Taken {
        let current = now.is_some_and(|asset| SourceIdentity::of(asset) == *source);
        if measured {
            if self.measuring.as_ref().map(|m| m.request) != Some(request) {
                return Taken::Dropped;
            }
            let measuring = self.measuring.take();
            if project != instance || measuring.is_none_or(|m| m.asset != asset_id) {
                return Taken::Dropped;
            }
            if !current {
                return Taken::Changed;
            }
        } else {
            let asked = (self.entries.get(&asset_id)).is_some_and(|entry| {
                entry.known == Known::Asked(request) && entry.source == *source
            });
            if !asked || project != instance {
                return Taken::Dropped;
            }
            if !current {
                // What it was asked about is no more: asked again when wanted.
                self.entries.remove(&asset_id);
                return Taken::Dropped;
            }
        }
        let (known, taken) = match result {
            Ok(Steadied::Ready(motion)) if motion.path.is_empty() => (
                Known::Failed("no frame was read".to_owned()),
                Taken::Failed("no frame was read".to_owned()),
            ),
            Ok(Steadied::Ready(motion)) => (Known::Ready(motion), Taken::Kept),
            Ok(Steadied::Stale) => (Known::Stale, Taken::Noted),
            Ok(Steadied::Missing) => (Known::Missing, Taken::Noted),
            Err(why) => (Known::Failed(why.clone()), Taken::Failed(why)),
        };
        // A measuring that failed leaves what was held of the media: a path
        // that is good stays good.
        let holds = (self.entries.get(&asset_id))
            .is_some_and(|entry| entry.source == *source && matches!(entry.known, Known::Ready(_)));
        if measured && holds && matches!(known, Known::Failed(_)) {
            return taken;
        }
        self.entries.insert(
            asset_id,
            Entry {
                source: source.clone(),
                known,
                used: self.frame,
            },
        );
        self.wants.retain(|id| *id != asset_id);
        self.make_room(asset_id);
        taken
    }

    fn ready(&self, asset: &Asset) -> Option<&Arc<Motion>> {
        let entry = self.entries.get(&asset.id)?;
        match &entry.known {
            Known::Ready(motion) if entry.source == SourceIdentity::of(asset) => Some(motion),
            _ => None,
        }
    }

    /// What is known of the camera path of `asset` as it is now.
    pub fn status(&self, asset: &Asset) -> Status {
        if let Some(measuring) = (self.measuring.as_ref()).filter(|m| m.asset == asset.id) {
            return Status::Analysing(measuring.done);
        }
        let Some(entry) = self.entries.get(&asset.id) else {
            return Status::Reading;
        };
        if entry.source != SourceIdentity::of(asset) {
            return Status::Reading;
        }
        match &entry.known {
            Known::Wanted | Known::Asked(_) => Status::Reading,
            Known::Missing => Status::NotAnalysed,
            Known::Stale => Status::Stale,
            Known::Failed(why) => Status::Failed(why.clone()),
            Known::Ready(motion) => Status::Ready {
                frames: motion.path.len(),
                unmeasured: motion.unmeasured,
            },
        }
    }

    /// How the picture of `asset` decoded for the source time `at` is read
    /// to steady it: the correction of that frame at `smoothness`, as much
    /// of it as `mix` says and as a crop of `zoom` hides the borders of,
    /// with the crop. Nothing while the path is not held or is of what the
    /// media no longer is: the layer is then drawn as it is, uncropped.
    pub fn sampling(
        &self,
        asset: &Asset,
        at: Time,
        smoothness: f32,
        zoom: f32,
        mix: f32,
    ) -> Option<Sampling> {
        let motion = self.ready(asset)?;
        let aspect = asset.width.max(1) as f32 / asset.height.max(1) as f32;
        let zoom = zoom.clamp(1.0, 4.0);
        let fix = (motion.fix_at(at, smoothness))
            .scaled(mix.clamp(0.0, 1.0))
            .fitted(zoom, aspect);
        Some(fix.sampling(zoom, aspect))
    }

    /// The least crop that hides the borders of every correction of the
    /// source stretch `from..until` of `asset` at `smoothness`: see
    /// [`Motion::crop_zoom`]. It is worked out when what it is of changes,
    /// and remembered until then.
    pub fn suggested(
        &mut self,
        asset: &Asset,
        from: Time,
        until: Time,
        smoothness: f32,
    ) -> Option<f32> {
        let motion = Arc::clone(self.ready(asset)?);
        let of = Suggestion {
            asset: asset.id,
            path: Arc::as_ptr(&motion) as usize,
            from,
            until,
            smoothness: smoothness.to_bits(),
        };
        if let Some((held, zoom)) = &self.suggested
            && *held == of
        {
            return Some(*zoom);
        }
        let aspect = asset.width.max(1) as f32 / asset.height.max(1) as f32;
        let zoom = motion.crop_zoom(from, until, smoothness, aspect);
        self.suggested = Some((of, zoom));
        Some(zoom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::FrameRate;

    fn video(seconds: f64) -> Asset {
        let mut asset = Asset::new(
            "/media/shot.mkv".into(),
            MediaKind::Video,
            Time::from_seconds(seconds),
        );
        asset.width = 1920;
        asset.height = 1080;
        asset.frame_rate = Some(FrameRate::default());
        asset.file_size = 4321;
        asset
    }

    /// A path with one frame knocked a twentieth of the width aside.
    fn knocked(frames: usize) -> Arc<Motion> {
        let mut path = vec![[0.0; 4]; frames];
        path[frames / 2] = [0.05, 0.0, 0.0, 0.0];
        Arc::new(Motion {
            rate: FrameRate::default(),
            path,
            unmeasured: 1,
        })
    }

    const HERE: Uuid = Uuid::from_u128(1);
    const ELSEWHERE: Uuid = Uuid::from_u128(2);

    /// Wants `asset` and has the worker asked about it.
    fn asked(steadies: &mut Steadies, asset: &Asset, request: u64) {
        steadies.want(asset);
        assert_eq!(steadies.take_wants(), vec![asset.id]);
        steadies.asked(asset.id, request);
    }

    fn read(
        steadies: &mut Steadies,
        asset: &Asset,
        request: u64,
        result: Result<Steadied, String>,
    ) -> Taken {
        let source = SourceIdentity::of(asset);
        steadies.take(
            HERE,
            Some(asset),
            request,
            HERE,
            asset.id,
            &source,
            false,
            result,
        )
    }

    #[test]
    fn a_path_is_asked_for_once_and_says_what_is_known_of_it() {
        let mut steadies = Steadies::default();
        let asset = video(4.0);
        // Nothing is known until something wants it.
        assert_eq!(steadies.status(&asset), Status::Reading);
        assert!(
            steadies
                .sampling(&asset, Time::ZERO, 0.5, 1.1, 1.0)
                .is_none()
        );
        steadies.want(&asset);
        steadies.want(&asset);
        assert_eq!(steadies.take_wants(), vec![asset.id]);
        steadies.asked(asset.id, 7);
        // Wanted again while the answer is awaited, it is not asked again.
        steadies.begin_frame();
        steadies.want(&asset);
        assert!(steadies.take_wants().is_empty());
        assert_eq!(steadies.status(&asset), Status::Reading);

        assert_eq!(
            read(&mut steadies, &asset, 7, Ok(Steadied::Missing)),
            Taken::Noted
        );
        assert_eq!(steadies.status(&asset), Status::NotAnalysed);
        assert!(
            steadies
                .sampling(&asset, Time::ZERO, 0.5, 1.1, 1.0)
                .is_none()
        );
        // Known to be missing, it is not asked about every frame.
        steadies.want(&asset);
        assert!(steadies.take_wants().is_empty());

        // A still, a sound and a title have no camera.
        let mut still = video(4.0);
        still.kind = MediaKind::Image;
        steadies.want(&still);
        assert!(steadies.take_wants().is_empty());

        // A question that could not be put is put again.
        let other = video(2.0);
        steadies.want(&other);
        assert_eq!(steadies.take_wants(), vec![other.id]);
        steadies.unasked(other.id);
        steadies.want(&other);
        assert_eq!(steadies.take_wants(), vec![other.id]);
    }

    #[test]
    fn a_measuring_is_taken_only_for_this_project_this_request_and_this_media() {
        let asset = video(4.0);
        let source = SourceIdentity::of(&asset);
        let ready = || Ok(Steadied::Ready(knocked(120)));
        let begun = || {
            let mut steadies = Steadies::default();
            asked(&mut steadies, &asset, 1);
            assert_eq!(
                read(&mut steadies, &asset, 1, Ok(Steadied::Missing)),
                Taken::Noted
            );
            steadies.begin(&asset, 9);
            steadies.progress(9, 0.4);
            steadies.progress(8, 0.9);
            assert_eq!(steadies.status(&asset), Status::Analysing(0.4));
            steadies
        };

        // Another request's answer, and nothing under way.
        let mut steadies = begun();
        let taken = steadies.take(
            HERE,
            Some(&asset),
            8,
            HERE,
            asset.id,
            &source,
            true,
            ready(),
        );
        assert_eq!(taken, Taken::Dropped);
        assert_eq!(steadies.status(&asset), Status::Analysing(0.4));
        assert_eq!(steadies.cancel(), Some(9));
        let taken = steadies.take(
            HERE,
            Some(&asset),
            9,
            HERE,
            asset.id,
            &source,
            true,
            ready(),
        );
        assert_eq!(taken, Taken::Dropped);
        assert_eq!(steadies.status(&asset), Status::NotAnalysed);

        // Another project's: the one it was asked by was replaced.
        let mut steadies = begun();
        let taken = steadies.take(
            HERE,
            Some(&asset),
            9,
            ELSEWHERE,
            asset.id,
            &source,
            true,
            ready(),
        );
        assert_eq!(taken, Taken::Dropped);
        assert_eq!(steadies.status(&asset), Status::NotAnalysed);

        // The media changed while it was measured, or left the library.
        let mut relinked = asset.clone();
        relinked.file_size += 1;
        let mut steadies = begun();
        let taken = steadies.take(
            HERE,
            Some(&relinked),
            9,
            HERE,
            asset.id,
            &source,
            true,
            ready(),
        );
        assert_eq!(taken, Taken::Changed);
        assert!(
            steadies
                .sampling(&relinked, Time::ZERO, 0.5, 1.2, 1.0)
                .is_none()
        );
        let mut steadies = begun();
        let taken = steadies.take(HERE, None, 9, HERE, asset.id, &source, true, ready());
        assert_eq!(taken, Taken::Changed);

        // This one's.
        let mut steadies = begun();
        let taken = steadies.take(
            HERE,
            Some(&asset),
            9,
            HERE,
            asset.id,
            &source,
            true,
            ready(),
        );
        assert_eq!(taken, Taken::Kept);
        assert_eq!(
            steadies.status(&asset),
            Status::Ready {
                frames: 120,
                unmeasured: 1
            }
        );
        assert!(steadies.measuring().is_none());
        // Measured again and failed, the path that was good is kept.
        steadies.begin(&asset, 12);
        let failed = Err("ffmpeg: gone".to_owned());
        let taken = steadies.take(
            HERE,
            Some(&asset),
            12,
            HERE,
            asset.id,
            &source,
            true,
            failed,
        );
        assert_eq!(taken, Taken::Failed("ffmpeg: gone".to_owned()));
        assert!(matches!(steadies.status(&asset), Status::Ready { .. }));
    }

    #[test]
    fn a_reading_is_taken_only_as_the_answer_to_the_question_last_put() {
        let asset = video(4.0);
        let source = SourceIdentity::of(&asset);
        let ready = || Ok(Steadied::Ready(knocked(120)));
        let mut steadies = Steadies::default();
        asked(&mut steadies, &asset, 3);
        // Another request, another project.
        assert_eq!(read(&mut steadies, &asset, 2, ready()), Taken::Dropped);
        let taken = steadies.take(
            HERE,
            Some(&asset),
            3,
            ELSEWHERE,
            asset.id,
            &source,
            false,
            ready(),
        );
        assert_eq!(taken, Taken::Dropped);
        assert_eq!(steadies.status(&asset), Status::Reading);

        // The media was relinked while the file was read: the path read is
        // of what it was, and the media as it is now is asked about anew.
        let mut relinked = asset.clone();
        relinked.path = "/media/other.mkv".into();
        let taken = steadies.take(
            HERE,
            Some(&relinked),
            3,
            HERE,
            asset.id,
            &source,
            false,
            ready(),
        );
        assert_eq!(taken, Taken::Dropped);
        assert!(
            steadies
                .sampling(&relinked, Time::ZERO, 0.5, 1.2, 1.0)
                .is_none()
        );
        steadies.want(&relinked);
        assert_eq!(steadies.take_wants(), vec![asset.id]);
        steadies.asked(asset.id, 4);
        // The old answer comes late, after the new question.
        assert_eq!(read(&mut steadies, &asset, 3, ready()), Taken::Dropped);
        let taken = read(&mut steadies, &relinked, 4, Ok(Steadied::Stale));
        assert_eq!(taken, Taken::Noted);
        assert_eq!(steadies.status(&relinked), Status::Stale);

        // A path held is of the media it was read of: relinked back, the
        // other media's is not drawn with.
        asked(&mut steadies, &asset, 5);
        assert_eq!(read(&mut steadies, &asset, 5, ready()), Taken::Kept);
        assert!(
            steadies
                .sampling(&asset, Time::ZERO, 0.5, 1.2, 1.0)
                .is_some()
        );
        assert!(
            steadies
                .sampling(&relinked, Time::ZERO, 0.5, 1.2, 1.0)
                .is_none()
        );
        assert_eq!(steadies.status(&relinked), Status::Reading);

        // Another project: nothing is held, and what was asked answers
        // nothing.
        asked(&mut steadies, &relinked, 6);
        steadies.clear();
        assert_eq!(read(&mut steadies, &relinked, 6, ready()), Taken::Dropped);
        assert_eq!(steadies.len(), 0);
    }

    #[test]
    fn a_held_path_corrects_the_frame_it_is_asked_about() {
        let asset = video(4.0);
        let mut steadies = Steadies::default();
        asked(&mut steadies, &asset, 1);
        read(&mut steadies, &asset, 1, Ok(Steadied::Ready(knocked(120))));
        let rate = FrameRate::default();
        let at = |frame: i64, zoom: f32, mix: f32| {
            steadies
                .sampling(&asset, rate.frame_time(frame), 0.5, zoom, mix)
                .unwrap()
        };
        // The knocked frame is read where the knock took the picture: moved
        // back left, so read to the right. A still frame is cropped alone.
        let knocked = at(60, 1.2, 1.0);
        assert!((knocked.offset[0] - 0.047).abs() < 0.004, "{knocked:?}");
        assert!((knocked.matrix[0] - 1.0 / 1.2).abs() < 1e-6);
        let still = at(5, 1.2, 1.0);
        assert_eq!(still.offset, [0.0, 0.0]);
        assert!((still.matrix[0] - 1.0 / 1.2).abs() < 1e-6);
        // Half mixed, half corrected; too small a crop, as much as it hides.
        assert!((at(60, 1.2, 0.5).offset[0] - knocked.offset[0] * 0.5).abs() < 1e-4);
        let held = at(60, 1.05, 1.0);
        assert!(held.offset[0] > 0.02 && held.offset[0] < 0.025, "{held:?}");
        assert_eq!(at(60, 1.0, 1.0), Sampling::IDENTITY);

        // The crop suggested hides the knock, and is worked out once.
        let span = (Time::ZERO, Time::from_seconds(4.0));
        let zoom = steadies.suggested(&asset, span.0, span.1, 0.5).unwrap();
        assert!((1.09..=1.11).contains(&zoom), "{zoom}");
        assert_eq!(steadies.suggested(&asset, span.0, span.1, 0.5), Some(zoom));
        // A stretch without the knock needs next to none.
        let calm = steadies
            .suggested(&asset, Time::ZERO, Time::from_seconds(0.5), 0.5)
            .unwrap();
        assert!(calm < 1.02, "{calm}");
        let other = video(1.0);
        assert_eq!(steadies.suggested(&other, span.0, span.1, 0.5), None);
    }

    #[test]
    fn the_paths_held_are_bounded_and_the_least_lately_wanted_makes_room() {
        let mut steadies = Steadies::default();
        let assets: Vec<Asset> = (0..MAX_STEADIES + 2).map(|_| video(4.0)).collect();
        for (index, asset) in assets.iter().enumerate() {
            steadies.begin_frame();
            asked(&mut steadies, asset, index as u64);
            read(
                &mut steadies,
                asset,
                index as u64,
                Ok(Steadied::Ready(knocked(30))),
            );
            // The first is wanted in every frame.
            steadies.want(&assets[0]);
        }
        assert_eq!(steadies.len(), MAX_STEADIES);
        assert!(matches!(steadies.status(&assets[0]), Status::Ready { .. }));
        assert_eq!(steadies.status(&assets[1]), Status::Reading);
        assert_eq!(steadies.status(&assets[2]), Status::Reading);
        assert!(matches!(steadies.status(&assets[3]), Status::Ready { .. }));

        // And by their frames: two long paths do not fit beside each other.
        let mut steadies = Steadies::default();
        for (index, asset) in assets[..3].iter().enumerate() {
            steadies.begin_frame();
            asked(&mut steadies, asset, index as u64);
            let long = Ok(Steadied::Ready(knocked(MAX_STEADY_HELD_FRAMES / 2 + 1)));
            read(&mut steadies, asset, index as u64, long);
        }
        assert_eq!(steadies.len(), 1);
        assert!(matches!(steadies.status(&assets[2]), Status::Ready { .. }));
    }
}
