//! What a playing layer asks the media worker for, and when.
//!
//! Composed and published as in the tests this is nested in: real layers in a
//! real context against a real [`MediaService`] whose worker is held at each
//! result, so every frame here is composed while it stands still with what was
//! asked before still waiting. Which frame a layer was served is read from the
//! time the worker publishes with it.
//!
//! `PLAYBACK:` is the behavior these were written for: a layer that plays
//! asks for one frame at a time, and something other than the clock takes the
//! place of what it waits for. `CONTROL:` holds without that too.
//!
//! The stills here need no decoder, so nothing is shown of how many video
//! decoders are open at once, nor of what a display paints or how fast.
use super::*;

/// The own picture of a layer, published for `consumer` at the time it was
/// asked at, filling its box with the colour of its still, and kept.
fn shown(
    consumer: PreviewConsumer,
    asset: AssetId,
    side: u32,
    time: Time,
    colour: [u8; 4],
) -> Outcome {
    Outcome {
        consumer,
        asset,
        size: [side, side],
        result: Published::Picture {
            time,
            texels: [side, side],
            colour: Some(colour),
            taken: true,
        },
    }
}

/// A clip of a still of its own, drawn alone at each frame of `clock` while
/// playing, with nothing published in between.
fn played(rig: &mut Rig, clock: std::ops::Range<i64>) -> (Viewing, AssetId, PreviewConsumer) {
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, RED);
    for index in clock {
        rig.frame(true, &[at(clip, shot, BOX, frame(index))]);
    }
    (clip, shot, rig.consumer(clip))
}

#[test]
fn a_playing_layer_is_served_the_frame_it_asked_for_and_then_asks_the_next() {
    let mut rig = Rig::new();
    let (clip, shot, consumer) = played(&mut rig, 0..3);
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(0), RED)],
        "PLAYBACK: a layer drawn at three frames of the clock, with nothing published \
         in between, was not served once, the frame it first asked for"
    );

    // That one is decoded: the clock's next frame is asked for, once.
    rig.frame(true, &[at(clip, shot, BOX, frame(3))]);
    rig.frame(true, &[at(clip, shot, BOX, frame(4))]);
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(3), RED)],
        "PLAYBACK: a playing layer whose frame had been published was not served once \
         more, the frame the clock was at when it next asked"
    );
    rig.frame(true, &[at(clip, shot, BOX, frame(3))]);
    let again = rig.asked_for();
    assert!(
        again.is_empty(),
        "CONTROL: a playing layer asked again for the frame it was last served: {again:?}"
    );
}

#[test]
fn a_playing_layer_is_refused_only_while_the_service_has_no_room_for_it() {
    let mut rig = Rig::new();
    let (clip, shot, consumer) = played(&mut rig, 0..1);
    assert!(
        !rig.textures.refused(),
        "CONTROL: a layer whose first request was taken is said to be refused"
    );
    // The clock moves on while the first frame still waits to be decoded.
    rig.frame(true, &[at(clip, shot, BOX, frame(1))]);
    assert!(
        rig.textures.refused(),
        "PLAYBACK: a playing layer that asked while the frame before was still \
         waiting is not said to be refused"
    );
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(0), RED)],
        "CONTROL: a refused layer was not served once, the frame it first asked for"
    );
    rig.frame(true, &[at(clip, shot, BOX, frame(2))]);
    assert!(
        !rig.textures.refused(),
        "PLAYBACK: a layer is still said to be refused after its request was taken"
    );
    // Nor is one that is no longer drawn, whatever it waited for.
    rig.frame(true, &[at(clip, shot, BOX, frame(3))]);
    fixture(rig.textures.refused(), "a layer that waits is refused");
    rig.frame(true, &[]);
    assert!(
        !rig.textures.refused(),
        "PLAYBACK: a layer that is not drawn is said to be refused"
    );
}

#[test]
fn a_paused_layer_is_served_the_frame_it_last_asked_for() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, BLUE);
    for index in 0..3 {
        rig.frame(false, &[at(clip, shot, BOX, frame(index))]);
    }
    let consumer = rig.consumer(clip);
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(2), BLUE)],
        "CONTROL: a paused layer moved over three frames, with nothing published in \
         between, was not served once, the frame it was left at"
    );
}

#[test]
fn a_layer_that_stops_playing_is_served_the_frame_it_stopped_at() {
    let mut rig = Rig::new();
    let (clip, shot, consumer) = played(&mut rig, 0..3);
    rig.frame(false, &[at(clip, shot, BOX, frame(2))]);
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(2), RED)],
        "PLAYBACK: a layer paused while the frame it asked for playing was still \
         waiting was not served once, the frame it stopped at"
    );
}

