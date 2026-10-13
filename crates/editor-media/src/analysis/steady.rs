//! Stabilization: how the camera moved from frame to frame, and what puts
//! each frame back on a smoother path.
//!
//! What this is, and all it is: a 2D deshake. The whole picture's motion
//! between two frames is measured as one similarity (a shift, a turn and a
//! change of size) from blocks of luma matched across the frame, with the
//! blocks that disagree left out. The steps are added up into the path the
//! camera took, that path is smoothed over a window, and a frame is corrected
//! by the difference between the two. The correction moves, turns and sizes
//! the whole frame; a crop hides the borders that would show. There is no
//! correction of a rolling shutter, no warping of parts of the frame against
//! each other, no depth, and nothing is known of what the picture shows: a
//! subject that fills the frame is taken for the camera.
//!
//! The path is derived data: it is no part of a project and is kept in
//! [`SteadyCache`], a bounded directory of files that can be made again.
use super::{Control, Gray, Luma};
use crate::{MediaError, process::Processes};
use editor_core::{Asset, FrameRate, MediaKind, Time};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

/// The version of the measurement: a path measured by another is not read.
pub const STEADY_ALGORITHM: u32 = 1;
/// The frames one path may have. A video of more is not analysed.
pub const MAX_STEADY_FRAMES: usize = 1 << 20;
/// What the cache directory may hold, and in how many files.
pub const STEADY_CACHE_BYTES: u64 = 64 * 1024 * 1024;
pub const STEADY_CACHE_ENTRIES: usize = 256;
/// The deviation, in seconds, of the gaussian a path is smoothed with at full
/// strength: the window reaches three of them either way.
pub const STEADY_SECONDS: f64 = 0.5;
/// The frames of a clip a crop is suggested from: more are read evenly.
pub const STEADY_SUGGEST_FRAMES: usize = 2048;

// A block is this many pixels a side on the smallest picture of the pyramid,
// and is looked for this far each way there.
const TOP_BLOCK: usize = 8;
const TOP_REACH: i32 = 6;
// The blocks across and down a frame, at most.
const MAX_ACROSS: usize = 8;
const MAX_DOWN: usize = 5;
// The least of a block's two spreads of slope (8-bit levels a pixel,
// squared, a texel) for it to say where it went both ways.
const MIN_CORNER: f32 = 4.0;
// The most of a block's own spread its match may leave unexplained.
const MAX_UNEXPLAINED: f32 = 0.5;
// The fewest blocks a similarity is fitted to; fewer give a shift alone.
const MIN_FITTED: usize = 6;
// How far from their own middle the blocks a similarity is fitted to are,
// at the least and as a root mean square, in shorter sides of the frame.
const MIN_APART: f32 = 0.25;
// The most the blocks a turn or a growth is read from may be off it, as a
// root mean square in pixels.
const MAX_OFF: f32 = 0.35;
// The most a frame may turn or grow against the one before, as the two
// numbers of its similarity less the identity. More is not believed.
const MAX_TURN: f32 = 0.05;

/// The path a camera took through a video: where what the first frame shows
/// has got to by each frame, as the sum of the steps measured between frames.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    /// The rate the frames were read at: frame `i` is at `rate.frame_time(i)`.
    pub rate: FrameRate,
    /// For each frame the shift across and down as fractions of the frame's
    /// width and height, the turn in radians clockwise, and the natural
    /// logarithm of the growth. The first is nought.
    pub path: Vec<[f32; 4]>,
    /// The frames whose step from the one before could not be measured (too
    /// even, too dark, or moved too far) and was taken to be none.
    pub unmeasured: u32,
}

/// What one frame is corrected by: the motion to give its picture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fix {
    /// Across and down, as fractions of the frame's width and height.
    pub shift: [f32; 2],
    /// Radians, clockwise, about the frame's centre.
    pub turn: f32,
    /// The natural logarithm of the growth about the frame's centre.
    pub grow: f32,
}

/// Where a corrected and cropped frame reads its picture: the place `uv` of
/// what is shown is read at `0.5 + matrix * (uv - 0.5) + offset` of the
/// frame, all in fractions of the frame from its left top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampling {
    /// Row by row: `[xx, xy, yx, yy]`.
    pub matrix: [f32; 4],
    pub offset: [f32; 2],
}

impl Sampling {
    pub const IDENTITY: Self = Self {
        matrix: [1.0, 0.0, 0.0, 1.0],
        offset: [0.0, 0.0],
    };

    /// Where of the frame the place `uv` of what is shown is read.
    pub fn at(&self, uv: [f32; 2]) -> [f32; 2] {
        let [x, y] = [uv[0] - 0.5, uv[1] - 0.5];
        let [xx, xy, yx, yy] = self.matrix;
        [
            0.5 + xx * x + xy * y + self.offset[0],
            0.5 + yx * x + yy * y + self.offset[1],
        ]
    }

