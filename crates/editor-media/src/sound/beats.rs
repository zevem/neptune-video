//! The beats of a media file's sound, found in its peaks: where an energy
//! onset detector at the peaks' 10 ms hears a steady pulse.
//!
//! Three steps, after Ellis (2007), "Beat tracking by dynamic programming".
//! An onset envelope, from how much the low level and the whole level rise
//! from one bucket to the next. A tempo, the period at which that envelope
//! best repeats, weighted towards 120 BPM. And the beats, the onsets about one
//! period apart that score best together. Sound without a clear period has
//! no beats. Pure: nothing is read, written or kept.
use super::peaks::{PEAKS_PER_SECOND, Peak, Peaks};

/// The beats a grid holds at most: 82 minutes at 200 BPM. Later beats are
/// not given.
pub const MAX_BEATS: usize = 16_384;

/// The shortest and the longest period looked for, in buckets: 200 and
/// 60 BPM.
const MIN_LAG: usize = 30;
const MAX_LAG: usize = 100;
/// The period a tempo is weighted towards, 120 BPM, and how far either way
/// the weight halves at about 1.2 octaves.
const PRIOR_LAG: f64 = 50.0;
const PRIOR_OCTAVES: f64 = 1.0;
/// Half of the moving mean the envelope is taken from: 1 s in all.
const MEAN_REACH: usize = 50;
/// How much of its own power the envelope must keep one period on for the
/// period to be clear, and at least this many standard errors of noise.
const CLEAR: f64 = 0.1;
const NOISE_ERRORS: f64 = 4.0;
/// How dearly a beat pays for falling off the period.
const TIGHTNESS: f32 = 100.0;
/// A bucket's length in milliseconds.
const BUCKET_MS: u32 = 1000 / PEAKS_PER_SECOND;

/// A steady pulse found in a file's sound.
#[derive(Clone, Debug, PartialEq)]
pub struct BeatGrid {
    /// The tempo, in beats a minute: 60 to 200. A pulse faster than about
    /// 170 BPM may read as half its tempo.
    pub bpm: f32,
    /// When each beat falls, in milliseconds of container time, earliest
    /// first: the middle of the bucket the beat's onset is in. At most
    /// [`MAX_BEATS`].
    pub beats: Vec<u32>,
}

/// The beats of a file's peaks, or none when its sound has no clear period:
/// silence, a steady sound, noise, or a file too short to hold four periods.
pub fn beats(peaks: &Peaks) -> Option<BeatGrid> {
    let envelope = envelope(&peaks.buckets);
    let period = tempo(&envelope)?;
    let found = track(envelope, period)?;
    Some(BeatGrid {
        bpm: (60.0 * f64::from(PEAKS_PER_SECOND) / period) as f32,
        beats: (found.into_iter())
            .map(|bucket| bucket as u32 * BUCKET_MS + BUCKET_MS / 2)
            .collect(),
    })
}

/// How much each bucket's sound rises over the one before, in dB: the low
/// level's rise and half of the whole level's, none for a fall, less the
/// mean of that over the second around it. Before the file is silence, so
/// a file that starts on a beat has it in its first bucket.
fn envelope(buckets: &[[u8; 4]]) -> Vec<f32> {
    let rise = |from: u8, to: u8| (Peak::decibels(to) - Peak::decibels(from)).max(0.0);
    let onsets: Vec<f32> = std::iter::once(&[0; 4])
        .chain(buckets)
        .zip(buckets)
        .map(|(before, bucket)| rise(before[3], bucket[3]) + 0.5 * rise(before[2], bucket[2]))
        .collect();
    // The window's sum, moved one bucket at a time.
    let count = onsets.len();
    let mut sum: f64 = onsets[..MEAN_REACH.min(count)]
        .iter()
        .map(|&o| f64::from(o))
        .sum();
    let mut envelope = Vec::with_capacity(count);
    for (at, onset) in onsets.iter().enumerate() {
        if let Some(entering) = onsets.get(at + MEAN_REACH) {
            sum += f64::from(*entering);
        }
        if let Some(leaving) = at.checked_sub(MEAN_REACH + 1) {
            sum -= f64::from(onsets[leaving]);
        }
        let width = (at + MEAN_REACH + 1).min(count) - at.saturating_sub(MEAN_REACH);
        envelope.push(onset - (sum / width as f64) as f32);
    }
    envelope
}

/// The envelope's correlation with itself `lag` buckets on, per bucket of
/// overlap.
fn correlation(envelope: &[f32], lag: usize) -> f64 {
    let overlap = envelope.len() - lag;
    let sum: f64 = (envelope.iter().zip(&envelope[lag..]))
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum();
    sum / overlap as f64
}

