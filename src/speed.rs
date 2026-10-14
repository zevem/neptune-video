//! A clip's speed as a person edits it.
//!
//! The project keeps a time map: where in the source each moment of the clip
//! is. A person thinks of how fast the clip runs, so the interface reads the
//! map as a [`Curve`] of speed over the clip, changes that, and writes it back
//! as a map through `SetRetime`. Nothing here is kept: a curve is read from
//! the clip whenever it is wanted.
//!
//! A stretch whose speed changes evenly is a quadratic in the source, which a
//! cubic easing with its handles a third along holds exactly; one that eases
//! between its speeds has them half way along, where the speed's own rate of
//! change is nought at both ends. Any other easing the map came with is kept
//! as it is until its speeds are changed.
use editor_core::{
    Clip, ClipContent, ClipId, Command, Ease, MediaKind, Project, Retime, Time, TimeKey, Track,
    TrackKind, retime::MAX_TIME_KEYS,
};
use std::borrow::Cow;

/// The speeds a clip may be played at, as the project holds them.
pub const SLOWEST: f64 = 0.05;
pub const FASTEST: f64 = 16.0;
/// The constant speeds offered by name.
pub const RATES: [f64; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];
/// How long a frame is frozen for when nothing else is asked.
pub const HOLD: Time = Time(2_000_000);
/// The longest a frame is frozen for in one step.
pub const LONGEST_HOLD: Time = Time(60_000_000);
/// Speeds this near are one speed: a stretch between them is not a ramp.
const SAME: f64 = 0.004;
/// The least of the clip a stretch keeps when a point beside it is moved or
/// one is put into it.
const GAP: Time = Time(20_000);

/// How the speed gets from one end of a stretch to the other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Evenly.
    Linear,
    /// Leaving and arriving gently.
    Smooth,
    /// As the map it was read from eases it.
    Custom(Ease),
}

/// What a stretch's easing is called where it is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Constant,
    Linear,
    Smooth,
}

impl Easing {
    pub const ALL: [Self; 3] = [Self::Constant, Self::Linear, Self::Smooth];

    pub fn label(self) -> &'static str {
        match self {
            Self::Constant => "Constant",
            Self::Linear => "Linear",
            Self::Smooth => "Smooth",
        }
    }
}

/// One stretch of a clip between two points of its speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    /// The source it covers, in microseconds: negative backwards, and nought
    /// for a frame that is held.
    pub moved: i64,
    /// How long it lasts on the timeline.
    pub span: Time,
    /// The speeds it begins and ends at, never negative.
    pub from: f64,
    pub to: f64,
    pub shape: Shape,
}

impl Piece {
    pub fn frozen(&self) -> bool {
        self.moved == 0
    }

    /// Whether it runs at one speed.
    pub fn flat(&self) -> bool {
        (self.from - self.to).abs() <= SAME
    }

    fn mean(&self) -> f64 {
        (self.from + self.to) * 0.5
    }

    /// Its length for the speeds it has now.
    fn settle(&mut self) {
        if !self.frozen() {
            let span = self.moved.abs() as f64 / self.mean().max(1e-9);
            self.span = Time((span.round() as i64).max(1));
        }
    }

    /// Takes new speeds, and with them a shape that can hold any.
    fn reshape(&mut self) {
        if matches!(self.shape, Shape::Custom(_)) {
            self.shape = Shape::Smooth;
        }
    }

    pub fn easing(&self) -> Option<Easing> {
        match self.shape {
            _ if self.frozen() => None,
            Shape::Custom(_) => None,
            _ if self.flat() => Some(Easing::Constant),
            Shape::Linear => Some(Easing::Linear),
            Shape::Smooth => Some(Easing::Smooth),
        }
    }

    fn ease(&self) -> Ease {
        let handles = |along: f32| {
            let mean = self.mean().max(1e-9);
            let (leaving, arriving) = (self.from / mean, self.to / mean);
            Ease::Bezier {
                x1: along,
                y1: (leaving as f32 * along).clamp(0.0, 1.0),
                x2: 1.0 - along,
                y2: (1.0 - arriving as f32 * along).clamp(0.0, 1.0),
            }
        };
        match self.shape {
            Shape::Custom(ease) => ease,
            _ if self.frozen() || self.flat() => Ease::Linear,
            Shape::Linear => handles(1.0 / 3.0),
            Shape::Smooth => handles(0.5),
        }
    }

    fn read(from: &TimeKey, to: &TimeKey) -> Self {
        let span = to.at - from.at;
        let moved = to.source.0 - from.source.0;
        let mean = moved.abs() as f64 / span.0.max(1) as f64;
        let flat = |shape| Self {
            moved,
            span,
            from: mean,
            to: mean,
            shape,
        };
        if moved == 0 {
            return flat(Shape::Smooth);
        }
        match from.ease {
            Ease::Linear => flat(Shape::Smooth),
            Ease::Hold => Self {
                from: 0.0,
                to: 0.0,
                ..flat(Shape::Custom(Ease::Hold))
            },
            ease @ Ease::Bezier { x1, y1, x2, y2 } => {
                // Handles a third or a half along are a ramp this module
                // writes, when the two speeds they give have the mean the
                // stretch has.
                let near = |a: f32, b: f32| (a - b).abs() < 0.01;
                let ramp = |along: f32, shape| {
                    let (leaving, arriving) = (y1 / along, (1.0 - y2) / along);
                    (near(x1, along) && near(x2, 1.0 - along) && near(leaving + arriving, 2.0))
                        .then(|| Self {
                            from: mean * f64::from(leaving),
                            to: mean * f64::from(arriving),
                            ..flat(shape)
                        })
                };
                ramp(1.0 / 3.0, Shape::Linear)
                    .or_else(|| ramp(0.5, Shape::Smooth))
                    .unwrap_or_else(|| Self {
                        from: mean * ease.slope(0.0),
                        to: mean * ease.slope(1.0),
                        ..flat(Shape::Custom(ease))
                    })
            }
        }
    }
}