    /// Whether everything shown is read from inside the frame.
    fn inside(&self) -> bool {
        const SLACK: f32 = 1e-5;
        [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]
            .into_iter()
            .all(|corner| (self.at(corner).iter()).all(|at| (-SLACK..=1.0 + SLACK).contains(at)))
    }
}

impl Fix {
    /// The correction made `part` of what it is.
    pub fn scaled(self, part: f32) -> Self {
        Self {
            shift: [self.shift[0] * part, self.shift[1] * part],
            turn: self.turn * part,
            grow: self.grow * part,
        }
    }

    fn mixed(self, other: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self {
            shift: [
                mix(self.shift[0], other.shift[0]),
                mix(self.shift[1], other.shift[1]),
            ],
            turn: mix(self.turn, other.turn),
            grow: mix(self.grow, other.grow),
        }
    }

    /// How a frame of `aspect` (width over height) is read to show it
    /// corrected and enlarged `zoom` times about its centre.
    pub fn sampling(self, zoom: f32, aspect: f32) -> Sampling {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        let zoom = zoom.max(1e-3);
        // The picture is moved by the correction, so it is read through the
        // correction undone: turned and sized back about the centre, in
        // pixels, which a fraction of the width is not what one of the
        // height is.
        let back = (-self.grow).exp();
        let (sin, cos) = self.turn.sin_cos();
        let undo = [
            back * cos,
            back * sin / aspect,
            -back * sin * aspect,
            back * cos,
        ];
        let [dx, dy] = self.shift;
        Sampling {
            matrix: undo.map(|value| value / zoom),
            offset: [
                -(undo[0] * dx + undo[1] * dy),
                -(undo[2] * dx + undo[3] * dy),
            ],
        }
    }

    /// As much of the correction as a crop of `zoom` hides the borders of:
    /// all of it when nothing outside the frame would be read, and else the
    /// largest part of it that reads none. A frame is then never shown with
    /// its border in the picture, and is steadied less where the crop is too
    /// small for what it would take.
    pub fn fitted(self, zoom: f32, aspect: f32) -> Self {
        if self.sampling(zoom, aspect).inside() {
            return self;
        }
        let (mut low, mut high) = (0.0f32, 1.0f32);
        for _ in 0..12 {
            let half = (low + high) * 0.5;
            if self.scaled(half).sampling(zoom, aspect).inside() {
                low = half;
            } else {
                high = half;
            }
        }
        self.scaled(low)
    }
}

impl Motion {
    /// The deviation in frames of the gaussian `smoothness` smooths with.
    fn deviation(&self, smoothness: f32) -> f64 {
        f64::from(smoothness.clamp(0.0, 1.0)) * STEADY_SECONDS * self.rate.fps()
    }

    /// The smoothed path at a frame: the mean of the path around it under a
    /// gaussian cut at three deviations. Past either end the path is taken
    /// to go on along the straight line that best says its last window, so
    /// an even pan stays the pan it is up to its ends, and a knock on the
    /// last frame is not taken for where the camera was going.
    fn smoothed(&self, frame: usize, deviation: f64) -> [f32; 4] {
        let reach = (3.0 * deviation).ceil() as i64;
        let last = self.path.len() as i64 - 1;
        // The line through the frames `from..=to`: its middle, the path's
        // mean there, and its slope a frame.
        let line = |from: i64, to: i64| {
            let count = (to - from + 1) as f64;
            let middle = (from + to) as f64 * 0.5;
            let mut mean = [0f64; 4];
            let mut rise = [0f64; 4];
            let mut run = 0f64;
            for index in from..=to {
                let along = index as f64 - middle;
                run += along * along;
                for part in 0..4 {
                    let value = f64::from(self.path[index as usize][part]);
                    mean[part] += value / count;
                    rise[part] += along * value;
                }
            }
            let slope = rise.map(|rise| if run > 0.0 { rise / run } else { 0.0 });
            (middle, mean, slope)
        };
        let before = (frame as i64 - reach < 0).then(|| line(0, reach.max(2).min(last)));
        let after = (frame as i64 + reach > last).then(|| line((last - reach.max(2)).max(0), last));
        let mut sum = [0f64; 4];
        let mut weights = 0f64;
        for step in -reach..=reach {
            let index = frame as i64 + step;
            let away = step as f64 / deviation;
            let weight = (-0.5 * away * away).exp();
            weights += weight;
            let beyond = if index < 0 {
                before.as_ref()
            } else if index > last {
                after.as_ref()
            } else {
                None
            };
            for part in 0..4 {
                let value = match beyond {
                    Some((middle, mean, slope)) => {
                        mean[part] + slope[part] * (index as f64 - middle)
                    }
                    None => f64::from(self.path[index as usize][part]),
                };
                sum[part] += weight * value;
            }
        }
        sum.map(|sum| (sum / weights) as f32)
    }