/// The period the envelope repeats at most clearly, in buckets and parts of
/// one, weighted towards [`PRIOR_LAG`]. None when no period is clear.
fn tempo(envelope: &[f32]) -> Option<f64> {
    // Four periods at least are in the file.
    let longest = MAX_LAG.min(envelope.len() / 4);
    if longest < MIN_LAG {
        return None;
    }
    let power = correlation(envelope, 0);
    if power <= f64::EPSILON {
        return None;
    }
    // One lag either side of the range, for the peak's neighbours.
    let first = MIN_LAG - 1;
    let shared: Vec<f64> = (first..=longest + 1)
        .map(|lag| correlation(envelope, lag) / power)
        .collect();
    let at = |lag: usize| shared[lag - first];
    let weight = |lag: usize| {
        let octaves = (lag as f64 / PRIOR_LAG).log2() / PRIOR_OCTAVES;
        (-0.5 * octaves * octaves).exp()
    };
    // A period between two buckets splits its correlation between them;
    // the lag is chosen with half of each neighbour's added, or a multiple
    // that falls on a bucket would win over it, as 100 over 33.3 at 180 BPM.
    let spread = |lag: usize| at(lag) + 0.5 * (at(lag - 1) + at(lag + 1));
    let best = (MIN_LAG..=longest)
        .filter(|&lag| at(lag) > 0.0)
        .max_by(|&a, &b| (spread(a) * weight(a)).total_cmp(&(spread(b) * weight(b))))?;
    // What noise of this length would reach by chance is not a period.
    let noise = NOISE_ERRORS / ((envelope.len() - best) as f64).sqrt();
    if at(best) < CLEAR.max(noise) {
        return None;
    }
    // Between the buckets: the top of the parabola through the peak and its
    // two neighbours, kept within 60 to 200 BPM.
    let (before, peak, after) = (at(best - 1), at(best), at(best + 1));
    let bend = before - 2.0 * peak + after;
    let shift = if bend < 0.0 {
        (0.5 * (before - after) / bend).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some((best as f64 + shift).clamp(MIN_LAG as f64, MAX_LAG as f64))
}

/// The buckets the beats are in, earliest first: the path of onsets about
/// `period` apart that scores best, without the weak beats at either end.
fn track(envelope: Vec<f32>, period: f64) -> Option<Vec<usize>> {
    let count = envelope.len();
    let spread = (correlation(&envelope, 0).sqrt()) as f32;
    if spread <= 0.0 {
        return None;
    }
    // Each bucket's own score: the envelope in units of its spread, smoothed
    // by a bell a thirty-second of a period wide, so that an onset a bucket
    // off the period still scores.
    let width = period / 32.0;
    let reach = (4.0 * width).ceil() as usize;
    let bell: Vec<f32> = (0..=2 * reach)
        .map(|index| {
            let off = (index as f64 - reach as f64) / width;
            ((-0.5 * off * off).exp()) as f32 / spread
        })
        .collect();
    let local: Vec<f32> = (0..count)
        .map(|at| {
            let from = at.saturating_sub(reach);
            let to = (at + reach + 1).min(count);
            (envelope[from..to].iter())
                .zip(&bell[from + reach - at..])
                .map(|(value, weight)| value * weight)
                .sum()
        })
        .collect();
    drop(envelope);

    // The best score of a path of beats ending at each bucket, and how far
    // back the beat before it is; 0 for the first of its path. A beat
    // follows another from half a period to two periods on, and pays for
    // the distance from one period.
    let near = ((period / 2.0).round() as usize).max(1);
    let far = (2.0 * period).round() as usize;
    debug_assert!(far <= usize::from(u8::MAX));
    let cost: Vec<f32> = (0..=far)
        .map(|distance| {
            let off = (distance as f64 / period).ln() as f32;
            -TIGHTNESS * off * off
        })
        .collect();
    let mut score = vec![0.0f32; count];
    let mut back = vec![0u8; count];
    for at in 0..count {
        // A path is worth following only while it scores above nothing.
        let (mut best, mut from) = (0.0f32, 0u8);
        for distance in near..=far.min(at) {
            let through = score[at - distance] + cost[distance];
            if through > best {
                (best, from) = (through, distance as u8);
            }
        }
        score[at] = local[at] + best;
        back[at] = from;
    }

    // The path ends at its best score within the last two periods.
    let tail = count.saturating_sub(far + 1);
    let mut at = (tail..count).max_by(|&a, &b| score[a].total_cmp(&score[b]).then(b.cmp(&a)))?;
    let mut found = vec![at];
    while back[at] != 0 {
        at -= usize::from(back[at]);
        found.push(at);
    }
    found.reverse();

    // Beats at either end where nothing is heard are the path running on
    // through silence: under half the typical beat's score, they go.
    let typical =
        (found.iter().map(|&at| local[at] * local[at]).sum::<f32>() / found.len() as f32).sqrt();
    let weak = |at: &usize| local[*at] < 0.5 * typical;
    let first = found.iter().position(|at| !weak(at))?;
    let last = found.iter().rposition(|at| !weak(at))?;
    let mut found = found[first..=last].to_vec();
    if found.len() < 2 {
        return None;
    }
    found.truncate(MAX_BEATS);
    Some(found)
}