/// A clip's speed: its stretches in order, from the source offset it begins
/// at. A point of the curve is where one stretch ends and the next begins,
/// and the two ends of the clip are points too.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    /// The source the clip begins at, from its `source_in`.
    pub start: Time,
    pub pieces: Vec<Piece>,
}

impl Curve {
    /// The curve a clip plays: its map's, or one stretch at its rate.
    pub fn of(clip: &Clip) -> Self {
        match &clip.retime {
            Some(retime) => Self::of_map(retime),
            None => Self {
                start: Time::ZERO,
                pieces: vec![Piece {
                    moved: clip.duration.scaled(clip.playback_rate).0,
                    span: clip.duration,
                    from: clip.playback_rate,
                    to: clip.playback_rate,
                    shape: Shape::Smooth,
                }],
            },
        }
    }

    pub fn of_map(retime: &Retime) -> Self {
        Self {
            start: retime.keys.first().map_or(Time::ZERO, |key| key.source),
            pieces: (retime.keys.windows(2))
                .map(|pair| Piece::read(&pair[0], &pair[1]))
                .collect(),
        }
    }

    /// How long the clip lasts.
    pub fn duration(&self) -> Time {
        Time(self.pieces.iter().map(|piece| piece.span.0).sum())
    }

    /// The time map this is, and the clip's length with it.
    pub fn retime(&self) -> (Retime, Time) {
        let mut keys = Vec::with_capacity(self.pieces.len() + 1);
        let (mut at, mut source) = (Time::ZERO, self.start);
        for piece in &self.pieces {
            keys.push(TimeKey {
                at,
                source,
                ease: piece.ease(),
            });
            at = at + piece.span;
            source = Time(source.0 + piece.moved);
        }
        keys.push(TimeKey {
            at,
            source,
            ease: Ease::Linear,
        });
        (Retime { keys }, at)
    }

    /// The map, when the project would take it for a clip of its own.
    pub fn valid(&self) -> Option<(Retime, Time)> {
        let (retime, duration) = self.retime();
        (duration.is_valid() && retime.validate(duration)).then_some((retime, duration))
    }

    /// How many points the curve has.
    pub fn points(&self) -> usize {
        self.pieces.len() + 1
    }

    /// When in the clip a point is.
    pub fn at(&self, point: usize) -> Time {
        Time(
            (self.pieces.iter().take(point))
                .map(|piece| piece.span.0)
                .sum(),
        )
    }

    /// The speed at a point: that of the stretch it begins, or of the one it
    /// ends when it begins none that moves.
    pub fn speed(&self, point: usize) -> f64 {
        let moving = |index: usize| self.pieces.get(index).filter(|piece| !piece.frozen());
        match (moving(point), point.checked_sub(1).and_then(moving)) {
            (Some(next), _) => next.from,
            (None, Some(previous)) => previous.to,
            (None, None) => 0.0,
        }
    }

    /// The stretch a point's easing is chosen for: the one that leaves it,
    /// or the last one for the clip's end.
    pub fn piece_of(&self, point: usize) -> Option<usize> {
        let last = self.pieces.len().checked_sub(1)?;
        Some(point.min(last))
    }

    /// Gives a point a speed. The stretches on both sides take it at that
    /// end and last as long as their new speeds make them.
    pub fn set_speed(&mut self, point: usize, speed: f64) {
        let speed = if speed.is_finite() {
            speed.clamp(0.0, FASTEST)
        } else {
            return;
        };
        if let Some(next) = (self.pieces.get_mut(point)).filter(|piece| !piece.frozen()) {
            next.reshape();
            next.from = speed;
            next.settle();
        }
        if let Some(previous) = (point.checked_sub(1))
            .and_then(|index| self.pieces.get_mut(index))
            .filter(|piece| !piece.frozen())
        {
            previous.reshape();
            previous.to = speed;
            previous.settle();
        }
    }

    /// Moves a point towards `to` in the clip, by giving one of its
    /// stretches source from the other. The end of a held frame moves by
    /// holding it longer or shorter. The clip's two ends stay.
    pub fn slide(&mut self, point: usize, to: Time) {
        if point == 0 || point >= self.pieces.len() {
            return;
        }
        let by = to - self.at(point);
        let (before, after) = self.pieces.split_at_mut(point);
        let (previous, next) = (&mut before[point - 1], &mut after[0]);
        if previous.frozen() {
            previous.span = (previous.span + by).clamp(GAP, LONGEST_HOLD);
            return;
        }
        if next.frozen() || (previous.moved < 0) != (next.moved < 0) {
            return;
        }
        let direction = previous.moved.signum();
        let speed = previous.to.max(next.from).max(SLOWEST);
        // Each stretch keeps a little of the clip.
        let least = |piece: &Piece| (piece.mean().max(SLOWEST) * GAP.0 as f64).ceil() as i64;
        let (low, high) = (
            least(previous) - previous.moved.abs(),
            next.moved.abs() - least(next),
        );
        if low > high {
            return;
        }
        let given = ((by.0 as f64 * speed).round() as i64).clamp(low, high);
        previous.moved += direction * given;
        next.moved -= direction * given;
        previous.settle();
        next.settle();
    }