    /// What frame `frame` is corrected by at `smoothness` (`0..=1`): what
    /// takes it from the path the camera took to the smoothed one. Nothing
    /// outside the path, or at no smoothness.
    pub fn fix(&self, frame: usize, smoothness: f32) -> Fix {
        let deviation = self.deviation(smoothness);
        let Some(taken) = self.path.get(frame).filter(|_| deviation > 1e-6) else {
            return Fix::default();
        };
        let smooth = self.smoothed(frame, deviation);
        Fix {
            shift: [smooth[0] - taken[0], smooth[1] - taken[1]],
            turn: smooth[2] - taken[2],
            grow: smooth[3] - taken[3],
        }
    }

    /// The correction at a source time, a line between those of the frames
    /// either side of it.
    pub fn fix_at(&self, source: Time, smoothness: f32) -> Fix {
        let Some(last) = self.path.len().checked_sub(1) else {
            return Fix::default();
        };
        let at = (source.0.max(0) as f64 / 1_000_000.0 * self.rate.fps()).clamp(0.0, last as f64);
        let before = at.floor() as usize;
        let part = (at - before as f64) as f32;
        // A time a microsecond off its frame is that frame.
        if part < 1e-3 || before == last {
            return self.fix(before, smoothness);
        }
        if part > 1.0 - 1e-3 {
            return self.fix(before + 1, smoothness);
        }
        (self.fix(before, smoothness)).mixed(self.fix(before + 1, smoothness), part)
    }

    /// The least crop, in hundredths and `1..=2`, that hides the borders of
    /// every correction between two source times at `smoothness`, for a
    /// frame of `aspect`. It is read from every frame of a stretch of up to
    /// [`STEADY_SUGGEST_FRAMES`] and evenly from that many of a longer one,
    /// so a longer stretch may have a frame that wants more: that frame is
    /// then steadied less, as [`Fix::fitted`] has it, and shows no border.
    pub fn crop_zoom(&self, from: Time, until: Time, smoothness: f32, aspect: f32) -> f32 {
        let Some(last) = self.path.len().checked_sub(1) else {
            return 1.0;
        };
        let frame =
            |time: Time| (self.rate.frame_at(Time(time.0.max(0))).max(0) as usize).min(last);
        let (first, end) = (frame(from), frame(until).max(frame(from)));
        let count = (end - first + 1).min(STEADY_SUGGEST_FRAMES);
        let fixes: Vec<Fix> = (0..count)
            .map(|index| {
                let at = if count > 1 {
                    first + index * (end - first) / (count - 1)
                } else {
                    first
                };
                self.fix(at, smoothness)
            })
            .collect();
        let hides = |zoom: f32| fixes.iter().all(|fix| fix.sampling(zoom, aspect).inside());
        if hides(1.0) {
            return 1.0;
        }
        if !hides(2.0) {
            return 2.0;
        }
        let (mut low, mut high) = (1.0f32, 2.0f32);
        for _ in 0..12 {
            let half = (low + high) * 0.5;
            if hides(half) {
                high = half;
            } else {
                low = half;
            }
        }
        ((high * 100.0).ceil() / 100.0).clamp(1.0, 2.0)
    }
}

// ---- Measuring ---------------------------------------------------------------

/// A picture's luma as numbers, and the same at half the size and again.
struct Plane {
    width: usize,
    height: usize,
    values: Vec<f32>,
}

impl Plane {
    fn of(frame: &Gray) -> Self {
        Self {
            width: frame.width,
            height: frame.height,
            values: frame.luma.iter().map(|value| f32::from(*value)).collect(),
        }
    }

    fn halved(&self) -> Self {
        let (width, height) = (self.width / 2, self.height / 2);
        let mut values = Vec::with_capacity(width * height);
        for y in 0..height {
            let (top, bottom) = (2 * y * self.width, (2 * y + 1) * self.width);
            for x in 0..width {
                values.push(
                    0.25 * (self.values[top + 2 * x]
                        + self.values[top + 2 * x + 1]
                        + self.values[bottom + 2 * x]
                        + self.values[bottom + 2 * x + 1]),
                );
            }
        }
        Self {
            width,
            height,
            values,
        }
    }

    fn at(&self, x: usize, y: usize) -> f32 {
        self.values[y * self.width + x]
    }

    // Between pixels, held at the edge.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let x = x.clamp(0.0, (self.width - 1) as f32);
        let y = y.clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let top = self.at(x0, y0) * (1.0 - fx) + self.at(x1, y0) * fx;
        let bottom = self.at(x0, y1) * (1.0 - fx) + self.at(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    // The sum of differences between a block of this at `(x, y)` and the
    // block of `other` moved by `(dx, dy)`. Both are inside their pictures.
    fn differs(&self, other: &Self, x: usize, y: usize, side: usize, dx: i32, dy: i32) -> f32 {
        let (ox, oy) = ((x as i32 + dx) as usize, (y as i32 + dy) as usize);
        let mut sum = 0.0;
        for row in 0..side {
            let mine = &self.values[(y + row) * self.width + x..][..side];
            let theirs = &other.values[(oy + row) * other.width + ox..][..side];
            sum += (mine.iter().zip(theirs))
                .map(|(a, b)| (a - b).abs())
                .sum::<f32>();
        }
        sum
    }
}

struct Pyramid {
    /// The picture, then each half of the one before.
    levels: Vec<Plane>,
}

impl Pyramid {
    fn of(frame: &Gray) -> Self {
        let shorter = frame.width.min(frame.height);
        let count = if shorter >= 128 {
            3
        } else if shorter >= 64 {
            2
        } else {
            1
        };
        let mut levels = vec![Plane::of(frame)];
        while levels.len() < count {
            let half = levels[levels.len() - 1].halved();
            levels.push(half);
        }
        Self { levels }
    }

