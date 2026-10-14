//! The filters of a voice's decoder: the one place FFmpeg filter syntax is
//! written, what each audio kind delays the sound by, and which kinds this
//! FFmpeg runs at all.
//!
//! A chain is read by FFmpeg once, when the decoder starts. The kinds are
//! proved by running each one's own string, never by a listing: a build can
//! name a filter and still refuse an option of it.
use crate::{
    MediaError,
    process::{Processes, capture, tool_command},
};
use editor_core::{Clip, Effect, EffectKind, ParamKey, sound};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU32, Ordering},
};

/// The kinds that shape sound, in the order they are probed.
const AUDIO_KINDS: [EffectKind; 7] = [
    EffectKind::HighPass,
    EffectKind::EqBand,
    EffectKind::Compressor,
    EffectKind::Limiter,
    EffectKind::Denoise,
    EffectKind::Pitch,
    EffectKind::CenterCut,
];
/// The filter the probe's control run uses: one every build has.
const CONTROL: &str = "anull";
/// The rate of the probe's `anullsrc`, FFmpeg's default for it.
const PROBE_RATE: u32 = 44_100;
/// An `atempo` factor this close to 1 is left out of the chain.
const TEMPO_UNITY: f64 = 0.001;
/// A filter's frequency stays under this part of the output rate, short of
/// the half where a biquad stops being one.
const TOP_OF_BAND: f64 = 0.45;
/// The bit of `Filters::said` that says nothing could be probed.
const UNKNOWN: u32 = 1 << AUDIO_KINDS.len();
/// The bit of [`left_out`] and of `Filters::said` for centre reduction that
/// an output of one channel went without.
const ONE_CHANNEL: u32 = UNKNOWN << 1;

// The compressor's time constants, in milliseconds. They are the spec's
// first values: the readback tests hold with them, and nobody has listened.
const COMPRESSOR_ATTACK_MS: u32 = 10;
const COMPRESSOR_RELEASE_MS: u32 = 150;
// `Denoise` amount 0..1 is `afftdn`'s `nr` from 6 to 36 dB.
const DENOISE_LEAST_DB: f64 = 6.0;
const DENOISE_SPAN_DB: f64 = 30.0;

fn bit(kind: EffectKind) -> u32 {
    (AUDIO_KINDS.iter().position(|audio| *audio == kind)).map_or(0, |index| 1 << index)
}

/// What a kind's filter delays the sound by, in microseconds, whatever the
/// rate. Only delays that do not move are here: measured on FFmpeg 8.1.2
/// with a 5 ms tone burst, `afftdn` 25.0 ms and `alimiter` 5.0 ms at 44.1
/// and 48 kHz, the others nothing. The `atempo` of the speed stage is not
/// here: its sound arrives early, not late, by an amount that moves, and
/// dropping frames can only take off a delay.
const fn delay_micros(kind: EffectKind) -> u64 {
    match kind {
        EffectKind::Denoise => 25_000,
        EffectKind::Limiter => 5_000,
        _ => 0,
    }
}

/// Which audio kinds this FFmpeg runs, and what has been said about the
/// ones it does not.
#[derive(Debug, Default)]
pub(crate) struct Filters {
    /// Kinds whose own filter string FFmpeg refused.
    missing: Vec<EffectKind>,
    /// The control run failed, so nothing is known and every kind counts
    /// as available.
    unknown: bool,
    /// A bit for each kind already said to be left out, [`UNKNOWN`] and
    /// [`ONE_CHANNEL`].
    said: AtomicU32,
}

impl Filters {
    pub(crate) fn has(&self, kind: EffectKind) -> bool {
        !self.missing.contains(&kind)
    }