    /// Makes a stretch run at the speed it begins at, or ramp as named.
    pub fn set_easing(&mut self, piece: usize, easing: Easing) {
        let Some(piece) = (self.pieces.get_mut(piece)).filter(|piece| !piece.frozen()) else {
            return;
        };
        match easing {
            Easing::Constant => {
                piece.reshape();
                piece.to = piece.from;
            }
            Easing::Linear => piece.shape = Shape::Linear,
            Easing::Smooth => piece.shape = Shape::Smooth,
        }
        piece.settle();
    }

    /// The stretch `at` is inside, with a gap to spare on both sides.
    fn inside(&self, at: Time) -> Option<(usize, Time)> {
        let mut begins = Time::ZERO;
        for (index, piece) in self.pieces.iter().enumerate() {
            let ends = begins + piece.span;
            if at >= begins + GAP && at + GAP <= ends {
                return Some((index, begins));
            }
            begins = ends;
        }
        None
    }

    /// Puts a point on the curve at `at`, where the clip plays as it did.
    /// Hands back the point, when there is room for one there.
    pub fn insert(&mut self, at: Time) -> Option<usize> {
        if self.points() >= MAX_TIME_KEYS {
            return None;
        }
        let (index, _) = self.inside(at)?;
        let piece = self.pieces[index];
        if piece.frozen() {
            return None;
        }
        let (retime, _) = self.retime();
        let begins = retime.keys[index].source;
        let covered = retime.source_at(at).0 - begins.0;
        let rest = piece.moved - covered;
        if covered == 0 || rest == 0 || (covered < 0) != (rest < 0) {
            return None;
        }
        let speed = retime.speed_at(at).abs();
        // The halves of an even ramp are even ramps; of any other, ramps
        // that meet at the speed it had there.
        let shape = match piece.shape {
            Shape::Linear => Shape::Linear,
            _ => Shape::Smooth,
        };
        let mut first = Piece {
            moved: covered,
            to: speed,
            shape,
            ..piece
        };
        let mut second = Piece {
            moved: rest,
            from: speed,
            shape,
            ..piece
        };
        first.settle();
        second.settle();
        self.pieces.splice(index..=index, [first, second]);
        Some(index + 1)
    }

    /// Takes a point away: the stretches beside it become one, and a held
    /// frame beside it is let go. An end of the clip is not a point to take.
    pub fn remove(&mut self, point: usize) -> bool {
        if point == 0 || point >= self.pieces.len() {
            return false;
        }
        let (previous, next) = (self.pieces[point - 1], self.pieces[point]);
        if next.frozen() {
            self.pieces.remove(point);
        } else if previous.frozen() {
            self.pieces.remove(point - 1);
        } else if (previous.moved < 0) == (next.moved < 0) {
            let mut joined = Piece {
                moved: previous.moved + next.moved,
                to: next.to,
                shape: match (previous.shape, next.shape) {
                    (Shape::Linear, Shape::Linear) => Shape::Linear,
                    _ => Shape::Smooth,
                },
                ..previous
            };
            joined.settle();
            self.pieces.splice(point - 1..=point, [joined]);
        } else {
            return false;
        }
        // Two held frames that have come together are one.
        self.pieces.dedup_by(|later, earlier| {
            let both = later.frozen() && earlier.frozen();
            if both {
                earlier.span = earlier.span + later.span;
            }
            both
        });
        !self.pieces.is_empty()
    }

    /// The same clip played from its end to its beginning.
    pub fn reverse(&mut self) {
        let end = self.start.0 + self.pieces.iter().map(|piece| piece.moved).sum::<i64>();
        self.start = Time(end);
        self.pieces.reverse();
        for piece in &mut self.pieces {
            piece.moved = -piece.moved;
            std::mem::swap(&mut piece.from, &mut piece.to);
            if let Shape::Custom(Ease::Bezier { x1, y1, x2, y2 }) = piece.shape {
                piece.shape = Shape::Custom(Ease::Bezier {
                    x1: 1.0 - x2,
                    y1: 1.0 - y2,
                    x2: 1.0 - x1,
                    y2: 1.0 - y1,
                });
            }
        }
    }

    /// Whether everything that moves in the clip moves backwards.
    pub fn reversed(&self) -> bool {
        let mut moving = self
            .pieces
            .iter()
            .filter(|piece| !piece.frozen())
            .peekable();
        moving.peek().is_some() && moving.all(|piece| piece.moved < 0)
    }

    /// Holds the frame shown at `at` for `hold`, and the rest of the clip
    /// follows it.
    pub fn freeze(&mut self, at: Time, hold: Time) -> bool {
        let held = Piece {
            moved: 0,
            span: hold,
            from: 0.0,
            to: 0.0,
            shape: Shape::Smooth,
        };
        let at = at.clamp(Time::ZERO, self.duration());
        // A frame already held there is held longer.
        let mut begins = Time::ZERO;
        for piece in &mut self.pieces {
            let ends = begins + piece.span;
            if piece.frozen() && at >= begins && at <= ends {
                piece.span = piece.span + hold;
                return true;
            }
            begins = ends;
        }
        if self.points() >= MAX_TIME_KEYS {
            return false;
        }
        let point = match self.inside(at) {
            Some(_) => match self.insert(at) {
                Some(point) => point,
                None => return false,
            },
            // Next to a point there is no stretch to cut: the frame is held
            // at the point.
            None => (0..self.points())
                .min_by_key(|point| (self.at(*point).0 - at.0).abs())
                .unwrap_or(0),
        };
        self.pieces.insert(point, held);
        true
    }

