//! Frame-accurate time for typed tools. A time is given as a frame of the
//! project's rate, seconds, microseconds or a timecode, and every time a
//! tool reports carries all four. Core commands (`edit_apply`) keep the
//! core's microseconds.
use super::errors::ToolError;
use editor_core::{FrameRate, Time};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A point or length on the timeline. Give exactly one field. Points are
/// snapped to the nearest start of a project frame.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeSpec {
    /// A frame index at the project's frame rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// Microseconds, the core's own unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub us: Option<i64>,
    /// `HH:MM:SS:FF`, non-drop, at the project's rate rounded to whole
    /// frames per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timecode: Option<String>,
}

impl TimeSpec {
    #[cfg(test)]
    pub fn seconds(seconds: f64) -> Self {
        Self {
            seconds: Some(seconds),
            ..Self::default()
        }
    }

    /// The exact time asked for, not snapped.
    pub fn exact(&self, rate: FrameRate) -> Result<Time, ToolError> {
        let given = [
            self.frame.is_some(),
            self.seconds.is_some(),
            self.us.is_some(),
            self.timecode.is_some(),
        ];
        if given.iter().filter(|given| **given).count() != 1 {
            return Err(ToolError::invalid(
                "A time takes exactly one of frame, seconds, us or timecode",
            ));
        }
        let time = if let Some(frame) = self.frame {
            rate.frame_time(frame)
        } else if let Some(seconds) = self.seconds {
            if !seconds.is_finite() {
                return Err(ToolError::invalid("seconds must be a finite number"));
            }
            Time::from_seconds(seconds)
        } else if let Some(us) = self.us {
            Time(us)
        } else {
            let text = self.timecode.as_deref().unwrap_or_default();
            rate.frame_time(timecode_frames(text, rate)?)
        };
        if !time.is_valid() {
            return Err(ToolError::invalid(format!(
                "{} is outside the timeline",
                time.as_seconds()
            )));
        }
        Ok(time)
    }

    /// The time snapped to the nearest frame start, as the window snaps.
    pub fn at(&self, rate: FrameRate) -> Result<Time, ToolError> {
        Ok(crate::rules::quantize(self.exact(rate)?, rate))
    }

    /// A length of at least one frame, snapped to whole frames.
    pub fn length(&self, rate: FrameRate) -> Result<Time, ToolError> {
        let time = self.at(rate)?;
        if time <= Time::ZERO {
            return Err(ToolError::invalid("A length must be at least one frame"));
        }
        Ok(time)
    }
}

fn nominal(rate: FrameRate) -> i64 {
    (rate.fps().round() as i64).max(1)
}

fn timecode_frames(text: &str, rate: FrameRate) -> Result<i64, ToolError> {
    if text.contains(';') {
        return Err(ToolError::invalid(format!(
            "“{text}” is drop-frame timecode, which is not read; write non-drop HH:MM:SS:FF, \
             or give frame or seconds"
        )));
    }
    let parts: Vec<&str> = text.trim().split(':').collect();
    let numbers: Option<Vec<i64>> = parts.iter().map(|part| part.parse().ok()).collect();
    let (Some(numbers), 4) = (numbers, parts.len()) else {
        return Err(ToolError::invalid(format!(
            "“{text}” is not a timecode; write HH:MM:SS:FF"
        )));
    };
    let fps = nominal(rate);
    let (h, m, s, f) = (numbers[0], numbers[1], numbers[2], numbers[3]);
    if !(0..100_000).contains(&h)
        || !(0..60).contains(&m)
        || !(0..60).contains(&s)
        || !(0..fps).contains(&f)
    {
        return Err(ToolError::invalid(format!(
            "“{text}” is out of range at {fps} frames a second"
        )));
    }
    Ok(((h * 60 + m) * 60 + s) * fps + f)
}

/// A time as every tool reports it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
pub struct TimeOut {
    pub us: i64,
    /// The project frame this time falls in.
    pub frame: i64,
    pub seconds: f64,
    #[serde(skip)]
    rate: FrameRate,
}

impl TimeOut {
    pub fn new(time: Time, rate: FrameRate) -> Self {
        Self {
            us: time.0,
            frame: rate.frame_at(time),
            seconds: time.as_seconds(),
            rate,
        }
    }
}

/// `HH:MM:SS:FF` of a frame, non-drop.
pub fn timecode(frame: i64, rate: FrameRate) -> String {
    let fps = nominal(rate);
    let frame = frame.max(0);
    let (seconds, f) = (frame / fps, frame % fps);
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60,
        f
    )
}

/// A time with its timecode, for results an agent reads.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Stamp {
    pub us: i64,
    pub frame: i64,
    pub seconds: f64,
    pub timecode: String,
}

impl Stamp {
    pub fn new(time: Time, rate: FrameRate) -> Self {
        let out = TimeOut::new(time, rate);
        Self {
            us: out.us,
            frame: out.frame,
            seconds: out.seconds,
            timecode: timecode(out.frame, out.rate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NTSC: FrameRate = FrameRate {
        numerator: 30_000,
        denominator: 1001,
    };

    #[test]
    fn every_form_names_the_same_frame() {
        let rate = FrameRate {
            numerator: 25,
            denominator: 1,
        };
        let forms = [
            TimeSpec {
                frame: Some(50),
                ..TimeSpec::default()
            },
            TimeSpec::seconds(2.0),
            TimeSpec {
                us: Some(2_000_000),
                ..TimeSpec::default()
            },
            TimeSpec {
                timecode: Some("00:00:02:00".into()),
                ..TimeSpec::default()
            },
        ];
        for form in forms {
            assert_eq!(form.at(rate).unwrap(), Time(2_000_000), "{form:?}");
        }
    }

    #[test]
    fn a_time_inside_a_frame_snaps_to_that_frame_and_round_trips() {
        let inside = TimeSpec::seconds(1.01).at(NTSC).unwrap();
        let frame = NTSC.frame_at(inside);
        assert_eq!(inside, NTSC.frame_time(frame));
        assert_eq!(Stamp::new(inside, NTSC).frame, frame);
        assert_eq!(timecode(frame, NTSC), "00:00:01:00");
    }

    #[test]
    fn a_time_needs_exactly_one_form_and_a_valid_timecode() {
        let rate = FrameRate::default();
        assert!(TimeSpec::default().at(rate).is_err());
        let both = TimeSpec {
            frame: Some(1),
            seconds: Some(1.0),
            ..TimeSpec::default()
        };
        assert!(both.at(rate).is_err());
        for bad in ["1:2:3", "00:00:00:30", "00:61:00:00", "aa:00:00:00"] {
            let spec = TimeSpec {
                timecode: Some(bad.into()),
                ..TimeSpec::default()
            };
            assert!(spec.at(rate).is_err(), "{bad}");
        }
        assert!(TimeSpec::seconds(-1.0).at(rate).is_err());
        assert!(TimeSpec::seconds(0.0).length(rate).is_err());
    }
}