    /// The side of a block on the picture, and how far from the picture's
    /// edge a block keeps so that it can be looked for.
    fn block(&self) -> (usize, usize) {
        let grown = 1usize << (self.levels.len() - 1);
        // The reach of the smallest picture, a pixel more for each finer
        // one, one for the slopes, and what a block is moved by to lie on
        // the grid of the smallest.
        let reach = TOP_REACH as usize * grown + 2 * grown + 2;
        (TOP_BLOCK * grown, reach)
    }

    /// The left top of each block: a grid inside the margin.
    fn blocks(&self) -> Vec<[usize; 2]> {
        let (side, margin) = self.block();
        let plane = &self.levels[0];
        let grown = side / TOP_BLOCK;
        let along = |extent: usize, most: usize| -> Vec<usize> {
            let Some(room) = extent.checked_sub(2 * margin + side) else {
                return Vec::new();
            };
            let count = (1 + room / (side / 2)).min(most);
            (0..count)
                .map(|index| {
                    let step = if count > 1 {
                        index * room / (count - 1)
                    } else {
                        room / 2
                    };
                    // On the grid of the smallest picture, so the block is
                    // the same block on every one.
                    (margin + step) / grown * grown
                })
                .collect()
        };
        let (across, down) = (
            along(plane.width, MAX_ACROSS),
            along(plane.height, MAX_DOWN),
        );
        down.iter()
            .flat_map(|y| across.iter().map(|x| [*x, *y]))
            .collect()
    }
}

/// Where one block went, in pixels, and where its centre was from the
/// frame's centre.
#[derive(Clone, Copy, Debug)]
struct Went {
    at: [f32; 2],
    by: [f32; 2],
}

/// How the whole frame moved against the one before: a place `p` from the
/// frame's centre went to `p + shift + [[a, -b], [b, a]] p`, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    shift: [f32; 2],
    a: f32,
    b: f32,
}

// Where the block of `before` at `corner` is in `after`, between pixels.
// None for a block that cannot say: too even one way or both, or not found
// again.
fn follow(before: &Pyramid, after: &Pyramid, corner: [usize; 2]) -> Option<[f32; 2]> {
    let (side, _) = before.block();
    let plane = &before.levels[0];
    let [left, top] = corner;
    // What the block has to be told apart by: the slopes of its luma.
    let (mut xx, mut xy, mut yy) = (0f64, 0f64, 0f64);
    let mut slopes = Vec::with_capacity(side * side);
    let (mut sum, mut squares) = (0f64, 0f64);
    for row in 0..side {
        for column in 0..side {
            let (x, y) = (left + column, top + row);
            let across = 0.5 * (plane.at(x + 1, y) - plane.at(x - 1, y));
            let down = 0.5 * (plane.at(x, y + 1) - plane.at(x, y - 1));
            slopes.push([across, down]);
            xx += f64::from(across * across);
            xy += f64::from(across * down);
            yy += f64::from(down * down);
            let value = f64::from(plane.at(x, y));
            sum += value;
            squares += value * value;
        }
    }
    let texels = (side * side) as f64;
    let spread = squares - sum * sum / texels;
    // The lesser of the two spreads of slope: an edge has one, a corner two.
    let lesser = 0.5 * (xx + yy - ((xx - yy) * (xx - yy) + 4.0 * xy * xy).sqrt());
    if lesser / texels < f64::from(MIN_CORNER) {
        return None;
    }

    // Coarsely on the smallest picture, then a pixel either way on each
    // finer one.
    let top_level = before.levels.len() - 1;
    let mut moved = [0i32; 2];
    for level in (0..=top_level).rev() {
        let shrink = 1usize << level;
        let (x, y, block) = (left / shrink, top / shrink, side / shrink);
        let reach = if level == top_level { TOP_REACH } else { 1 };
        let origin = if level == top_level {
            [0, 0]
        } else {
            [moved[0] * 2, moved[1] * 2]
        };
        let (from, to) = (&before.levels[level], &after.levels[level]);
        let mut best = (f32::INFINITY, origin);
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let by = [origin[0] + dx, origin[1] + dy];
                let (ox, oy) = (x as i32 + by[0], y as i32 + by[1]);
                if ox < 0
                    || oy < 0
                    || ox as usize + block > to.width
                    || oy as usize + block > to.height
                {
                    continue;
                }
                let differs = from.differs(to, x, y, block, by[0], by[1]);
                // Of two as alike, the one that moved less.
                let nearer = by[0].abs() + by[1].abs() < best.1[0].abs() + best.1[1].abs();
                if differs < best.0 || (differs == best.0 && nearer) {
                    best = (differs, by);
                }
            }
        }
        if !best.0.is_finite() {
            return None;
        }
        moved = best.1;
    }

    // Between pixels: steps of least squares on the block's slopes against
    // what the block still differs from the frame by.
    let det = xx * yy - xy * xy;
    if det <= 1e-6 {
        return None;
    }
    let other = &after.levels[0];
    let whole = [moved[0] as f32, moved[1] as f32];
    let mut by = whole;
    let mut left_over = f64::INFINITY;
    for _ in 0..6 {
        let (mut xr, mut yr, mut unexplained) = (0f64, 0f64, 0f64);
        for row in 0..side {
            for column in 0..side {
                let (x, y) = (left + column, top + row);
                let differs = plane.at(x, y) - other.sample(x as f32 + by[0], y as f32 + by[1]);
                let [across, down] = slopes[row * side + column];
                // What the block has over the frame here is the slope times
                // how far the frame has still to be moved.
                xr += f64::from(across * differs);
                yr += f64::from(down * differs);
                unexplained += f64::from(differs * differs);
            }
        }
        left_over = unexplained;
        let step = [
            ((yy * xr - xy * yr) / det) as f32,
            ((xx * yr - xy * xr) / det) as f32,
        ];
        by = [by[0] + step[0], by[1] + step[1]];
        if (by[0] - whole[0]).abs() > 1.5 || (by[1] - whole[1]).abs() > 1.5 {
            return None;
        }
        if step[0].abs() < 0.005 && step[1].abs() < 0.005 {
            break;
        }
    }
    (left_over <= f64::from(MAX_UNEXPLAINED) * spread.max(1e-6)).then_some(by)
}

