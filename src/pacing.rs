//! When a playing clock next needs a frame painted.
//!
//! A clock that plays shows another picture only when it crosses a frame of
//! its grid, so that is when the next pass is asked for: not a fixed interval
//! after this one, which drifts against the grid and wakes for passes that
//! show the frame already shown.
use editor_core::{FrameRate, Playback, Time};
use std::time::Duration;

/// The shortest wait asked for: no clock is woken more than 120 times a
/// second, however fast it crosses its frames, and never at once.
const SHORTEST_WAKE: Duration = Duration::from_micros(8_333);
/// The longest: a clock that plays is looked in on at least once a second.
const LONGEST_WAKE: Duration = Duration::from_secs(1);

/// How long to ask `egui::Context::request_repaint_after` to wait for the
/// pass that shows the next frame of `playback` on the grid of `rate`, or
/// nothing while it does not play: a clock that stands still wakes nobody.
///
/// That pass is due when the clock, at the speed it runs, reaches the next
/// frame of the grid from where it is now, wherever in a frame this pass
/// found it. It is asked for no sooner than [`SHORTEST_WAKE`], so a clock
/// that crosses frames faster than that, or stands a moment short of one,
/// never asks for the next pass at once; and no later than [`LONGEST_WAKE`].
///
/// `predicted_dt` is what egui expects a pass to take, as its input says.
/// egui takes that off every wait it is asked for, which at 30 frames a
/// second and a sixtieth expected wakes twice a frame, and at 60 leaves no
/// wait at all. A pass that begins before the clock has reached the frame
/// shows the one already shown, so the same is added here and the wait the
/// windowing integration is handed is the one meant.
pub fn playback_wake(playback: &Playback, rate: FrameRate, predicted_dt: f32) -> Option<Duration> {
    if !playback.playing {
        return None;
    }
    let position = playback.position.max(Time::ZERO);
    let next = rate.frame_time(rate.frame_at(position).saturating_add(1));
    let ahead = next.0.saturating_sub(position.0);
    let speed = if playback.speed.is_finite() && playback.speed > 0.0 {
        playback.speed
    } else {
        1.0
    };
    let wait = if ahead > 0 {
        // Rounded up: a pass a moment early would find the frame before.
        Duration::from_micros((ahead as f64 / speed).ceil() as u64)
    } else {
        // A grid with no next frame is paced as one frame of its rate.
        Duration::from_secs_f64(1.0 / rate.fps().clamp(1.0, 120.0))
    };
    Some(handed(wait, predicted_dt))
}

/// How long to ask for while a layer drawn for a playing clock was refused by
/// the media service, which had no room for what it asked: half a frame of
/// the grid of `rate`, within the same bounds as [`playback_wake`].
///
/// The service says when a frame is decoded, and not when it then has room
/// for the next, so a layer refused in the pass a frame brought about would
/// otherwise wait for the clock's next frame to ask again and miss one.
/// Nothing refused asks for nothing here: this is no clock of its own.
pub fn refused_wake(rate: FrameRate, predicted_dt: f32) -> Duration {
    let frame = u64::try_from(rate.frame_time(1).0).unwrap_or(0);
    handed(Duration::from_micros(frame / 2), predicted_dt)
}

