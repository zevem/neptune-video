//! How times are written and read in the interface.
use editor_core::{FrameRate, Time};

/// `HH:MM:SS:FF` at the project's frame rate.
pub fn timecode(time: Time, rate: FrameRate) -> String {
    let frame = rate.frame_at(time.max(Time::ZERO));
    // Frames within a second are counted against the rounded rate, as
    // non-drop-frame timecode does.
    let per_second = rate.fps().round().max(1.0) as i64;
    let seconds = frame / per_second;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        frame % per_second
    )
}

/// A compact duration for badges and lists: `0:07`, `12:30`, `1:02:03`.
pub fn short(time: Time) -> String {
    let seconds = (time.as_seconds().max(0.0)).round() as i64;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// A ruler label for a tick at `seconds`, with frames when ticks are closer
/// than a second.
pub fn ruler(seconds: f64, step: f64, rate: FrameRate) -> String {
    let whole = seconds.floor() as i64;
    let base = if whole >= 3600 {
        format!("{}:{:02}:{:02}", whole / 3600, whole / 60 % 60, whole % 60)
    } else {
        format!("{}:{:02}", whole / 60, whole % 60)
    };
    if step < 1.0 {
        let frame = ((seconds - whole as f64) * rate.fps()).round() as i64;
        if frame != 0 {
            return format!("{frame}f");
        }
    }
    base
}

/// Reads a time a person typed: `12`, `12.5s`, `500ms`, `48f`, `1:23`,
/// `1:02:03` or `00:01:02:15` (the last field of four is frames).
pub fn parse(text: &str, rate: FrameRate) -> Option<Time> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if let Some(value) = lower.strip_suffix("ms") {
        return finite(value).map(|ms| Time::from_seconds(ms / 1000.0));
    }
    if let Some(value) = lower.strip_suffix('f') {
        return value.trim().parse::<i64>().ok().map(|f| rate.frame_time(f));
    }
    if let Some(value) = lower.strip_suffix('m') {
        return finite(value).map(|m| Time::from_seconds(m * 60.0));
    }
    let lower = lower.strip_suffix('s').unwrap_or(&lower);
    let fields: Vec<&str> = lower.split(':').collect();
    let seconds = match fields.as_slice() {
        [s] => finite(s)?,
        [m, s] => finite(m)? * 60.0 + finite(s)?,
        [h, m, s] => finite(h)? * 3600.0 + finite(m)? * 60.0 + finite(s)?,
        [h, m, s, f] => {
            finite(h)? * 3600.0 + finite(m)? * 60.0 + finite(s)? + finite(f)? / rate.fps()
        }
        _ => return None,
    };
    Some(Time::from_seconds(seconds))
}

fn finite(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// How long ago something happened, in the coarsest unit that says it.
pub fn age(seconds: u64) -> String {
    match seconds {
        0..60 => "Just now".to_owned(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        86_400..172_800 => "1 day ago".to_owned(),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

pub fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: FrameRate = FrameRate {
        numerator: 30,
        denominator: 1,
    };

    #[test]
    fn timecode_counts_frames_within_the_second() {
        assert_eq!(timecode(Time::from_seconds(61.5), RATE), "00:01:01:15");
    }

    #[test]
    fn parse_reads_every_documented_form() {
        assert_eq!(parse("12", RATE), Some(Time::from_seconds(12.0)));
        assert_eq!(parse("12.5s", RATE), Some(Time::from_seconds(12.5)));
        assert_eq!(parse("500ms", RATE), Some(Time::from_seconds(0.5)));
        assert_eq!(parse("60f", RATE), Some(Time::from_seconds(2.0)));
        assert_eq!(parse("1:23", RATE), Some(Time::from_seconds(83.0)));
        assert_eq!(parse("1:02:03", RATE), Some(Time::from_seconds(3723.0)));
        assert_eq!(parse("00:00:01:15", RATE), Some(Time::from_seconds(1.5)));
    }

    #[test]
    fn parse_refuses_what_is_not_a_time() {
        assert_eq!(parse("soon", RATE), None);
        assert_eq!(parse("", RATE), None);
    }

    #[test]
    fn age_uses_the_coarsest_unit() {
        assert_eq!(age(12), "Just now");
        assert_eq!(age(150), "2 min ago");
        assert_eq!(age(7300), "2 h ago");
        assert_eq!(age(90_000), "1 day ago");
        assert_eq!(age(400_000), "4 days ago");
    }

    #[test]
    fn short_durations_grow_an_hours_field_only_when_needed() {
        assert_eq!(short(Time::from_seconds(7.0)), "0:07");
        assert_eq!(short(Time::from_seconds(3723.0)), "1:02:03");
    }
}