fn median(values: &mut [f32]) -> f32 {
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

// The similarity that says best where `blocks` went, by least squares, in
// a frame whose shorter side is `short` pixels.
fn fitted(blocks: &[Went], short: f32) -> Option<Step> {
    let count = blocks.len() as f64;
    if blocks.is_empty() {
        return None;
    }
    let mean = |of: &dyn Fn(&Went) -> f32| {
        blocks.iter().map(|went| f64::from(of(went))).sum::<f64>() / count
    };
    let centre = [mean(&|went| went.at[0]), mean(&|went| went.at[1])];
    let shift = [mean(&|went| went.by[0]), mean(&|went| went.by[1])];
    let (mut arm, mut along, mut around) = (0f64, 0f64, 0f64);
    for went in blocks {
        let (x, y) = (
            f64::from(went.at[0]) - centre[0],
            f64::from(went.at[1]) - centre[1],
        );
        let (dx, dy) = (f64::from(went.by[0]), f64::from(went.by[1]));
        arm += x * x + y * y;
        along += x * dx + y * dy;
        around += x * dy - y * dx;
    }
    // Blocks close together say little of a turn or a growth, and what
    // little they say wrongly is made much of at the frame's centre.
    let apart = (arm / count).sqrt() >= f64::from(MIN_APART * short);
    let (a, b) = if blocks.len() >= MIN_FITTED && apart {
        (along / arm, around / arm)
    } else {
        (0.0, 0.0)
    };
    // The shift is that of the frame's centre, not of the blocks' own.
    Some(Step {
        shift: [
            (shift[0] - (a * centre[0] - b * centre[1])) as f32,
            (shift[1] - (b * centre[0] + a * centre[1])) as f32,
        ],
        a: a as f32,
        b: b as f32,
    })
}

fn misses(step: &Step, went: &Went) -> f32 {
    let [x, y] = went.at;
    let said = [
        step.shift[0] + step.a * x - step.b * y,
        step.shift[1] + step.b * x + step.a * y,
    ];
    (said[0] - went.by[0]).hypot(said[1] - went.by[1])
}

/// The motion most of the blocks agree on. Blocks that went elsewhere (what
/// moves in the picture, a wrong match) are left out, first by how far from
/// the middle block they went and then by how far from the similarity the
/// others make. None when too few blocks say anything.
fn agreed(blocks: &[Went], short: f32) -> Option<Step> {
    if blocks.len() < 2 {
        return None;
    }
    let middle = [0, 1].map(|axis| {
        let mut values: Vec<f32> = blocks.iter().map(|went| went.by[axis]).collect();
        median(&mut values)
    });
    let away: Vec<f32> = (blocks.iter())
        .map(|went| (went.by[0] - middle[0]).hypot(went.by[1] - middle[1]))
        .collect();
    let allowed = (3.0 * 1.4826 * median(&mut away.clone())).max(3.0);
    let mut kept: Vec<Went> = (blocks.iter().zip(&away))
        .filter(|(_, away)| **away <= allowed)
        .map(|(went, _)| *went)
        .collect();
    let mut step = fitted(&kept, short)?;
    for _ in 0..4 {
        let missed: Vec<f32> = kept.iter().map(|went| misses(&step, went)).collect();
        let allowed = (2.5 * 1.4826 * median(&mut missed.clone())).max(0.25);
        let before = kept.len();
        kept = (kept.iter().zip(&missed))
            .filter(|(_, missed)| **missed <= allowed)
            .map(|(went, _)| *went)
            .collect();
        if kept.len() == before {
            break;
        }
        step = fitted(&kept, short)?;
    }
    // Fewer than a third of the blocks is not the frame.
    if kept.len() < 2 || kept.len() * 3 < blocks.len() {
        return None;
    }
    // What the blocks left in are off the similarity by. Blocks of two
    // things that move apart are fitted by a turn or a growth that neither
    // made, and are off it by pixels; those of one frame, by little.
    let off = (kept.iter())
        .map(|went| f64::from(misses(&step, went)).powi(2))
        .sum::<f64>()
        / kept.len() as f64;
    let turns = step.a != 0.0 || step.b != 0.0;
    if turns
        && (step.a.abs() > MAX_TURN
            || step.b.abs() > MAX_TURN
            || off.sqrt() > f64::from(MAX_OFF)
            || kept.len() * 2 < blocks.len())
    {
        // A turn or a growth this fast, this badly kept to or of fewer
        // than half the blocks is a wrong fit: the shift the blocks agree
        // on, alone.
        step = Step {
            shift: middle,
            a: 0.0,
            b: 0.0,
        };
    }
    Some(step)
}

fn between(before: &Pyramid, after: &Pyramid) -> Option<Step> {
    let (side, _) = before.block();
    let plane = &before.levels[0];
    let centre = [plane.width as f32 * 0.5, plane.height as f32 * 0.5];
    let half = (side - 1) as f32 * 0.5;
    let blocks: Vec<Went> = (before.blocks().into_iter())
        .filter_map(|corner| {
            let by = follow(before, after, corner)?;
            Some(Went {
                at: [
                    corner[0] as f32 + half + 0.5 - centre[0],
                    corner[1] as f32 + half + 0.5 - centre[1],
                ],
                by,
            })
        })
        .collect();
    agreed(&blocks, plane.width.min(plane.height) as f32)
}

/// The camera's path as its frames are read, one after another.
pub(crate) struct Camera {
    before: Option<Pyramid>,
    path: Vec<[f32; 4]>,
    unmeasured: u32,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            before: None,
            path: Vec::new(),
            unmeasured: 0,
        }
    }

    /// Takes the next frame. All frames are of one size.
    pub fn push(&mut self, frame: &Gray) -> Result<(), String> {
        let pyramid = Pyramid::of(frame);
        if pyramid.blocks().is_empty() {
            return Err("The picture is too small to measure the camera's motion in".into());
        }
        let Some(before) = &self.before else {
            self.path.push([0.0; 4]);
            self.before = Some(pyramid);
            return Ok(());
        };
        let last = self.path[self.path.len() - 1];
        let next = match between(before, &pyramid) {
            Some(step) => {
                let (width, height) = (frame.width as f32, frame.height as f32);
                let (grown, turned) = (1.0 + step.a, step.b);
                [
                    last[0] + step.shift[0] / width,
                    last[1] + step.shift[1] / height,
                    last[2] + turned.atan2(grown),
                    last[3] + 0.5 * (grown * grown + turned * turned).ln(),
                ]
            }
            None => {
                self.unmeasured += 1;
                last
            }
        };
        self.path.push(next);
        self.before = Some(pyramid);
        Ok(())
    }

    pub fn finish(self, rate: FrameRate) -> Motion {
        Motion {
            rate,
            path: self.path,
            unmeasured: self.unmeasured,
        }
    }
}