    /// The speed the clip has on the whole where it moves.
    pub fn mean(&self) -> Option<f64> {
        let moving = || self.pieces.iter().filter(|piece| !piece.frozen());
        let moved: i64 = moving().map(|piece| piece.moved.abs()).sum();
        let span: i64 = moving().map(|piece| piece.span.0).sum();
        (span > 0).then(|| moved as f64 / span as f64)
    }

    /// The one speed the whole clip plays at, either way, when it has one.
    pub fn steady(&self) -> Option<f64> {
        match self.pieces.as_slice() {
            [piece] if piece.moved != 0 && piece.flat() && piece.ease() == Ease::Linear => {
                Some(piece.mean())
            }
            _ => None,
        }
    }

    /// Whether some of the clip plays no sound: the preview plays a map's
    /// sound only where it runs forwards at one speed.
    pub fn silent(&self) -> bool {
        (self.pieces.iter()).any(|piece| piece.moved <= 0 || piece.ease() != Ease::Linear)
    }

    /// The lowest source the clip reaches, and how much source it spans.
    fn extent(&self) -> (Time, i64) {
        let mut source = self.start.0;
        let (mut low, mut high) = (source, source);
        for piece in &self.pieces {
            source += piece.moved;
            low = low.min(source);
            high = high.max(source);
        }
        (Time(low), high - low)
    }
}

/// A speed ramp offered by name: speeds at places along the clip's source,
/// eased between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ramp {
    Up,
    Down,
    Montage,
    Hero,
    Bullet,
    FlashIn,
    FlashOut,
}

impl Ramp {
    pub const ALL: [Self; 7] = [
        Self::Up,
        Self::Down,
        Self::Montage,
        Self::Hero,
        Self::Bullet,
        Self::FlashIn,
        Self::FlashOut,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Up => "Ramp up",
            Self::Down => "Ramp down",
            Self::Montage => "Montage",
            Self::Hero => "Hero",
            Self::Bullet => "Bullet",
            Self::FlashIn => "Flash in",
            Self::FlashOut => "Flash out",
        }
    }

    /// The ramp's points: how far along the source, and the speed there.
    pub fn points(self) -> &'static [(f64, f64)] {
        match self {
            Self::Up => &[(0.0, 0.5), (1.0, 2.5)],
            Self::Down => &[(0.0, 2.5), (1.0, 0.5)],
            Self::Montage => &[(0.0, 3.0), (0.25, 0.5), (0.5, 3.0), (0.75, 0.5), (1.0, 3.0)],
            Self::Hero => &[(0.0, 3.0), (0.35, 0.4), (0.65, 0.4), (1.0, 3.0)],
            Self::Bullet => &[
                (0.0, 4.0),
                (0.42, 4.0),
                (0.5, 0.25),
                (0.58, 4.0),
                (1.0, 4.0),
            ],
            Self::FlashIn => &[(0.0, 4.0), (0.3, 1.0), (1.0, 1.0)],
            Self::FlashOut => &[(0.0, 1.0), (0.7, 1.0), (1.0, 4.0)],
        }
    }

    /// The ramp over the source `curve` spans, in the direction it plays.
    pub fn over(self, curve: &Curve) -> Curve {
        let (start, total) = curve.extent();
        let place = |along: f64| (total as f64 * along).round() as i64;
        let pieces = (self.points().windows(2))
            .filter_map(|pair| {
                let mut piece = Piece {
                    moved: place(pair[1].0) - place(pair[0].0),
                    span: Time::ZERO,
                    from: pair[0].1,
                    to: pair[1].1,
                    shape: Shape::Smooth,
                };
                piece.settle();
                (!piece.frozen()).then_some(piece)
            })
            .collect();
        let mut ramp = Curve { start, pieces };
        if curve.reversed() {
            ramp.reverse();
        }
        ramp
    }
}

/// Whether a clip has a speed to edit: it is cut from media that runs, and
/// is not a sound on an audio track.
pub fn timed(project: &Project, track: &Track, clip: &Clip) -> bool {
    track.kind != TrackKind::Audio
        && match &clip.content {
            ClipContent::Media { asset_id } => (project.assets.get(asset_id))
                .is_some_and(|asset| matches!(asset.kind, MediaKind::Video | MediaKind::Gif)),
            ClipContent::Text { .. } | ClipContent::Graphic { .. } => false,
        }
}

/// How long `clip` may become before it reaches the next clip of its track.
pub fn room(track: &Track, clip: &Clip) -> Option<Time> {
    (track.clips.iter())
        .filter(|other| other.id != clip.id && other.start >= clip.end())
        .map(|other| other.start - clip.start)
        .min()
}