    /// What is still to be said, once a session each: that nothing could be
    /// probed, each kind of `left_out` (bits of [`left_out`]) and that one
    /// channel has no centre to reduce.
    pub(crate) fn unsaid(&self, left_out: u32) -> Vec<(u32, String)> {
        let said = self.said.load(Ordering::Acquire);
        let mut notices = Vec::new();
        if self.unknown && said & UNKNOWN == 0 {
            notices.push((
                UNKNOWN,
                "Audio: FFmpeg's filters could not be checked; audio effects are taken as available"
                    .to_owned(),
            ));
        }
        for kind in AUDIO_KINDS {
            if left_out & bit(kind) & !said != 0 {
                notices.push((
                    bit(kind),
                    format!("Audio: {} is not available in this FFmpeg", name(kind)),
                ));
            }
        }
        if left_out & ONE_CHANNEL & !said != 0 {
            notices.push((
                ONE_CHANNEL,
                "Audio: centre reduction needs two output channels and is not heard on this device"
                    .to_owned(),
            ));
        }
        notices
    }

    /// A notice of [`Filters::unsaid`] reached the person.
    pub(crate) fn mark_said(&self, notice: u32) {
        self.said.fetch_or(notice, Ordering::AcqRel);
    }
}

fn name(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::HighPass => "the high-pass filter",
        EffectKind::EqBand => "the equalizer band",
        EffectKind::Compressor => "the compressor",
        EffectKind::Limiter => "the limiter",
        EffectKind::Denoise => "noise reduction",
        EffectKind::Pitch => "pitch",
        EffectKind::CenterCut => "centre reduction",
        _ => "that effect",
    }
}

/// One audio effect as FFmpeg reads it, for output at `rate`. Numbers have a
/// fixed precision and no locale. Decibels become linear where the filter
/// wants linear. `Pitch` is no filter of its own: it is the speed stage.
fn filter(effect: &Effect, rate: u32) -> Option<String> {
    let value = |key| f64::from(effect.param(key).unwrap_or_default());
    let linear = |decibels: f64| 10_f64.powf(decibels / 20.0);
    let hertz = |frequency: f64| frequency.min(f64::from(rate) * TOP_OF_BAND);
    Some(match effect.kind {
        EffectKind::HighPass => format!("highpass=f={:.3}", hertz(value(ParamKey::Frequency))),
        EffectKind::EqBand => format!(
            "equalizer=f={:.3}:t=o:w={:.3}:g={:.3}",
            hertz(value(ParamKey::Frequency)),
            value(ParamKey::Width),
            value(ParamKey::Gain)
        ),
        EffectKind::Compressor => format!(
            "acompressor=threshold={:.6}:ratio={:.3}:attack={COMPRESSOR_ATTACK_MS}\
             :release={COMPRESSOR_RELEASE_MS}:makeup={:.6}",
            linear(value(ParamKey::Threshold)),
            value(ParamKey::Amount),
            linear(value(ParamKey::Gain))
        ),
        EffectKind::Limiter => format!(
            "alimiter=limit={:.6}:level=false",
            linear(value(ParamKey::Threshold))
        ),
        // No noise tracking: with it the reduction measured under 1 dB
        // whatever `nr` was.
        EffectKind::Denoise => format!(
            "afftdn=nr={:.3}:nf={:.3}",
            DENOISE_LEAST_DB + DENOISE_SPAN_DB * value(ParamKey::Amount),
            value(ParamKey::Threshold)
        ),
        EffectKind::CenterCut => {
            let amount = value(ParamKey::Amount);
            format!(
                "aformat=channel_layouts=stereo,pan=stereo|c0=c0-{amount:.4}*c1|c1=c1-{amount:.4}*c0"
            )
        }
        _ => return None,
    })
}

/// Whether an output of `channels` has no place for `kind`. Centre reduction
/// leaves two channels that are each other turned over wherever the source's
/// two are alike, and at its full amount everywhere; FFmpeg's sum of them
/// into one channel is `1 - amount` of the plain sum, silence at the default.
fn unfit(kind: EffectKind, channels: usize) -> bool {
    kind == EffectKind::CenterCut && channels == 1
}

/// The enabled audio effects of a stack that this FFmpeg runs and an output
/// of `channels` can carry, in stack order.
fn heard<'a>(
    stack: &'a [Effect],
    channels: usize,
    filters: &'a Filters,
) -> impl Iterator<Item = &'a Effect> {
    stack.iter().filter(move |effect| {
        effect.enabled
            && effect.kind.is_audio()
            && filters.has(effect.kind)
            && !unfit(effect.kind, channels)
    })
}