/// One video to measure, or to read the measurement of.
#[derive(Clone, Debug)]
pub struct SteadyJob {
    /// The project that asks, handed back with the answer and never read.
    pub project: Uuid,
    pub asset: Asset,
    pub cache: SteadyCache,
}

/// What is known of a video's camera path.
#[derive(Clone, Debug, PartialEq)]
pub enum Steadied {
    Ready(Arc<Motion>),
    /// A path is kept, measured from what the file was before it changed.
    Stale,
    /// None is kept.
    Missing,
}

pub(crate) fn check_steady(job: &SteadyJob) -> Result<(), MediaError> {
    let invalid = |why: &str| Err(MediaError::Invalid(why.into()));
    if job.asset.kind != MediaKind::Video {
        return invalid("Only a video has a camera to steady");
    }
    if job.asset.width == 0 || job.asset.height == 0 {
        return invalid("Media has no video dimensions");
    }
    if !job.asset.duration.is_valid() || job.asset.duration <= Time::ZERO {
        return invalid("Media has no length");
    }
    Ok(())
}

/// Reads every frame of the job's asset and measures the camera's path
/// through them, then keeps it in the job's cache. The path is the answer
/// whether or not it could be kept: the second value says why it was not.
pub(crate) fn steady_asset(
    job: &SteadyJob,
    processes: &Processes,
    control: &Control,
) -> Result<(Arc<Motion>, Option<String>), MediaError> {
    let asset = &job.asset;
    let rate = asset.frame_rate.unwrap_or_default();
    let frames = (rate.frame_at(Time(asset.duration.0.max(1) - 1)) + 1).max(1) as usize;
    if frames > MAX_STEADY_FRAMES {
        return Err(MediaError::Invalid(format!(
            "This video has more frames ({frames}) than a stabilization holds ({MAX_STEADY_FRAMES})"
        )));
    }
    // What the file is now, before it is read: a file that changes while it
    // is read is kept as what it was, and is stale the next time it is asked.
    let stamp = Stamp::of(asset);
    let mut luma = Luma::open(asset, 0, frames, processes)?;
    let mut camera = Camera::new();
    let mut said = 0.0;
    while camera.path.len() < frames {
        if !luma.next(control)? {
            break;
        }
        camera.push(&luma.gray).map_err(MediaError::Invalid)?;
        let done = camera.path.len() as f32 / frames as f32;
        if done - said >= 0.01 {
            said = done;
            control.progress(done);
        }
    }
    if camera.path.is_empty() {
        return Err(luma.failure("no frame could be read"));
    }
    let motion = Arc::new(camera.finish(rate));
    let unkept = (job.cache.store(asset, &stamp, &motion)).err();
    Ok((motion, unkept))
}