#[test]
fn a_seek_while_playing_takes_the_place_of_the_frame_waited_for_once() {
    let mut rig = Rig::new();
    let (clip, shot, consumer) = played(&mut rig, 0..3);
    // The application moved the playhead; the clock then carries on from it.
    rig.textures.jumped();
    rig.frame(true, &[at(clip, shot, BOX, frame(40))]);
    rig.frame(true, &[at(clip, shot, BOX, frame(41))]);
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(40), RED)],
        "PLAYBACK: a playing layer whose playhead was moved was not served once, the \
         frame it was moved to: neither the one it waited for before, nor one the \
         clock passed after"
    );
    rig.frame(true, &[at(clip, shot, BOX, frame(42))]);
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(42), RED)],
        "PLAYBACK: a playing layer did not carry on with the clock after a seek"
    );
}

/// The frame `consumer` asked for at `time`, published and held back from
/// its layer: decoded as asked, and no longer what the layer asks for.
fn turned_away(
    consumer: PreviewConsumer,
    asset: AssetId,
    side: u32,
    time: Time,
    colour: [u8; 4],
) -> Outcome {
    Outcome {
        consumer,
        asset,
        size: [side, side],
        result: Published::Picture {
            time,
            texels: [side, side],
            colour: Some(colour),
            taken: false,
        },
    }
}

#[test]
fn a_frame_published_before_a_paused_seek_is_not_taken_after_it() {
    let mut rig = Rig::new();
    let clip = Viewing::Clip(ClipId::new_v4());
    let shot = rig.painted(BOX, BLUE);
    rig.frame(false, &[at(clip, shot, BOX, frame(3))]);
    let consumer = rig.consumer(clip);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );

    // The frame is published and not yet taken when the playhead is moved:
    // the service takes the request at the new time in its place.
    rig.frame(false, &[at(clip, shot, BOX, frame(40))]);
    assert_eq!(
        rig.take(),
        turned_away(consumer, shot, BOX, frame(3), BLUE),
        "SEEK: a frame published before a seek, at the time asked before it, was \
         taken by the layer after the seek"
    );
    assert_eq!(
        rig.picture(clip),
        None,
        "SEEK: a layer shows a frame of the time it asked for before a seek"
    );
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(40), BLUE)],
        "SEEK: a layer whose earlier frame was turned away was not served once, the \
         frame it was moved to"
    );
    assert_eq!(rig.picture(clip), Some(side(BOX)));
}

#[test]
fn a_frame_published_while_the_clock_asks_for_its_next_is_still_taken() {
    let mut rig = Rig::new();
    let (clip, shot, consumer) = played(&mut rig, 0..1);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );

    // The frame is published and not yet taken when the clock, one frame
    // on, asks for its next: nothing took its place, and the service takes
    // the next because that one is decoded.
    rig.frame(true, &[at(clip, shot, BOX, frame(1))]);
    assert_eq!(
        rig.take(),
        shown(consumer, shot, BOX, frame(0), RED),
        "PLAYBACK: a frame the clock asked for, published while its next was asked \
         for, was not taken by its layer"
    );
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(1), RED)],
        "PLAYBACK: a playing layer was not served once, the frame the clock asked \
         for next"
    );
}

#[test]
fn a_frame_published_before_a_seek_while_playing_is_not_taken_after_it() {
    let mut rig = Rig::new();
    let (clip, shot, consumer) = played(&mut rig, 0..1);
    fixture(
        rig.publication(),
        "the worker published nothing for a picture that was asked for",
    );

    // The application moved the playhead with that frame published and not
    // yet taken; the clock then carries on from where it was moved to.
    rig.textures.jumped();
    rig.frame(true, &[at(clip, shot, BOX, frame(40))]);
    assert_eq!(
        rig.take(),
        turned_away(consumer, shot, BOX, frame(0), RED),
        "SEEK: a frame published before a seek while playing, at the time asked \
         before it, was taken by the layer after the seek"
    );
    rig.frame(true, &[at(clip, shot, BOX, frame(41))]);
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(40), RED)],
        "SEEK: a playing layer whose earlier frame was turned away was not served \
         once, the frame it was moved to"
    );
    rig.frame(true, &[at(clip, shot, BOX, frame(42))]);
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(42), RED)],
        "PLAYBACK: a playing layer did not carry on with the clock after a seek"
    );
}

