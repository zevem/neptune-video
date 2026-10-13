use crate::{EditError, FrameRate, Time};
use std::time::Instant;

/// Monotonic playback clock. Decode and audio device lifetime belong to runtime.
pub struct Playback {
    pub position: Time,
    pub playing: bool,
    pub speed: f64,
    anchor: Option<(Instant, Time)>,
}
impl Default for Playback {
    fn default() -> Self {
        Self {
            position: Time::ZERO,
            playing: false,
            speed: 1.0,
            anchor: None,
        }
    }
}
impl Playback {
    pub fn play(&mut self) {
        self.playing = true;
        self.anchor = None;
    }
    pub fn pause(&mut self) {
        self.playing = false;
        self.anchor = None;
    }
    pub fn seek(&mut self, time: Time) {
        self.position = time.max(Time::ZERO);
        self.anchor = None;
    }
    pub fn step(&mut self, frames: i64, rate: FrameRate) {
        self.pause();
        self.position = rate.frame_time(rate.frame_at(self.position).saturating_add(frames).max(0));
    }
    pub fn set_speed(&mut self, speed: f64) -> Result<(), EditError> {
        if !speed.is_finite() || !(0.1..=8.0).contains(&speed) {
            return Err(EditError::Invalid("playback speed".into()));
        }
        self.speed = speed;
        self.anchor = None;
        Ok(())
    }
    pub fn tick(&mut self, now: Instant, duration: Time) -> bool {
        if !self.playing {
            return false;
        }
        let (anchor, position) = *self.anchor.get_or_insert((now, self.position));
        let elapsed = now.saturating_duration_since(anchor).as_secs_f64();
        let next =
            (position + Time::from_seconds(elapsed * self.speed)).min(duration.max(Time::ZERO));
        let changed = next != self.position;
        self.position = next;
        if next >= duration {
            self.pause();
        }
        changed
    }
    /// Use the audio device clock when available, holding the current position
    /// if a delayed first buffer trails the fallback clock. Explicit seek starts
    /// a new interval. A matching anchor permits fallback after device failure.
    pub fn tick_with_audio(
        &mut self,
        now: Instant,
        duration: Time,
        audio_position: Option<Time>,
    ) -> bool {
        let Some(position) = audio_position else {
            return self.tick(now, duration);
        };
        if !self.playing {
            return false;
        }
        let next = position
            .max(self.position)
            .max(Time::ZERO)
            .min(duration.max(Time::ZERO));
        let changed = next != self.position;
        self.position = next;
        self.anchor = Some((now, next));
        if next >= duration {
            self.pause();
        }
        changed
    }
}