// ---- The cache -----------------------------------------------------------------

/// Where camera paths are kept between sessions: one file for a video's
/// path, named for the path of the video, in a directory held to a size and
/// a count. The least recently read are removed to make room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteadyCache {
    pub dir: PathBuf,
    pub bytes: u64,
    pub entries: usize,
}

/// What a video's file was when its path was measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    /// The file as the disk has it: nought for one that cannot be asked.
    length: u64,
    modified: u64,
    /// And as the project has it.
    file_size: u64,
    duration: i64,
    width: u32,
    height: u32,
    rate: [u32; 2],
}

impl Stamp {
    fn of(asset: &Asset) -> Self {
        let on_disk = fs::metadata(&asset.path).ok();
        let modified = (on_disk.as_ref())
            .and_then(|file| file.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos().min(u128::from(u64::MAX)) as u64);
        let rate = asset.frame_rate.unwrap_or_default();
        Self {
            length: on_disk.map_or(0, |file| file.len()),
            modified,
            file_size: asset.file_size,
            duration: asset.duration.0,
            width: asset.width,
            height: asset.height,
            rate: [rate.numerator, rate.denominator],
        }
    }

    fn write(&self, to: &mut Vec<u8>) {
        to.extend(self.length.to_le_bytes());
        to.extend(self.modified.to_le_bytes());
        to.extend(self.file_size.to_le_bytes());
        to.extend(self.duration.to_le_bytes());
        for word in [self.width, self.height, self.rate[0], self.rate[1]] {
            to.extend(word.to_le_bytes());
        }
    }
}

const MAGIC: &[u8; 8] = b"ASTRSTDY";
const EXTENSION: &str = "steady";

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

// What is read of a file's bytes, in order.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.0.split_at_checked(count)?;
        self.0 = rest;
        Some(taken)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