/// The commands that give `clip` this map. Where the next clip is in the way
/// of a longer one it is cut to the room there is, as a slower clip is.
pub fn retime_commands(
    track: &Track,
    clip: &Clip,
    retime: &Retime,
    duration: Time,
) -> Vec<Command> {
    // `SetRetime` reads a map in the clip's own rate and bakes that in.
    let mut retime = retime.clone();
    if clip.retime.is_none() && clip.playback_rate != 1.0 {
        for key in &mut retime.keys {
            key.source = key.source.scaled(1.0 / clip.playback_rate);
        }
    }
    let mut commands = vec![Command::SetRetime {
        clip_id: clip.id,
        retime: Some(retime),
        duration,
    }];
    commands.extend(fitted(track, clip, duration));
    commands
}

/// The trim that keeps a clip of `duration` off the next one, when it is
/// needed.
fn fitted(track: &Track, clip: &Clip, duration: Time) -> Option<Command> {
    let room = room(track, clip).filter(|room| *room < duration)?;
    Some(Command::TrimClip {
        clip_id: clip.id,
        source_in: clip.source_in,
        duration: room,
        ripple: false,
    })
}

fn batch(label: &str, commands: Vec<Command>) -> Command {
    Command::Batch {
        label: label.to_owned(),
        commands,
    }
}

/// One undo step that makes `clip` play `curve`; none when the project would
/// not take the map.
pub fn curve_command(
    project: &Project,
    clip_id: ClipId,
    curve: &Curve,
    label: &str,
) -> Option<Command> {
    let (track, clip) = project.clip(clip_id)?;
    let (retime, duration) = curve.valid()?;
    Some(batch(
        label,
        retime_commands(track, clip, &retime, duration),
    ))
}

/// One undo step that plays all the source `clip` shows at `rate`, whatever
/// its speed was: backwards if all of it played backwards, forwards
/// otherwise. A map that can be let go is: the clip is then cut from its
/// source as any other.
pub fn constant_command(project: &Project, clip_id: ClipId, rate: f64) -> Option<Command> {
    let (track, clip) = project.clip(clip_id)?;
    let rate = rate.clamp(SLOWEST, FASTEST);
    let curve = Curve::of(clip);
    let (lowest, total) = curve.extent();
    let backwards = curve.reversed();
    let duration = Time(((total as f64 / rate).round() as i64).max(1));
    let mut commands = match &clip.retime {
        None => vec![Command::SetPlaybackRate {
            clip_id: clip.id,
            rate,
        }],
        Some(_) if lowest == Time::ZERO && !backwards => vec![
            Command::SetRetime {
                clip_id: clip.id,
                retime: None,
                duration: Time(total.max(1)),
            },
            Command::SetPlaybackRate {
                clip_id: clip.id,
                rate,
            },
        ],
        // The source does not begin where the clip's does, or runs
        // backwards: a straight map keeps the place and the way.
        Some(_) => {
            let key = |at, source| TimeKey {
                at,
                source,
                ease: Ease::Linear,
            };
            let highest = Time(lowest.0 + total);
            vec![Command::SetRetime {
                clip_id: clip.id,
                retime: Some(Retime {
                    keys: if backwards {
                        vec![key(Time::ZERO, highest), key(duration, lowest)]
                    } else {
                        vec![key(Time::ZERO, lowest), key(duration, highest)]
                    },
                }),
                duration,
            }]
        }
    };
    commands.extend(fitted(track, clip, duration));
    Some(batch("Change speed", commands))
}

/// One undo step that holds the frame `clip` shows at `playhead` for `hold`.
/// Says why not when there is no such frame.
pub fn freeze_command(
    project: &Project,
    clip_id: ClipId,
    playhead: Time,
    hold: Time,
) -> Result<Command, String> {
    let (track, clip) = (project.clip(clip_id)).ok_or("That clip is no longer in the project.")?;
    if !timed(project, track, clip) {
        return Err("Only a video clip has a frame to freeze.".to_owned());
    }
    if playhead < clip.start || playhead > clip.end() {
        return Err("Put the playhead on the clip to freeze a frame of it.".to_owned());
    }
    // The frame the canvas shows there, held for whole frames.
    let rate = project.settings.frame_rate;
    let playhead = crate::rules::quantize(playhead, rate).clamp(clip.start, clip.end());
    let hold = crate::rules::quantize(hold, rate).clamp(crate::rules::frame(rate), LONGEST_HOLD);
    let mut curve = Curve::of(clip);
    if !curve.freeze(playhead - clip.start, hold) {
        return Err("This clip's speed has no room for another point.".to_owned());
    }
    curve_command(project, clip_id, &curve, "Freeze frame")
        .ok_or_else(|| "A frame cannot be frozen there.".to_owned())
}

/// The clip a frame is frozen in when no clip was pointed at: the selected
/// one, or else the topmost one under the playhead that is not locked.
pub fn freeze_target(
    project: &Project,
    selected: Option<ClipId>,
    playhead: Time,
) -> Option<ClipId> {
    let under = |clip: &Clip| clip.start <= playhead && playhead <= clip.end();
    if let Some((track, clip)) = selected.and_then(|id| project.clip(id))
        && timed(project, track, clip)
        && under(clip)
    {
        return Some(clip.id);
    }
    (project.tracks.iter().rev())
        .filter(|track| !track.locked)
        .flat_map(|track| track.clips.iter().map(move |clip| (track, clip)))
        .find(|(track, clip)| timed(project, track, clip) && under(clip))
        .map(|(_, clip)| clip.id)
}

/// A speed as it is written: `2×`, `0.5×`, `1.25×`.
pub fn times(speed: f64) -> String {
    let text = format!("{speed:.2}");
    format!("{}×", text.trim_end_matches('0').trim_end_matches('.'))
}