#[test]
fn a_smaller_box_while_playing_takes_the_place_of_the_frame_waited_for() {
    let mut rig = Rig::new();
    let (clip, shot, consumer) = played(&mut rig, 0..2);
    let smaller = BOX / 2;
    rig.frame(true, &[at(clip, shot, smaller, frame(2))]);
    let asked = rig.asked_for();
    let served: Vec<_> = asked
        .iter()
        .map(|outcome| (outcome.consumer, outcome.size, outcome.result))
        .collect();
    assert!(
        matches!(
            served.as_slice(),
            [(by, size, Published::Picture { time, texels, taken: true, .. })]
                if *by == consumer
                    && *size == [smaller, smaller]
                    && *texels == [smaller, smaller]
                    && *time == frame(2)
        ),
        "PLAYBACK: a playing layer held to a smaller box was not served once, at that \
         box and the frame it was then at, in the place of the frame it waited for at \
         the larger one: {asked:?}"
    );
    assert_eq!(rig.picture(clip), Some(side(smaller)));
}

#[test]
fn another_source_while_playing_is_asked_for_in_the_place_of_the_first() {
    let mut rig = Rig::new();
    let first = rig.painted(BOX, RED);
    let second = rig.painted(BOX, BLUE);
    rig.frame(true, &[at(Viewing::Source, first, BOX, frame(0))]);
    let before = rig.consumer(Viewing::Source);
    rig.frame(true, &[at(Viewing::Source, second, BOX, frame(1))]);
    let consumer = rig.consumer(Viewing::Source);
    fixture(
        consumer != before,
        "the monitor turned to another asset under the consumer it had",
    );
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, second, BOX, frame(1), BLUE)],
        "CONTROL: a playing monitor turned to another asset was not served once, that \
         asset, with nothing of the one it left"
    );
}

#[test]
fn a_clip_that_carries_on_from_a_cut_while_playing_is_served_its_own_frame() {
    let mut rig = Rig::new();
    let (from, to) = (ClipId::new_v4(), ClipId::new_v4());
    let shot = rig.painted(BOX, AMBER);
    for index in 0..2 {
        rig.frame(true, &[at(Viewing::Clip(from), shot, BOX, frame(index))]);
    }
    let consumer = rig.consumer(Viewing::Clip(from));

    // The playhead crosses the cut: the next clip takes the layer over, at
    // a source time of its own.
    rig.textures.begin_frame();
    let wanted = [Wanted {
        viewing: Viewing::Clip(to),
        asset: &rig.library[&shot],
        size: [BOX, BOX],
        time: frame(40),
        carries_on: vec![from],
    }];
    rig.textures
        .compose(Some(&rig.media.service), true, &wanted);
    fixture(
        rig.textures.asked(Viewing::Clip(to)) == Some((consumer, [BOX, BOX])),
        "the layer of the clip before the cut did not change hands",
    );
    assert_eq!(
        rig.asked_for(),
        [shown(consumer, shot, BOX, frame(40), AMBER)],
        "PLAYBACK: a clip that took over the layer of a cut while playing was not \
         served once, its own frame, in the place of the one the clip before it \
         waited for"
    );
}

#[test]
fn five_playing_layers_are_each_served_in_turn_the_frame_they_asked_for() {
    const LAYERS: u8 = 5;
    let mut rig = Rig::new();
    let cohort: Vec<Member> = (0..LAYERS)
        .map(|index| {
            let colour = [40 + 40 * index, 220 - 30 * index, 90, 255];
            Member {
                viewing: Viewing::Clip(ClipId::new_v4()),
                asset: rig.painted(BOX, colour),
                colour,
            }
        })
        .collect();
    rig.frame(true, &all_at(&cohort, frame(0)));

    // A result published and then a frame of playback, one frame on, that
    // draws them all: twice round.
    let mut served = vec![Vec::new(); cohort.len()];
    for round in 0..2 * i64::from(LAYERS) {
        let Some(outcome) = rig.result() else {
            panic!("PLAYBACK: the worker published nothing in round {round}: {served:?}");
        };
        let (Some(member), Published::Picture { time, .. }) =
            (rig.own_picture(&cohort, outcome), outcome.result)
        else {
            panic!("CONTROL: the worker published what is no layer's own picture: {outcome:?}");
        };
        served[member].push(time);
        rig.frame(true, &all_at(&cohort, frame(round + 1)));
    }
    // Each first asked at the first frame, and asked next in the frame after
    // its own was published, which is one frame later for each in turn.
    let turns: Vec<Vec<Time>> = (1..=i64::from(LAYERS))
        .map(|next| vec![frame(0), frame(next)])
        .collect();
    assert_eq!(
        served, turns,
        "PLAYBACK: the frames {LAYERS} layers drawn together while playing were served, \
         in paint order, are the left; twice each in turn, the frame each asked for \
         when it was last free to ask, is the right"
    );
}