impl SteadyCache {
    /// A cache in `dir`, held to [`STEADY_CACHE_BYTES`] and
    /// [`STEADY_CACHE_ENTRIES`].
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            bytes: STEADY_CACHE_BYTES,
            entries: STEADY_CACHE_ENTRIES,
        }
    }

    fn name_of(path: &Path) -> Vec<u8> {
        path.as_os_str().as_encoded_bytes().to_vec()
    }

    /// The file a video's path is kept in: one for a video's place on disk,
    /// so that a file changed in place finds what was measured of it before.
    pub fn file(&self, asset: &Asset) -> PathBuf {
        self.dir.join(format!(
            "{:016x}.{EXTENSION}",
            fnv(&Self::name_of(&asset.path))
        ))
    }

    fn encode(asset: &Asset, stamp: &Stamp, motion: &Motion) -> Vec<u8> {
        let name = Self::name_of(&asset.path);
        let mut bytes = Vec::with_capacity(96 + name.len() + motion.path.len() * 16);
        bytes.extend(MAGIC);
        bytes.extend(STEADY_ALGORITHM.to_le_bytes());
        bytes.extend(super::ANALYSIS_SIDE.to_le_bytes());
        stamp.write(&mut bytes);
        bytes.extend((name.len() as u32).to_le_bytes());
        bytes.extend(&name);
        for word in [
            motion.rate.numerator,
            motion.rate.denominator,
            motion.unmeasured,
            motion.path.len() as u32,
        ] {
            bytes.extend(word.to_le_bytes());
        }
        for value in motion.path.iter().flatten() {
            bytes.extend(value.to_le_bytes());
        }
        let sum = fnv(&bytes);
        bytes.extend(sum.to_le_bytes());
        bytes
    }

    // The path a file holds, when it is whole, of this measurement and of
    // this video's place; and whether the video is still what it was.
    fn decode(bytes: &[u8], asset: &Asset, stamp: &Stamp) -> Option<Steadied> {
        let (body, sum) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
        if fnv(body) != u64::from_le_bytes(sum.try_into().ok()?) {
            return None;
        }
        let mut reader = Reader(body);
        if reader.take(8)? != MAGIC
            || reader.u32()? != STEADY_ALGORITHM
            || reader.u32()? != super::ANALYSIS_SIDE
        {
            return None;
        }
        let mut held = Vec::new();
        stamp.write(&mut held);
        let same = reader.take(held.len())? == held.as_slice();
        let name = reader.u32()? as usize;
        if reader.take(name)? != Self::name_of(&asset.path).as_slice() {
            return None;
        }
        if !same {
            return Some(Steadied::Stale);
        }
        let rate = FrameRate {
            numerator: reader.u32()?,
            denominator: reader.u32()?,
        };
        let unmeasured = reader.u32()?;
        let frames = reader.u32()? as usize;
        if !rate.validate() || frames == 0 || frames > MAX_STEADY_FRAMES {
            return None;
        }
        let values = reader.take(frames.checked_mul(16)?)?;
        if !reader.0.is_empty() {
            return None;
        }
        let path: Vec<[f32; 4]> = (values.chunks_exact(16))
            .map(|frame| {
                [0, 4, 8, 12].map(|at| {
                    f32::from_le_bytes([frame[at], frame[at + 1], frame[at + 2], frame[at + 3]])
                })
            })
            .collect();
        if !path.iter().flatten().all(|value| value.is_finite()) {
            return None;
        }
        Some(Steadied::Ready(Arc::new(Motion {
            rate,
            path,
            unmeasured,
        })))
    }

    /// What is kept of a video's path. A file that is damaged, of another
    /// measurement or of another video is as good as none; one read whole is
    /// marked as read now, so it is the last to be removed.
    pub fn load(&self, asset: &Asset) -> Steadied {
        let file = self.file(asset);
        // No path is larger than this: a larger file is not read at all.
        let most = 4096 + MAX_STEADY_FRAMES as u64 * 16;
        if !fs::metadata(&file).is_ok_and(|kept| kept.is_file() && kept.len() <= most) {
            return Steadied::Missing;
        }
        let Ok(bytes) = fs::read(&file) else {
            return Steadied::Missing;
        };
        let found = Self::decode(&bytes, asset, &Stamp::of(asset)).unwrap_or(Steadied::Missing);
        if matches!(found, Steadied::Ready(_))
            && let Ok(kept) = fs::File::options().write(true).open(&file)
        {
            let _ = kept.set_modified(SystemTime::now());
        }
        found
    }

    fn store(&self, asset: &Asset, stamp: &Stamp, motion: &Motion) -> Result<(), String> {
        let bytes = Self::encode(asset, stamp, motion);
        let failed = |error: std::io::Error| format!("{}: {error}", self.dir.display());
        fs::create_dir_all(&self.dir).map_err(failed)?;
        let file = self.file(asset);
        // Written whole beside its place and then moved into it, so a
        // reader never finds half of one.
        let beside = file.with_extension(format!("{EXTENSION}.{}", Uuid::new_v4().simple()));
        let written = fs::File::create(&beside)
            .and_then(|mut to| to.write_all(&bytes))
            .and_then(|()| fs::rename(&beside, &file));
        if let Err(error) = written {
            let _ = fs::remove_file(&beside);
            return Err(failed(error));
        }
        self.make_room(&file);
        Ok(())
    }

    /// The files kept, each with its size and when it was last read or
    /// written.
    fn kept(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
                    return None;
                }
                let file = entry.metadata().ok().filter(|file| file.is_file())?;
                Some((path, file.len(), file.modified().unwrap_or(UNIX_EPOCH)))
            })
            .collect()
    }

    /// The bytes and the files the directory holds.
    pub fn held(&self) -> (u64, usize) {
        let kept = self.kept();
        (kept.iter().map(|(_, bytes, _)| bytes).sum(), kept.len())
    }

    // Removes the files read longest ago until what is left is within the
    // bounds. `keep`, just written, is the last to go.
    fn make_room(&self, keep: &Path) {
        // What a write that never ended left beside its place.
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let old = (entry.metadata().ok())
                    .and_then(|file| file.modified().ok())
                    .and_then(|at| at.elapsed().ok())
                    .is_some_and(|age| age.as_secs() > 3600);
                if old && name.to_string_lossy().contains(&format!(".{EXTENSION}.")) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let mut kept = self.kept();
        kept.sort_by(|a, b| (a.0 == keep).cmp(&(b.0 == keep)).then(a.2.cmp(&b.2)));
        let mut bytes: u64 = kept.iter().map(|(_, bytes, _)| bytes).sum();
        let mut count = kept.len();
        for (path, size, _) in kept {
            if (bytes <= self.bytes && count <= self.entries) || path == keep {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                bytes -= size;
                count -= 1;
            }
        }
    }
}

#[cfg(test)]
#[path = "steady_tests.rs"]
mod tests;
