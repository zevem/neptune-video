//! What the source monitor's layer takes of the frames asked for before the
//! monitor was opened again.
//!
//! Previewing the asset the monitor already shows opens it again: a fresh
//! monitor takes the place of the one there was, its clock at the start, and
//! the layer kept for it stays, with its picture and its consumer. That is no
//! seek, and nothing says so to the layers but the opening itself. It is
//! applied at the end of a frame like a seek, so the next frame first takes
//! what the worker has published and only then composes, which is where the
//! frame at the start is asked for: a frame asked for where the monitor stood
//! before, published and not yet taken, is handed over after the clock went
//! back and before anything was asked at where it went back to.
//!
//! These open the monitor with [`start_source`], which is what the
//! application opens it with, against the real service of the tests this is
//! nested in, and then take and compose in that order. The monitor here is
//! its clock alone, which is all of it a layer is asked by.
//!
//! `RESET:` is the behavior these were written for: from the moment the
//! monitor is opened again its layer takes nothing asked for before that, and
//! keeps the picture it has until the frame at the start arrives. `CONTROL:`
//! holds without that too, and is what it must not cost: a monitor opened
//! again where it rests asks for nothing, a clip of the timeline loses none
//! of its frames to it, and a monitor opened on another asset starts a layer
//! of that asset.
use super::*;
use crate::app::start_source;
use editor_core::Playback;

const SOURCE: Viewing = Viewing::Source;

/// A source monitor that stands at `time`, paused.
fn monitor_at(time: Time) -> Option<Playback> {
    let mut clock = Playback::default();
    clock.seek(time);
    Some(clock)
}

/// Opens the monitor again as the application does. Returns where its clock
/// then stands, which is where its layer is next asked.
fn open_again(rig: &mut Rig, monitor: &mut Option<Playback>) -> Time {
    start_source(monitor, Playback::default(), &mut rig.textures);
    let Some(clock) = monitor else {
        panic!("FIXTURE: opening the source monitor left none");
    };
    fixture(
        clock.position == Time::ZERO && !clock.playing,
        "a source monitor opened again did not stand paused at its start",
    );
    clock.position
}

#[test]
fn a_frame_published_before_the_source_is_opened_again_is_not_taken_after_it() {
    let mut rig = Rig::new();
    let shot = rig.painted(BOX, BLUE);
    rig.frame(false, &[at(SOURCE, shot, BOX, frame(3))]);
    let consumer = rig.consumer(SOURCE);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(3), BLUE, true)],
        "CONTROL: a paused source monitor was not served once, the frame it asked for"
    );
    let texture = settled(&rig, SOURCE);

    // The monitor is dragged on, and the frame at where it was dragged to is
    // published and not yet taken when the same asset is previewed again.
    let mut monitor = monitor_at(frame(5));
    rig.frame(false, &[at(SOURCE, shot, BOX, frame(5))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    let start = open_again(&mut rig, &mut monitor);

    // The next frame takes what was published before it composes.
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(5), BLUE, false),
        "RESET: a frame asked for before the source monitor was opened again was \
         taken by its layer after that, before the frame at its start was asked for"
    );
    assert!(
        untouched(&rig, SOURCE, texture),
        "RESET: the picture the source monitor had when it was opened again was \
         replaced, or uploaded to, before the frame at its start arrived"
    );

    rig.frame(false, &[at(SOURCE, shot, BOX, start)]);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, Time::ZERO, BLUE, true)],
        "RESET: a source monitor whose earlier frame was turned away was not served \
         once, the frame at its start"
    );
    assert_eq!(
        (rig.consumer(SOURCE), uploads(&rig, texture)),
        (consumer, 1),
        "RESET: the frame at the start of a source monitor opened again did not come \
         to its own layer as the left number of uploads to the texture it had"
    );
    assert_eq!(rig.picture(SOURCE), Some(side(BOX)));
}