/// The enabled audio effects of `clip` that are left out of its chain, as
/// bits for [`Filters::unsaid`]: the kinds this FFmpeg does not run, and
/// [`ONE_CHANNEL`] for centre reduction on an output of one channel.
pub(crate) fn left_out(clip: &Clip, channels: usize, filters: &Filters) -> u32 {
    (clip.effects.iter())
        .filter(|effect| effect.enabled && effect.kind.is_audio())
        .fold(0, |bits, effect| {
            if !filters.has(effect.kind) {
                bits | bit(effect.kind)
            } else if unfit(effect.kind, channels) {
                bits | ONE_CHANNEL
            } else {
                bits
            }
        })
}

/// The effects part of a chain alone, for output at `rate` in `channels`:
/// no speed stage and no `Pitch`. Empty when the clip has none to run.
pub(crate) fn effects(clip: &Clip, rate: u32, channels: usize, filters: &Filters) -> String {
    stack_effects(&clip.effects, rate, channels, filters)
}

/// [`effects`] of a clip's stack handed over without its clip, as the sound
/// lane's loudness measurement has it.
pub(crate) fn stack_effects(
    stack: &[Effect],
    rate: u32,
    channels: usize,
    filters: &Filters,
) -> String {
    let parts: Vec<String> = heard(stack, channels, filters)
        .filter_map(|effect| filter(effect, rate))
        .collect();
    parts.join(",")
}

/// The two factors of a voice's speed stage for a clip at `clip_speed`: `k`,
/// by which `asetrate` changes speed and pitch together, and the `atempo`
/// factor `a`, which changes speed alone, when there is one. `k · a` is always
/// the clip's speed.
///
/// Without a `Pitch` this is the stage there always was: `k` is the speed and
/// there is no `atempo`. With one of `semitones`, its ratio `P` is taken out
/// of the speed and `a` is what is left, held to what one stage takes
/// ([`sound::PITCH_KEPT`]). While that hold does not bite, `k` is `P` and the
/// pitch is the one asked for; beyond it the remainder moves the pitch. One
/// stage and never a chain of them: each one makes the sound arrive earlier,
/// by an amount that moves from one event to the next and is not taken off.
/// An `a` within [`TEMPO_UNITY`] of 1 is no stage, and `k` is then the speed,
/// so the clip keeps its length and its pitch is off by under 2 cents.
pub(crate) fn speed_stage(clip_speed: f64, semitones: Option<f32>) -> (f64, Option<f64>) {
    let Some(semitones) = semitones else {
        return (clip_speed, None);
    };
    let (least, most) = (*sound::PITCH_KEPT.start(), *sound::PITCH_KEPT.end());
    let tempo = (clip_speed / sound::pitch_ratio(semitones)).clamp(least, most);
    if (tempo - 1.0).abs() <= TEMPO_UNITY {
        return (clip_speed, None);
    }
    (clip_speed / tempo, Some(tempo))
}

/// The `-af` of a voice: the speed stage, then the clip's audio effects in
/// stack order, so a frequency is the one heard, for output at `rate` in
/// `channels`. A clip without audio effects gets the speed stage alone. The
/// first enabled `Pitch` that this FFmpeg runs is the speed stage's
/// ([`speed_stage`]); the transport's speed always shifts pitch.
pub(crate) fn chain(
    clip: &Clip,
    clip_speed: f64,
    transport: f64,
    rate: u32,
    channels: usize,
    filters: &Filters,
) -> Result<String, MediaError> {
    let semitones = sound::pitch(clip).filter(|_| filters.has(EffectKind::Pitch));
    let (changes, tempo) = speed_stage(clip_speed, semitones);
    let changed_rate = (f64::from(rate) * (transport * changes)).round();
    if !(100.0..=1_536_000.0).contains(&changed_rate) {
        return Err(MediaError::Invalid(
            "Clip and preview speeds exceed the audio resampling range".into(),
        ));
    }
    // The first byte read is the sound at the seek point in the file's time,
    // as the picture is read. Where the stream's first sample comes later (a
    // late audio stream, a seek landing on a packet past the point, the
    // lead-in Opus drops) the gap is silence rather than the sound being
    // pulled early. Gaps of 0.1 s or more later in the stream are filled the
    // same way.
    let mut chain = format!(
        "aresample={rate}:async=1:first_pts=0,asetrate={},aresample={rate}",
        changed_rate as u32
    );
    if let Some(tempo) = tempo {
        chain.push_str(&format!(",atempo={tempo:.6}"));
    }
    let effects = effects(clip, rate, channels, filters);
    if !effects.is_empty() {
        chain.push(',');
        chain.push_str(&effects);
    }
    Ok(chain)
}