/// What to ask egui for so that the windowing integration is handed `wait`,
/// held to the shortest and the longest asked for. As egui reads what it
/// expects a pass to take: what it would not take off is not added.
fn handed(wait: Duration, predicted_dt: f32) -> Duration {
    let predicted = Duration::try_from_secs_f32(predicted_dt).unwrap_or_default();
    wait.clamp(SHORTEST_WAKE, LONGEST_WAKE)
        .saturating_add(predicted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;
    use std::sync::{Arc, Mutex};

    /// What egui expects a pass to take until an integration says otherwise.
    const PREDICTED: f32 = 1.0 / 60.0;

    const NTSC: FrameRate = FrameRate {
        numerator: 30_000,
        denominator: 1001,
    };

    fn rate(fps: u32) -> FrameRate {
        FrameRate {
            numerator: fps,
            denominator: 1,
        }
    }

    fn playing(position: Time, speed: f64) -> Playback {
        let mut playback = Playback::default();
        playback.seek(position);
        playback.set_speed(speed).expect("a speed the clock takes");
        playback.play();
        playback
    }

    /// What egui 0.36 leaves of a delay it is asked for: it takes the time it
    /// expects a pass to take off it. `egui_takes_its_predicted_pass_off_a_delay`
    /// holds this to the real thing.
    fn left_by_egui(asked: Duration, predicted_dt: f32) -> Duration {
        asked.saturating_sub(Duration::try_from_secs_f32(predicted_dt).unwrap_or_default())
    }

    /// The wait the windowing integration is handed for a clock that plays.
    fn waited(playback: &Playback, rate: FrameRate, predicted_dt: f32) -> Duration {
        let Some(asked) = playback_wake(playback, rate, predicted_dt) else {
            panic!("PACING: a clock that plays asked for no pass");
        };
        left_by_egui(asked, predicted_dt)
    }

    fn micros(micros: u64) -> Duration {
        Duration::from_micros(micros)
    }

    #[test]
    fn egui_takes_its_predicted_pass_off_a_delay() {
        let ctx = egui::Context::default();
        let handed = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&handed);
        ctx.set_request_repaint_callback(move |info| {
            sink.lock().expect("unpoisoned").push(info.delay);
        });
        let predicted = ctx.input(|input| input.predicted_dt);
        let asked = Duration::from_millis(100);
        ctx.request_repaint_after(asked);
        assert_eq!(
            *handed.lock().expect("unpoisoned"),
            [left_by_egui(asked, predicted)],
            "CONTROL: egui hands its integration another wait than these tests take \
             it to"
        );
        assert!(left_by_egui(asked, predicted) < asked);
    }

    #[test]
    fn a_clock_that_does_not_play_asks_for_no_pass() {
        let mut paused = Playback::default();
        paused.seek(rate(30).frame_time(7));
        assert_eq!(playback_wake(&paused, rate(30), PREDICTED), None);
        assert_eq!(playback_wake(&paused, NTSC, 0.0), None);
        // Nor one that played and then stopped, by itself or by a pause.
        paused.play();
        assert!(playback_wake(&paused, rate(30), PREDICTED).is_some());
        paused.pause();
        assert_eq!(
            playback_wake(&paused, rate(30), PREDICTED),
            None,
            "PACING: a clock that was stopped asks for a pass"
        );
    }

    #[test]
    fn a_playing_clock_waits_a_whole_frame_from_a_frame_boundary() {
        for (fps, frame) in [(24, 41_666), (25, 40_000), (30, 33_333), (60, 16_666)] {
            let rate = rate(fps);
            for index in [0, 1, 29, 1800] {
                let playback = playing(rate.frame_time(index), 1.0);
                let next = rate.frame_time(index + 1).0 - rate.frame_time(index).0;
                fixture_frame(next, frame);
                assert_eq!(
                    waited(&playback, rate, PREDICTED),
                    micros(next as u64),
                    "PACING: at frame {index} of {fps} fps the integration is handed the \
                     left to wait; the time to the next frame is the right"
                );
            }
        }
    }

    /// A frame of a whole rate is its nominal length or a microsecond more.
    fn fixture_frame(next: i64, nominal: i64) {
        assert!(
            (nominal..=nominal + 1).contains(&next),
            "FIXTURE: a frame of {next} us where {nominal} was meant"
        );
    }

    #[test]
    fn a_sixty_hertz_clock_is_not_woken_at_once() {
        let rate = rate(60);
        let playback = playing(rate.frame_time(120), 1.0);
        let wait = waited(&playback, rate, PREDICTED);
        assert!(
            wait > Duration::ZERO,
            "PACING: at 60 fps, with a pass expected to take a sixtieth of a second, \
             the integration is handed no wait: every pass asks for the next at once"
        );
        assert_eq!(wait, micros(16_666));
    }

    #[test]
    fn a_pass_between_two_frames_waits_only_for_what_is_left_of_the_frame() {
        let rate = rate(30);
        let start = rate.frame_time(10);
        for into in [1, 10_000, 20_000] {
            let playback = playing(Time(start.0 + into), 1.0);
            assert_eq!(
                waited(&playback, rate, PREDICTED),
                micros((33_333 - into) as u64),
                "PACING: {into} us into a frame of 30 fps the integration is handed the \
                 left to wait; what is left of the frame is the right"
            );
        }
    }

    #[test]
    fn a_fractional_rate_is_paced_on_its_own_grid() {
        // 30000/1001: boundaries at 0, 33366, 66733 and 100100 us.
        let boundaries = [0, 33_366, 66_733, 100_100].map(Time);
        for (index, boundary) in boundaries.iter().enumerate() {
            assert_eq!(
                NTSC.frame_time(index as i64),
                *boundary,
                "FIXTURE: the grid of 30000/1001 is not what this test takes it to be"
            );
        }
        for pair in boundaries.windows(2) {
            let playback = playing(pair[0], 1.0);
            assert_eq!(
                waited(&playback, NTSC, PREDICTED),
                micros((pair[1].0 - pair[0].0) as u64),
                "PACING: from the boundary at {:?} of 30000/1001",
                pair[0]
            );
        }
        let between = playing(Time(10_000), 1.0);
        assert_eq!(waited(&between, NTSC, PREDICTED), micros(23_366));
        // The last microsecond of a frame is still that frame.
        let last = playing(Time(33_365), 1.0);
        assert_eq!(waited(&last, NTSC, PREDICTED), SHORTEST_WAKE);
    }

    #[test]
    fn a_faster_or_slower_clock_reaches_the_next_frame_sooner_or_later() {
        let rate = rate(30);
        let start = rate.frame_time(3);
        for (speed, wait) in [(0.5, 66_666), (1.0, 33_333), (1.5, 22_222), (2.0, 16_667)] {
            let playback = playing(start, speed);
            assert_eq!(
                waited(&playback, rate, PREDICTED),
                micros(wait),
                "PACING: at {speed} times the speed of a 30 fps clock"
            );
        }
        // The slowest the clock goes, and a quarter of the way into a frame.
        let slow = playing(Time(start.0 + 8_333), 0.1);
        assert_eq!(waited(&slow, rate, PREDICTED), micros(250_000));
    }

    #[test]
    fn no_clock_is_woken_sooner_than_the_shortest_wait_nor_at_once() {
        let rate = rate(30);
        // Eight times the speed crosses a frame in about four milliseconds.
        let fast = playing(rate.frame_time(3), 8.0);
        assert_eq!(waited(&fast, rate, PREDICTED), SHORTEST_WAKE);
        // A microsecond short of a boundary.
        let nearly = playing(Time(rate.frame_time(4).0 - 1), 1.0);
        assert_eq!(waited(&nearly, rate, PREDICTED), SHORTEST_WAKE);
        // The fastest grid a project may have.
        let quick = FrameRate {
            numerator: 240,
            denominator: 1,
        };
        assert_eq!(
            waited(&playing(Time::ZERO, 1.0), quick, PREDICTED),
            SHORTEST_WAKE
        );
        assert!(SHORTEST_WAKE > Duration::ZERO);
    }

    #[test]
    fn the_wait_is_the_same_whatever_a_pass_is_expected_to_take() {
        let rate = rate(30);
        let playback = playing(rate.frame_time(3), 1.0);
        for predicted in [0.0, 1.0 / 240.0, 1.0 / 144.0, 1.0 / 60.0, 1.0 / 30.0, 0.25] {
            assert_eq!(
                waited(&playback, rate, predicted),
                micros(33_333),
                "PACING: with a pass expected to take {predicted} s"
            );
        }
        // A reading egui itself would not take off the wait is not added.
        for unusable in [f32::NAN, -1.0, f32::INFINITY] {
            assert_eq!(
                playback_wake(&playback, rate, unusable),
                Some(micros(33_333))
            );
        }
    }

    #[test]
    fn a_grid_or_a_speed_that_cannot_be_read_still_waits() {
        let nothing = FrameRate {
            numerator: 0,
            denominator: 1,
        };
        let playback = playing(Time::from_seconds(2.0), 1.0);
        let wait = waited(&playback, nothing, PREDICTED);
        assert!((SHORTEST_WAKE..=LONGEST_WAKE).contains(&wait));
        let mut stuck = playing(Time::ZERO, 1.0);
        for speed in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            stuck.speed = speed;
            assert_eq!(waited_at(&stuck), micros(33_333), "at a speed of {speed}");
        }
        // A grid slower than one frame a second is looked in on each second.
        let crawl = FrameRate {
            numerator: 1,
            denominator: 4,
        };
        assert_eq!(
            waited(&playing(Time::ZERO, 1.0), crawl, PREDICTED),
            LONGEST_WAKE
        );
    }

    fn waited_at(playback: &Playback) -> Duration {
        waited(playback, rate(30), PREDICTED)
    }

    #[test]
    fn a_refused_layer_is_looked_in_on_half_a_frame_later() {
        let halves = [
            (rate(24), micros(20_833)),
            (rate(30), micros(16_666)),
            (NTSC, micros(16_683)),
            (rate(60), SHORTEST_WAKE),
            (rate(240), SHORTEST_WAKE),
        ];
        for (rate, half) in halves {
            for predicted in [0.0, PREDICTED, 1.0 / 144.0] {
                assert_eq!(
                    left_by_egui(refused_wake(rate, predicted), predicted),
                    half,
                    "PACING: a layer refused on the grid of {rate:?}, with a pass \
                     expected to take {predicted} s"
                );
            }
        }
        // Never at once, whatever the grid, and never sooner than a clock is.
        let nothing = FrameRate {
            numerator: 0,
            denominator: 1,
        };
        assert_eq!(
            left_by_egui(refused_wake(nothing, PREDICTED), PREDICTED),
            SHORTEST_WAKE
        );
        // No sooner than the clock's own next frame comes round twice.
        let playback = playing(rate(30).frame_time(3), 1.0);
        assert!(
            2 * left_by_egui(refused_wake(rate(30), PREDICTED), PREDICTED)
                <= waited(&playback, rate(30), PREDICTED)
        );
    }
}