/// What a clip's tile says of its speed; nothing for one that plays as it
/// was shot.
pub fn badge(clip: &Clip) -> Option<Cow<'static, str>> {
    let Some(retime) = &clip.retime else {
        return (clip.playback_rate != 1.0).then(|| times(clip.playback_rate).into());
    };
    let straight = |pair: &[TimeKey]| pair[0].ease == Ease::Linear;
    let speed = |pair: &[TimeKey]| {
        (pair[1].source.0 - pair[0].source.0) as f64 / (pair[1].at.0 - pair[0].at.0).max(1) as f64
    };
    let mut segments = retime.keys.windows(2);
    let first = segments.next()?;
    let one = speed(first);
    if !straight(first) || segments.any(|pair| !straight(pair) || (speed(pair) - one).abs() > SAME)
    {
        let all = |test: &dyn Fn(&[TimeKey]) -> bool| retime.keys.windows(2).all(test);
        return Some(
            // A frame held in a clip that otherwise plays as it was shot.
            if all(&|pair| {
                straight(pair) && (speed(pair) == 0.0 || (speed(pair) - 1.0).abs() <= SAME)
            }) {
                "Freeze"
            } else if all(&straight) {
                // Straight stretches at speeds of their own, or both ways.
                "Retimed"
            } else {
                "Ramp"
            }
            .into(),
        );
    }
    Some(if one < 0.0 && (one + 1.0).abs() <= SAME {
        "Reversed".into()
    } else if one < 0.0 {
        format!("Reversed {}", times(-one)).into()
    } else if one == 0.0 {
        "Freeze".into()
    } else if (one - 1.0).abs() <= SAME {
        return None;
    } else {
        times(one).into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::{Asset, Editor, FrameRate};
    use std::{path::PathBuf, sync::Arc};

    fn seconds(value: f64) -> Time {
        Time::from_seconds(value)
    }

    /// A four second clip from the start of a ten second video, at `start`.
    fn shot(start: f64) -> (Project, ClipId) {
        let mut asset = Asset::new(
            PathBuf::from("/media/shot.mp4"),
            MediaKind::Video,
            seconds(10.0),
        );
        asset.frame_rate = Some(FrameRate::default());
        let mut clip = Clip::from_asset(&asset, seconds(start));
        clip.duration = seconds(4.0);
        let id = clip.id;
        let mut project = Project::new("Speed");
        Arc::make_mut(&mut project.assets).insert(asset.id, asset);
        Arc::make_mut(&mut project.tracks[0]).clips.push(clip);
        project.validate().unwrap();
        (project, id)
    }

    fn clip(project: &Project, id: ClipId) -> &Clip {
        project.clip(id).unwrap().1
    }

    fn applied(project: &Project, command: Command) -> Editor {
        let mut editor = Editor::new(project.clone()).unwrap();
        editor.dispatch(command).unwrap();
        editor
    }

    #[test]
    fn a_curve_read_from_a_map_writes_the_same_map() {
        let (project, id) = shot(0.0);
        let mut curve = Curve::of(clip(&project, id));
        assert_eq!(curve.steady(), Some(1.0));
        curve.insert(seconds(2.0)).unwrap();
        curve.set_speed(1, 3.0);
        curve.set_easing(0, Easing::Linear);
        curve.freeze(seconds(0.5), seconds(1.0));
        let (retime, duration) = curve.valid().unwrap();
        let read = Curve::of_map(&retime);
        assert_eq!(read.retime(), (retime, duration));
        let shapes: Vec<_> = read.pieces.iter().map(Piece::easing).collect();
        assert_eq!(
            shapes,
            [
                Some(Easing::Linear),
                None,
                Some(Easing::Linear),
                Some(Easing::Smooth)
            ]
        );
        for (read, made) in read.pieces.iter().zip(&curve.pieces) {
            assert!((read.from - made.from).abs() < 1e-3 && (read.to - made.to).abs() < 1e-3);
        }
    }

    #[test]
    fn an_even_ramp_is_the_speed_it_says_at_every_moment() {
        let (project, id) = shot(0.0);
        let mut curve = Curve::of(clip(&project, id));
        curve.set_speed(0, 0.5);
        curve.set_speed(1, 2.0);
        curve.set_easing(0, Easing::Linear);
        // Four seconds of source at a mean of 1.25.
        let (retime, duration) = curve.valid().unwrap();
        assert_eq!(duration, seconds(3.2));
        for step in 1..8 {
            let at = Time(duration.0 * step / 8);
            let wanted = 0.5 + 1.5 * step as f64 / 8.0;
            assert!((retime.speed_at(at) - wanted).abs() < 0.02, "{step}");
        }
        // Eased, it leaves and arrives at those speeds without a jolt.
        curve.set_easing(0, Easing::Smooth);
        let (retime, duration) = curve.valid().unwrap();
        assert_eq!(duration, seconds(3.2));
        assert!((retime.speed_at(Time(20_000)) - 0.5).abs() < 0.03);
        assert!((retime.speed_at(duration - Time(20_000)) - 2.0).abs() < 0.03);
        assert!((retime.speed_at(Time(duration.0 / 2)) - 1.25).abs() < 0.03);
    }

    #[test]
    fn reversing_plays_the_same_source_backwards_and_twice_is_as_before() {
        let (project, id) = shot(0.0);
        let stored = clip(&project, id);
        let mut curve = Ramp::Hero.over(&Curve::of(stored));
        let forwards = curve.clone();
        let (map, duration) = curve.valid().unwrap();
        curve.reverse();
        assert!(curve.reversed());
        let (reversed, same) = curve.valid().unwrap();
        assert_eq!(same, duration);
        for step in 0..=16 {
            let at = Time(duration.0 * step / 16);
            let mirrored = map.source_at(duration - at);
            assert!(
                (reversed.source_at(at).0 - mirrored.0).abs() <= 50,
                "{step}"
            );
        }
        curve.reverse();
        assert_eq!(curve, forwards);

        let command = {
            let mut curve = Curve::of(stored);
            curve.reverse();
            curve_command(&project, id, &curve, "Reverse clip").unwrap()
        };
        let editor = applied(&project, command);
        let after = clip(editor.project(), id);
        assert_eq!(editor.undo_label(), Some("Reverse clip"));
        assert_eq!(after.source_time(after.start), seconds(4.0));
        assert_eq!(after.source_time(after.end()), Time::ZERO);
        assert_eq!(badge(after).as_deref(), Some("Reversed"));
    }

    #[test]
    fn a_frozen_frame_is_held_and_the_rest_follows_it() {
        let (project, id) = shot(1.0);
        let command = freeze_command(&project, id, seconds(2.5), HOLD).unwrap();
        let mut editor = applied(&project, command);
        assert_eq!(editor.history().count(), 1);
        assert_eq!(editor.undo_label(), Some("Freeze frame"));
        let after = clip(editor.project(), id).clone();
        assert_eq!(after.duration, seconds(6.0));
        for (playhead, source) in [
            (1.0, 0.0),
            (2.5, 1.5),
            (3.5, 1.5),
            (4.5, 1.5),
            (5.5, 2.5),
            (7.0, 4.0),
        ] {
            assert_eq!(
                after.source_time(seconds(playhead)),
                seconds(source),
                "{playhead}"
            );
        }
        assert_eq!(badge(&after).as_deref(), Some("Freeze"));
        // Frozen again inside the hold, the hold is longer and has no more keys.
        let again = freeze_command(editor.project(), id, seconds(3.0), seconds(1.0)).unwrap();
        editor.dispatch(again).unwrap();
        let longer = clip(editor.project(), id);
        assert_eq!(longer.duration, seconds(7.0));
        assert_eq!(longer.retime.as_ref().unwrap().keys.len(), 4);
        assert!(editor.undo() && editor.undo());
        assert_eq!(clip(editor.project(), id), clip(&project, id));
    }

    #[test]
    fn a_frame_is_frozen_at_either_end_of_a_clip_and_of_a_clip_with_a_rate() {
        let (mut project, id) = shot(0.0);
        Arc::make_mut(&mut project.tracks[0]).clips[0].playback_rate = 2.0;
        project.validate().unwrap();
        // Four seconds at twice the speed show eight of source.
        for (playhead, shown) in [(0.0, 0.0), (4.0, 8.0), (1.0, 2.0)] {
            let command = freeze_command(&project, id, seconds(playhead), HOLD).unwrap();
            let editor = applied(&project, command);
            let after = clip(editor.project(), id);
            assert_eq!(after.duration, seconds(6.0));
            assert_eq!(after.playback_rate, 1.0);
            let held = after.source_time(seconds(playhead + 1.0));
            assert!(
                (held.0 - seconds(shown).0).abs() <= 2,
                "{playhead}: {held:?}"
            );
            assert_eq!(after.source_time(after.end()), seconds(8.0));
        }
    }

    #[test]
    fn a_freeze_says_why_it_cannot_be() {
        let (mut project, id) = shot(1.0);
        assert!(freeze_command(&project, id, seconds(0.5), HOLD).is_err());
        assert!(freeze_command(&project, id, seconds(5.5), HOLD).is_err());
        let title = Clip::text("Title", Time::ZERO, seconds(2.0));
        let title_id = title.id;
        let text = (project.tracks.iter())
            .position(|track| track.kind == TrackKind::Text)
            .unwrap();
        Arc::make_mut(&mut project.tracks[text]).clips.push(title);
        assert!(freeze_command(&project, title_id, seconds(1.0), HOLD).is_err());
        // The selected clip when the playhead is on it, else the one under it.
        assert_eq!(
            freeze_target(&project, Some(title_id), seconds(1.5)),
            Some(id)
        );
        assert_eq!(freeze_target(&project, Some(id), seconds(1.5)), Some(id));
        assert_eq!(freeze_target(&project, None, seconds(0.5)), None);
    }

    #[test]
    fn a_longer_clip_is_cut_to_the_room_before_the_next_one() {
        let (mut project, id) = shot(0.0);
        let mut next = clip(&project, id).clone();
        next.id = uuid::Uuid::new_v4();
        next.start = seconds(5.0);
        Arc::make_mut(&mut project.tracks[0]).clips.push(next);
        project.validate().unwrap();
        let command = freeze_command(&project, id, seconds(2.0), HOLD).unwrap();
        let editor = applied(&project, command);
        assert_eq!(editor.history().count(), 1);
        let after = clip(editor.project(), id);
        assert_eq!(after.duration, seconds(5.0));
        assert_eq!(after.source_time(seconds(4.5)), seconds(2.5));
    }

    #[test]
    fn every_ramp_is_a_map_the_project_takes_over_the_same_source() {
        let (project, id) = shot(0.0);
        for ramp in Ramp::ALL {
            let curve = ramp.over(&Curve::of(clip(&project, id)));
            let command = curve_command(&project, id, &curve, ramp.label())
                .unwrap_or_else(|| panic!("{ramp:?} is not a map"));
            let editor = applied(&project, command);
            let after = clip(editor.project(), id);
            assert_eq!(after.source_span(), (Time::ZERO, seconds(4.0)), "{ramp:?}");
            assert_eq!(badge(after).as_deref(), Some("Ramp"));
            // The speeds at its ends are the ramp's.
            let points = ramp.points();
            let read = Curve::of(after);
            assert!((read.speed(0) - points[0].1).abs() < 0.01, "{ramp:?}");
            let last = read.speed(read.points() - 1);
            assert!((last - points[points.len() - 1].1).abs() < 0.01, "{ramp:?}");
        }
    }

    #[test]
    fn a_constant_speed_lets_a_map_go() {
        let (project, id) = shot(0.0);
        let ramped = {
            let curve = Ramp::Up.over(&Curve::of(clip(&project, id)));
            applied(
                &project,
                curve_command(&project, id, &curve, "Ramp").unwrap(),
            )
        };
        let command = constant_command(ramped.project(), id, 2.0).unwrap();
        let editor = applied(ramped.project(), command);
        let after = clip(editor.project(), id);
        assert_eq!((after.retime.as_ref(), after.playback_rate), (None, 2.0));
        assert_eq!(after.duration, seconds(2.0));
        assert_eq!(after.source_span(), (Time::ZERO, seconds(4.0)));
        assert_eq!(badge(after).as_deref(), Some("2×"));
        assert_eq!(editor.undo_label(), Some("Change speed"));
    }

    #[test]
    fn a_constant_speed_keeps_a_reversed_clip_reversed() {
        let (project, id) = shot(0.0);
        let reversed = {
            let mut curve = Curve::of(clip(&project, id));
            curve.reverse();
            applied(
                &project,
                curve_command(&project, id, &curve, "Reverse clip").unwrap(),
            )
        };
        let command = constant_command(reversed.project(), id, 2.0).unwrap();
        let mut editor = applied(reversed.project(), command);
        let after = clip(editor.project(), id);
        assert!(Curve::of(after).reversed());
        assert_eq!(Curve::of(after).steady(), Some(2.0));
        assert_eq!(after.duration, seconds(2.0));
        assert_eq!(after.source_span(), (Time::ZERO, seconds(4.0)));
        assert_eq!(badge(after).as_deref(), Some("Reversed 2×"));
        assert!(editor.undo());
        assert_eq!(clip(editor.project(), id), clip(reversed.project(), id));
    }

    #[test]
    fn points_are_put_in_moved_and_taken_out() {
        let (project, id) = shot(0.0);
        let mut curve = Curve::of(clip(&project, id));
        let flat = curve.clone();
        let point = curve.insert(seconds(1.0)).unwrap();
        // A point on a flat stretch changes nothing that plays.
        assert_eq!((point, curve.duration()), (1, seconds(4.0)));
        assert_eq!(curve.speed(1), 1.0);
        // Faster at the point, the clip is shorter.
        curve.set_speed(1, 3.0);
        assert!(curve.duration() < seconds(4.0));
        assert_eq!(curve.speed(1), 3.0);
        // It moves in the clip, between its neighbours.
        let before = curve.at(1);
        curve.slide(1, before + seconds(0.25));
        assert!(curve.at(1) > before);
        curve.slide(1, seconds(100.0));
        assert!(curve.pieces[1].span >= GAP);
        assert!(curve.valid().is_some());
        // The ends stay, and are not taken out.
        curve.slide(0, seconds(1.0));
        assert!(!curve.remove(0) && !curve.remove(2));
        assert!(curve.remove(1));
        assert_eq!(curve, flat);
    }

    #[test]
    fn a_ramp_too_slow_for_the_project_is_no_map() {
        let (project, id) = shot(0.0);
        let mut curve = Curve::of(clip(&project, id));
        curve.set_speed(0, 0.0);
        curve.set_speed(1, 0.06);
        assert!(curve.valid().is_none());
        assert!(curve_command(&project, id, &curve, "Ramp").is_none());
        curve.set_speed(1, 0.5);
        assert!(curve.valid().is_some(), "a ramp from a stop is a map");
    }

    #[test]
    fn a_tile_says_how_its_clip_is_retimed() {
        let (mut project, id) = shot(0.0);
        assert_eq!(badge(clip(&project, id)), None);
        // Forwards, held and back again is none of the three by name.
        let key = |at: f64, source: f64| TimeKey {
            at: seconds(at),
            source: seconds(source),
            ease: Ease::Linear,
        };
        let mut mixed = clip(&project, id).clone();
        mixed.retime = Some(Retime {
            keys: vec![key(0.0, 0.0), key(1.0, 1.0), key(2.0, 1.0), key(4.0, 0.0)],
        });
        assert_eq!(badge(&mixed).as_deref(), Some("Retimed"));
        mixed.retime = Some(Retime {
            keys: vec![key(0.0, 0.0), key(4.0, 2.0)],
        });
        assert_eq!(badge(&mixed).as_deref(), Some("0.5×"));
        Arc::make_mut(&mut project.tracks[0]).clips[0].playback_rate = 0.5;
        assert_eq!(badge(clip(&project, id)).as_deref(), Some("0.5×"));
        assert_eq!(times(1.25), "1.25×");
        assert_eq!(times(4.0), "4×");
    }
}