/// How many leading frames of a voice at `rate` are the delay of its chain
/// and not its sound.
pub(crate) fn latency_frames(clip: &Clip, rate: u32, channels: usize, filters: &Filters) -> usize {
    let micros: u64 = heard(&clip.effects, channels, filters)
        .map(|effect| delay_micros(effect.kind))
        .sum();
    (micros as f64 * f64::from(rate) / 1_000_000.0).round() as usize
}

/// What the probe runs for a kind: its own string at its defaults, and one
/// `atempo` stage for `Pitch`.
fn probe_string(kind: EffectKind) -> String {
    filter(&Effect::new(kind), PROBE_RATE).unwrap_or_else(|| "atempo=1.5".into())
}

/// Finds which kinds run. `runs` says whether FFmpeg ran a filter string to
/// the end. A control comes first: when even that fails (no FFmpeg, no
/// `lavfi`) nothing is known and no kind is held to be missing.
pub(crate) fn probe(mut runs: impl FnMut(&str) -> bool) -> Filters {
    if !runs(CONTROL) {
        return Filters {
            unknown: true,
            ..Filters::default()
        };
    }
    Filters {
        missing: (AUDIO_KINDS.into_iter())
            .filter(|kind| !runs(&probe_string(*kind)))
            .collect(),
        ..Filters::default()
    }
}

/// Runs one filter string over 50 ms of silence and says whether FFmpeg
/// ended well. A refused option ends it with a status and no output.
pub(crate) fn runs(processes: &Processes, filter: &str) -> bool {
    let mut command = tool_command("ffmpeg");
    command.args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "anullsrc",
        "-t",
        "0.05",
        "-af",
        filter,
        "-f",
        "null",
        "-",
    ]);
    capture("ffmpeg", &mut command, processes, 0).is_ok()
}

static SESSION: OnceLock<Arc<Filters>> = OnceLock::new();

/// This session's answer, probed by the first caller: eight short FFmpeg
/// runs that later callers wait for. Its processes are its own, so nothing
/// that stops a service's children stops the probe. Never call it on the
/// UI thread.
pub(crate) fn session() -> Arc<Filters> {
    Arc::clone(SESSION.get_or_init(|| {
        let processes = Processes::default();
        Arc::new(probe(|filter| runs(&processes, filter)))
    }))
}

/// The audio kinds this FFmpeg does not run, for the interface to show as
/// unavailable. `None` until the session's probe has finished; it starts
/// with the sound worker and this never waits for it. Empty when nothing
/// could be probed.
pub fn missing_audio_filters() -> Option<Vec<EffectKind>> {
    SESSION.get().map(|filters| filters.missing.clone())
}

#[cfg(test)]
impl Filters {
    /// A session in which FFmpeg refused `missing`.
    pub(crate) fn without(missing: &[EffectKind]) -> Self {
        Self {
            missing: missing.to_vec(),
            ..Self::default()
        }
    }

    pub(crate) fn missing(&self) -> &[EffectKind] {
        &self.missing
    }

    pub(crate) fn unknown(&self) -> bool {
        self.unknown
    }
}

#[cfg(test)]
pub(crate) fn probe_strings() -> Vec<String> {
    AUDIO_KINDS.into_iter().map(probe_string).collect()
}

#[cfg(test)]
pub(crate) fn bit_of(kind: EffectKind) -> u32 {
    bit(kind)
}

#[cfg(test)]
pub(crate) const ONE_CHANNEL_BIT: u32 = ONE_CHANNEL;