#[test]
fn frames_the_source_clock_asked_for_are_not_taken_after_it_is_opened_again() {
    let mut rig = Rig::new();
    let shot = rig.painted(BOX, RED);
    let mut monitor = monitor_at(frame(0));
    rig.frame(true, &[at(SOURCE, shot, BOX, frame(0))]);
    let consumer = rig.consumer(SOURCE);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(0), RED, true)],
        "CONTROL: a playing source monitor was not served once, the frame it first \
         asked for"
    );
    let texture = settled(&rig, SOURCE);

    // The clock asks for its next frame, and for the one after it while that
    // is published and not yet taken: two are the layer's to take, as long
    // as the clock carries on.
    rig.frame(true, &[at(SOURCE, shot, BOX, frame(1))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    rig.frame(true, &[at(SOURCE, shot, BOX, frame(2))]);
    fixture(
        !rig.textures.refused(),
        "the service did not take the clock's next request with the one before decoded",
    );

    // The same asset is previewed again instead.
    let start = open_again(&mut rig, &mut monitor);
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(1), RED, false),
        "RESET: the frame the source clock asked for before its next was taken by \
         its layer after the monitor was opened again"
    );
    fixture(
        rig.publication(),
        "the worker published nothing for the frame the clock asked for last",
    );
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(2), RED, false),
        "RESET: the frame the source clock last asked for was taken by its layer \
         after the monitor was opened again, before the frame at its start was \
         asked for"
    );
    assert!(
        untouched(&rig, SOURCE, texture),
        "RESET: the picture a playing source monitor had when it was opened again \
         was replaced, or uploaded to, before the frame at its start arrived"
    );

    // It stands paused at its start, and is served that frame once.
    rig.frame(false, &[at(SOURCE, shot, BOX, start)]);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, Time::ZERO, RED, true)],
        "RESET: a source monitor whose clock's frames were turned away was not \
         served once, the frame at its start"
    );
    assert_eq!(
        uploads(&rig, texture),
        1,
        "RESET: the frame at the start of a source monitor opened again left the \
         left number of uploads to the texture it had"
    );
}

#[test]
fn a_source_opened_again_where_it_rests_asks_for_nothing() {
    let mut rig = Rig::new();
    let shot = rig.painted(BOX, BLUE);
    let mut monitor = monitor_at(Time::ZERO);
    rig.frame(false, &[at(SOURCE, shot, BOX, Time::ZERO)]);
    let consumer = rig.consumer(SOURCE);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, Time::ZERO, BLUE, true)],
        "CONTROL: a paused source monitor was not served once, the frame it asked for"
    );
    let texture = settled(&rig, SOURCE);

    let start = open_again(&mut rig, &mut monitor);
    for _ in 0..3 {
        rig.frame(false, &[at(SOURCE, shot, BOX, start)]);
    }
    fixture(rig.consumer(SOURCE) == consumer, "the layer changed hands");
    let again = rig.asked_for();
    assert!(
        again.is_empty(),
        "CONTROL: a source monitor opened again on the frame it shows asked for it \
         again: {again:?}"
    );
    assert!(
        untouched(&rig, SOURCE, texture),
        "CONTROL: the picture of a source monitor opened again on the frame it shows \
         was replaced or uploaded to"
    );
}

#[test]
fn a_clip_keeps_the_frame_it_asked_for_when_the_source_is_opened() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, BLUE);
    rig.frame(false, &[at(clip, shot, BOX, frame(3))]);
    let consumer = rig.consumer(clip);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(3), BLUE, true)],
        "CONTROL: a paused layer was not served once, the frame it asked for"
    );
    let texture = settled(&rig, clip);

    // The timeline's playhead rests on a frame that is published and not yet
    // taken when an asset is previewed. Nothing moved that playhead.
    rig.frame(false, &[at(clip, shot, BOX, frame(5))]);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );
    let mut monitor = None;
    open_again(&mut rig, &mut monitor);
    assert_eq!(
        rig.take(),
        published(consumer, shot, frame(5), BLUE, true),
        "CONTROL: a frame a clip of the timeline asked for was not taken by its \
         layer because the source monitor was opened"
    );
    assert_eq!(
        uploads(&rig, texture),
        1,
        "CONTROL: the frame a clip took when the source monitor was opened left the \
         left number of uploads to the texture it had"
    );
}

#[test]
fn a_source_opened_on_another_asset_starts_a_layer_of_that_asset() {
    let mut rig = Rig::new();
    let shot = rig.painted(BOX, BLUE);
    let other = rig.painted(BOX, AMBER);
    let mut monitor = monitor_at(frame(3));
    rig.frame(false, &[at(SOURCE, shot, BOX, frame(3))]);
    let consumer = rig.consumer(SOURCE);
    assert_eq!(
        rig.asked_for(),
        [published(consumer, shot, frame(3), BLUE, true)],
        "CONTROL: a paused source monitor was not served once, the frame it asked for"
    );

    let start = open_again(&mut rig, &mut monitor);
    rig.frame(false, &[at(SOURCE, other, BOX, start)]);
    let fresh = rig.consumer(SOURCE);
    assert_ne!(
        fresh, consumer,
        "CONTROL: a source monitor opened on another asset kept the consumer its \
         frames of the first were asked under"
    );
    assert_eq!(
        rig.asked_for(),
        [published(fresh, other, Time::ZERO, AMBER, true)],
        "CONTROL: a source monitor opened on another asset was not served once, the \
         frame at its start"
    );
    assert_eq!(rig.picture(SOURCE), Some(side(BOX)));
}
